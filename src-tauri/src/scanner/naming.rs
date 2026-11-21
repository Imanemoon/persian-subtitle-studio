//! تشخیص نام سریال، فصل و قسمت از نام فایل‌ها و پوشه‌ها.
//!
//! قواعد حساس:
//! - `S04E01` نباید به اشتباه داخل `S04E010` (قسمت ۱۰) تطبیق داده شود؛
//!   پس از ارقام شماره قسمت باید حتماً یک کاراکتر غیر عددی (یا انتهای رشته) بیاید.
//! - الگوهای `2x05` نیز پشتیبانی می‌شوند.
//! - هویت چندقسمتی دقیق نگه داشته می‌شود: زنجیرهٔ صریح (`S04E01E02E03`) با هر
//!   طولی کامل ثبت می‌شود و هرگز به دو قسمت کوتاه نمی‌شود؛ بازهٔ پیوسته
//!   (`S04E01-12`) و فهرست صریح (`S04E01E12`) نیز هرگز با هم برابر نیستند.

use std::sync::LazyLock;

use regex::Regex;

/// پسوندهای ویدیویی پشتیبانی‌شده.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "mov", "avi", "ts", "m2ts", "mts", "webm", "mpg", "mpeg", "vob", "wmv",
    "flv",
];

/// پسوندهای زیرنویس متنی/کناری که در فاز ۱ بررسی می‌شوند.
pub const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "vtt", "sub"];

// الگوی اصلی: S04E01 / s4e1 / S04.E01 / S04_E01
// کریت regex در Rust از lookahead پشتیبانی نمی‌کند؛ برای گارد «پس از شماره قسمت
// رقم نباید بیاید» (جلوگیری از تطبیق E01 داخل E010) کاراکتر بعدی مصرف می‌شود:
// (?:[^0-9]|$)
static SE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[\s._\-\[])[s](\d{1,2})[\s._\-]*e(\d{1,3})(?:[^0-9]|$)").unwrap()
});

// الگوی جایگزین: 2x05
static X_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|[\s._\-\[])(\d{1,2})x(\d{1,3})(?:[^0-9]|$)").unwrap());

// ادامهٔ زنجیرهٔ صریح قسمت‌ها: S01E01E02 / S01E01.E02 / S01E01_E02
// خط تیره عمداً جداکنندهٔ زنجیره نیست؛ `S01E01-E02` بازه است.
static EXPLICIT_EPISODE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[\s._]*e(\d{1,3})").unwrap());

// ادامهٔ بازه: S01E01-12 / S01E01-E12 / S01E01 to E12
// گروه ۱ وجود `E` صریح و گروه ۲ ارقام نقطهٔ پایانی را نگه می‌دارد.
// مرز و تگ کیفیت در `range_continuation` بررسی می‌شوند.
static RANGE_CONTINUATION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[\s._\-]*(?:-|–|—|to\b|till\b)[\s._\-]*(e?)(\d{1,3})").unwrap()
});

// نشانگر `E` ناقص پس از جداکنندهٔ بازه: S01E01-E / S01E01-E.fa
// اگر بعد از `E` حرف بیاید (مثل `-Extended`) متن عنوان است، نه نشانگر ناقص.
static DANGLING_E_MARKER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[\s._\-]*(?:-|–|—|to\b|till\b)[\s._\-]*e(?:$|[^\p{L}\p{N}])").unwrap()
});

// نام پوشه فصل: "Season 02" / "S02" / "Season 2"
static SEASON_DIR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:season|s)[\s._\-]*(\d{1,2})$").unwrap());

// نام پوشه فصل فارسی: "فصل 1" / "فصل01"
static FA_SEASON_DIR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*فصل[\s]*(\d{1,2})\s*$").unwrap());

// نشانه‌های فارسی در نام زیرنویس — ترتیب طولانی‌به-کوتاه مهم است
// تا «persian» به اشتباه با «per» نیمه‌تطبیق نشود.
static FA_NAME_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[^a-z])(?:persian|farsi|fas|per|fa)(?:[^a-z]|$)|فارسی").unwrap()
});

// نشانه‌های زیرنویس هاردساب (چسبیده به تصویر) در نام ویدیو — قرارداد
// شناخته‌شدهٔ گروه‌های رلیز فارسی: Farsi.Sub / Farsi.Subbed / HardSub.
// مرز قبل/بعد سخت‌گیرانه است تا «Farsi.Subs» (پک زیرنویس چسبیده) و
// «Subtitle.Off» جعلی نشوند؛ کدک/کیفیت بعد از sub اجازه ندارد حروف اضافه بگیرد.
static FA_HARDSUB_NAME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|[\s._\-\[(])(?:hard[\s._-]?sub|(?:farsi|persian|fa)[\s._-]?sub(?:bed|title)?)(?:[\s._\-\])]|$)",
    )
    .unwrap()
});

/// آیا نام فایل ویدیو نشانهٔ زیرنویس فارسی هاردساب (داخل تصویر حک‌شده) دارد؟
///
/// این تشخیص نام‌مبناست (OCR تصویر در دسترس نیست) و فقط وقتی معنا دارد که
/// ویدیو هیچ منبع زیرنویس دیگری (Track داخلی/sidecar) نداشته باشد — بالادست
/// در pipeline همین‌طور مصرف می‌شود. گزارش‌های کاربر: Film2Media/WorldMkv و
/// گروه‌های مشابه «Farsi.Sub» یعنی زیرنویس چسبیده.
pub fn name_suggests_fa_hardsub(file_name: &str) -> bool {
    FA_HARDSUB_NAME_RE.is_match(file_name)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeMatch {
    pub season: u32,
    pub episode: u32,
    /// بایت آغازین الگوی SxxEyy در رشته ورودی (شامل کاراکتر جداکننده قبل از آن).
    pub token_start: usize,
}

/// شماره فصل و قسمت را از نام فایل ویدیویی استخراج می‌کند.
pub fn parse_episode(file_stem: &str) -> Option<EpisodeMatch> {
    if let Some(c) = SE_RE.captures(file_stem) {
        let whole = c.get(0)?;
        return Some(EpisodeMatch {
            season: c[1].parse().ok()?,
            episode: c[2].parse().ok()?,
            token_start: whole.start(),
        });
    }
    let c = X_RE.captures(file_stem)?;
    let whole = c.get(0)?;
    Some(EpisodeMatch {
        season: c[1].parse().ok()?,
        episode: c[2].parse().ok()?,
        token_start: whole.start(),
    })
}

/// هویت قسمت یک نام فایل.
///
/// سه شکل از هم متمایزند و هرگز با هم برابر نمی‌شوند:
/// - `Single`: یک قسمت (`S01E01`)
/// - `Range`: بازهٔ پیوسته (`S01E01-12`، `S01E01-E12`)
/// - `Sequence`: فهرست صریح قسمت‌ها (`S01E01E02`، `S01E01.E02.E03`)
///
/// `Range` و `Sequence` حتی اگر همان مجموعه قسمت‌ها را توصیف کنند برابر نیستند.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpisodeIdentity {
    Single {
        season: u32,
        episode: u32,
    },
    Range {
        season: u32,
        episode_start: u32,
        episode_end: u32,
    },
    Sequence {
        season: u32,
        episodes: Vec<u32>,
    },
}

enum ChainItem {
    Stop,
    Valid { episode: u32, next: usize },
    Unsupported,
}

/// یک شناسهٔ صریح `E02` را از ابتدای متن مصرف می‌کند.
fn episode_chain_item(text: &str) -> ChainItem {
    let Some(c) = EXPLICIT_EPISODE_RE.captures(text) else {
        return ChainItem::Stop;
    };
    let Some(digits) = c.get(1) else {
        return ChainItem::Stop;
    };
    // `E1000` یعنی شناسهٔ خارج از پشتیبانی؛ نباید به `E100` کوتاه شود.
    if text[digits.end()..]
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
    {
        return ChainItem::Unsupported;
    }
    let Ok(episode) = digits.as_str().parse::<u32>() else {
        return ChainItem::Unsupported;
    };
    ChainItem::Valid {
        episode,
        next: digits.end(),
    }
}

enum RangeContinuation {
    NoRange,
    Valid { episode: u32, next: usize },
    Unsupported,
}

/// بازهٔ پیوسته را از ابتدای متن مصرف می‌کند.
///
/// سه حالت را از هم جدا می‌کند: نشانگر بازه وجود ندارد، بازهٔ معتبر است، یا
/// نشانگر وجود دارد ولی نقطهٔ پایانی خارج از پشتیبانی/مخلوط با زنجیره است.
/// تگ‌های کیفیت (`-720p`، `-1080i`) بازه نیستند.
fn range_continuation(text: &str) -> RangeContinuation {
    let Some(c) = RANGE_CONTINUATION_RE.captures(text) else {
        return if DANGLING_E_MARKER_RE.is_match(text) {
            RangeContinuation::Unsupported
        } else {
            RangeContinuation::NoRange
        };
    };
    let Some(digits) = c.get(2) else {
        return RangeContinuation::NoRange;
    };
    let explicit_episode = !c[1].is_empty();

    // کل رشتهٔ رقمی پس از نشانگر خوانده می‌شود؛ `-1080p` پیش از بررسی سرریز
    // باید به‌عنوان تگ کیفیت شناخته شود.
    let mut full_end = digits.end();
    for ch in text[full_end..].chars() {
        if !ch.is_ascii_digit() {
            break;
        }
        full_end += 1;
    }
    let after_full = text[full_end..].chars().next();

    if !explicit_episode
        && after_full
            .is_some_and(|ch| ch.eq_ignore_ascii_case(&'p') || ch.eq_ignore_ascii_case(&'i'))
    {
        return RangeContinuation::NoRange;
    }

    // نقطهٔ پایانی بیش از سه رقم (مثل `-E1000`) پشتیبانی نمی‌شود.
    if full_end > digits.end() {
        return RangeContinuation::Unsupported;
    }

    match after_full {
        None => {}
        Some(ch) if ch.is_ascii_alphabetic() => {
            if ch.eq_ignore_ascii_case(&'e') {
                let after_ch = text[full_end + ch.len_utf8()..].chars().next();
                let incomplete_or_mixed = match after_ch {
                    None => true,
                    Some(d) => d.is_ascii_digit() || !d.is_alphanumeric(),
                };
                // `-E02E03` (مخلوط) یا `-12E` / `-12E.fa` (ناقص) پشتیبانی نمی‌شود.
                if incomplete_or_mixed {
                    return RangeContinuation::Unsupported;
                }
            }
            // حروف دیگر (مثل `-10bit`، `-x264` یا `-Extended`) ادامهٔ بازه نیستند.
            return RangeContinuation::NoRange;
        }
        Some(_) => {}
    }

    let Ok(episode) = digits.as_str().parse::<u32>() else {
        return RangeContinuation::Unsupported;
    };
    RangeContinuation::Valid {
        episode,
        next: digits.end(),
    }
}

/// آیا پس از یک بازه، ادامهٔ قسمت دیگری مانده است؟ (فرم ترکیبی پشتیبانی نمی‌شود)
fn has_dangling_episode(text: &str) -> bool {
    match episode_chain_item(text) {
        ChainItem::Stop => matches!(
            range_continuation(text),
            RangeContinuation::Valid { .. } | RangeContinuation::Unsupported
        ),
        ChainItem::Valid { .. } | ChainItem::Unsupported => true,
    }
}

/// هویت کامل قسمت را از نام فایل استخراج می‌کند.
///
/// زنجیره‌های صریح با هر طولی دقیق حفظ می‌شوند (`S01E01E02E03` سه قسمت است)،
/// بازه‌ها با `Range` و فهرست‌های صریح با `Sequence` نمایش داده می‌شوند و
/// هرگز به هم تبدیل نمی‌شوند. شناسه‌های خارج از پشتیبانی (مثلاً `E1000`)
/// به‌جای کوتاه‌شدن، کل نتیجه را نامعتبر می‌کنند.
pub fn parse_episode_identity(name: &str) -> Option<EpisodeIdentity> {
    if let Some(c) = SE_RE.captures(name) {
        let season: u32 = c[1].parse().ok()?;
        let episode: u32 = c[2].parse().ok()?;
        let token_end = c.get(2)?.end();
        return parse_identity_tail(name, season, episode, token_end);
    }
    let c = X_RE.captures(name)?;
    let season: u32 = c[1].parse().ok()?;
    let episode: u32 = c[2].parse().ok()?;
    let token_end = c.get(2)?.end();
    parse_identity_tail(name, season, episode, token_end)
}

fn parse_identity_tail(
    name: &str,
    season: u32,
    first_episode: u32,
    mut pos: usize,
) -> Option<EpisodeIdentity> {
    let mut episodes = vec![first_episode];

    loop {
        match episode_chain_item(&name[pos..]) {
            ChainItem::Stop => break,
            ChainItem::Unsupported => return None,
            ChainItem::Valid { episode, next } => {
                episodes.push(episode);
                pos += next;
            }
        }
    }

    if episodes.len() > 1 {
        // ترکیب زنجیرهٔ صریح و بازه پشتیبانی نمی‌شود؛ به‌جای نادیده‌گرفتن رد می‌شود.
        if !matches!(range_continuation(&name[pos..]), RangeContinuation::NoRange) {
            return None;
        }
        return Some(EpisodeIdentity::Sequence { season, episodes });
    }

    match range_continuation(&name[pos..]) {
        RangeContinuation::NoRange => {}
        RangeContinuation::Unsupported => return None,
        RangeContinuation::Valid {
            episode: last,
            next,
        } => {
            if has_dangling_episode(&name[pos + next..]) {
                return None;
            }
            let start = first_episode.min(last);
            let end = first_episode.max(last);
            if start == end {
                return Some(EpisodeIdentity::Single {
                    season,
                    episode: start,
                });
            }
            return Some(EpisodeIdentity::Range {
                season,
                episode_start: start,
                episode_end: end,
            });
        }
    }

    Some(EpisodeIdentity::Single {
        season,
        episode: first_episode,
    })
}

/// اگر نام پوشه الگوی پوشه فصل باشد شماره فصل را برمی‌گرداند.
pub fn parse_season_dir(dir_name: &str) -> Option<u32> {
    SEASON_DIR_RE
        .captures(dir_name)
        .and_then(|c| c[1].parse().ok())
        .or_else(|| {
            FA_SEASON_DIR_RE
                .captures(dir_name)
                .and_then(|c| c[1].parse().ok())
        })
}

/// پسوند فایل را بدون نقطه و به‌صورت حروف کوچک برمی‌گرداند.
pub fn file_extension(file_name: &str) -> String {
    std::path::Path::new(file_name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

pub fn is_video_file(file_name: &str) -> bool {
    let ext = file_extension(file_name);
    VIDEO_EXTENSIONS.contains(&ext.as_str())
}

pub fn is_subtitle_file(file_name: &str) -> bool {
    let ext = file_extension(file_name);
    SUBTITLE_EXTENSIONS.contains(&ext.as_str())
}

/// آیا نام فایل زیرنویس نشانه فارسی دارد؟ (fa / fas / per / farsi / persian / فارسی)
pub fn name_suggests_fa(file_name: &str) -> bool {
    FA_NAME_TOKEN_RE.is_match(file_name)
}

/// آیا یک متن آزاد (Language Tag یا عنوان Track) نشانه فارسی دارد؟
pub fn text_suggests_fa(text: &str) -> bool {
    FA_NAME_TOKEN_RE.is_match(text)
}

/// آیا نام زیرنویس کناری متعلق به همین ویدیو است؟
///
/// گارد مهم: `Show.S01E010.srt` نباید برای ویدیوی `Show.S01E01.mkv`
/// تطبیق داده شود — پس از پیشوند نام ویدیو نباید رقم بیاید.
pub fn sub_matches_video_stem(sub_stem: &str, video_stem: &str) -> bool {
    let s = sub_stem.to_lowercase();
    let v = video_stem.to_lowercase();
    if v.is_empty() || s.len() < v.len() || !s.starts_with(&v) {
        return false;
    }
    match s[v.len()..].chars().next() {
        None => true,
        Some(c) => !c.is_ascii_digit(),
    }
}

/// نام سریال را از متن خام قبل از الگوی SxxEyy تمیز می‌کند.
pub fn clean_show_name(raw: &str) -> String {
    let cleaned = raw.replace(['.', '_', '-', '[', ']', '(', ')'], " ");
    let mut out = String::with_capacity(cleaned.len());
    let mut last_space = false;
    for ch in cleaned.chars() {
        if ch.is_whitespace() {
            if !last_space && !out.is_empty() {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(name: &str) -> Option<(u32, u32)> {
        parse_episode(name).map(|m| (m.season, m.episode))
    }

    /// تشخیص هاردساب نام‌مبنا — قراردادهای واقعی گروه‌های رلیز فارسی.
    /// نمونه‌های مثبت از آرشیو واقعی کاربر (Film2Media و مشابه).
    #[test]
    fn detects_hardsub_release_conventions() {
        for name in [
            "Overlord.IV.S04E13.1080p.WEB-DL.Farsi.Sub.Film2Media.mkv",
            "Liar.Game.S01E01.1080p.WEB-DL.Farsi.Sub.Film2Media.mkv",
            "Show.S01E01.1080p.WEB-DL.Farsi.Subbed.Film2Media.mkv",
            "Movie.2023.Farsi.HardSub.1080p.mkv",
            "Movie.HardSub.720p.mkv",
            "Movie.Hard-Sub.mkv",
            "Movie.FarsiSub.mkv",
            "Movie.Persian.Sub.mkv",
            "show fa sub S01E01.mkv",
        ] {
            assert!(name_suggests_fa_hardsub(name), "باید هاردساب باشد: {name}");
        }
    }

    /// جعلی‌های متعارف نباید هاردساب شمرده شوند — «Subs» پک زیرنویس چسبیده است
    /// و «Subtitle» تنها با واژهٔ پیشین فارسی معنا دارد.
    #[test]
    fn does_not_flag_softsub_or_unrelated_names() {
        for name in [
            "Show.S01E01.1080p.WEB-DL.x264.mkv",
            "Show.S01E01.Farsi.Dubbed.1080p.mkv",
            "Movie.2023.1080p.BluRay.mkv",
            "Show.S01E01.Farsi.Subs.Pack.mkv",
            "SubsPlease.Show.S01E01.mkv",
            "Movie.Farsi.Dub.mkv",
            "Movie.Subtitle.Off.mkv",
            "",
        ] {
            assert!(
                !name_suggests_fa_hardsub(name),
                "نباید هاردساب باشد: {name}"
            );
        }
    }

    #[test]
    fn parses_standard_sxxeyy() {
        assert_eq!(ep("Dexter.S04E01.720p.BluRay.x264.mkv"), Some((4, 1)));
        assert_eq!(ep("dexter s04e01.mkv"), Some((4, 1)));
        assert_eq!(ep("Show.S01E10.Final.1080p.mkv"), Some((1, 10)));
        assert_eq!(ep("S01.E01.mkv"), Some((1, 1)));
        assert_eq!(ep("[SubsPlease] Show S02E03 (1080p).mkv"), Some((2, 3)));
    }

    #[test]
    fn parses_2x05_pattern() {
        assert_eq!(ep("Show.2x05.mkv"), Some((2, 5)));
        assert_eq!(ep("2x12 - Title.mkv"), Some((2, 12)));
    }

    #[test]
    fn does_not_confuse_e01_with_e010() {
        // E010 یعنی قسمت ۱۰، نه قسمت ۱
        assert_eq!(ep("Show.S04E010.mkv"), Some((4, 10)));
        assert_eq!(ep("Show.S04E0112.mkv"), None); // نامشخص — نباید حدس بزند
                                                   // تطبیق زیرنویس: E010 برای E01 نباید قبول شود
        assert!(!sub_matches_video_stem("Show.S01E010", "Show.S01E01"));
        assert!(sub_matches_video_stem("Show.S01E01.fa", "Show.S01E01"));
        assert!(sub_matches_video_stem("Show.S01E01", "Show.S01E01"));
        assert!(sub_matches_video_stem(
            "Show.S01E01.synced.fa",
            "Show.S01E01"
        ));
        assert!(!sub_matches_video_stem("Show.S01E02.fa", "Show.S01E01"));
    }

    #[test]
    fn ignores_resolutions_and_movies() {
        assert_eq!(ep("Movie.1920x1080.mkv"), None);
        assert_eq!(ep("Movie.2023.1080p.mkv"), None);
        assert_eq!(ep("My.Holiday.Video.mkv"), None);
        assert_eq!(ep("640x480.clip.mkv"), None);
    }

    #[test]
    fn parses_multi_episode_identities_exactly() {
        assert_eq!(
            parse_episode_identity("Dexter.S04E01-12.DVDRip.zip"),
            Some(EpisodeIdentity::Range {
                season: 4,
                episode_start: 1,
                episode_end: 12
            })
        );

        assert_eq!(
            parse_episode_identity("Show.S01E01-E04.zip"),
            Some(EpisodeIdentity::Range {
                season: 1,
                episode_start: 1,
                episode_end: 4
            })
        );

        // فرم چسبیده: S01E01E02 — دو قسمت، نه فقط قسمت اول
        assert_eq!(
            parse_episode_identity("Show.S01E01E02.zip"),
            Some(EpisodeIdentity::Sequence {
                season: 1,
                episodes: vec![1, 2]
            })
        );

        assert_eq!(
            parse_episode_identity("Show.S01E01.E02.zip"),
            Some(EpisodeIdentity::Sequence {
                season: 1,
                episodes: vec![1, 2]
            })
        );

        assert_eq!(
            parse_episode_identity("Show.S02E07.zip"),
            Some(EpisodeIdentity::Single {
                season: 2,
                episode: 7
            })
        );

        assert_eq!(parse_episode_identity("collection.2023.zip"), None);
    }

    #[test]
    fn episode_identity_distinguishes_single_range_and_sequence() {
        assert_eq!(
            parse_episode_identity("Show.S1E1.1080p"),
            Some(EpisodeIdentity::Single {
                season: 1,
                episode: 1
            })
        );
        assert_eq!(
            parse_episode_identity("Show.S01E01-12"),
            Some(EpisodeIdentity::Range {
                season: 1,
                episode_start: 1,
                episode_end: 12
            })
        );
        assert_eq!(
            parse_episode_identity("Show.S01E01-E02"),
            Some(EpisodeIdentity::Range {
                season: 1,
                episode_start: 1,
                episode_end: 2
            })
        );
        assert_eq!(
            parse_episode_identity("Show.S01E01E02"),
            Some(EpisodeIdentity::Sequence {
                season: 1,
                episodes: vec![1, 2]
            })
        );
        // فهرست صریح [1, 12] با بازهٔ پیوستهٔ 1..12 یکی نیست
        assert_eq!(
            parse_episode_identity("Show.S01E01E12"),
            Some(EpisodeIdentity::Sequence {
                season: 1,
                episodes: vec![1, 12]
            })
        );
        assert_ne!(
            parse_episode_identity("Show.S01E01E12"),
            parse_episode_identity("Show.S01E01-12")
        );
        // زنجیرهٔ سه‌قسمتی نباید به دو قسمت کوتاه شود
        assert_eq!(
            parse_episode_identity("Show.S01E01E02E03"),
            Some(EpisodeIdentity::Sequence {
                season: 1,
                episodes: vec![1, 2, 3]
            })
        );
        assert_eq!(
            parse_episode_identity("Show.S01E01E02E03E04"),
            Some(EpisodeIdentity::Sequence {
                season: 1,
                episodes: vec![1, 2, 3, 4]
            })
        );
        // ترکیب زنجیرهٔ صریح و بازه پشتیبانی نمی‌شود و باید کامل رد شود
        assert_eq!(parse_episode_identity("Show.S01E01E02-03"), None);
        // E010 قسمت ۱۰ است، نه بازه ۱..۰
        assert_eq!(
            parse_episode_identity("Show.S01E010"),
            Some(EpisodeIdentity::Single {
                season: 1,
                episode: 10
            })
        );
        assert_eq!(
            parse_episode_identity("Show.2x05"),
            Some(EpisodeIdentity::Single {
                season: 2,
                episode: 5
            })
        );
        assert_eq!(parse_episode_identity("Movie.2023.1080p"), None);
        // شناسهٔ چهاررقمی پشتیبانی نمی‌شود و هرگز نباید کوتاه شود
        assert_eq!(parse_episode_identity("Show.S01E01E1000"), None);
        // بازهٔ معکوس نرمال می‌شود؛ بازهٔ یک‌عضوی همان تک‌قسمت است
        assert_eq!(
            parse_episode_identity("Show.S01E12-01"),
            Some(EpisodeIdentity::Range {
                season: 1,
                episode_start: 1,
                episode_end: 12
            })
        );
        assert_eq!(
            parse_episode_identity("Show.S01E05-05"),
            Some(EpisodeIdentity::Single {
                season: 1,
                episode: 5
            })
        );
        // تگ کیفیت با خط تیره بازه نیست
        assert_eq!(
            parse_episode_identity("Show.S01E01-720p"),
            Some(EpisodeIdentity::Single {
                season: 1,
                episode: 1
            })
        );
        assert_eq!(
            parse_episode_identity("Show.S01E01E02-720p"),
            Some(EpisodeIdentity::Sequence {
                season: 1,
                episodes: vec![1, 2]
            })
        );
    }

    #[test]
    fn malformed_range_continuations_fail_closed() {
        // نقطهٔ پایانی چهاررقمی یا خارج از پشتیبانی نباید به Single تبدیل شود.
        assert_eq!(parse_episode_identity("Show.S01E01-E1000"), None);
        assert_eq!(parse_episode_identity("Show.S01E01-1000"), None);
        // ترکیب بازه و زنجیرهٔ صریح پشتیبانی نمی‌شود.
        assert_eq!(parse_episode_identity("Show.S01E01-E02E03"), None);
        assert_eq!(parse_episode_identity("Show.S01E01-12E13"), None);
        // نشانگر صریح E بدون شمارهٔ قسمت نباید به Single/Sequence تبدیل شود.
        assert_eq!(parse_episode_identity("Show.S01E01-E"), None);
        assert_eq!(parse_episode_identity("Show.S01E01-12E"), None);
        assert_eq!(parse_episode_identity("Show.S01E01E02-E"), None);
        // پسوند متنی معتبر بعد از قسمت نباید نشانگر ناقص تلقی شود.
        assert_eq!(
            parse_episode_identity("Show.S01E01-Extended"),
            Some(EpisodeIdentity::Single {
                season: 1,
                episode: 1
            })
        );
        assert_eq!(
            parse_episode_identity("Show.S01E01E02-Extended"),
            Some(EpisodeIdentity::Sequence {
                season: 1,
                episodes: vec![1, 2]
            })
        );
        // کنترل‌ها: بازهٔ معتبر و تگ کیفیت باید مثل قبل بمانند.
        assert_eq!(
            parse_episode_identity("Show.S01E01-12"),
            Some(EpisodeIdentity::Range {
                season: 1,
                episode_start: 1,
                episode_end: 12
            })
        );
        assert_eq!(
            parse_episode_identity("Show.S01E01-720p"),
            Some(EpisodeIdentity::Single {
                season: 1,
                episode: 1
            })
        );
        assert_eq!(
            parse_episode_identity("Show.S01E01-1080p"),
            Some(EpisodeIdentity::Single {
                season: 1,
                episode: 1
            })
        );
        assert_eq!(
            parse_episode_identity("Show.S01E01-10bit"),
            Some(EpisodeIdentity::Single {
                season: 1,
                episode: 1
            })
        );
    }

    #[test]
    fn parses_season_dirs() {
        assert_eq!(parse_season_dir("Season 04"), Some(4));
        assert_eq!(parse_season_dir("Season 2"), Some(2));
        assert_eq!(parse_season_dir("S02"), Some(2));
        assert_eq!(parse_season_dir("s01"), Some(1));
        assert_eq!(parse_season_dir("فصل 1"), Some(1));
        assert_eq!(parse_season_dir("فصل01"), Some(1));
        assert_eq!(parse_season_dir("Extras"), None);
        assert_eq!(parse_season_dir("Dexter"), None);
    }

    #[test]
    fn detects_fa_markers() {
        assert!(name_suggests_fa("Movie.fa.srt"));
        assert!(name_suggests_fa("Movie.farsi.srt"));
        assert!(name_suggests_fa("Movie.Persian.srt"));
        assert!(name_suggests_fa("Movie.FA.synced.srt"));
        assert!(name_suggests_fa("زیرنویس-فارسی.srt"));
        assert!(!name_suggests_fa("Movie.english.srt"));
        // «fast» و «supernatural» نباید fa حساب شوند
        assert!(!name_suggests_fa("Movie.fastsub.srt"));
        assert!(!name_suggests_fa("Supernatural.S01E01.srt"));
        // متن آزاد (Language Tag / عنوان Track)
        assert!(text_suggests_fa("fa"));
        assert!(text_suggests_fa("fas"));
        assert!(text_suggests_fa("per"));
        assert!(text_suggests_fa("farsi"));
        assert!(text_suggests_fa("Persian Subtitle"));
        assert!(text_suggests_fa("فارسی"));
        assert!(text_suggests_fa("زبان: فارسی"));
        assert!(!text_suggests_fa("english"));
        assert!(!text_suggests_fa("ara"));
        assert!(!text_suggests_fa("Forced"));
        assert!(!text_suggests_fa("Supernatural"));
        assert!(!text_suggests_fa("SDH"));
        // عنوان ترکیبی: نشانه فارسی وسط عنوان
        assert!(text_suggests_fa("Subs: Persian / English"));
    }

    #[test]
    fn cleans_show_names() {
        assert_eq!(clean_show_name("Dexter"), "Dexter");
        assert_eq!(clean_show_name("The.Bold.Type"), "The Bold Type");
        assert_eq!(clean_show_name("Show_Name-2020"), "Show Name 2020");
        assert_eq!(clean_show_name(""), "");
        assert_eq!(clean_show_name("..."), "");
    }

    #[test]
    fn file_type_checks() {
        for n in ["a.mkv", "b.MP4", "c.WebM", "d.m2ts"] {
            assert!(is_video_file(n), "{n}");
        }
        assert!(!is_video_file("e.txt"));
        assert!(!is_video_file("f.srt"));
        assert!(is_subtitle_file("f.srt"));
        assert!(is_subtitle_file("g.ASS"));
        assert!(!is_subtitle_file("a.mkv"));
    }
}
