//! استخراج زیرنویس با FFmpeg — فایل موقت، اعتبارسنجی، انتقال اتمیک بدون بازنویسی.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::models::SubtitleValidation;

use super::validation::validate_file;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// حداکثر زمان انتظار برای استخراج یک Track (فایل‌های بزرگ MKV نیاز به seek دارند).
const EXTRACT_TIMEOUT: Duration = Duration::from_secs(180);

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum ExtractError {
    Cancelled,
    Spawn(String),
    /// FFmpeg با کد خطا خارج شد (جزئیات stderr)
    Unsuccessful(String),
    /// خروجی تولید شد اما معتبر نیست (فایل موقت پاک شده)
    Validation(SubtitleValidation),
    /// مقصد در فاصله بین بررسی و انتقال ایجاد شده — هرگز بازنویسی نمی‌کنیم
    TargetExists(PathBuf),
    Io(String),
}

impl ExtractError {
    pub fn message(&self) -> String {
        match self {
            Self::Cancelled => "عملیات توسط کاربر لغو شد".into(),
            Self::Spawn(e) => format!("اجرای FFmpeg ناموفق بود: {e}"),
            Self::Unsuccessful(d) => format!("FFmpeg شکست خورد: {d}"),
            Self::Validation(v) => format!(
                "خروجی اعتبارسنجی نشد: {}",
                v.error.as_deref().unwrap_or("دلیل نامشخص")
            ),
            Self::TargetExists(p) => format!(
                "مقصد «{}» موجود است — برای جلوگیری از بازنویسی هیچ فایلی دست نخورد",
                p.display()
            ),
            Self::Io(e) => format!("خطای فایل: {e}"),
        }
    }
}

/// استخراج Track به `<video-base>.fa.srt` با جریان امن:
/// temp → validate → move_if_absent. در همهٔ مسیرهای خطا، فایل موقت پاک می‌شود.
pub fn extract_fa_srt(
    ffmpeg: &Path,
    video: &Path,
    stream_index: u32,
    cancel: &AtomicBool,
) -> Result<(PathBuf, SubtitleValidation), ExtractError> {
    let dir = video
        .parent()
        .ok_or_else(|| ExtractError::Io("ویدیو پوشه والد ندارد".into()))?;
    let base = video
        .file_stem()
        .ok_or_else(|| ExtractError::Io("نام ویدیو نامعتبر است".into()))?
        .to_string_lossy()
        .into_owned();

    let target = dir.join(format!("{base}.fa.srt"));
    extract_and_move(ffmpeg, video, stream_index, &target, cancel)
}

/// مسیر فایل بررسی برای یک Track: `<base>.track-<N>.review.srt`
pub fn review_target(video: &Path, stream_index: u32) -> Option<PathBuf> {
    let base = video.file_stem()?.to_string_lossy().into_owned();
    let dir = video.parent().filter(|d| !d.as_os_str().is_empty())?;
    Some(dir.join(format!("{base}.track-{stream_index}.review.srt")))
}

/// استخراج آزمایشی یک Track به `<base>.track-<N>.review.srt` — بدون بازنویسی.
pub fn extract_for_review(
    ffmpeg: &Path,
    video: &Path,
    stream_index: u32,
    cancel: &AtomicBool,
) -> Result<(PathBuf, SubtitleValidation), ExtractError> {
    let target = review_target(video, stream_index)
        .ok_or_else(|| ExtractError::Io("نمی‌توان مسیر فایل بررسی را ساخت".into()))?;
    extract_and_move(ffmpeg, video, stream_index, &target, cancel)
}

/// هسته مشترک: temp یکتا → اعتبارسنجی → انتقال اتمیک بدون بازنویسی.
/// در همهٔ مسیرهای خطا فقط فایل موقتِ همین عملیات پاک می‌شود.
fn extract_and_move(
    ffmpeg: &Path,
    video: &Path,
    stream_index: u32,
    target: &Path,
    cancel: &AtomicBool,
) -> Result<(PathBuf, SubtitleValidation), ExtractError> {
    let dir = target
        .parent()
        .ok_or_else(|| ExtractError::Io("مقصد پوشه والد ندارد".into()))?;
    let base = video
        .file_stem()
        .ok_or_else(|| ExtractError::Io("نام ویدیو نامعتبر است".into()))?
        .to_string_lossy()
        .into_owned();

    let token = unique_token();
    let temp = dir.join(format!("{base}.fa.extracting.{token}.srt"));

    let result = extract_to(ffmpeg, video, stream_index, &temp, cancel);
    match result {
        Ok(validation) => {
            // انتقال اتمیک روی همان درایو — بدون بازنویسی
            if target.exists() {
                let _ = std::fs::remove_file(&temp);
                return Err(ExtractError::TargetExists(target.to_path_buf()));
            }
            std::fs::rename(&temp, target).map_err(|e| {
                let _ = std::fs::remove_file(&temp);
                ExtractError::Io(format!("انتقال به مقصد: {e}"))
            })?;
            Ok((target.to_path_buf(), validation))
        }
        Err(e) => {
            // پاک‌سازی فقط فایل موقتِ همین عملیات
            let _ = std::fs::remove_file(&temp);
            Err(e)
        }
    }
}

fn extract_to(
    ffmpeg: &Path,
    video: &Path,
    stream_index: u32,
    temp: &Path,
    cancel: &AtomicBool,
) -> Result<SubtitleValidation, ExtractError> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-n", // فایل موقت نباید بازنویسی شود
        "-i",
    ]);
    cmd.arg(video);
    cmd.arg("-map");
    cmd.arg(format!("0:{stream_index}")); // Global stream index
    cmd.arg("-c:s");
    cmd.arg("srt");
    cmd.arg(temp);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| ExtractError::Spawn(e.to_string()))?;
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");
    let t_err = std::thread::spawn(move || {
        use std::io::Read;
        let mut s = String::new();
        let _ = stderr_pipe.read_to_string(&mut s);
        s
    });

    let deadline = Instant::now() + EXTRACT_TIMEOUT;
    let status = loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ExtractError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ExtractError::Unsuccessful(
                        "FFmpeg در زمان مشخص تمام نشد (Timeout)".into(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(ExtractError::Io(format!("انتظار برای FFmpeg: {e}")));
            }
        }
    };

    let stderr = t_err.join().unwrap_or_default();
    let Some(st) = status else {
        return Err(ExtractError::Unsuccessful("Timeout".into()));
    };
    if !st.success() {
        let detail = stderr
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(ExtractError::Unsuccessful(if detail.is_empty() {
            format!("کد خروج {}", st.code().unwrap_or(-1))
        } else {
            detail
        }));
    }

    let validation = validate_file(temp);
    if !validation.valid {
        return Err(ExtractError::Validation(validation));
    }
    Ok(validation)
}

fn unique_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{:x}", nanos, counter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_tokens_are_unique() {
        let a = unique_token();
        let b = unique_token();
        assert_ne!(a, b);
    }

    /// تست زندهٔ انتها-به-انتها با فایل رسانه‌ای مصنوعی (فقط `cargo test -- --ignored`).
    /// یک MKV یک‌ثانیه‌ای با Track زیرنویس subrip فارسی می‌سازد و کل جریان
    /// probe → classify → extract → validate → move را آزمایش می‌کند.
    /// هیچ فایل رسانه‌ای داخل ریپو قرار نمی‌گیرد — همه چیز در temp ساخته و پاک می‌شود.
    #[test]
    #[ignore]
    fn live_ffmpeg_probe_extract_roundtrip() {
        use crate::models::{AnalysisStatus, SubtitleKind};

        let ffmpeg =
            crate::tools::resolve_tool_path("ffmpeg", &Default::default()).expect("ffmpeg در PATH");
        let ffprobe = crate::tools::resolve_tool_path("ffprobe", &Default::default())
            .expect("ffprobe در PATH");

        let dir = std::env::temp_dir().join(format!(
            "subsync-live-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let fixture = dir.join("fixture.srt");
        std::fs::write(
            &fixture,
            "1\n00:00:00,200 --> 00:00:00,800\nسلام دنیا\n\n2\n00:00:00,850 --> 00:00:00,990\nخط دوم فارسی\n",
        )
        .unwrap();
        let video = dir.join("Show.S01E01.720p.mkv");

        // ساخت فایل مصنوعی: ویدیو + صوت + زیرنویس با Tag فارسی
        let mut cmd = Command::new(&ffmpeg);
        cmd.args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=320x240:d=1",
            "-f",
            "lavfi",
            "-i",
            "sine=d=1",
            "-i",
        ])
        .arg(&fixture)
        .args(["-map", "0:v", "-map", "1:a", "-map", "2:s"])
        .args([
            "-metadata:s:s:0",
            "language=fas",
            "-metadata:s:s:0",
            "title=فارسی",
        ])
        .args(["-c:v", "mpeg4", "-q:v", "7", "-c:a", "aac", "-c:s", "srt"])
        .arg(&video)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let out = cmd.output().expect("اجرای ffmpeg");
        assert!(
            out.status.success(),
            "ساخت فایل مصنوعی ناموفق: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        // ۱) پروب — Global index حفظ شود
        let streams = crate::media::ffprobe::probe_subtitle_streams(&ffprobe, &video).unwrap();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].stream_index, 2);
        assert_eq!(streams[0].kind, SubtitleKind::Text);
        assert!(streams[0].persian_candidate);

        // ۲) طبقه‌بندی — بدون ابهام
        let (status, _) = crate::subtitle::detection::classify_analysis(&streams);
        assert_eq!(status, AnalysisStatus::PersianTextTrackFound);

        // ۳) مقصد مجاز است
        let video_path = video.clone();
        let _target =
            crate::subtitle::detection::check_extraction_target(&video_path, &[]).unwrap();

        // ۴) استخراج کامل
        let cancel = AtomicBool::new(false);
        let (target, validation) =
            extract_fa_srt(&ffmpeg, &video, streams[0].stream_index, &cancel).expect("استخراج");
        assert_eq!(target, dir.join("Show.S01E01.720p.fa.srt"));
        assert!(validation.valid);
        assert_eq!(validation.cue_count, 2);
        assert!(validation.has_persian, "محتوا باید فارسی تأیید شود");
        assert_eq!(validation.encoding, "UTF-8");
        assert!(target.is_file());

        // ۵) فایل موقت پاک شده باشد
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".extracting."))
            .collect();
        assert!(leftovers.is_empty(), "فایل موقت باقی مانده: {leftovers:?}");

        // ۶) استخراج دوباره → ممنوع (بازنویسی ممنوع)
        let err = extract_fa_srt(&ffmpeg, &video, streams[0].stream_index, &cancel).unwrap_err();
        assert!(matches!(err, ExtractError::TargetExists(_)), "{err:?}");

        // ۷) لغو قبل از اجرا → Cancelled و بدون فایل باقی‌مانده
        let cancel_now = AtomicBool::new(true);
        let video2 = dir.join("Show.S01E02.720p.mkv");
        std::fs::copy(&video, &video2).unwrap();
        let err =
            extract_fa_srt(&ffmpeg, &video2, streams[0].stream_index, &cancel_now).unwrap_err();
        assert!(matches!(err, ExtractError::Cancelled), "{err:?}");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".extracting."))
            .collect();
        assert!(leftovers.is_empty(), "فایل موقت بعد از لغو باقی مانده");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
