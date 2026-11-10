//! تشخیص نام سریال، فصل و قسمت از نام فایل‌ها و پوشه‌ها.
//!
//! قواعد حساس:
//! - `S04E01` نباید به اشتباه داخل `S04E010` (قسمت ۱۰) تطبیق داده شود؛
//!   پس از ارقام شماره قسمت باید حتماً یک کاراکتر غیر عددی (یا انتهای رشته) بیاید.
//! - الگوهای `2x05` نیز پشتیبانی می‌شوند.

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

// آرشیو/زیرنویس چندقسمتی:
//   S04E01-12 / S04E01-E12 / S01E01 to E04 / S01E01E02 / S01E01.E02
// گارد پایانی از تطبیق E01 داخل E010 جلوگیری می‌کند.
static RANGE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|[\s._\-\[])s(\d{1,2})[\s._\-]*e(\d{1,3})(?:[\s._\-]*(?:-|–|—|to|till)[\s._\-]*e?|[\s._\-]*e)(\d{1,3})(?:[^0-9]|$)",
    )
    .unwrap()
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

// فاز ۳: تطبیق دقیق آرشیوهای چندقسمتی مثل Dexter.S04E01-12.zip
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveMatch {
    pub season: u32,
    pub episode_start: u32,
    pub episode_end: u32,
}

/// هویت قسمت یک نام فایل: تک‌قسمت یا چندقسمتی.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpisodeIdentity {
    Single {
        season: u32,
        episode: u32,
    },
    Multi {
        season: u32,
        episode_start: u32,
        episode_end: u32,
    },
}

/// نام آرشیو (مثلاً `Dexter.S04E01-12.DVDRip.zip`) را به بازه قسمت‌ها تبدیل می‌کند.
pub fn parse_archive(file_name: &str) -> Option<ArchiveMatch> {
    if let Some(c) = RANGE_RE.captures(file_name) {
        let season: u32 = c[1].parse().ok()?;
        let a: u32 = c[2].parse().ok()?;
        let b: u32 = c[3].parse().ok()?;
        return Some(ArchiveMatch {
            season,
            episode_start: a.min(b),
            episode_end: a.max(b),
        });
    }
    let m = parse_episode(file_name)?;
    Some(ArchiveMatch {
        season: m.season,
        episode_start: m.episode,
        episode_end: m.episode,
    })
}

/// هویت قسمت را با تفکیک صریح تک‌قسمت/چندقسمتی برمی‌گرداند.
///
/// این تابع برای تطبیق زیرنویس حیاتی است: `S01E01-12` و `S01E01E02` هرگز
/// نباید به یک قسمت تکی نسبت داده شوند.
pub fn parse_episode_identity(name: &str) -> Option<EpisodeIdentity> {
    let m = parse_archive(name)?;
    if m.episode_start == m.episode_end {
        Some(EpisodeIdentity::Single {
            season: m.season,
            episode: m.episode_start,
        })
    } else {
        Some(EpisodeIdentity::Multi {
            season: m.season,
            episode_start: m.episode_start,
            episode_end: m.episode_end,
        })
    }
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
    fn parses_multi_episode_archives() {
        let m = parse_archive("Dexter.S04E01-12.DVDRip.zip").unwrap();
        assert_eq!((m.season, m.episode_start, m.episode_end), (4, 1, 12));

        let m = parse_archive("Show.S01E01-E04.zip").unwrap();
        assert_eq!((m.season, m.episode_start, m.episode_end), (1, 1, 4));

        // فرم چسبیده: S01E01E02 — دو قسمت، نه فقط قسمت اول
        let m = parse_archive("Show.S01E01E02.zip").unwrap();
        assert_eq!((m.season, m.episode_start, m.episode_end), (1, 1, 2));

        let m = parse_archive("Show.S01E01.E02.zip").unwrap();
        assert_eq!((m.season, m.episode_start, m.episode_end), (1, 1, 2));

        let m = parse_archive("Show.S02E07.zip").unwrap();
        assert_eq!((m.season, m.episode_start, m.episode_end), (2, 7, 7));

        assert!(parse_archive("collection.2023.zip").is_none());
    }

    #[test]
    fn episode_identity_distinguishes_single_and_multi() {
        assert_eq!(
            parse_episode_identity("Show.S1E1.1080p"),
            Some(EpisodeIdentity::Single {
                season: 1,
                episode: 1
            })
        );
        assert_eq!(
            parse_episode_identity("Show.S01E01E02"),
            Some(EpisodeIdentity::Multi {
                season: 1,
                episode_start: 1,
                episode_end: 2
            })
        );
        assert_eq!(
            parse_episode_identity("Show.S01E01-12"),
            Some(EpisodeIdentity::Multi {
                season: 1,
                episode_start: 1,
                episode_end: 12
            })
        );
        // زنجیره سه‌قسمتی هم حداقل چندقسمتی شناخته می‌شود و تک‌قسمت نیست
        assert!(matches!(
            parse_episode_identity("Show.S01E01E02E03"),
            Some(EpisodeIdentity::Multi { .. })
        ));
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
