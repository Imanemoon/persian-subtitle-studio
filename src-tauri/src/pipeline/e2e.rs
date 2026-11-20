//! تست‌های انتها-به-انتهای pipeline (Task 6) — پوشش سناریوهای واقعی با
//! فیکسچرهای موقت و ابزارهای جعلی (ffprobe/ffmpeg/ffsubsync/subsource/7-Zip).
//!
//! هیچ رسانه واقعی استفاده نمی‌شود؛ هر تست در پوشهٔ موقت یکتا اجرا می‌شود و
//! کلید API هیچ‌گاه به مخزن کلید واقعی دست نمی‌زند (تزریق از طریق PipelineIo).
//!
//! اجرا: `cargo test --lib pipeline::e2e`

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::engine::{run_worker, PipelineIo};
use super::types::{
    ItemArtifacts, ItemOutcome, PendingDecision, PipelineEvent, PipelineInput,
    PipelineItemSnapshot, PipelineSnapshot, PipelineStage, PipelineStatus, PipelineSummary,
    StageStatus,
};
use super::{cancel_active_pipeline, resolve_decision_core, PipelineControl, WorkerCleanup};
use crate::archive::provenance::ProvenanceRegistry;
use crate::settings::Settings;
use crate::state::{acquire_batch_op, acquire_single_op, AppState, OpsRegistry};

// ---------------------------------------------------------------------------
// فیکسچرها
// ---------------------------------------------------------------------------

/// زیرنویس فارسی معتبر (نسخهٔ اول) — CRLF مثل خروجی واقعی
const SRT_FA_A: &str = "1\r\n00:00:01,000 --> 00:00:04,000\r\nسلام دنیا از نسخهٔ اول\r\n";

/// زیرنویس فارسی معتبر (نسخهٔ دوم — برای سناریوی انتخاب کاربر)
const SRT_FA_B: &str = "1\r\n00:00:01,000 --> 00:00:04,000\r\nسلام دنیا از نسخهٔ دوم\r\n";

/// زیرنویس غیرفارسی — برای مسیر review تأییدنشده
const SRT_LATIN: &str = "1\r\n00:00:01,000 --> 00:00:04,000\r\nOnly latin text here\r\n";

/// sidecar خراب — بایت‌های غیرمتنِ معتبر
const GARBAGE: &[u8] = &[0xFF, 0xFE, 0x00, 0xC3, 0x28, 0xE2, 0x80, 0x8B, 0x01, 0x02];

/// محتوای عضو آرشیو در fake_subsource — دقیقاً همان SRT_FA_MEMBER آن (LF)
const ARCHIVE_MEMBER_FA: &str = "1\n00:00:01,000 --> 00:00:04,000\nسلام دنیا\n\n2\n00:00:05,000 --> 00:00:08,000\nخط دوم فارسی\n";

/// JSON پروب: دقیقاً یک Track متنی فارسی → استخراج خودکار (بدون تصمیم)
const JSON_ONE_FA: &str = r#"{"streams": [{"index": 2, "codec_name": "subrip", "codec_type": "subtitle", "tags": {"language": "fas", "title": "Persian"}}]}"#;

/// JSON پروب: دو Track متنی فارسی → تصمیم صریح کاربر
const JSON_TWO_FA: &str = r#"{"streams": [{"index": 2, "codec_name": "subrip", "codec_type": "subtitle", "tags": {"language": "fas"}}, {"index": 3, "codec_name": "subrip", "codec_type": "subtitle", "tags": {"language": "fas", "title": "second"}}]}"#;

/// JSON پروب: Track متنی بدون هیچ نشانهٔ فارسی → مسیر review
const JSON_UNTAGGED: &str =
    r#"{"streams": [{"index": 2, "codec_name": "subrip", "codec_type": "subtitle"}]}"#;

/// JSON پروب: بدون زیرنویس داخلی → مسیر sidecar/دانلود
const JSON_NONE: &str = r#"{"streams": []}"#;

/// ffsubsync جعلی — مود از فایل کنارش خوانده می‌شود (کپی اسکریپت تست‌های سینک).
/// پارسر shift-based بدون `for %%A in (%*)` — مسیرهای دارای فاصله را سالم نگه می‌دارد.
const FAKE_FFS_SCRIPT: &str = "@echo off\r\nsetlocal enabledelayedexpansion\r\nset \"MODE=\"\r\nif exist \"%~dp0.fake_ffs_mode\" set /p MODE=<\"%~dp0.fake_ffs_mode\"\r\nset \"OUT=\"\r\nset \"IN=\"\r\nset \"VIDEO=\"\r\n:loop\r\nif \"%~1\"==\"\" goto run\r\nif /i \"%~1\"==\"-i\" goto capi\r\nif /i \"%~1\"==\"-o\" goto capo\r\nif \"%VIDEO%\"==\"\" set \"VIDEO=%~1\"\r\nshift\r\ngoto loop\r\n:capi\r\nshift\r\nset \"IN=%~1\"\r\nshift\r\ngoto loop\r\n:capo\r\nshift\r\nset \"OUT=%~1\"\r\nshift\r\ngoto loop\r\n:run\r\nif \"%MODE%\"==\"success\" goto success\r\nif \"%MODE%\"==\"lowquality\" goto lowquality\r\nif \"%MODE%\"==\"fail\" goto fail\r\nif \"%MODE%\"==\"slow\" goto slow\r\nexit /b 9\r\n\r\n:success\r\ncopy /y \"%IN%\" \"%OUT%\" >nul\r\necho score: 4.20 1>&2\r\necho offset seconds: 1.234 1>&2\r\nexit /b 0\r\n\r\n:lowquality\r\ncopy /y \"%IN%\" \"%OUT%\" >nul\r\necho low-quality alignment (score too low); leaving subtitles unmodified 1>&2\r\necho writing original (unsynced) output to \"%OUT%\" 1>&2\r\nexit /b 0\r\n\r\n:fail\r\necho ffmpeg crashed badly 1>&2\r\nexit /b 3\r\n\r\n:slow\r\ncopy /y \"%IN%\" \"%OUT%\" >nul\r\necho offset seconds: 1.234 1>&2\r\nping -n 31 127.0.0.1 >nul\r\nexit /b 0\r\n";

/// ffprobe جعلی: JSON هر ویدیو از `<tools>/<stem>.probe.json` خوانده می‌شود.
/// توجه: `shift` در cmd مقدار %0 را هم جابه‌جا می‌کند — پس `%~dp0` باید
/// **قبل از حلقه** در متغیر ثبت شود، وگرنه به پوشهٔ ویدیو اشاره می‌کند!
const FAKE_PROBE_SCRIPT: &str = "@echo off\r\nsetlocal\r\nset \"TOOLS=%~dp0\"\r\nset \"VID=\"\r\n:loop\r\nif \"%~1\"==\"\" goto done\r\nif /i \"%~x1\"==\".mkv\" set \"VID=%~n1\"\r\nif /i \"%~x1\"==\".mp4\" set \"VID=%~n1\"\r\nshift\r\ngoto loop\r\n:done\r\ntype \"%TOOLS%%VID%.probe.json\"\r\n";

/// ffprobe جعلی کند: ابتدا marker شروع، سپس حلقهٔ محاسباتی kill-able، در پایان JSON.
const FAKE_PROBE_SLOW_SCRIPT: &str = "@echo off\r\nsetlocal\r\nset \"TOOLS=%~dp0\"\r\necho started > \"%TOOLS%probe_started.marker\"\r\nfor /l %%i in (1,1,15000000) do @set /a x=%%i >nul\r\ntype \"%TOOLS%all.probe.json\"\r\n";

/// ffmpeg جعلی: آخرین آرگومان = مقصد؛ محتوا بر اساس mode.
/// توجه: capture ورودی‌ها بدون `call` انجام می‌شود — `shift` داخل زیرروتین
/// `call`-شده آرگومان‌های caller را نمی‌بیند و FIRST خالی می‌ماند.
const FAKE_FFMPEG_SCRIPT: &str = "@echo off\r\nsetlocal EnableDelayedExpansion\r\nset \"TOOLS=%~dp0\"\r\nset \"MODE=success\"\r\nif exist \"%TOOLS%.fake_ffmpeg_mode\" set /p MODE=<\"%TOOLS%.fake_ffmpeg_mode\"\r\nset \"LAST=\"\r\nset \"FIRST=\"\r\nset \"INPUTS=0\"\r\n:loop\r\nif \"%~1\"==\"\" goto run\r\nif /i not \"%~1\"==\"-i\" goto store\r\nshift\r\nset /a INPUTS+=1\r\nif \"%INPUTS%\"==\"1\" set \"FIRST=%~1\"\r\n:store\r\nset \"LAST=%~1\"\r\nshift\r\ngoto loop\r\n:run\r\nif \"%INPUTS%\"==\"2\" goto mux\r\nif /i \"%MODE%\"==\"fail\" exit /b 3\r\nif /i \"%MODE%\"==\"latin\" goto latin\r\ncopy /y \"%TOOLS%fixture_fa.srt\" \"%LAST%\" >nul\r\nexit /b 0\r\n:latin\r\ncopy /y \"%TOOLS%fixture_latin.srt\" \"%LAST%\" >nul\r\nexit /b 0\r\n:mux\r\necho %* > \"%TOOLS%mux_args.txt\"\r\nif /i \"%MODE%\"==\"mux-fail\" exit /b 9\r\nif /i \"%MODE%\"==\"mux-slow\" (\r\n  echo started > \"%TOOLS%mux_started.marker\"\r\n  ping -n 12 127.0.0.1 >nul\r\n)\r\ncopy /y \"%FIRST%\" \"%LAST%\" >nul\r\n>>\"%LAST%\" echo SOFTSUB-REMUX-MARKER\r\nexit /b 0\r\n";

// ---------------------------------------------------------------------------
// زیرساخت هارنس
// ---------------------------------------------------------------------------

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "subsync-pipeline-e2e-{tag}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// کامپایل یک fake executable از سورس مستقل (الگوی تست‌های آرشیو) — با
/// بازسازی در صورت کهنه‌بودن.
fn compile_fake(src_name: &str, exe_name: &str) -> PathBuf {
    static FAKE_7Z: OnceLock<PathBuf> = OnceLock::new();
    static FAKE_SUB: OnceLock<PathBuf> = OnceLock::new();
    let slot = if src_name.contains("7z") {
        &FAKE_7Z
    } else {
        &FAKE_SUB
    };
    slot.get_or_init(|| {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let target_dir = PathBuf::from(manifest_dir).join("target").join("debug");
        fs::create_dir_all(&target_dir).unwrap();
        let exe_path = target_dir.join(exe_name);
        let source_path = PathBuf::from(manifest_dir).join("tests").join(src_name);
        let stale = match (fs::metadata(&exe_path), fs::metadata(&source_path)) {
            (Ok(exe), Ok(src)) => match (exe.modified(), src.modified()) {
                (Ok(exe_time), Ok(src_time)) => src_time > exe_time,
                _ => true,
            },
            _ => true,
        };
        if stale {
            let status = std::process::Command::new("rustc")
                .arg(&source_path)
                .arg("-o")
                .arg(&exe_path)
                .arg("--edition")
                .arg("2021")
                .status();
            assert!(
                status.map(|s| s.success()).unwrap_or(false) && exe_path.exists(),
                "fake executable not found at {exe_path:?}"
            );
        }
        exe_path
    })
    .clone()
}

/// انتخاب کلید API برای تست — هرگز مخزن کلید واقعی لمس نمی‌شود.
enum KeyChoice {
    Missing,
    Present(&'static str),
    StoreError(&'static str),
}

type Resolver = Arc<dyn Fn(&PendingDecision) + Send + Sync>;

/// هارنس هر تست — پوشهٔ موقت + AppState با ابزارهای جعلی.
/// پایان تست (حتی با panic) → پاک‌سازی پوشهٔ فیکسچر (Drop).
struct Harness {
    state: Arc<AppState>,
    /// ریشهٔ فیکسچر (کتابخانهٔ ویدیو در `lib/`)
    dir: PathBuf,
    tools: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        // پوشهٔ موقت خودِ تست — هرگز مسیر بیرونی نیست
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Harness {
    fn new(tag: &str) -> Self {
        let dir = unique_dir(tag);
        let tools = dir.join("tools");
        fs::create_dir_all(&tools).unwrap();
        fs::create_dir_all(dir.join("lib")).unwrap();

        let mut settings = Settings::default();
        settings.excluded_folders.clear();

        // ffprobe جعلی per-video + ffmpeg جعلی
        let probe = tools.join("fake-ffprobe.cmd");
        fs::write(&probe, FAKE_PROBE_SCRIPT).unwrap();
        let ffmpeg = tools.join("fake-ffmpeg.cmd");
        fs::write(&ffmpeg, FAKE_FFMPEG_SCRIPT).unwrap();
        fs::write(tools.join("fixture_fa.srt"), SRT_FA_A).unwrap();
        fs::write(tools.join("fixture_latin.srt"), SRT_LATIN).unwrap();

        settings.tool_paths.ffprobe = Some(probe.to_string_lossy().into_owned());
        settings.tool_paths.ffmpeg = Some(ffmpeg.to_string_lossy().into_owned());

        let state = Arc::new(AppState {
            settings: Mutex::new(settings),
            library: Mutex::new(Vec::new()),
            last_scan: Mutex::new(None),
            ops: Arc::new(Mutex::new(OpsRegistry::default())),
            archive_provenance: Mutex::new(ProvenanceRegistry::default()),
            pipeline: Mutex::new(None),
            hardsub: Mutex::new(None),
            hardsub_encoder_cache: Mutex::new(None),
            setup_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        Harness { state, dir, tools }
    }

    /// تمام سناریوهای قدیمی برای حفظ قرارداد قبلی در حالت sidecar اجرا می‌شوند.
    fn sidecar_mode(tag: &str) -> Self {
        let harness = Self::new(tag);
        harness
            .state
            .settings
            .lock()
            .unwrap()
            .processing
            .subtitle_output_mode = crate::settings::SubtitleOutputMode::Sidecar;
        harness
    }

    /// ffsubsync جعلی با mode مشخص
    fn set_ffs(&self, mode: &str) {
        let ffs = self.tools.join("fake-ffsubsync.cmd");
        fs::write(&ffs, FAKE_FFS_SCRIPT).unwrap();
        fs::write(self.tools.join(".fake_ffs_mode"), mode).unwrap();
        self.state.settings.lock().unwrap().tool_paths.ffsubsync =
            Some(ffs.to_string_lossy().into_owned());
    }

    /// mode استخراج ffmpeg (success | latin | fail)
    fn set_ffmpeg_mode(&self, mode: &str) {
        fs::write(self.tools.join(".fake_ffmpeg_mode"), mode).unwrap();
    }

    /// آرگومان‌های ثبت‌شدهٔ آخرین فراخوانی mux جعلی — برای ادعای شکل invocation
    /// (حفظ استریم‌ها / حذف فقط Track جایگزین‌شده).
    fn mux_args(&self) -> Option<String> {
        fs::read_to_string(self.tools.join("mux_args.txt")).ok()
    }

    /// ابزار subsource جعلی + marker mode در ریشهٔ فیکسچر (در مسیر ancestor
    /// staging قرار می‌گیرد تا fake آن را بخواند)
    fn set_subsource_tool(&self, mode: &str) {
        let exe = compile_fake("fake_subsource.rs", "fake_subsource_e2e.exe");
        fs::write(self.dir.join(".fake_mode"), mode).unwrap();
        self.state.settings.lock().unwrap().tool_paths.subsource =
            Some(exe.to_string_lossy().into_owned());
    }

    fn set_seven_zip_tool(&self) {
        let exe = compile_fake("fake_7z.rs", "fake_7z_e2e.exe");
        self.state.settings.lock().unwrap().tool_paths.seven_zip =
            Some(exe.to_string_lossy().into_owned());
    }

    /// ffprobe جعلی کند با marker شروع
    fn set_slow_probe(&self, json: &str) {
        let probe = self.tools.join("fake-ffprobe-slow.cmd");
        fs::write(&probe, FAKE_PROBE_SLOW_SCRIPT).unwrap();
        fs::write(self.tools.join("all.probe.json"), json).unwrap();
        self.state.settings.lock().unwrap().tool_paths.ffprobe =
            Some(probe.to_string_lossy().into_owned());
    }

    /// JSON پروب برای یک ویدیو (بر اساس stem)
    fn probe_json(&self, video_path: &Path, json: &str) {
        let stem = video_path.file_stem().unwrap().to_string_lossy();
        fs::write(self.tools.join(format!("{stem}.probe.json")), json).unwrap();
    }

    /// ویدیوی جعلی در کتابخانه
    fn video(&self, name: &str) -> PathBuf {
        let path = self.dir.join("lib").join(name);
        fs::write(&path, b"FAKE-VIDEO-BYTES-DO-NOT-TOUCH").unwrap();
        path
    }

    /// sidecar کنار یک ویدیو با محتوای دلخواه
    fn sidecar(&self, video: &Path, suffix: &str, content: &str) -> PathBuf {
        let stem = video.file_stem().unwrap().to_string_lossy();
        let path = video.parent().unwrap().join(format!("{stem}{suffix}"));
        fs::write(&path, content).unwrap();
        path
    }

    /// sidecar با بایت‌های خراب
    fn sidecar_garbage(&self, video: &Path, suffix: &str) -> PathBuf {
        let stem = video.file_stem().unwrap().to_string_lossy();
        let path = video.parent().unwrap().join(format!("{stem}{suffix}"));
        fs::write(&path, GARBAGE).unwrap();
        path
    }

    fn video_bytes(&self, video: &Path) -> Vec<u8> {
        fs::read(video).unwrap()
    }
}

/// نتیجهٔ یک اجرا
struct Run {
    summary: PipelineSummary,
    /// تصمیم‌های منتشرشده (برای ادعاهای «هیچ تصمیمی نبود» / «دقیقاً یک تصمیم»)
    decisions: Vec<PendingDecision>,
    /// همهٔ رویدادهای منتشرشده به‌ترتیب — برای ادعاهای پیشرفت زنده
    events: Vec<PipelineEvent>,
}

impl Run {
    /// outcome نهایی یک ویدیو
    fn outcome(&self, video: &Path) -> &ItemOutcome {
        let path = video.to_string_lossy();
        self.summary
            .items
            .iter()
            .find(|i| i.video_path == path)
            .unwrap_or_else(|| panic!("آیتم {path} در خلاصه نیست"))
            .outcome
            .as_ref()
            .unwrap_or_else(|| {
                panic!(
                    "آیتم {path} outcome ندارد — counts={:?} stage_status={:?} sync_input={:?}",
                    self.summary.counts,
                    self.summary
                        .items
                        .iter()
                        .find(|i| i.video_path == path)
                        .map(|i| i.stage_status.clone()),
                    self.summary
                        .items
                        .iter()
                        .find(|i| i.video_path == path)
                        .and_then(|i| i.artifacts.sync_input_path.clone()),
                )
            })
    }

    fn artifacts(&self, video: &Path) -> &ItemArtifacts {
        let path = video.to_string_lossy();
        &self
            .summary
            .items
            .iter()
            .find(|i| i.video_path == path)
            .expect("آیتم در خلاصه نیست")
            .artifacts
    }
}

/// انتظار برای برآورده‌شدن شرط روی snapshotِ control منتشرشده.
fn wait_for_snapshot(
    state: &AppState,
    timeout: Duration,
    pred: impl Fn(&PipelineSnapshot) -> bool,
) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        let snap = state
            .pipeline
            .lock()
            .unwrap()
            .as_ref()
            .map(|c| c.snapshot());
        if let Some(snap) = snap {
            if pred(&snap) {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// اجرای کامل pipeline مثل command layer — admission، publish، worker thread،
/// capture رویدادها و resolve تصمیم‌ها. پس از پایان: بدون نشتی رزرو/کنترل.
fn run_pipeline(
    state: Arc<AppState>,
    paths: Vec<String>,
    resolver: Option<Resolver>,
    key: KeyChoice,
    staging_base: Option<PathBuf>,
) -> Run {
    // admission اتمیک — همان مسیر start_pipeline
    let guard = acquire_batch_op(&state.ops).expect("admission باید موفق باشد");
    let run_id = format!("run-e2e-{}", std::process::id());
    let control = Arc::new(PipelineControl::from_guard(
        &guard,
        run_id.clone(),
        "تست e2e",
    ));
    {
        let mut published = state.pipeline.lock().unwrap();
        *published = Some(Arc::clone(&control));
    }

    let events: Arc<Mutex<Vec<PipelineEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let input = PipelineInput::Files { paths };

    // تصمیم باز فقط داخل snapshotِ رویداد State می‌آید (بدون رویداد Decision
    // جداگانه) — resolver با dedup روی تصمیم‌های قبلاً پاسخ‌داده صدا زده می‌شود.
    let seen_decisions: Arc<Mutex<std::collections::HashSet<String>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));

    let events_for_sink = Arc::clone(&events);
    let seen_for_sink = Arc::clone(&seen_decisions);
    let sink = Arc::new(Mutex::new(Box::new(move |event: PipelineEvent| {
        let pending = match &event {
            PipelineEvent::State { snapshot } => snapshot.pending_decision.clone(),
            _ => None,
        };
        events_for_sink.lock().unwrap().push(event);
        // resolve تصمیم از داخل مسیر رویداد — مثل فرانت واقعی
        if let (Some(res), Some(dec)) = (&resolver, pending) {
            let fresh = seen_for_sink
                .lock()
                .unwrap()
                .insert(dec.decision_id.clone());
            if fresh {
                res(&dec);
            }
        }
    }) as Box<dyn FnMut(PipelineEvent) + Send>));

    let key_fn: Arc<dyn Fn() -> Result<Option<String>, String> + Send + Sync> = match key {
        KeyChoice::Missing => Arc::new(|| Ok(None)),
        KeyChoice::Present(k) => Arc::new(move || Ok(Some(k.to_string()))),
        KeyChoice::StoreError(e) => Arc::new(move || Err(e.to_string())),
    };

    let state_for_worker = Arc::clone(&state);
    let handle = std::thread::spawn(move || {
        let io = PipelineIo {
            key_provider: &*key_fn,
            staging_base: staging_base.as_deref(),
        };
        let cleanup = WorkerCleanup::new(&state_for_worker, run_id, guard);
        run_worker(&state_for_worker, &control, cleanup, input, &sink, &io);
    });
    handle.join().expect("worker نباید panic کند");

    // تضمین‌های سطح هارنس در هر سناریو: بدون نشتی رزرو و بدون control باقی‌مانده
    assert!(
        acquire_batch_op(&state.ops).is_ok(),
        "رزرو pipeline باید پس از پایان آزاد شده باشد"
    );
    assert!(
        state.pipeline.lock().unwrap().is_none(),
        "control باید پس از پایان حذف شده باشد"
    );

    let mut decisions = Vec::new();
    let (summary, captured) = {
        let evs = events.lock().unwrap();
        let mut seen_ids = std::collections::HashSet::new();
        for e in evs.iter() {
            if let PipelineEvent::State { snapshot } = e {
                if let Some(dec) = &snapshot.pending_decision {
                    if seen_ids.insert(dec.decision_id.clone()) {
                        decisions.push(dec.clone());
                    }
                }
            }
        }
        let summary = evs
            .iter()
            .find_map(|e| match e {
                PipelineEvent::Done { summary } => Some(summary.clone()),
                _ => None,
            })
            .expect("رویداد Done باید صادر شده باشد");
        (summary, evs.clone())
    };
    Run {
        summary,
        decisions,
        events: captured,
    }
}

fn placed_path(video: &Path) -> PathBuf {
    video.parent().unwrap().join(format!(
        "{}.fa.synced.srt",
        video.file_stem().unwrap().to_string_lossy()
    ))
}

fn extracted_path(video: &Path) -> PathBuf {
    video.parent().unwrap().join(format!(
        "{}.fa.srt",
        video.file_stem().unwrap().to_string_lossy()
    ))
}

// ---------------------------------------------------------------------------
// سناریوهای جدید خروجی SoftSub (حالت پیش‌فرض)
// ---------------------------------------------------------------------------

#[test]
fn default_mode_embedded_persian_track_becomes_softsub_without_sidecars() {
    let h = Harness::new("softsub-embedded");
    h.set_ffs("success");
    h.set_ffmpeg_mode("success");
    let video = h.video("Show.S01E01.mkv");
    h.probe_json(&video, JSON_ONE_FA);
    let before = h.video_bytes(&video);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    match run.outcome(&video) {
        ItemOutcome::SoftSubbed { video_path } => {
            assert_eq!(Path::new(video_path), video.as_path());
        }
        other => panic!("انتظار SoftSubbed، دریافت: {other:?}"),
    }
    let after = h.video_bytes(&video);
    assert_ne!(after, before, "خروجی SoftSub باید جایگزین فایل اصلی شود");
    assert!(
        after
            .windows(b"SOFTSUB-REMUX-MARKER".len())
            .any(|window| window == b"SOFTSUB-REMUX-MARKER"),
        "نشانگر remux باید داخل فایل نهایی باشد"
    );
    assert!(
        !placed_path(&video).exists(),
        "نباید sidecar سینک‌شده ساخته شود"
    );
    assert!(
        !extracted_path(&video).exists(),
        "فایل استخراج موقت پس از mux موفق پاک می‌شود"
    );
    assert_eq!(run.summary.counts.softsubbed, 1);
    assert!(run.decisions.is_empty());
    // Track داخلی (index 2) باید حذف و نسخهٔ سینک‌شده جایگزین شود — بدون Track فارسی تکراری
    let args = h.mux_args().expect("mux جعلی باید فراخوانی شود");
    assert!(
        args.contains("-0:2"),
        "mux باید فقط Track داخلی فارسی را حذف کند: {args}"
    );
    assert!(
        args.contains("1:0"),
        "زیرنویس سینک‌شده باید ورودی دوم باشد: {args}"
    );
}

#[test]
fn default_mode_downloads_missing_persian_subtitle_and_softsubs_mp4() {
    let h = Harness::new("softsub-download");
    h.set_ffs("success");
    h.set_ffmpeg_mode("success");
    h.set_subsource_tool("write-manifest-archives");
    h.set_seven_zip_tool();
    let video = h.video("Show.S01E02.mp4");
    h.probe_json(&video, JSON_NONE);
    let before = h.video_bytes(&video);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Present("k-test"),
        Some(h.dir.clone()),
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert!(matches!(
        run.outcome(&video),
        ItemOutcome::SoftSubbed { .. }
    ));
    let after = h.video_bytes(&video);
    assert_ne!(after, before);
    assert!(after
        .windows(b"SOFTSUB-REMUX-MARKER".len())
        .any(|window| window == b"SOFTSUB-REMUX-MARKER"));
    assert!(!placed_path(&video).exists());
    assert!(!extracted_path(&video).exists());
    assert_eq!(run.summary.counts.downloaded, 1);
    assert_eq!(run.summary.counts.archives_placed, 1);
    assert_eq!(run.summary.counts.softsubbed, 1);
    // بدون Track داخلی فارسی، همهٔ استریم‌های قبلی حفظ می‌شوند — هیچ حذفی نباید
    let args = h.mux_args().expect("mux جعلی باید فراخوانی شود");
    assert!(
        !args.contains("-0:"),
        "بدون Track داخلی هیچ استریمی نباید حذف شود: {args}"
    );
    assert!(args.contains("1:0"), "زیرنویس باید ورودی دوم باشد: {args}");
}

#[test]
fn failed_softsub_remux_preserves_original_video_and_recovery_inputs() {
    let h = Harness::new("softsub-fail");
    h.set_ffs("success");
    h.set_ffmpeg_mode("mux-fail");
    let video = h.video("Show.S01E03.mkv");
    h.probe_json(&video, JSON_ONE_FA);
    let before = h.video_bytes(&video);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    match run.outcome(&video) {
        ItemOutcome::Failed { reason } => assert!(reason.contains("SoftSub")),
        other => panic!("انتظار شکست SoftSub، دریافت: {other:?}"),
    }
    assert_eq!(
        h.video_bytes(&video),
        before,
        "فایل اصلی در شکست mux باید دست‌نخورده بماند"
    );
    assert!(
        extracted_path(&video).exists(),
        "ورودی استخراج‌شده برای بازیابی حفظ می‌شود"
    );
    assert!(run.artifacts(&video).sync_candidate_path.is_some());
}

#[test]
fn cancel_during_softsub_mux_leaves_original_video_untouched() {
    let h = Harness::new("softsub-cancel");
    h.set_ffs("success");
    h.set_ffmpeg_mode("mux-slow");
    let video = h.video("Show.S01E04.mkv");
    h.probe_json(&video, JSON_ONE_FA);
    let before = h.video_bytes(&video);

    let st = Arc::clone(&h.state);
    let marker = h.tools.join("mux_started.marker");
    let watcher = std::thread::spawn(move || {
        let start = Instant::now();
        while !marker.exists() && start.elapsed() < Duration::from_secs(30) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists(), "فرایند remux باید شروع شده باشد");
        assert!(cancel_active_pipeline(&st).expect("لغو"));
    });

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Missing,
        None,
    );
    watcher.join().expect("watcher");

    assert_eq!(run.summary.status, PipelineStatus::Cancelled);
    assert!(matches!(run.outcome(&video), ItemOutcome::Cancelled));
    assert_eq!(
        h.video_bytes(&video),
        before,
        "لغو حین mux نباید فایل اصلی را تغییر دهد"
    );
    assert!(!placed_path(&video).exists());
}

/// SoftSub با sidecar از قبل سینک‌شده: در تضاد با حالت sidecar (AlreadySynced)،
/// نسخهٔ سینک‌شدهٔ موجود به‌عنوان ورودی سینک استفاده و داخل ویدیو قرار می‌گیرد؛
/// فایل کاربر حفظ می‌شود و هیچ Track داخلی‌ای حذف نمی‌شود (ویدیو Track ندارد).
#[test]
fn softsub_with_already_synced_sidecar_muxes_it_without_sidecar_outcome() {
    let h = Harness::new("softsub-presynced");
    h.set_ffs("success");
    h.set_ffmpeg_mode("success");
    let video = h.video("Show.S01E08.mkv");
    h.probe_json(&video, JSON_NONE);
    let sub = h.sidecar(&video, ".fa.synced.srt", SRT_FA_A);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert!(matches!(
        run.outcome(&video),
        ItemOutcome::SoftSubbed { .. }
    ));
    assert_eq!(
        run.summary.counts.already_synced, 0,
        "در SoftSub نباید AlreadySynced بماند"
    );
    assert_eq!(
        fs::read(&sub).unwrap(),
        SRT_FA_A.as_bytes(),
        "sidecar کاربر حفظ شود"
    );
    let after = h.video_bytes(&video);
    assert!(after
        .windows(b"SOFTSUB-REMUX-MARKER".len())
        .any(|w| w == b"SOFTSUB-REMUX-MARKER"));
    let args = h.mux_args().expect("mux جعلی باید فراخوانی شود");
    assert!(
        !args.contains("-0:"),
        "بدون Track داخلی هیچ استریمی حذف نمی‌شود: {args}"
    );
    assert_eq!(run.summary.counts.softsubbed, 1);
    assert!(run.decisions.is_empty());
}

/// سناریوی مسیرهای ویندوز با فاصله و نویسه‌های فارسی — کل چرخهٔ SoftSub باید
/// بدون هیچ خطای مسیری کار کند (آرگومان‌ها هرگز از shell عبور نمی‌کنند).
#[test]
fn softsub_handles_windows_paths_with_spaces_and_persian_characters() {
    let h = Harness::new("softsub-persian-path");
    h.set_ffs("success");
    h.set_ffmpeg_mode("success");
    let subdir = h.dir.join("lib").join("نمونه نمایش فصل ۱ - HD");
    fs::create_dir_all(&subdir).unwrap();
    let video = subdir.join("Show.S01E09.mkv");
    fs::write(&video, b"FAKE-VIDEO-BYTES-DO-NOT-TOUCH").unwrap();
    h.probe_json(&video, JSON_ONE_FA);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    match run.outcome(&video) {
        ItemOutcome::SoftSubbed { video_path } => {
            assert_eq!(Path::new(video_path), video.as_path());
        }
        other => panic!("انتظار SoftSubbed، دریافت: {other:?}"),
    }
    let after = h.video_bytes(&video);
    assert!(
        after
            .windows(b"SOFTSUB-REMUX-MARKER".len())
            .any(|w| w == b"SOFTSUB-REMUX-MARKER"),
        "نشانگر remux باید داخل فایل نهایی باشد"
    );
    assert!(!placed_path(&video).exists());
    assert!(!extracted_path(&video).exists());
    assert_eq!(run.summary.counts.softsubbed, 1);
}

/// SoftSub با sidecar موجود کاربر: sidecar حفظ می‌شود (هرگز حذف نمی‌شود)،
/// ویدیو remux می‌شود و Track داخلی فارسی یکتا جایگزین می‌شود — بدون Track تکراری.
#[test]
fn softsub_with_existing_sidecar_preserves_user_file_and_replaces_embedded_track() {
    let h = Harness::new("softsub-sidecar");
    h.set_ffs("success");
    h.set_ffmpeg_mode("success");
    let video = h.video("Show.S01E05.mkv");
    h.probe_json(&video, JSON_ONE_FA);
    let sub = h.sidecar(&video, ".fa.srt", SRT_FA_A);
    let before = h.video_bytes(&video);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert!(matches!(
        run.outcome(&video),
        ItemOutcome::SoftSubbed { .. }
    ));
    // فایل کاربر دست‌نخورده می‌ماند — حتی پس از mux موفق
    assert_eq!(
        fs::read(&sub).unwrap(),
        SRT_FA_A.as_bytes(),
        "sidecar کاربر حفظ شود"
    );
    let after = h.video_bytes(&video);
    assert_ne!(after, before, "ویدیو باید با نسخهٔ remux جایگزین شود");
    assert!(after
        .windows(b"SOFTSUB-REMUX-MARKER".len())
        .any(|w| w == b"SOFTSUB-REMUX-MARKER"));
    // Track داخلی یکتا (index 2) جایگزین می‌شود — بدون Track فارسی تکراری
    let args = h.mux_args().expect("mux جعلی باید فراخوانی شود");
    assert!(
        args.contains("-0:2"),
        "Track داخلی باید جایگزین شود: {args}"
    );
    assert_eq!(run.summary.counts.softsubbed, 1);
    assert!(
        run.decisions.is_empty(),
        "با sidecar موجود هیچ تصمیمی لازم نیست"
    );
}

/// چند Track داخلی فارسی + sidecar موجود: انتخاب Track جایگزین‌شدنی تصمیم صریح
/// کاربر است؛ Track انتخابی حذف و نسخهٔ سینک‌شده جایگزین می‌شود و sidecar حفظ می‌شود.
#[test]
fn softsub_with_ambiguous_embedded_tracks_asks_which_track_to_replace() {
    let h = Harness::new("softsub-ambiguous");
    h.set_ffs("success");
    h.set_ffmpeg_mode("success");
    let video = h.video("Show.S01E06.mkv");
    h.probe_json(&video, JSON_TWO_FA);
    let sub = h.sidecar(&video, ".fa.srt", SRT_FA_A);
    let before = h.video_bytes(&video);

    let st = Arc::clone(&h.state);
    let resolver: Resolver = Arc::new(move |decision: &PendingDecision| {
        assert_eq!(decision.kind, crate::pipeline::DecisionKind::EmbeddedTrack);
        assert_eq!(
            decision.options.len(),
            2,
            "دو Track فارسی باید دو گزینه بدهند"
        );
        // کاربر Track دوم (index 3) را برای جایگزینی انتخاب می‌کند
        let chosen = decision
            .options
            .iter()
            .find(|o| {
                matches!(
                    &o.payload,
                    crate::pipeline::DecisionOptionPayload::EmbeddedTrack {
                        stream_index: 3,
                        ..
                    }
                )
            })
            .expect("گزینهٔ Track دوم باید موجود باشد");
        resolve_decision_core(&st, &decision.decision_id, &chosen.option_id).expect("resolve");
    });

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        Some(resolver),
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert_eq!(run.decisions.len(), 1);
    assert!(matches!(
        run.outcome(&video),
        ItemOutcome::SoftSubbed { .. }
    ));
    assert_eq!(
        fs::read(&sub).unwrap(),
        SRT_FA_A.as_bytes(),
        "sidecar کاربر حفظ شود"
    );
    assert_ne!(
        h.video_bytes(&video),
        before,
        "ویدیو باید با نسخهٔ remux جایگزین شود"
    );
    // فقط Track انتخاب‌شدهٔ کاربر (index 3) حذف می‌شود
    let args = h.mux_args().expect("mux جعلی باید فراخوانی شود");
    assert!(
        args.contains("-0:3"),
        "فقط Track انتخابی باید حذف شود: {args}"
    );
    assert_eq!(run.summary.counts.softsubbed, 1);
}

/// SoftSub برای Track بدون تگ فارسی که با review محتوا تأیید شده — نسخهٔ سینک‌شده
/// جایگزین همان Track می‌شود (regression: probe تازه فارسی‌بودنِ تأییدشده با
/// محتوا را نمی‌داند و نباید mux را رد کند).
#[test]
fn softsub_review_confirmed_untagged_track_is_replaced_not_rejected() {
    let h = Harness::new("softsub-untagged");
    h.set_ffs("success");
    h.set_ffmpeg_mode("success");
    let video = h.video("Show.S01E07.mkv");
    h.probe_json(&video, JSON_UNTAGGED);

    let st = Arc::clone(&h.state);
    let resolver: Resolver = Arc::new(move |decision: &PendingDecision| {
        assert_eq!(decision.kind, crate::pipeline::DecisionKind::ReviewTrack);
        resolve_decision_core(&st, &decision.decision_id, &decision.options[0].option_id)
            .expect("resolve باید موفق باشد");
    });

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        Some(resolver),
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert!(matches!(
        run.outcome(&video),
        ItemOutcome::SoftSubbed { .. }
    ));
    // Track بدون تگ (index 2) که با review تأیید شد جایگزین می‌شود
    let args = h.mux_args().expect("mux جعلی باید فراخوانی شود");
    assert!(
        args.contains("-0:2"),
        "Track تأییدشده با review باید جایگزین شود: {args}"
    );
    // فایل استخراجِ موقتِ همین اجرا پس از mux موفق پاک می‌شود
    assert!(!extracted_path(&video).exists());
    assert_eq!(run.summary.counts.softsubbed, 1);
}

// ---------------------------------------------------------------------------
// سناریوهای محتوایی (۱ تا ۱۳، ۱۸ تا ۲۲)
// ---------------------------------------------------------------------------

/// سناریو ۱+۱۲: ویدیو با Track فارسی معتبر → استخراج خودکار → سینک → جانمایی.
#[test]
fn embedded_valid_persian_track_is_extracted_synced_and_placed() {
    let h = Harness::sidecar_mode("embedded-fa");
    h.set_ffs("success");
    h.set_ffmpeg_mode("success");
    let video = h.video("Show.S01E01.mkv");
    h.probe_json(&video, JSON_ONE_FA);
    let before = h.video_bytes(&video);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    match run.outcome(&video) {
        ItemOutcome::Synced { output_path } => {
            assert_eq!(
                Path::new(output_path),
                placed_path(&video),
                "مسیر جانمایی باید کنار ویدیو باشد"
            );
        }
        other => panic!("انتظار Synced، دریافت: {other:?}"),
    }
    // جانمایی واقعی + محتوای سینک‌شده = محتوای زیرنویس منبع
    assert_eq!(fs::read(placed_path(&video)).unwrap(), SRT_FA_A.as_bytes());
    // ویدیوی اصلی دست‌نخورده
    assert_eq!(h.video_bytes(&video), before, "ویدیوی اصلی نباید تغییر کند");
    // sidecar استخراج‌شده حفظ شده
    assert_eq!(
        fs::read(extracted_path(&video)).unwrap(),
        SRT_FA_A.as_bytes()
    );
    assert_eq!(run.summary.counts.synced, 1);
    assert!(run.summary.counts.extracted >= 1);
    // هیچ تصمیمی لازم نبوده
    assert!(run.decisions.is_empty(), "سناریوی تک‌تراک نباید تصمیم بگیرد");
}

/// سناریو ۲: Track مشکوک/بدون نشانه → تصمیم review صریح → تأیید → سینک.
#[test]
fn untagged_track_requires_explicit_review_decision() {
    let h = Harness::sidecar_mode("untagged-review");
    h.set_ffs("success");
    h.set_ffmpeg_mode("success");
    let video = h.video("Show.S01E02.mkv");
    h.probe_json(&video, JSON_UNTAGGED);
    let before = h.video_bytes(&video);

    let st = Arc::clone(&h.state);
    let resolver: Resolver = Arc::new(move |decision: &PendingDecision| {
        assert_eq!(decision.kind, crate::pipeline::DecisionKind::ReviewTrack);
        assert_eq!(decision.options.len(), 1);
        resolve_decision_core(&st, &decision.decision_id, &decision.options[0].option_id)
            .expect("resolve باید موفق باشد");
    });

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        Some(resolver),
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert_eq!(run.decisions.len(), 1, "دقیقاً یک تصمیم review");
    assert!(matches!(run.outcome(&video), ItemOutcome::Synced { .. }));
    // فایل ارتقایافته به .fa.srt — مبنای سینک
    assert_eq!(
        fs::read(extracted_path(&video)).unwrap(),
        SRT_FA_A.as_bytes()
    );
    assert_eq!(fs::read(placed_path(&video)).unwrap(), SRT_FA_A.as_bytes());
    assert_eq!(h.video_bytes(&video), before);
}

/// سناریو ۳: دقیقاً یک sidecar معتبر → بدون هیچ تصمیم، سینک خودکار.
#[test]
fn single_valid_sidecar_syncs_without_any_decision() {
    let h = Harness::sidecar_mode("single-sidecar");
    h.set_ffs("success");
    h.set_ffmpeg_mode("success");
    let video = h.video("Show.S01E03.mkv");
    h.probe_json(&video, JSON_NONE);
    let sub = h.sidecar(&video, ".fa.srt", SRT_FA_A);
    let before = h.video_bytes(&video);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert!(run.decisions.is_empty(), "یک کاندیدا نباید تصمیم بگیرد");
    assert!(matches!(run.outcome(&video), ItemOutcome::Synced { .. }));
    assert_eq!(fs::read(placed_path(&video)).unwrap(), SRT_FA_A.as_bytes());
    assert_eq!(
        fs::read(&sub).unwrap(),
        SRT_FA_A.as_bytes(),
        "sidecar اصلی دست‌نخورده"
    );
    assert_eq!(h.video_bytes(&video), before);
    assert_eq!(
        run.summary.counts.needs_api_key, 0,
        "با sidecar معتبر دانلودی نباید رخ دهد"
    );
}

/// سناریو ۴: دو sidecar معتبر → تصمیم صریح؛ انتخاب کاربر سینک می‌شود.
#[test]
fn multiple_sidecars_require_explicit_user_choice() {
    let h = Harness::sidecar_mode("multi-sidecar");
    h.set_ffs("success");
    h.set_ffmpeg_mode("success");
    let video = h.video("Show.S01E04.mkv");
    h.probe_json(&video, JSON_NONE);
    h.sidecar(&video, ".fa.srt", SRT_FA_A);
    let second = h.sidecar(&video, ".fa.1.srt", SRT_FA_B);
    let before = h.video_bytes(&video);

    let second_path = second.to_string_lossy().into_owned();
    let st = Arc::clone(&h.state);
    let resolver: Resolver = Arc::new(move |decision: &PendingDecision| {
        assert_eq!(decision.kind, crate::pipeline::DecisionKind::SyncSidecar);
        assert_eq!(decision.options.len(), 2, "دو کاندیدا باید دو گزینه بدهند");
        // کاربر نسخهٔ دوم را انتخاب می‌کند — نه پیش‌فرض
        let chosen = decision
            .options
            .iter()
            .find(|o| {
                matches!(
                    &o.payload,
                    crate::pipeline::DecisionOptionPayload::SidecarPath { path }
                        if path.to_string_lossy() == second_path
                )
            })
            .expect("گزینهٔ دوم باید جزو گزینه‌ها باشد");
        resolve_decision_core(&st, &decision.decision_id, &chosen.option_id).expect("resolve");
    });

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        Some(resolver),
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert_eq!(run.decisions.len(), 1);
    assert!(matches!(run.outcome(&video), ItemOutcome::Synced { .. }));
    // محتوای جانمایی‌شده = انتخاب کاربر (نه پیش‌فرض)
    assert_eq!(fs::read(placed_path(&video)).unwrap(), SRT_FA_B.as_bytes());
    assert_eq!(h.video_bytes(&video), before);
}

/// سناریو ۵: sidecar خراب → تعارض تایپ‌شده؛ هرگز استفاده و هرگز بازنویسی نمی‌شود.
#[test]
fn corrupted_sidecar_is_reported_as_conflict_and_never_used() {
    let h = Harness::new("corrupt-sidecar");
    h.set_ffs("success");
    h.set_subsource_tool("write-manifest-archives");
    let video = h.video("Show.S01E05.mkv");
    h.probe_json(&video, JSON_NONE);
    let bad = h.sidecar_garbage(&video, ".fa.srt");
    let before = h.video_bytes(&video);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Present("k-test"),
        Some(h.dir.clone()),
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert!(matches!(run.outcome(&video), ItemOutcome::Skipped { .. }));
    assert!(
        !run.artifacts(&video).sidecar_conflicts.is_empty(),
        "تعارض sidecar باید ساختاریافته گزارش شود"
    );
    assert_eq!(
        fs::read(&bad).unwrap(),
        GARBAGE,
        "فایل خراب باید دست‌نخورده بماند"
    );
    assert_eq!(h.video_bytes(&video), before);
    assert_eq!(
        run.summary.counts.downloaded, 0,
        "با تعارض sidecar دانلود نباید رخ دهد"
    );
}

/// سناریو ۶+۹: دانلود موفق → آرشیو با یک عضو منطبق → استخراج → سینک → جانمایی.
#[test]
fn download_success_flows_through_archive_to_placed_sync() {
    let h = Harness::sidecar_mode("download-ok");
    h.set_ffs("success");
    h.set_subsource_tool("write-manifest-archives");
    h.set_seven_zip_tool();
    let video = h.video("Show.S01E06.mkv");
    h.probe_json(&video, JSON_NONE);
    let before = h.video_bytes(&video);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Present("k-test"),
        Some(h.dir.clone()),
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert_eq!(run.summary.counts.downloaded, 1);
    assert_eq!(run.summary.counts.archives_placed, 1);
    assert_eq!(run.summary.counts.synced, 1);
    assert!(matches!(run.outcome(&video), ItemOutcome::Synced { .. }));
    // محتوای جانمایی‌شده از عضو آرشیو آمده (همان محتوای fake_subsource)
    assert_eq!(
        fs::read(placed_path(&video)).unwrap(),
        ARCHIVE_MEMBER_FA.as_bytes()
    );
    // هیچ فایلی دور ریخته نشده: مسیرهای staging گزارش شده‌اند
    assert!(
        !run.summary.staged_paths.is_empty(),
        "ریشه‌های staging باید برای بازیابی گزارش شوند"
    );
    assert_eq!(h.video_bytes(&video), before);
}

/// سناریو ۷: بدون API key → دانلود ناشناس با sub-rs → آرشیو → سینک → جانمایی.
/// کلید دیگر گِیت نیست؛ نبود آن مسیر دانلود را نمی‌بندد.
#[test]
fn missing_api_key_downloads_anonymously_and_syncs() {
    let h = Harness::sidecar_mode("no-key-anonymous");
    h.set_ffs("success");
    h.set_subsource_tool("write-manifest-archives");
    h.set_seven_zip_tool();
    let video = h.video("Show.S01E07.mkv");
    h.probe_json(&video, JSON_NONE);
    let before = h.video_bytes(&video);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Missing,
        Some(h.dir.clone()),
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert_eq!(
        run.summary.counts.needs_api_key, 0,
        "با دانلود ناشناس دیگر نباید NeedsApiKey تولید شود"
    );
    assert_eq!(run.summary.counts.downloaded, 1);
    assert_eq!(run.summary.counts.synced, 1);
    assert!(matches!(run.outcome(&video), ItemOutcome::Synced { .. }));
    assert!(
        !run.summary.staged_paths.is_empty(),
        "ریشه‌های staging باید برای بازیابی گزارش شوند"
    );
    assert_eq!(h.video_bytes(&video), before);
}

/// سناریو ۷b: خطای مخزن کلید ≠ «کلید تنظیم نشده» — جدا و صادقانه.
#[test]
fn keystore_failure_is_reported_separately_from_missing_key() {
    let h = Harness::new("keystore-err");
    h.set_ffs("success");
    let video = h.video("Show.S01E08.mkv");
    h.probe_json(&video, JSON_NONE);
    let before = h.video_bytes(&video);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::StoreError("مخزن کلید قفل است"),
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    match run.outcome(&video) {
        ItemOutcome::KeyStoreUnavailable { reason } => {
            assert_eq!(reason, "مخزن کلید قفل است");
        }
        other => panic!("انتظار KeyStoreUnavailable، دریافت: {other:?}"),
    }
    assert_eq!(
        run.summary.key_store_error.as_deref(),
        Some("مخزن کلید قفل است"),
        "خطای مخزن کلید باید در خلاصه هم بیاید"
    );
    assert_eq!(h.video_bytes(&video), before);
}

/// سناریو ۸: شکست دانلود → Failed تایپ‌شده با دلیل واقعی، نه Skipped عمومی.
#[test]
fn download_failure_is_typed_not_masked() {
    let h = Harness::new("download-fail");
    h.set_ffs("success");
    h.set_subsource_tool("fail-only");
    let video = h.video("Show.S01E09.mkv");
    h.probe_json(&video, JSON_NONE);
    let before = h.video_bytes(&video);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Present("k-test"),
        Some(h.dir.clone()),
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    match run.outcome(&video) {
        ItemOutcome::Failed { reason } => {
            assert!(
                reason.contains("دانلود ناموفق"),
                "دلیل باید شکست دانلود را بگوید: {reason}"
            );
        }
        other => panic!("انتظار Failed، دریافت: {other:?}"),
    }
    assert_eq!(h.video_bytes(&video), before);
    assert!(run.summary.counts.failed >= 1);
}

/// سناریو ۱۰: آرشیو با ابهام → تصمیم صریح کاربر → عضو انتخابی سینک می‌شود.
#[test]
fn ambiguous_archive_requires_explicit_user_choice() {
    let h = Harness::sidecar_mode("archive-ambiguous");
    h.set_ffs("success");
    h.set_subsource_tool("write-manifest-archives-ambiguous");
    h.set_seven_zip_tool();
    let video = h.video("Show.S01E10.mkv");
    h.probe_json(&video, JSON_NONE);
    let before = h.video_bytes(&video);

    let st = Arc::clone(&h.state);
    let resolver: Resolver = Arc::new(move |decision: &PendingDecision| {
        assert_eq!(
            decision.kind,
            crate::pipeline::DecisionKind::ArchiveCandidate
        );
        assert_eq!(
            decision.options.len(),
            2,
            "دو عضو منطبق باید دو گزینه بدهند"
        );
        // گزینهٔ دوم (fa.2.srt) انتخاب می‌شود
        resolve_decision_core(&st, &decision.decision_id, &decision.options[1].option_id)
            .expect("resolve");
    });

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        Some(resolver),
        KeyChoice::Present("k-test"),
        Some(h.dir.clone()),
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert_eq!(run.decisions.len(), 1);
    assert!(matches!(run.outcome(&video), ItemOutcome::Synced { .. }));
    // عضو انتخابی (fa.2.srt) — همان محتوای اعضای آرشیو
    assert_eq!(
        fs::read(placed_path(&video)).unwrap(),
        ARCHIVE_MEMBER_FA.as_bytes()
    );
    assert_eq!(h.video_bytes(&video), before);
}

/// سناریو ۱۱: آرشیو خراب/غیرقابل‌فهم → نتیجهٔ صادقانهٔ پایانی (طبق قرارداد
/// موتور آرشیو: محتوای بدون magic = بدون تطبیق)؛ ویدیو و هیچ فایلی
/// دست‌نخورده نمی‌ماند و staging برای بازیابی حفظ می‌شود.
#[test]
fn corrupted_archive_fails_without_touching_anything() {
    let h = Harness::new("archive-corrupt");
    h.set_ffs("success");
    // آرشیو با بایت‌های بی‌معنی (بدون magic) — قرارداد موتور آرشیو:
    // فهرست خالی → بدون تطبیق؛ تشخیص خرابیِ magic خودش در تست‌های موتور پوشش دارد
    h.set_subsource_tool("write-archives");
    h.set_seven_zip_tool();
    let video = h.video("Show.S01E11.mkv");
    h.probe_json(&video, JSON_NONE);
    let before = h.video_bytes(&video);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Present("k-test"),
        Some(h.dir.clone()),
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    match run.outcome(&video) {
        ItemOutcome::Skipped { reason } => {
            assert!(!reason.is_empty(), "دلیل باید صادقانه گزارش شود");
        }
        other => panic!("انتظار Skipped، دریافت: {other:?}"),
    }
    assert!(!placed_path(&video).exists());
    assert!(!extracted_path(&video).exists());
    assert_eq!(h.video_bytes(&video), before);
    // staging دور ریخته نشده — قابل بازیابی برای بررسی دستی
    assert!(
        !run.summary.staged_paths.is_empty(),
        "ریشه‌های staging باید حفظ و گزارش شوند"
    );
}

/// سناریو ۱۳: شکست ffsubsync → Failed؛ ورودی‌ها دست‌نخورده؛ هیچ جانمایی.
#[test]
fn ffsubsync_failure_keeps_inputs_untouched() {
    let h = Harness::new("ffs-fail");
    h.set_ffs("fail");
    h.set_ffmpeg_mode("success");
    let video = h.video("Show.S01E13.mkv");
    h.probe_json(&video, JSON_NONE);
    let sub = h.sidecar(&video, ".fa.srt", SRT_FA_A);
    let before = h.video_bytes(&video);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    match run.outcome(&video) {
        ItemOutcome::Failed { reason } => assert!(!reason.is_empty()),
        other => panic!("انتظار Failed، دریافت: {other:?}"),
    }
    assert!(
        !placed_path(&video).exists(),
        "با شکست سینک جانمایی نباید رخ دهد"
    );
    assert_eq!(
        fs::read(&sub).unwrap(),
        SRT_FA_A.as_bytes(),
        "sidecar دست‌نخورده"
    );
    assert_eq!(h.video_bytes(&video), before);
}

/// سناریو ۱۸+۲۲: تعارض نام خروجی → هیچ بازنویسی؛ هر دو فایل حفظ؛
/// workspace موفق اما بی‌جانمایی برای بازیابی حفظ می‌شود.
#[test]
fn output_collision_preserves_both_files_and_recovers_workspace() {
    let h = Harness::sidecar_mode("collision");
    h.set_ffs("success");
    h.set_ffmpeg_mode("success");
    let video = h.video("Show.S01E14.mkv");
    h.probe_json(&video, JSON_NONE);
    h.sidecar(&video, ".fa.srt", SRT_FA_A);
    let existing = placed_path(&video);
    fs::write(&existing, b"EXISTING SYNCED SUBTITLE").unwrap();
    let before = h.video_bytes(&video);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    match run.outcome(&video) {
        ItemOutcome::OutputConflict { preserved_path } => {
            assert_eq!(Path::new(preserved_path), existing);
        }
        other => panic!("انتظار OutputConflict، دریافت: {other:?}"),
    }
    // فایل موجود بازنویسی نشده
    assert_eq!(
        fs::read(&existing).unwrap(),
        b"EXISTING SYNCED SUBTITLE".to_vec()
    );
    // کاندیدای سینک‌شده در workspace برای بازیابی دستی حفظ شده
    let ws = run
        .artifacts(&video)
        .sync_candidate_path
        .clone()
        .expect("مسیر workspace باید گزارش شود");
    let ws_root = PathBuf::from(&ws);
    assert!(ws_root.is_dir(), "workspace حفظ‌شده باید موجود باشد: {ws}");
    let candidate = ws_root.join("out").join(
        placed_path(&video)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    );
    assert_eq!(
        fs::read(&candidate).unwrap(),
        SRT_FA_A.as_bytes(),
        "کاندیدای سینک‌شده باید در workspace قابل بازیابی باشد"
    );
    assert_eq!(run.summary.counts.conflicts, 1);
    assert_eq!(h.video_bytes(&video), before);
}

/// سناریو ۱۹+۲۰: حذف منبع در میانهٔ اجرا → فقط همان ویدیو Fail می‌شود؛
/// ویدیوی دیگر به کار خود ادامه می‌دهد.
#[test]
fn missing_source_midrun_fails_only_that_video() {
    let h = Harness::sidecar_mode("missing-midrun");
    h.set_ffs("success");
    h.set_ffmpeg_mode("success");
    // Alpha: دو تراک فارسی → تصمیم (قلاب هم‌زمان‌بندی‌شده برای حذف Beta)
    let alpha = h.video("Alpha.S01E01.mkv");
    h.probe_json(&alpha, JSON_TWO_FA);
    // Beta: sidecar معتبر → سینک
    let beta = h.video("Beta.S01E01.mkv");
    h.probe_json(&beta, JSON_NONE);
    h.sidecar(&beta, ".fa.srt", SRT_FA_A);

    let beta_path = beta.to_string_lossy().into_owned();
    let st = Arc::clone(&h.state);
    let resolver: Resolver = Arc::new(move |decision: &PendingDecision| {
        // در لحظهٔ انتظار تصمیم، ویدیوی Beta حذف می‌شود (شبیه‌سازی حذف در میانهٔ اجرا)
        let _ = fs::remove_file(&beta_path);
        resolve_decision_core(&st, &decision.decision_id, &decision.options[0].option_id)
            .expect("resolve");
    });

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![
            alpha.to_string_lossy().into_owned(),
            beta.to_string_lossy().into_owned(),
        ],
        Some(resolver),
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    // Alpha با وجود خطای همزمان، کامل سینک و جانمایی می‌شود
    assert!(matches!(run.outcome(&alpha), ItemOutcome::Synced { .. }));
    assert_eq!(fs::read(placed_path(&alpha)).unwrap(), SRT_FA_A.as_bytes());
    // Beta شکست تایپ‌شده می‌گیرد — نه Synced، نه حذف بی‌صدا
    match run.outcome(&beta) {
        ItemOutcome::Failed { reason } => assert!(!reason.is_empty()),
        other => panic!("انتظار Failed برای ویدیوی حذف‌شده، دریافت: {other:?}"),
    }
    assert!(!placed_path(&beta).exists());
    assert_eq!(
        h.video_bytes(&alpha),
        b"FAKE-VIDEO-BYTES-DO-NOT-TOUCH".to_vec()
    );
}

/// تضمین: مسیرهای تکراری در ورودی → یک آیتم (یک job سینک به ازای هر ویدیو).
#[test]
fn duplicate_input_paths_collapse_to_one_item() {
    let h = Harness::sidecar_mode("dup-paths");
    h.set_ffs("success");
    let video = h.video("Show.S01E15.mkv");
    h.probe_json(&video, JSON_NONE);
    h.sidecar(&video, ".fa.srt", SRT_FA_A);
    let path = video.to_string_lossy().into_owned();

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![path.clone(), path.clone()],
        None,
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.items.len(), 1, "مسیر تکراری باید یکی شود");
    assert!(matches!(run.outcome(&video), ItemOutcome::Synced { .. }));
}

/// تضمین ۲۱: snapshot در میانهٔ اجرا (شبیه‌سازی reconnect فرانت)؛ تصمیم باز
/// در snapshot هم می‌آید؛ پس از پایان control حذف شده و resolve تصمیم کهنه
/// خطا می‌دهد.
#[test]
fn snapshot_midrun_supports_reconnect_and_stale_decision_is_rejected() {
    let h = Harness::sidecar_mode("reconnect");
    h.set_ffs("success");
    let video = h.video("Show.S01E16.mkv");
    h.probe_json(&video, JSON_TWO_FA); // → تصمیم

    let state_for_snapshot = Arc::clone(&h.state);
    let st = Arc::clone(&h.state);
    let resolver: Resolver = Arc::new(move |decision: &PendingDecision| {
        // شبیه‌سازی اتصال مجدد فرانت: snapshot از control منتشرشده
        let snap = state_for_snapshot
            .pipeline
            .lock()
            .unwrap()
            .as_ref()
            .map(|c| c.snapshot())
            .expect("control باید در میانهٔ تصمیم منتشر باشد");
        assert_eq!(snap.status, PipelineStatus::AwaitingDecision);
        assert_eq!(snap.total, 1);
        assert_eq!(snap.counts.decisions_pending, 1);
        // تصمیم باز در snapshot قابل بازیابی است (بدون آن UI راهی برای پاسخ ندارد)
        let pending = snap
            .pending_decision
            .expect("تصمیم باز باید در snapshot باشد");
        assert_eq!(pending.decision_id, decision.decision_id);
        assert_eq!(pending.options.len(), decision.options.len());
        resolve_decision_core(&st, &decision.decision_id, &decision.options[0].option_id)
            .expect("resolve");
    });

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        Some(resolver),
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert!(matches!(run.outcome(&video), ItemOutcome::Synced { .. }));
    // پس از پایان: control حذف شده (حالت پس از restart)
    assert!(h.state.pipeline.lock().unwrap().is_none());
    // تصمیم کهنه دیگر قابل resolve نیست
    let stale = &run.decisions[0];
    assert!(
        resolve_decision_core(&h.state, &stale.decision_id, &stale.options[0].option_id).is_err(),
        "resolve تصمیم کهنه باید خطا بدهد"
    );
}

// ---------------------------------------------------------------------------
// سناریوهای لغو (۱۴ تا ۱۷)
// ---------------------------------------------------------------------------

/// سناریو ۱۴: لغو در میانهٔ تحلیل — هیچ حالت جزئی و هیچ فایلی باقی نمی‌ماند.
#[test]
fn cancel_during_analysis_leaves_no_partial_state() {
    let h = Harness::new("cancel-analysis");
    h.set_ffs("success");
    h.set_ffmpeg_mode("success");
    h.set_slow_probe(JSON_ONE_FA);

    let video = h.video("Show.S01E17.mkv");
    let before = h.video_bytes(&video);

    // watcher لغو: صبر برای marker شروع پروب، سپس لغو
    let st = Arc::clone(&h.state);
    let marker = h.tools.join("probe_started.marker");
    let watcher = std::thread::spawn(move || {
        let start = Instant::now();
        while !marker.exists() && start.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists(), "پروب کند باید شروع شده باشد");
        assert!(
            wait_for_snapshot(&st, Duration::from_secs(10), |_| true),
            "control باید منتشر باشد"
        );
        let cancelled = cancel_active_pipeline(&st).expect("لغو باید موفق باشد");
        assert!(cancelled, "pipeline فعال باید لغو شود");
    });

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Missing,
        None,
    );
    watcher.join().expect("watcher نباید panic کند");

    assert_eq!(run.summary.status, PipelineStatus::Cancelled);
    assert!(run.summary.cancelled);
    assert!(
        matches!(run.outcome(&video), ItemOutcome::Cancelled),
        "آیتم باز باید Cancelled شود"
    );
    assert_eq!(run.summary.counts.decisions_pending, 0);
    assert!(!placed_path(&video).exists());
    assert!(!extracted_path(&video).exists());
    assert_eq!(h.video_bytes(&video), before);
}

/// سناریو ۱۵: لغو در میانهٔ دانلود — هیچ جانمایی؛ ویدیو دست‌نخورده.
#[test]
fn cancel_during_download_preserves_state() {
    let h = Harness::new("cancel-download");
    h.set_ffs("success");
    h.set_subsource_tool("write-then-slow");
    let video = h.video("Show.S01E18.mkv");
    h.probe_json(&video, JSON_NONE);
    let before = h.video_bytes(&video);

    let st = Arc::clone(&h.state);
    let watcher = std::thread::spawn(move || {
        // صبر تا ورود به مرحلهٔ دانلود، سپس لغو
        assert!(
            wait_for_snapshot(&st, Duration::from_secs(20), |snap| {
                snap.stage == PipelineStage::Downloading
            }),
            "باید به مرحلهٔ دانلود برسد"
        );
        let cancelled = cancel_active_pipeline(&st).expect("لغو");
        assert!(cancelled);
    });

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Present("k-test"),
        Some(h.dir.clone()),
    );
    watcher.join().expect("watcher");

    assert_eq!(run.summary.status, PipelineStatus::Cancelled);
    assert!(matches!(run.outcome(&video), ItemOutcome::Cancelled));
    assert!(!placed_path(&video).exists());
    assert_eq!(h.video_bytes(&video), before);
}

/// سناریو ۱۶: لغو در انتظار تصمیم — waiter بیدار می‌شود؛ هیچ استخراجی رخ
/// نداده؛ resolve تصمیم کهنه پس از پایان خطا می‌دهد.
#[test]
fn cancel_while_awaiting_decision_cleans_pending_state() {
    let h = Harness::new("cancel-decision");
    h.set_ffs("success");
    let video = h.video("Show.S01E19.mkv");
    h.probe_json(&video, JSON_TWO_FA);
    let before = h.video_bytes(&video);

    let stale: Arc<Mutex<Option<(String, String)>>> = Arc::new(Mutex::new(None));
    let st = Arc::clone(&h.state);
    let stale_for_resolver = Arc::clone(&stale);
    let resolver: Resolver = Arc::new(move |decision: &PendingDecision| {
        // decision را ثبت می‌کنیم برای تست تصمیم کهنه پس از پایان
        *stale_for_resolver.lock().unwrap() = Some((
            decision.decision_id.clone(),
            decision.options[0].option_id.clone(),
        ));
        // لغو بلافاصله پس از ثبت تصمیم — بدون resolve
        let cancelled = cancel_active_pipeline(&st).expect("لغو");
        assert!(cancelled);
    });

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        Some(resolver),
        KeyChoice::Missing,
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Cancelled);
    assert!(matches!(run.outcome(&video), ItemOutcome::Cancelled));
    assert_eq!(run.summary.counts.decisions_pending, 0);
    // هیچ استخراج/سینکی رخ نداده
    assert!(!extracted_path(&video).exists());
    assert!(!placed_path(&video).exists());
    assert_eq!(h.video_bytes(&video), before);
    // تصمیم کهنه پس از پایان run قابل resolve نیست
    let stale_pair = stale.lock().unwrap().take().expect("تصمیم ثبت شده");
    assert!(
        resolve_decision_core(&h.state, &stale_pair.0, &stale_pair.1).is_err(),
        "resolve تصمیم کهنه باید خطا بدهد"
    );
}

/// حین سینک، وضعیت زنده منتشر می‌شود: فایل جاری، تعداد فعال، پیام واقعی.
/// `completed` تا قبل از outcome نهایی صفر می‌ماند و seq یکنواست.
#[test]
fn sync_publishes_live_activity_before_completion() {
    let h = Harness::new("sync-live");
    h.set_ffs("success");
    let video = h.video("Show.S01E08.mkv");
    h.probe_json(&video, JSON_NONE);
    h.sidecar(&video, ".fa.srt", SRT_FA_A);

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Missing,
        None,
    );
    assert_eq!(run.summary.status, PipelineStatus::Completed);

    let states: Vec<&PipelineSnapshot> = run
        .events
        .iter()
        .filter_map(|e| match e {
            PipelineEvent::State { snapshot } => Some(snapshot),
            _ => None,
        })
        .collect();
    assert!(
        states.iter().any(|s| s.stage == PipelineStage::Syncing),
        "حین سینک باید snapshot منتشر شود، نه فقط در پایان"
    );
    let started = states
        .iter()
        .find(|s| s.stage == PipelineStage::Syncing && s.active > 0)
        .expect("snapshot سینک باید فایل فعال را نشان دهد");
    assert_eq!(started.current_file.as_deref(), Some("Show.S01E08.mkv"));
    assert!(started.message.contains("همگام‌سازی"));
    assert_eq!(started.completed, 0, "completed فقط از outcome نهایی");
    assert!(!started.cancelling);

    let item_events: Vec<&PipelineItemSnapshot> = run
        .events
        .iter()
        .filter_map(|e| match e {
            PipelineEvent::Item { item } => Some(item),
            _ => None,
        })
        .collect();
    assert!(
        item_events.iter().any(|i| {
            i.stage == PipelineStage::Syncing && i.stage_status == StageStatus::InProgress
        }),
        "رویداد آیتم باید شروع سینک را نشان دهد"
    );

    let seqs: Vec<u64> = states.iter().map(|s| s.seq).collect();
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "seq باید اکیداً یکنوا باشد: {seqs:?}"
    );
}

/// سناریو ۱۷: لغو در میانهٔ سینک — هیچ جانمایی؛ sidecar دست‌نخورده.
#[test]
fn cancel_during_sync_places_nothing() {
    let h = Harness::new("cancel-sync");
    h.set_ffs("slow"); // ~۳۰ ثانیه
    h.set_ffmpeg_mode("success");
    let video = h.video("Show.S01E20.mkv");
    h.probe_json(&video, JSON_NONE);
    let sub = h.sidecar(&video, ".fa.srt", SRT_FA_A);
    let before = h.video_bytes(&video);

    let st = Arc::clone(&h.state);
    let watcher = std::thread::spawn(move || {
        // در حین سینک، UI باید فایل جاری و تعداد فعال را ببیند — نه فقط
        // پیام مرحله. این همان پلی است که قبلاً وجود نداشت.
        assert!(
            wait_for_snapshot(&st, Duration::from_secs(20), |snap| {
                snap.stage == PipelineStage::Syncing
                    && snap.active > 0
                    && snap.current_file.as_deref() == Some("Show.S01E20.mkv")
                    && snap.message.contains("Show.S01E20.mkv")
            }),
            "در حین سینک باید فایل جاری/تعداد فعال/پیام زنده منتشر شود"
        );
        let cancelled = cancel_active_pipeline(&st).expect("لغو");
        assert!(cancelled);
    });

    let run = run_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
        KeyChoice::Missing,
        None,
    );
    watcher.join().expect("watcher");

    assert_eq!(run.summary.status, PipelineStatus::Cancelled);
    assert!(matches!(run.outcome(&video), ItemOutcome::Cancelled));
    assert!(
        !placed_path(&video).exists(),
        "با لغو هیچ جانمایی نباید رخ دهد"
    );
    assert_eq!(
        fs::read(&sub).unwrap(),
        SRT_FA_A.as_bytes(),
        "sidecar دست‌نخورده"
    );
    assert_eq!(h.video_bytes(&video), before);
}

// ---------------------------------------------------------------------------
// تضمین‌های ایمنی مستقل
// ---------------------------------------------------------------------------

/// تضمین: cancel_pipeline فقط control منتشرشده را هدف می‌گیرد. عملیات تکی
/// نامرتبط (در registry دیگر) هرگز پرچم لغو آن ست نمی‌شود؛ admission هم
/// در طول pipeline فعال، عملیات تکی دیگر را در همان registry رد می‌کند.
#[test]
fn cancel_pipeline_spares_unrelated_single_ops() {
    let h = Harness::new("cancel-scope");
    let guard = acquire_batch_op(&h.state.ops).expect("batch");
    let control = Arc::new(PipelineControl::from_guard(
        &guard,
        "run-scope".into(),
        "تست",
    ));
    {
        let mut published = h.state.pipeline.lock().unwrap();
        *published = Some(Arc::clone(&control));
    }
    // عملیات تکی نامرتبط روی registry مستقل (مثل کاربر دیگری که همان لحظه
    // عملیات تکی خودش را دارد)
    let other_state = Arc::new(AppState {
        settings: Mutex::new(Settings::default()),
        library: Mutex::new(Vec::new()),
        last_scan: Mutex::new(None),
        ops: Arc::new(Mutex::new(OpsRegistry::default())),
        archive_provenance: Mutex::new(ProvenanceRegistry::default()),
        pipeline: Mutex::new(None),
        hardsub: Mutex::new(None),
        hardsub_encoder_cache: Mutex::new(None),
        setup_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    let single = acquire_single_op(&other_state.ops, "C:\\other\\video.mkv").expect("single");
    assert!(!single.cancel.load(Ordering::SeqCst));

    // admission: در حین pipeline فعال، عملیات تکی/گروهی دیگر در همان registry رد می‌شود
    assert!(
        acquire_batch_op(&h.state.ops).is_err(),
        "slot batch همچنان اشغال است"
    );
    assert!(
        acquire_single_op(&h.state.ops, "C:\\other\\video2.mkv").is_err(),
        "عملیات تکی در حین batch باید رد شود"
    );

    let cancelled = cancel_active_pipeline(&h.state).expect("لغو");
    assert!(cancelled);
    // پرچم batch ست شده (همان Arc پرچم control)
    assert!(control.is_cancelled());
    // پرچم عملیات تکیِ نامرتبط دست‌نخورده — لغو هرگز سرریز نمی‌شود
    assert!(
        !single.cancel.load(Ordering::SeqCst),
        "لغو pipeline نباید عملیات تکی نامرتبط را لغو کند"
    );
    // بدون pipeline منتشرشده، لغو بی‌اثر و بی‌خطر است
    drop(control);
    // control هنوز منتشر است (drop(control) فقط Arc محلی را کم می‌کند)
    let published = h.state.pipeline.lock().unwrap().as_ref().map(Arc::clone);
    if let Some(c) = published {
        assert!(cancel_active_pipeline(&h.state).expect("لغو دوباره"));
        assert!(c.is_cancelled());
    }
    drop(single);
    drop(guard);
}
