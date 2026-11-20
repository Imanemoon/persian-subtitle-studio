//! هستهٔ هاردساب — burn-in زیرنویس فارسی سینک‌شده داخل تصویر ویدیو.
//!
//! قواعد:
//! - فایل اصلی **هرگز** تغییر نمی‌کند؛ خروجی یک فایل جدید است و اگر مقصد
//!   از قبل موجود باشد، عملیات Conflict می‌دهد (بدون بازنویسی).
//! - workspace موقت کنار ویدیو (همان volume) برای rename اتمیک خروجی.
//! - فیلتر `subtitles` فقط با نام نسبی `sub.srt` صدا زده می‌شود و ffmpeg
//!   با `current_dir(workspace)` اجرا می‌شود — دور زدن کامل escape
//!   دردسرساز مسیرهای ویندوزی در فیلترها (کولن/بک‌اسلش/آپاستروف).
//! - پیشرفت فقط از `-progress pipe:1` (out_time واقعی ffmpeg) و فقط وقتی
//!   مدت ویدیو معلوم است؛ clamp ≤99 تا اعتبارسنجی نهایی — هیچ درصد ساختگی.
//! - لغو/timeout: kill+wait فرایند و detach خوانندهٔ stderr (درس mux/ffprobe:
//!   فرایند نوه‌ای ممکن است هندل لوله را نگه دارد).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::archive::listing::is_link_or_reparse;
use crate::hardsub::encoder::VideoEncoder;
use crate::hardsub::plan::SubtitleSource;
use crate::media::ffprobe;
use crate::settings::HardsubQuality;
use crate::subtitle::validation::validate_file;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// سقف استخراج Track داخلی (فایل‌های بزرگ MKV نیاز به seek دارند).
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(180);

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BurnError {
    Cancelled,
    Invalid(String),
    Probe(String),
    Extract(String),
    Spawn(String),
    Unsuccessful(String),
    Timeout,
    Io(String),
    /// خروجی مقصد موجود است — هرگز بازنویسی نمی‌شود
    Conflict,
}

impl BurnError {
    pub fn message(&self) -> String {
        match self {
            Self::Cancelled => "عملیات هاردساب توسط کاربر لغو شد".into(),
            Self::Invalid(m) | Self::Probe(m) | Self::Extract(m) | Self::Spawn(m)
            | Self::Unsuccessful(m) | Self::Io(m) => m.clone(),
            Self::Timeout => "هاردساب بیش از زمان مجاز طول کشید؛ فرایند متوقف شد".into(),
            Self::Conflict => {
                "خروجی در فاصلهٔ شروع تا نصب ساخته شد — برای جلوگیری از بازنویسی رد شد".into()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BurnOutcome {
    pub output_path: PathBuf,
    pub encoder_used: VideoEncoder,
    /// هشدار پاک‌سازی workspace؛ موفقیت هاردساب را باطل نمی‌کند.
    pub cleanup_warning: Option<String>,
}

/// درخواست burn — همهٔ داده‌ها توسط engine از پلن ساخته می‌شود.
pub struct BurnRequest<'a> {
    pub ffmpeg: &'a Path,
    pub ffprobe: &'a Path,
    pub video: &'a Path,
    /// مسیر خروجی نهایی (هرگز نباید از قبل موجود باشد).
    pub output: &'a Path,
    pub source: SubtitleSource,
    pub encoder: VideoEncoder,
    pub quality: HardsubQuality,
    /// مدت ویدیو از ffprobe — فقط برای timeout و درصد؛ None = بدون درصد.
    pub duration_seconds: Option<f64>,
    /// override timeout برای تست‌ها؛ None = clamp(duration×40, ۲۰ دقیقه, ۴ ساعت).
    pub timeout_override: Option<Duration>,
    pub cancel: &'a AtomicBool,
    /// درصد زنده (0..=99) — None یعنی هنوز تخمین واقعی‌ای نیست.
    pub progress_slot: Option<Arc<Mutex<Option<u32>>>>,
}

/// هستهٔ هاردساب برای یک ویدیو — workspace → آماده‌سازی sub.srt → ffmpeg →
/// اعتبارسنجی → نصب اتمیک. در همهٔ مسیرهای خطا فقط workspace همین عملیات
/// پاک می‌شود و فایل اصلی/زیرنویس دست‌نخورده می‌مانند.
pub fn burn(request: &BurnRequest) -> Result<BurnOutcome, BurnError> {
    if request.cancel.load(Ordering::SeqCst) {
        return Err(BurnError::Cancelled);
    }
    validate_plain_file(request.video, "ویدیو")?;
    if fs::symlink_metadata(request.output).is_ok() {
        return Err(BurnError::Conflict);
    }

    let parent = request
        .video
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or_else(|| BurnError::Invalid("پوشهٔ والد ویدیو مشخص نیست".into()))?;
    let workspace = create_workspace(parent)?;

    let result = burn_in_workspace(request, &workspace);
    if result.is_err() {
        let _ = fs::remove_dir_all(&workspace);
        return result;
    }

    // پاک‌سازی نهایی — هشدار موفقیت را باطل نمی‌کند
    let cleanup_warning = fs::remove_dir_all(&workspace).err().map(|error| {
        format!(
            "هاردساب با موفقیت نصب شد، اما پاک‌سازی پوشهٔ موقت ناموفق بود: {error} ({})",
            workspace.display()
        )
    });
    let outcome = result?;
    Ok(BurnOutcome {
        cleanup_warning,
        ..outcome
    })
}

fn burn_in_workspace(
    request: &BurnRequest,
    workspace: &Path,
) -> Result<BurnOutcome, BurnError> {
    let cancel = request.cancel;

    // ۱) آماده‌سازی sub.srt داخل workspace (نام نسبی ساده برای فیلتر)
    let sub_srt = workspace.join("sub.srt");
    match &request.source {
        SubtitleSource::File(path) => {
            validate_plain_file(path, "زیرنویس")?;
            let validation = validate_file(path);
            if !validation.valid || !validation.has_persian {
                return Err(BurnError::Invalid(format!(
                    "زیرنویس ورودی معتبر و فارسی نیست: {}",
                    validation
                        .error
                        .unwrap_or_else(|| "محتوای فارسی تأیید نشد".into())
                )));
            }
            fs::copy(path, &sub_srt).map_err(|error| {
                BurnError::Io(format!("کپی زیرنویس به فضای موقت ناموفق بود: {error}"))
            })?;
        }
        SubtitleSource::EmbeddedStream { stream_index } => {
            if cancel.load(Ordering::SeqCst) {
                return Err(BurnError::Cancelled);
            }
            extract_embedded(request.ffmpeg, request.video, *stream_index, &sub_srt, cancel)?;
            let validation = validate_file(&sub_srt);
            if !validation.valid || !validation.has_persian {
                return Err(BurnError::Extract(format!(
                    "Track داخلی استخراج شد اما زیرنویس فارسی معتبر نیست: {}",
                    validation
                        .error
                        .unwrap_or_else(|| "محتوای فارسی تأیید نشد".into())
                )));
            }
        }
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(BurnError::Cancelled);
    }

    // ۲) خروجی موقت روی همان volume (workspace کنار ویدیوست)
    let extension = request
        .output
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("mkv");
    let candidate = workspace.join(format!("hardsub-out.{extension}"));

    // ۳) انکود — تنها جایی که ویدیو بازانکود می‌شود
    run_burn_ffmpeg(request, workspace, &candidate)?;

    if cancel.load(Ordering::SeqCst) {
        return Err(BurnError::Cancelled);
    }

    // ۴) اعتبارسنجی خروجی: فایل معمولیِ غیرخالی + خوانا با ffprobe
    let candidate_meta = fs::symlink_metadata(&candidate).map_err(|error| {
        BurnError::Unsuccessful(format!("خروجی موقت FFmpeg ساخته نشد: {error}"))
    })?;
    if !candidate_meta.is_file() || candidate_meta.len() == 0 {
        return Err(BurnError::Unsuccessful(
            "خروجی هاردساب خالی یا فایل معمولی نیست؛ فایل اصلی تغییر نکرد".into(),
        ));
    }
    ffprobe::probe_duration(request.ffprobe, &candidate, cancel).map_err(|error| {
        BurnError::Unsuccessful(format!(
            "خروجی هاردساب قابل اعتبارسنجی نبود (ffprobe): {} — فایل اصلی تغییر نکرد",
            error.message()
        ))
    })?;

    // ۵) نصب اتمیک با نام نهایی — بدون بازنویسی
    if request.cancel.load(Ordering::SeqCst) {
        return Err(BurnError::Cancelled);
    }
    if fs::symlink_metadata(request.output).is_ok() {
        return Err(BurnError::Conflict);
    }
    fs::rename(&candidate, request.output).map_err(|error| {
        BurnError::Io(format!("نصب خروجی هاردساب ناموفق بود: {error}"))
    })?;

    Ok(BurnOutcome {
        output_path: request.output.to_path_buf(),
        encoder_used: request.encoder,
        cleanup_warning: None,
    })
}

/// استخراج Track متنی داخلی به `sub.srt` داخل workspace — الگوی
/// `subtitle/extraction.rs` ولی مقصد فایل خصوصی workspace است و به
/// rename/تعارض نیاز ندارد.
fn extract_embedded(
    ffmpeg: &Path,
    video: &Path,
    stream_index: u32,
    sub_srt: &Path,
    cancel: &AtomicBool,
) -> Result<(), BurnError> {
    let workspace = sub_srt
        .parent()
        .ok_or_else(|| BurnError::Io("workspace بدون والد است".into()))?;
    let mut command = Command::new(ffmpeg);
    command.args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-i",
    ]);
    command.arg(video);
    command.arg("-map");
    command.arg(format!("0:{stream_index}")); // index گلوبال
    command.arg("-c:s");
    command.arg("srt");
    command.arg("sub.srt"); // نسبی — cwd همان workspace است
    command
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command
        .spawn()
        .map_err(|error| BurnError::Spawn(format!("اجرای FFmpeg برای استخراج Track ناموفق بود: {error}")))?;
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");
    let stderr_reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut value = String::new();
        let _ = stderr_pipe.read_to_string(&mut value);
        value
    });

    let deadline = Instant::now() + EXTRACT_TIMEOUT;
    let status = loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            drop(stderr_reader); // detach — درس فرایند نوه‌ای
            return Err(BurnError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(stderr_reader);
                return Err(BurnError::Timeout);
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(stderr_reader);
                return Err(BurnError::Io(format!("انتظار برای FFmpeg ناموفق بود: {error}")));
            }
        }
    };

    let stderr = stderr_reader.join().unwrap_or_default();
    if !status.success() {
        let detail = stderr
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(4)
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(BurnError::Extract(if detail.is_empty() {
            format!(
                "استخراج Track داخلی با کد خروج {} شکست خورد",
                status.code().unwrap_or(-1)
            )
        } else {
            format!("استخراج Track داخلی ناموفق بود: {detail}")
        }));
    }
    Ok(())
}

/// انکود اصلی — فیلتر subtitles با نام نسبی و cwd=workspace.
fn run_burn_ffmpeg(
    request: &BurnRequest,
    workspace: &Path,
    candidate: &Path,
) -> Result<(), BurnError> {
    let cancel = request.cancel;
    let mut command = Command::new(request.ffmpeg);
    command.args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-progress",
        "pipe:1",
        "-i",
    ]);
    command.arg(request.video);
    // فقط استریم اول ویدیو بازانکود می‌شود تا `-vf` بدون ابهام به همان
    // استریم اعمال شود؛ همهٔ استریم‌های صدا copy می‌شوند (چندزبانه حفظ می‌شود)
    // و زیرنویس/داده خروجی نمی‌آیند (زیرنویس داخل تصویر است).
    command.args([
        "-map",
        "0:v:0",
        "-map",
        "0:a?",
        "-sn",
        "-dn",
        "-map_metadata",
        "0",
        "-map_chapters",
        "0",
        "-c:a",
        "copy",
    ]);
    for arg in request.encoder.codec_args(request.quality) {
        command.arg(arg);
    }
    command.arg("-vf");
    command.arg("subtitles=sub.srt"); // نسبی + cwd=workspace → بدون escape
    if matches!(
        request
            .output
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase()),
        Some(ref ext) if ext == "mp4" || ext == "m4v" || ext == "mov"
    ) {
        command.arg("-movflags");
        command.arg("+faststart");
    }
    command.arg(candidate);
    command
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command.spawn().map_err(|error| {
        BurnError::Spawn(format!("اجرای FFmpeg برای هاردساب ناموفق بود: {error}"))
    })?;
    let stdout_pipe = child.stdout.take().expect("stdout piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");
    let progress_slot = request.progress_slot.clone();
    let duration = request.duration_seconds;
    let stdout_reader = std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        let reader = BufReader::new(stdout_pipe);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(percent) = progress_percent_from_line(&line, duration) {
                if let Some(slot) = &progress_slot {
                    let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
                    *slot = Some(percent);
                }
            }
        }
    });
    let stderr_reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut value = String::new();
        let _ = stderr_pipe.read_to_string(&mut value);
        value
    });

    let timeout = request.timeout_override.unwrap_or_else(|| {
        // clamp(duration×40, ۲۰ دقیقه, ۴ ساعت)؛ بدون مدت معلوم → ۲ ساعت
        let base = request.duration_seconds.unwrap_or(7200.0);
        let secs = (base * 40.0) as u64;
        Duration::from_secs(secs.clamp(20 * 60, 4 * 60 * 60))
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            // خواننده‌ها join نمی‌شوند — detach تا لغو بلافاصله برگردد
            drop(stdout_reader);
            drop(stderr_reader);
            return Err(BurnError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(stdout_reader);
                drop(stderr_reader);
                return Err(BurnError::Timeout);
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(stdout_reader);
                drop(stderr_reader);
                return Err(BurnError::Io(format!("انتظار برای FFmpeg ناموفق بود: {error}")));
            }
        }
    };

    let stderr = stderr_reader.join().unwrap_or_default();
    let _ = stdout_reader.join(); // stdout کوچک است — join تا آخرین درصد ثبت شود
    // timeout/لغو داخل حلقه Err برمی‌گردانند؛ اینجا همیشه Some است
    let Some(status) = status else {
        return Err(BurnError::Timeout);
    };
    if !status.success() {
        let detail = stderr
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(4)
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(BurnError::Unsuccessful(if detail.is_empty() {
            format!(
                "FFmpeg برای هاردساب با کد خروج {} شکست خورد؛ فایل اصلی تغییر نکرد",
                status.code().unwrap_or(-1)
            )
        } else {
            format!("هاردساب ناموفق بود: {detail}")
        }));
    }
    Ok(())
}

/// درصد واقعی از یک خط `-progress` — بدون مدت معلوم هیچ درصدی نمی‌دهد.
pub fn progress_percent_from_line(line: &str, duration_seconds: Option<f64>) -> Option<u32> {
    let Some(duration) = duration_seconds else {
        return None;
    };
    let out_time_us = parse_out_time_us(line)?;
    Some(percent_for(out_time_us, duration))
}

/// یک خط خروجی `-progress` → microsecond. `out_time_ms` کوارک ffmpeg است
/// و واقعاً microsecond دارد؛ هر دو پشتیبانی می‌شوند.
pub fn parse_out_time_us(line: &str) -> Option<f64> {
    let rest = line
        .strip_prefix("out_time_us=")
        .or_else(|| line.strip_prefix("out_time_ms="))?;
    let value: f64 = rest.trim().parse().ok()?;
    if value.is_finite() {
        Some(value)
    } else {
        None
    }
}

/// درصد clamp شدهٔ 0..=99 — ۱۰۰ هرگز از این مسیر نمی‌آید؛ فقط اعتبارسنجی
/// نهایی و نصب، وضعیت Done می‌سازد.
pub fn percent_for(out_time_us: f64, duration_seconds: f64) -> u32 {
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return 0;
    }
    let percent = (out_time_us / 1_000_000.0 / duration_seconds * 100.0) as i64;
    percent.clamp(0, 99) as u32
}

fn validate_plain_file(path: &Path, label: &str) -> Result<(), BurnError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| BurnError::Invalid(format!("{label} قابل دسترسی نیست: {error}")))?;
    if !metadata.is_file() || is_link_or_reparse(&metadata) || metadata.len() == 0 {
        return Err(BurnError::Invalid(format!(
            "{label} باید یک فایل معمولیِ غیرخالی و بدون symlink/reparse باشد"
        )));
    }
    Ok(())
}

fn create_workspace(parent: &Path) -> Result<PathBuf, BurnError> {
    for _ in 0..12 {
        let token = unique_token();
        let workspace = parent.join(format!(".subsync-hardsub-{token}"));
        match fs::create_dir(&workspace) {
            Ok(()) => return Ok(workspace),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(BurnError::Io(format!(
                    "ساخت فضای موقت هاردساب ناموفق بود: {error}"
                )))
            }
        }
    }
    Err(BurnError::Io(
        "برای ساخت فضای موقت یکتا تلاش‌های کافی انجام شد، اما نام آزاد پیدا نشد".into(),
    ))
}

fn unique_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{counter:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "subsync-hardsub-burn-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }

    const SRT_FA: &str = "1\n00:00:01,000 --> 00:00:02,000\nسلام دنیا\n\n2\n00:00:02,100 --> 00:00:03,900\nخط دوم فارسی\n";

    // ---------- تست‌های pure (بدون فرایند) ----------

    #[test]
    fn out_time_parsing_and_percent_clamping() {
        assert_eq!(parse_out_time_us("out_time_us=250000"), Some(250000.0));
        // کوارک ffmpeg: out_time_ms واقعاً microsecond است
        assert_eq!(parse_out_time_us("out_time_ms=250000"), Some(250000.0));
        assert_eq!(parse_out_time_us("progress=continue"), None);
        assert_eq!(parse_out_time_us("frame=12"), None);
        assert_eq!(parse_out_time_us("out_time_us=abc"), None);
        assert_eq!(parse_out_time_us("out_time_us=NaN"), None);

        // 0.25s از 1s → ۲۵٪
        assert_eq!(percent_for(250000.0, 1.0), 25);
        // overshoot → clamp 99 (هیچ ۱۰۰ از این مسیر نمی‌آید)
        assert_eq!(percent_for(5_000_000.0, 1.0), 99);
        assert_eq!(percent_for(-10.0, 1.0), 0);
        // مدت نامعتبر → 0، نه panic
        assert_eq!(percent_for(1000.0, 0.0), 0);
        assert_eq!(percent_for(1000.0, f64::NAN), 0);

        assert_eq!(progress_percent_from_line("out_time_us=500000", Some(1.0)), Some(50));
        assert_eq!(
            progress_percent_from_line("out_time_us=500000", None),
            None,
            "بدون مدت معلوم نباید درصد ساختگی بدهد"
        );
    }

    #[test]
    fn workspace_tokens_are_unique() {
        assert_ne!(unique_token(), unique_token());
    }

    // ---------- تست‌های فرایند واقعی با ffmpeg جعلی (فقط ویندوز) ----------

    /// ffmpeg جعلی سالم: فقط اگر sub.srt در cwd بود، خروجی (آخرین arg) را
    /// با مارکر می‌سازد و یک خط پیشرفت واقعی می‌دهد.
    const FAKE_FFMPEG_OK: &str = concat!(
        "@echo off\r\n",
        "set \"LAST=\"\r\n",
        ":loop\r\n",
        "if \"%~1\"==\"\" goto run\r\n",
        "set \"LAST=%~1\"\r\n",
        "shift\r\n",
        "goto loop\r\n",
        ":run\r\n",
        "if not exist sub.srt exit /b 3\r\n",
        "echo out_time_us=250000\r\n",
        "echo progress=continue\r\n",
        ">>\"%LAST%\" echo HARDSUB-OUTPUT-MARKER\r\n",
        "exit /b 0\r\n"
    );

    const FAKE_FFMPEG_FAIL: &str = "@echo off\r\necho Failure reason issued by fake ffmpeg >&2\r\nexit /b 1\r\n";

    /// ffmpeg جعلی بدون ساخت خروجی — خروجی خالی باید Unsuccessful شود.
    const FAKE_FFMPEG_NO_OUTPUT: &str = "@echo off\r\nexit /b 0\r\n";

    /// ffmpeg جعلی «در جریان» — حلقهٔ محاسباتی خالص cmd با دست‌دادن شروع؛
    /// kill کردن آن هیچ نوه‌ای باقی نمی‌گذارد (درس e2e/ffprobe).
    const FAKE_FFMPEG_SLOW_TEMPLATE: &str = "@echo off\r\necho started > \"{}\"\r\nfor /l %%i in (1,1,100000000) do @set /a x=%%i >nul\r\nexit /b 0\r\n";

    struct Fixture {
        dir: PathBuf,
        video: PathBuf,
        subtitle: PathBuf,
        output: PathBuf,
        ffprobe: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn fixture(tag: &str, ffmpeg_body: &str) -> Fixture {
        let dir = unique_temp_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        let video = dir.join("Movie.S01E01.720p.mkv");
        fs::write(&video, b"FAKE-VIDEO-CONTENTS").unwrap();
        let subtitle = dir.join("Movie.S01E01.720p.fa.synced.srt");
        fs::write(&subtitle, SRT_FA).unwrap();
        let output = dir.join("Movie.S01E01.720p.fa.hardsubbed.mkv");
        let ffprobe = dir.join("fake-probe.cmd");
        fs::write(&ffprobe, "@echo off\r\necho 12.5\r\n").unwrap();
        let ffmpeg = dir.join("fake-ffmpeg.cmd");
        fs::write(&ffmpeg, ffmpeg_body).unwrap();
        Fixture {
            dir,
            video,
            subtitle,
            output,
            ffprobe,
        }
    }

    fn request<'a>(
        fx: &'a Fixture,
        ffmpeg: &'a Path,
        cancel: &'a AtomicBool,
        slot: Option<Arc<Mutex<Option<u32>>>>,
        timeout: Option<Duration>,
    ) -> BurnRequest<'a> {
        BurnRequest {
            ffmpeg,
            ffprobe: &fx.ffprobe,
            video: &fx.video,
            output: &fx.output,
            source: SubtitleSource::File(fx.subtitle.clone()),
            encoder: VideoEncoder::Libx264,
            quality: HardsubQuality::Fast,
            duration_seconds: Some(1.0),
            timeout_override: timeout,
            cancel,
            progress_slot: slot,
        }
    }

    fn leftover_workspaces(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().starts_with(".subsync-hardsub-"))
                    .unwrap_or(false)
            })
            .collect()
    }

    #[test]
    #[cfg(windows)]
    fn successful_burn_creates_new_output_and_keeps_source_untouched() {
        let fx = fixture("ok", FAKE_FFMPEG_OK);
        let ffmpeg = fx.dir.join("fake-ffmpeg.cmd");
        let cancel = AtomicBool::new(false);
        let slot = Arc::new(Mutex::new(None));
        let video_before = fs::read(&fx.video).unwrap();
        let sub_before = fs::read(&fx.subtitle).unwrap();

        let result = burn(&request(&fx, &ffmpeg, &cancel, Some(Arc::clone(&slot)), None));
        let outcome = result.unwrap_or_else(|error| panic!("burn باید موفق شود: {error:?}"));

        // خروجی جدید با محتوای fake ساخته و نصب شده
        assert_eq!(outcome.output_path, fx.output);
        let output_text = fs::read_to_string(&fx.output).unwrap();
        assert!(output_text.contains("HARDSUB-OUTPUT-MARKER"));
        assert_eq!(
            outcome.encoder_used,
            VideoEncoder::Libx264
        );
        // فایل اصلی و زیرنویس دست‌نخورده
        assert_eq!(fs::read(&fx.video).unwrap(), video_before);
        assert_eq!(fs::read(&fx.subtitle).unwrap(), sub_before);
        // درصد واقعی از خط out_time_us ثبت شده (0.25s از 1s)
        let percent = slot.lock().unwrap_or_else(PoisonError::into_inner).clone();
        assert_eq!(percent, Some(25));
        // workspace پاک شده
        assert!(
            leftover_workspaces(&fx.dir).is_empty(),
            "workspace پس از موفقیت باید پاک شود: {:?}",
            leftover_workspaces(&fx.dir)
        );
    }

    #[test]
    #[cfg(windows)]
    fn burn_failure_preserves_source_and_cleans_workspace() {
        let fx = fixture("fail", FAKE_FFMPEG_FAIL);
        let ffmpeg = fx.dir.join("fake-ffmpeg.cmd");
        let cancel = AtomicBool::new(false);
        let video_before = fs::read(&fx.video).unwrap();

        let result = burn(&request(&fx, &ffmpeg, &cancel, None, None));
        match &result {
            Err(BurnError::Unsuccessful(message)) => {
                assert!(message.contains("Failure reason"), "جزئیات stderr باید بیاید: {message}");
            }
            other => panic!("انتظار Unsuccessful، دریافت: {other:?}"),
        }
        assert!(!fx.output.exists(), "در شکست نباید خروجی نصب شود");
        assert_eq!(fs::read(&fx.video).unwrap(), video_before);
        assert!(
            leftover_workspaces(&fx.dir).is_empty(),
            "workspace پس از شکست باید پاک شود"
        );
    }

    #[test]
    #[cfg(windows)]
    fn empty_output_is_rejected_and_workspace_cleaned() {
        let fx = fixture("empty", FAKE_FFMPEG_NO_OUTPUT);
        let ffmpeg = fx.dir.join("fake-ffmpeg.cmd");
        let cancel = AtomicBool::new(false);

        let result = burn(&request(&fx, &ffmpeg, &cancel, None, None));
        assert!(matches!(result, Err(BurnError::Unsuccessful(_))), "{result:?}");
        assert!(!fx.output.exists());
        assert!(leftover_workspaces(&fx.dir).is_empty());
    }

    #[test]
    #[cfg(windows)]
    fn pre_existing_output_conflicts_without_running_ffmpeg() {
        let fx = fixture("conflict", FAKE_FFMPEG_OK);
        let ffmpeg = fx.dir.join("fake-ffmpeg.cmd");
        let cancel = AtomicBool::new(false);
        fs::write(&fx.output, b"ALREADY-THERE").unwrap();

        let result = burn(&request(&fx, &ffmpeg, &cancel, None, None));
        assert!(matches!(result, Err(BurnError::Conflict)), "{result:?}");
        assert_eq!(
            fs::read(&fx.output).unwrap(),
            b"ALREADY-THERE",
            "خروجی موجود هرگز بازنویسی نمی‌شود"
        );
    }

    #[test]
    #[cfg(windows)]
    fn invalid_sidecar_rejected_before_spawn() {
        let fx = fixture("invalid-sub", FAKE_FFMPEG_OK);
        let ffmpeg = fx.dir.join("fake-ffmpeg.cmd");
        let cancel = AtomicBool::new(false);
        fs::write(&fx.subtitle, "").unwrap(); // خالی → نامعتبر

        let result = burn(&request(&fx, &ffmpeg, &cancel, None, None));
        assert!(matches!(result, Err(BurnError::Invalid(_))), "{result:?}");
        assert!(!fx.output.exists());
        assert!(leftover_workspaces(&fx.dir).is_empty());
    }

    /// لغو حین انکود با دست‌دادن قطعی شروع: fake اول marker می‌نویسد، تست
    /// فقط بعد از دیدن marker لغو می‌کند؛ برگشت باید سریع باشد و منبع
    /// دست‌نخورده بماند.
    #[test]
    #[cfg(windows)]
    fn cancel_during_encode_kills_child_and_cleans_workspace() {
        let marker_dir = unique_temp_dir("cancel-marker");
        fs::create_dir_all(&marker_dir).unwrap();
        let marker = marker_dir.join("started.marker");
        let fx = fixture(
            "cancel",
            &FAKE_FFMPEG_SLOW_TEMPLATE.replace("{}", &marker.to_string_lossy()),
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let video_before = fs::read(&fx.video).unwrap();

        // فقط PathBufها به thread می‌روند — Fixture اینجا می‌ماند تا Drop پس از
        // ادعاها انجام شود (دو Drop پشت‌سرهم پوشه را زودتر حذف می‌کرد)
        let t_cancel = cancel.clone();
        let t_video = fx.video.clone();
        let t_ffmpeg = fx.dir.join("fake-ffmpeg.cmd");
        let t_ffprobe = fx.ffprobe.clone();
        let t_output = fx.output.clone();
        let t_subtitle = fx.subtitle.clone();
        let worker = std::thread::spawn(move || {
            let req = BurnRequest {
                ffmpeg: &t_ffmpeg,
                ffprobe: &t_ffprobe,
                video: &t_video,
                output: &t_output,
                source: SubtitleSource::File(t_subtitle.clone()),
                encoder: VideoEncoder::Libx264,
                quality: HardsubQuality::Fast,
                duration_seconds: Some(1.0),
                timeout_override: Some(Duration::from_secs(60)),
                cancel: &t_cancel,
                progress_slot: None,
            };
            burn(&req)
        });

        // دست‌دادن شروع: انتظار محدود برای نوشته‌شدن marker توسط فرایند فرزند
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.is_file() {
            assert!(Instant::now() < deadline, "فرایند جعلی شروع نشد");
            std::thread::sleep(Duration::from_millis(25));
        }
        cancel.store(true, Ordering::SeqCst);

        let started_cancel = Instant::now();
        let result = worker.join().unwrap_or_else(|e| std::panic::resume_unwind(e));
        assert!(
            started_cancel.elapsed() < Duration::from_secs(15),
            "لغو باید بلافاصله برگردد: {:?}",
            started_cancel.elapsed()
        );
        assert!(matches!(result, Err(BurnError::Cancelled)), "{result:?}");
        assert!(!fx.output.exists());
        assert_eq!(fs::read(&fx.video).unwrap(), video_before);
        assert!(
            leftover_workspaces(&fx.dir).is_empty(),
            "workspace پس از لغو باید پاک شود"
        );
    }

    /// timeout — فرایند گیرکرده kill و خطای Timeout جدا از شکست ffmpeg.
    #[test]
    #[cfg(windows)]
    fn stuck_encode_times_out_and_cleans_workspace() {
        let marker_dir = unique_temp_dir("timeout-marker");
        fs::create_dir_all(&marker_dir).unwrap();
        let marker = marker_dir.join("started.marker");
        let fx = fixture(
            "timeout",
            &FAKE_FFMPEG_SLOW_TEMPLATE.replace("{}", &marker.to_string_lossy()),
        );
        let ffmpeg = fx.dir.join("fake-ffmpeg.cmd");
        let cancel = AtomicBool::new(false);

        let started = Instant::now();
        let result = burn(&request(&fx, &ffmpeg, &cancel, None, Some(Duration::from_millis(700))));
        let elapsed = started.elapsed();
        assert!(matches!(result, Err(BurnError::Timeout)), "{result:?}");
        assert!(elapsed >= Duration::from_millis(600), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(15), "{elapsed:?}");
        assert!(!fx.output.exists());
        assert!(leftover_workspaces(&fx.dir).is_empty());
    }
}
