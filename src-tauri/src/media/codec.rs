//! دسته‌بندی کدک‌های زیرنویس به متنی / تصویری / ناشناخته.

use crate::models::SubtitleKind;

/// زیرنویس‌های متنی که FFmpeg می‌تواند به SRT تبدیل کند.
pub const TEXT_CODECS: &[&str] = &["subrip", "ass", "ssa", "webvtt", "mov_text", "text"];

/// زیرنویس‌های تصویری — استخراج متن از آن‌ها OCR می‌خواهد.
pub const IMAGE_CODECS: &[&str] = &["hdmv_pgs_subtitle", "dvd_subtitle", "xsub", "dvb_subtitle"];

pub fn classify_codec(codec_name: &str) -> SubtitleKind {
    let name = codec_name.to_lowercase();
    if TEXT_CODECS.contains(&name.as_str()) {
        SubtitleKind::Text
    } else if IMAGE_CODECS.contains(&name.as_str()) {
        SubtitleKind::Image
    } else {
        SubtitleKind::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_text_codecs() {
        for c in [
            "subrip", "ass", "ssa", "webvtt", "mov_text", "text", "SUBRIP",
        ] {
            assert_eq!(classify_codec(c), SubtitleKind::Text, "{c}");
        }
    }

    #[test]
    fn classifies_image_codecs() {
        for c in ["hdmv_pgs_subtitle", "dvd_subtitle", "xsub", "dvb_subtitle"] {
            assert_eq!(classify_codec(c), SubtitleKind::Image, "{c}");
        }
    }

    #[test]
    fn unknown_codecs_stay_unknown() {
        for c in ["foobar", "", "dvb_teletext", "arib_caption"] {
            assert_eq!(classify_codec(c), SubtitleKind::Unknown, "{c}");
        }
    }
}
