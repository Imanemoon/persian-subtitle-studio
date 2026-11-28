//! اعتبارسنجی فایل زیرنویس: ساختار SRT/VTT/ASS/SSA/MicroDVD، Encoding و
//! محتوای فارسی.
//!
//! `validate_bytes` رفتار عمومی قبلی (تشخیص تایم‌کد) را حفظ می‌کند؛
//! `validate_bytes_for_extension` برای موتور آرشیو از ساختار مخصوص هر فرمت
//! استفاده می‌کند تا فایل ASS/SSA/SUB به‌اشتباه SRT فرض نشود.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::models::SubtitleValidation;

use super::encoding::{decode_bytes, has_persian_text, persian_char_count};

// خط تایم‌کد معتبر — ساعت اختیاری (WebVTT)، جداکنندهٔ میلی‌ثانیه , یا .
pub(crate) static CUE_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^\s*(\d{1,2}:)?\d{1,2}:\d{2}[.,]\d{1,3}\s*-->\s*(\d{1,2}:)?\d{1,2}:\d{2}[.,]\d{1,3}",
    )
    .unwrap()
});

static ASS_SCRIPT_INFO_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^\s*\[script info\]\s*$").unwrap());
static ASS_EVENTS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^\s*\[events\]\s*$").unwrap());
static ASS_EVENTS_FORMAT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^\s*format\s*:\s*(.+)$").unwrap());
static ASS_DIALOGUE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?im)^\s*dialogue\s*:\s*(.+)$").unwrap());
static ASS_TIME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+:\d{2}:\d{2}[.,]\d{1,3}").unwrap());

static MICRODVD_LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*\{(\d+)\}\{(\d+)\}(.*)$").unwrap());

/// اعتبارسنجی کامل یک فایل روی دیسک با ساختار متناسب پسوند.
pub fn validate_file(path: &Path) -> SubtitleValidation {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match std::fs::read(path) {
        Ok(bytes) => validate_bytes_for_extension(&bytes, &ext),
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

/// اعتبارسنجی عمومی بایت‌های خام زیرنویس (سازگار با رفتار قبلی).
pub fn validate_bytes(bytes: &[u8]) -> SubtitleValidation {
    if bytes.is_empty() {
        return empty_validation();
    }
    let (encoding, text) = match decode_checked(bytes) {
        Ok(v) => v,
        Err(error) => return decode_error_validation(bytes, error),
    };
    let cue_count = CUE_LINE_RE.find_iter(&text).count();
    let error = if cue_count == 0 {
        Some("هیچ تایم‌کد معتبری (مثل 00:00:01,000 --> 00:00:04,000) پیدا نشد".into())
    } else {
        None
    };
    build_validation(bytes, &text, encoding, cue_count, error)
}

/// اعتبارسنجی آگاه از فرمت — برای موتور آرشیو (SRT/VTT/ASS/SSA/MicroDVD).
pub fn validate_bytes_for_extension(bytes: &[u8], ext: &str) -> SubtitleValidation {
    match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "srt" => validate_srt_like(bytes, false),
        "vtt" => validate_srt_like(bytes, true),
        "ass" | "ssa" => validate_ass(bytes),
        "sub" => validate_microdvd(bytes),
        // پسوند ناشناخته/غیرزیرنویس: رفتار عمومی قبلی
        _ => validate_bytes(bytes),
    }
}

fn validate_srt_like(bytes: &[u8], require_webvtt: bool) -> SubtitleValidation {
    if bytes.is_empty() {
        return empty_validation();
    }
    let (encoding, text) = match decode_checked(bytes) {
        Ok(v) => v,
        Err(error) => return decode_error_validation(bytes, error),
    };

    let cue_count = CUE_LINE_RE.find_iter(&text).count();
    let error = if require_webvtt {
        let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        if !first.trim().eq_ignore_ascii_case("WEBVTT") {
            Some("سربرگ WEBVTT در فایل VTT پیدا نشد".into())
        } else if cue_count == 0 {
            Some("هیچ تایم‌کد معتبری در فایل VTT پیدا نشد".into())
        } else if !has_cue_text(&text) {
            Some("هیچ متن دیالوگی بعد از تایم‌کد پیدا نشد".into())
        } else {
            None
        }
    } else if cue_count == 0 {
        Some("هیچ تایم‌کد معتبری (مثل 00:00:01,000 --> 00:00:04,000) پیدا نشد".into())
    } else if !has_cue_text(&text) {
        Some("هیچ متن دیالوگی بعد از تایم‌کد پیدا نشد".into())
    } else {
        None
    };

    build_validation(bytes, &text, encoding, cue_count, error)
}

fn validate_ass(bytes: &[u8]) -> SubtitleValidation {
    if bytes.is_empty() {
        return empty_validation();
    }
    let (encoding, text) = match decode_checked(bytes) {
        Ok(v) => v,
        Err(error) => return decode_error_validation(bytes, error),
    };

    let has_script_info = ASS_SCRIPT_INFO_RE.is_match(&text);
    let has_events = ASS_EVENTS_RE.is_match(&text);
    let has_events_format = ASS_EVENTS_FORMAT_RE.captures_iter(&text).any(|c| {
        let lower = c[1].to_ascii_lowercase();
        lower.contains("start") && lower.contains("end")
    });
    let cue_count = ASS_DIALOGUE_RE
        .captures_iter(&text)
        .filter(|c| ASS_TIME_RE.find_iter(&c[1]).count() >= 2)
        .count();

    let error = if !has_script_info {
        Some("بخش [Script Info] در فایل ASS/SSA پیدا نشد".into())
    } else if !has_events {
        Some("بخش [Events] در فایل ASS/SSA پیدا نشد".into())
    } else if !has_events_format {
        Some("خط Format با فیلدهای Start/End در بخش [Events] پیدا نشد".into())
    } else if cue_count == 0 {
        Some("هیچ خط Dialogue با تایم‌کد معتبر Start/End پیدا نشد".into())
    } else {
        None
    };

    build_validation(bytes, &text, encoding, cue_count, error)
}

fn validate_microdvd(bytes: &[u8]) -> SubtitleValidation {
    if bytes.is_empty() {
        return empty_validation();
    }
    let (encoding, text) = match decode_checked(bytes) {
        Ok(v) => v,
        Err(error) => return decode_error_validation(bytes, error),
    };

    let mut cue_count = 0usize;
    let mut has_text = false;
    for c in MICRODVD_LINE_RE.captures_iter(&text) {
        cue_count += 1;
        let payload = c[3].trim();
        // خط اول MicroDVD می‌تواند فقط FPS باشد: {1}{1}23.976
        let is_fps_only =
            !payload.is_empty() && payload.chars().all(|ch| ch.is_ascii_digit() || ch == '.');
        if !payload.is_empty() && !is_fps_only {
            has_text = true;
        }
    }

    let error = if cue_count == 0 {
        Some("هیچ خط MicroDVD با ساختار {شروع}{پایان} پیدا نشد".into())
    } else if !has_text {
        Some("هیچ دیالوگ متنی در فایل MicroDVD پیدا نشد".into())
    } else {
        None
    };
    build_validation(bytes, &text, encoding, cue_count, error)
}

/// متن بعد از یک خط تایم‌کد وجود دارد؟ (جلوگیری از فایل فقط-تایم‌کد)
fn has_cue_text(text: &str) -> bool {
    let mut after_cue = false;
    for line in text.lines() {
        if CUE_LINE_RE.is_match(line) {
            after_cue = true;
            continue;
        }
        if after_cue {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.chars().all(|c| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------
// آمار per-cue — مادهٔ خام امتیازدهی Track (نه اعتبارسنجی)
// ---------------------------------------------------------------------

/// آمار قابل‌اندازه‌گیری cueهای یک زیرنویس متنی (SRT/VTT) — مادهٔ خام
/// امتیازدهی Track در `scoring.rs`. هیچ تصمیمی اینجا گرفته نمی‌شود.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CueStats {
    pub cue_count: usize,
    /// cue بدون هیچ متن دیالوگی
    pub empty_cue_count: usize,
    pub persian_chars: usize,
    /// کاراکترهای غیرفاصلهٔ کل متن cueها
    pub text_chars: usize,
    /// نسبت cueهای با متن تکراری (نشانِ تبلیغات/Track کپی‌شده)
    pub duplicate_ratio: f64,
    /// نسبت cue با متن خیلی کوتاه — نشانهٔ تبلیغ/لینک
    pub short_ratio: f64,
    /// بازهٔ پوشش زمانی برای سنجش coverage نسبت به طول ویدیو
    pub first_start_ms: Option<u64>,
    pub last_end_ms: Option<u64>,
}

static CUE_TIME_PARSE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{1,2}):(\d{2}):(\d{2})[.,](\d{1,3})").unwrap());

fn parse_cue_ms(text: &str) -> Option<u64> {
    let c = CUE_TIME_PARSE_RE.captures(text)?;
    let h: u64 = c[1].parse().ok()?;
    let m: u64 = c[2].parse().ok()?;
    let s: u64 = c[3].parse().ok()?;
    let ms: u64 = c[4].parse().ok()?;
    Some(((h * 60 + m) * 60 + s) * 1000 + ms)
}

/// آمار cueهای متن SRT/VTT — block-based: خط تایم‌کد → متن تا خط خالی.
/// متن‌های تکراری/کوتاه برای تشخیص Track تبلیغاتی شمرده می‌شوند.
pub(crate) fn analyze_srt_like(text: &str) -> CueStats {
    use std::collections::HashMap;

    let mut stats = CueStats::default();
    let mut text_counts: HashMap<String, usize> = HashMap::new();
    let mut in_cue = false;
    let mut seen_cue_text = false;
    let mut short_texts = 0usize;

    let close_cue = |stats: &mut CueStats, seen: bool| {
        if !seen {
            stats.empty_cue_count += 1;
        }
    };

    for line in text.lines() {
        let trimmed = line.trim();
        if CUE_LINE_RE.is_match(trimmed) {
            if in_cue {
                close_cue(&mut stats, seen_cue_text);
            }
            stats.cue_count += 1;
            if stats.first_start_ms.is_none() {
                stats.first_start_ms = parse_cue_ms(trimmed);
            }
            if let Some(end) = parse_cue_ms(trimmed.split("-->").nth(1).unwrap_or("")) {
                stats.last_end_ms = Some(stats.last_end_ms.unwrap_or(0).max(end));
            }
            in_cue = true;
            seen_cue_text = false;
            continue;
        }
        if !in_cue {
            continue;
        }
        if trimmed.is_empty() {
            close_cue(&mut stats, seen_cue_text);
            in_cue = false;
            continue;
        }
        if trimmed.chars().all(|c| c.is_ascii_digit()) {
            // شمارهٔ cue بعدی — نه متن دیالوگ
            continue;
        }
        seen_cue_text = true;
        for c in trimmed.chars() {
            if c.is_whitespace() {
                continue;
            }
            stats.text_chars += 1;
            if crate::subtitle::encoding::is_persian_char(c) {
                stats.persian_chars += 1;
            }
        }
        let normalized: String = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
        *text_counts.entry(normalized).or_insert(0) += 1;
        let non_ws = trimmed.chars().filter(|c| !c.is_whitespace()).count();
        if non_ws > 0 && non_ws < 4 {
            short_texts += 1;
        }
    }
    if in_cue {
        close_cue(&mut stats, seen_cue_text);
    }

    stats.duplicate_ratio = if stats.cue_count == 0 {
        0.0
    } else {
        let extra: usize = text_counts
            .values()
            .filter(|c| **c > 1)
            .map(|c| c - 1)
            .sum();
        extra as f64 / stats.cue_count as f64
    };
    stats.short_ratio = if stats.cue_count == 0 {
        0.0
    } else {
        short_texts as f64 / stats.cue_count as f64
    };
    stats
}

/// decode امن برای تحلیل محتوا — همان قواعد decode_checked (رد باینری/encoding نامشخص).
pub(crate) fn decode_for_analysis(bytes: &[u8]) -> Result<String, String> {
    decode_checked(bytes).map(|(_, text)| text)
}

/// Decode با رد صریح باینری/Encoding ناشناخته.
///
/// فایل‌های UTF-16 به‌خاطر BOM دارای بایت NUL هستند و مجازند؛ NUL در فایل
/// غیر UTF-16 یعنی باینری (مثل VobSub) و نباید به‌عنوان متن پذیرفته شود.
fn decode_checked(bytes: &[u8]) -> Result<(&'static str, String), String> {
    let is_utf16 = bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]);
    if !is_utf16 && bytes.contains(&0) {
        return Err("فایل باینری است (بایت NUL دارد) — زیرنویس متنی معتبر نیست".into());
    }
    if bytes.starts_with(&[0x00, 0x00, 0x01, 0xBA]) {
        return Err("فایل SUB باینری (VobSub) است — بدون فایل idx جفت‌شده قابل استفاده نیست".into());
    }
    let (encoding, text) = decode_bytes(bytes);
    if encoding.starts_with("نامشخص") {
        return Err("Encoding فایل پشتیبانی نمی‌شود (فقط UTF-8 یا UTF-16)".into());
    }
    Ok((encoding, text))
}

fn empty_validation() -> SubtitleValidation {
    SubtitleValidation {
        valid: false,
        size_bytes: 0,
        encoding: "—".into(),
        line_count: 0,
        cue_count: 0,
        has_persian: false,
        persian_char_count: 0,
        error: Some("فایل خالی است".into()),
    }
}

fn decode_error_validation(bytes: &[u8], error: String) -> SubtitleValidation {
    SubtitleValidation {
        valid: false,
        size_bytes: bytes.len() as u64,
        encoding: "نامشخص".into(),
        line_count: 0,
        cue_count: 0,
        has_persian: false,
        persian_char_count: 0,
        error: Some(error),
    }
}

fn build_validation(
    bytes: &[u8],
    text: &str,
    encoding: &str,
    cue_count: usize,
    error: Option<String>,
) -> SubtitleValidation {
    let persian_char_count = persian_char_count(text);
    SubtitleValidation {
        valid: error.is_none(),
        size_bytes: bytes.len() as u64,
        encoding: encoding.to_string(),
        line_count: text.lines().count(),
        cue_count,
        has_persian: has_persian_text(text),
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

    #[test]
    fn format_aware_srt_vtt_and_binary_sub() {
        let srt = "1\n00:00:01,000 --> 00:00:04,000\nسلام\n";
        assert!(validate_bytes_for_extension(srt.as_bytes(), "srt").valid);

        let vtt = "WEBVTT\n\n00:01.000 --> 00:04.000\nسلام\n";
        assert!(validate_bytes_for_extension(vtt.as_bytes(), "vtt").valid);

        // VTT بدون سربرگ رد می‌شود، هرچند تایم‌کد دارد
        let no_header = "00:01.000 --> 00:04.000\nسلام\n";
        assert!(!validate_bytes_for_extension(no_header.as_bytes(), "vtt").valid);

        // فقط تایم‌کد بدون متن
        let no_text = "1\n00:00:01,000 --> 00:00:04,000\n";
        assert!(!validate_bytes_for_extension(no_text.as_bytes(), "srt").valid);

        // VobSub باینری هرگز به‌عنوان متن پذیرفته نمی‌شود
        let binary_sub = [0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x00, 0x00];
        let v = validate_bytes_for_extension(&binary_sub, "sub");
        assert!(!v.valid);
        assert!(v.error.as_deref().unwrap_or_default().contains("باینری"));
    }

    #[test]
    fn ass_and_ssa_are_validated_by_their_own_structure() {
        let ass = "[Script Info]\nTitle: Test\nScriptType: v4.00+\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:04.00,Default,,0,0,0,,سلام دنیا\n";
        let v = validate_bytes_for_extension(ass.as_bytes(), "ass");
        assert!(v.valid, "{:?}", v.error);
        assert_eq!(v.cue_count, 1);
        assert!(v.has_persian);

        let ssa = "[Script Info]\nTitle: Test\n\n[Events]\nFormat: Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: Marked=0,0:00:01.00,0:00:04.00,Default,,0,0,0,,سلام\n";
        let v = validate_bytes_for_extension(ssa.as_bytes(), "ssa");
        assert!(v.valid, "{:?}", v.error);

        // ASS بدون بخش Events نباید معتبر باشد
        let broken = "[Script Info]\nTitle: Test\n";
        assert!(!validate_bytes_for_extension(broken.as_bytes(), "ass").valid);

        // ASS معتبر اما لاتین — ساختار درست، فارسی تأیید نشده
        let latin = "[Script Info]\nTitle: Test\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:04.00,Default,,0,0,0,,hello\n";
        let v = validate_bytes_for_extension(latin.as_bytes(), "ass");
        assert!(v.valid);
        assert!(!v.has_persian);
    }

    #[test]
    fn microdvd_text_is_valid_but_binary_is_not() {
        let microdvd = "{1}{1}23.976\n{100}{200}سلام دنیا\n{250}{300}خط دوم\n";
        let v = validate_bytes_for_extension(microdvd.as_bytes(), "sub");
        assert!(v.valid, "{:?}", v.error);
        assert_eq!(v.cue_count, 3);
        assert!(v.has_persian);

        // فقط FPS — دیالوگ متنی ندارد
        let fps_only = "{1}{1}23.976\n";
        assert!(!validate_bytes_for_extension(fps_only.as_bytes(), "sub").valid);
    }

    #[test]
    fn unsupported_encoding_is_rejected_not_lossily_decoded() {
        // بایت‌های نامعتبر UTF-8 (شبیه Windows-1256) نباید پذیرفته شوند
        let bytes = [0xC7, 0xE1, 0xE1, 0xE5, 0x0A];
        let v = validate_bytes_for_extension(&bytes, "srt");
        assert!(!v.valid);
        assert!(v.error.as_deref().unwrap_or_default().contains("Encoding"));
    }
}
