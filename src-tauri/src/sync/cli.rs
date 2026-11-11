//! اجرای امن ffs.exe: ساخت آرگومان‌ها، اجرای بدون shell، ثبت خروجی با سقف حافظه،
//! طبقه‌بندی fail-closed و خاتمهٔ درخت فرایند.

use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::types::{SyncRequest, CAPTURE_CAP_BYTES, LOW_QUALITY_MARKER, OFFSET_MARKER};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// فاصلهٔ polling برای `try_wait` — هم‌راستا با سایر آداپتورها.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

// ---------------------------------------------------------------------
// ساخت آرگومان‌ها
// ---------------------------------------------------------------------

/// آرگومان‌های دقیق ffs — هر آرگومان جدا، بدون هیچ shell.
///
/// قرارداد تأییدشدهٔ ffsubsync 0.5.1:
/// `ffs <video> -i <sub> -o <out> [--multi-segment-sync --segment-count N]
///  [--skip-intro-outro] [--skip-sync-on-low-quality] --parallel-workers 1
///  --output-encoding utf-8 [--ffmpeg-path <ffmpeg>] --log-dir-path <dir>`
///
/// `--parallel-workers 1` همیشه اعمال می‌شود — تعداد workerهای سطح ویدیو را
/// pipeline کنترل می‌کند، نه هر فرایند.
pub fn build_cli_args(request: &SyncRequest, output: &Path, log_dir: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        request.video_path.as_os_str().to_os_string(),
        "-i".into(),
        request.subtitle_path.as_os_str().to_os_string(),
        "-o".into(),
        output.as_os_str().to_os_string(),
    ];

    if request.config.multi_segment {
        args.push("--multi-segment-sync".into());
        args.push("--segment-count".into());
        args.push(request.config.effective_segment_count().to_string().into());
    }
    if request.config.skip_intro_outro {
        args.push("--skip-intro-outro".into());
    }
    if request.config.skip_sync_on_low_quality {
        args.push("--skip-sync-on-low-quality".into());
    }

    args.push("--parallel-workers".into());
    args.push("1".into());
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RunClassification {
    /// marker کیفیت پایین — بر کد خروج و marker آفست اولویت دارد؛ خروجی دور ریخته می‌شود.
    LowQuality,
    /// کد خروج صفر + marker آفست — هنوز باید فایل خروجی اعتبارسنجی شود.
    Synchronized,
    /// کد خروج غیرصفر (یا خاتمهٔ غیرعادی بدون کد).
    FailedExit(Option<i32>),
    /// کد خروج صفر بدون marker آفست — هیچ ادعایی دربارهٔ سینک ممکن نیست.
    Unknown,
}

/// marker کیفیت پایین بر همه‌چیز مقدم است (قانون ۱)، بعد کد خروج، بعد marker آفست.
pub(crate) fn classify_run(exit_code: Option<i32>, logs: &str) -> RunClassification {
    if logs.contains(LOW_QUALITY_MARKER) {
        return RunClassification::LowQuality;
    }
    match exit_code {
        Some(0) => {
            if logs.contains(OFFSET_MARKER) {
                RunClassification::Synchronized
            } else {
                RunClassification::Unknown
            }
        }
        Some(code) => RunClassification::FailedExit(Some(code)),
        None => RunClassification::FailedExit(None),
    }
}

/// استخراج آفست از marker `offset seconds: %.3f` — اولین مقدار معتبر.
pub(crate) fn extract_offset_seconds(logs: &str) -> Option<f64> {
    let idx = logs.find(OFFSET_MARKER)?;
    let rest = &logs[idx + OFFSET_MARKER.len()..];
    let token = rest.split_whitespace().next()?;
    token.trim_end_matches(',').parse::<f64>().ok()
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
        let log_path = log_dir.join("ffsubsync.log");
        if let Ok(bytes) = std::fs::read(&log_path) {
            let text = String::from_utf8_lossy(&bytes);
            if !text.trim().is_empty() {
                logs.push_str("[ffsubsync.log]\n");
                logs.push_str(&text);
                logs.push('\n');
            }
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

    fn request(config: SyncConfig, ffmpeg: Option<PathBuf>) -> SyncRequest {
        SyncRequest {
            video_path: PathBuf::from("C:\\media\\Show.S01E01.mkv"),
            subtitle_path: PathBuf::from("C:\\media\\Show.S01E01.fa.srt"),
            ffsubsync_path: PathBuf::from("C:\\tools\\ffs.exe"),
            ffmpeg_path: ffmpeg,
            config,
            workspace_root: None,
        }
    }

    #[test]
    fn cli_args_map_settings_and_enforce_single_worker() {
        let output = PathBuf::from("C:\\tmp\\out\\Show.S01E01.fa.synced.srt");
        let log_dir = PathBuf::from("C:\\tmp\\ws\\logs");

        // همهٔ پرچم‌ها خاموش — هیچ پرچم اختیاری نباید بیاید
        let off = SyncConfig {
            multi_segment: false,
            segment_count: 6,
            skip_intro_outro: false,
            skip_sync_on_low_quality: false,
        };
        let args = build_cli_args(&request(off, None), &output, &log_dir);
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
        assert!(!texts.iter().any(|t| t.contains("multi-segment")));
        assert!(!texts.iter().any(|t| t.contains("segment-count")));
        assert!(!texts.iter().any(|t| t.contains("skip-intro-outro")));
        assert!(!texts.iter().any(|t| t.contains("skip-sync-on-low-quality")));
        assert!(!texts.iter().any(|t| t.contains("ffmpeg-path")));
        // --parallel-workers 1 همیشه و فقط یک‌بار
        assert_eq!(
            texts.iter().filter(|t| *t == "--parallel-workers").count(),
            1
        );
        let idx = texts
            .iter()
            .position(|t| t == "--parallel-workers")
            .unwrap();
        assert_eq!(texts[idx + 1], "1");
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--output-encoding" && w[1] == "utf-8"));
        assert!(texts
            .windows(2)
            .any(|w| w[0] == "--log-dir-path" && w[1].contains("logs")));

        // همهٔ پرچم‌ها روشن + ffmpeg
        let on = SyncConfig {
            multi_segment: true,
            segment_count: 6,
            skip_intro_outro: true,
            skip_sync_on_low_quality: true,
        };
        let args = build_cli_args(
            &request(on, Some(PathBuf::from("C:\\ffmpeg\\ffmpeg.exe"))),
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
            .any(|w| w[0] == "--ffmpeg-path" && w[1] == "C:\\ffmpeg\\ffmpeg.exe"));
        assert_eq!(
            texts.iter().filter(|t| *t == "--parallel-workers").count(),
            1
        );

        // segment_count نادرست هم به مقدار امن ≥1 کلامپ می‌شود
        let bad = SyncConfig {
            segment_count: 0,
            ..on
        };
        let args = build_cli_args(&request(bad, None), &output, &log_dir);
        let texts: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let idx = texts.iter().position(|t| t == "--segment-count").unwrap();
        assert_eq!(texts[idx + 1], "1");
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
        assert_eq!(classify_run(Some(0), logs), RunClassification::Synchronized);
    }

    #[test]
    fn extract_offset_seconds_parses_first_value() {
        let logs = "score: 1.0\noffset seconds: -12.345\nmore text";
        assert!((extract_offset_seconds(logs).unwrap() - (-12.345)).abs() < 1e-9);
        assert_eq!(extract_offset_seconds("no marker here"), None);
        assert_eq!(extract_offset_seconds("offset seconds: not-a-number"), None);
        assert!((extract_offset_seconds("offset seconds: 0.000").unwrap()).abs() < 1e-9);
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
