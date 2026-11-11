//! تطبیق دقیق نام زیرنویس با ویدیو — بدون حدس فازی.
//!
//! - zero-padding نرمال می‌شود (`S1E1` == `S01E01`).
//! - `E01` هرگز با `E010` تطبیق نمی‌خورد (گارد رقم بعدی).
//! - فصل جداگانه بررسی می‌شود.
//! - `2x05` پشتیبانی می‌شود.
//! - زیرنویس چندقسمتی (`S01E01-12`، `S01E01E02`) هرگز به یک قسمت تکی
//!   نسبت داده نمی‌شود؛ فقط با ویدیوی همان هویت دقیق تطبیق می‌خورد.
//! - بازهٔ پیوسته (`S01E01-12`) و فهرست صریح (`S01E01E12`) هویت‌های جدا هستند
//!   و هرگز با هم تطبیق نمی‌خورند؛ زنجیره‌های سه‌قسمتی و بلندتر هم کامل مقایسه
//!   می‌شوند و به دو قسمت کوتاه نمی‌شوند.
//! - برای فیلم (بدون هویت قسمت)، تطبیق پیشوندی سختگیرانهٔ نام انجام می‌شود.

use std::path::Path;

use crate::scanner::naming::{
    file_extension, parse_episode_identity, sub_matches_video_stem, EpisodeIdentity,
    SUBTITLE_EXTENSIONS,
};

use super::types::SubtitleCandidate;

/// هویت تطبیق ویدیو.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoKey {
    Episode(EpisodeIdentity),
    Movie { stem: String },
}

/// هویت ویدیو را از نام فایل و در صورت نبود الگو از فصل/قسمت اسکن‌شده می‌سازد.
pub fn video_key(video_file_name: &str, season: Option<u32>, episode: Option<u32>) -> VideoKey {
    if let Some(identity) = parse_episode_identity(video_file_name) {
        return VideoKey::Episode(identity);
    }
    if let (Some(season), Some(episode)) = (season, episode) {
        return VideoKey::Episode(EpisodeIdentity::Single { season, episode });
    }
    let stem = Path::new(video_file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    VideoKey::Movie { stem }
}

/// آیا این عضو آرشیو نامزد تطبیق با ویدیو است؟ (بدون بررسی محتوا)
pub fn member_matches_video(key: &VideoKey, member_path: &str) -> bool {
    let file_name = member_path.rsplit('/').next().unwrap_or(member_path);
    let ext = file_extension(file_name);
    if !SUBTITLE_EXTENSIONS.contains(&ext.as_str()) {
        return false;
    }
    let stem = Path::new(file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if stem.is_empty() {
        return false;
    }

    match key {
        VideoKey::Episode(identity) => {
            parse_episode_identity(&stem).is_some_and(|parsed| &parsed == identity)
        }
        VideoKey::Movie { stem: video_stem } => {
            // زیرنویس دارای هویت قسمت برای فیلم پذیرفته نمی‌شود.
            parse_episode_identity(&stem).is_none() && movie_stem_matches(&stem, video_stem)
        }
    }
}

/// تطبیق سختگیرانهٔ نام فیلم: پیشوند کامل ویدیو + جداکنندهٔ واقعی.
/// الحاق دلخواه (مثل `Movie.2023X`) پذیرفته نمی‌شود.
fn movie_stem_matches(sub_stem: &str, video_stem: &str) -> bool {
    if !sub_matches_video_stem(sub_stem, video_stem) {
        return false;
    }
    let sub = sub_stem.to_lowercase();
    let video = video_stem.to_lowercase();
    match sub[video.len()..].chars().next() {
        None => true,
        Some(c) => c.is_whitespace() || matches!(c, '.' | '_' | '-' | '[' | '('),
    }
}

/// رتبهٔ کیفیت: ۱۰۸۰p > ۷۲۰p > سایر.
pub fn quality_of(archive_file_name: &str, member_path: &str) -> &'static str {
    let haystack = format!(
        "{}\n{}",
        archive_file_name.to_ascii_lowercase(),
        member_path.to_ascii_lowercase()
    );
    if haystack.contains("1080p") {
        "1080p"
    } else if haystack.contains("720p") {
        "720p"
    } else {
        "other"
    }
}

fn quality_rank(quality: &str) -> u8 {
    match quality {
        "1080p" => 3,
        "720p" => 2,
        _ => 1,
    }
}

/// مرتب‌سازی قطعی کاندیدها: کیفیت، سپس اندازهٔ بزرگ‌تر، سپس مسیر.
pub fn sort_candidates(candidates: &mut [SubtitleCandidate]) {
    candidates.sort_by(|a, b| {
        quality_rank(&b.quality)
            .cmp(&quality_rank(&a.quality))
            .then_with(|| b.size_bytes.cmp(&a.size_bytes))
            .then_with(|| a.member_path.cmp(&b.member_path))
            .then_with(|| a.archive_file_name.cmp(&b.archive_file_name))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> VideoKey {
        video_key(name, None, None)
    }

    #[test]
    fn zero_padding_is_normalized() {
        let k = key("Show.S01E05.1080p.mkv");
        assert!(member_matches_video(&k, "Show.S1E5.fa.srt"));
        assert!(member_matches_video(&k, "Show.S01E05.fa.srt"));
        assert!(!member_matches_video(&k, "Show.S01E15.fa.srt"));
    }

    #[test]
    fn e01_never_matches_e010() {
        let k = key("Show.S01E01.mkv");
        assert!(!member_matches_video(&k, "Show.S01E010.fa.srt"));
        assert!(member_matches_video(&k, "Show.S01E001.fa.srt"));
    }

    #[test]
    fn season_is_separate() {
        let k = key("Show.S02E05.mkv");
        assert!(member_matches_video(&k, "Show.S02E05.fa.srt"));
        assert!(!member_matches_video(&k, "Show.S01E05.fa.srt"));
    }

    #[test]
    fn x_form_is_supported() {
        let k = key("Show.2x05.mkv");
        assert!(member_matches_video(&k, "Show.2x05.fa.srt"));
        assert!(member_matches_video(&k, "Show.S02E05.fa.srt"));
        assert!(!member_matches_video(&k, "Show.2x15.fa.srt"));
    }

    #[test]
    fn multi_episode_subtitles_never_match_single_episode_video() {
        let k = key("Show.S01E01.mkv");
        assert!(!member_matches_video(&k, "Show.S01E01-12.fa.srt"));
        assert!(!member_matches_video(&k, "Show.S01E01-E04.fa.srt"));
        assert!(!member_matches_video(&k, "Show.S01E01E02.fa.srt"));

        // ویدیوی دو قسمتی فقط با همان بازهٔ دقیق
        let double = key("Show.S01E01E02.mkv");
        assert!(member_matches_video(&double, "Show.S01E01E02.fa.srt"));
        assert!(!member_matches_video(&double, "Show.S01E01.fa.srt"));
        assert!(!member_matches_video(&double, "Show.S01E01-03.fa.srt"));
    }

    #[test]
    fn malformed_range_subtitles_do_not_match_valid_videos() {
        let single = key("Show.S01E01.mkv");
        assert!(!member_matches_video(&single, "Show.S01E01-E1000.fa.srt"));
        assert!(!member_matches_video(&single, "Show.S01E01-E02E03.fa.srt"));
        assert!(!member_matches_video(&single, "Show.S01E01-E.fa.srt"));
        assert!(!member_matches_video(&single, "Show.S01E01-12E.fa.srt"));
        // پسوند متنی معتبر نباید نشانگر ناقص تلقی شود.
        assert!(member_matches_video(&single, "Show.S01E01-Extended.fa.srt"));

        let double = key("Show.S01E01E02.mkv");
        assert!(!member_matches_video(&double, "Show.S01E01E02-E.fa.srt"));
    }

    #[test]
    fn three_episode_sequences_are_never_truncated() {
        let double = key("Show.S01E01E02.mkv");
        assert!(!member_matches_video(&double, "Show.S01E01E02E03.fa.srt"));

        let triple = key("Show.S01E01E02E03.mkv");
        assert!(member_matches_video(&triple, "Show.S01E01E02E03.fa.srt"));
        assert!(member_matches_video(&triple, "Show.S1E1E2E3.fa.srt"));
        assert!(!member_matches_video(&triple, "Show.S01E01E02.fa.srt"));
    }

    #[test]
    fn explicit_sequences_do_not_equal_continuous_ranges() {
        let explicit = key("Show.S01E01E12.mkv");
        assert!(!member_matches_video(&explicit, "Show.S01E01-12.fa.srt"));
        assert!(member_matches_video(&explicit, "Show.S1E1E12.fa.srt"));

        let range = key("Show.S01E01-12.mkv");
        assert!(member_matches_video(&range, "Show.S1E1-12.fa.srt"));
        assert!(member_matches_video(&range, "Show.S01E01-E12.fa.srt"));
        assert!(!member_matches_video(&range, "Show.S01E01E12.fa.srt"));
    }

    #[test]
    fn scanner_season_episode_is_fallback_when_name_has_no_pattern() {
        let k = video_key("Episode 5.mkv", Some(3), Some(5));
        assert!(member_matches_video(&k, "Show.S03E05.fa.srt"));
        assert!(!member_matches_video(&k, "Show.S03E06.fa.srt"));
    }

    #[test]
    fn movie_uses_strict_prefix_match() {
        let k = key("Movie.2023.1080p.mkv");
        // نام زیرنویس باید با نام کامل ویدیو شروع شود — بدون حدس فازی
        assert!(member_matches_video(&k, "Movie.2023.1080p.fa.ass"));
        assert!(!member_matches_video(&k, "Movie.2023.fa.srt"));
        assert!(member_matches_video(&k, "Movie.2023.1080p.Extended.fa.srt"));
        assert!(!member_matches_video(&k, "Movie.2023.1080pX.fa.srt"));
        assert!(!member_matches_video(&k, "Movie.20230.fa.srt"));
        // زیرنویس قسمت‌دار برای فیلم پذیرفته نمی‌شود
        assert!(!member_matches_video(&k, "Movie.2023.1080p.S01E01.fa.srt"));
        // پسوند غیرزیرنویس
        assert!(!member_matches_video(&k, "Movie.2023.1080p.fa.txt"));
    }

    #[test]
    fn ranking_prefers_1080_then_720_then_size() {
        let mut candidates = vec![
            SubtitleCandidate {
                archive_file_name: "a.zip".into(),
                member_path: "Show.S01E01.fa.srt".into(),
                subtitle_extension: "srt".into(),
                size_bytes: 900,
                quality: "other".into(),
            },
            SubtitleCandidate {
                archive_file_name: "b.zip".into(),
                member_path: "Show.S01E01.720p.fa.srt".into(),
                subtitle_extension: "srt".into(),
                size_bytes: 100,
                quality: "720p".into(),
            },
            SubtitleCandidate {
                archive_file_name: "c.zip".into(),
                member_path: "Show.S01E01.1080p.fa.srt".into(),
                subtitle_extension: "srt".into(),
                size_bytes: 50,
                quality: "1080p".into(),
            },
            SubtitleCandidate {
                archive_file_name: "d.zip".into(),
                member_path: "Show.S01E01.1080p.alt.fa.srt".into(),
                subtitle_extension: "srt".into(),
                size_bytes: 80,
                quality: "1080p".into(),
            },
        ];
        sort_candidates(&mut candidates);
        assert_eq!(candidates[0].archive_file_name, "d.zip"); // 1080p بزرگ‌تر
        assert_eq!(candidates[1].archive_file_name, "c.zip");
        assert_eq!(candidates[2].archive_file_name, "b.zip");
        assert_eq!(candidates[3].archive_file_name, "a.zip");
    }

    #[test]
    fn quality_detection_uses_archive_and_member_names() {
        assert_eq!(quality_of("Show.S01.1080p.zip", "x.srt"), "1080p");
        assert_eq!(quality_of("Show.S01.zip", "x.720p.srt"), "720p");
        assert_eq!(quality_of("Show.S01.zip", "x.srt"), "other");
    }
}
