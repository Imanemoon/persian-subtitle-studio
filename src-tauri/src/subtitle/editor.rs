//! ویرایشگر زیرنویس — هستهٔ مستقل از Tauri برای parse/validate/write به SRT.
//!
//! قراردادها:
//! - این فاز فقط SRT را round-trip می‌کند (جدی‌ترین و رایج‌ترین فرمت)؛
//!   فرمت‌های دیگر (ASS/VTT/MicroDVD) در ویرایشگر باز نمی‌شوند و پیام صریح
//!   می‌گیرند — ساختار استایل‌شان حفظ‌کردنی با این مینیمال‌گرایی نیست.
//! - رندر شماره‌های cue را ۱..n نرمال می‌کند (قرارداد SRT)؛ متن cueها و
//!   ترتیبشان دقیقاً حفظ می‌شود، خطوط متن با `\n` حمل و با `\r\n` نوشته
//!   می‌شوند (خروجی استاندارد SRT).
//! - هیچ بازنویسی فایل اصلی: ذخیره همیشه به مقصد جدید `<stem>.edited.srt`
//!   با create_new اتمیک می‌رود (فراخواننده در commands/subtitle.rs).
//! - اعتبارسنجی: خطاها (مدت نامعتبر/منفی، ترتیب واژگون) ذخیره را رد
//!   می‌کنند؛ هشدارها (همپوشانی، متن خالی) فقط گزارش‌اند — هیچ محتوایی
//!   بی‌صدا حذف نمی‌شود.

use std::path::Path;

use crate::models::{EditorLoadResult, SubtitleCue};

use super::validation::CUE_LINE_RE;

// SubtitleCue/EditorLoadResult wire-types در models تعریف شده‌اند —
// این ماژول فقط parse/validate/write می‌کند و همان قرارداد را مصرف می‌کند.

/// خط زمان‌گذار SRT — ساعت اجباری، جداکنندهٔ , یا .
static EDITOR_TIME_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"^\s*(\d{1,2}):(\d{2}):(\d{2})[.,](\d{1,3})\s*-->\s*(\d{1,2}):(\d{2}):(\d{2})[.,](\d{1,3})\s*$",
    )
    .unwrap()
});

use std::sync::LazyLock;

/// hh:mm:ss,mmm → ms — ورودی از regex آماده.
fn parse_time_ms(h: &str, m: &str, s: &str, ms: &str) -> i64 {
    let h: i64 = h.parse().unwrap_or(0);
    let m: i64 = m.parse().unwrap_or(0);
    let s: i64 = s.parse().unwrap_or(0);
    let mut fraction: i64 = ms.parse().unwrap_or(0);
    // ۱ تا ۳ رقم مجاز — به میلی‌ثانیه نرمال شود (123 → 123، 12 → 120)
    let digits = ms.len() as i64;
    if digits < 3 {
        fraction *= 10_i64.pow((3 - digits) as u32);
    }
    ((h * 60 + m) * 60 + s) * 1000 + fraction
}

/// ms → hh:mm:ss,mmm — خروجی استاندارد SRT.
pub fn format_time_ms(total_ms: i64) -> String {
    let total_ms = total_ms.max(0);
    let h = total_ms / 3_600_000;
    let m = (total_ms % 3_600_000) / 60_000;
    let s = (total_ms % 60_000) / 1000;
    let ms = total_ms % 1000;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

/// parse متن SRT به cueها — block-based، خطوط متن با `\n` حفظ می‌شوند.
/// خط شمارهٔ cue اختیاری است (غیبتش هشدار است، نه خطا)؛ خط تایم‌گذار
/// بدشکل = خطا با شمارهٔ خط.
pub fn parse_srt_for_edit(text: &str) -> Result<EditorLoadResult, String> {
    let mut cues: Vec<SubtitleCue> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut pending_text: Vec<String> = Vec::new();
    let mut current: Option<SubtitleCue> = None;
    let mut saw_index_for_current = false;
    // شمارهٔ cue قبل از خط تایم‌گذار دیده شده (قرارداد استاندارد SRT)
    let mut pending_index = false;

    let flush = |current: &mut Option<SubtitleCue>,
                 pending: &mut Vec<String>,
                 saw_index: &mut bool,
                 cues: &mut Vec<SubtitleCue>,
                 warnings: &mut Vec<String>| {
        if let Some(mut cue) = current.take() {
            if !pending.is_empty() {
                cue.text = pending.join("\n");
            }
            if !*saw_index {
                warnings.push(format!(
                    "cue {c} بدون شماره بود — شماره‌گذاری هنگام ذخیره نرمال می‌شود.",
                    c = cues.len() + 1
                ));
            }
            cues.push(cue);
        }
        pending.clear();
        *saw_index = false;
    };

    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches(['\r']);
        if CUE_LINE_RE.is_match(line) {
            // متنِ جمع‌شدهٔ قبلی به cue فعلی می‌نشیند (خطوط خالی داخل متن حفظ
            // نمی‌شوند — جداکنندهٔ بلاک SRT است؛ فرمت استاندارد اجازهٔ متنِ
            // خالی بین دو cue را عملاً نمی‌دهد)
            flush(
                &mut current,
                &mut pending_text,
                &mut saw_index_for_current,
                &mut cues,
                &mut warnings,
            );
            let caps = EDITOR_TIME_RE
                .captures(line)
                .ok_or_else(|| format!("خط {}: خط تایم‌گذار بدشکل است: «{}»", line_no + 1, line))?;
            let start_ms = parse_time_ms(&caps[1], &caps[2], &caps[3], &caps[4]);
            let end_ms = parse_time_ms(&caps[5], &caps[6], &caps[7], &caps[8]);
            current = Some(SubtitleCue {
                start_ms,
                end_ms,
                text: String::new(),
            });
            // شمارهٔ دیده‌شده قبل از تایم‌گذار
            saw_index_for_current = pending_index;
            pending_index = false;
            continue;
        }
        if current.is_some() {
            if line.trim().is_empty() {
                // پایان بلاک cue
                flush(
                    &mut current,
                    &mut pending_text,
                    &mut saw_index_for_current,
                    &mut cues,
                    &mut warnings,
                );
            } else if line.trim().chars().all(|c| c.is_ascii_digit())
                && current.as_ref().map(|c| c.text.is_empty()).unwrap_or(false)
                && pending_text.is_empty()
            {
                // شمارهٔ بعد از تایم‌گذار — نگه نمی‌داریم (نرمال‌سازی ۱..n)
                saw_index_for_current = true;
            } else {
                pending_text.push(line.to_string());
            }
        } else if line.trim().is_empty() {
            continue; // فاصلهٔ بین بلاک‌ها
        } else if line.trim().chars().all(|c| c.is_ascii_digit()) {
            // شمارهٔ cue بعدی — قبل از تایم‌گذار (قرارداد استاندارد)
            pending_index = true;
        } else {
            return Err(format!(
                "خط {}: متن خارج از بلاک cue — فایل ساختار SRT معتبر ندارد.",
                line_no + 1
            ));
        }
    }
    flush(
        &mut current,
        &mut pending_text,
        &mut saw_index_for_current,
        &mut cues,
        &mut warnings,
    );

    if cues.is_empty() {
        return Err("هیچ cue معتبری در فایل پیدا نشد.".into());
    }
    Ok(EditorLoadResult {
        path: String::new(),
        cues,
        warnings,
    })
}

/// اعتبارسنجی cueهای ویرایش‌شده — خطاها ذخیره را رد می‌کنند، هشدارها فقط
/// نمایش داده می‌شوند. هیچ cueای بی‌صدا حذف نمی‌شود.
pub fn validate_cues(cues: &[SubtitleCue]) -> (Vec<String>, Vec<String>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for (index, cue) in cues.iter().enumerate() {
        let number = index + 1;
        if cue.start_ms < 0 || cue.end_ms < 0 {
            errors.push(format!("cue {number}: زمان منفی مجاز نیست."));
            continue;
        }
        if cue.end_ms <= cue.start_ms {
            errors.push(format!(
                "cue {number}: پایان باید بعد از شروع باشد ({start} → {end}).",
                start = format_time_ms(cue.start_ms),
                end = format_time_ms(cue.end_ms)
            ));
            continue;
        }
        if cue.text.trim().is_empty() {
            warnings.push(format!("cue {number}: متن خالی است — حفظ می‌شود."));
        }
        if let Some(previous) = cues.get(index.checked_sub(1).unwrap_or(usize::MAX)) {
            if cue.start_ms < previous.start_ms {
                errors.push(format!(
                    "cue {number}: شروع قبل از cue قبلی است — ترتیب زمانی باید صعودی باشد."
                ));
            } else if cue.start_ms < previous.end_ms {
                warnings.push(format!(
                    "cue {number}: با cue قبلی هم‌پوشانی دارد — مجاز ولی غیرمعمول."
                ));
            }
        }
    }
    (errors, warnings)
}

/// رندر SRT از cueها — شماره‌ها ۱..n، زمان‌ها hh:mm:ss,mmm، پایان خطوط CRLF.
pub fn write_srt(cues: &[SubtitleCue]) -> String {
    let mut out = String::new();
    for (index, cue) in cues.iter().enumerate() {
        out.push_str(&format!("{}", index + 1));
        out.push_str("\r\n");
        out.push_str(&format!(
            "{} --> {}\r\n",
            format_time_ms(cue.start_ms),
            format_time_ms(cue.end_ms)
        ));
        for line in cue.text.split('\n') {
            out.push_str(line.trim_end_matches('\r'));
            out.push_str("\r\n");
        }
        out.push_str("\r\n");
    }
    out
}

/// مقصد ذخیرهٔ ویرایش — `<stem>.edited.srt` کنار فایل اصلی؛ همیشه فایل جدید.
pub fn edited_target(source: &Path) -> Option<std::path::PathBuf> {
    let stem = source.file_stem()?.to_string_lossy().into_owned();
    let dir = source.parent()?;
    Some(dir.join(format!("{stem}.edited.srt")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "1\r\n00:00:01,000 --> 00:00:04,000\r\nسلام دنیا\r\n\r\n2\r\n00:00:05,500 --> 00:00:08,000\r\nخط دوم فارسی\r\nخط دوم ادامه\r\n";

    #[test]
    fn parse_and_write_roundtrip_preserves_text_and_order() {
        let result = parse_srt_for_edit(SAMPLE).unwrap();
        assert_eq!(result.cues.len(), 2);
        assert_eq!(result.cues[0].start_ms, 1000);
        assert_eq!(result.cues[0].end_ms, 4000);
        assert_eq!(result.cues[0].text, "سلام دنیا");
        // متن چندخطی با \n حفظ می‌شود
        assert_eq!(result.cues[1].text, "خط دوم فارسی\nخط دوم ادامه");

        let written = write_srt(&result.cues);
        let reparsed = parse_srt_for_edit(&written).unwrap();
        assert_eq!(reparsed.cues, result.cues, "roundtrip باید بی‌تغییر باشد");
    }

    #[test]
    fn mixed_crlf_and_lf_parse_identically() {
        let lf = SAMPLE.replace("\r\n", "\n");
        let a = parse_srt_for_edit(SAMPLE).unwrap();
        let b = parse_srt_for_edit(&lf).unwrap();
        assert_eq!(a.cues, b.cues);
    }

    #[test]
    fn missing_index_line_is_warning_not_error() {
        let text = "00:00:01,000 --> 00:00:04,000\nسلام\n";
        let result = parse_srt_for_edit(text).unwrap();
        assert_eq!(result.cues.len(), 1);
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn malformed_timestamp_is_error_with_line_number() {
        let err = parse_srt_for_edit("1\n00:00:01 --> \nمتن\n").unwrap_err();
        assert!(err.contains("خط 2"), "{err}");
    }

    #[test]
    fn empty_file_is_error() {
        assert!(parse_srt_for_edit("").is_err());
        assert!(parse_srt_for_edit("this is not a subtitle").is_err());
    }

    #[test]
    fn validate_rejects_negative_and_nonpositive_duration() {
        let (errors, _) = validate_cues(&[SubtitleCue {
            start_ms: -5,
            end_ms: 100,
            text: "x".into(),
        }]);
        assert!(errors.iter().any(|e| e.contains("منفی")));

        let (errors, _) = validate_cues(&[SubtitleCue {
            start_ms: 100,
            end_ms: 100,
            text: "x".into(),
        }]);
        assert!(errors.iter().any(|e| e.contains("بعد از شروع")));
    }

    #[test]
    fn validate_rejects_out_of_order_starts_and_warns_on_overlap() {
        let cues = vec![
            SubtitleCue {
                start_ms: 2000,
                end_ms: 4000,
                text: "اول".into(),
            },
            SubtitleCue {
                start_ms: 1000,
                end_ms: 3000,
                text: "واژگون".into(),
            },
        ];
        let (errors, _) = validate_cues(&cues);
        assert!(errors.iter().any(|e| e.contains("ترتیب زمانی")));

        let overlapping = vec![
            SubtitleCue {
                start_ms: 1000,
                end_ms: 5000,
                text: "اول".into(),
            },
            SubtitleCue {
                start_ms: 2000,
                end_ms: 3000,
                text: "دوم".into(),
            },
        ];
        let (errors, warnings) = validate_cues(&overlapping);
        assert!(errors.is_empty(), "هم‌پوشانی هشدار است نه خطا");
        assert!(warnings.iter().any(|w| w.contains("هم‌پوشانی")));
    }

    #[test]
    fn empty_text_is_warning_not_dropped() {
        let cues = vec![SubtitleCue {
            start_ms: 1000,
            end_ms: 2000,
            text: String::new(),
        }];
        let (errors, warnings) = validate_cues(&cues);
        assert!(errors.is_empty());
        assert!(warnings.iter().any(|w| w.contains("متن خالی")));
        // و در write حفظ می‌شود
        let written = write_srt(&cues);
        let reparsed = parse_srt_for_edit(&written).unwrap();
        assert_eq!(reparsed.cues.len(), 1);
    }

    #[test]
    fn write_normalizes_numbering_and_formats_time() {
        let cues = vec![
            SubtitleCue {
                start_ms: 3_723_456,
                end_ms: 3_725_000,
                text: "الف".into(),
            },
            SubtitleCue {
                start_ms: 4000,
                end_ms: 5000,
                text: "ب".into(),
            },
        ];
        let written = write_srt(&cues);
        assert!(written.contains("1\r\n01:02:03,456 --> 01:02:05,000\r\nالف"));
        assert!(written.contains("2\r\n00:00:04,000 --> 00:00:05,000\r\nب"));
    }

    #[test]
    fn edited_target_is_sibling_new_file() {
        let source = std::path::Path::new("D:/Media/Show.S01E01.fa.srt");
        let target = edited_target(source).unwrap();
        assert_eq!(
            target,
            std::path::Path::new("D:/Media/Show.S01E01.fa.edited.srt")
        );
        assert_ne!(target, source, "مقصد هرگز فایل اصلی نیست");
    }
}
