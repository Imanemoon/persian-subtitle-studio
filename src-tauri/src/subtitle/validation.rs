//! اعتبارسنجی فایل زیرنویس: ساختار SRT/VTT، Encoding و محتوای فارسی.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::models::SubtitleValidation;

use super::encoding::{decode_bytes, has_persian_text, persian_char_count};

// خط تایم‌کد معتبر — ساعت اختیاری (WebVTT)، جداکنندهٔ میلی‌ثانیه , یا .
static CUE_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^\s*(\d{1,2}:)?\d{1,2}:\d{2}[.,]\d{1,3}\s*-->\s*(\d{1,2}:)?\d{1,2}:\d{2}[.,]\d{1,3}",
    )
    .unwrap()
});

/// اعتبارسنجی کامل یک فایل روی دیسک.
pub fn validate_file(path: &Path) -> SubtitleValidation {
    match std::fs::read(path) {
        Ok(bytes) => validate_bytes(&bytes),
        Err(e) => SubtitleValidation {
            valid: false,
            size_bytes: 0,
            encoding: "—".into(),
            line_count: 0,
            cue_count: 0,
            has_persian: false,
            persian_char_count: 0,
            error: Some(format!("خواندن فایل ناموفق بود: {e}")),
        },
    }
}

/// اعتبارسنجی بایت‌های خام زیرنویس.
pub fn validate_bytes(bytes: &[u8]) -> SubtitleValidation {
    let size_bytes = bytes.len() as u64;
    if size_bytes == 0 {
        return SubtitleValidation {
            valid: false,
            size_bytes: 0,
            encoding: "—".into(),
            line_count: 0,
            cue_count: 0,
            has_persian: false,
            persian_char_count: 0,
            error: Some("فایل خالی است".into()),
        };
    }

    let (encoding, text) = decode_bytes(bytes);
    let line_count = text.lines().count();
    let cue_count = CUE_LINE_RE.find_iter(&text).count();
    let persian_char_count = persian_char_count(&text);
    let has_persian = has_persian_text(&text);

    let error = if cue_count == 0 {
        Some("هیچ تایم‌کد معتبری (مثل 00:00:01,000 --> 00:00:04,000) پیدا نشد".into())
    } else {
        None
    };

    SubtitleValidation {
        valid: error.is_none(),
        size_bytes,
        encoding: encoding.to_string(),
        line_count,
        cue_count,
        has_persian,
        persian_char_count,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_srt_passes() {
        let srt = "1\n00:00:01,000 --> 00:00:04,000\nسلام دنیا\n\n2\n00:00:05,500 --> 00:00:08,000\nخط دوم\n";
        let v = validate_bytes(srt.as_bytes());
        assert!(v.valid);
        assert_eq!(v.cue_count, 2);
        assert_eq!(v.encoding, "UTF-8");
        assert!(v.has_persian);
        assert!(v.persian_char_count >= 9);
        assert!(v.error.is_none());
    }

    #[test]
    fn webvtt_style_cues_count() {
        let vtt = "WEBVTT\n\n00:01.000 --> 00:04.000\nhello\n";
        let v = validate_bytes(vtt.as_bytes());
        assert!(v.valid);
        assert_eq!(v.cue_count, 1);
        assert!(!v.has_persian);
    }

    #[test]
    fn empty_file_is_invalid() {
        let v = validate_bytes(b"");
        assert!(!v.valid);
        assert_eq!(v.error.as_deref(), Some("فایل خالی است"));
    }

    #[test]
    fn corrupt_srt_without_cues_is_invalid() {
        // متن بدون هیچ تایم‌کدی
        let v = validate_bytes("this is not a subtitle".as_bytes());
        assert!(!v.valid);
        assert_eq!(v.cue_count, 0);

        // تایم‌کد ناقص/خراب
        let v = validate_bytes("1\n00:00:01 --> \nمتن\n".as_bytes());
        assert!(!v.valid);

        // باینری تصادفی
        let v = validate_bytes(&[0x00, 0xFF, 0xC3, 0x28, 0x7F, 0x80]);
        assert!(!v.valid);
    }

    #[test]
    fn utf16_srt_is_valid() {
        let mut bytes: Vec<u8> = vec![0xFF, 0xFE];
        for unit in "1\n00:00:01,000 --> 00:00:04,000\nسلام\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let v = validate_bytes(&bytes);
        assert!(v.valid);
        assert_eq!(v.encoding, "UTF-16 LE");
        assert!(v.has_persian);
    }

    #[test]
    fn fa_tagged_but_latin_content_is_not_confirmed_persian() {
        // محتوای لاتین — has_persian=false؛ تصمیم‌گیری در لایه بالاتر
        let srt = "1\n00:00:01,000 --> 00:00:04,000\nJust latin text\n";
        let v = validate_bytes(srt.as_bytes());
        assert!(v.valid);
        assert!(!v.has_persian);
    }
}
