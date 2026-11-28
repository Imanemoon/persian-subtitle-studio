//! نام‌گذاری خروجی‌ها — قالبِ محدود و قابل‌اعتبارسنجی برای بخش پایهٔ نام.
//!
//! قراردادهای امنیتی:
//! - قالب فقط **بخش پایهٔ نام** خروجی sidecar سینک‌شده را می‌سازد؛ پسوند
//!   قراردادی `.fa.synced.srt` همیشه حفظ می‌شود تا تشخیص فارسی/سینک‌شده در
//!   اجراهای بعدی (`inspect_persian_sidecars` + `is_synced_name`) کار کند.
//! - رندر فقط رشتهٔ نام می‌سازد — نه مسیر مطلق، نه هیچ ارزیابی عبارت؛ خروجی
//!   همیشه زیر پوشهٔ خود ویدیو join می‌شود (path traversal ساختاری ناممکن است:
//!   جداکننده‌ها در sanitize حذف می‌شوند و مقادیر متادیتا هم از نام فایل‌های
//!   موجود روی فایل‌سیستم می‌آیند).
//! - پلاسهولدر ناشناخته/قالب بدشکل در زمان ذخیره رد می‌شود؛ فقط «متادیتای
//!   گم‌شده» در زمان رندر → fallback قطعی به پیش‌فرض `{name}` (هیچ براکت
//!   خامی در نام نهایی نمی‌نشیند).
//! - تعارض با فایل موجود از مسیر no-overwrite موجود (`place_subtitle` با
//!   `create_new`) حل می‌شود — قالب هرگز بازنویسی نمی‌کند.

use std::path::Path;

use crate::models::VideoEntry;

/// قالب پیش‌فرض پایهٔ نام — همان رفتار قبلی (هم‌نام ویدیو).
pub const DEFAULT_BASE_TEMPLATE: &str = "{name}";

/// پلاسهولدرهای مجاز — عمداً محدود به متادیتایی که اسکن واقعاً دارد.
pub const ALLOWED_PLACEHOLDERS: [&str; 5] = ["name", "show", "season", "episode", "parent"];

/// سقف طول پایهٔ نام رندرشده — جلوگیری از مسیرهای نزدیک MAX_PATH ویندوز.
const MAX_BASE_CHARS: usize = 100;

/// کاراکترهای ممنوع در نام فایل ویندوز + جداکننده‌های مسیر.
const FORBIDDEN_CHARS: [char; 11] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|', '\0', '\n'];

/// نام‌های رزرو دستگاه ویندوز — به‌عنوان اولین توکن نام فایل با هر پسوندی ممنوع‌اند.
const RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// متادیتای نام‌گذاری یک ویدیو — فقط از اسکن واقعی می‌آید، هرگز از فرانت.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputNameContext {
    /// stem فایل ویدیو — همیشه هست
    pub name: String,
    /// نام نمایشی سریال از نام‌گذاری (fallback اسکن) — ممکن نیست
    pub show: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    /// نام پوشهٔ والد
    pub parent: String,
}

impl OutputNameContext {
    pub fn from_entry(entry: &VideoEntry) -> Self {
        let name = Path::new(&entry.file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| entry.file_name.clone());
        let parent = Path::new(&entry.parent_dir)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        OutputNameContext {
            name,
            show: entry.show_name.clone(),
            season: entry.season,
            episode: entry.episode,
            parent,
        }
    }
}

/// اعتبارسنجی قالب در زمان ذخیره‌سازی — خطاها به کاربر برمی‌گردند، هرگز
/// بی‌صدا sanitize نمی‌شوند (کاربر باید بداند قالبش پذیرفته نشد).
///
/// قواعد: فقط پلاسهولدرهای `ALLOWED_PLACEHOLDERS`، براکت‌های متوازن، بدون
/// جداکنندهٔ مسیر و کاراکتر ممنوع ویندوز، بدون کاراکتر کنترلی.
pub fn validate_template(template: &str) -> Result<(), String> {
    let trimmed = template.trim();
    if trimmed.is_empty() {
        return Err("قالب نام خالی است — حداقل یک متن یا پلاسهولدر لازم است.".into());
    }
    let mut chars = trimmed.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                let mut inner = String::new();
                let closed = loop {
                    match chars.next() {
                        Some('}') => break true,
                        Some('{') => break false, // براکت تو در تو — بدشکل
                        Some(next) => inner.push(next),
                        None => break false,
                    }
                };
                if !closed {
                    return Err("براکت باز بدون بستن — «{» باید با «}» بسته شود.".into());
                }
                if !ALLOWED_PLACEHOLDERS.contains(&inner.as_str()) {
                    return Err(format!(
                        "پلاسهولدر «{{{inner}}}» ناشناخته است — مجازها: {}",
                        ALLOWED_PLACEHOLDERS
                            .iter()
                            .map(|p| format!("{{{p}}}"))
                            .collect::<Vec<_>>()
                            .join("، ")
                    ));
                }
            }
            '}' => return Err("براکت بسته بدون باز کردن — «}» اضافه است.".into()),
            forbidden if FORBIDDEN_CHARS.contains(&forbidden) => {
                return Err(format!(
                    "کاراکتر ممنوع «{forbidden}» در قالب نیست — نام خروجی هرگز نمی‌تواند جداکنندهٔ مسیر یا کاراکتر ممنوع ویندوز داشته باشد."
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// پاک‌سازی قطعی پایهٔ نام رندرشده — لایهٔ دفاعی آخر قبل از join:
/// کاراکترهای ممنوع → فاصله، فشرده‌سازی فاصله‌ها، حذف نقطه/فاصلهٔ ابتدا و
/// انتها، گارد نام رزرو ویندوز، سقف طول. خروجی خالی → None.
fn sanitize_base(rendered: &str) -> Option<String> {
    let cleaned: String = rendered
        .chars()
        .map(|c| {
            if FORBIDDEN_CHARS.contains(&c) || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(['.', ' ']);
    if trimmed.is_empty() {
        return None;
    }
    let mut base = trimmed.to_string();
    // نام رزرو ویندوز با هر پسوندی ممنوع است — پیشوند امن می‌گیرد
    let first_token = base.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED_NAMES.contains(&first_token.as_str()) {
        base = format!("_{}", base);
    }
    // سقف طول روی مرز کاراکتر
    if base.chars().count() > MAX_BASE_CHARS {
        base = base.chars().take(MAX_BASE_CHARS).collect();
        let trimmed_again = base.trim_end_matches(['.', ' ']);
        if trimmed_again.is_empty() {
            return None;
        }
        base = trimmed_again.to_string();
    }
    if base.is_empty() {
        None
    } else {
        Some(base)
    }
}

/// رندر بخش پایهٔ نام از قالب — خروجی `None` یعنی متادیتای لازم برای یک
/// پلاسهولدر موجود نیست (مثلاً `{episode}` روی فیلم) → فراخواننده قطعی به
/// پیش‌فرض `{name}` fallback می‌کند. هیچ براکت خامی در خروجی نمی‌نشیند.
pub fn render_base(template: &str, ctx: &OutputNameContext) -> Option<String> {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '{' {
            out.push(c);
            continue;
        }
        let mut inner = String::new();
        loop {
            match chars.next() {
                Some('}') => break,
                Some(next) => inner.push(next),
                None => return None, // قالب بدشکل — نباید رخ دهد (validate در save)
            }
        }
        match inner.as_str() {
            "name" => out.push_str(&ctx.name),
            "parent" => out.push_str(&ctx.parent),
            "show" => out.push_str(ctx.show.as_ref()?),
            "season" => out.push_str(&format!("{:02}", ctx.season?)),
            "episode" => out.push_str(&format!("{:02}", ctx.episode?)),
            // ناشناخته → بدون حدس، fallback کامل
            _ => return None,
        }
    }
    sanitize_base(&out)
}

/// مسیر نهایی sidecar سینک‌شده — همیشه زیر پوشهٔ خود ویدیو، با پسوند
/// قراردادی حفظ‌شده. `ctx` نبودِ اسکن → پیش‌فرض هم‌نام ویدیو.
pub fn synced_sidecar_target(
    video: &Path,
    ctx: Option<&OutputNameContext>,
    template: &str,
) -> std::path::PathBuf {
    let parent = video.parent().unwrap_or_else(|| Path::new("."));
    let stem = video
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "subtitle".into());
    let base = ctx
        .and_then(|c| render_base(template, c))
        .and_then(|rendered| sanitize_base(&rendered))
        .or_else(|| sanitize_base(&stem))
        .unwrap_or_else(|| "subtitle".into());
    parent.join(format!("{}.fa.synced.srt", base))
}

/// پاک‌سازی ملایم قالب برای مسیرهای بارگذاری/گارانتی (نه مسیر save) —
/// trim و در صورت invalid بودن، بازگشت قطعی به پیش‌فرض. قالب‌های ورودیِ
/// کاربر در save با `validate_template` صریحاً رد می‌شوند؛ این فقط
/// مقادیر خرابِ تنظیمات/پیش‌فرض‌ها را امن نگه می‌دارد.
pub fn sanitize_template_lenient(template: &str) -> String {
    let trimmed = template.trim().to_string();
    match validate_template(&trimmed) {
        Ok(()) => trimmed,
        Err(_) => DEFAULT_BASE_TEMPLATE.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(
        name: &str,
        show: Option<&str>,
        season: Option<u32>,
        episode: Option<u32>,
    ) -> OutputNameContext {
        // از_entry همیشه stem می‌دهد — helper هم همان قرارداد را دارد
        let stem = Path::new(name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.to_string());
        OutputNameContext {
            name: stem,
            show: show.map(|s| s.into()),
            season,
            episode,
            parent: "Season 01".into(),
        }
    }

    // ---- validate_template ----

    #[test]
    fn default_template_is_valid() {
        assert_eq!(validate_template(DEFAULT_BASE_TEMPLATE), Ok(()));
        assert_eq!(validate_template("{show} - S{season}E{episode}"), Ok(()));
        assert_eq!(validate_template("{name} 1080p"), Ok(()));
    }

    #[test]
    fn unknown_placeholder_is_rejected_with_known_list() {
        let err = validate_template("{name}-{title}").unwrap_err();
        assert!(err.contains("ناشناخته"), "{err}");
        assert!(
            err.contains("{episode}"),
            "فهرست پلاسهولدرها باید باشد: {err}"
        );
    }

    #[test]
    fn unbalanced_braces_are_rejected() {
        assert!(validate_template("{name").is_err());
        assert!(validate_template("name}").is_err());
        assert!(validate_template("{na{me}").is_err());
    }

    #[test]
    fn path_separators_and_forbidden_chars_are_rejected() {
        assert!(validate_template("{name}/sub").is_err());
        assert!(validate_template("..\\{name}").is_err());
        assert!(validate_template("a:b?{name}").is_err());
        assert!(validate_template("{name}|x").is_err());
    }

    #[test]
    fn empty_template_is_rejected() {
        assert!(validate_template("").is_err());
        assert!(validate_template("   ").is_err());
    }

    // ---- render_base ----

    #[test]
    fn default_template_renders_video_stem() {
        let c = ctx("Show.S01E05.1080p.mkv", Some("Show"), Some(1), Some(5));
        assert_eq!(
            render_base(DEFAULT_BASE_TEMPLATE, &c),
            Some("Show.S01E05.1080p".into())
        );
    }

    #[test]
    fn episode_template_renders_padded_metadata() {
        let c = ctx("Show.S01E05.mkv", Some("My Show"), Some(1), Some(5));
        assert_eq!(
            render_base("{show} S{season}E{episode}", &c),
            Some("My Show S01E05".into())
        );
        // اعداد یک‌رقمی صفرپر می‌شوند
        let c2 = ctx("Show.S01E05.mkv", Some("S"), Some(7), Some(9));
        assert_eq!(
            render_base("S{season}E{episode} - {show}", &c2),
            Some("S07E09 - S".into())
        );
    }

    #[test]
    fn missing_metadata_never_renders_raw_braces() {
        // فیلم — بدون فصل/قسمت
        let movie = ctx("Movie.2023.mkv", Some("Movie"), None, None);
        assert_eq!(render_base("{show} S{season}E{episode}", &movie), None);
        // بدون show هم همان نتیجه
        let no_show = ctx("Movie.2023.mkv", None, Some(1), Some(2));
        assert_eq!(render_base("{show} - S{season}E{episode}", &no_show), None);
    }

    #[test]
    fn parent_placeholder_renders_folder_name_and_separator_is_neutralized() {
        let c = ctx("Show.S01E05.mkv", Some("Show"), Some(1), Some(5));
        // قالب با جداکننده در save رد می‌شود؛ اینجا لایهٔ دفاعی رندر:
        // حتی اگر جداکننده‌ای برسد، به فاصله تبدیل می‌شود و مسیر نمی‌سازد
        assert_eq!(
            render_base("{parent} - {name}", &c),
            Some("Season 01 - Show.S01E05".into())
        );
        assert_eq!(
            render_base("{parent}/{name}", &c),
            Some("Season 01 Show.S01E05".into()),
            "لایهٔ دفاعی: جداکنندهٔ مسیر هرگز در نام نهایی نمی‌ماند"
        );
    }

    #[test]
    fn sanitized_metadata_values_cannot_escape_or_break_names() {
        // متادیتا از نام فایل‌های موجود می‌آید — ولی لایهٔ دفاعی جداکننده‌ها
        // و کاراکترهای ممنوع را حذف می‌کند
        let evil = OutputNameContext {
            name: "a..b:c?d".into(),
            show: Some("..".into()),
            season: Some(1),
            episode: Some(2),
            parent: "x".into(),
        };
        let rendered = render_base("{name}", &evil).unwrap();
        assert!(
            !rendered.contains(':') && !rendered.contains('?'),
            "{rendered}"
        );
        // نقطه‌های لبه حذف می‌شوند (Windows)؛ نقطهٔ داخلی قانونی است
        assert!(
            !rendered.starts_with('.') && !rendered.ends_with('.'),
            "{rendered}"
        );
    }

    #[test]
    fn reserved_device_name_gets_safe_prefix() {
        let c = ctx("CON.mkv", None, None, None);
        assert_eq!(render_base(DEFAULT_BASE_TEMPLATE, &c), Some("_CON".into()));
    }

    #[test]
    fn long_render_is_capped_on_char_boundary() {
        let long = "ف".repeat(200);
        let c = ctx(&long, None, None, None);
        let rendered = render_base(DEFAULT_BASE_TEMPLATE, &c).unwrap();
        assert!(rendered.chars().count() <= MAX_BASE_CHARS);
    }

    // ---- synced_sidecar_target ----

    #[test]
    fn target_stays_inside_video_directory_with_contract_suffix() {
        let video = std::path::PathBuf::from("D:/Media/Season 01/Show.S01E05.mkv");
        let c = ctx("Show.S01E05.mkv", Some("Show"), Some(1), Some(5));
        let target = synced_sidecar_target(&video, Some(&c), "{show} S{season}E{episode}");
        assert_eq!(
            target,
            std::path::PathBuf::from("D:/Media/Season 01/Show S01E05.fa.synced.srt")
        );
        assert!(
            target.parent() == video.parent(),
            "خروجی باید داخل همان پوشه باشد"
        );
        assert!(target.to_string_lossy().ends_with(".fa.synced.srt"));
    }

    #[test]
    fn missing_metadata_falls_back_to_video_stem() {
        let video = std::path::PathBuf::from("D:/Media/Movie.2023.mkv");
        let movie = ctx("Movie.2023.mkv", Some("Movie"), None, None);
        let target = synced_sidecar_target(&video, Some(&movie), "{show} S{season}E{episode}");
        assert_eq!(
            target,
            std::path::PathBuf::from("D:/Media/Movie.2023.fa.synced.srt")
        );
    }

    #[test]
    fn no_context_falls_back_to_stem() {
        let video = std::path::PathBuf::from("D:/Media/Show.S01E05.mkv");
        let target = synced_sidecar_target(&video, None, "{show} S{season}");
        assert_eq!(
            target,
            std::path::PathBuf::from("D:/Media/Show.S01E05.fa.synced.srt")
        );
    }

    #[test]
    fn lenient_sanitize_resets_broken_templates() {
        assert_eq!(sanitize_template_lenient("  {name}  "), "{name}");
        assert_eq!(
            sanitize_template_lenient("{name}/{title}"),
            DEFAULT_BASE_TEMPLATE,
            "قالب خرابِ تنظیمات قطعی به پیش‌فرض برمی‌گردد"
        );
    }
}
