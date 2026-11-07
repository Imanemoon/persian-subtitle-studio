//! تست یکپارچه هسته فاز ۲ با فایل رسانه‌ای مصنوعی.
//!
//! هسته analyze/extract/review/promote/plan — کاملاً مستقل از Tauri — با
//! ffprobe/ffmpeg واقعی این سیستم آزمایش می‌شود. هیچ رسانه‌ای داخل ریپو نیست.
//!
//! اجرا: `cargo test --lib -- --ignored`

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use super::media::{get_entry, run_analyze_blocking};
use super::subtitle::{
    execute_plan_items, plan_extraction_from_library, run_extract_blocking, run_promote_blocking,
    run_review_blocking,
};
use crate::models::{AnalysisStatus, OpsProgress, SubtitleKind, VideoEntry};
use crate::settings::Settings;
use crate::state::{AppState, OpsRegistry};
use crate::tools::resolve_tool_path;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "subsync-cmdtest-{tag}-{}-{nanos}",
        std::process::id()
    ))
}

fn run_ffmpeg(args: &[&str], outputs: &[&Path]) {
    let ffmpeg = resolve_tool_path("ffmpeg", &Default::default()).expect("ffmpeg در PATH");
    let mut cmd = Command::new(&ffmpeg);
    cmd.args(args);
    for o in outputs {
        cmd.arg(o);
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().expect("اجرای ffmpeg");
    assert!(
        out.status.success(),
        "ffmpeg ناموفق\nargs: {args:?}\nstderr tail: {}",
        {
            let text = String::from_utf8_lossy(&out.stderr);
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            let start = lines.len().saturating_sub(8);
            lines[start..].join("\n")
        }
    );
}

fn write_fixtures(dir: &Path, name: &str) -> (PathBuf, PathBuf) {
    let fa = dir.join(format!("{name}.fa-fixture.srt"));
    std::fs::write(
        &fa,
        "1\n00:00:00,200 --> 00:00:00,800\nسلام دنیا\n\n2\n00:00:00,850 --> 00:00:00,990\nخط دوم\n",
    )
    .unwrap();
    let eng = dir.join(format!("{name}.eng-fixture.srt"));
    std::fs::write(&eng, "1\n00:00:00,200 --> 00:00:00,800\nhello world\n").unwrap();
    (fa, eng)
}

/// ساخت MKV مصنوعی با Trackهای متنی دلخواه:
/// هر عنصر: (language یا "" برای بدون tag، title یا ""، محتوا "fa"|"en")
fn make_video_meta(dir: &Path, name: &str, subs: &[(&str, &str, &str)]) -> PathBuf {
    let (fa_fixture, eng_fixture) = write_fixtures(dir, name);
    let video = dir.join(name);

    let mut args: Vec<String> = vec![
        "-y".into(),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "color=c=black:s=320x240:d=1".into(),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "sine=d=1".into(),
    ];
    let mut maps: Vec<String> = vec!["0:v".into(), "1:a".into()];
    for (i, (_, _, content)) in subs.iter().enumerate() {
        let fixture = if *content == "fa" {
            &fa_fixture
        } else {
            &eng_fixture
        };
        args.push("-i".into());
        args.push(fixture.to_string_lossy().into_owned());
        maps.push(format!("{}:s", 2 + i));
    }
    for (i, (lang, title, _)) in subs.iter().enumerate() {
        if !lang.is_empty() {
            args.push(format!("-metadata:s:s:{i}"));
            args.push(format!("language={lang}"));
        }
        if !title.is_empty() {
            args.push(format!("-metadata:s:s:{i}"));
            args.push(format!("title={title}"));
        }
    }
    // mapها بعد از همه ورودی‌ها و metadataها
    for m in &maps {
        args.push("-map".into());
        args.push(m.clone());
    }
    args.extend([
        "-c:v".into(),
        "mpeg4".into(),
        "-q:v".into(),
        "7".into(),
        "-c:a".into(),
        "aac".into(),
        "-c:s".into(),
        "srt".into(),
    ]);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_ffmpeg(&arg_refs, &[&video]);
    video
}

/// نسخه سازگار با تست‌های قبلی: fa_tracks عدد + eng_tracks عدد
fn make_video(dir: &Path, name: &str, fa_tracks: usize, eng_tracks: usize) -> PathBuf {
    let mut subs: Vec<(&str, &str, &str)> = Vec::new();
    for _ in 0..fa_tracks {
        subs.push(("fas", "فارسی", "fa"));
    }
    for _ in 0..eng_tracks {
        subs.push(("eng", "English", "en"));
    }
    make_video_meta(dir, name, &subs)
}

fn make_state() -> AppState {
    AppState {
        settings: std::sync::Mutex::new(Settings::default()),
        library: std::sync::Mutex::new(Vec::new()),
        last_scan: std::sync::Mutex::new(None),
        scanning: std::sync::atomic::AtomicBool::new(false),
        cancel_scan: std::sync::atomic::AtomicBool::new(false),
        ops: std::sync::Mutex::new(OpsRegistry::default()),
    }
}

fn push_entry(state: &AppState, video: &Path) -> String {
    let path_str = video.to_string_lossy().to_string();
    let file_name = video.file_name().unwrap().to_string_lossy().to_string();
    let parent = video.parent().unwrap().to_string_lossy().to_string();
    let entry = VideoEntry {
        id: path_str.clone(),
        path: path_str.clone(),
        file_name,
        size_bytes: std::fs::metadata(video).map(|m| m.len()).unwrap_or(0),
        modified_ms: None,
        parent_dir: parent,
        show_name: None,
        season: None,
        episode: None,
        sibling_subtitles: Vec::new(),
        analysis: None,
    };
    state.library.lock().unwrap().push(entry);
    path_str
}

fn analyze(state: &AppState, path: &str) -> VideoEntry {
    run_analyze_blocking(state, path).expect("تحلیل موفق");
    get_entry(state, path).expect("entry")
}

/// استخراج تکی با چرخهٔ واقعی اسلات: acquire → run → release
fn extract_single(
    state: &AppState,
    path: &str,
    stream_index: u32,
) -> Result<crate::models::ExtractionOutcome, String> {
    let flag = state.ops.lock().unwrap().acquire_single(path)?;
    let r = run_extract_blocking(state, path, stream_index, &flag);
    state.ops.lock().unwrap().release_single(path);
    r
}

/// استخراج آزمایشی با چرخهٔ واقعی اسلات
fn review_single(
    state: &AppState,
    path: &str,
    stream_index: u32,
) -> Result<crate::models::TrackReview, String> {
    let flag = state.ops.lock().unwrap().acquire_single(path)?;
    let r = run_review_blocking(state, path, stream_index, &flag);
    state.ops.lock().unwrap().release_single(path);
    r
}

// ---------------------------------------------------------------------------
// سناریوهای اصلی فاز ۲
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn analyze_and_extract_single_fa_track() {
    let state = make_state();
    let dir = unique_dir("single");
    std::fs::create_dir_all(&dir).unwrap();
    let video = make_video(&dir, "Show.S01E01.720p.mkv", 1, 1);
    let path = push_entry(&state, &video);

    let entry = analyze(&state, &path);
    let analysis = entry.analysis.expect("analysis موجود");
    assert_eq!(analysis.status, AnalysisStatus::PersianTextTrackFound);
    assert_eq!(analysis.streams.len(), 2);
    let fa = analysis
        .streams
        .iter()
        .find(|s| s.persian_candidate)
        .expect("Track فارسی");
    assert_eq!(fa.kind, SubtitleKind::Text);
    assert_eq!(fa.language.as_deref(), Some("fas"));
    assert_eq!(fa.stream_index, 2); // Global index (0=video، 1=audio، 2=زیرنویس فارسی)

    let outcome = extract_single(&state, &path, fa.stream_index).expect("استخراج موفق");
    assert!(outcome.persian_confirmed, "محتوا باید فارسی تأیید شود");
    assert!(outcome.validation.valid);
    assert!(outcome.validation.cue_count >= 1);

    let target = Path::new(&outcome.target_path);
    assert!(target.is_file());
    let on_disk = std::fs::read(target).unwrap();
    assert!(String::from_utf8_lossy(&on_disk).contains("سلام دنیا"));

    let updated = get_entry(&state, &path).unwrap();
    let a = updated.analysis.as_ref().unwrap();
    assert_eq!(a.status, AnalysisStatus::Extracted);
    assert!(updated
        .sibling_subtitles
        .iter()
        .any(|s| s.file_name.ends_with(".fa.srt")));

    // استخراج دوباره → ممنوع (بازنویسی)
    let err = extract_single(&state, &path, fa.stream_index).unwrap_err();
    assert!(err.contains("موجود"), "{err}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn multiple_fa_tracks_need_selection_and_plan_skips() {
    let state = make_state();
    let dir = unique_dir("multi");
    std::fs::create_dir_all(&dir).unwrap();
    let video = make_video(&dir, "Show.S01E02.720p.mkv", 2, 0);
    let path = push_entry(&state, &video);

    let entry = analyze(&state, &path);
    let a = entry.analysis.unwrap();
    assert_eq!(a.status, AnalysisStatus::NeedsTrackSelection);

    let library = state.library.lock().unwrap();
    let plan = plan_extraction_from_library(&library);
    drop(library);
    assert!(plan.items.is_empty());
    assert!(plan
        .skipped
        .iter()
        .any(|s| s.video_path == path && s.reason.contains("انتخاب")));

    // استخراج دستی Track اول → موفق
    let fa = a.streams.iter().find(|s| s.persian_candidate).unwrap();
    let outcome = extract_single(&state, &path, fa.stream_index).expect("استخراج انتخابی موفق");
    assert!(outcome.persian_confirmed);
    let updated = get_entry(&state, &path).unwrap();
    assert_eq!(updated.analysis.unwrap().status, AnalysisStatus::Extracted);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn video_without_subs_and_extract_all_plan() {
    let state = make_state();
    let dir = unique_dir("none");
    std::fs::create_dir_all(&dir).unwrap();

    let video = dir.join("Movie.2023.mkv");
    run_ffmpeg(
        &[
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=320x240:d=1",
            "-c:v",
            "mpeg4",
            "-q:v",
            "7",
        ],
        &[&video],
    );
    let path = push_entry(&state, &video);
    let entry = analyze(&state, &path);
    assert_eq!(
        entry.analysis.unwrap().status,
        AnalysisStatus::NoEmbeddedSubtitle
    );

    let video2 = make_video(&dir, "Show.S01E03.720p.mkv", 1, 0);
    let path2 = push_entry(&state, &video2);
    let entry2 = analyze(&state, &path2);
    assert_eq!(
        entry2.analysis.as_ref().unwrap().status,
        AnalysisStatus::PersianTextTrackFound
    );

    let library = state.library.lock().unwrap();
    let plan = plan_extraction_from_library(&library);
    drop(library);
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].video_path, path2);
    assert!(plan.skipped.iter().any(|s| s.video_path == path));

    // اجرای پلن با پرچم گروهی واقعی از registry
    let cancel = state.ops.lock().unwrap().start_batch().unwrap();
    let (success, failed) = execute_plan_items(
        &state,
        plan.items.len() as u64,
        &plan.items,
        &cancel,
        |_p: OpsProgress| {},
        |_e: VideoEntry| {},
    );
    state.ops.lock().unwrap().end_batch();
    assert_eq!((success, failed), (1, 0));

    let updated = get_entry(&state, &path2).unwrap();
    assert_eq!(updated.analysis.unwrap().status, AnalysisStatus::Extracted);

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Fix 1 — مدیریت عمر لغو و هم‌زمانی
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn stale_cancel_flag_does_not_break_next_single_extraction() {
    let state = make_state();
    let dir = unique_dir("cancel-lifecycle");
    std::fs::create_dir_all(&dir).unwrap();
    let video = make_video_meta(&dir, "Show.S01E04.720p.mkv", &[("fas", "فارسی", "fa")]);
    let path = push_entry(&state, &video);
    analyze(&state, &path);

    // عملیات تکی اول: کاربر لغو می‌کند
    let flag1 = state.ops.lock().unwrap().acquire_single(&path).unwrap();
    flag1.store(true, Ordering::SeqCst);
    let err = run_extract_blocking(&state, &path, 2, &flag1).unwrap_err();
    assert!(err.contains("لغو"), "{err}");
    state.ops.lock().unwrap().release_single(&path);

    // عملیات تکی بعدی باید پرچم کاملاً تازه بگیرد و موفق شود
    let outcome =
        extract_single(&state, &path, 2).expect("پرچم باقی‌مانده نباید استخراج بعدی را لغو کند");
    assert!(outcome.persian_confirmed);

    // مقصد واقعاً ساخته شده
    assert!(dir.join("Show.S01E04.720p.fa.srt").is_file());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn concurrent_single_ops_and_batch_guard() {
    let state = make_state();
    let dir = unique_dir("concurrency");
    std::fs::create_dir_all(&dir).unwrap();
    let video = make_video_meta(&dir, "Show.S01E05.720p.mkv", &[("fas", "فارسی", "fa")]);
    let path = push_entry(&state, &video);
    analyze(&state, &path);

    // دو عملیات هم‌زمان روی یک ویدیو → دومی رد می‌شود
    let _flag = state.ops.lock().unwrap().acquire_single(&path).unwrap();
    let err = state.ops.lock().unwrap().acquire_single(&path).unwrap_err();
    assert!(err.contains("عملیات دیگری"), "{err}");
    state.ops.lock().unwrap().release_single(&path);

    // حین عملیات گروهی → اسلات تکی رد می‌شود (بک‌اند، نه فقط UI)
    let _batch = state.ops.lock().unwrap().start_batch().unwrap();
    let err = state.ops.lock().unwrap().acquire_single(&path).unwrap_err();
    assert!(err.contains("گروهی"), "{err}");

    // لغو همه → پرچم گروهی true می‌شود (حلقه گروهی متوقف می‌شود)
    assert!(state.ops.lock().unwrap().cancel_all());
    assert!(_batch.load(Ordering::SeqCst));
    state.ops.lock().unwrap().end_batch();

    // بعد از پایان گروهی → اسلات تکی دوباره مجاز
    assert!(state.ops.lock().unwrap().acquire_single(&path).is_ok());
    state.ops.lock().unwrap().release_single(&path);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn cancelled_extract_only_removes_its_own_temp() {
    let state = make_state();
    let dir = unique_dir("temp-isolation");
    std::fs::create_dir_all(&dir).unwrap();
    let video = make_video_meta(&dir, "Show.S01E06.720p.mkv", &[("fas", "فارسی", "fa")]);
    let path = push_entry(&state, &video);
    analyze(&state, &path);

    // فایل موقت «عملیات دیگر» را شبیه‌سازی کن — نباید دست بخورد
    let foreign_temp = dir.join("Other.S01E99.fa.extracting.deadbeef.srt");
    std::fs::write(&foreign_temp, b"other-op-temp").unwrap();

    // لغو قبل از اجرا → Cancelled و فقط temp خودش پاک می‌شود
    let flag = state.ops.lock().unwrap().acquire_single(&path).unwrap();
    flag.store(true, Ordering::SeqCst);
    let err = run_extract_blocking(&state, &path, 2, &flag).unwrap_err();
    assert!(err.contains("لغو"), "{err}");
    state.ops.lock().unwrap().release_single(&path);

    assert!(foreign_temp.is_file(), "temp عملیات دیگر نباید پاک شود");
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with("Show.S01E06") && n.contains(".extracting.")
        })
        .collect();
    assert!(
        leftovers.is_empty(),
        "temp خود عملیات باید پاک شود: {leftovers:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Fix 2 — بررسی دستی Trackهای متنی بدون Tag معتبر
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn untagged_persian_track_review_and_promote() {
    let state = make_state();
    let dir = unique_dir("review-und");
    std::fs::create_dir_all(&dir).unwrap();
    // محتوای فارسی با language=und و title خالی — هیچ نشانه‌ای در metadata
    let video = make_video_meta(&dir, "Show.S01E07.720p.mkv", &[("und", "", "fa")]);
    let path = push_entry(&state, &video);

    // تحلیل: Tag ندارد → نباید خودبه‌خود فارسی حساب شود
    let entry = analyze(&state, &path);
    let a = entry.analysis.as_ref().unwrap();
    assert_eq!(a.status, AnalysisStatus::NoPersianTrack);
    assert!(!a.streams[0].persian_candidate);

    // استخراج آزمایشی برای بررسی
    let review = review_single(&state, &path, 2).expect("استخراج آزمایشی موفق");
    assert!(review.persian_confirmed, "محتوای فارسی باید تأیید شود");
    assert!(review.validation.valid);
    assert_eq!(review.validation.encoding, "UTF-8");
    let review_path = Path::new(&review.review_path);
    assert!(review_path.is_file());
    assert!(
        review_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".track-2.review.srt"),
        "{}",
        review.review_path
    );

    // State: بررسی ثبت شده، Track تأییدشده و وضعیت دوباره ارزیابی شده
    let updated = get_entry(&state, &path).unwrap();
    let a = updated.analysis.as_ref().unwrap();
    assert_eq!(a.reviews.len(), 1);
    assert!(a.streams[0].persian_confirmed);
    assert_eq!(a.status, AnalysisStatus::PersianTextTrackFound);

    // پلن خودکار اکنون شامل این ویدیوست (Track تأییدشده = کاندید)
    let library = state.library.lock().unwrap();
    let plan = plan_extraction_from_library(&library);
    drop(library);
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].stream_index, 2);

    // تبدیل به استاندارد .fa.srt
    let promoted = run_promote_blocking(&state, &path, 2).expect("promote موفق");
    let a = promoted.analysis.as_ref().unwrap();
    assert_eq!(a.status, AnalysisStatus::Extracted);
    assert!(a.last_extraction.as_ref().unwrap().persian_confirmed);
    assert!(a.reviews[0].promoted);
    let target = Path::new(&a.last_extraction.as_ref().unwrap().target_path);
    assert!(target.is_file());
    assert!(String::from_utf8_lossy(&std::fs::read(target).unwrap()).contains("سلام دنیا"));
    // فایل بررسی منتقل شده (دیگر موجود نیست)
    assert!(!review_path.exists());

    // promote دوباره → بررسی در دسترس نیست
    let err = run_promote_blocking(&state, &path, 2).unwrap_err();
    assert!(err.contains("بررسی"), "{err}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn untagged_english_review_is_not_registered_as_persian() {
    let state = make_state();
    let dir = unique_dir("review-eng");
    std::fs::create_dir_all(&dir).unwrap();
    // بدون هیچ tagی، محتوای انگلیسی
    let video = make_video_meta(&dir, "Show.S01E08.720p.mkv", &[("", "", "en")]);
    let path = push_entry(&state, &video);
    analyze(&state, &path);

    let review = review_single(&state, &path, 2).expect("استخراج آزمایشی موفق");
    assert!(!review.persian_confirmed, "انگلیسی نباید فارسی تأیید شود");

    let updated = get_entry(&state, &path).unwrap();
    let a = updated.analysis.as_ref().unwrap();
    assert!(a.reviews.len() == 1 && !a.reviews[0].persian_confirmed);
    assert!(!a.streams[0].persian_confirmed);
    // وضعیت نباید PersianTextTrackFound شود
    assert_eq!(a.status, AnalysisStatus::NoPersianTrack);

    // پلن نباید شاملش شود و promote هم باید رد کند
    let library = state.library.lock().unwrap();
    let plan = plan_extraction_from_library(&library);
    drop(library);
    assert!(plan.items.is_empty());
    let err = run_promote_blocking(&state, &path, 2).unwrap_err();
    assert!(err.contains("بررسی"), "{err}");

    // فایل بررسی روی دیسک مانده تا کاربر خودش ببیند، اما ثبتِ زیرنویس فارسی نشده
    assert!(Path::new(&review.review_path).is_file());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn fa_tag_with_latin_content_is_not_confirmed() {
    let state = make_state();
    let dir = unique_dir("fa-tag-latin");
    std::fs::create_dir_all(&dir).unwrap();
    // دروغ Tag: language=fa ولی متن انگلیسی
    let video = make_video_meta(&dir, "Show.S01E09.720p.mkv", &[("fa", "فارسی", "en")]);
    let path = push_entry(&state, &video);
    let entry = analyze(&state, &path);
    assert_eq!(
        entry.analysis.as_ref().unwrap().status,
        AnalysisStatus::PersianTextTrackFound
    );

    let outcome = extract_single(&state, &path, 2).expect("استخراج موفق");
    assert!(!outcome.persian_confirmed, "Tag دروغگو نباید تأیید شود");

    let updated = get_entry(&state, &path).unwrap();
    let a = updated.analysis.as_ref().unwrap();
    assert!(a.message.as_deref().unwrap().contains("تأیید نشد"));
    // استخراج مستقیم نباید Track را «تأییدشده» علامت بزند
    assert!(!a.streams[0].persian_confirmed);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn review_extract_never_overwrites_existing_review_file() {
    let state = make_state();
    let dir = unique_dir("review-overwrite");
    std::fs::create_dir_all(&dir).unwrap();
    let video = make_video_meta(&dir, "Show.S01E10.720p.mkv", &[("", "", "fa")]);
    let path = push_entry(&state, &video);
    analyze(&state, &path);

    // فایل بررسی موجود از قبل — با محتوای مشخص
    let existing = dir.join("Show.S01E10.720p.track-2.review.srt");
    std::fs::write(&existing, b"PRE-EXISTING REVIEW FILE").unwrap();

    let err = review_single(&state, &path, 2).unwrap_err();
    assert!(err.contains("موجود"), "{err}");
    // محتوای فایل موجود دست نخورده
    assert_eq!(
        std::fs::read(&existing).unwrap(),
        b"PRE-EXISTING REVIEW FILE"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
