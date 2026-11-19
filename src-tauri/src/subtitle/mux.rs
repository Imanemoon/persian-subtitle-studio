//! Remux ایمن زیرنویس فارسی سینک‌شده به‌صورت SoftSub داخل ویدیو.
//!
//! ورودی اصلی هرگز در محل تغییر داده نمی‌شود: FFmpeg ابتدا یک خروجی کامل در
//! پوشهٔ موقتِ همان volume می‌سازد؛ فقط پس از خروج موفق، انتقال به فایل اصلی
//! با backup و rollback انجام می‌شود. لغو/شکست پیش از مرحلهٔ swap، ویدیو را
//! دست‌نخورده باقی می‌گذارد.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::archive::listing::is_link_or_reparse;
use crate::models::SubtitleKind;
use crate::subtitle::validation::validate_file;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const MUX_TIMEOUT: Duration = Duration::from_secs(10 * 60);
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MuxError {
    Cancelled,
    Invalid(String),
    Probe(String),
    Spawn(String),
    Unsuccessful(String),
    Timeout,
    Io(String),
}

impl std::fmt::Display for MuxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "عملیات SoftSub لغو شد"),
            Self::Invalid(message)
            | Self::Probe(message)
            | Self::Spawn(message)
            | Self::Unsuccessful(message)
            | Self::Io(message) => write!(f, "{message}"),
            Self::Timeout => write!(f, "ساخت SoftSub بیش از زمان مجاز طول کشید"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MuxSuccess {
    /// هشدار پاک‌سازی backup/workspace؛ موفقیت remux را باطل نمی‌کند.
    pub cleanup_warning: Option<String>,
}

/// فایل ویدیو را با یک subtitle ورودی remux می‌کند.
///
/// replace_stream_index فقط برای Track فارسی داخلی‌ای است که همین اجرا از آن
/// استخراج کرده است؛ در این حالت همان Track حذف و نسخهٔ سینک‌شده جایگزین می‌شود.
/// برای sidecar یا زیرنویس دانلودی None بدهید تا تمام استریم‌های قبلی حفظ شوند.
pub fn softsub_in_place(
    ffmpeg: &Path,
    ffprobe: &Path,
    video: &Path,
    subtitle: &Path,
    replace_stream_index: Option<u32>,
    cancel: &AtomicBool,
) -> Result<MuxSuccess, MuxError> {
    validate_plain_file(video, "ویدیو")?;
    validate_plain_file(subtitle, "زیرنویس")?;

    if video == subtitle {
        return Err(MuxError::Invalid(
            "مسیر ویدیو و زیرنویس نمی‌تواند یکسان باشد".into(),
        ));
    }

    let video_name = video
        .file_name()
        .ok_or_else(|| MuxError::Invalid("نام فایل ویدیو معتبر نیست".into()))?;
    let ext = video
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let subtitle_codec = codec_for_container(&ext).ok_or_else(|| {
        MuxError::Invalid(format!(
            "قرار دادن SoftSub در کانتینر «.{ext}» پشتیبانی نمی‌شود؛ فایل اصلی بدون تغییر باقی ماند"
        ))
    })?;

    let validation = validate_file(subtitle);
    if !validation.valid || !validation.has_persian {
        return Err(MuxError::Invalid(format!(
            "زیرنویس ورودی معتبر و فارسی نیست: {}",
            validation
                .error
                .unwrap_or_else(|| "محتوای فارسی تأیید نشد".into())
        )));
    }

    let streams = crate::media::ffprobe::probe_subtitle_streams(ffprobe, video, cancel)
        .map_err(|error| match error {
            crate::media::ffprobe::ProbeError::Cancelled => MuxError::Cancelled,
            crate::media::ffprobe::ProbeError::Failed(reason) => MuxError::Probe(reason),
        })?;

    let output_subtitle_index = if let Some(index) = replace_stream_index {
        let Some(selected) = streams.iter().find(|stream| stream.stream_index == index) else {
            return Err(MuxError::Invalid(format!(
                "Track زیرنویس شمارهٔ {index} در بازبینی نهایی پیدا نشد؛ ویدیو تغییر نکرد"
            )));
        };
        if selected.kind != SubtitleKind::Text
            || !(selected.persian_candidate || selected.persian_confirmed)
        {
            return Err(MuxError::Invalid(
                "Track انتخاب‌شده دیگر یک زیرنویس متنی فارسی معتبر نیست؛ ویدیو تغییر نکرد"
                    .into(),
            ));
        }
        streams.len().saturating_sub(1)
    } else {
        streams.len()
    };

    let parent = video
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| MuxError::Invalid("پوشهٔ والد ویدیو مشخص نیست".into()))?;

    // workspace حتماً کنار فایل اصلی و روی همان volume است تا rename اتمیک باشد.
    let workspace = create_workspace(parent)?;
    let candidate = workspace.join(video_name);

    let mux_result = run_ffmpeg(
        ffmpeg,
        video,
        subtitle,
        &candidate,
        subtitle_codec,
        output_subtitle_index,
        replace_stream_index,
        cancel,
    );

    if let Err(error) = mux_result {
        let _ = fs::remove_dir_all(&workspace);
        return Err(error);
    }

    if cancel.load(Ordering::SeqCst) {
        let _ = fs::remove_dir_all(&workspace);
        return Err(MuxError::Cancelled);
    }

    let candidate_meta = fs::symlink_metadata(&candidate)
        .map_err(|error| MuxError::Io(format!("خروجی موقت FFmpeg ساخته نشد: {error}")))?;
    if !candidate_meta.is_file() || candidate_meta.len() == 0 {
        let _ = fs::remove_dir_all(&workspace);
        return Err(MuxError::Unsuccessful(
            "خروجی remux خالی یا فایل معمولی نیست؛ ویدیو تغییر نکرد".into(),
        ));
    }

    // اگر فایل اصلی حین remux به symlink/reparse تبدیل شده باشد، روی آن نمی‌نویسیم.
    let source_meta = fs::symlink_metadata(video)
        .map_err(|error| MuxError::Io(format!("بازبینی دوبارهٔ ویدیو ناموفق بود: {error}")))?;
    if !source_meta.is_file() || is_link_or_reparse(&source_meta) {
        let _ = fs::remove_dir_all(&workspace);
        return Err(MuxError::Invalid(
            "فایل ویدیو هنگام remux به فایل معمولی قابل‌اعتماد تبدیل نماند".into(),
        ));
    }

    // backup و candidate روی همان volume هستند. اگر نصب candidate شکست بخورد،
    // backup به مسیر اصلی برمی‌گردد؛ در صورت شکست rollback مسیر backup گزارش می‌شود.
    let backup = workspace.join(format!("{}.original-backup", video_name.to_string_lossy()));
    fs::rename(video, &backup).map_err(|error| {
        let _ = fs::remove_dir_all(&workspace);
        MuxError::Io(format!(
            "ایجاد backup امن از ویدیوی اصلی ناموفق بود: {error} — فایل اصلی تغییر نکرد"
        ))
    })?;

    if let Err(install_error) = fs::rename(&candidate, video) {
        return match fs::rename(&backup, video) {
            Ok(()) => {
                let _ = fs::remove_dir_all(&workspace);
                Err(MuxError::Io(format!(
                    "نصب خروجی SoftSub ناموفق بود و ویدیوی اصلی با موفقیت بازیابی شد: {install_error}"
                )))
            }
            Err(restore_error) => Err(MuxError::Io(format!(
                "نصب خروجی SoftSub و بازیابی خودکار ناموفق بود: {install_error}; نسخهٔ اصلی در backup باقی مانده است: {} ({restore_error})",
                backup.display()
            ))),
        };
    }

    let cleanup_warning = fs::remove_dir_all(&workspace).err().map(|error| {
        format!(
            "SoftSub با موفقیت نصب شد، اما پاک‌سازی پوشهٔ موقت ناموفق بود: {error} ({})",
            workspace.display()
        )
    });

    Ok(MuxSuccess { cleanup_warning })
}

fn validate_plain_file(path: &Path, label: &str) -> Result<(), MuxError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| MuxError::Invalid(format!("{label} قابل دسترسی نیست: {error}")))?;
    if !metadata.is_file() || is_link_or_reparse(&metadata) || metadata.len() == 0 {
        return Err(MuxError::Invalid(format!(
            "{label} باید یک فایل معمولیِ غیرخالی و بدون symlink/reparse باشد"
        )));
    }
    Ok(())
}

/// کدک سازگار برای subtitle جدید؛ سایر استریم‌ها بدون تغییر stream-copy می‌شوند.
fn codec_for_container(extension: &str) -> Option<&'static str> {
    match extension {
        "mkv" => Some("srt"),
        "mp4" | "m4v" | "mov" => Some("mov_text"),
        "webm" => Some("webvtt"),
        _ => None,
    }
}

fn create_workspace(parent: &Path) -> Result<PathBuf, MuxError> {
    for _ in 0..12 {
        let token = unique_token();
        let workspace = parent.join(format!(".subsync-softsub-{token}"));
        match fs::create_dir(&workspace) {
            Ok(()) => return Ok(workspace),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(MuxError::Io(format!(
                    "ساخت فضای موقت SoftSub ناموفق بود: {error}"
                )))
            }
        }
    }
    Err(MuxError::Io(
        "برای ساخت فضای موقت یکتا تلاش‌های کافی انجام شد، اما نام آزاد پیدا نشد".into(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn run_ffmpeg(
    ffmpeg: &Path,
    video: &Path,
    subtitle: &Path,
    candidate: &Path,
    subtitle_codec: &str,
    output_subtitle_index: usize,
    replace_stream_index: Option<u32>,
    cancel: &AtomicBool,
) -> Result<(), MuxError> {
    let spec = output_subtitle_index.to_string();
    let mut command = Command::new(ffmpeg);
    command.args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-n",
        "-i",
    ]);
    command.arg(video);
    command.arg("-i");
    command.arg(subtitle);
    command.args(["-map", "0"]);
    if let Some(index) = replace_stream_index {
        command.arg("-map");
        command.arg(format!("-0:{index}"));
    }
    command.args(["-map", "1:0", "-map_metadata", "0", "-map_chapters", "0", "-c", "copy"]);
    command.arg(format!("-c:s:{spec}"));
    command.arg(subtitle_codec);
    command.arg(format!("-metadata:s:s:{spec}"));
    command.arg("language=fas");
    command.arg(format!("-metadata:s:s:{spec}"));
    command.arg("title=Persian (Synced)");
    command.arg(format!("-disposition:s:{spec}"));
    command.arg("default");
    command.arg(candidate);
    command
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
        .map_err(|error| MuxError::Spawn(format!("اجرای FFmpeg برای ساخت SoftSub ناموفق بود: {error}")))?;
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");
    let stderr_reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut value = String::new();
        let _ = stderr_pipe.read_to_string(&mut value);
        value
    });

    let deadline = Instant::now() + MUX_TIMEOUT;
    let status = loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stderr_reader.join();
            return Err(MuxError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_reader.join();
                return Err(MuxError::Timeout);
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_reader.join();
                return Err(MuxError::Io(format!(
                    "انتظار برای پایان FFmpeg ناموفق بود: {error}"
                )));
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
        return Err(MuxError::Unsuccessful(if detail.is_empty() {
            format!(
                "FFmpeg برای ساخت SoftSub با کد خروج {} شکست خورد؛ ویدیو تغییر نکرد",
                status.code().unwrap_or(-1)
            )
        } else {
            format!("ساخت SoftSub ناموفق بود: {detail}")
        }));
    }
    Ok(())
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

    #[test]
    fn codec_selection_is_container_specific() {
        assert_eq!(codec_for_container("mkv"), Some("srt"));
        assert_eq!(codec_for_container("mp4"), Some("mov_text"));
        assert_eq!(codec_for_container("webm"), Some("webvtt"));
        assert_eq!(codec_for_container("avi"), None);
        assert_eq!(codec_for_container("ts"), None);
    }

    #[test]
    fn workspace_tokens_are_unique() {
        assert_ne!(unique_token(), unique_token());
    }

    #[test]
    fn output_subtitle_index_excludes_only_the_replaced_stream() {
        let streams = [0_u32, 2, 4];
        let index = streams.iter().filter(|stream| **stream != 2).count();
        assert_eq!(index, 2);
    }
}
