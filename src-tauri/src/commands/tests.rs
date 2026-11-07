//! تست یکپارچه هسته فاز ۲ با فایل رسانه‌ای مصنوعی.
//!
//! هسته analyze/extract/plan — کاملاً مستقل از Tauri — با ffprobe/ffmpeg واقعی
//! این سیستم آزمایش می‌شود. هیچ رسانه‌ای داخل ریپو نیست؛ همه‌چیز در temp ساخته
//! و پاک می‌شود.
//!
//! اجرا: `cargo test --lib -- --ignored`

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

use super::media::{get_entry, run_analyze_blocking};
use super::subtitle::{execute_plan_items, plan_extraction_from_library, run_extract_blocking};
use crate::models::{AnalysisStatus, OpsProgress, SubtitleKind, VideoEntry};
use crate::settings::Settings;
use crate::state::AppState;
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

/// ساخت یک MKV مصنوعی با Trackهای زیرنویس دلخواه.
/// `fa_tracks` و `eng_tracks`: تعداد Trackهای متنی با Tag زبان مربوطه.
fn make_video(dir: &Path, name: &str, fa_tracks: usize, eng_tracks: usize) -> PathBuf {
    let fixture = dir.join(format!("{name}.srt"));
    std::fs::write(
        &fixture,
        "1\n00:00:00,200 --> 00:00:00,800\nسلام دنیا\n\n2\n00:00:00,850 --> 00:00:00,990\nخط دوم\n",
    )
    .unwrap();
    let eng_fixture = dir.join(format!("{name}-eng.srt"));
    std::fs::write(
        &eng_fixture,
        "1\n00:00:00,200 --> 00:00:00,800\nhello world\n",
    )
    .unwrap();

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
    for i in 0..fa_tracks {
        args.push("-i".into());
        args.push(fixture.to_string_lossy().into_owned());
        maps.push(format!("{}:s", 2 + i));
    }
    for i in 0..eng_tracks {
        args.push("-i".into());
        args.push(eng_fixture.to_string_lossy().into_owned());
        maps.push(format!("{}:s", 2 + fa_tracks + i));
    }
    for i in 0..fa_tracks {
        args.push(format!("-metadata:s:s:{i}"));
        args.push("language=fas".into());
        args.push(format!("-metadata:s:s:{i}"));
        args.push("title=فارسی".into());
    }
    for i in 0..eng_tracks {
        let idx = fa_tracks + i;
        args.push(format!("-metadata:s:s:{idx}"));
        args.push("language=eng".into());
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

fn make_state() -> AppState {
    AppState {
        settings: std::sync::Mutex::new(Settings::default()),
        library: std::sync::Mutex::new(Vec::new()),
        last_scan: std::sync::Mutex::new(None),
        scanning: AtomicBool::new(false),
        cancel_scan: AtomicBool::new(false),
        ops_running: AtomicBool::new(false),
        cancel_ops: AtomicBool::new(false),
    }
}

/// شبیه‌سازی نتیجه اسکن: entry را مستقیم داخل State می‌گذارد (اسکن فاز ۱ جدا تست شده).
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

fn extract(
    state: &AppState,
    path: &str,
    stream_index: u32,
) -> Result<crate::models::ExtractionOutcome, String> {
    run_extract_blocking(state, path, stream_index)
}

#[test]
#[ignore]
fn analyze_and_extract_single_fa_track() {
    let state = make_state();
    let dir = unique_dir("single");
    std::fs::create_dir_all(&dir).unwrap();
    let video = make_video(&dir, "Show.S01E01.720p.mkv", 1, 1);
    let path = push_entry(&state, &video);

    // --- تحلیل ---
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

    // --- استخراج ---
    let outcome = extract(&state, &path, fa.stream_index).expect("استخراج موفق");
    assert!(outcome.persian_confirmed, "محتوا باید فارسی تأیید شود");
    assert!(outcome.validation.valid);
    assert!(outcome.validation.cue_count >= 1);

    // فایل مقصد واقعاً ساخته شده و معتبر است
    let target = Path::new(&outcome.target_path);
    assert!(target.is_file());
    let on_disk = std::fs::read(target).unwrap();
    let text = String::from_utf8_lossy(&on_disk);
    assert!(text.contains("سلام دنیا"));

    // State به‌روز شده: Extracted + زیرنویس کناری .fa.srt دیده می‌شود
    let updated = get_entry(&state, &path).unwrap();
    let a = updated.analysis.as_ref().unwrap();
    assert_eq!(a.status, AnalysisStatus::Extracted);
    assert!(updated
        .sibling_subtitles
        .iter()
        .any(|s| s.file_name.ends_with(".fa.srt")));

    // --- استخراج دوباره → ممنوع (بازنویسی) ---
    let err = extract(&state, &path, fa.stream_index).unwrap_err();
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

    // پلن خودکار باید این ویدیو را رد کند (ابهام)
    let library = state.library.lock().unwrap();
    let plan = plan_extraction_from_library(&library);
    drop(library);
    assert!(plan.items.is_empty());
    assert!(plan
        .skipped
        .iter()
        .any(|s| s.video_path == path && s.reason.contains("انتخاب")));

    // استخراج Track اول توسط کاربر (انتخاب دستی از UI) → موفق
    let fa = a.streams.iter().find(|s| s.persian_candidate).unwrap();
    let outcome = extract(&state, &path, fa.stream_index).expect("استخراج انتخابی موفق");
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

    // ویدیوی بدون زیرنویس
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

    // ویدیوی با یک Track فارسی — کاندید پلن
    let video2 = make_video(&dir, "Show.S01E03.720p.mkv", 1, 0);
    let path2 = push_entry(&state, &video2);
    let entry2 = analyze(&state, &path2);
    assert_eq!(
        entry2.analysis.as_ref().unwrap().status,
        AnalysisStatus::PersianTextTrackFound
    );

    // پلن: فقط video2 آماده است؛ Movie رد می‌شود
    let library = state.library.lock().unwrap();
    let plan = plan_extraction_from_library(&library);
    drop(library);
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].video_path, path2);
    assert!(plan.skipped.iter().any(|s| s.video_path == path));

    // اجرای پلن — همان حلقه‌ای که command گروهی استفاده می‌کند
    let (success, failed) = execute_plan_items(
        &state,
        plan.items.len() as u64,
        &plan.items,
        |_p: OpsProgress| {},
        |_e: VideoEntry| {},
    );
    assert_eq!((success, failed), (1, 0));

    let updated = get_entry(&state, &path2).unwrap();
    assert_eq!(updated.analysis.unwrap().status, AnalysisStatus::Extracted);

    let _ = std::fs::remove_dir_all(&dir);
}
