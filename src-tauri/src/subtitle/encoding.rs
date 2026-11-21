//! تشخیص Encoding و شمارش کاراکترهای فارسی.
//!
//! رنج‌های اصلی فارسی: U+0600–06FF، U+FB50–FDFF، U+FE70–FEFF

/// حداقل حروف فارسی برای پذیرش تبدیل cp1256 — فایل بدون فارسی نباید
/// بازنویسی شود (heuristic ضعیف‌تر از محتوای واقعی نیست).
const MIN_PERSIAN_CHARS_FOR_CONVERSION: usize = 10;

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
