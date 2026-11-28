//! تست‌های سطح pipeline body — بدون Tauri runtime.
//!
//! کلید API هرگز به مخزن واقعی دست نمی‌زند — از طریق `PipelineIo` تزریق
//! می‌شود تا تست‌ها قطعی و مستقل از ماشین باشند.

use super::*;
use crate::archive::provenance::ProvenanceRegistry;
use crate::media::ffprobe::test_support::write_fake_probe;
use crate::pipeline::SinkStore;
use crate::pipeline::{ItemOutcome, PipelineEvent, PipelineInput, PipelineStatus, StageStatus};
use crate::state::OpsRegistry;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

/// کلید همیشه ناموجود — هیچ تستی مخزن کلید واقعی را لمس نمی‌کند.
fn missing_key() -> Result<Option<String>, String> {
    Ok(None)
}

fn make_pipeline_state() -> AppState {
    AppState {
        settings: Mutex::new(crate::settings::Settings::default()),
        library: Mutex::new(Vec::new()),
        last_scan: Mutex::new(None),
        ops: Arc::new(Mutex::new(OpsRegistry::default())),
        archive_provenance: Mutex::new(ProvenanceRegistry::default()),
        pipeline: Mutex::new(None),
        hardsub: Mutex::new(None),
        hardsub_encoder_cache: Mutex::new(None),
        setup_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        sync_attempts: Mutex::new(std::collections::HashMap::new()),
    }
}

fn make_video(dir: &Path, name: &str) -> PathBuf {
    let video = dir.join(name);
    std::fs::write(&video, b"video-bytes").unwrap();
    video
}

fn control_for(state: &AppState) -> Arc<PipelineControl> {
    let guard = crate::state::acquire_batch_op(&state.ops).unwrap();
    let control = Arc::new(PipelineControl::from_guard(
        &guard,
        "run-test".into(),
        "تست",
    ));
    std::mem::forget(guard); // بدون WorkerCleanup — پایان دستی lifecycle در تست
    control
}

fn events_store() -> (SinkStore, Arc<Mutex<Vec<PipelineEvent>>>) {
    let store: Arc<Mutex<Vec<PipelineEvent>>> = Default::default();
    let clone = Arc::clone(&store);
    let sink: SinkStore = Arc::new(Mutex::new(Box::new(move |evt: PipelineEvent| {
        clone
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(evt);
    })));
    (sink, store)
}

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "pipeline-body-test-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// pipeline کامل روی ویدیوهای بدون زیرنویس داخلی (فیک ffprobe) و بدون
/// API key → دانلود ناشناس واقعاً تلاش می‌شود؛ با fake subsource که شکست
/// می‌خورد، هر دو آیتم Failed صادقانه با دلیل دانلود می‌گیرند (هرگز
/// NeedsApiKey). staging_base تزریقی تا هیچ چیز بیرون dir تمیز نشود.
#[test]
fn pipeline_without_key_fails_download_honestly() {
    let state = make_pipeline_state();
    let dir = unique_dir("needs-key");

    // فیک ffprobe: هیچ streamی — هر دو ویدیو NoEmbeddedSubtitle می‌گیرند
    let probe = write_fake_probe(&dir, "empty", 0, "{\"streams\": []}");
    state.settings.lock().unwrap().tool_paths.ffprobe = Some(probe.to_string_lossy().into_owned());

    // فیک subsource که بلافاصله شکست می‌خورد — تا مسیر ناشناس به‌صورت
    // قطعی و بدون ابزار واقعی/شبکه اجرا شود
    let fake_sub = dir.join("fake-subsource.cmd");
    std::fs::write(&fake_sub, "@echo off\r\nexit /b 1\r\n").unwrap();
    state.settings.lock().unwrap().tool_paths.subsource =
        Some(fake_sub.to_string_lossy().into_owned());

    let v1 = make_video(&dir, "Show.S01E01.mkv");
    let v2 = make_video(&dir, "Show.S01E02.mkv");

    let control = control_for(&state);
    let input = PipelineInput::Files {
        paths: vec![
            v1.to_string_lossy().into_owned(),
            v2.to_string_lossy().into_owned(),
        ],
    };
    let (sink, events) = events_store();
    let io = PipelineIo {
        key_provider: &missing_key,
        staging_base: Some(&dir),
        report_dir: None,
        auto_mode: false,
    };

    let summary = run_pipeline_body(&state, &control, input, &sink, &io).unwrap();

    // وضعیت نهایی و شمارش‌ها
    assert_eq!(summary.status, PipelineStatus::Completed);
    assert_eq!(summary.counts.analyzed, 2);
    assert_eq!(
        summary.counts.needs_api_key, 0,
        "کلید اختیاری است — نباید NeedsApiKey تولید شود"
    );
    assert_eq!(summary.counts.decisions_pending, 0);
    // هر دو آیتم با شکست صادقانهٔ دانلود بسته شده‌اند — نه پیام کلید
    for item in &summary.items {
        match &item.outcome {
            Some(ItemOutcome::Failed { reason }) => {
                assert!(reason.contains("دانلود"), "{}", reason);
                assert!(!reason.contains("API key"), "{}", reason);
            }
            other => panic!("انتظار Failed، دریافت: {other:?}"),
        }
        assert!(matches!(item.stage_status, StageStatus::Done));
    }
    // رویدادهای کلیدی منتشر شده‌اند — Done توسط run_worker صادر می‌شود نه body
    let events = events.lock().unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e, PipelineEvent::LibraryReplaced { entries } if entries.len() == 2)));
    assert!(
        events
            .iter()
            .filter(|e| matches!(e, PipelineEvent::Item { .. }))
            .count()
            >= 4
    );
    assert_eq!(control.snapshot().status, PipelineStatus::Completed);

    let _ = std::fs::remove_dir_all(&dir);
}

/// cancel از قبل ست → run فوراً Err(Cancelled) می‌دهد.
#[test]
fn cancelled_pipeline_returns_cancelled_error() {
    let state = make_pipeline_state();
    let dir = unique_dir("cancel-early");
    let probe = write_fake_probe(&dir, "cancel", 0, "{\"streams\": []}");
    state.settings.lock().unwrap().tool_paths.ffprobe = Some(probe.to_string_lossy().into_owned());
    let v1 = make_video(&dir, "Show.S01E01.mkv");

    let control = control_for(&state);
    // همان پرچمی که cancel_active_pipeline ست می‌کند — قبل از شروع
    control.cancel_flag().store(true, Ordering::SeqCst);
    let input = PipelineInput::Files {
        paths: vec![v1.to_string_lossy().into_owned()],
    };
    let (sink, _events) = events_store();
    let io = PipelineIo {
        key_provider: &missing_key,
        staging_base: None,
        report_dir: None,
        auto_mode: false,
    };
    let err = run_pipeline_body(&state, &control, input, &sink, &io).unwrap_err();
    assert!(matches!(err, PipelineError::Cancelled));
    let _ = std::fs::remove_dir_all(&dir);
}

/// تست زندهٔ انتها-به-انتها: ffprobe/ffmpeg/ffs واقعی + بدون API key.
/// پیش‌نیاز: ffmpeg و ffprobe در PATH (مثل سایر تست‌های #[ignore]).
/// ویدیوی سینتتیک با Track متنی فارسیِ tagدار (`language=fas`) → بدون هیچ
/// تصمیم کاربر: استخراج واقعی → skip دانلود (sidecar معتبر) → سینک واقعی →
/// جانمایی یا کیفیت پایین → Completed.
///
/// ⚠️ Track بدون tag باید **مطلقاً** استفاده نشود — pipeline برای آن تصمیم
/// D-Review ثبت می‌کند و تستِ بدون resolver تا ابد منتظر می‌ماند (باگ واقعی
/// نسخهٔ قبلی این تست: `live_pipeline_extracts_and_reports_needs_api_key`
/// روی انتظار تصمیم بلاک می‌شد و هیچ‌وقت تمام نمی‌شد).
#[test]
#[ignore]
fn live_pipeline_extracts_syncs_and_places_tagged_track() {
    let _probe = crate::tools::resolve_tool_path("ffprobe", &Default::default())
        .expect("ffprobe واقعی لازم است");
    let ffmpeg = crate::tools::resolve_tool_path("ffmpeg", &Default::default())
        .expect("ffmpeg واقعی لازم است");
    let state = make_pipeline_state();
    let dir = unique_dir("live");

    // ویدیوی سینتتیک با Track متنی فارسی tagدار (الگوی commands/tests.rs)
    let fa_fixture = dir.join("fa-fixture.srt");
    std::fs::write(&fa_fixture, "1\n00:00:00,200 --> 00:00:00,800\nسلام دنیا\n").unwrap();
    let video = dir.join("Live.Show.S01E01.mkv");
    let args: Vec<String> = vec![
        "-y".into(),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "color=c=black:s=320x240:d=2".into(),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        "sine=d=2".into(),
        "-i".into(),
        fa_fixture.to_string_lossy().into_owned(),
        "-map".into(),
        "0:v".into(),
        "-map".into(),
        "1:a".into(),
        "-map".into(),
        "2:s".into(),
        "-c:v".into(),
        "mpeg4".into(),
        "-c:a".into(),
        "aac".into(),
        "-c:s".into(),
        "srt".into(),
        "-metadata:s:s:0".into(),
        "language=fas".into(),
    ];
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut cmd = std::process::Command::new(&ffmpeg);
    cmd.args(&arg_refs).arg(&video);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().expect("ffmpeg ساخت ویدیو");
    assert!(out.status.success(), "ساخت MKV سینتتیک ناموفق بود");

    let control = control_for(&state);
    let input = PipelineInput::Files {
        paths: vec![video.to_string_lossy().into_owned()],
    };
    let (sink, _events) = events_store();
    let io = PipelineIo {
        key_provider: &missing_key,
        staging_base: None,
        report_dir: None,
        auto_mode: false,
    };
    let summary = run_pipeline_body(&state, &control, input, &sink, &io).unwrap();

    // استخراج واقعی → sidecar معتبر دانلود را skip می‌کند (بدون نیاز به کلید)
    // → سینک واقعی → جانمایی «.fa.synced.srt» یا کیفیت پایین — هر دو پایان صادقانه
    assert_eq!(summary.status, PipelineStatus::Completed);
    assert!(summary.counts.analyzed >= 1);
    assert_eq!(summary.counts.extracted, 1);
    assert_eq!(
        summary.counts.needs_api_key, 0,
        "sidecar معتبر باید دانلود را skip کند"
    );
    assert_eq!(summary.counts.decisions_pending, 0);
    let item = &summary.items[0];
    assert!(item
        .artifacts
        .sync_input_path
        .as_deref()
        .unwrap_or("")
        .contains("fa.srt"));
    match &item.outcome {
        Some(ItemOutcome::Synced { output_path }) => {
            assert!(
                output_path.contains("fa.synced.srt"),
                "خروجی جانمایی‌شده باید نسخهٔ synced باشد: {output_path}"
            );
            assert!(
                std::path::Path::new(output_path).exists(),
                "فایل سینک‌شده باید روی دیسک باشد: {output_path}"
            );
        }
        Some(ItemOutcome::UnchangedLowQuality { .. }) => {
            // آفست پایین‌کیفیت — فایل اصلی دست‌نخورده؛ پایان صادقانهٔ دیگر
        }
        other => panic!("outcome غیرمنتظره برای ویدیوی tagدار با sینک واقعی: {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}
