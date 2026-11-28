//! لایهٔ سیاست تصمیم pipeline — قلب حالت «کاملاً خودکار».
//!
//! قرارداد: هیچ‌وقت امنیت را دور نمی‌زند.
//! - `Ask`: حالت تعاملی — همان مانع تصمیم فعلی (رفتار حفظ‌شده).
//! - `Auto`: انتخابِ بااطمینان؛ engine انتخاب را با **همان revalidation مسیر
//!   تعاملی** ادامه می‌دهد (گزینهٔ خودکار هیچ امتیاز ویژه‌ای ندارد).
//! - `NeedsReview`: اطمینان کافی نیست — آیتم به‌جای حدس، «نیازمند بررسی» می‌شود.
//! - تعارض/بازنویسی هرگز سیاست‌گذاری نمی‌شود: مسیرهای no-overwrite دست‌نخورده‌اند.
//!
//! این ماژول مستقل از Tauri است؛ فقط probe ابزارها از state می‌گیرد.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::archive::types::SubtitleCandidate;
use crate::subtitle::scoring::{self, ScoredCandidate};

// ---------------------------------------------------------------------
// انواع تصمیم
// ---------------------------------------------------------------------

/// انتخاب خودکار یک Track — engine آن را با revalidation مسیر تعاملی ادامه می‌دهد.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TrackAutoPick {
    pub stream_index: u32,
    pub reasons: Vec<String>,
}

/// تصمیم سیاست برای مانع‌های انتخاب Track (D1/D2).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TrackDecision {
    Auto(TrackAutoPick),
    /// اطمینان کافی نیست → نیازمند بررسی
    NeedsReview {
        reason: String,
    },
    /// فقط در مسیر review (D2): هیچ شواهد فارسی در هیچ Track نیست — معادل
    /// «بررسی محتوا فارسی تأیید نکرد» مسیر تعاملی → skip صادقانه.
    NoPersianEvidence {
        reason: String,
    },
    /// حالت تعاملی — مانع تصمیم کاربر مثل سابق
    Ask,
}

/// تصمیم سیاست برای مانع کاندیدای آرشیو (D3) — بدون re-extraction:
/// کاندیداها از قبل validate و رتبه‌بندی کیفیت شده‌اند.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ArchiveDecision {
    Auto {
        member: SubtitleCandidate,
        reasons: Vec<String>,
    },
    NeedsReview {
        reason: String,
    },
    Ask,
}

/// تصمیم سیاست برای مانع انتخاب sidecar (D4).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SidecarDecision {
    Auto { path: PathBuf, reasons: Vec<String> },
    NeedsReview { reason: String },
    Ask,
}

// ---------------------------------------------------------------------
// D1/D2 — انتخاب Track با امتیازدهی محتوا
// ---------------------------------------------------------------------

/// تصمیم بر اساس امتیازهای از پیش محاسبه‌شده — pure و قابل تست.
///
/// `no_persian_is_skip`: در D2 (بررسی Track بی‌نشانه) هیچ شواهد فارسی =
/// همان «تأیید نشد» مسیر تعاملی → skip؛ در D1 (Trackهای tag-خورده) همان
/// وضعیت مشکوک‌تر است (tag دروغ؟) → NeedsReview صادقانه.
pub(crate) fn decide_track(
    auto: bool,
    scores: &[ScoredCandidate<u32>],
    no_persian_is_skip: bool,
) -> TrackDecision {
    if !auto {
        return TrackDecision::Ask;
    }
    match scoring::pick(scores) {
        scoring::PickOutcome::Best { index, reasons } => {
            let stream_index = scores[index].id;
            TrackDecision::Auto(TrackAutoPick {
                stream_index,
                reasons,
            })
        }
        scoring::PickOutcome::Uncertain { reason } => TrackDecision::NeedsReview { reason },
        scoring::PickOutcome::NoPersianEvidence { reason } => {
            if no_persian_is_skip {
                TrackDecision::NoPersianEvidence { reason }
            } else {
                TrackDecision::NeedsReview { reason }
            }
        }
    }
}

// ---------------------------------------------------------------------
// D3 — کاندیدای آرشیو با قانون tier کیفیت (بدون استخراج مجدد)
// ---------------------------------------------------------------------

/// tierهای کیفیت — همان ترتیب deterministic رتبه‌بندی `archive::matching`.
const QUALITY_TIERS: [&str; 3] = ["1080p", "720p", "other"];

/// قانون انتخاب خودکار آرشیو: فقط وقتی **دقیقاً یک** کاندیدا در tier کیفیتِ
/// برتر باشد؛ هم‌سطح‌ها یعنی واقعاً مبهم → NeedsReview. این قانون همان
/// ترتیب preference موجود (`sort_candidates`) است — هیچ سیاست تازه‌ای
/// وارد بازی کیفیت نمی‌شود.
pub(crate) fn decide_archive(auto: bool, candidates: &[SubtitleCandidate]) -> ArchiveDecision {
    if !auto {
        return ArchiveDecision::Ask;
    }
    let Some(top_tier) = QUALITY_TIERS
        .iter()
        .find(|tier| candidates.iter().any(|c| c.quality == **tier))
    else {
        return ArchiveDecision::NeedsReview {
            reason: "هیچ tier کیفیتی قابل تشخیص نبود".into(),
        };
    };
    let top: Vec<&SubtitleCandidate> = candidates
        .iter()
        .filter(|c| c.quality == *top_tier)
        .collect();
    if top.len() == 1 {
        let member = top[0].clone();
        let reasons = vec![
            format!("تنها کاندیدای tier کیفیت برتر ({top_tier})"),
            format!("منبع: {}", member.archive_file_name),
        ];
        ArchiveDecision::Auto { member, reasons }
    } else {
        ArchiveDecision::NeedsReview {
            reason: format!(
                "{} کاندیدای هم‌سطح کیفیت {top_tier} — انتخاب خودکار معنا ندارد",
                top.len()
            ),
        }
    }
}

// ---------------------------------------------------------------------
// D4 — sidecarهای هم‌نام با امتیازدهی محتوای فایل
// ---------------------------------------------------------------------

pub(crate) fn decide_sync_sidecar(
    auto: bool,
    scores: &[ScoredCandidate<PathBuf>],
) -> SidecarDecision {
    if !auto {
        return SidecarDecision::Ask;
    }
    match scoring::pick(scores) {
        scoring::PickOutcome::Best { index, reasons } => SidecarDecision::Auto {
            path: scores[index].id.clone(),
            reasons,
        },
        // NoPersianEvidence اینجا ممکن نیست (sidecarها از قبل validate فارسی
        // شده‌اند) — اگر رخ داد هم صادقانه NeedsReview، نه حدس
        scoring::PickOutcome::Uncertain { reason }
        | scoring::PickOutcome::NoPersianEvidence { reason } => {
            SidecarDecision::NeedsReview { reason }
        }
    }
}

// ---------------------------------------------------------------------
// probe و امتیازدهی Trackهای داخلی — orchestration با workspace موقت
// ---------------------------------------------------------------------

#[derive(Debug)]
pub(crate) enum ProbeError {
    Cancelled,
    Failed(String),
}

/// probe workspace — یکتا زیر temp؛ همهٔ فایل‌های استخراج فقط داخل آن.
struct TrackProbeWorkspace {
    root: PathBuf,
}

const PROBE_PREFIX: &str = "subsync-trackscore-";

impl TrackProbeWorkspace {
    fn create() -> Result<Self, ProbeError> {
        let token = crate::subtitle::extraction::unique_probe_token();
        let root = std::env::temp_dir().join(format!("{PROBE_PREFIX}{token}"));
        std::fs::create_dir_all(&root)
            .map_err(|e| ProbeError::Failed(format!("ساخت workspace امتیازدهی Track: {e}")))?;
        Ok(TrackProbeWorkspace { root })
    }

    /// پاک‌سازی کامل — هرگز panic نمی‌کند؛ خطاها لاگ می‌شوند.
    fn remove(&self) {
        if let Err(e) = std::fs::remove_dir_all(&self.root) {
            crate::logging::warn(
                "pipeline",
                &format!("پاک‌سازی workspace امتیازدهی Track ناموفق بود: {e}"),
            );
        }
    }
}

/// استخراج و امتیازدهی Trackهای متنی مشخص به‌صورت موقت — هیچ فایل دائمی
/// ساخته نمی‌شود و ورودی‌ها دست‌نخورده می‌مانند. کاندیداهای غیرقابل‌تحلیل
/// (خراب/قابل استخراج نبودن) حذف می‌شوند؛ اگر همه حذف شوند، خروجی خالی است
/// و `pick` صادقانه Uncertain/NoPersianEvidence می‌دهد.
pub(crate) fn probe_and_score_tracks(
    state: &crate::state::AppState,
    video_path: &Path,
    stream_indexes: &[(u32, bool)],
    cancel: &AtomicBool,
) -> Result<Vec<ScoredCandidate<u32>>, ProbeError> {
    if cancel.load(Ordering::SeqCst) {
        return Err(ProbeError::Cancelled);
    }
    let ffmpeg =
        crate::commands::media::resolve_tool(state, "ffmpeg").map_err(ProbeError::Failed)?;
    // طول ویدیو برای متریک coverage — نبودش فقط fallback به شمارش cue است
    let duration: Option<f64> = crate::commands::media::resolve_tool(state, "ffprobe")
        .ok()
        .and_then(|ffprobe| {
            crate::media::ffprobe::probe_duration(&ffprobe, video_path, cancel)
                .ok()
                .flatten()
        });

    let workspace = TrackProbeWorkspace::create()?;
    let result = (|| {
        let mut candidates = Vec::new();
        for (stream_index, tagged_fa) in stream_indexes {
            if cancel.load(Ordering::SeqCst) {
                return Err(ProbeError::Cancelled);
            }
            let extracted = match crate::subtitle::extraction::extract_track_to_temp(
                &ffmpeg,
                video_path,
                *stream_index,
                &workspace.root,
                cancel,
            ) {
                Ok((path, _validation)) => path,
                Err(crate::subtitle::extraction::ExtractError::Cancelled) => {
                    return Err(ProbeError::Cancelled)
                }
                // Track خراب/غیرقابل استخراج = کاندیدای ردشده، نه شکست probe
                Err(_) => continue,
            };
            let bytes = match std::fs::read(&extracted) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            let Some(score) = scoring::score_bytes(&bytes, "srt", duration, *tagged_fa) else {
                continue;
            };
            candidates.push(ScoredCandidate {
                id: *stream_index,
                score,
            });
        }
        Ok(candidates)
    })();
    workspace.remove();
    result
}

/// امتیازدهی sidecarهای موجود — فقط خواندن؛ هیچ فایلی تغییر نمی‌کند.
pub(crate) fn score_sidecar_files(
    paths: &[PathBuf],
    duration: Option<f64>,
) -> Vec<ScoredCandidate<PathBuf>> {
    paths
        .iter()
        .filter_map(|path| {
            let bytes = std::fs::read(path).ok()?;
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default();
            // sidecarهای معتبر فارسی همگی نشانهٔ fa در نام دارند (شرط
            // inspect_persian_sidecars) → tagged یکسان، بدون اثر بر رتبه
            let score = scoring::score_bytes(&bytes, &ext, duration, true)?;
            Some(ScoredCandidate {
                id: path.clone(),
                score,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subtitle::scoring::TrackScore;

    fn candidate(
        id: u32,
        score_value: f64,
        cue_count: usize,
        persian: usize,
    ) -> ScoredCandidate<u32> {
        ScoredCandidate {
            id,
            score: TrackScore {
                score: score_value,
                metrics: crate::subtitle::scoring::TrackMetrics {
                    cue_count,
                    persian_chars: persian,
                    persian_density: 0.9,
                    empty_ratio: 0.0,
                    duplicate_ratio: 0.0,
                    short_ratio: 0.0,
                    coverage: Some(0.9),
                    tagged_fa: true,
                },
                reasons: vec!["تست".into()],
            },
        }
    }

    #[test]
    fn interactive_mode_always_asks() {
        let scores = vec![candidate(2, 0.9, 100, 500), candidate(3, 0.2, 100, 500)];
        assert_eq!(decide_track(false, &scores, false), TrackDecision::Ask);
        assert_eq!(decide_track(false, &scores, true), TrackDecision::Ask);
        assert_eq!(decide_sync_sidecar(false, &[]), SidecarDecision::Ask);
    }

    #[test]
    fn auto_mode_picks_clear_winner() {
        let scores = vec![candidate(2, 0.9, 100, 500), candidate(3, 0.2, 100, 500)];
        match decide_track(true, &scores, false) {
            TrackDecision::Auto(pick) => assert_eq!(pick.stream_index, 2),
            other => panic!("انتظار Auto، دریافت: {other:?}"),
        }
    }

    #[test]
    fn auto_mode_close_scores_become_needs_review() {
        let scores = vec![candidate(2, 0.70, 100, 500), candidate(3, 0.65, 100, 500)];
        match decide_track(true, &scores, false) {
            TrackDecision::NeedsReview { reason } => {
                assert!(reason.contains("هم‌امتیاز"), "{reason}");
            }
            other => panic!("انتظار NeedsReview، دریافت: {other:?}"),
        }
    }

    #[test]
    fn no_persian_evidence_maps_by_barrier_kind() {
        let empty = vec![candidate(2, 0.0, 100, 0)];
        // D1: tag دروغین؟ — مشکوک، نه skip
        assert!(matches!(
            decide_track(true, &empty, false),
            TrackDecision::NeedsReview { .. }
        ));
        // D2: هیچ فارسی نیست = همان «تأیید نشد» تعاملی → skip
        assert!(matches!(
            decide_track(true, &empty, true),
            TrackDecision::NoPersianEvidence { .. }
        ));
    }

    #[test]
    fn archive_unique_top_quality_tier_is_auto() {
        let mk = |quality: &str, member: &str| SubtitleCandidate {
            archive_file_name: "a.zip".into(),
            member_path: member.into(),
            subtitle_extension: "srt".into(),
            size_bytes: 1000,
            quality: quality.into(),
        };
        let candidates = vec![mk("1080p", "good.srt"), mk("720p", "low.srt")];
        match decide_archive(true, &candidates) {
            ArchiveDecision::Auto { member, .. } => {
                assert_eq!(member.member_path, "good.srt");
            }
            other => panic!("انتظار Auto، دریافت: {other:?}"),
        }
    }

    #[test]
    fn archive_same_tier_tie_is_needs_review() {
        let mk = |quality: &str, member: &str| SubtitleCandidate {
            archive_file_name: "a.zip".into(),
            member_path: member.into(),
            subtitle_extension: "srt".into(),
            size_bytes: 1000,
            quality: quality.into(),
        };
        let candidates = vec![mk("1080p", "a.srt"), mk("1080p", "b.srt")];
        match decide_archive(true, &candidates) {
            ArchiveDecision::NeedsReview { reason } => {
                assert!(reason.contains("هم‌سطح"), "{reason}");
            }
            other => panic!("انتظار NeedsReview، دریافت: {other:?}"),
        }
    }

    #[test]
    fn sidecar_pick_uses_scoring() {
        let mk = |path: u32, score_value: f64| ScoredCandidate {
            id: PathBuf::from(format!("sub-{path}.fa.srt")),
            score: TrackScore {
                score: score_value,
                metrics: crate::subtitle::scoring::TrackMetrics {
                    cue_count: 100,
                    persian_chars: 500,
                    persian_density: 0.9,
                    empty_ratio: 0.0,
                    duplicate_ratio: 0.0,
                    short_ratio: 0.0,
                    coverage: Some(0.9),
                    tagged_fa: true,
                },
                reasons: vec!["تست".into()],
            },
        };
        let scores = vec![mk(1, 0.9), mk(2, 0.3)];
        match decide_sync_sidecar(true, &scores) {
            SidecarDecision::Auto { path, .. } => {
                assert_eq!(path, PathBuf::from("sub-1.fa.srt"));
            }
            other => panic!("انتظار Auto، دریافت: {other:?}"),
        }
    }
}
