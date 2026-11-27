//! اجرای امن ffs.exe: ساخت آرگومان‌ها، اجرای بدون shell، ثبت خروجی با سقف حافظه،
//! طبقه‌بندی fail-closed و خاتمهٔ درخت فرایند.

use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::types::{
    SyncConfig, SyncRequest, CAPTURE_CAP_BYTES, LOG_TAIL_LIMIT_BYTES, LOW_QUALITY_MARKER,
    OFFSET_MARKER, QUALITY_MAX_OFFSET_SECONDS, SEARCH_MAX_OFFSET_SECONDS,
};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// فاصلهٔ polling برای `try_wait` — هم‌راستا با سایر آداپتورها.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

// ---------------------------------------------------------------------
// ساخت آرگومان‌ها
// ---------------------------------------------------------------------

/// استراتژی اجرای ffsubsync — در یک run چند استراتژی پشت‌سرهم امتحان می‌شوند
/// (ترتیب در `sync_strategies` بر اساس تلاشِ تنظیم دوباره).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SyncStrategy {
    /// حالت کامل — ffsubsync خودش بهترین مرجع را برمی‌دارد: اگر ویدیو Track
    /// زیرنویس متنی داخلی داشته باشد، همان مرجع می‌شود (هم‌ترازیِ
    /// زیرنویس‌به‌زیرنویس — پایدار، دقیق و بدون نویز صوت) وگرنه VAD کامل صوت.
    /// شواهد واقعی: روی فایل 6CH پر موسیقی، فقط همین مسیر آفست 0.000 با اسکور
    /// ۵۶۵k داد؛ مسیر نمونه‌برداری برای همان فایل چهار آفست متفاوت و ناپایدار
    /// (+36.11، +32.6، ‎-8.64، ‎-297.39) پیدا کرده بود.
    ///
    /// نکتهٔ مهم: وقتی sidecar همان Track داخلی است، این مسیر به‌ناچار آفست 0
    /// می‌دهد — اگر Track داخلیِ خودش با صوت هم‌تراز نباشد، این استراتژی
    /// نمی‌تواند آن را ببیند؛ همان حالتی که «تنظیم دوباره» با AudioOnly جبران
    /// می‌کند (قضاوت نهایی با پخشِ ویدیو توسط کاربر است).
    FullReference,
    /// مرجع صوتیِ کامل — `--vad webrtc` مسیر زیرنویسِ داخلی را دور می‌زند و
    /// مستقیماً VAD کامل صوت را مرجع می‌گیرد: تنها شاهدِ مستقل از خودِ
    /// زیرنویس. اولین تلاشِ «تنظیم دوباره» همین است.
    AudioOnly,
    /// نمونه‌برداری چندsegmentای صوت (رفتار قبلی) — سریع‌تر اما پرنویزتر.
    MultiSegmentAudio,
}

impl SyncStrategy {
    /// برچسب فارسی برای پیام پیشرفت و خلاصهٔ تلاش‌ها.
    pub(crate) fn label(self) -> &'static str {
        match self {
            SyncStrategy::FullReference => "مرجع کامل (Track داخلی/VAD کامل صوت)",
            SyncStrategy::AudioOnly => "مرجع صوتی کامل",
            SyncStrategy::MultiSegmentAudio => "نمونه‌برداری صوت",
        }
    }
}

/// ترتیب استراتژی‌های یک run بر اساس شمارهٔ تلاش.
///
/// - تلاش ۰ (اجرای اولیه): مرجع کامل، بعد نمونه‌برداری صوت.
/// - تلاش‌های «تنظیم دوباره» (≥۱): هر تلاش با استراتژیِ متفاوتی شروع می‌شود تا
///   نتیجه با تلاش قبل واقعاً فرق کند — کاربر با «پخش ویدیو» قضاوت می‌کند و
///   اگر راضی نبود، تنظیم دوبارهٔ بعدی استراتژی بعدی را می‌آموزد:
///   AudioOnly → FullReference → MultiSegmentAudio → تکرار (با زیرنویس تازه).
pub(crate) fn sync_strategies(config: &SyncConfig, attempt: u32) -> Vec<SyncStrategy> {
    let ladder = [
        SyncStrategy::AudioOnly,
        SyncStrategy::FullReference,
        SyncStrategy::MultiSegmentAudio,
    ];
    if attempt == 0 {
        let mut strategies = vec![SyncStrategy::FullReference];
        if config.multi_segment {
            strategies.push(SyncStrategy::MultiSegmentAudio);
        }
        return strategies;
    }
    let primary = ladder[(attempt as usize - 1) % ladder.len()];
    let mut strategies = vec![primary];
    strategies.extend(ladder.iter().copied().filter(|s| *s != primary));
    strategies
}

/// آرگومان‌های دقیق ffs — هر آرگومان جدا، بدون هیچ shell.
///
/// قرارداد تأییدشدهٔ ffsubsync 0.5.1:
/// - FullReference:
///   `ffs <video> -i <sub> -o <out> [--skip-sync-on-low-quality
///   --quality-max-offset-seconds 30] --max-offset-seconds 60
///   --output-encoding utf-8 [--ffmpeg-path <ffmpeg>] --log-dir-path <dir>`
/// - MultiSegmentAudio (همان موارد بالا +):
///   `--multi-segment-sync --segment-count N [--skip-intro-outro]
///   --parallel-workers 1`
///
/// `--max-offset-seconds` و `--quality-max-offset-seconds` هر دو صریح پین
/// می‌شوند (پیش‌فرض‌های 0.5.1). آستانهٔ کیفیت عمداً از دامنهٔ جستجو کوچک‌تر
/// می‌ماند: اسکور خام FFT نرمال‌شده نیست و روی صدای پرنویز آفست‌های کاذبِ
/// ۳۰–۶۰ ثانیه‌ای را برنده می‌کند (رسانهٔ واقعی: آفست «36.1s» که در واقع
/// زیرنویس از قبل سینک بود). `--parallel-workers` فقط برای نمونه‌برداری صوت
/// معنا دارد و همان‌جا پین می‌شود.
pub fn build_cli_args(
    request: &SyncRequest,
    strategy: SyncStrategy,
    output: &Path,
    log_dir: &Path,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        request.video_path.as_os_str().to_os_string(),
        "-i".into(),
        request.subtitle_path.as_os_str().to_os_string(),
        "-o".into(),
        output.as_os_str().to_os_string(),
    ];

    if request.config.skip_sync_on_low_quality {
        args.push("--skip-sync-on-low-quality".into());
        // آستانهٔ گیت کیفیت پین‌شده — هیچ‌وقت به 60 بالا برده نمی‌شود؛
        // آفست‌های بزرگِ صوتی معمولاً پیک کاذب‌اند (توضیح const در types.rs).
        args.push("--quality-max-offset-seconds".into());
        args.push(QUALITY_MAX_OFFSET_SECONDS.to_string().into());
    }

    // دامنهٔ جستجوی آفست — صریح و همیشه، تا از پیش‌فرض‌های آیندهٔ ffsubsync
    // جدا بمانیم.
    args.push("--max-offset-seconds".into());
    args.push(SEARCH_MAX_OFFSET_SECONDS.to_string().into());

    if strategy == SyncStrategy::MultiSegmentAudio {
        args.push("--multi-segment-sync".into());
        args.push("--segment-count".into());
        args.push(request.config.effective_segment_count().to_string().into());
        if request.config.skip_intro_outro {
            args.push("--skip-intro-outro".into());
        }
        args.push("--parallel-workers".into());
        args.push("1".into());
    }

    if strategy == SyncStrategy::AudioOnly {
        // دور زدن مرجعِ زیرنویس داخلی: "subs_then_" حذف می‌شود و VAD صوتِ
        // کامل تنها مرجع می‌شود — تنها شاهدِ مستقل از خودِ زیرنویس
        args.push("--vad".into());
        args.push("webrtc".into());
    }

    args.push("--output-encoding".into());
    args.push("utf-8".into());

    if let Some(ffmpeg) = &request.ffmpeg_path {
        args.push("--ffmpeg-path".into());
        args.push(ffmpeg.as_os_str().to_os_string());
    }

    args.push("--log-dir-path".into());
    args.push(log_dir.as_os_str().to_os_string());
    args
}

// ---------------------------------------------------------------------
// طبقه‌بندی fail-closed
// ---------------------------------------------------------------------

/// طبقه‌بندی خام یک run بر اساس کد خروج + متن لاگ‌ها — پیش از اعتبارسنجی فایل خروجی.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RunClassification {
    /// marker کیفیت پایین — بر کد خروج و marker آفست اولویت دارد؛ خروجی دور ریخته می‌شود.
    LowQuality,
    /// کد خروج صفر + marker آفست با مقدار عددی متناهی — آفستِ validated حمل می‌شود
    /// تا «سینک‌شده با آفست گمشده» هرگز ممکن نشود. هنوز فایل خروجی باید اعتبارسنجی شود.
    Synchronized { offset_seconds: f64 },
    /// کد خروج غیرصفر (یا خاتمهٔ غیرعادی بدون کد).
    FailedExit(Option<i32>),
    /// کد خروج صفر بدون marker آفستِ معتبر — هیچ ادعایی دربارهٔ سینک ممکن نیست.
    Unknown,
}

/// طبقه‌بندی صارم: موفقیت = کد خروج صفر **و** آفست عددی متناهیِ parse‌شده.
/// مقدارهای NaN/±inf/گم‌شده/ناقص و حتی marker خرابِ اولین، fail-closed به Unknown
/// می‌روند و marker معتبرِ بعدی‌شان جایگزین نمی‌شود (فرمت واقعی ffsubsync برای
/// یک ورودی فقط یک `offset seconds:` می‌دهد؛ پس هیچ دلیل موجهی برای override نیست).
pub(crate) fn classify_run(exit_code: Option<i32>, logs: &str) -> RunClassification {
    if logs.contains(LOW_QUALITY_MARKER) {
        return RunClassification::LowQuality;
    }
    match exit_code {
        Some(0) => match extract_offset_seconds(logs) {
            Some(offset_seconds) => RunClassification::Synchronized { offset_seconds },
            None => RunClassification::Unknown,
        },
        Some(code) => RunClassification::FailedExit(Some(code)),
        None => RunClassification::FailedExit(None),
    }
}

/// استخراج آفست از marker `offset seconds: %.3f` — فقط اولین رخداد، فقط عدد متناهی.
/// NaN/±inf/مقدار گم‌شده یا ناقص → None (fail-closed؛ بدون هیچ leniency مثل trim کاما).
pub(crate) fn extract_offset_seconds(logs: &str) -> Option<f64> {
    let idx = logs.find(OFFSET_MARKER)?;
    let rest = &logs[idx + OFFSET_MARKER.len()..];
    let token = rest.split_whitespace().next()?;
    token
        .parse::<f64>()
        .ok()
        .filter(|offset| offset.is_finite())
}

/// خواندن tail محدودشدهٔ ffsubsync.log — حداکثر ۱۲۸KiB از انتهای فایل، با seek؛
/// فایل کامل هرگز به حافظه کشیده نمی‌شود. فایل ناموجود/ناخوانا/خالی → None.
pub(crate) fn read_log_tail(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len == 0 {
        return None;
    }
    let start = len.saturating_sub(LOG_TAIL_LIMIT_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bounded = file.take(LOG_TAIL_LIMIT_BYTES);
    let mut buffer = Vec::with_capacity(LOG_TAIL_LIMIT_BYTES as usize);
    bounded.read_to_end(&mut buffer).ok()?;
    let text = String::from_utf8_lossy(&buffer).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// tail محدودشدهٔ متن برای diagnostic — متن خام کامل هرگز به بیرون نمی‌رود.
pub(crate) fn bounded_tail(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    text.chars().skip(count - max_chars).collect()
}

// ---------------------------------------------------------------------
// ثبت خروجی با سقف حافظه
// ---------------------------------------------------------------------

/// بافر چرخشی با سقف بایت — خواندن pipe هرگز حافظه را بی‌نهایت پر نمی‌کند.
pub(crate) struct RollingCapture {
    buffer: Mutex<Vec<u8>>,
}

impl RollingCapture {
    pub(crate) fn new_shared() -> Arc<Self> {
        Arc::new(RollingCapture {
            buffer: Mutex::new(Vec::with_capacity(4096)),
        })
    }

    pub(crate) fn push(&self, data: &[u8]) {
        let mut buffer = self.buffer.lock().unwrap();
        buffer.extend_from_slice(data);
        let overflow = buffer.len().saturating_sub(CAPTURE_CAP_BYTES);
        if overflow > 0 {
            buffer.drain(..overflow);
        }
    }

    pub(crate) fn text(&self) -> String {
        let buffer = self.buffer.lock().unwrap();
        String::from_utf8_lossy(&buffer).into_owned()
    }
}

fn drain_pipe(pipe: impl Read, capture: Arc<RollingCapture>) {
    let mut pipe = pipe;
    let mut chunk = [0u8; 8192];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => break, // EOF
            Ok(n) => capture.push(&chunk[..n]),
            Err(_) => break,
        }
    }
}

// ---------------------------------------------------------------------
// خاتمهٔ فرایند
// ---------------------------------------------------------------------

/// خاتمهٔ درخت فرایند: ویندوز → `taskkill /PID <pid> /T /F` (بدون shell، آرگومان جدا)؛
/// شکست taskkill → بازگشت به kill مستقیم با هشدار صریح. سایر پلتفرم‌ها → فقط فرزند مستقیم.
/// یادداشت‌های برگشتی هرگز ادعای قطعیِ خاتمهٔ فرزندان نمی‌کنند مگر taskkill خودش موفق شده باشد.
pub(crate) fn terminate_process_tree(child: &mut Child) -> Vec<String> {
    let mut notes: Vec<String> = Vec::new();

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let pid = child.id();
        let status = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status();
        match status {
            Ok(s) if s.success() => {
                notes.push("درخت فرایند با taskkill /T /F خاتمه داده شد.".into());
                let _ = child.wait();
                return notes;
            }
            Ok(s) => notes.push(format!(
                "taskkill با کد خروج {} ناموفق بود؛ به خاتمهٔ مستقیم بازگشت.",
                s.code().unwrap_or(-1)
            )),
            Err(e) => notes.push(format!(
                "اجرای taskkill ناموفق بود: {e}؛ به خاتمهٔ مستقیم بازگشت."
            )),
        }
    }

    // fallback (ویندوز بعد از شکست taskkill) یا رفتار سایر پلتفرم‌ها
    match child.kill() {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {}
        Err(e) => notes.push(format!("kill مستقیم ناموفق بود: {e}")),
    }
    let _ = child.wait();

    #[cfg(windows)]
    notes.push("تأیید خاتمهٔ کامل درخت فرایند ممکن نشد — فرزندان ممکن است باقی مانده باشند.".into());
    #[cfg(not(windows))]
    notes.push(
        "فقط فرزند مستقیم خاتمه داده شد؛ نوه‌ها (مثل ffmpeg داخلی ffsubsync) ممکن است باقی بمانند."
            .into(),
    );

    notes
}

// ---------------------------------------------------------------------
// اجرای فرایند
// ---------------------------------------------------------------------

/// نتیجهٔ خام اجرای ffs — پیش از طبقه‌بندی.
pub(crate) struct ProcessRun {
    pub exit_code: Option<i32>,
    pub cancelled: bool,
    /// stdout + stderr (باسقف) + محتوای فایل لاگ ffsubsync.log (در صورت وجود).
    pub logs: String,
    pub termination_notes: Vec<String>,
}

/// اجرای ffs بدون shell با پشتیبانی لغو. خطای برگشتی = spawn/pipe؛ لغو در خود نتیجه است.
pub(crate) fn run_process(
    exe: &Path,
    args: &[OsString],
    log_dir: &Path,
    cancel: &AtomicBool,
) -> Result<ProcessRun, String> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(ProcessRun {
            exit_code: None,
            cancelled: true,
            logs: String::new(),
            termination_notes: vec!["عملیات پیش از شروع فرایند لغو شد.".into()],
        });
    }

    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("اجرای FFsubsync ناموفق بود: {e}"))?;

    // لغو در فاصلهٔ بین spawn و شروع wait — با خاتمهٔ درخت فرایند
    if cancel.load(Ordering::Relaxed) {
        let termination_notes = terminate_process_tree(&mut child);
        return Ok(ProcessRun {
            exit_code: None,
            cancelled: true,
            logs: String::new(),
            termination_notes,
        });
    }

    let stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| "stdout pipe در دسترس نیست".to_string())?;
    let stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| "stderr pipe در دسترس نیست".to_string())?;

    let stdout_capture = RollingCapture::new_shared();
    let stderr_capture = RollingCapture::new_shared();
    let stdout_thread = std::thread::spawn({
        let capture = Arc::clone(&stdout_capture);
        move || drain_pipe(stdout_pipe, capture)
    });
    let stderr_thread = std::thread::spawn({
        let capture = Arc::clone(&stderr_capture);
        move || drain_pipe(stderr_pipe, capture)
    });

    // انتظار با polling — لغو هر لحظه ممکن است و هرگز منتظر درصد نیستیم
    let mut cancelled = false;
    let mut termination_notes: Vec<String> = Vec::new();
    let exit_code;
    loop {
        if cancel.load(Ordering::Relaxed) {
            termination_notes = terminate_process_tree(&mut child);
            cancelled = true;
            exit_code = None;
            break;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                exit_code = status.code();
                break;
            }
            Ok(None) => std::thread::sleep(WAIT_POLL_INTERVAL),
            Err(e) => {
                termination_notes.push(format!("خطا در try_wait: {e}"));
                termination_notes.extend(terminate_process_tree(&mut child));
                return Err(termination_notes.join(" "));
            }
        }
    }

    // پس از خروج طبیعی، pipeها بسته شده‌اند و readerها سریع به EOF می‌رسند؛
    // در مسیر لغو readerها detach می‌شوند تا لغو هرگز روی join بلاک نشود
    // (بافرها Arc هستند و پس از EOF خودشان آزاد می‌شوند).
    if !cancelled {
        let _ = stdout_thread.join();
        let _ = stderr_thread.join();
    }

    let mut logs = String::new();
    if !cancelled {
        let stdout_text = stdout_capture.text();
        if !stdout_text.trim().is_empty() {
            logs.push_str("[stdout]\n");
            logs.push_str(&stdout_text);
            logs.push('\n');
        }
        let stderr_text = stderr_capture.text();
        if !stderr_text.trim().is_empty() {
            logs.push_str("[stderr]\n");
            logs.push_str(&stderr_text);
            logs.push('\n');
        }
        // لاگ فایل ffsubsync هم بررسی می‌شود (با --log-dir-path نگه داشته می‌شود)
        // — فقط tail محدودشده؛ فایل کامل هرگز به حافظه کشیده نمی‌شود
        let log_path = log_dir.join("ffsubsync.log");
        if let Some(text) = read_log_tail(&log_path) {
            logs.push_str("[ffsubsync.log]\n");
            logs.push_str(&text);
            logs.push('\n');
        }
    }

    Ok(ProcessRun {
        exit_code,
        cancelled,
        logs,
        termination_notes,
    })
}

// ---------------------------------------------------------------------
// token یکتا برای workspace
// ---------------------------------------------------------------------

pub(crate) fn unique_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{:x}-{}", nanos, counter, std::process::id())
}

// ---------------------------------------------------------------------
// تست‌های pure
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::SyncConfig;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "subsync-sync-clitest-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn sync_config() -> SyncConfig {
        SyncConfig {
            multi_segment: true,
            segment_count: 6,
            skip_intro_outro: true,
            skip_sync_on_low_quality: true,
        }
    }

    fn request(config: SyncConfig, ffmpeg: Option<PathBuf>) -> SyncRequest {
        SyncRequest {
            video_path: PathBuf::from("C:\\media\\Show.S01E01.mkv"),
            subtitle_path: PathBuf::from("C:\\media\\Show.S01E01.fa.srt"),
            ffsubsync_path: PathBuf::from("C:\\tools\\ffs.exe"),
            ffmpeg_path: ffmpeg,
            config,
            workspace_root: None,
            attempt: None,
        }
    }

    #[test]
    fn full_reference_args_omit_multi_segment_flags() {
        let output = PathBuf::from("C:\\tmp\\out\\Show.S01E01.fa.synced.srt");
        let log_dir = PathBuf::from("C:\\tmp\\ws\\logs");

        // همهٔ پرچم‌ها خاموش — هیچ پرچم اختیاریِ مرتبط با گیت/segment نباید بیاید
        let off = SyncConfig {
            multi_segment: true,
            segment_count: 6,
            skip_intro_outro: true,
            skip_sync_on_low_quality: false,
        };
        let args = build_cli_args(
            &request(off, None),
            SyncStrategy::FullReference,
            &output,
            &log_dir,
        );
        let texts: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(texts[0], "C:\\media\\Show.S01E01.mkv");
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "-i" && w[1].contains("Show.S01E01.fa.srt")));
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "-o" && w[1].contains("fa.synced.srt")));
        // حالت مرجع کامل هیچ پرچم نمونه‌برداری‌ای نمی‌گیرد — ffsubsync خودش
        // Track زیرنویس داخلی را (در صورت وجود) مرجع برمی‌دارد
        assert!(!texts.iter().any(|t| t.contains("multi-segment")));
        assert!(!texts.iter().any(|t| t.contains("segment-count")));
        assert!(!texts.iter().any(|t| t.contains("skip-intro-outro")));
        assert!(!texts.iter().any(|t| t.contains("parallel-workers")));
        assert!(!texts
            .iter()
            .any(|t| t.contains("quality-max-offset-seconds")));
        assert!(!texts.iter().any(|t| t.contains("ffmpeg-path")));
        // دامنهٔ جستجو همیشه صریح پین می‌شود — مستقل از تنظیم کیفیت
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--max-offset-seconds" && w[1] == "60"));
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--output-encoding" && w[1] == "utf-8"));
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--log-dir-path" && w[1].contains("logs")));

        // گیت کیفیت روشن + ffmpeg — آستانهٔ پین‌شدهٔ 30، بدون پرچم‌های segment
        let on = SyncConfig {
            multi_segment: true,
            segment_count: 6,
            skip_intro_outro: true,
            skip_sync_on_low_quality: true,
        };
        let args = build_cli_args(
            &request(on, Some(PathBuf::from("C:\\ffmpeg\\ffmpeg.exe"))),
            SyncStrategy::FullReference,
            &output,
            &log_dir,
        );
        let texts: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(!texts.iter().any(|t| t.contains("multi-segment")));
        assert!(!texts.iter().any(|t| t.contains("segment-count")));
        assert!(!texts.iter().any(|t| t.contains("skip-intro-outro")));
        assert!(!texts.iter().any(|t| t.contains("parallel-workers")));
        assert!(texts.iter().any(|t| t == "--skip-sync-on-low-quality"));
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--quality-max-offset-seconds" && w[1] == "30"));
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--max-offset-seconds" && w[1] == "60"));
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--ffmpeg-path" && w[1] == "C:\\ffmpeg\\ffmpeg.exe"));
    }

    #[test]
    fn multi_segment_args_map_settings_and_enforce_single_worker() {
        let output = PathBuf::from("C:\\tmp\\out\\Show.S01E01.fa.synced.srt");
        let log_dir = PathBuf::from("C:\\tmp\\ws\\logs");

        let on = SyncConfig {
            multi_segment: true,
            segment_count: 6,
            skip_intro_outro: true,
            skip_sync_on_low_quality: true,
        };
        let args = build_cli_args(
            &request(on, Some(PathBuf::from("C:\\ffmpeg\\ffmpeg.exe"))),
            SyncStrategy::MultiSegmentAudio,
            &output,
            &log_dir,
        );
        let texts: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(texts.iter().any(|t| t == "--multi-segment-sync"));
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--segment-count" && w[1] == "6"));
        assert!(texts.iter().any(|t| t == "--skip-intro-outro"));
        assert!(texts.iter().any(|t| t == "--skip-sync-on-low-quality"));
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--quality-max-offset-seconds" && w[1] == "30"));
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--max-offset-seconds" && w[1] == "60"));
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--ffmpeg-path" && w[1] == "C:\\ffmpeg\\ffmpeg.exe"));
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--output-encoding" && w[1] == "utf-8"));
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--log-dir-path" && w[1].contains("logs")));
        // --parallel-workers 1 فقط برای نمونه‌برداری صوت و فقط یک‌بار
        assert_eq!(
            texts.iter().filter(|t| *t == "--parallel-workers").count(),
            1
        );
        let idx = texts
            .iter()
            .position(|t| t == "--parallel-workers")
            .unwrap();
        assert_eq!(texts[idx + 1], "1");

        // segment_count نادرست هم به مقدار امن ≥1 کلامپ می‌شود
        let bad = SyncConfig {
            segment_count: 0,
            ..on
        };
        let args = build_cli_args(
            &request(bad, None),
            SyncStrategy::MultiSegmentAudio,
            &output,
            &log_dir,
        );
        let texts: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let idx = texts.iter().position(|t| t == "--segment-count").unwrap();
        assert_eq!(texts[idx + 1], "1");
    }

    #[test]
    fn quality_offset_gate_stays_within_default_threshold() {
        let output = PathBuf::from("C:\\tmp\\out\\Show.S01E01.fa.synced.srt");
        let log_dir = PathBuf::from("C:\\tmp\\ws\\logs");
        let config = SyncConfig {
            multi_segment: true,
            segment_count: 6,
            skip_intro_outro: true,
            skip_sync_on_low_quality: true,
        };
        for strategy in [SyncStrategy::FullReference, SyncStrategy::MultiSegmentAudio] {
            let args = build_cli_args(&request(config, None), strategy, &output, &log_dir);
            let texts: Vec<String> = args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();

            let search = texts
                .windows(2)
                .find(|w| w[0] == "--max-offset-seconds")
                .map(|w| w[1].clone())
                .expect("--max-offset-seconds باید همیشه پاس شود");
            let quality = texts
                .windows(2)
                .find(|w| w[0] == "--quality-max-offset-seconds")
                .map(|w| w[1].clone())
                .expect("با فعال بودن گیت کیفیت، آستانهٔ آفست باید پاس شود");

            // گیت کیفیت هرگز از دامنهٔ جستجو بزرگ‌تر نیست: اسکور خام FFT
            // نرمال‌شده نیست و آفست‌های ۳۰–۶۰ ثانیه‌ایِ صوتی معمولاً پیک کاذب‌اند
            // (رسانهٔ واقعی: آفست 36.1s در حالی که زیرنویس از قبل سینک بود).
            assert_eq!(quality, "30", "{strategy:?}");
            assert_eq!(search, "60", "{strategy:?}");
        }

        // ترتیب استراتژی‌ها: تلاش ۰ مرجع کامل اول؛ نمونه‌برداری فقط با multi_segment
        let strategies = sync_strategies(&config, 0);
        assert_eq!(
            strategies,
            vec![SyncStrategy::FullReference, SyncStrategy::MultiSegmentAudio]
        );
        let only_full = sync_strategies(
            &SyncConfig {
                multi_segment: false,
                ..config
            },
            0,
        );
        assert_eq!(only_full, vec![SyncStrategy::FullReference]);
    }

    /// نردبان «تنظیم دوباره»: هر تلاش با استراتژیِ متفاوتی شروع می‌شود تا
    /// نتیجه با تلاش قبل واقعاً فرق کند؛ تلاش ۱ اول از شاهدِ مستقلِ صوتی
    /// استفاده می‌کند (تنها منبعی که وابستگیِ خودِ زیرنویس را ندارد).
    #[test]
    fn redo_attempts_rotate_strategies() {
        assert_eq!(
            sync_strategies(&sync_config(), 1),
            vec![
                SyncStrategy::AudioOnly,
                SyncStrategy::FullReference,
                SyncStrategy::MultiSegmentAudio
            ]
        );
        assert_eq!(
            sync_strategies(&sync_config(), 2),
            vec![
                SyncStrategy::FullReference,
                SyncStrategy::AudioOnly,
                SyncStrategy::MultiSegmentAudio
            ]
        );
        assert_eq!(
            sync_strategies(&sync_config(), 3),
            vec![
                SyncStrategy::MultiSegmentAudio,
                SyncStrategy::AudioOnly,
                SyncStrategy::FullReference
            ]
        );
        // تکرار با زیرنویس تازه — همان چرخه
        assert_eq!(
            sync_strategies(&sync_config(), 4),
            sync_strategies(&sync_config(), 1)
        );
        // هر نردبان: اولویت اول یکتا و سه استراتژی متمایز
        for attempt in 1u32..=6 {
            let ladder = sync_strategies(&sync_config(), attempt);
            assert_eq!(ladder.len(), 3);
            let unique: std::collections::HashSet<_> = ladder.iter().collect();
            assert_eq!(unique.len(), 3, "attempt {attempt}");
        }
    }

    /// آرگومان‌های AudioOnly: پرچم `--vad webrtc` مسیر زیرنویس داخلی را دور
    /// می‌زند و بدون پرچم‌های multi-segment اجرا می‌شود.
    #[test]
    fn audio_only_args_force_audio_vad() {
        let output = PathBuf::from("C:\\tmp\\out\\Show.S01E01.fa.synced.srt");
        let log_dir = PathBuf::from("C:\\tmp\\ws\\logs");
        let config = SyncConfig {
            multi_segment: true,
            segment_count: 6,
            skip_intro_outro: true,
            skip_sync_on_low_quality: true,
        };
        let args = build_cli_args(
            &request(config, None),
            SyncStrategy::AudioOnly,
            &output,
            &log_dir,
        );
        let texts: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let idx = texts.iter().position(|t| t == "--vad").unwrap();
        assert_eq!(texts[idx + 1], "webrtc");
        assert!(!texts.iter().any(|t| t.contains("multi-segment")));
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--quality-max-offset-seconds" && w[1] == "30"));
    }

    #[test]
    fn low_quality_marker_dominates_offset_marker_and_exit_code() {
        let logs = "[stderr]\nscore: 0.10\nlow-quality alignment (score too low); leaving subtitles unmodified\noffset seconds: 1.500\n";
        assert_eq!(classify_run(Some(0), logs), RunClassification::LowQuality);
        // حتی کد خروج غیرصفر هم اگر marker کیفیت پایین هست، اولویت دارد
        assert_eq!(classify_run(Some(2), logs), RunClassification::LowQuality);
    }

    #[test]
    fn zero_exit_without_offset_marker_is_unknown() {
        let logs = "[stderr]\nsync finished\n";
        assert_eq!(classify_run(Some(0), logs), RunClassification::Unknown);
        assert_eq!(classify_run(Some(0), ""), RunClassification::Unknown);
    }

    #[test]
    fn non_zero_or_missing_exit_code_is_failure() {
        assert_eq!(
            classify_run(Some(3), ""),
            RunClassification::FailedExit(Some(3))
        );
        assert_eq!(classify_run(None, ""), RunClassification::FailedExit(None));
        // marker آفست بدون کد خروج صفر هم شکست است
        assert_eq!(
            classify_run(Some(1), "offset seconds: 1.0"),
            RunClassification::FailedExit(Some(1))
        );
    }

    #[test]
    fn offset_marker_with_zero_exit_is_synchronized() {
        let logs = "[stderr]\nscore: 5.0\noffset seconds: -12.345\n";
        assert_eq!(
            classify_run(Some(0), logs),
            RunClassification::Synchronized {
                offset_seconds: -12.345
            }
        );
    }

    #[test]
    fn malformed_offset_marker_with_zero_exit_is_unknown() {
        // هیچ‌کدام از این‌ها نباید «سینک‌شده» شوند — fail-closed به Unknown
        let malformed = [
            "offset seconds: not-a-number",
            "offset seconds: NaN",
            "offset seconds: nan",
            "offset seconds: inf",
            "offset seconds: -inf",
            "offset seconds: Infinity",
            "offset seconds:",    // مقدار گم‌شده
            "offset seconds:   ", // مقدار خالی
            "offset seconds:\n",  // marker پایان‌یافته
        ];
        for logs in malformed {
            assert_eq!(
                classify_run(Some(0), logs),
                RunClassification::Unknown,
                "{logs:?}"
            );
        }
    }

    #[test]
    fn first_malformed_offset_marker_is_not_overridden_by_later_valid_one() {
        // فرمت واقعی ffsubsync برای یک ورودی فقط یک marker می‌دهد؛ پس اولین
        // مقدار خراب بدون هیچ override ای fail-closed می‌شود
        let logs = "offset seconds: garbage\nscore: 5.0\noffset seconds: 1.500\n";
        assert_eq!(classify_run(Some(0), logs), RunClassification::Unknown);
        assert_eq!(extract_offset_seconds(logs), None);
    }

    #[test]
    fn extract_offset_seconds_parses_first_value() {
        let logs = "score: 1.0\noffset seconds: -12.345\nmore text";
        assert!((extract_offset_seconds(logs).unwrap() - (-12.345)).abs() < 1e-9);
        assert_eq!(extract_offset_seconds("no marker here"), None);
        assert_eq!(extract_offset_seconds("offset seconds: not-a-number"), None);
        assert!((extract_offset_seconds("offset seconds: 0.000").unwrap()).abs() < 1e-9);
        // فقط مقادیر عددی متناهی پذیرفته می‌شوند
        assert_eq!(extract_offset_seconds("offset seconds: 12.5"), Some(12.5));
        assert_eq!(extract_offset_seconds("offset seconds: NaN"), None);
        assert_eq!(extract_offset_seconds("offset seconds: nan"), None);
        assert_eq!(extract_offset_seconds("offset seconds: inf"), None);
        assert_eq!(extract_offset_seconds("offset seconds: -inf"), None);
        assert_eq!(extract_offset_seconds("offset seconds: Infinity"), None);
    }

    #[test]
    fn log_tail_missing_or_empty_returns_none() {
        let tmp = unique_temp_dir("logtail");
        std::fs::create_dir_all(&tmp).unwrap();
        // فایل ناموجود
        assert_eq!(read_log_tail(&tmp.join("missing.log")), None);
        // فایل خالی
        let empty = tmp.join("empty.log");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(read_log_tail(&empty), None);
        // فایل فقط whitespace
        let blank = tmp.join("blank.log");
        std::fs::write(&blank, b"\n\n  \n").unwrap();
        assert_eq!(read_log_tail(&blank), None);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn log_tail_shorter_than_limit_returns_full_content() {
        let tmp = unique_temp_dir("logtail");
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("short.log");
        let content = "score: 5.0\noffset seconds: 1.234\n";
        std::fs::write(&path, content).unwrap();
        // متن trim می‌شود — newline انتهایی حذف می‌شود
        assert_eq!(
            read_log_tail(&path).as_deref(),
            Some("score: 5.0\noffset seconds: 1.234")
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn log_tail_larger_than_limit_returns_only_final_bytes() {
        let tmp = unique_temp_dir("logtail");
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("large.log");
        // ۱۲۸KiB + ۵۰۰ بایت — فقط آخرین ۱۲۸KiB باید برگردد
        let mut content = vec![b'x'; LOG_TAIL_LIMIT_BYTES as usize + 500];
        content.extend_from_slice(b"offset seconds: 1.500");
        std::fs::write(&path, &content).unwrap();

        let tail = read_log_tail(&path).expect("tail");
        assert_eq!(tail.len(), LOG_TAIL_LIMIT_BYTES as usize);
        // دقیقاً انتهای فایل — نه از ابتدای آن
        let expected_suffix = &content[content.len() - LOG_TAIL_LIMIT_BYTES as usize..];
        assert_eq!(tail.as_bytes(), expected_suffix);
        assert!(tail.ends_with("offset seconds: 1.500"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn marker_inside_retained_tail_is_detected() {
        let tmp = unique_temp_dir("logtail");
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("in-tail.log");
        // marker در انتهای فایل بزرگ — داخل tail نگه داشته می‌شود
        let mut content = vec![b'x'; LOG_TAIL_LIMIT_BYTES as usize + 500];
        content.extend_from_slice(b"offset seconds: 1.500");
        std::fs::write(&path, &content).unwrap();

        let tail = read_log_tail(&path).unwrap();
        assert_eq!(
            classify_run(Some(0), &tail),
            RunClassification::Synchronized {
                offset_seconds: 1.500
            }
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn marker_outside_retained_tail_does_not_create_false_success() {
        let tmp = unique_temp_dir("logtail");
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("out-of-tail.log");
        // marker فقط در ابتدای فایل — بیرون از tail؛ نباید false success بدهد
        let mut content = b"offset seconds: 1.500\n".to_vec();
        content.extend(std::iter::repeat_n(
            b'x',
            LOG_TAIL_LIMIT_BYTES as usize + 500,
        ));
        std::fs::write(&path, &content).unwrap();

        let tail = read_log_tail(&path).unwrap();
        assert!(
            !tail.contains("offset seconds"),
            "tail نباید marker بیرونی را داشته باشد"
        );
        assert_eq!(classify_run(Some(0), &tail), RunClassification::Unknown);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn bounded_tail_limits_chars() {
        let long = "abcdef".repeat(1000); // 6000 کاراکتر
        let tail = bounded_tail(&long, 100);
        assert_eq!(tail.chars().count(), 100);
        assert!(long.ends_with(&tail));
        assert_eq!(bounded_tail("کوتاه", 100), "کوتاه");
    }

    #[test]
    fn styling_loss_warning_only_for_ass_ssa() {
        assert!(super::super::styling_loss_warning("srt").is_none());
        assert!(super::super::styling_loss_warning("vtt").is_none());
        assert!(super::super::styling_loss_warning("ass").is_some());
        assert!(super::super::styling_loss_warning("ssa").is_some());
    }
}
