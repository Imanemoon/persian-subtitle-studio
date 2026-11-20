//! انتخاب انکودر ویدیو برای هاردساب.
//!
//! سرعت هدف اصلی است: در حالت Auto با یک انکود آزمایشی کوچک (~۱-۲ ثانیه)
//! اولین انکودر GPU اثبات‌شده (NVENC → QSV → AMF) انتخاب می‌شود؛ در صورت
//! نبود GPU یا شکست انکود HW، همان آیتم یک‌بار با CPU تلاش مجدد می‌شود.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::settings::{HardsubEncoder, HardsubQuality};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// انکودر انتخاب‌شده برای یک اجرا.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoder {
    Nvenc,
    Qsv,
    Amf,
    Libx264,
}

impl VideoEncoder {
    /// شناسهٔ ffmpeg — برای لاگ و گزارش به فرانت.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Nvenc => "h264_nvenc",
            Self::Qsv => "h264_qsv",
            Self::Amf => "h264_amf",
            Self::Libx264 => "libx264",
        }
    }

    /// برچسب نمایشی فارسی/انگلیسی برای UI.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Nvenc => "NVIDIA NVENC",
            Self::Qsv => "Intel QSV",
            Self::Amf => "AMD AMF",
            Self::Libx264 => "CPU (libx264)",
        }
    }

    /// آرگومان‌های کدک ویدیو مطابق سطح کیفیت؛ صدا stream-copy می‌شود و
    /// وارد این آرگومان‌ها نمی‌شود.
    pub fn codec_args(&self, quality: HardsubQuality) -> Vec<&'static str> {
        match self {
            Self::Nvenc => match quality {
                HardsubQuality::Fast => vec![
                    "-c:v",
                    "h264_nvenc",
                    "-preset",
                    "p4",
                    "-rc",
                    "vbr",
                    "-cq",
                    "23",
                    "-b:v",
                    "0",
                ],
                HardsubQuality::Balanced => vec![
                    "-c:v",
                    "h264_nvenc",
                    "-preset",
                    "p5",
                    "-rc",
                    "vbr",
                    "-cq",
                    "22",
                    "-b:v",
                    "0",
                ],
                HardsubQuality::High => vec![
                    "-c:v",
                    "h264_nvenc",
                    "-preset",
                    "p6",
                    "-rc",
                    "vbr",
                    "-cq",
                    "20",
                    "-b:v",
                    "0",
                ],
            },
            Self::Qsv => match quality {
                HardsubQuality::Fast => vec![
                    "-c:v",
                    "h264_qsv",
                    "-preset",
                    "veryfast",
                    "-global_quality",
                    "23",
                ],
                HardsubQuality::Balanced => vec![
                    "-c:v",
                    "h264_qsv",
                    "-preset",
                    "fast",
                    "-global_quality",
                    "22",
                ],
                HardsubQuality::High => vec![
                    "-c:v",
                    "h264_qsv",
                    "-preset",
                    "medium",
                    "-global_quality",
                    "20",
                ],
            },
            Self::Amf => match quality {
                HardsubQuality::Fast => vec![
                    "-c:v", "h264_amf", "-quality", "speed", "-rc", "cqp", "-qp_i", "23", "-qp_p",
                    "23", "-qp_b", "23",
                ],
                HardsubQuality::Balanced => vec![
                    "-c:v", "h264_amf", "-quality", "balanced", "-rc", "cqp", "-qp_i", "22",
                    "-qp_p", "22", "-qp_b", "22",
                ],
                HardsubQuality::High => vec![
                    "-c:v", "h264_amf", "-quality", "quality", "-rc", "cqp", "-qp_i", "20",
                    "-qp_p", "20", "-qp_b", "20",
                ],
            },
            Self::Libx264 => match quality {
                HardsubQuality::Fast => {
                    vec!["-c:v", "libx264", "-preset", "veryfast", "-crf", "23"]
                }
                HardsubQuality::Balanced => {
                    vec!["-c:v", "libx264", "-preset", "faster", "-crf", "22"]
                }
                HardsubQuality::High => vec!["-c:v", "libx264", "-preset", "medium", "-crf", "20"],
            },
        }
    }
}

/// انکود آزمایشی کوچک — آیا ffmpeg این انکودر را واقعاً می‌تواند اجرا کند؟
///
/// درایور GPU ممکن است نصب باشد ولی انکودر در دسترس نباشد (سرور headless،
/// رنج سِشن پر، درایور قدیمی)؛ برای همین probe واقعی اجباری است و نتیجه
/// فقط از exit code واقعی ffmpeg ساخته می‌شود.
pub fn probe_encoder(ffmpeg: &Path, encoder: VideoEncoder, cancel: &AtomicBool) -> bool {
    let mut command = Command::new(ffmpeg);
    command.args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=320x240:d=0.2:r=10",
        "-frames:v",
        "2",
    ]);
    for arg in encoder.codec_args(HardsubQuality::Fast) {
        command.arg(arg);
    }
    command.args(["-f", "null", "-"]);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return false,
    };
    // stderr فقط برای مسدودنشدن بافر خوانده می‌شود؛ نتیجه از exit code است.
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");
    let stderr_reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut value = String::new();
        let _ = stderr_pipe.read_to_string(&mut value);
        value
    });

    let deadline = Instant::now() + PROBE_TIMEOUT;
    let success = loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            // خوانندهٔ stderr join نمی‌شود — درس mux/ffprobe: فرایند نوه‌ای
            // ممکن است هندل لوله را نگه دارد؛ detach تا لغو بلافاصله برگردد.
            drop(stderr_reader);
            return false;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(stderr_reader);
                return false;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(stderr_reader);
                return false;
            }
        }
    };

    let _ = stderr_reader.join();
    success
}

/// انتخاب انکودر نهایی از تنظیمات.
///
/// - Auto: اولین انکودر GPU اثبات‌شده، وگرنه CPU.
/// - Override (Nvidia/Intel/Amd): فقط همان انکودر probe می‌شود؛ شکست → CPU
///   (تصمیم عمل‌گرا: کاربر به‌جای خطا خروجی می‌گیرد و انکودر واقعی در UI
///   شفاف گزارش می‌شود).
pub fn resolve_encoder(
    ffmpeg: &Path,
    setting: HardsubEncoder,
    cancel: &AtomicBool,
) -> VideoEncoder {
    let order: &[VideoEncoder] = match setting {
        HardsubEncoder::Auto => &[VideoEncoder::Nvenc, VideoEncoder::Qsv, VideoEncoder::Amf],
        HardsubEncoder::Nvidia => &[VideoEncoder::Nvenc],
        HardsubEncoder::Intel => &[VideoEncoder::Qsv],
        HardsubEncoder::Amd => &[VideoEncoder::Amf],
        HardsubEncoder::Cpu => &[],
    };
    for candidate in order {
        if probe_encoder(ffmpeg, *candidate, cancel) {
            return *candidate;
        }
    }
    VideoEncoder::Libx264
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "subsync-hardsub-encoder-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn write_script(tag: &str, body: &str) -> PathBuf {
        let dir = unique_temp_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fake-ffmpeg.cmd");
        fs::write(&path, body).unwrap();
        path
    }

    /// آرگومان‌های هر انکودر باید کدک درست و کیفیت مطابق سطح داشته باشند.
    #[test]
    fn codec_args_match_encoder_and_quality() {
        let fast = VideoEncoder::Nvenc.codec_args(HardsubQuality::Fast);
        assert!(fast.contains(&"h264_nvenc"));
        assert!(fast.contains(&"-cq"));
        assert_eq!(
            *fast.iter().find(|a| **a == "23").unwrap(),
            "23",
            "سطح Fast باید cq/crf ۲۳ بدهد"
        );

        let high = VideoEncoder::Libx264.codec_args(HardsubQuality::High);
        assert!(high.contains(&"libx264"));
        assert!(high.contains(&"medium"));

        let amd = VideoEncoder::Amf.codec_args(HardsubQuality::Balanced);
        assert!(amd.contains(&"h264_amf"));

        let qsv = VideoEncoder::Qsv.codec_args(HardsubQuality::Fast);
        assert!(qsv.contains(&"h264_qsv"));
    }

    /// probe فقط از exit code واقعی تصمیم می‌گیرد — نه از وجود درایور.
    #[test]
    #[cfg(windows)]
    fn probe_accepts_and_rejects_by_exit_code() {
        let ok = write_script("probe-ok", "@echo off\r\nexit /b 0\r\n");
        let bad = write_script("probe-bad", "@echo off\r\nexit /b 1\r\n");
        let cancel = AtomicBool::new(false);
        assert!(probe_encoder(&ok, VideoEncoder::Nvenc, &cancel));
        assert!(!probe_encoder(&bad, VideoEncoder::Nvenc, &cancel));
        // script ناموجود → spawn شکست → false (بدون panic)
        assert!(!probe_encoder(
            Path::new("Z:\\definitely-missing-ffmpeg.cmd"),
            VideoEncoder::Nvenc,
            &cancel
        ));
    }

    /// Override با probe ناموفق → CPU (بدون خطا؛ شفافیت از طریق UI).
    #[test]
    #[cfg(windows)]
    fn resolve_encoder_overrides_fall_back_to_cpu() {
        let bad = write_script("resolve-bad", "@echo off\r\nexit /b 1\r\n");
        let cancel = AtomicBool::new(false);
        assert_eq!(
            resolve_encoder(&bad, HardsubEncoder::Nvidia, &cancel),
            VideoEncoder::Libx264
        );
        assert_eq!(
            resolve_encoder(&bad, HardsubEncoder::Cpu, &cancel),
            VideoEncoder::Libx264,
            "تنظیم Cpu باید بدون probe مستقیم CPU بدهد"
        );
    }

    /// Auto با انکودر NVENC جعلیِ سالم → Nvenc (نه ادامه به QSV).
    #[test]
    #[cfg(windows)]
    fn resolve_encoder_auto_stops_at_first_working_gpu() {
        let ok = write_script("resolve-ok", "@echo off\r\nexit /b 0\r\n");
        let cancel = AtomicBool::new(false);
        assert_eq!(
            resolve_encoder(&ok, HardsubEncoder::Auto, &cancel),
            VideoEncoder::Nvenc
        );
    }
}
