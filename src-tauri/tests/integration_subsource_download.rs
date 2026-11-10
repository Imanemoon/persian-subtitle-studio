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
    run_download_blocking, run_download_blocking_with_remover,
};
use subsync_studio_lib::models::{OpsProgress, VideoEntry};
use subsync_studio_lib::settings::Settings;
use subsync_studio_lib::state::{acquire_batch_op, acquire_single_op, AppState, OpsRegistry};
use subsync_studio_lib::subsource::plan::{
    create_download_plan, has_final_persian_sidecar, validate_video, DownloadPlanItem,
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
        scanning: AtomicBool::new(false),
        cancel_scan: AtomicBool::new(false),
        ops: Arc::new(Mutex::new(OpsRegistry::default())),
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

/// تست ۷: عدم دانلود و عدم بازنویسی در صورت وجود زیرنویس فارسی کنار ویدیو
#[test]
fn test_skip_videos_with_existing_persian_subtitles() {
    let base_dir = unique_test_dir("skip-fa-sub");

    let v1 = base_dir.join("Show.S01E01.mkv");
    fs::write(&v1, b"VIDEO").unwrap();
    let fa_sub = base_dir.join("Show.S01E01.fa.srt");
    fs::write(&fa_sub, b"EXISTING_PERSIAN_SUBTITLE").unwrap();

    let entry = make_video_entry(&v1);

    // ۱) بررسی تابع اختصاصی تشخیص
    assert!(has_final_persian_sidecar(&v1, Some(&entry)));

    // ۲) بررسی اعتبارسنجی پلن
    let err = validate_video(&v1.to_string_lossy(), Some(&entry)).unwrap_err();
    assert!(err.contains("زیرنویس فارسی"));

    // ۳) بررسی ساخت پلن
    let plan = create_download_plan(&[entry], &[v1.to_string_lossy().to_string()]);
    assert_eq!(plan.items.len(), 0);
    assert_eq!(plan.skipped.len(), 1);
    assert!(plan.skipped[0].reason.contains("زیرنویس فارسی"));

    // زیرنویس موجود به هیچ وجه تغییر نکند
    assert_eq!(fs::read(&fa_sub).unwrap(), b"EXISTING_PERSIAN_SUBTITLE");

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
    assert!(reg.lock().unwrap().start_batch().is_err());

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
