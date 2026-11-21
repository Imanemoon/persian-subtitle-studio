//! اعتبارسنجی real-world — pipeline واقعی با ابزارهای واقعی (FFmpeg/FFprobe
//! از PATH، FFsubsync واقعی، sub-rs واقعی) روی نمونه‌های ویدیویی واقعی.
//!
//! قواعد حفاظت از آرشیو:
//! - همهٔ تست‌ها فقط روی «کپی»های قابل‌دورریز داخل workspace ایزوله اجرا
//!   می‌شوند؛ هیچ مسیری از آرشیو منبع نوشتنی نیست.
//! - مسیرها هرگز در کد کامیت نمی‌شوند — فقط از متغیرهای محیطی می‌آیند تا
//!   نام فایل‌های خصوصی کاربر در ریپو ظاهر نشود.
//! - هر تست `#[ignore]` است؛ CI هرگز اجرایش نمی‌کند. اجرای دستی:
//!   ```text
//!   set SUBSYNC_REAL_DIR=D:\...\SubSync-SoftSub-RealWorld-Tests
//!   set SUBSYNC_REAL_SAMPLE_A=D:\...\samples\sample-a.mkv
//!   set SUBSYNC_REAL_SAMPLE_MULTI=...\sample-multi.mkv
//!   set SUBSYNC_REAL_SAMPLE_NO_SUB=...\sample-nosub.mkv
//!   set SUBSYNC_REAL_SAMPLE_NO_SUB_MP4=...\sample-nosub.mp4
//!   set SUBSYNC_REAL_SAMPLE_UNTAGGED=...\sample-untagged.mkv
//!   set SUBSYNC_REAL_SAMPLE_SIDECAR=...\sample-sidecar.mkv
//!   set SUBSYNC_REAL_FFSUBSYNC=D:\...\ffsubsync-venv\Scripts\ffsubsync.exe
//!   cargo test --lib pipeline::real_world -- --ignored --test-threads=2
//!   ```

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use super::engine::{run_worker, PipelineIo};
use super::types::{
    ItemOutcome, PendingDecision, PipelineEvent, PipelineInput, PipelineStatus, PipelineSummary,
};
use super::{resolve_decision_core, PipelineControl, WorkerCleanup};
use crate::archive::provenance::ProvenanceRegistry;
use crate::settings::{Settings, SubtitleOutputMode};
use crate::state::{acquire_batch_op, AppState, OpsRegistry};

type Resolver = Arc<dyn Fn(&PendingDecision) + Send + Sync>;

// ---------------------------------------------------------------------------
// پیکربندی از محیط — بدون هیچ مسیر کامیت‌شده
// ---------------------------------------------------------------------------

fn real_dir() -> PathBuf {
    std::env::var_os("SUBSYNC_REAL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("SUBSYNC_REAL_DIR تنظیم نشده — workspace ایزوله لازم است"))
}

fn sample(key: &str) -> PathBuf {
    std::env::var_os(format!("SUBSYNC_REAL_SAMPLE_{key}"))
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("SUBSYNC_REAL_SAMPLE_{key} تنظیم نشده"))
}

fn ffsubsync_tool() -> PathBuf {
    std::env::var_os("SUBSYNC_REAL_FFSUBSYNC")
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("SUBSYNC_REAL_FFSUBSYNC تنظیم نشده — سینک واقعی لازم است"))
}

fn real_ffmpeg() -> PathBuf {
    which("ffmpeg").expect("FFmpeg واقعی باید در PATH باشد")
}

fn real_ffprobe() -> PathBuf {
    which("ffprobe").expect("FFprobe واقعی باید در PATH باشد")
}

fn which(exe: &str) -> Option<PathBuf> {
    let dirs = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&dirs) {
        let candidate = dir.join(format!("{exe}.exe"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// هارنس واقعی
// ---------------------------------------------------------------------------

struct RealHarness {
    state: Arc<AppState>,
    /// ریشهٔ اجرای این تست داخل workspace — همهٔ نوشته‌ها فقط اینجا
    run_root: PathBuf,
}

impl RealHarness {
    /// `skip_low_quality=true` = پیش‌فرض اپ (گیت کیفیت ffsubsync فعال)؛
    /// `false` = پیکربندی صریح کاربر در UI که زنجیرهٔ کامل sync→mux را
    /// حتی برای زیرنویسِ از-قبل-سینک‌شده اجرا می‌کند.
    fn new(tag: &str, output_mode: SubtitleOutputMode, skip_low_quality: bool) -> Self {
        let dir = real_dir().join(format!("out-{tag}-{}", unique_token()));
        fs::create_dir_all(&dir).unwrap();

        let mut settings = Settings::default();
        settings.excluded_folders.clear();
        settings.processing.subtitle_output_mode = output_mode;
        settings.processing.skip_sync_on_low_quality = skip_low_quality;
        settings.tool_paths.ffsubsync = Some(ffsubsync_tool().to_string_lossy().into_owned());
        if let Some(sub) = std::env::var_os("SUBSYNC_REAL_SUBSOURCE") {
            settings.tool_paths.subsource = Some(PathBuf::from(sub).to_string_lossy().into_owned());
        }

        let state = Arc::new(AppState {
            settings: Mutex::new(settings),
            library: Mutex::new(Vec::new()),
            last_scan: Mutex::new(None),
            ops: Arc::new(Mutex::new(OpsRegistry::default())),
            archive_provenance: Mutex::new(ProvenanceRegistry::default()),
            pipeline: Mutex::new(None),
            hardsub: Mutex::new(None),
            hardsub_encoder_cache: Mutex::new(None),
            setup_cancel: Arc::new(AtomicBool::new(false)),
        });
        RealHarness {
            state,
            run_root: dir,
        }
    }

    /// کپی قابل‌دورریز از نمونه + تضمین یکپارچگی کپی (hash برابر نمونهٔ مبدأ)
    fn copy_sample(&self, key: &str, rename_to: &str) -> PathBuf {
        let source = sample(key);
        let dest = self.run_root.join(rename_to);
        fs::copy(&source, &dest).expect("کپی نمونه");
        assert_eq!(
            sha256_file(&source),
            sha256_file(&dest),
            "کپی باید بایت‌به‌بایت با نمونه یکسان باشد"
        );
        dest
    }

    /// کپی از هر مسیر مطلق خارجی (مثلاً ویدیوی واقعی آرشیو کاربر) به
    /// workspace ایزوله — منبع فقط خوانده می‌شود.
    fn copy_external(&self, source: &Path, rename_to: &str) -> PathBuf {
        assert!(
            source.is_file(),
            "منبع خارجی موجود نیست: {}",
            source.display()
        );
        let dest = self.run_root.join(rename_to);
        fs::copy(source, &dest).expect("کپی نمونهٔ خارجی");
        assert_eq!(
            sha256_file(source),
            sha256_file(&dest),
            "کپی باید بایت‌به‌بایت با منبع یکسان باشد"
        );
        dest
    }

    fn video_hash(&self, video: &Path) -> String {
        sha256_file(video)
    }
}

impl Drop for RealHarness {
    fn drop(&mut self) {
        // پاک‌سازی فقط داخل workspace ایزوله — هرگز مسیر بیرونی
        let _ = fs::remove_dir_all(&self.run_root);
    }
}

struct RealRun {
    summary: PipelineSummary,
    decisions: Vec<PendingDecision>,
}

fn run_real_pipeline(
    state: Arc<AppState>,
    paths: Vec<String>,
    resolver: Option<Resolver>,
) -> RealRun {
    let guard = acquire_batch_op(&state.ops).expect("admission باید موفق باشد");
    let run_id = format!("run-real-{}", unique_token());
    let control = Arc::new(PipelineControl::from_guard(
        &guard,
        run_id.clone(),
        "تست real-world",
    ));
    {
        let mut published = state.pipeline.lock().unwrap();
        *published = Some(Arc::clone(&control));
    }

    let events: Arc<Mutex<Vec<PipelineEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let input = PipelineInput::Files { paths };

    let seen_decisions: Arc<Mutex<std::collections::HashSet<String>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));
    let events_for_sink = Arc::clone(&events);
    let seen_for_sink = Arc::clone(&seen_decisions);
    let sink: super::SinkStore = Arc::new(Mutex::new(Box::new(move |event: PipelineEvent| {
        let pending = match &event {
            PipelineEvent::State { snapshot } => snapshot.pending_decision.clone(),
            _ => None,
        };
        events_for_sink.lock().unwrap().push(event);
        if let (Some(res), Some(dec)) = (&resolver, pending) {
            let fresh = seen_for_sink
                .lock()
                .unwrap()
                .insert(dec.decision_id.clone());
            if fresh {
                res(&dec);
            }
        }
    })));

    // کلید API واقعی هرگز لمس نمی‌شود — مثل نصبِ بدون کلید (Ok(None))
    let key_fn: Arc<dyn Fn() -> Result<Option<String>, String> + Send + Sync> =
        Arc::new(|| Ok(None));

    let state_for_worker = Arc::clone(&state);
    let handle = std::thread::spawn(move || {
        let io = PipelineIo {
            key_provider: &*key_fn,
            staging_base: None,
        };
        let cleanup = WorkerCleanup::new(&state_for_worker, run_id, guard);
        run_worker(&state_for_worker, &control, cleanup, input, &sink, &io);
    });
    handle.join().expect("worker نباید panic کند");

    assert!(state.pipeline.lock().unwrap().is_none());

    let mut decisions = Vec::new();
    let summary = {
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
        evs.iter()
            .find_map(|e| match e {
                PipelineEvent::Done { summary } => Some(summary.clone()),
                _ => None,
            })
            .expect("رویداد Done باید صادر شده باشد")
    };
    RealRun { summary, decisions }
}

/// resolver تصمیم‌ها مثل کاربر واقعی: اولین گزینهٔ هر تصمیم انتخاب می‌شود
/// (برای ReviewTrack یعنی تأیید Track پیشنهادی).
fn first_option_resolver(state: &Arc<AppState>) -> Resolver {
    let state = Arc::clone(state);
    Arc::new(move |decision: &PendingDecision| {
        let first = decision
            .options
            .first()
            .expect("تصمیم باید حداقل یک گزینه داشته باشد")
            .option_id
            .clone();
        resolve_decision_core(&state, &decision.decision_id, &first).expect("resolve real-world");
    })
}

fn unique_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}-{}", std::process::id())
}

fn sha256_file(path: &Path) -> String {
    let mut file = fs::File::open(path).expect("باز کردن فایل برای هش");
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).expect("خواندن فایل برای هش");
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// راستی‌آزمایی با FFmpeg/FFprobe واقعی
// ---------------------------------------------------------------------------

struct StreamInfo {
    index: u32,
    codec_type: String,
    codec_name: String,
    language: Option<String>,
}

fn full_probe(video: &Path) -> Vec<StreamInfo> {
    let output = Command::new(real_ffprobe())
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=index,codec_type,codec_name:stream_tags=language,title",
            "-of",
            "json",
        ])
        .arg(video)
        .stdin(Stdio::null())
        .output()
        .expect("اجرای ffprobe واقعی");
    assert!(output.status.success(), "ffprobe شکست خورد روی {video:?}");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("پارس JSON");
    json["streams"]
        .as_array()
        .expect("آرایهٔ streams")
        .iter()
        .map(|s| StreamInfo {
            index: s["index"].as_u64().unwrap_or(u64::MAX) as u32,
            codec_type: s["codec_type"].as_str().unwrap_or("").to_string(),
            codec_name: s["codec_name"].as_str().unwrap_or("").to_string(),
            language: s["tags"]["language"].as_str().map(str::to_string),
        })
        .collect()
}

/// MD5 بیت‌استریم یک استریم (stream-copy) — برای اثبات حفظ دقیق packetها.
/// map_spec باید ordinal نوع استریم باشد (مثل 0:v:0 یا 0:s:1).
fn stream_md5(video: &Path, map_spec: &str) -> Option<String> {
    let output = Command::new(real_ffmpeg())
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(video)
        .args(["-map", map_spec, "-c", "copy", "-f", "md5", "-"])
        .stdin(Stdio::null())
        .output()
        .expect("اجرای ffmpeg برای hash استریم");
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim().strip_prefix("MD5=").map(str::to_string)
}

/// ordinal نوع استریم برای index سراسری (مثلاً stream سراسری ۶ = 0:s:0 اگر
/// اولین زیرنویسِ فایل باشد) — لازم برای -map دقیق.
fn stream_ordinal(probe: &[StreamInfo], kind: &str, global_index: u32) -> usize {
    probe
        .iter()
        .filter(|s| s.codec_type == kind && s.index < global_index)
        .count()
}

/// اعتبارسنجی decode کامل فایل — هر خطای بیت باید شکست بخورد (-xerror).
fn decode_ok(video: &Path) -> Result<(), String> {
    let output = Command::new(real_ffmpeg())
        .args(["-nostdin", "-hide_banner", "-v", "error", "-xerror", "-i"])
        .arg(video)
        .args(["-f", "null", "-"])
        .stdin(Stdio::null())
        .output()
        .expect("اجرای ffmpeg برای decode validation");
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr)
            .lines()
            .take(3)
            .collect::<Vec<_>>()
            .join(" | "))
    }
}

fn streams_of<'a>(probe: &'a [StreamInfo], kind: &str) -> Vec<&'a StreamInfo> {
    probe.iter().filter(|s| s.codec_type == kind).collect()
}

fn fa_tagged_count(probe: &[StreamInfo]) -> usize {
    streams_of(probe, "subtitle")
        .iter()
        .filter(|s| {
            s.language
                .as_deref()
                .map(|l| matches!(l.to_lowercase().as_str(), "fa" | "fas" | "per" | "fa-ir"))
                .unwrap_or(false)
        })
        .count()
}

/// بدون sidecar سینک‌شدهٔ جدید، بدون workspace موقت mux و بدون backup.
fn assert_no_softsub_leftover(video: &Path) {
    let stem = video.file_stem().unwrap().to_string_lossy();
    let synced = video
        .parent()
        .unwrap()
        .join(format!("{stem}.fa.synced.srt"));
    assert!(
        !synced.exists(),
        "نباید sidecar سینک‌شدهٔ خارجی ساخته شود: {}",
        synced.display()
    );
    for entry in fs::read_dir(video.parent().unwrap()).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(
            !name.starts_with(".subsync-softsub-"),
            "workspace موقت mux باید پاک شده باشد: {name}"
        );
        assert!(
            !name.ends_with(".original-backup"),
            "backup ویدیو نباید باقی مانده باشد: {name}"
        );
    }
}

/// srt فارسی مصنوعی با offset مشخص — مثل زیرنویس دانلودیِ ناهم‌زمان که
/// فاز آرشیو کنار ویدیو می‌گذارد؛ سینک واقعی ffsubsync باید اصلاحش کند.
fn synthetic_fa_srt(duration_secs: f64, offset: f64) -> String {
    let mut out = String::new();
    let mut index = 1u32;
    let mut start = 1.0f64;
    let lines = [
        "در جنگل‌های سرسبز شمال، صدای پرندگان می‌پیچد.",
        "او به آرامی پنجره را باز کرد و به آسمان نگاه کرد.",
        "این داستان دربارهٔ دوستی و شجاعت است.",
        "زنگ تلفن در سکوت شب پیچید.",
        "مادرش گفت: دیر شده؛ باید برگردیم.",
    ];
    while start < duration_secs - 6.0 {
        let (s, e) = (start + offset, start + offset + 3.5);
        let (s, e) = (s.max(0.1), e.max(0.5));
        out.push_str(&format!(
            "{index}\n{:02}:{:02}:{:02},{:03} --> {:02}:{:02}:{:02},{:03}\n{}\n\n",
            s as u64 / 3600,
            (s as u64 % 3600) / 60,
            s as u64 % 60,
            ((s - s.floor()) * 1000.0) as u64,
            e as u64 / 3600,
            (e as u64 % 3600) / 60,
            e as u64 % 60,
            ((e - e.floor()) * 1000.0) as u64,
            lines[(index as usize - 1) % lines.len()],
        ));
        index += 1;
        start += 5.0;
    }
    out
}

fn ffprobe_duration(video: &Path) -> f64 {
    let output = Command::new(real_ffprobe())
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(video)
        .stdin(Stdio::null())
        .output()
        .expect("اجرای ffprobe برای duration");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .split(',')
        .next()
        .unwrap_or("0")
        .parse()
        .unwrap_or(0.0)
}

impl RealRun {
    fn outcome_of(&self, video: &Path) -> &ItemOutcome {
        let path = video.to_string_lossy();
        self.summary
            .items
            .iter()
            .find(|i| i.video_path == path)
            .unwrap_or_else(|| panic!("آیتم {path} در خلاصه نیست"))
            .outcome
            .as_ref()
            .unwrap_or_else(|| panic!("آیتم {path} outcome ندارد"))
    }
}

// ---------------------------------------------------------------------------
// آزمون‌های واقعی
// ---------------------------------------------------------------------------

/// آزمون A — MKV با Track متنی فارسی تگ‌دار → استخراج واقعی → سینک واقعی
/// (ffsubsync 0.5.1) → remux واقعی (ffmpeg 8.0) → SoftSub داخل ویدیو؛
/// بدون sidecar خارجی، بدون Track تکراری، بدون backup/workspace باقی‌مانده.
#[test]
#[ignore = "فقط اجرای دستی real-world — به SUBSYNC_REAL_* و ابزارهای واقعی نیاز دارد"]
fn real_a_embedded_fa_track_becomes_softsub() {
    let h = RealHarness::new("real-a", SubtitleOutputMode::SoftSub, false);
    let video = h.copy_sample("A", "Show.S01E01.mkv");
    let before = h.video_hash(&video);
    let before_probe = full_probe(&video);
    assert_eq!(before_probe.len(), 3, "نمونهٔ A باید ویدیو+صدا+زیرنویس باشد");
    assert_eq!(fa_tagged_count(&before_probe), 1);

    let run = run_real_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    match run.outcome_of(&video) {
        ItemOutcome::SoftSubbed { video_path } => {
            assert_eq!(Path::new(video_path), video.as_path());
        }
        other => panic!("انتظار SoftSubbed، دریافت: {other:?}"),
    }

    let after = h.video_hash(&video);
    assert_ne!(after, before, "remux باید فایل را جایگزین کرده باشد");

    let after_probe = full_probe(&video);
    assert_eq!(after_probe.len(), 3, "تعداد استریم‌ها باید حفظ شود");
    assert_eq!(streams_of(&after_probe, "video")[0].codec_name, "h264");
    assert_eq!(streams_of(&after_probe, "audio")[0].codec_name, "aac");
    let subs = streams_of(&after_probe, "subtitle");
    assert_eq!(subs.len(), 1, "دقیقاً یک Track زیرنویس");
    assert_eq!(subs[0].codec_name, "subrip");
    assert_eq!(
        subs[0].language.as_deref(),
        Some("fas"),
        "زیرنویس جدید باید language=fas بگیرد"
    );
    assert_eq!(fa_tagged_count(&after_probe), 1, "بدون Track فارسی تکراری");

    // حفظ بیت‌استریم ویدیو و صدا (stream-copy واقعی) — مقایسه با نمونهٔ اصلی
    let pristine = sample("A");
    let v_md5_before = stream_md5(&pristine, "0:v:0").expect("hash ویدیوی مبدأ");
    let v_md5_after = stream_md5(&video, "0:v:0").expect("hash ویدیوی خروجی");
    assert_eq!(
        v_md5_before, v_md5_after,
        "ویدیو باید بدون re-encode کپی شده باشد"
    );
    let a_md5_before = stream_md5(&pristine, "0:a:0").expect("hash صدای مبدأ");
    let a_md5_after = stream_md5(&video, "0:a:0").expect("hash صدای خروجی");
    assert_eq!(
        a_md5_before, a_md5_after,
        "صدا باید بدون تغییر کپی شده باشد"
    );

    // فایل استخراج موقتِ همین اجرا باید پاک شده باشد
    let stem = video.file_stem().unwrap().to_string_lossy();
    assert!(
        !video
            .parent()
            .unwrap()
            .join(format!("{stem}.fa.srt"))
            .exists(),
        "زیرنویس استخراجِ موقت باید پس از mux موفق پاک شود"
    );
    assert_no_softsub_leftover(&video);
    decode_ok(&video).expect("خروجی باید بدون خطای decode پخش‌پذیر باشد");
}

/// آزمون A2 — رفتار پیش‌فرض اپ (گیت کیفیت فعال) برای زیرنویس داخلیِ
/// از-قبل-سینک‌شده: ffsubsync واقعی روی چنین محتوایی (مثل کمدی دیالوگ‌محور)
/// score منفی می‌دهد و اپ باید صادقانه UnchangedLowQuality بدهد؛ ویدیو
/// دست‌نخورده می‌ماند و sidecar استخراج‌شده برای بازبینی حفظ می‌شود.
#[test]
#[ignore = "فقط اجرای دستی real-world — به SUBSYNC_REAL_* و ابزارهای واقعی نیاز دارد"]
fn real_a2_default_settings_keep_already_synced_video_untouched() {
    let h = RealHarness::new("real-a2", SubtitleOutputMode::SoftSub, true);
    let video = h.copy_sample("A", "Show.S01E01.mkv");
    let before = h.video_hash(&video);

    let run = run_real_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert!(
        matches!(run.outcome_of(&video), ItemOutcome::UnchangedLowQuality),
        "با پیش‌فرض اپ انتظار UnchangedLowQuality: {:?}",
        run.outcome_of(&video)
    );
    assert_eq!(
        h.video_hash(&video),
        before,
        "با کیفیت پایین ویدیو نباید تغییر کند"
    );
    // sidecar استخراج‌شده برای بازبینی کاربر حفظ می‌شود
    let stem = video.file_stem().unwrap().to_string_lossy();
    assert!(
        video
            .parent()
            .unwrap()
            .join(format!("{stem}.fa.srt"))
            .exists(),
        "زیرنویس استخراج‌شده باید برای بازبینی حفظ شود"
    );
    // و هیچ mux باقی‌مانده‌ای نباید باشد
    assert_no_softsub_leftover(&video);
}

/// آزمون C — MKV با دو صدا، cover art، سه Track PGS انگلیسی و یک subrip فارسی:
/// همهٔ استریم‌های بی‌ربط باید بیت‌به‌بیت حفظ شوند و فقط Track فارسی با
/// نسخهٔ سینک‌شده جایگزین شود.
#[test]
#[ignore = "فقط اجرای دستی real-world — به SUBSYNC_REAL_* و ابزارهای واقعی نیاز دارد"]
fn real_c_multi_stream_preservation_with_stream_hashes() {
    let h = RealHarness::new("real-c", SubtitleOutputMode::SoftSub, false);
    let video = h.copy_sample("MULTI", "Show.S01E01.multi.mkv");
    let before = h.video_hash(&video);
    let before_probe = full_probe(&video);
    let n_before = before_probe.len();
    assert!(
        n_before >= 7,
        "نمونهٔ MULTI باید استریم‌های چندگانه داشته باشد"
    );
    assert_eq!(
        fa_tagged_count(&before_probe),
        1,
        "دقیقاً یک Track فارسی مبدأ"
    );

    let run = run_real_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert!(
        matches!(run.outcome_of(&video), ItemOutcome::SoftSubbed { .. }),
        "انتظار SoftSubbed: {:?}",
        run.outcome_of(&video)
    );
    assert_ne!(h.video_hash(&video), before);

    let after_probe = full_probe(&video);
    assert_eq!(after_probe.len(), n_before, "هیچ استریمی حذف یا اضافه نشود");

    // مقایسه بر اساس «ترتیب درون هر نوع» — دید واقعی پخش‌کننده. جایگزینی یک
    // Track، Track جدید را در انتهای mapping اضافه می‌کند و index سراسری
    // استریم‌های بعد از آن جابجا می‌شود (مثلاً cover art در انتها)؛ اما ترتیب
    // ordinal هر نوع و همهٔ محتواها دقیقاً حفظ می‌شود.
    for kind in ["video", "audio", "subtitle"] {
        let before_list = streams_of(&before_probe, kind);
        let after_list = streams_of(&after_probe, kind);
        assert_eq!(
            before_list.len(),
            after_list.len(),
            "تعداد استریم {kind} باید حفظ شود"
        );
        for (before_s, after_s) in before_list.iter().zip(after_list.iter()) {
            assert_eq!(
                before_s.codec_name,
                after_s.codec_name,
                "codec {kind} ordinal {} باید حفظ شود",
                stream_ordinal(&before_probe, kind, before_s.index)
            );
        }
    }

    // استریم‌های ویدیو/صدا/PGS بیت‌به‌بیت (نمونهٔ pristine مبدأ)
    let pristine = sample("MULTI");
    for s in before_probe.iter().filter(|s| s.codec_type != "subtitle") {
        let kind = match s.codec_type.as_str() {
            "video" => "v",
            "audio" => "a",
            _ => continue,
        };
        let ordinal = stream_ordinal(&before_probe, s.codec_type.as_str(), s.index);
        let spec = format!("0:{kind}:{ordinal}");
        let md5_b = stream_md5(&pristine, &spec).unwrap_or_else(|| panic!("hash مبدأ {spec}"));
        let md5_a = stream_md5(&video, &spec).unwrap_or_else(|| panic!("hash خروجی {spec}"));
        assert_eq!(md5_b, md5_a, "استریم {spec} باید stream-copy کامل باشد");
    }
    let pgs_before: Vec<&StreamInfo> = before_probe
        .iter()
        .filter(|s| s.codec_name == "hdmv_pgs_subtitle")
        .collect();
    for pb in &pgs_before {
        let ordinal = stream_ordinal(&before_probe, "subtitle", pb.index);
        let spec = format!("0:s:{ordinal}");
        let md5_b = stream_md5(&pristine, &spec);
        let md5_a = stream_md5(&video, &spec);
        match (md5_b, md5_a) {
            (Some(b), Some(a)) => assert_eq!(b, a, "محتوای PGS@{} باید حفظ شود", pb.index),
            (None, None) => {
                // ffmpeg برای این استریم hash نمی‌دهد — بررسی جایگزین: وجود Track
                continue;
            }
            (b, a) => panic!("hash PGS {spec} ناهماهنگ: {b:?} vs {a:?}"),
        }
    }

    // Track فارسی: همان ordinal زیرنویس قبلی، حالا subrip/fas
    let fa_before = before_probe
        .iter()
        .find(|s| s.codec_type == "subtitle" && s.language.as_deref() == Some("per"))
        .expect("Track فارسی مبدأ");
    let fa_after = after_probe
        .iter()
        .find(|s| s.codec_type == "subtitle" && s.language.as_deref() == Some("fas"))
        .expect("Track فارسی سینک‌شده");
    assert_eq!(
        stream_ordinal(&before_probe, "subtitle", fa_before.index),
        stream_ordinal(&after_probe, "subtitle", fa_after.index),
        "Track فارسی جایگزین همان ordinal زیرنویس می‌شود"
    );
    assert_eq!(fa_after.codec_name, "subrip");
    assert_eq!(fa_tagged_count(&after_probe), 1, "بدون Track فارسی تکراری");

    assert_no_softsub_leftover(&video);
    decode_ok(&video).expect("خروجی چند-استریمی باید decode سالم داشته باشد");
}

/// آزمون B1 — ویدیو بدون Track داخلی و بدون sidecar و بدون کلید API:
/// کلید گِیت نیست — دانلود ناشناس واقعی تلاش می‌شود (نیازمند شبکه)؛
/// نتیجه هر که باشد هرگز NeedsApiKey نیست.
#[test]
#[ignore = "فقط اجرای دستی real-world — به SUBSYNC_REAL_* و ابزارهای واقعی نیاز دارد"]
fn real_b1_no_key_downloads_anonymously() {
    let h = RealHarness::new("real-b1", SubtitleOutputMode::SoftSub, true);
    let video = h.copy_sample("NO_SUB", "Show.S01E02.mkv");
    let before = h.video_hash(&video);

    let run = run_real_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    let outcome = run.outcome_of(&video);
    assert!(
        !matches!(outcome, ItemOutcome::NeedsApiKey),
        "بدون کلید دیگر NeedsApiKey تولید نمی‌شود: {outcome:?}"
    );
    // اگر دانلود ناشناس خروجی واقعی داشت → SoftSubbed؛ وگرنه شکست/پرش صادقانه
    let has_output = run
        .summary
        .items
        .iter()
        .any(|item| item.video_path == video && item.artifacts.placed_path.is_some());
    if has_output {
        assert!(
            matches!(outcome, ItemOutcome::SoftSubbed { .. }),
            "با خروجی واقعی انتظار SoftSubbed: {outcome:?}"
        );
    } else {
        assert_eq!(
            h.video_hash(&video),
            before,
            "بدون خروجی واقعی ویدیو نباید دست بخورد"
        );
    }
    assert_no_softsub_leftover(&video);
}

/// آزمون H — ویدیوی واقعی با قرارداد نام هاردساب فارسی (مثل Film2Media):
/// دانلود انجام نمی‌شود و آیتم با Hardsubbed صادقانه بسته می‌شود.
/// ورودی: SUBSYNC_REAL_HARDSUB_VIDEO = مسیر یک ویدیوی واقعی هاردساب‌دار.
#[test]
#[ignore = "فقط اجرای دستی real-world — به SUBSYNC_REAL_HARDSUB_VIDEO و ابزارهای واقعی نیاز دارد"]
fn real_hardsub_named_video_skips_download_and_reports_hardsubbed() {
    let Ok(src) = std::env::var("SUBSYNC_REAL_HARDSUB_VIDEO") else {
        eprintln!("SKIP: SUBSYNC_REAL_HARDSUB_VIDEO تنظیم نشده — این تست اختیاری است");
        return;
    };
    let h = RealHarness::new("real-hardsub", SubtitleOutputMode::SoftSub, true);
    let src_path = std::path::PathBuf::from(&src);
    let name = src_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Overlord.IV.S04E13.1080p.WEB-DL.Farsi.Sub.Film2Media.mkv".into());
    let video = h.copy_external(&src_path, &name);
    let before = h.video_hash(&video);

    let run = run_real_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert_eq!(
        run.summary.counts.hardsubbed, 1,
        "ویدیوی هاردساب باید در شمارش خودش بیاید: {:?}",
        run.summary.counts
    );
    assert_eq!(
        run.summary.counts.downloaded, 0,
        "برای هاردساب نباید دانلود رخ دهد"
    );
    assert!(
        matches!(run.outcome_of(&video), ItemOutcome::Hardsubbed),
        "انتظار Hardsubbed: {:?}",
        run.outcome_of(&video)
    );
    assert_eq!(h.video_hash(&video), before, "ویدیو نباید دست بخورد");
    assert!(
        run.summary.staged_paths.is_empty(),
        "برای هاردساب نباید staging ساخته شود"
    );
    assert_no_softsub_leftover(&video);
}

/// آزمون H2 — جریان کامل «افزودن زیرنویس نرم» روی همان ویدیوی هاردساب:
/// place_sidecar_copy واقعی (هم‌نام ویدیو) → سینک واقعی ffsubsync → remux
/// SoftSub داخل ویدیو — دقیقاً همان مسیری که کاربر از UI طی می‌کند.
#[test]
#[ignore = "فقط اجرای دستی real-world — به SUBSYNC_REAL_HARDSUB_VIDEO و ابزارهای واقعی نیاز دارد"]
fn real_hardsub_with_placed_sidecar_syncs_and_softsubs() {
    let Ok(src) = std::env::var("SUBSYNC_REAL_HARDSUB_VIDEO") else {
        eprintln!("SKIP: SUBSYNC_REAL_HARDSUB_VIDEO تنظیم نشده — این تست اختیاری است");
        return;
    };
    // skip_low_quality=false — زیرنویس مصنوعی روی دیالوگ واقعی score منفی
    // می‌گیرد (یافتهٔ مستند آزمون B2)؛ هدف تست مسیر سینک/remux است نه گیت کیفیت
    let h = RealHarness::new("real-hardsub-softsub", SubtitleOutputMode::SoftSub, false);
    let src_path = std::path::PathBuf::from(&src);
    let name = src_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Overlord.IV.S04E13.1080p.WEB-DL.Farsi.Sub.Film2Media.mkv".into());
    let video = h.copy_external(&src_path, &name);
    let before = h.video_hash(&video);

    // زیرنویس نرم «به‌دست‌آمده» با خطای زمانی — اول در مسیر موقت، بعد از
    // مسیر تولید واقعی (place_sidecar_copy) هم‌نام کنار ویدیو جانمایی می‌شود
    let staging_src = video.parent().unwrap().join("candidate.fa.srt");
    fs::write(
        &staging_src,
        synthetic_fa_srt(ffprobe_duration(&video), 3.2),
    )
    .unwrap();
    let placed = crate::commands::subsource::place_sidecar_copy(&staging_src, &video)
        .expect("جانمایی هم‌نام باید موفق شود");
    let _ = fs::remove_file(&staging_src);
    let sidecar = video.parent().unwrap().join(
        video
            .file_stem()
            .map(|s| format!("{}.fa.srt", s.to_string_lossy()))
            .unwrap(),
    );
    assert_eq!(
        std::path::Path::new(&placed),
        sidecar,
        "جانمایی باید دقیقاً هم‌نام ویدیو باشد"
    );

    // اجرای مجدد pipeline — همان جریان کاربر پس از افزودن زیرنویس نرم
    let run = run_real_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert!(
        matches!(run.outcome_of(&video), ItemOutcome::SoftSubbed { .. }),
        "با sidecar هم‌نام، انتظار سینک واقعی و SoftSub: {:?}",
        run.outcome_of(&video)
    );
    assert_ne!(h.video_hash(&video), before, "SoftSub باید داخل ویدیو برود");
    assert_no_softsub_leftover(&video);
    decode_ok(&video).expect("خروجی SoftSub باید decode سالم داشته باشد");
}

/// آزمون B2 — ویدیو بدون Track داخلی؛ زیرنویس فارسی «به‌دست‌آمده» (مثل خروجی
/// فاز دانلود/آرشیو) کنار ویدیو گذاشته می‌شود → سینک واقعی ffsubsync →
/// remux واقعی داخل ویدیو؛ بدون ساختن sidecar سینک‌شدهٔ جدید.
#[test]
#[ignore = "فقط اجرای دستی real-world — به SUBSYNC_REAL_* و ابزارهای واقعی نیاز دارد"]
fn real_b2_external_sidecar_synced_and_softsubbed() {
    let h = RealHarness::new("real-b2", SubtitleOutputMode::SoftSub, false);
    let video = h.copy_sample("NO_SUB", "Show.S01E03.mkv");
    let before = h.video_hash(&video);

    // زیرنویس «به‌دست‌آمده» با ۳.۲ ثانیه خطای زمانی — سینک واقعی باید اصلاحش کند
    let sidecar = video.parent().unwrap().join("Show.S01E03.fa.srt");
    fs::write(&sidecar, synthetic_fa_srt(ffprobe_duration(&video), 3.2)).unwrap();

    let run = run_real_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert!(
        matches!(run.outcome_of(&video), ItemOutcome::SoftSubbed { .. }),
        "انتظار SoftSubbed: {:?}",
        run.outcome_of(&video)
    );
    assert_ne!(h.video_hash(&video), before);

    let after_probe = full_probe(&video);
    assert_eq!(after_probe.len(), 3);
    assert_eq!(streams_of(&after_probe, "video")[0].codec_name, "h264");
    assert_eq!(streams_of(&after_probe, "audio")[0].codec_name, "aac");
    let subs = streams_of(&after_probe, "subtitle");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].codec_name, "subrip");
    assert_eq!(subs[0].language.as_deref(), Some("fas"));

    // هیچ sidecar سینک‌شدهٔ جدیدی ساخته نشده؛ فایل مبدأ کاربر حفظ شده
    assert_no_softsub_leftover(&video);
    assert!(sidecar.exists(), "فایل مبدأ کاربر باید حفظ شود");
    decode_ok(&video).expect("decode سالم");
}

/// آزمون B3 — همان سناریو روی MP4: کدک زیرنویس باید mov_text شود و
/// ویدیو/صدای MP4 حفظ شود.
#[test]
#[ignore = "فقط اجرای دستی real-world — به SUBSYNC_REAL_* و ابزارهای واقعی نیاز دارد"]
fn real_b3_mp4_sidecar_softsubbed_as_mov_text() {
    let h = RealHarness::new("real-b3", SubtitleOutputMode::SoftSub, false);
    let video = h.copy_sample("NO_SUB_MP4", "Show.S01E04.mp4");
    let before = h.video_hash(&video);
    let sidecar = video.parent().unwrap().join("Show.S01E04.fa.srt");
    fs::write(&sidecar, synthetic_fa_srt(ffprobe_duration(&video), -2.5)).unwrap();

    let run = run_real_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert!(
        matches!(run.outcome_of(&video), ItemOutcome::SoftSubbed { .. }),
        "انتظار SoftSubbed روی MP4: {:?}",
        run.outcome_of(&video)
    );
    assert_ne!(h.video_hash(&video), before);

    let after_probe = full_probe(&video);
    assert_eq!(streams_of(&after_probe, "video")[0].codec_name, "h264");
    assert_eq!(streams_of(&after_probe, "audio")[0].codec_name, "aac");
    let subs = streams_of(&after_probe, "subtitle");
    assert_eq!(subs.len(), 1);
    assert_eq!(
        subs[0].codec_name, "mov_text",
        "زیرنویس داخل MP4 باید mov_text شود"
    );
    assert_eq!(subs[0].language.as_deref(), Some("fas"));
    assert_no_softsub_leftover(&video);
    assert!(sidecar.exists(), "فایل مبدأ کاربر باید حفظ شود");
    decode_ok(&video).expect("decode سالم MP4");
}

/// آزمون D — Track متنی فارسی بدون تگ زبان (review محتوا): تصمیم صریح
/// کاربر شبیه‌سازی می‌شود؛ Track باید استخراج، تأیید، سینک و جایگزین شود.
#[test]
#[ignore = "فقط اجرای دستی real-world — به SUBSYNC_REAL_* و ابزارهای واقعی نیاز دارد"]
fn real_d_untagged_persian_track_review_confirmed_and_replaced() {
    let h = RealHarness::new("real-d", SubtitleOutputMode::SoftSub, false);
    let video = h.copy_sample("UNTAGGED", "Show.S01E05.mkv");
    let before = h.video_hash(&video);
    let before_probe = full_probe(&video);
    let untagged = before_probe
        .iter()
        .find(|s| s.codec_type == "subtitle" && s.language.is_none())
        .expect("نمونهٔ UNTAGGED باید Track بدون تگ داشته باشد")
        .index;

    let run = run_real_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        Some(first_option_resolver(&h.state)),
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    assert_eq!(
        run.decisions.len(),
        1,
        "دقیقاً یک تصمیم review برای Track بدون تگ"
    );
    assert!(
        matches!(run.outcome_of(&video), ItemOutcome::SoftSubbed { .. }),
        "انتظار SoftSubbed پس از review: {:?}",
        run.outcome_of(&video)
    );
    assert_ne!(h.video_hash(&video), before);

    let after_probe = full_probe(&video);
    assert_eq!(
        after_probe.len(),
        before_probe.len(),
        "تعداد استریم حفظ شود"
    );
    let subs = streams_of(&after_probe, "subtitle");
    assert_eq!(subs.len(), 1, "دقیقاً یک Track زیرنویس (جایگزین‌شده)");
    assert_eq!(
        subs[0].codec_name, "subrip",
        "Track فارسی بدون تگ باید به subrip تبدیل و جایگزین شود"
    );
    assert_eq!(subs[0].language.as_deref(), Some("fas"));
    assert_eq!(
        stream_ordinal(&after_probe, "subtitle", subs[0].index),
        stream_ordinal(&before_probe, "subtitle", untagged),
        "جایگزینی در همان ordinal زیرنویس"
    );
    assert_eq!(streams_of(&after_probe, "video")[0].codec_name, "h264");
    assert_no_softsub_leftover(&video);
    decode_ok(&video).expect("decode سالم");
}

/// آزمون E — حالت قدیمی Sidecar: خروجی `.fa.synced.srt` کنار ویدیو ساخته
/// می‌شود و خود ویدیو بیت‌به‌بیت دست‌نخورده می‌ماند.
#[test]
#[ignore = "فقط اجرای دستی real-world — به SUBSYNC_REAL_* و ابزارهای واقعی نیاز دارد"]
fn real_e_sidecar_mode_places_synced_sidecar_and_leaves_video_untouched() {
    let h = RealHarness::new("real-e", SubtitleOutputMode::Sidecar, false);
    let video = h.copy_sample("SIDECAR", "Show.S01E06.mkv");
    let before = h.video_hash(&video);
    let sidecar = video.parent().unwrap().join("Show.S01E06.fa.srt");
    fs::write(&sidecar, synthetic_fa_srt(ffprobe_duration(&video), 1.7)).unwrap();

    let run = run_real_pipeline(
        Arc::clone(&h.state),
        vec![video.to_string_lossy().into_owned()],
        None,
    );

    assert_eq!(run.summary.status, PipelineStatus::Completed);
    match run.outcome_of(&video) {
        ItemOutcome::Synced { output_path } => {
            assert_eq!(
                Path::new(output_path),
                video.parent().unwrap().join("Show.S01E06.fa.synced.srt")
            );
        }
        other => panic!("انتظار Synced در حالت sidecar، دریافت: {other:?}"),
    }
    let synced = video.parent().unwrap().join("Show.S01E06.fa.synced.srt");
    assert!(synced.exists(), "sidecar سینک‌شده باید ساخته شده باشد");
    assert_eq!(
        h.video_hash(&video),
        before,
        "در حالت sidecar ویدیو نباید تغییر کند"
    );
    let text = fs::read_to_string(&synced).unwrap();
    assert!(
        text.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c)),
        "خروجی سینک‌شده باید فارسی باشد"
    );
}

/// شکست ۱ — زیرنویس خراب (فارسیِ بدون timestamp): FFmpeg واقعی شکست می‌خورد؛
/// ویدیوی مبدأ باید بیت‌به‌بیت دست‌نخورده بماند و workspace پاک شود.
#[test]
#[ignore = "فقط اجرای دستی real-world — به SUBSYNC_REAL_* و ابزارهای واقعی نیاز دارد"]
fn real_fail_corrupt_subtitle_fails_mux_and_preserves_source() {
    let h = RealHarness::new("real-fail-mux", SubtitleOutputMode::SoftSub, true);
    let video = h.copy_sample("A", "Show.S01E07.mkv");
    let before = h.video_hash(&video);

    // زیرنویس فارسیِ خراب: validate می‌گذرد (UTF-8 + فارسی) اما FFmpeg رد می‌کند
    let broken = h.run_root.join("broken-fa.srt");
    fs::write(
        &broken,
        "سلام دنیا\nخط دوم فارسی بدون هیچ تایم‌استمپی\n\nمتن بعدی\n",
    )
    .unwrap();

    let result = crate::subtitle::mux::softsub_in_place(
        &real_ffmpeg(),
        &real_ffprobe(),
        &video,
        &broken,
        Some(2),
        &AtomicBool::new(false),
    );
    assert!(result.is_err(), "mux با زیرنویس خراب باید شکست بخورد");
    assert_eq!(
        h.video_hash(&video),
        before,
        "شکست mux نباید ویدیو را تغییر دهد"
    );
    assert_no_softsub_leftover(&video);
}

/// شکست ۲ — لغو واقعی در میانهٔ remux (ffmpeg واقعی روی نمونهٔ چندصد مگابایتی):
/// ویدیوی مبدأ بیت‌به‌بیت دست‌نخورده می‌ماند و workspace موقت پاک می‌شود.
#[test]
#[ignore = "فقط اجرای دستی real-world — به SUBSYNC_REAL_* و ابزارهای واقعی نیاز دارد"]
fn real_fail_cancel_during_mux_preserves_source() {
    let h = RealHarness::new("real-fail-cancel", SubtitleOutputMode::SoftSub, true);
    let video = h.copy_sample("MULTI", "Show.S01E08.mkv");
    let before = h.video_hash(&video);

    let cancel = Arc::new(AtomicBool::new(false));
    let srt = h.run_root.join("synced-fa.srt");
    fs::write(&srt, synthetic_fa_srt(ffprobe_duration(&video), 0.0)).unwrap();

    let cancel_flag = Arc::clone(&cancel);
    let video_for_watcher = video.clone();
    let watcher = std::thread::spawn(move || {
        // صبر تا شروع mux واقعی: ظهور workspace موقت کنار ویدیو، سپس لغو فوری
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            assert!(Instant::now() < deadline, "mux باید شروع می‌شد");
            let started = fs::read_dir(video_for_watcher.parent().unwrap())
                .unwrap()
                .filter_map(Result::ok)
                .any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with(".subsync-softsub-")
                });
            if started {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        cancel_flag.store(true, Ordering::SeqCst);
    });

    let result = crate::subtitle::mux::softsub_in_place(
        &real_ffmpeg(),
        &real_ffprobe(),
        &video,
        &srt,
        None, // بدون جایگزینی — افزودن Track؛ remux کامل تا لغو اجرا می‌شود
        &cancel,
    );
    watcher.join().expect("watcher");

    match &result {
        Err(crate::subtitle::mux::MuxError::Cancelled) => {}
        other => panic!("انتظار Cancelled در میانهٔ remux واقعی، دریافت: {other:?}"),
    }
    assert_eq!(
        h.video_hash(&video),
        before,
        "لغو حین remux نباید ویدیو را تغییر دهد"
    );
    assert_no_softsub_leftover(&video);
}
