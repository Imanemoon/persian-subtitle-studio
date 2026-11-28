//! امتیازدهی Trackهای زیرنویس فارسی — سیاست «انتخاب خودکارِ شفاف».
//!
//! هدف: بین چند کاندیدای زیرنویس فارسی، یک برندهٔ **قابل‌توضیح و قطعی** پیدا
//! کند یا صادقانه بگوید «مطمئن نیستم». اصل طراحی (مطابق ممیزی فاز ۰):
//! - tag فارسی به‌تنهایی ارزش ندارد؛ محتوا ملاک است.
//! - تعداد حروف فارسی به‌تنهایی هم ارزش ندارد؛ تراکم/پوشش/سلامت cue می‌آید.
//! - امتیاز ۰..۱ فقط برای **ترتیب‌دهی** است — احتمال کالیبره‌شده نیست و
//!   هیچ‌جا به‌عنوان درصد اطمینان نمایش داده نمی‌شود.
//! - کاندیداهای نزدیک به هم یا کم‌شواهد → `Uncertain` (نیازمند بررسی)،
//!   هرگز حدسِ بی‌دلیل.
//!
//! این ماژول pure است (بدون I/O و بدون Tauri) و فقط متن decode‌شده مصرف می‌کند.

use crate::models::SubtitleValidation;
use crate::subtitle::validation::{analyze_srt_like, CueStats};

// ---------------------------------------------------------------------
// آستانه‌ها — عمداً ثابت کد (نه تنظیمات کاربر): امتیاز کالیبره نیست و
// عدد بی‌معنی برای کاربر فقط تنظیمات را پیچیده می‌کند. مستند در README.
// ---------------------------------------------------------------------

/// حداقل امتیاز مطلق برای انتخاب خودکار — زیر این، حتی تک‌کاندیدا انتخاب نمی‌شود.
pub(crate) const MIN_SELECT_SCORE: f64 = 0.55;
/// حداقل فاصلهٔ امتیاز برنده با نفر دوم — کمتر از این = «تقریباً برابر».
pub(crate) const MIN_MARGIN: f64 = 0.12;
/// حداقل شواهد فارسی مطلق (کاراکتر) — زیر این «فارسی نیست» نتیجه گرفته می‌شود
/// (هم‌خوان با گیت تبدیل cp1256 در encoding.rs).
pub(crate) const MIN_PERSIAN_CHARS: usize = 10;
/// حداقل تراکم فارسی معنادار — زیر این، حتی با tag فارسی، شواهد ضعیف است.
pub(crate) const MIN_PERSIAN_DENSITY: f64 = 0.2;
/// حداقل تعداد cue برای «Track گفتگو» — زیرنویس واقعی فیلم/سریال ده‌ها تا صدها
/// cue دارد؛ چند cue پراکنده (تیتراژ/تبلیغ) هرگز به‌صورت خودکار انتخاب نمی‌شود.
pub(crate) const MIN_CUES_FOR_SELECT: usize = 10;

// ---------------------------------------------------------------------
// متریک‌ها و امتیاز
// ---------------------------------------------------------------------

/// متریک‌های خام یک کاندیدا — برای دلایل قابل‌نمایش و تست‌ها.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackMetrics {
    /// تعداد cue معتبر
    pub cue_count: usize,
    /// کاراکترهای فارسی
    pub persian_chars: usize,
    /// تراکم فارسی = حروف فارسی / کل کاراکترهای غیرفاصلهٔ متن cueها
    pub persian_density: f64,
    /// نسبت cue خالی/بدون متن
    pub empty_ratio: f64,
    /// نسبت cue متنِ تکراری
    pub duplicate_ratio: f64,
    /// نسبت cue خیلی کوتاه (نشانهٔ تبلیغ/لینک)
    pub short_ratio: f64,
    /// پوشش زمانی نسبت به طول ویدیو — بدون طول ویدیو `None`
    pub coverage: Option<f64>,
    /// آیا tag/metadata نشانهٔ فارسی دارد
    pub tagged_fa: bool,
}

impl TrackMetrics {
    fn from_cue_stats(stats: &CueStats, video_duration_secs: Option<f64>, tagged_fa: bool) -> Self {
        let persian_density = if stats.text_chars == 0 {
            0.0
        } else {
            stats.persian_chars as f64 / stats.text_chars as f64
        };
        let cue_count = stats.cue_count;
        let empty_ratio = ratio(stats.empty_cue_count, cue_count);
        let coverage = match (video_duration_secs, stats.first_start_ms, stats.last_end_ms) {
            (Some(duration), Some(start), Some(end)) if duration > 0.0 => {
                let covered = (end.saturating_sub(start)) as f64 / 1000.0;
                Some((covered / duration).clamp(0.0, 1.0))
            }
            _ => None,
        };
        TrackMetrics {
            cue_count,
            persian_chars: stats.persian_chars,
            persian_density,
            empty_ratio,
            duplicate_ratio: stats.duplicate_ratio,
            short_ratio: stats.short_ratio,
            coverage,
            tagged_fa,
        }
    }
}

fn ratio(part: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 / total as f64
    }
}

/// نتیجهٔ امتیازدهی یک کاندیدا — امتیاز + دلایل انسانی (فارسی، قابل‌نمایش).
#[derive(Debug, Clone, PartialEq)]
pub struct TrackScore {
    /// ۰..۱ — فقط برای ترتیب‌دهی؛ احتمال/درصد اطمینان نیست.
    pub score: f64,
    pub metrics: TrackMetrics,
    pub reasons: Vec<String>,
}

/// وزن‌ها — مجموع ۱.۰۰؛ هر تغییر باید با تست‌های امتیازدهی هم‌راستا شود.
const W_DENSITY: f64 = 0.35;
const W_COVERAGE: f64 = 0.25;
const W_HEALTH: f64 = 0.20;
const W_SANITY: f64 = 0.10;
const W_TAG: f64 = 0.10;

/// امتیازدهی یک کاندیدا از روی متریک‌های آماده — pure.
pub fn score_metrics(metrics: TrackMetrics) -> TrackScore {
    let mut reasons = Vec::new();

    // ۱) تراکم فارسی — ۰٫۶۰ به بالا نمرهٔ کامل (متن فارسی واقعی معمولاً >۰٫۵ است)
    let density_n = (metrics.persian_density / 0.6).clamp(0.0, 1.0);
    reasons.push(format!(
        "تراکم فارسی {:.0}٪",
        metrics.persian_density * 100.0
    ));

    // ۲) پوشش — با طول ویدیو؛ بدون آن بر اساس تعداد cue (زیرنویس کامل سریال ~۲۰۰-۸۰۰ cue)
    let coverage_n = match metrics.coverage {
        Some(coverage) => {
            reasons.push(format!("پوشش زمانی {:.0}٪", coverage * 100.0));
            (coverage / 0.8).clamp(0.0, 1.0)
        }
        None => {
            reasons.push(format!(
                "طول ویدیو نامعلوم — بر اساس {} cue",
                metrics.cue_count
            ));
            (metrics.cue_count as f64 / 250.0).clamp(0.0, 1.0)
        }
    };

    // ۳) سلامت cue — خالی/تکراری/کوتاه جریمه می‌شوند (نشانهٔ تبلیغات یا Track خراب)
    let health_penalty = metrics.empty_ratio + metrics.duplicate_ratio + metrics.short_ratio * 0.5;
    let health_n = (1.0 - health_penalty).clamp(0.0, 1.0);
    if metrics.empty_ratio > 0.05 {
        reasons.push(format!(
            "{}٪ cue خالی",
            (metrics.empty_ratio * 100.0) as u64
        ));
    }
    if metrics.duplicate_ratio > 0.05 {
        reasons.push(format!(
            "{}٪ cue تکراری (نشانهٔ تبلیغ/کپی)",
            (metrics.duplicate_ratio * 100.0) as u64
        ));
    }
    if metrics.short_ratio > 0.15 {
        reasons.push(format!(
            "{}٪ cue خیلی کوتاه (نشانهٔ تبلیغ/لینک)",
            (metrics.short_ratio * 100.0) as u64
        ));
    }

    // ۴) سلامت طول متن cue — باند معقول؛ خیلی کوتاه یا خیلی بلند جریمه
    let sanity_n = sanity_score(&metrics);

    // ۵) tag — بونوس کوچک؛ tag هرگز جای محتوا را نمی‌گیرد (وزن فقط ۰٫۱)
    let tag_n = if metrics.tagged_fa { 1.0 } else { 0.0 };

    let score = (W_DENSITY * density_n
        + W_COVERAGE * coverage_n
        + W_HEALTH * health_n
        + W_SANITY * sanity_n
        + W_TAG * tag_n)
        .clamp(0.0, 1.0);

    TrackScore {
        score,
        metrics,
        reasons,
    }
}

/// نمرهٔ سلامت طول متن cue — cueهای خیلی کوتاه (تبلیغ/لینک) جریمه می‌شوند.
fn sanity_score(metrics: &TrackMetrics) -> f64 {
    if metrics.cue_count == 0 {
        return 0.0;
    }
    (1.0 - metrics.short_ratio * 2.0).clamp(0.0, 1.0)
}

/// امتیازدهی از روی بایت‌های یک کاندیدا — decode امن + آمار cue.
/// خروجی `None` = فایل قابل تحلیل نیست (باینری/encoding نامشخص) → کاندیدا رد.
pub fn score_bytes(
    bytes: &[u8],
    ext: &str,
    video_duration_secs: Option<f64>,
    tagged_fa: bool,
) -> Option<TrackScore> {
    let text = crate::subtitle::validation::decode_for_analysis(bytes).ok()?;
    let normalized_ext = ext.trim_start_matches('.').to_ascii_lowercase();
    let metrics = if matches!(normalized_ext.as_str(), "srt" | "vtt" | "") {
        TrackMetrics::from_cue_stats(&analyze_srt_like(&text), video_duration_secs, tagged_fa)
    } else {
        // ASS/SSA/MicroDVD: فقط متریک‌های سطح فایل (بدون coverage per-cue)
        let validation: SubtitleValidation =
            crate::subtitle::validation::validate_bytes_for_extension(bytes, &normalized_ext);
        if !validation.valid {
            return None;
        }
        TrackMetrics {
            cue_count: validation.cue_count,
            persian_chars: validation.persian_char_count,
            persian_density: {
                let total: usize = text.chars().filter(|c| !c.is_whitespace()).count();
                if total == 0 {
                    0.0
                } else {
                    validation.persian_char_count as f64 / total as f64
                }
            },
            empty_ratio: 0.0,
            duplicate_ratio: 0.0,
            short_ratio: 0.0,
            coverage: None,
            tagged_fa,
        }
    };
    Some(score_metrics(metrics))
}

/// امتیازدهی از روی متن decode‌شده (SRT/VTT) — برای probeهای استخراج‌شده.
pub fn score_text(text: &str, video_duration_secs: Option<f64>, tagged_fa: bool) -> TrackScore {
    score_metrics(TrackMetrics::from_cue_stats(
        &analyze_srt_like(text),
        video_duration_secs,
        tagged_fa,
    ))
}

// ---------------------------------------------------------------------
// انتخاب قطعی
// ---------------------------------------------------------------------

/// نتیجهٔ انتخاب قطعی بین کاندیداهای امتیازگرفته.
#[derive(Debug, Clone, PartialEq)]
pub enum PickOutcome {
    /// برندهٔ بااطمینان — index در آرایهٔ ورودی + دلایل انتخاب.
    Best { index: usize, reasons: Vec<String> },
    /// کاندیدا هست اما اطمینان کافی نیست → نیازمند بررسی.
    Uncertain { reason: String },
    /// هیچ شواهد فارسی معناداری در هیچ کاندیدا نیست → معادل مسیر skip تعاملی.
    NoPersianEvidence { reason: String },
}

/// یک کاندیدای امتیازگرفته — `id` هر نوع قطعیِ قابل‌مقایسه (stream index، مسیر…).
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredCandidate<I: Clone + Ord> {
    pub id: I,
    pub score: TrackScore,
}

/// انتخاب قطعی بین کاندیداها:
/// ۱) فقط کاندیداهای با شواهد فارسی واقعی وارد مسابقه می‌شوند؛ بدون هیچ → NoPersianEvidence.
/// ۲) برنده = بیشترین امتیاز؛ tie-break قطعی با `id` صعودی.
/// ۳) انتخاب فقط با امتیاز مطلق کافی + فاصلهٔ کافی از نفر دوم؛ وگرنه Uncertain.
pub fn pick<I: Clone + Ord>(candidates: &[ScoredCandidate<I>]) -> PickOutcome {
    if candidates.is_empty() {
        return PickOutcome::Uncertain {
            reason: "کاندیدایی برای امتیازدهی وجود نداشت".into(),
        };
    }

    let evidence: Vec<&ScoredCandidate<I>> = candidates
        .iter()
        .filter(|c| {
            c.score.metrics.persian_chars >= MIN_PERSIAN_CHARS
                && c.score.metrics.persian_density >= MIN_PERSIAN_DENSITY
        })
        .collect();
    if evidence.is_empty() {
        return PickOutcome::NoPersianEvidence {
            reason: format!(
                "هیچ کاندیدایی شواهد فارسی معنادار ندارد (حداقل {} حرف فارسی با تراکم {:.0}٪)",
                MIN_PERSIAN_CHARS,
                MIN_PERSIAN_DENSITY * 100.0
            ),
        };
    }

    // مرتب‌سازی قطعی: امتیاز نزولی، سپس id صعودی (tie-break پایدار)
    let mut sorted = evidence;
    sorted.sort_by(|a, b| {
        b.score
            .score
            .partial_cmp(&a.score.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    let best = &sorted[0];

    if best.score.metrics.cue_count < MIN_CUES_FOR_SELECT {
        return PickOutcome::Uncertain {
            reason: format!(
                "برندهٔ احتمالی فقط {} cue دارد (حداقل {} برای انتخاب خودکار) — شواهد گفتگو کافی نیست",
                best.score.metrics.cue_count,
                MIN_CUES_FOR_SELECT
            ),
        };
    }

    if best.score.score < MIN_SELECT_SCORE {
        return PickOutcome::Uncertain {
            reason: format!(
                "قوی‌ترین کاندیدا امتیاز کافی ندارد ({:.2} از حداقل {:.2}) — {}",
                best.score.score,
                MIN_SELECT_SCORE,
                best.score.reasons.join("؛ ")
            ),
        };
    }

    if sorted.len() > 1 {
        let second = &sorted[1];
        let margin = best.score.score - second.score.score;
        if margin < MIN_MARGIN {
            return PickOutcome::Uncertain {
                reason: format!(
                    "دو کاندیدای برتر تقریباً هم‌امتیازند ({:.2} در برابر {:.2} — فاصله {:.2} < {:.2})",
                    best.score.score,
                    second.score.score,
                    margin,
                    MIN_MARGIN
                ),
            };
        }
    }

    let mut reasons = best.score.reasons.clone();
    reasons.push(format!(
        "امتیاز {:.2} با فاصلهٔ معنادار از بقیه",
        best.score.score
    ));
    PickOutcome::Best {
        index: candidates.iter().position(|c| c.id == best.id).unwrap_or(0),
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ساخت SRT از (شروع ثانیه‌ای، متن‌ها) — هر cue پنج ثانیه طول دارد.
    fn srt(cues: &[(u32, String)]) -> String {
        let mut out = String::new();
        for (i, (base, text)) in cues.iter().enumerate() {
            let end = base + 5;
            out.push_str(&format!(
                "{}\n00:{:02}:{:02},000 --> 00:{:02}:{:02},000\n{}\n\n",
                i + 1,
                (base % 3600) / 60,
                base % 60,
                (end % 3600) / 60,
                end % 60,
                text
            ));
        }
        out
    }

    #[test]
    fn dialogue_track_scores_high_and_ads_score_low() {
        // دیالوگ واقعی: ۴۰ cue متن بلندِ متنوع فارسی
        let dialogue_cues: Vec<(u32, String)> = (0..40u32)
            .map(|i| {
                (
                    i * 10,
                    format!("جملهٔ گفتگوی شمارهٔ {i} با متن کامل و طبیعی فارسی برای صحنهٔ فیلم"),
                )
            })
            .collect();
        let dialogue = score_text(&srt(&dialogue_cues), Some(600.0), true);

        // Track تبلیغاتی: cueهای کوتاه تکراری با لینک
        let ad_cues: Vec<(u32, String)> = (0..40u32)
            .map(|i| (i * 10, "t.me/xyz Ads Group".to_string()))
            .collect();
        let ad_track = score_text(&srt(&ad_cues), Some(600.0), true);

        assert!(
            dialogue.score > ad_track.score + MIN_MARGIN,
            "دیالوگ {:.2} باید با فاصله از تبلیغ {:.2} جلو بزند",
            dialogue.score,
            ad_track.score
        );
        assert!(dialogue.metrics.persian_density > 0.8);
        assert!(ad_track.metrics.duplicate_ratio > 0.8);
    }

    #[test]
    fn empty_track_has_no_persian_evidence() {
        let cues: Vec<(u32, String)> = (0..10u32).map(|i| (i * 10, String::new())).collect();
        let score = score_text(&srt(&cues), Some(600.0), true);
        assert_eq!(score.metrics.persian_chars, 0);
        assert_eq!(score.metrics.empty_ratio, 1.0);

        let picked = pick(&[ScoredCandidate { id: 2u32, score }]);
        assert!(matches!(picked, PickOutcome::NoPersianEvidence { .. }));
    }

    #[test]
    fn malformed_text_fails_decode_not_scored() {
        // بایت‌های نامعتبر UTF-8 — decode_for_analysis رد می‌کند
        let bytes = [0xC7, 0xE1, 0xE1, 0xE5, 0x0A];
        assert!(score_bytes(&bytes, "srt", None, true).is_none());
    }

    #[test]
    fn misleading_tag_gets_small_bonus_only() {
        // tag فارسی با محتوای لاتین — بونوس tag نمی‌تواند از محتوای فارسی عبور کند
        let latin_tagged = score_text(
            &srt(&[(0, "Just latin dialogue text here".into())]),
            Some(600.0),
            true,
        );
        let persian_untagged = score_text(
            &srt(&[(0, "متن فارسی کامل و طبیعی برای گفتگو".into())]),
            Some(600.0),
            false,
        );
        assert!(persian_untagged.score > latin_tagged.score);
        assert!(latin_tagged.score < MIN_SELECT_SCORE);
    }

    #[test]
    fn two_similar_candidates_become_uncertain_not_guess() {
        // دو نسخه با ساختار کاملاً متقارن («اول» ↔ «دوم») → امتیاز دقیقاً برابر
        let a_cues: Vec<(u32, String)> = (0..12u32)
            .map(|i| {
                (
                    i * 10,
                    format!("متن فارسی نسخهٔ اول شمارهٔ {i} برای همین قسمت فیلم"),
                )
            })
            .collect();
        let b_cues: Vec<(u32, String)> = (0..12u32)
            .map(|i| {
                (
                    i * 10,
                    format!("متن فارسی نسخهٔ دوم شمارهٔ {i} برای همین قسمت فیلم"),
                )
            })
            .collect();
        let a = score_text(&srt(&a_cues), Some(600.0), true);
        let b = score_text(&srt(&b_cues), Some(600.0), true);
        let picked = pick(&[
            ScoredCandidate { id: 2u32, score: a },
            ScoredCandidate { id: 3u32, score: b },
        ]);
        match picked {
            PickOutcome::Uncertain { reason } => {
                assert!(reason.contains("هم‌امتیاز"), "{reason}");
            }
            other => panic!("انتظار Uncertain، دریافت: {other:?}"),
        }
    }

    #[test]
    fn tie_break_is_deterministic_on_id() {
        // برندهٔ واقعی (۱۲ cue فارسی) + کاندیدای بی‌شواهد (متن لاتین کوتاه):
        // قطعیت یعنی همان کاندیدا برنده شود، مستقل از ترتیب ورودی —
        // (ایندکس خروجی به آرایهٔ ورودی همان فراخوانی اشاره دارد)
        let good_cues: Vec<(u32, String)> = (0..12u32)
            .map(|i| {
                (
                    i * 10,
                    format!("جملهٔ گفتگوی طبیعی شمارهٔ {i} با متن کامل فارسی"),
                )
            })
            .collect();
        let good = score_text(&srt(&good_cues), Some(600.0), true);
        let weak_cues: Vec<(u32, String)> = (0..12u32)
            .map(|i| (i * 10, format!("ad {i} t.me/x")))
            .collect();
        let weak = score_text(&srt(&weak_cues), Some(600.0), true);
        let first_set = [
            ScoredCandidate {
                id: 5u32,
                score: good.clone(),
            },
            ScoredCandidate {
                id: 2u32,
                score: weak.clone(),
            },
        ];
        let second_set = [
            ScoredCandidate {
                id: 2u32,
                score: weak,
            },
            ScoredCandidate {
                id: 5u32,
                score: good,
            },
        ];
        let first = match pick(&first_set) {
            PickOutcome::Best { index, .. } => first_set[index].id,
            other => panic!("انتظار Best، دریافت: {other:?}"),
        };
        let second = match pick(&second_set) {
            PickOutcome::Best { index, .. } => second_set[index].id,
            other => panic!("انتظار Best، دریافت: {other:?}"),
        };
        assert_eq!(first, second, "نتیجه باید مستقل از ترتیب ورودی باشد");
    }

    #[test]
    fn coverage_uses_video_duration_when_known() {
        let short = score_text(
            &srt(&[(0, "متن فارسی کامل و طبیعی برای گفتگوی طولانی فیلم".into())]),
            Some(3600.0),
            true,
        );
        let long = score_text(
            &srt(&[
                (0, "متن فارسی کامل و طبیعی برای گفتگوی طولانی فیلم".into()),
                (30, "خط دوم گفتگوی طبیعی فیلم با ادامهٔ متن".into()),
                (55, "خط سوم گفتگوی طبیعی فیلم با ادامهٔ متن".into()),
            ]),
            Some(60.0),
            true,
        );
        assert!(short.metrics.coverage.unwrap() < 0.05);
        assert!(long.metrics.coverage.unwrap() > 0.9);
        assert!(long.score > short.score);
    }

    #[test]
    fn single_strong_candidate_is_selected_without_margin() {
        let good_cues: Vec<(u32, String)> = (0..20u32)
            .map(|i| {
                (
                    i * 10,
                    format!("جملهٔ گفتگوی طبیعی شمارهٔ {i} با متن کامل فارسی"),
                )
            })
            .collect();
        let good = score_text(&srt(&good_cues), Some(600.0), true);
        let picked = pick(&[ScoredCandidate {
            id: 7u32,
            score: good,
        }]);
        assert!(matches!(picked, PickOutcome::Best { index: 0, .. }));
    }

    #[test]
    fn weak_single_candidate_is_uncertain() {
        // فقط چند cue پراکنده — شواهد گفتگو کافی نیست
        let weak = score_text(
            &srt(&[(0, "سلام دوستان عزیز".into()), (300, "خط دوم کوتاه".into())]),
            Some(3600.0),
            true,
        );
        let picked = pick(&[ScoredCandidate {
            id: 7u32,
            score: weak,
        }]);
        assert!(matches!(picked, PickOutcome::Uncertain { .. }));
    }
}
