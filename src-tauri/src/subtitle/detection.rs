//! تشخیص وضعیت ویدیو بر اساس Streamها و بررسی مقصد استخراج.

use std::path::{Path, PathBuf};

use crate::models::{AnalysisStatus, SubtitleFileInfo, SubtitleKind, SubtitleStream};

use super::encoding::{decode_bytes, has_persian_text};

/// وضعیت تحلیل را از روی Streamهای شناخته‌شده تعیین می‌کند.
///
/// Track متنی فارسی = کاندید از روی Tag/عنوان، یا تأییدشده از طریق استخراج آزمایشی.
pub fn classify_analysis(streams: &[SubtitleStream]) -> (AnalysisStatus, Option<String>) {
    if streams.is_empty() {
        return (
            AnalysisStatus::NoEmbeddedSubtitle,
            Some("هیچ Stream زیرنویسی داخل فایل نیست".into()),
        );
    }

    let is_fa_text = |s: &SubtitleStream| {
        s.kind == SubtitleKind::Text && (s.persian_candidate || s.persian_confirmed)
    };
    let fa_text_count = streams.iter().filter(|s| is_fa_text(s)).count();
    let fa_image_count = streams
        .iter()
        .filter(|s| s.persian_candidate && s.kind == SubtitleKind::Image)
        .count();
    let fa_unknown_count = streams
        .iter()
        .filter(|s| s.persian_candidate && s.kind == SubtitleKind::Unknown)
        .count();

    if fa_text_count == 1 {
        return (AnalysisStatus::PersianTextTrackFound, None);
    }
    if fa_text_count > 1 {
        return (
            AnalysisStatus::NeedsTrackSelection,
            Some(format!(
                "چند Track متنی فارسی یافت شد ({}) — از جزئیات ویدیو یکی را انتخاب کنید",
                fa_text_count
            )),
        );
    }
    if fa_image_count > 0 {
        return (
            AnalysisStatus::ImageSubtitleFound,
            Some(
                "زیرنویس تصویری فارسی (PGS/VobSub/DVB) یافت شد — استخراج متن به OCR نیاز دارد"
                    .into(),
            ),
        );
    }
    if fa_unknown_count > 0 {
        return (
            AnalysisStatus::NeedsTrackSelection,
            Some("Track فارسی با کدک ناشناخته یافت شد — از جزئیات ویدیو بررسی کنید".into()),
        );
    }

    (
        AnalysisStatus::NoPersianTrack,
        Some(format!(
            "زیرنویس داخلی هست اما Track فارسی در آن نیست ({} Track) — برای Trackهای متنی می‌توانید «استخراج آزمایشی برای بررسی» بزنید",
            streams.len()
        )),
    )
}

/// مسیر مقصد استاندارد استخراج: `<video-base>.fa.srt`
pub fn extraction_target(video_path: &Path) -> Option<PathBuf> {
    let base = video_path.file_stem()?;
    // روی ویندوز، parent یک نام فایل بدون پوشه، Some(«») است — قابل اتکا نیست
    let dir = video_path.parent().filter(|d| !d.as_os_str().is_empty())?;
    Some(dir.join(format!("{}.fa.srt", base.to_string_lossy())))
}

/// بررسی‌های اجباری قبل از استخراج — مطابق اسکریپت مرجع.
/// `Err(reason)` یعنی عملیات باید Skip شود (فایل‌های موجود هرگز بازنویسی نمی‌شوند).
pub fn check_extraction_target(
    video_path: &Path,
    sibling_subtitles: &[SubtitleFileInfo],
) -> Result<PathBuf, String> {
    let target =
        extraction_target(video_path).ok_or_else(|| "نمی‌توان مسیر مقصد را ساخت".to_string())?;

    if target.is_file() {
        // حتی اگر فایل موجود خالی/خراب باشد هم بازنویسی ممنوع است؛
        // وضعیت به‌صورت صریح گزارش می‌شود تا کاربر دستی تصمیم بگیرد.
        let bytes = std::fs::read(&target).map_err(|e| format!("خواندن فایل موجود: {e}"))?;
        let (_, text) = decode_bytes(&bytes);
        let validity = if bytes.is_empty() {
            "خالی"
        } else if !has_persian_text(&text) {
            "بدون محتوای فارسی"
        } else {
            "معتبر"
        };
        return Err(format!(
            "زیرنویس «{}» از قبل موجود است ({}) — برای جلوگیری از بازنویسی Skip شد",
            target.file_name().unwrap_or_default().to_string_lossy(),
            validity
        ));
    }

    // اگر نسخه سینک‌شده موجود است، استخراج بی‌معناست
    let base = video_path.file_stem().unwrap_or_default().to_string_lossy();
    if let Some(dir) = video_path.parent() {
        let synced = dir.join(format!("{base}.fa.synced.srt"));
        if synced.is_file() {
            return Err(format!(
                "زیرنویس سینک‌شده «{}» موجود است — نیازی به استخراج نیست",
                synced.file_name().unwrap_or_default().to_string_lossy()
            ));
        }
        // زیرنویس legacy ساده با محتوای فارسی = فارسیِ معتبر موجود (مطابق اسکریپت مرجع)
        let legacy = dir.join(format!("{base}.srt"));
        if legacy.is_file() && sibling_has_fa(&legacy) {
            return Err(format!(
                "زیرنویس «{}» با محتوای فارسی موجود است — Skip شد",
                legacy.file_name().unwrap_or_default().to_string_lossy()
            ));
        }
    }

    let _ = sibling_subtitles; // (برای سازگاری امضا؛ بررسی واقعی روی دیسک انجام می‌شود)
    Ok(target)
}

fn sibling_has_fa(path: &Path) -> bool {
    match std::fs::read(path) {
        Ok(bytes) => {
            let (_, text) = decode_bytes(&bytes);
            has_persian_text(&text)
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(index: u32, codec: &str, persian: bool) -> SubtitleStream {
        SubtitleStream {
            stream_index: index,
            codec_name: codec.into(),
            kind: crate::media::codec::classify_codec(codec),
            language: None,
            language_ietf: None,
            title: None,
            default_disposition: false,
            forced_disposition: false,
            persian_candidate: persian,
            persian_confirmed: false,
        }
    }

    fn confirmed(index: u32, codec: &str) -> SubtitleStream {
        let mut s = stream(index, codec, false);
        s.persian_confirmed = true;
        s
    }

    #[test]
    fn classifies_statuses() {
        let (status, _) = classify_analysis(&[]);
        assert_eq!(status, AnalysisStatus::NoEmbeddedSubtitle);

        let (status, _) = classify_analysis(&[stream(2, "subrip", true)]);
        assert_eq!(status, AnalysisStatus::PersianTextTrackFound);

        // یک متنی فارسی + چند Track دیگر فارسی نیست → بدون ابهام
        let (status, _) =
            classify_analysis(&[stream(2, "subrip", true), stream(3, "subrip", false)]);
        assert_eq!(status, AnalysisStatus::PersianTextTrackFound);

        // دو متنی فارسی → انتخاب کاربر
        let (status, msg) = classify_analysis(&[stream(2, "subrip", true), stream(4, "ass", true)]);
        assert_eq!(status, AnalysisStatus::NeedsTrackSelection);
        assert!(msg.unwrap().contains('2'));

        // فقط تصویری فارسی → OCR
        let (status, msg) = classify_analysis(&[stream(2, "hdmv_pgs_subtitle", true)]);
        assert_eq!(status, AnalysisStatus::ImageSubtitleFound);
        assert!(msg.unwrap().contains("OCR"));

        // فارسی با کدک ناشناخته
        let (status, _) = classify_analysis(&[stream(2, "weird", true)]);
        assert_eq!(status, AnalysisStatus::NeedsTrackSelection);

        // زیرنویس هست ولی فارسی نیست
        let (status, _) = classify_analysis(&[stream(2, "subrip", false)]);
        assert_eq!(status, AnalysisStatus::NoPersianTrack);

        // Track تأییدشده از جریان بررسی، هم‌ارز کاندید فارسی است
        let (status, _) = classify_analysis(&[confirmed(2, "subrip")]);
        assert_eq!(status, AnalysisStatus::PersianTextTrackFound);

        // دو Track تأییدشده → انتخاب کاربر
        let (status, _) = classify_analysis(&[confirmed(2, "subrip"), confirmed(3, "ass")]);
        assert_eq!(status, AnalysisStatus::NeedsTrackSelection);
    }

    #[test]
    fn target_path_uses_video_base() {
        let t = extraction_target(Path::new("D:/Series/Show.S01E01.720p.mkv")).unwrap();
        assert_eq!(t, Path::new("D:/Series/Show.S01E01.720p.fa.srt"));
        // نام فایل بدون پوشه والد — مقصد قابل ساخت نیست
        assert!(extraction_target(Path::new("just-a-name")).is_none());
    }

    #[test]
    fn overwrite_protection() {
        let dir = std::env::temp_dir().join(format!(
            "subsync-target-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("Show.S01E01.mkv");
        std::fs::write(&video, b"").unwrap();

        // بدون هیچ فایل موجود → مجاز
        let target = check_extraction_target(&video, &[]).unwrap();
        assert_eq!(target, dir.join("Show.S01E01.fa.srt"));

        // .fa.srt موجود → ممنوع حتی اگر خالی باشد
        std::fs::write(dir.join("Show.S01E01.fa.srt"), b"").unwrap();
        let err = check_extraction_target(&video, &[]).unwrap_err();
        assert!(err.contains("خالی"), "{err}");

        // .fa.synced.srt موجود → ممنوع
        std::fs::remove_file(dir.join("Show.S01E01.fa.srt")).unwrap();
        std::fs::write(dir.join("Show.S01E01.fa.synced.srt"), "x").unwrap();
        let err = check_extraction_target(&video, &[]).unwrap_err();
        assert!(err.contains("سینک"), "{err}");

        // legacy .srt با محتوای فارسی → ممنوع؛ بدون فارسی → مجاز
        std::fs::remove_file(dir.join("Show.S01E01.fa.synced.srt")).unwrap();
        std::fs::write(
            dir.join("Show.S01E01.srt"),
            "1\n00:00:01,000 --> 00:00:02,000\nenglish\n",
        )
        .unwrap();
        assert!(check_extraction_target(&video, &[]).is_ok());
        std::fs::write(
            dir.join("Show.S01E01.srt"),
            "1\n00:00:01,000 --> 00:00:02,000\nسلام\n",
        )
        .unwrap();
        let err = check_extraction_target(&video, &[]).unwrap_err();
        assert!(err.contains("فارسی"), "{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
