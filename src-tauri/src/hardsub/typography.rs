//! تایپوگرافی هاردساب — ساخت امن فیلتر `subtitles` برای libass.
//!
//! قراردادهای امنیتی:
//! - متن زیرنویس هرگز لمس نمی‌شود (بدون معکوس‌سازی دستی RTL) — bidi/shaping
//!   خود libass (HarfBuzz+FriBidi داخل ffmpeg) انجام می‌دهد.
//! - مقادیر force_style در save اعتبارسنجی شده‌اند (`settings::validate_
//!   hardsub_typography`): نام فونت بدون `' " , : ; \ / % ` = ` و کاراکتر
//!   کنترلی، رنگ‌ها `&H[AA]BBGGRR` hex، محدوده‌ها clamp — پس اینجا فقط
//!   مونتاژ رشته است، بدون هیچ escape پیچیده‌ای که بشکند.
//! - `fontsdir` مسیر مطلق است و ممکن است `:` داشته باشد → داخل کوتیشن‌های
//!   فیلتر ffmpeg می‌نشیند (value quoting) با جداکننده‌های forward-slash
//!   (Windows API هر دو را می‌پذیرد) — هیچ مسیری بدون کوتیشن پاس نمی‌شود.
//! - `enabled = false` → رشتهٔ دقیقاً فعلی `subtitles=sub.srt` (بدون تغییر رفتار).

use crate::settings::HardsubTypography;

/// آیا رنگ با فرمت ASS `&H[AA]BBGGRR` (۶ یا ۸ رقم hex) سازگار است؟
pub fn is_valid_color(color: &str) -> bool {
    let Some(hex) = color
        .strip_prefix("&H")
        .or_else(|| color.strip_prefix("&h"))
    else {
        return false;
    };
    (6..=8).contains(&hex.len()) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

/// مسیر fontsdir امن برای فیلتر — جداکننده‌ها به `/` و کوتیشن‌گذاری کامل.
/// هیچ `:` خامی بیرون کوتیشن نمی‌ماند؛ `'` داخل مسیر ناممکن است چون
/// font_path از file-picker واقعی می‌آید و در save با is_file بررسی شده —
/// لایهٔ دفاعی: هر `'` به‌هرحال حذف می‌شود.
fn fontsdir_value(path: &str) -> String {
    let normalized = path.replace('\\', "/").replace('\'', "");
    format!("'{}'", normalized)
}

/// رشتهٔ force_style — فقط وقتی تایپوگرافی فعال باشد؛ ترتیب فیلدها
/// قرارداد ASS است (BGR برای رنگ‌ها).
pub fn build_force_style(t: &HardsubTypography) -> String {
    let mut fields: Vec<String> = Vec::new();
    if !t.font_name.is_empty() {
        fields.push(format!("FontName={}", t.font_name));
    }
    fields.push(format!("Fontsize={}", t.font_size.clamp(10, 96)));
    fields.push(format!("PrimaryColour={}", t.primary_color));
    fields.push(format!("OutlineColour={}", t.outline_color));
    fields.push(format!("Outline={}", t.outline.clamp(0, 10)));
    fields.push(format!("Shadow={}", t.shadow.clamp(0, 10)));
    fields.push(format!("MarginV={}", t.margin_v.clamp(0, 400)));
    fields.join(",")
}

/// فیلتر کامل `-vf` — disabled = رشتهٔ فعلی بدون هیچ تغییری.
pub fn build_subtitles_filter(t: &HardsubTypography) -> String {
    if !t.enabled {
        return "subtitles=sub.srt".into();
    }
    let mut filter = "subtitles=sub.srt".to_string();
    if let Some(font_path) = &t.font_path {
        // fontsdir = پدرِ فایل فونت — قرارداد Settings (font_path = فایل)
        let dir = std::path::Path::new(font_path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !dir.is_empty() {
            filter.push_str(&format!(":fontsdir={}", fontsdir_value(&dir)));
        }
    }
    let style = build_force_style(t);
    if !style.is_empty() {
        filter.push_str(&format!(":force_style='{}'", style));
    }
    filter
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typography() -> HardsubTypography {
        HardsubTypography {
            enabled: true,
            font_name: "Vazirmatn".into(),
            font_path: Some("D:/Fonts/Vazirmatn-Regular.ttf".into()),
            font_size: 32,
            primary_color: "&H0000FFFF".into(),
            outline_color: "&H00000000".into(),
            outline: 3,
            shadow: 1,
            margin_v: 40,
        }
    }

    #[test]
    fn disabled_typography_keeps_exact_current_filter() {
        let mut t = typography();
        t.enabled = false;
        assert_eq!(build_subtitles_filter(&t), "subtitles=sub.srt");
    }

    #[test]
    fn enabled_filter_includes_fontsdir_and_force_style() {
        let filter = build_subtitles_filter(&typography());
        assert!(
            filter.starts_with("subtitles=sub.srt:fontsdir='D:/Fonts':force_style='"),
            "{filter}"
        );
        assert!(filter.contains("FontName=Vazirmatn"));
        assert!(filter.contains("Fontsize=32"));
        assert!(filter.contains("PrimaryColour=&H0000FFFF"));
        assert!(filter.contains("Outline=3"));
        assert!(filter.contains("Shadow=1"));
        assert!(filter.contains("MarginV=40"));
        assert!(filter.ends_with("'"), "force_style باید quoted باشد");
    }

    #[test]
    fn windows_backslash_path_is_normalized_and_quoted() {
        let mut t = typography();
        t.font_path = Some("C:\\Windows Fonts\\Vazir.ttf".into());
        let filter = build_subtitles_filter(&t);
        assert!(filter.contains(":fontsdir='C:/Windows Fonts'"), "{filter}");
        assert!(!filter.contains("\\"), "بک‌اسلش خام در فیلتر مجاز نیست");
    }

    #[test]
    fn without_font_path_only_force_style_is_added() {
        let mut t = typography();
        t.font_path = None;
        let filter = build_subtitles_filter(&t);
        assert!(!filter.contains("fontsdir"), "{filter}");
        assert!(filter.contains(":force_style='"), "{filter}");
    }

    #[test]
    fn empty_font_name_skips_fontname_field() {
        let mut t = typography();
        t.font_name = String::new();
        let style = build_force_style(&t);
        assert!(!style.contains("FontName="), "{style}");
        assert!(style.starts_with("Fontsize="));
    }

    #[test]
    fn color_validation_accepts_ass_formats_only() {
        assert!(is_valid_color("&H00FFFFFF"));
        // شش‌رقم BGR بدون آلفا هم مجاز است
        assert!(is_valid_color("&HFFFFFF"));
        assert!(is_valid_color("&h00000000"));
        assert!(!is_valid_color("FFFFFF"));
        assert!(!is_valid_color("&HFFF")); // ۳ رقم
        assert!(!is_valid_color("&H00FFF")); // ۵ رقم
        assert!(!is_valid_color("&HGGHHII"));
        assert!(!is_valid_color(""));
    }

    #[test]
    fn values_are_clamped_into_safe_ranges() {
        let mut t = typography();
        t.font_size = 500;
        t.outline = 99;
        t.shadow = 99;
        t.margin_v = 9999;
        let style = build_force_style(&t);
        assert!(style.contains("Fontsize=96"), "{style}");
        assert!(style.contains("Outline=10"), "{style}");
        assert!(style.contains("Shadow=10"), "{style}");
        assert!(style.contains("MarginV=400"), "{style}");
    }
}
