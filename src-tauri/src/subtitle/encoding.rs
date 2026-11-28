//! تشخیص Encoding و شمارش کاراکترهای فارسی.
//!
//! رنج‌های اصلی فارسی: U+0600–06FF، U+FB50–FDFF، U+FE70–FEFF

use serde::{Deserialize, Serialize};

/// حداقل حروف فارسی برای پذیرش تبدیل cp1256 — فایل بدون فارسی نباید
/// بازنویسی شود (heuristic ضعیف‌تر از محتوای واقعی نیست).
pub(crate) const MIN_PERSIAN_CHARS_FOR_CONVERSION: usize = 10;

/// بایت‌ها را با تشخیص BOM/Encoding به متن تبدیل می‌کند.
/// خروجی: (نام Encoding، متن)
pub fn decode_bytes(bytes: &[u8]) -> (&'static str, String) {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (
            "UTF-8 BOM",
            String::from_utf8_lossy(&bytes[3..]).into_owned(),
        );
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return ("UTF-16 LE", decode_utf16(&bytes[2..], true));
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return ("UTF-16 BE", decode_utf16(&bytes[2..], false));
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => ("UTF-8", s.to_string()),
        Err(_) => (
            "نامشخص (UTF-8 معتبر نیست)",
            String::from_utf8_lossy(bytes).into_owned(),
        ),
    }
}

/// تبدیل بایت‌های cp1256 به متن — بدون هیچ heuristic؛ فراخواننده تصمیم می‌گیرد.
pub const CP1256_HIGH: [char; 128] = [
    '€', 'پ', '‚', 'ƒ', '„', '…', '†', '‡', // 0x80
    'ˆ', '‰', 'ٹ', '‹', 'Œ', 'چ', 'ژ', 'ڈ', // 0x88
    'گ', '‘', '’', '“', '”', '•', '–', '—', // 0x90
    'ک', '™', 'ڑ', '›', 'œ', '\u{200c}', '\u{200d}', 'ں', // 0x98
    '\u{a0}', '،', '¢', '£', '¤', '¥', '¦', '§', // 0xA0
    '¨', '©', 'ھ', '«', '¬', '\u{ad}', '®', '¯', // 0xA8
    '°', '±', '²', '³', '´', 'µ', '¶', '·', // 0xB0
    '¸', '¹', '؛', '»', '¼', '½', '¾', '؟', // 0xB8
    'ہ', 'ء', 'آ', 'أ', 'ؤ', 'إ', 'ئ', 'ا', // 0xC0
    'ب', 'ة', 'ت', 'ث', 'ج', 'ح', 'خ', 'د', // 0xC8
    'ذ', 'ر', 'ز', 'س', 'ش', 'ص', 'ض', '×', // 0xD0
    'ط', 'ظ', 'ع', 'غ', 'ـ', 'ف', 'ق', 'ك', // 0xD8
    'à', 'ل', 'â', 'م', 'ن', 'ه', 'و', 'ç', // 0xE0
    'è', 'é', 'ê', 'ë', 'ى', 'ي', 'î', 'ï', // 0xE8
    'ً', 'ٌ', 'ٍ', 'َ', 'ô', 'ُ', 'ِ', '÷', // 0xF0
    'ّ', 'ù', 'ْ', 'û', 'ü', '\u{200e}', '\u{200f}', 'ے', // 0xF8
];

pub fn decode_windows1256(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b < 0x80 {
            out.push(b as char);
        } else {
            out.push(CP1256_HIGH[(b - 0x80) as usize]);
        }
    }
    out
}

/// آیا این بایت‌ها با cp1256 قابل خواندن‌اند و حرف فارسی کافی دارند؟
/// فقط برای زیرنویس‌های دانلودی صدا زده می‌شود؛ فایل‌های محلی کاربر هرگز
/// بازنویسی نمی‌شوند. UTF-8 سالم → None (نیازی به تبدیل نیست).
pub fn convert_windows1256_if_needed(bytes: &[u8]) -> Option<String> {
    if std::str::from_utf8(bytes).is_ok() {
        return None;
    }
    let text = decode_windows1256(bytes);
    let persian = text
        .chars()
        .filter(|c| ('\u{0600}'..='\u{06FF}').contains(c))
        .count();
    if persian >= MIN_PERSIAN_CHARS_FOR_CONVERSION {
        Some(text)
    } else {
        None
    }
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes(*pair)
            } else {
                u16::from_be_bytes(*pair)
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

pub fn is_persian_char(c: char) -> bool {
    let u = c as u32;
    (0x0600..=0x06FF).contains(&u)
        || (0xFB50..=0xFDFF).contains(&u)
        || (0xFE70..=0xFEFF).contains(&u)
}

/// تعداد کاراکترهای فارسی متن.
pub fn persian_char_count(text: &str) -> usize {
    text.chars().filter(|c| is_persian_char(*c)).count()
}

pub fn has_persian_text(text: &str) -> bool {
    persian_char_count(text) > 0
}

// ---------------------------------------------------------------------
// تحلیل و تبدیل encoding فایل‌های زیرنویس لوکال
// ---------------------------------------------------------------------

/// نوع encoding تشخیص‌داده‌شده — wire: camelCase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubtitleEncodingKind {
    /// UTF-8 بدون BOM
    Utf8,
    /// UTF-8 با BOM
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// Windows-1256 با شواهد فارسی کافی — تبدیل امن
    Windows1256,
    /// نامطمئن/پشتیبانی‌نشده — هرگز تبدیل نمی‌شود
    Unsupported,
}

/// گزارش تحلیل encoding یک فایل زیرنویس — تشخیص قابل‌اعتماد از حدس جدا است.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleEncodingReport {
    pub kind: SubtitleEncodingKind,
    /// برچسب فارسی برای نمایش
    pub label: String,
    /// آیا تشخیص قابل‌اعتماد است؟ (false = بایت‌های مبهم — هرگز تبدیل نمی‌شود)
    pub confident: bool,
    /// آیا تبدیل امن به UTF-8 ممکن است؟ (Utf8/Utf8Bom: false — نیازی نیست)
    pub convertible: bool,
    pub persian_chars: usize,
    /// دلیل عدم قطعیت/عدم پشتیبانی — برای نمایش صادقانه
    pub detail: Option<String>,
}

/// تحلیل بایت‌های فایل زیرنویس — بدون هیچ تبدیل یا نوشتن.
///
/// قواعد تشخیص (هم‌راستا با convert_windows1256_if_needed):
/// - BOM قطعی است (UTF-8 BOM / UTF-16 LE/BE).
/// - UTF-8 strict معتبر → قطعی (حتی بدون فارسی — تشخیص encoding است نه زبان).
/// - در غیر این صورت فقط یک مسیر: decode_windows1256 با گیت حداقل
///   ۱۰ حرف فارسی (همان گیت مسیر دانلود) — زیر گیت = نامطمئن و
///   پشتیبانی‌نشده؛ بایت‌های دلخواه هرگز به فارسی تفسیر نمی‌شوند.
pub fn analyze_subtitle_bytes(bytes: &[u8]) -> SubtitleEncodingReport {
    let build = |kind: SubtitleEncodingKind,
                 label: &str,
                 confident: bool,
                 convertible: bool,
                 persian: usize,
                 detail: Option<&str>| SubtitleEncodingReport {
        kind,
        label: label.into(),
        confident,
        convertible,
        persian_chars: persian,
        detail: detail.map(|d| d.into()),
    };
    if bytes.is_empty() {
        return build(
            SubtitleEncodingKind::Unsupported,
            "فایل خالی است",
            false,
            false,
            0,
            Some("فایل خالی است — تحلیل encoding ممکن نیست."),
        );
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let text = decode_bytes(bytes).1;
        return build(
            SubtitleEncodingKind::Utf8Bom,
            "UTF-8 با BOM",
            true,
            false,
            persian_char_count(&text),
            None,
        );
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let text = decode_utf16(&bytes[2..], true);
        return build(
            SubtitleEncodingKind::Utf16Le,
            "UTF-16 LE",
            true,
            true,
            persian_char_count(&text),
            None,
        );
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let text = decode_utf16(&bytes[2..], false);
        return build(
            SubtitleEncodingKind::Utf16Be,
            "UTF-16 BE",
            true,
            true,
            persian_char_count(&text),
            None,
        );
    }
    if std::str::from_utf8(bytes).is_ok() {
        let text = decode_bytes(bytes).1;
        return build(
            SubtitleEncodingKind::Utf8,
            "UTF-8",
            true,
            false,
            persian_char_count(&text),
            None,
        );
    }
    // فقط یک گسست باقی می‌ماند: Windows-1256 با گیت فارسی مسیر دانلود
    let decoded = decode_windows1256(bytes);
    let persian = persian_char_count(&decoded);
    if persian >= MIN_PERSIAN_CHARS_FOR_CONVERSION {
        build(
            SubtitleEncodingKind::Windows1256,
            "Windows-1256 (ANSI فارسی)",
            true,
            true,
            persian,
            None,
        )
    } else {
        build(
            SubtitleEncodingKind::Unsupported,
            "نامشخص — پشتیبانی نمی‌شود",
            false,
            false,
            persian,
            Some(
                "بایت‌ها نه UTF-8 معتبرند و تفسیر Windows-1256 هم متن فارسی معناداری نمی‌دهد — تبدیل انجام نمی‌شود تا متن سالم خراب نشود.",
            ),
        )
    }
}

/// تبدیل بایت‌های تشخیص‌داده‌شده به متن UTF-8 — بدون BOM (هم‌راستا با
/// `--output-encoding utf-8` مسیر سینک). ساختار cue/زمان‌ها بایت‌به‌بایت
/// حفظ می‌شود چون فقط re-encode است، نه reparse.
pub fn convert_to_utf8_text(bytes: &[u8], kind: SubtitleEncodingKind) -> Result<String, String> {
    match kind {
        SubtitleEncodingKind::Utf16Le | SubtitleEncodingKind::Utf16Be => Ok(decode_utf16(
            &bytes[2..],
            kind == SubtitleEncodingKind::Utf16Le,
        )),
        SubtitleEncodingKind::Windows1256 => Ok(decode_windows1256(bytes)),
        SubtitleEncodingKind::Utf8 | SubtitleEncodingKind::Utf8Bom => {
            // UTF-8 بدون BOM می‌نویسیم — هم‌راستا با خروجی سینک
            Ok(decode_bytes(bytes).1)
        }
        SubtitleEncodingKind::Unsupported => Err(
            "این encoding تشخیص قابل‌اعتماد ندارد و تبدیل نمی‌شود — متن سالم ممکن است خراب شود."
                .into(),
        ),
    }
}

#[cfg(test)]
mod local_encoding_tests {
    use super::*;

    /// cp1256 بایت‌های «سلام دنیا» — با جدول CP1256_HIGH سازگار
    fn cp1256_persian_bytes() -> Vec<u8> {
        let text = convert_windows1256_if_needed(&[0xC7, 0xE1, 0xC7, 0xE5]).map(|_| ());
        let _ = text;
        // مستقیم: چند بایت cp1256 که به حروف فارسی می‌روند (س/ل/ا/م)
        vec![0xD3, 0xE1, 0xC7, 0xE3] // س ل ا م
    }

    #[test]
    fn utf8_without_bom_is_confident_not_convertible() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"1\r\n00:00:01,000 --> 00:00:04,000\r\n");
        bytes.extend_from_slice("سلام".as_bytes());
        bytes.extend_from_slice(b"\r\n");
        let r = analyze_subtitle_bytes(&bytes);
        assert_eq!(r.kind, SubtitleEncodingKind::Utf8);
        assert!(r.confident);
        assert!(!r.convertible, "UTF-8 نیازی به تبدیل ندارد");
        assert!(r.persian_chars >= 4);
    }

    #[test]
    fn utf8_bom_and_utf16_variants_are_confident() {
        let mut bom_bytes = vec![0xEF, 0xBB, 0xBF];
        bom_bytes.extend_from_slice("سلام".as_bytes());
        let bom = analyze_subtitle_bytes(&bom_bytes);
        assert_eq!(bom.kind, SubtitleEncodingKind::Utf8Bom);
        assert!(bom.confident);

        let mut le = vec![0xFF, 0xFE];
        le.extend("سلام".encode_utf16().flat_map(|u| u.to_le_bytes()));
        let r = analyze_subtitle_bytes(&le);
        assert_eq!(r.kind, SubtitleEncodingKind::Utf16Le);
        assert!(r.confident && r.convertible);
        assert!(r.persian_chars >= 4);

        let mut be = vec![0xFE, 0xFF];
        be.extend("سلام".encode_utf16().flat_map(|u| u.to_be_bytes()));
        let r = analyze_subtitle_bytes(&be);
        assert_eq!(r.kind, SubtitleEncodingKind::Utf16Be);
        assert!(r.confident && r.convertible);
    }

    #[test]
    fn cp1256_with_enough_persian_is_confident_and_convertible() {
        // س ل ا م + د ن ی ا — هشت حرف؛ با تکرار جمله به گیت ۱۰ می‌رسیم
        let mut bytes = cp1256_persian_bytes();
        bytes.extend_from_slice(&[0x20, 0xC7, 0xE1, 0xC7, 0xE5, 0x20, 0xC7, 0xE1, 0xC7, 0xE5]);
        let r = analyze_subtitle_bytes(&bytes);
        assert_eq!(r.kind, SubtitleEncodingKind::Windows1256);
        assert!(r.confident && r.convertible);
        assert!(r.persian_chars >= 10);
        // تبدیل باید همان متن فارسی را بدهد
        let text = convert_to_utf8_text(&bytes, r.kind).unwrap();
        assert!(text.contains("سلام"));
    }

    #[test]
    fn ambiguous_bytes_are_unsupported_not_guessed() {
        // بایت‌های cp1256 سازگار با کمتر از ۱۰ حرف فارسی — نامطمئن
        let bytes = cp1256_persian_bytes(); // فقط ۴ حرف
        let r = analyze_subtitle_bytes(&bytes);
        assert_eq!(r.kind, SubtitleEncodingKind::Unsupported);
        assert!(!r.confident && !r.convertible);
        assert!(r.detail.is_some());
        // تبدیل هم رد می‌شود — بایت دلخواه به فارسی تفسیر نمی‌شود
        assert!(convert_to_utf8_text(&bytes, r.kind).is_err());
    }

    #[test]
    fn empty_bytes_are_unsupported() {
        let r = analyze_subtitle_bytes(b"");
        assert_eq!(r.kind, SubtitleEncodingKind::Unsupported);
    }

    #[test]
    fn utf16_conversion_preserves_persian_and_structure() {
        let srt = "1
00:00:01,000 --> 00:00:04,000
سلام دنیا
";
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(srt.encode_utf16().flat_map(|u| u.to_le_bytes()));
        let text = convert_to_utf8_text(&bytes, SubtitleEncodingKind::Utf16Le).unwrap();
        assert!(text.contains("00:00:01,000 --> 00:00:04,000"), "زمان‌ها حفظ");
        assert!(text.contains("سلام دنیا"), "فارسی حفظ");
        // ساختار با validator فعلی سالم می‌ماند
        let v = crate::subtitle::validation::validate_bytes_for_extension(text.as_bytes(), "srt");
        assert!(v.valid && v.has_persian);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8() {
        let (enc, text) = decode_bytes("سلام World".as_bytes());
        assert_eq!(enc, "UTF-8");
        assert_eq!(text, "سلام World");
    }

    #[test]
    fn decodes_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("۱۲۳".as_bytes());
        let (enc, text) = decode_bytes(&bytes);
        assert_eq!(enc, "UTF-8 BOM");
        assert_eq!(text, "۱۲۳");
    }

    #[test]
    fn decodes_utf16_le_and_be() {
        let text = "سلام 123";
        let mut le = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            le.extend_from_slice(&unit.to_le_bytes());
        }
        let (enc, decoded) = decode_bytes(&le);
        assert_eq!(enc, "UTF-16 LE");
        assert_eq!(decoded, text);

        let mut be = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            be.extend_from_slice(&unit.to_be_bytes());
        }
        let (enc, decoded) = decode_bytes(&be);
        assert_eq!(enc, "UTF-16 BE");
        assert_eq!(decoded, text);
    }

    #[test]
    fn counts_persian_chars() {
        assert_eq!(persian_char_count("سلام"), 4);
        assert_eq!(persian_char_count("Hello"), 0);
        // ی عربی (U+064A) و ی فارسی (U+06CC) هر دو در رنج 0600–06FF
        assert_eq!(persian_char_count("می‌خواهم"), 7); // نیم‌فاصله U+200C شمرده نمی‌شود
        assert!(has_persian_text("abc سلام def"));
        assert!(!has_persian_text("only latin 123"));
        // ناحیه presentation forms (FB50–FDFF / FE70–FEFF)
        assert!(is_persian_char('\u{FB8A}')); // ۊ
        assert!(is_persian_char('\u{FEA9}')); // د presentation
        assert!(!is_persian_char('a'));
        assert!(!is_persian_char('\u{200C}'));
    }

    #[test]
    fn invalid_utf8_is_reported_lossy() {
        let (enc, _text) = decode_bytes(&[0xC3, 0x28, 0x00]);
        assert_eq!(enc, "نامشخص (UTF-8 معتبر نیست)");
    }
}
