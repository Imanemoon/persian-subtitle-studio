//! تست‌های یکپارچه برای Staging و Batch Download با SubSource (فاز ۳ PR ۲)
//!
//! همه تست‌ها با fake executable و روی فایل‌سیستم واقعی انجام می‌شوند.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use subsync_studio_lib::commands::subsource::{
    download_done_payload, run_download_blocking, run_download_blocking_with_remover,
    run_download_blocking_with_removers, SUBSOURCE_DONE_EVENT,
};
use subsync_studio_lib::models::{OpsProgress, VideoEntry};
use subsync_studio_lib::settings::Settings;
use subsync_studio_lib::state::{acquire_batch_op, acquire_single_op, AppState, OpsRegistry};
use subsync_studio_lib::subsource::plan::{
    create_download_plan, inspect_persian_sidecars, validate_video_typed, DownloadPlanItem,
    SkipReasonCode,
};
use subsync_studio_lib::subsource::staging::{
    build, group_by_volume, hard_link_no_overwrite, volume_root_of, StagedPlacement,
    STAGING_DIR_PREFIX,
};

/// دریافت یا کامپایل fake_subsource executable
fn get_fake_exe() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let target_dir = PathBuf::from(manifest_dir).join("target").join("debug");

    #[cfg(windows)]
    let exe_name = "fake_subsource.exe";
    #[cfg(not(windows))]
    let exe_name = "fake_subsource";

    let exe_path = target_dir.join(exe_name);

    if !exe_path.exists() {
        let source_path = PathBuf::from(manifest_dir)
            .join("tests")
            .join("fake_subsource.rs");
        let status = std::process::Command::new("rustc")
            .arg(&source_path)
            .arg("-o")
            .arg(&exe_path)
            .arg("--edition")
            .arg("2021")
            .status();

        if status.map(|s| s.success()).unwrap_or(false) && exe_path.exists() {
            return exe_path;
        }

        panic!(
            "fake_subsource executable not found at {:?}. Run: rustc tests/fake_subsource.rs -o target/debug/fake_subsource.exe --edition 2021",
            exe_path
        );
    }

    exe_path
}

/// ایجاد پوشه موقت منحصربه‌فرد برای ایزولاسیون کامل تست‌های موازی
fn unique_test_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "subsync-dl-test-{tag}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("ایجاد پوشه تستی منحصربه‌فرد");
    dir
}

/// ساخت AppState آماده با override برای ابزار subsource
fn make_test_state(fake_exe: &Path, library: Vec<VideoEntry>) -> AppState {
    let mut settings = Settings::default();
    settings.tool_paths.subsource = Some(fake_exe.to_string_lossy().to_string());

    AppState {
        settings: Mutex::new(settings),
        library: Mutex::new(library),
        last_scan: Mutex::new(None),
        ops: Arc::new(Mutex::new(OpsRegistry::default())),
        pipeline: Mutex::new(None),
        archive_provenance: Mutex::new(
            subsync_studio_lib::archive::provenance::ProvenanceRegistry::default(),
        ),
        setup_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }
}

/// ساخت ورودی فرضی کتابخانه
fn make_video_entry(path: &Path) -> VideoEntry {
    VideoEntry {
        id: path.to_string_lossy().to_string(),
        path: path.to_string_lossy().to_string(),
        file_name: path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default(),
        size_bytes: 1024,
        modified_ms: None,
        parent_dir: path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        show_name: Some("Show".to_string()),
        season: Some(1),
        episode: Some(1),
        sibling_subtitles: Vec::new(),
        analysis: None,
    }
}

/// تست ۱ و ۲ و ۹ و ۱۰ و ۱۲ و ۱۳:
/// اجرای موفق کامل با Fake SubSource:
/// - ساخت staging از طریق hard links
/// - حفظ ساختار پوشه‌ها
/// - دانلود آرشیوها در sub/
/// - حذف لینک‌های ویدیوی ساخته‌شده برنامه پس از اجرا
/// - حفظ آرشیوهای دانلودشده و ریشه staging برای فاز بعدی
/// - سالم ماندن و دست‌نخورده ماندن فایل‌های ویدیوی اصلی
#[test]
fn test_successful_staging_and_batch_download_flow() {
    let fake_exe = get_fake_exe();
    let base_dir = unique_test_dir("success-flow");

    // نشانگر حالت دانلود موفق برای fake executable
    fs::write(base_dir.join(".fake_mode"), b"write-archives").unwrap();

    let s1_dir = base_dir.join("My Series").join("Season 01");
    let s2_dir = base_dir.join("My Series").join("Season 02");
    fs::create_dir_all(&s1_dir).unwrap();
    fs::create_dir_all(&s2_dir).unwrap();
    let v1 = s1_dir.join("Show.S01E01.1080p.mkv");
    let v2 = s2_dir.join("Show.S02E01.1080p.mp4");
    fs::write(&v1, b"ORIGINAL_VIDEO_CONTENT_1").unwrap();
    fs::write(&v2, b"ORIGINAL_VIDEO_CONTENT_2").unwrap();

    let entries = vec![make_video_entry(&v1), make_video_entry(&v2)];
    let state = make_test_state(&fake_exe, entries);

    let plan_items = vec![
        DownloadPlanItem {
            video_path: v1.to_string_lossy().to_string(),
            video_file_name: "Show.S01E01.1080p.mkv".into(),
        },
        DownloadPlanItem {
            video_path: v2.to_string_lossy().to_string(),
            video_file_name: "Show.S02E01.1080p.mp4".into(),
        },
    ];

    let staging_base = base_dir.join("staging_area");
    fs::create_dir_all(&staging_base).unwrap();

    let cancel = Arc::new(AtomicBool::new(false));
    let mut progress_events: Vec<OpsProgress> = Vec::new();

    let result = run_download_blocking(
        &state,
        &plan_items,
        Some("test-secret-api-key".into()),
        Some(&staging_base),
        &cancel,
        &mut |p| progress_events.push(p),
    );

    // ۱) بررسی خروجی عملیات
    assert!(result.success, "Download must succeed: {}", result.message);
    assert!(!result.cancelled);
    assert_eq!(result.planned, 2);
    assert_eq!(result.staged, 2);
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.staging_paths.len(), 1);

    let staging_root = PathBuf::from(&result.staging_paths[0]);
    assert!(staging_root.is_dir(), "Staging root must be preserved");

    // ۱.۵) نگاشت پایدار ویدیوی اصلی → مسیر staging برای فاز آرشیو
    assert_eq!(
        result.archive_sources.len(),
        2,
        "Each staged video must have archive provenance"
    );
    let library = state.library.lock().unwrap().clone();
    let registry = state.archive_provenance.lock().unwrap();
    for source in &result.archive_sources {
        let canonical = registry
            .validate(source, &library)
            .expect("staging created by backend must validate");
        assert_eq!(canonical.video_path, source.video_path);
        assert_eq!(canonical.staging_root, source.staging_root);
        assert_eq!((canonical.season, canonical.episode), (Some(1), Some(1)));
    }
    // season/episode فرانت قابل جعل نیست و با مقدار authoritative جایگزین می‌شود.
    let mut forged = result.archive_sources[0].clone();
    forged.season = Some(9);
    forged.episode = Some(9);
    let canonical = registry.validate(&forged, &library).unwrap();
    assert_eq!((canonical.season, canonical.episode), (Some(1), Some(1)));
    drop(registry);
    let provenance_v1 = result
        .archive_sources
        .iter()
        .find(|s| s.video_path == v1.to_string_lossy())
        .expect("provenance for v1");
    assert_eq!(provenance_v1.video_file_name, "Show.S01E01.1080p.mkv");
    assert_eq!(provenance_v1.staging_root, result.staging_paths[0]);
    assert_eq!(provenance_v1.relative_video_dir, "Season 01");
    assert_eq!(
        provenance_v1.relative_video_path,
        Path::new("Season 01")
            .join("Show.S01E01.1080p.mkv")
            .to_string_lossy()
    );
    let provenance_v2 = result
        .archive_sources
        .iter()
        .find(|s| s.video_path == v2.to_string_lossy())
        .expect("provenance for v2");
    assert_eq!(provenance_v2.relative_video_dir, "Season 02");
    // API key هرگز نباید در نتیجه یا provenance سریال شود
    let payload_text = serde_json::to_string(&download_done_payload(&result)).unwrap();
    assert!(!payload_text.contains("test-secret-api-key"));
    assert!(payload_text.contains("archiveSources"));

    // ۲) بررسی حذف لینک‌های ویدیویی موقت
    let staged_v1 = staging_root.join("Season 01").join("Show.S01E01.1080p.mkv");
    let staged_v2 = staging_root.join("Season 02").join("Show.S02E01.1080p.mp4");
    assert!(
        !staged_v1.exists(),
        "Temporary hard link v1 must be removed"
    );
    assert!(
        !staged_v2.exists(),
        "Temporary hard link v2 must be removed"
    );

    // ۳) بررسی حفظ خروجی‌های دانلود (آرشیوهای sub/)
    let sub_s1 = staging_root.join("Season 01").join("sub");
    let sub_s2 = staging_root.join("Season 02").join("sub");
    assert!(
        sub_s1.is_dir(),
        "Season 01 sub directory must be preserved in staging"
    );
    assert!(
        sub_s2.is_dir(),
        "Season 02 sub directory must be preserved in staging"
    );
    let a1: Vec<_> = fs::read_dir(&sub_s1)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().map(|x| x == "zip").unwrap_or(false))
        .collect();
    let a2: Vec<_> = fs::read_dir(&sub_s2)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().map(|x| x == "zip").unwrap_or(false))
        .collect();
    assert_eq!(
        a1.len(),
        1,
        "Must contain 1 downloaded fake archive for S01"
    );
    assert_eq!(
        a2.len(),
        1,
        "Must contain 1 downloaded fake archive for S02"
    );

    // ۴) بررسی سلامت کامل فایل‌های اصلی
    assert_eq!(fs::read(&v1).unwrap(), b"ORIGINAL_VIDEO_CONTENT_1");
    assert_eq!(fs::read(&v2).unwrap(), b"ORIGINAL_VIDEO_CONTENT_2");

    // ۵) بررسی گزارش پیشرفت مرحله‌دار
    assert!(!progress_events.is_empty());
    assert!(progress_events
        .iter()
        .any(|p| p.message.contains("در حال آماده‌سازی")));
    assert!(progress_events.iter().any(|p| p.message.contains("لینک")));
    assert!(progress_events
        .iter()
        .any(|p| p.message.contains("SubSource")));

    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۳: نام‌های فارسی، یونیکد و مسیرهای دارای فاصله
#[test]
fn test_persian_unicode_and_spaces_handling() {
    let fake_exe = get_fake_exe();
    let base_dir = unique_test_dir("persian-unicode");

    fs::write(base_dir.join(".fake_mode"), b"write-archives").unwrap();

    let lib_dir = base_dir
        .join("پوشه فارسی با فاصله")
        .join("فصل اول - Season 01");
    fs::create_dir_all(&lib_dir).unwrap();
    let v1 = lib_dir.join("قسمت اول - Episode 01 [1080p].mkv");
    fs::write(&v1, b"PERSIAN_UNICODE_CONTENT").unwrap();

    let entries = vec![make_video_entry(&v1)];
    let state = make_test_state(&fake_exe, entries);

    let plan_items = vec![DownloadPlanItem {
        video_path: v1.to_string_lossy().to_string(),
        video_file_name: "قسمت اول - Episode 01 [1080p].mkv".into(),
    }];

    let staging_base = base_dir.join("staging_area");
    fs::create_dir_all(&staging_base).unwrap();

    let cancel = Arc::new(AtomicBool::new(false));
    let result = run_download_blocking(
        &state,
        &plan_items,
        Some("valid-api-key".into()),
        Some(&staging_base),
        &cancel,
        &mut |_| {},
    );

    assert!(result.success, "{}", result.message);
    assert_eq!(result.staged, 1);

    // بررسی سلامت فایل فارسی
    assert_eq!(fs::read(&v1).unwrap(), b"PERSIAN_UNICODE_CONTENT");

    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۱۱: دانلود جزئی (خروج با خطا) باید خروجی‌های دانلودشده را حفظ کند
/// و مسیر بازیابی را برگرداند
#[test]
fn test_partial_failure_preserves_downloaded_output() {
    let fake_exe = get_fake_exe();
    let base_dir = unique_test_dir("partial-fail");

    // حالت خطا پس از نوشتن آرشیوها
    fs::write(base_dir.join(".fake_mode"), b"write-archives-then-fail").unwrap();

    let lib_dir = base_dir.join("Series");
    fs::create_dir_all(&lib_dir).unwrap();
    let v1 = lib_dir.join("Ep1.mkv");
    let v2 = lib_dir.join("Ep2.mkv");
    fs::write(&v1, b"EP1").unwrap();
    fs::write(&v2, b"EP2").unwrap();

    let entries = vec![make_video_entry(&v1), make_video_entry(&v2)];
    let state = make_test_state(&fake_exe, entries);

    let plan_items = vec![
        DownloadPlanItem {
            video_path: v1.to_string_lossy().to_string(),
            video_file_name: "Ep1.mkv".into(),
        },
        DownloadPlanItem {
            video_path: v2.to_string_lossy().to_string(),
            video_file_name: "Ep2.mkv".into(),
        },
    ];

    let staging_base = base_dir.join("staging_area");
    fs::create_dir_all(&staging_base).unwrap();

    let cancel = Arc::new(AtomicBool::new(false));
    let result = run_download_blocking(
        &state,
        &plan_items,
        Some("valid-api-key".into()),
        Some(&staging_base),
        &cancel,
        &mut |_| {},
    );

    assert!(!result.success);
    assert_eq!(result.exit_code, Some(1));
    assert!(result.message.contains("کد خروج 1"));
    assert_eq!(result.staging_paths.len(), 1);

    let staging_root = PathBuf::from(&result.staging_paths[0]);
    // خروجی جزئی هم باید provenance کامل برای فاز آرشیو داشته باشد
    assert_eq!(
        result.archive_sources.len(),
        2,
        "partial download must preserve archive provenance"
    );
    for source in &result.archive_sources {
        assert_eq!(source.staging_root, result.staging_paths[0]);
        assert_eq!(source.relative_video_dir, "");
        assert!(source.relative_video_path.ends_with(".mkv"));
    }
    // خروجی‌های تولیدشده حتی در خطای جزئی حفظ شده‌اند
    let sub_dir = staging_root.join("sub");
    assert!(sub_dir.is_dir());
    let archives: Vec<_> = fs::read_dir(&sub_dir)
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(!archives.is_empty(), "Partial archives must be preserved");

    // فایل‌های اصلی سالمند
    assert_eq!(fs::read(&v1).unwrap(), b"EP1");
    assert_eq!(fs::read(&v2).unwrap(), b"EP2");

    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۱۴: لغو واقعی فرایند و حفظ خروجی جزئی
#[test]
fn test_real_process_cancellation_preserves_output_and_waits() {
    let fake_exe = get_fake_exe();
    let base_dir = unique_test_dir("cancel-flow");

    // فرایند آرشیوها را می‌نویسد و سپس به صورت کند ادامه می‌دهد تا لغو شود
    fs::write(base_dir.join(".fake_mode"), b"write-then-slow").unwrap();

    let lib_dir = base_dir.join("Series");
    fs::create_dir_all(&lib_dir).unwrap();
    let v1 = lib_dir.join("Ep1.mkv");
    fs::write(&v1, b"EP1_CONTENT").unwrap();

    let entries = vec![make_video_entry(&v1)];
    let state = make_test_state(&fake_exe, entries);

    let plan_items = vec![DownloadPlanItem {
        video_path: v1.to_string_lossy().to_string(),
        video_file_name: "Ep1.mkv".into(),
    }];

    let staging_base = base_dir.join("staging_area");
    fs::create_dir_all(&staging_base).unwrap();

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    let handle = thread::spawn(move || {
        run_download_blocking(
            &state,
            &plan_items,
            Some("key".into()),
            Some(&staging_base),
            &cancel_clone,
            &mut |_| {},
        )
    });

    // مهلت کوتاه تا آرشیوها نوشته شوند و حلقه کند آغاز شود
    thread::sleep(Duration::from_millis(600));
    cancel.store(true, Ordering::SeqCst);

    let result = handle.join().expect("Thread should not panic");

    assert!(result.cancelled, "Result must be marked cancelled");
    assert!(!result.success);
    assert!(result.staging_paths.len() == 1);
    assert_eq!(
        result.archive_sources.len(),
        1,
        "cancelled download must preserve archive provenance"
    );
    assert_eq!(
        result.archive_sources[0].staging_root,
        result.staging_paths[0]
    );
    assert_eq!(result.archive_sources[0].relative_video_path, "Ep1.mkv");

    let staging_root = PathBuf::from(&result.staging_paths[0]);
    // خروجی دانلودشده قبل از لغو حفظ شده است
    let sub_dir = staging_root.join("sub");
    assert!(sub_dir.is_dir());

    // لینک موقت برداشته شده و فایل اصلی سالم است
    assert!(!staging_root.join("Ep1.mkv").exists());
    assert_eq!(fs::read(&v1).unwrap(), b"EP1_CONTENT");

    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۴: محافظت در برابر بازنویسی فایل‌های موجود در دیسک
#[test]
fn test_protection_against_overwriting_existing_files() {
    let base_dir = unique_test_dir("overwrite-protect");

    let src = base_dir.join("source.mkv");
    let existing_target = base_dir.join("existing_target.mkv");
    fs::write(&src, b"SOURCE").unwrap();
    fs::write(&existing_target, b"EXISTING_DO_NOT_TOUCH").unwrap();

    let err = hard_link_no_overwrite(&src, &existing_target).unwrap_err();
    assert!(err.contains("بازنویسی ممنوع"));
    assert_eq!(
        fs::read(&existing_target).unwrap(),
        b"EXISTING_DO_NOT_TOUCH"
    );
    assert_eq!(fs::read(&src).unwrap(), b"SOURCE");

    let _ = fs::remove_dir_all(&base_dir);
}

/// fixture زیرنویس فارسی معتبر برای تست‌های runtime
const FA_SRT: &str = "1\r\n00:00:01,000 --> 00:00:02,000\r\nسلام\r\n";

/// تست ۷: عدم دانلود و عدم بازنویسی در صورت وجود زیرنویس فارسی معتبر کنار ویدیو
#[test]
fn test_skip_videos_with_existing_persian_subtitles() {
    let base_dir = unique_test_dir("skip-fa-sub");

    let v1 = base_dir.join("Show.S01E01.mkv");
    fs::write(&v1, b"VIDEO").unwrap();
    let fa_sub = base_dir.join("Show.S01E01.fa.srt");
    fs::write(&fa_sub, FA_SRT).unwrap();

    let entry = make_video_entry(&v1);

    // ۱) بازبینی تازهٔ فایل‌سیستم: sidecar معتبر فارسی دیده می‌شود
    let inspection = inspect_persian_sidecars(&v1).unwrap();
    assert_eq!(inspection.valid.len(), 1);
    assert!(inspection.conflicts.is_empty());

    // ۲) اعتبارسنجی typed پلن
    let issue = validate_video_typed(&v1.to_string_lossy(), Some(&entry)).unwrap_err();
    assert_eq!(issue.code(), SkipReasonCode::ValidSidecarExists);

    // ۳) بررسی ساخت پلن — کد typed + مسیر valid حفظ می‌شود
    let plan = create_download_plan(&[entry], &[v1.to_string_lossy().to_string()]);
    assert_eq!(plan.items.len(), 0);
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].code, SkipReasonCode::ValidSidecarExists);
    assert!(plan.skipped[0].reason.contains("زیرنویس فارسی"));
    assert_eq!(plan.skipped[0].valid_sidecar_paths, vec![fa_sub.clone()]);

    // زیرنویس موجود به هیچ وجه تغییر نکند
    assert_eq!(fs::read(&fa_sub).unwrap(), FA_SRT.as_bytes());

    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۱۴ و ۱۶ و ۱۸: sidecar معتبرِ ظاهرشده بین پلن و اجرا فقط همان ویدیو را skip
/// می‌کند (کد SidecarAppearedAtExecution)؛ ویدیوی بی‌ارتباط عادی ادامه می‌دهد و
/// فایل sidecar بایت‌به‌بایت دست‌نخورده می‌ماند.
#[test]
fn sidecar_appearing_at_execution_skips_only_that_video() {
    let fake_exe = get_fake_exe();
    let base_dir = unique_test_dir("sidecar-appeared");
    fs::write(base_dir.join(".fake_mode"), b"write-archives").unwrap();

    let work_dir = base_dir.join("Season 01");
    fs::create_dir_all(&work_dir).unwrap();
    let v1 = work_dir.join("Show.S01E01.mkv");
    let v2 = work_dir.join("Show.S01E02.mkv");
    fs::write(&v1, b"VIDEO_1").unwrap();
    fs::write(&v2, b"VIDEO_2").unwrap();

    // پلن هر دو ویدیو — پیش از ظاهر شدن sidecar
    let plan = create_download_plan(
        &[make_video_entry(&v1), make_video_entry(&v2)],
        &[
            v1.to_string_lossy().to_string(),
            v2.to_string_lossy().to_string(),
        ],
    );
    assert_eq!(plan.items.len(), 2);

    // «بعد از پلن» یک sidecar معتبر فارسی برای v1 ظاهر می‌شود
    let fa = work_dir.join("Show.S01E01.fa.srt");
    fs::write(&fa, FA_SRT).unwrap();

    let state = make_test_state(
        &fake_exe,
        vec![make_video_entry(&v1), make_video_entry(&v2)],
    );
    let staging_base = base_dir.join("staging_area");
    fs::create_dir_all(&staging_base).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut progress_events: Vec<OpsProgress> = Vec::new();

    let result = run_download_blocking(
        &state,
        &plan.items,
        Some("test-secret-api-key".into()),
        Some(&staging_base),
        &cancel,
        &mut |p| progress_events.push(p),
    );

    // فقط v1 با کد typed skip می‌شود؛ v2 عادی ادامه می‌دهد
    assert_eq!(result.skipped.len(), 1, "{:?}", result.skipped);
    assert_eq!(
        result.skipped[0].code,
        SkipReasonCode::SidecarAppearedAtExecution
    );
    assert_eq!(
        result.skipped[0].video_path,
        v1.to_string_lossy().to_string()
    );
    assert_eq!(
        result.skipped[0].valid_sidecar_paths,
        vec![fa.to_string_lossy().to_string()]
    );
    assert_ne!(result.skipped[0].code, SkipReasonCode::Unknown);
    assert_eq!(result.planned, 1);
    assert_eq!(result.staged, 1);
    assert!(result.success, "{}", result.message);

    // sidecar بایت‌به‌بایت دست‌نخورده
    assert_eq!(fs::read(&fa).unwrap(), FA_SRT.as_bytes());
    // staging فقط برای v2 ساخته شده و آرشیو دانلودشدهٔ آن حفظ شده است
    // (لینک‌های ویدیو پس از اجرا پاک می‌شوند؛ فقط sub/ می‌ماند)
    let staging_root = PathBuf::from(&result.staging_paths[0]);
    fn has_archive(dir: &Path, needle: &str) -> bool {
        let mut found = false;
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_dir() {
                    found = found || has_archive(&path, needle);
                } else if entry.file_name().to_string_lossy().contains(needle) {
                    found = true;
                }
            }
        }
        found
    }
    assert!(
        has_archive(&staging_root, "S01E02"),
        "آرشیو v2 باید حفظ شده باشد"
    );
    assert!(
        !has_archive(&staging_root, "S01E01"),
        "برای v1 هیچ دانلودی نباید باشد"
    );

    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۱۵ و ۱۶ و ۱۸: sidecar فارسی‌نامِ نامعتبرِ ظاهرشده در اجرا → تعارض typed؛
/// ویدیوی بی‌ارتباط ادامه می‌دهد؛ فایل تعارض حفظ می‌شود.
#[test]
fn invalid_sidecar_at_execution_is_typed_conflict() {
    let fake_exe = get_fake_exe();
    let base_dir = unique_test_dir("sidecar-conflict");
    fs::write(base_dir.join(".fake_mode"), b"write-archives").unwrap();

    let work_dir = base_dir.join("Season 01");
    fs::create_dir_all(&work_dir).unwrap();
    let v1 = work_dir.join("Show.S01E01.mkv");
    let v2 = work_dir.join("Show.S01E02.mkv");
    fs::write(&v1, b"VIDEO_1").unwrap();
    fs::write(&v2, b"VIDEO_2").unwrap();

    let plan = create_download_plan(
        &[make_video_entry(&v1), make_video_entry(&v2)],
        &[
            v1.to_string_lossy().to_string(),
            v2.to_string_lossy().to_string(),
        ],
    );
    assert_eq!(plan.items.len(), 2);

    // «بعد از پلن» یک sidecar فارسی‌نامِ خراب ظاهر می‌شود
    let fa = work_dir.join("Show.S01E01.fa.srt");
    fs::write(&fa, b"GARBAGE_NOT_A_SUBTITLE").unwrap();

    let state = make_test_state(
        &fake_exe,
        vec![make_video_entry(&v1), make_video_entry(&v2)],
    );
    let staging_base = base_dir.join("staging_area");
    fs::create_dir_all(&staging_base).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));

    let result = run_download_blocking(
        &state,
        &plan.items,
        Some("test-secret-api-key".into()),
        Some(&staging_base),
        &cancel,
        &mut |_| {},
    );

    assert_eq!(result.skipped.len(), 1, "{:?}", result.skipped);
    assert_eq!(result.skipped[0].code, SkipReasonCode::SidecarConflict);
    assert_ne!(result.skipped[0].code, SkipReasonCode::Unknown);
    let conflicts = &result.skipped[0].sidecar_conflicts;
    assert_eq!(conflicts.len(), 1);
    assert!(conflicts[0].reason.contains("ساختار"));
    assert_eq!(result.planned, 1);
    assert_eq!(result.staged, 1);

    // فایل تعارض حفظ شده
    assert_eq!(fs::read(&fa).unwrap(), b"GARBAGE_NOT_A_SUBTITLE");

    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۱۷: ویدیویی که بین پلن و اجرا غیب می‌شود → MissingAtExecution برای همان
/// ویدیو؛ بقیه عادی ادامه می‌دهند.
#[test]
fn missing_video_at_execution_reports_missing_at_execution() {
    let fake_exe = get_fake_exe();
    let base_dir = unique_test_dir("missing-at-exec");
    fs::write(base_dir.join(".fake_mode"), b"write-archives").unwrap();

    let work_dir = base_dir.join("Season 01");
    fs::create_dir_all(&work_dir).unwrap();
    let v1 = work_dir.join("Show.S01E01.mkv");
    let v2 = work_dir.join("Show.S01E02.mkv");
    fs::write(&v1, b"VIDEO_1").unwrap();
    fs::write(&v2, b"VIDEO_2").unwrap();

    let plan = create_download_plan(
        &[make_video_entry(&v1), make_video_entry(&v2)],
        &[
            v1.to_string_lossy().to_string(),
            v2.to_string_lossy().to_string(),
        ],
    );
    assert_eq!(plan.items.len(), 2);

    // «بعد از پلن» v1 حذف می‌شود
    fs::remove_file(&v1).unwrap();

    let state = make_test_state(
        &fake_exe,
        vec![make_video_entry(&v1), make_video_entry(&v2)],
    );
    let staging_base = base_dir.join("staging_area");
    fs::create_dir_all(&staging_base).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));

    let result = run_download_blocking(
        &state,
        &plan.items,
        Some("test-secret-api-key".into()),
        Some(&staging_base),
        &cancel,
        &mut |_| {},
    );

    assert_eq!(result.skipped.len(), 1, "{:?}", result.skipped);
    assert_eq!(result.skipped[0].code, SkipReasonCode::MissingAtExecution);
    assert_eq!(
        result.skipped[0].video_path,
        v1.to_string_lossy().to_string()
    );
    assert_eq!(result.planned, 1);
    assert_eq!(result.staged, 1);

    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۵ و ۶: رد ویدیوهای ناموجود، خارج از کتابخانه و مسیرهای تکراری
#[test]
fn test_rejection_of_missing_and_duplicate_videos() {
    let base_dir = unique_test_dir("missing-dup");

    let v1 = base_dir.join("Show.S01E01.mkv");
    fs::write(&v1, b"VIDEO").unwrap();
    let entry1 = make_video_entry(&v1);

    let nonexistent = base_dir
        .join("Nonexistent.mkv")
        .to_string_lossy()
        .to_string();
    let v1_str = v1.to_string_lossy().to_string();

    let plan = create_download_plan(&[entry1], &[v1_str.clone(), v1_str.clone(), nonexistent]);

    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.skipped.len(), 2);
    assert!(plan.skipped.iter().any(|s| s.reason.contains("تکراری")));
    assert!(plan.skipped.iter().any(|s| s.reason.contains("کتابخانه")));

    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۸: Rollback کامل در صورت بروز خطا در ساخت Staging
#[test]
fn test_staging_build_failure_rolls_back_cleanly() {
    let base_dir = unique_test_dir("rollback-clean");

    let v1 = base_dir.join("valid.mkv");
    fs::write(&v1, b"VALID").unwrap();

    let placements = vec![
        StagedPlacement {
            source: v1.clone(),
            rel_dir: PathBuf::from(""),
            file_name: "valid.mkv".into(),
        },
        StagedPlacement {
            source: base_dir.join("nonexistent_source.mkv"),
            rel_dir: PathBuf::from(""),
            file_name: "nonexistent.mkv".into(),
        },
    ];

    let root = base_dir.join(format!("{STAGING_DIR_PREFIX}rollback_test"));
    let err = build(root.clone(), &placements, None, &mut |_, _, _| {}).unwrap_err();

    assert!(err.message.contains("hard link"));
    assert!(err.cleanup.is_success());
    assert!(err.session.is_none());
    assert!(
        !root.exists(),
        "Staging root must be completely removed on failure"
    );
    assert_eq!(
        fs::read(&v1).unwrap(),
        b"VALID",
        "Original file must be untouched"
    );

    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۱۵: جلوگیری از تداخل عملیات گروهی با عملیات تکی و سایر عملیات‌های گروهی
#[test]
fn test_batch_operation_concurrency_lock() {
    let reg = Arc::new(Mutex::new(OpsRegistry::default()));

    // ۱) رزرو گروهی
    let guard = acquire_batch_op(&reg).expect("Batch op acquire should succeed");

    // ۲) تلاش برای رزرو تکی یا گروهی دیگر باید رد شود
    assert!(acquire_single_op(&reg, "video-path").is_err());
    assert!(acquire_batch_op(&reg).is_err());

    // ۳) آزادسازی اسلات
    drop(guard);

    // ۴) اکنون هر دو قابل دریافت هستند
    assert!(acquire_single_op(&reg, "video-path").is_ok());
}

/// تست ۱۶: عدم وجود API Key مانع از هرگونه تغییر یا ساخت پوشه روی دیسک می‌شود
#[test]
fn test_missing_api_key_aborts_before_filesystem_changes() {
    let fake_exe = get_fake_exe();
    let base_dir = unique_test_dir("no-api-key");

    let v1 = base_dir.join("Ep1.mkv");
    fs::write(&v1, b"EP1").unwrap();
    let entries = vec![make_video_entry(&v1)];
    let state = make_test_state(&fake_exe, entries);

    let plan_items = vec![DownloadPlanItem {
        video_path: v1.to_string_lossy().to_string(),
        video_file_name: "Ep1.mkv".into(),
    }];

    let staging_base = base_dir.join("staging_area");
    let cancel = Arc::new(AtomicBool::new(false));

    // ارسال None به عنوان API key
    let result = run_download_blocking(
        &state,
        &plan_items,
        None,
        Some(&staging_base),
        &cancel,
        &mut |_| {},
    );

    assert!(!result.success);
    assert!(result.message.contains("API key"));
    assert_eq!(result.staged, 0);
    assert!(result.staging_paths.is_empty());
    assert!(
        !staging_base.exists(),
        "Staging directory must not be created"
    );

    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۱۷: تشخیص ریشه volume و رد مسیرهای نامعتبر/نسبی
#[test]
fn test_volume_root_and_invalid_path_rejection() {
    #[cfg(windows)]
    {
        assert_eq!(
            volume_root_of(Path::new("C:\\Videos\\Movie.mkv")),
            Some(PathBuf::from("C:\\"))
        );
        assert_eq!(
            volume_root_of(Path::new("D:\\Media\\Series\\S01\\Ep.mkv")),
            Some(PathBuf::from("D:\\"))
        );
    }
    assert_eq!(volume_root_of(Path::new("relative\\path.mkv")), None);

    // بررسی رفتار group_by_volume در صورت مسیر نسبی
    let relative_item = vec![DownloadPlanItem {
        video_path: "relative\\Show.mkv".into(),
        video_file_name: "Show.mkv".into(),
    }];
    let (groups, skipped) = group_by_volume(&relative_item, None);
    assert_eq!(groups.len(), 0);
    assert_eq!(skipped.len(), 1);
    assert!(skipped[0].reason.contains("نسبی"));
}

/// تست ۱۸: عدم موفقیت در حذف لینک‌ها منجر به هشدار پاک‌سازی، عدم گزارش موفقیت کامل،
/// و حفظ مسیر staging برای بازیابی دستی می‌شود
#[test]
fn test_cleanup_failure_propagates_to_download_result_and_preserves_staging() {
    let fake_exe = get_fake_exe();
    let base_dir = unique_test_dir("cleanup-fail-propagate");

    fs::write(base_dir.join(".fake_mode"), b"write-archives").unwrap();

    let lib_dir = base_dir.join("Series");
    fs::create_dir_all(&lib_dir).unwrap();
    let v1 = lib_dir.join("Ep1.mkv");
    let v2 = lib_dir.join("Ep2.mkv");
    fs::write(&v1, b"EP1").unwrap();
    fs::write(&v2, b"EP2").unwrap();

    let entries = vec![make_video_entry(&v1), make_video_entry(&v2)];
    let state = make_test_state(&fake_exe, entries);

    let plan_items = vec![
        DownloadPlanItem {
            video_path: v1.to_string_lossy().to_string(),
            video_file_name: "Ep1.mkv".into(),
        },
        DownloadPlanItem {
            video_path: v2.to_string_lossy().to_string(),
            video_file_name: "Ep2.mkv".into(),
        },
    ];

    let staging_base = base_dir.join("staging_area");
    fs::create_dir_all(&staging_base).unwrap();

    let cancel = Arc::new(AtomicBool::new(false));

    // شبیه‌سازی شکست حذف برای Ep2.mkv در cleanup
    let result = run_download_blocking_with_remover(
        &state,
        &plan_items,
        Some("valid-api-key".into()),
        Some(&staging_base),
        &cancel,
        &mut |_| {},
        |p| {
            if p.to_string_lossy().contains("Ep2.mkv") {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "دسترسی شبیه‌سازی‌شده ناموفق در پاک‌سازی",
                ))
            } else {
                std::fs::remove_file(p)
            }
        },
    );

    // ۱) عملیات نباید کاملاً موفق گزارش شود
    assert!(
        !result.success,
        "عملیات با شکست پاک‌سازی نباید موفق گزارش شود"
    );
    assert!(!result.cancelled);
    assert_eq!(result.planned, 2);
    assert_eq!(result.staged, 2);

    // ۲) هشدارهای پاک‌سازی صریح و همراه با مسیر فایل باشد
    assert_eq!(result.cleanup_warnings.len(), 1);
    assert!(result.cleanup_warnings[0].contains("Ep2.mkv"));
    assert!(result.cleanup_warnings[0].contains("پاک‌سازی دستی"));

    // ۳) مسیر staging برای بازیابی دستی حفظ شده باشد
    assert_eq!(result.staging_paths.len(), 1);
    let staging_root = PathBuf::from(&result.staging_paths[0]);
    assert!(staging_root.is_dir());
    assert!(
        staging_root.join("Ep2.mkv").exists(),
        "لینک باقیمانده باید روی دیسک بماند"
    );

    // ۴) لینک Ep1 باید پاک شده باشد
    assert!(!staging_root.join("Ep1.mkv").exists());

    // ۵) پیام خروجی به کاربر حاوی هشدار پاک‌سازی باشد
    assert!(result.message.contains("پاک‌سازی"));

    // پاک‌سازی نهایی برای پایان تست
    let _ = std::fs::remove_file(staging_root.join("Ep2.mkv"));
    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۱۹: شکست ساخت staging (شکست در لینک دوم) و شکست rollback (شبیه‌سازی خطای حذف لینک اول)
/// بررسی می‌کند که خطا در سطح ارکستریشن به طور کامل به DownloadResult منتقل شده،
/// لینک اول روی دیسک باقی مانده، مسیر staging حفظ شده و پیام نهایی ادعای rollback کامل نمی‌کند.
#[test]
fn test_staging_build_failure_with_failed_rollback_preserves_ownership_and_reports_warnings() {
    let fake_exe = get_fake_exe();
    let base_dir = unique_test_dir("build-fail-rollback-fail");

    let lib_dir = base_dir.join("Series");
    fs::create_dir_all(&lib_dir).unwrap();
    let v1 = lib_dir.join("Ep1.mkv");
    let v2 = lib_dir.join("Ep2.mkv");
    fs::write(&v1, b"EP1_CONTENT").unwrap();
    fs::write(&v2, b"EP2_CONTENT").unwrap();

    let entries = vec![make_video_entry(&v1), make_video_entry(&v2)];
    let state = make_test_state(&fake_exe, entries);

    let plan_items = vec![
        DownloadPlanItem {
            video_path: v1.to_string_lossy().to_string(),
            video_file_name: "Ep1.mkv".into(),
        },
        DownloadPlanItem {
            video_path: v2.to_string_lossy().to_string(),
            video_file_name: "Ep2.mkv".into(),
        },
    ];

    let staging_base = base_dir.join("staging_area");
    fs::create_dir_all(&staging_base).unwrap();

    let cancel = Arc::new(AtomicBool::new(false));

    // در حین پیشرفت لینک اول، فایل v2 را از دیسک حذف می‌کنیم تا ساخت لینک دوم در staging شکست بخورد
    let v2_to_delete = v2.clone();
    let mut on_progress = move |p: OpsProgress| {
        if p.message.contains("Ep1.mkv") {
            let _ = fs::remove_file(&v2_to_delete);
        }
    };

    // در زمان rollback لینک‌های ساخته‌شده (Ep1.mkv)، خطای PermissionDenied شبیه‌سازی می‌کنیم
    let result = run_download_blocking_with_remover(
        &state,
        &plan_items,
        Some("valid-api-key".into()),
        Some(&staging_base),
        &cancel,
        &mut on_progress,
        |p| {
            if p.to_string_lossy().contains("Ep1.mkv") {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "دسترسی شبیه‌سازی‌شده ناموفق در rollback ساخت staging",
                ))
            } else {
                std::fs::remove_file(p)
            }
        },
    );

    // ۱) عملیات نباید موفق گزارش شود
    assert!(!result.success, "عملیات نباید موفق باشد");
    assert_eq!(result.staged, 0, "هیچ مرحله دانلودی نباید کامل شده باشد");

    // ۲) فایل ویدیوی اصلی دست‌نخورده و سالم بماند
    assert_eq!(fs::read(&v1).unwrap(), b"EP1_CONTENT");

    // ۳) لینک موقت اول حذف‌نشده و روی دیسک باقی مانده باشد
    assert_eq!(
        result.staging_paths.len(),
        1,
        "مسیر ریشه staging برای بازیابی دستی باید حفظ شود"
    );
    let staging_root = PathBuf::from(&result.staging_paths[0]);
    assert!(staging_root.is_dir());
    let leftover_link = staging_root.join("Ep1.mkv");
    assert!(
        leftover_link.exists(),
        "لینک اول که rollback آن شکست خورد باید روی دیسک بماند"
    );

    // ۴) cleanup_warnings شامل مسیر لینک و دلیل شکست باشد
    assert_eq!(result.cleanup_warnings.len(), 1);
    assert!(result.cleanup_warnings[0].contains("Ep1.mkv"));
    assert!(result.cleanup_warnings[0].contains("rollback"));
    assert!(result.cleanup_warnings[0].contains("دسترسی"));

    // ۵) پیام نهایی ادعای rollback کامل نکند
    assert!(
        !result.message.contains("برگردانده شد") && !result.message.contains("کامل"),
        "پیام نباید ادعای بازگردانی کامل کند: {}",
        result.message
    );
    assert!(
        result.message.contains("ناقص") || result.message.contains("پاک‌سازی"),
        "پیام باید هشدار پاک‌سازی ناقص دهد: {}",
        result.message
    );

    // پاک‌سازی نهایی
    let _ = fs::remove_file(leftover_link);
    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۲۰: لغو بلافاصله پس از ساخت staging به همراه شکست در rollback
/// بررسی می‌کند که مسیرهای باقیمانده حفظ شده، cleanup_warnings ثبت شده و پیام ادعای بازگردانی کامل نکند.
#[test]
fn test_cancellation_after_staging_with_failed_rollback() {
    let fake_exe = get_fake_exe();
    let base_dir = unique_test_dir("cancel-rb-fail");

    let lib_dir = base_dir.join("Series");
    fs::create_dir_all(&lib_dir).unwrap();
    let v1 = lib_dir.join("Ep1.mkv");
    fs::write(&v1, b"EP1").unwrap();

    let entries = vec![make_video_entry(&v1)];
    let state = make_test_state(&fake_exe, entries);

    let plan_items = vec![DownloadPlanItem {
        video_path: v1.to_string_lossy().to_string(),
        video_file_name: "Ep1.mkv".into(),
    }];

    let staging_base = base_dir.join("staging_area");
    fs::create_dir_all(&staging_base).unwrap();

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    // فعال کردن لغو بلافاصله پس از اتمام ساخت staging
    let mut on_progress = move |p: OpsProgress| {
        if p.message.contains("لینک") {
            cancel_clone.store(true, Ordering::SeqCst);
        }
    };

    let result = run_download_blocking_with_remover(
        &state,
        &plan_items,
        Some("valid-api-key".into()),
        Some(&staging_base),
        &cancel,
        &mut on_progress,
        |p| {
            if p.to_string_lossy().contains("Ep1.mkv") {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "دسترسی شبیه‌سازی‌شده ناموفق در rollback لغو",
                ))
            } else {
                std::fs::remove_file(p)
            }
        },
    );

    assert!(result.cancelled);
    assert!(!result.success);
    assert_eq!(result.staging_paths.len(), 1);
    let staging_root = PathBuf::from(&result.staging_paths[0]);
    assert!(staging_root.join("Ep1.mkv").exists());
    assert_eq!(result.cleanup_warnings.len(), 1);
    assert!(result.cleanup_warnings[0].contains("Ep1.mkv"));
    assert!(!result.message.contains("برگردانده شد"));
    assert!(result.message.contains("پاک‌سازی"));

    let _ = fs::remove_file(staging_root.join("Ep1.mkv"));
    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۲۱: خطای حذف پوشه در `cleanup_keep_output` باید در
/// `DownloadResult.cleanup_warnings`، پیام نهایی و payload رویداد
/// `subsource:done` منتشر شود؛ آرشیوهای دانلودشده و ویدیوهای اصلی حفظ شوند.
#[test]
fn test_cleanup_directory_failure_propagates_to_result_and_done_payload() {
    let fake_exe = get_fake_exe();
    let base_dir = unique_test_dir("cleanup-dir-fail-payload");

    fs::write(base_dir.join(".fake_mode"), b"write-archives").unwrap();

    let series = base_dir.join("Series");
    let s1_dir = series.join("Season 01");
    let s2_dir = series.join("Season 02");
    fs::create_dir_all(&s1_dir).unwrap();
    fs::create_dir_all(&s2_dir).unwrap();
    let v1 = s1_dir.join("Ep1.mkv");
    let v2 = s2_dir.join("Ep2.mkv");
    fs::write(&v1, b"ORIGINAL_EP1").unwrap();
    fs::write(&v2, b"ORIGINAL_EP2").unwrap();

    let entries = vec![make_video_entry(&v1), make_video_entry(&v2)];
    let state = make_test_state(&fake_exe, entries);

    let plan_items = vec![
        DownloadPlanItem {
            video_path: v1.to_string_lossy().to_string(),
            video_file_name: "Ep1.mkv".into(),
        },
        DownloadPlanItem {
            video_path: v2.to_string_lossy().to_string(),
            video_file_name: "Ep2.mkv".into(),
        },
    ];

    let staging_base = base_dir.join("staging_area");
    fs::create_dir_all(&staging_base).unwrap();

    let cancel = Arc::new(AtomicBool::new(false));

    // فقط حذف پوشهٔ Season 02 در staging شکست می‌خورد؛ لینک‌ها عادی حذف می‌شوند.
    let result = run_download_blocking_with_removers(
        &state,
        &plan_items,
        Some("test-secret-api-key".into()),
        Some(&staging_base),
        &cancel,
        &mut |_| {},
        |p| std::fs::remove_file(p),
        |p| {
            if p.ends_with("Season 02") {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "خطای شبیه‌سازی‌شده حذف پوشه در cleanup",
                ))
            } else {
                std::fs::remove_dir(p)
            }
        },
    );

    // ۱) نتیجه نباید موفق گزارش شود و هشدار پوشه باید ثبت شود
    assert!(!result.success, "خطای حذف پوشه نباید موفقیت کامل گزارش شود");
    assert!(!result.cancelled);
    assert_eq!(result.planned, 2);
    assert_eq!(result.staged, 2);
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.cleanup_warnings.len(), 1);
    assert!(
        result.cleanup_warnings[0].contains("پوشه"),
        "{}",
        result.cleanup_warnings[0]
    );
    assert!(
        result.cleanup_warnings[0].contains("Season 02"),
        "{}",
        result.cleanup_warnings[0]
    );
    assert!(
        result.cleanup_warnings[0].contains("شبیه‌سازی‌شده"),
        "{}",
        result.cleanup_warnings[0]
    );

    // ۲) پیام نهایی باید شکست پاک‌سازی را صادقانه گزارش کند
    assert!(result.message.contains("پاک‌سازی"), "{}", result.message);
    assert!(result.message.contains('1'), "{}", result.message);

    // ۳) مسیر staging و همهٔ آرشیوهای دانلودشده باید حفظ شوند
    assert_eq!(result.staging_paths.len(), 1);
    let staging_root = PathBuf::from(&result.staging_paths[0]);
    assert!(staging_root.is_dir());
    let staged_s1 = staging_root.join("Season 01");
    let staged_s2 = staging_root.join("Season 02");
    assert!(staged_s1.is_dir());
    assert!(staged_s2.is_dir(), "پوشهٔ ناموفق باید حفظ شود");

    for season_dir in [&staged_s1, &staged_s2] {
        let sub_dir = season_dir.join("sub");
        assert!(sub_dir.is_dir(), "پوشهٔ sub باید حفظ شود: {sub_dir:?}");
        let archives: Vec<_> = fs::read_dir(&sub_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().map(|x| x == "zip").unwrap_or(false))
            .collect();
        assert_eq!(
            archives.len(),
            1,
            "آرشیو دانلودشده باید حفظ شود: {sub_dir:?}"
        );
    }

    // لینک‌های موقت ویدیو حذف شده‌اند
    assert!(!staged_s1.join("Ep1.mkv").exists());
    assert!(!staged_s2.join("Ep2.mkv").exists());

    // ۴) ویدیوهای اصلی کاملاً دست‌نخورده‌اند
    assert_eq!(fs::read(&v1).unwrap(), b"ORIGINAL_EP1");
    assert_eq!(fs::read(&v2).unwrap(), b"ORIGINAL_EP2");

    // ۵) payload رویداد `subsource:done` کامل و بدون راز است
    assert_eq!(SUBSOURCE_DONE_EVENT, "subsource:done");
    let payload = download_done_payload(&result);
    assert_eq!(payload["success"], serde_json::json!(false));
    assert_eq!(payload["cancelled"], serde_json::json!(false));
    assert_eq!(payload["planned"], serde_json::json!(2));
    assert_eq!(payload["staged"], serde_json::json!(2));
    assert_eq!(payload["exitCode"], serde_json::json!(0));
    assert_eq!(
        payload["stagingPaths"][0],
        serde_json::json!(staging_root.to_string_lossy())
    );
    let warnings = payload["cleanupWarnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].as_str().unwrap().contains("Season 02"));
    assert!(payload["message"].as_str().unwrap().contains("پاک‌سازی"));
    assert!(payload["skipped"].is_array());
    let serialized = payload.to_string();
    assert!(
        !serialized.contains("test-secret-api-key"),
        "payload نباید API key داشته باشد"
    );
    assert!(payload.get("apiKey").is_none());

    let _ = fs::remove_dir_all(&base_dir);
}

/// تست ۲۲: خطای حذف پوشه در rollback ساخت staging باید از مسیر BuildFailed
/// به `DownloadResult.cleanup_warnings` و payload نهایی منتقل شود و ادعای
/// rollback کامل نکند.
#[test]
fn test_staging_build_directory_rollback_failure_propagates_to_result() {
    let fake_exe = get_fake_exe();
    let base_dir = unique_test_dir("build-dir-rb-fail-propagate");

    let lib_dir = base_dir.join("Series");
    fs::create_dir_all(&lib_dir).unwrap();
    let v1 = lib_dir.join("Ep1.mkv");
    let v2 = lib_dir.join("Ep2.mkv");
    fs::write(&v1, b"EP1_CONTENT").unwrap();
    fs::write(&v2, b"EP2_CONTENT").unwrap();

    let entries = vec![make_video_entry(&v1), make_video_entry(&v2)];
    let state = make_test_state(&fake_exe, entries);

    let plan_items = vec![
        DownloadPlanItem {
            video_path: v1.to_string_lossy().to_string(),
            video_file_name: "Ep1.mkv".into(),
        },
        DownloadPlanItem {
            video_path: v2.to_string_lossy().to_string(),
            video_file_name: "Ep2.mkv".into(),
        },
    ];

    let staging_base = base_dir.join("staging_area");
    fs::create_dir_all(&staging_base).unwrap();

    let cancel = Arc::new(AtomicBool::new(false));
    let v2_to_delete = v2.clone();
    let mut on_progress = move |p: OpsProgress| {
        // پس از لینک اول، منبع دوم حذف می‌شود تا ساخت لینک دوم شکست بخورد
        if p.message.contains("Ep1.mkv") {
            let _ = fs::remove_file(&v2_to_delete);
        }
    };

    // حذف لینک‌ها موفق است، اما حذف ریشهٔ staging در rollback شکست می‌خورد
    let result = run_download_blocking_with_removers(
        &state,
        &plan_items,
        Some("valid-api-key".into()),
        Some(&staging_base),
        &cancel,
        &mut on_progress,
        |p| std::fs::remove_file(p),
        |p| {
            let is_staging_root = p
                .file_name()
                .map(|n| n.to_string_lossy().starts_with(STAGING_DIR_PREFIX))
                .unwrap_or(false);
            if is_staging_root {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "خطای شبیه‌سازی‌شده حذف ریشه در rollback ساخت",
                ))
            } else {
                std::fs::remove_dir(p)
            }
        },
    );

    assert!(!result.success, "rollback ناقص نباید موفق گزارش شود");
    assert_eq!(result.staged, 0);
    assert_eq!(result.staging_paths.len(), 1);
    assert_eq!(result.cleanup_warnings.len(), 1);
    assert!(
        result.cleanup_warnings[0].contains("پوشه"),
        "{}",
        result.cleanup_warnings[0]
    );
    assert!(
        result.cleanup_warnings[0].contains("rollback ساخت staging"),
        "{}",
        result.cleanup_warnings[0]
    );

    // پیام نهایی باید ناقص بودن پاک‌سازی را صادقانه بگوید
    assert!(
        !result.message.contains("برگردانده شد"),
        "{}",
        result.message
    );
    assert!(result.message.contains("ناقص"), "{}", result.message);
    assert!(result.message.contains('1'), "{}", result.message);

    let staging_root = PathBuf::from(&result.staging_paths[0]);
    assert!(staging_root.is_dir(), "ریشهٔ ناموفق باید حفظ شود");
    assert!(
        !staging_root.join("Ep1.mkv").exists(),
        "لینک اول باید حذف شده باشد"
    );
    assert_eq!(fs::read(&v1).unwrap(), b"EP1_CONTENT");

    let payload = download_done_payload(&result);
    let warnings = payload["cleanupWarnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].as_str().unwrap().contains("پوشه"));
    assert_eq!(
        payload["stagingPaths"][0],
        serde_json::json!(staging_root.to_string_lossy())
    );
    assert_eq!(payload["staged"], serde_json::json!(0));

    let _ = fs::remove_dir_all(&base_dir);
}
