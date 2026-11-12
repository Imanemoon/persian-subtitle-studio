//! دستورات استخراج زیرنویس داخلی — همیشه دو مرحله‌ای برای «Extract All»:
//! اول `plan_extraction_all` (خلاصه برای تأیید کاربر)، بعد `execute_extraction_plan`.
//!
//! عمر لغو: هر عملیات (گروهی یا تکی) پرچم لغو مخصوص خودش را در `OpsRegistry`
//! می‌گیرد؛ لغو یک عملیات هرگز عملیات دیگر را متوقف نمی‌کند.
//!
//! هسته منطق مستقل از Tauri است و فقط با `&AppState` کار می‌کند.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::models::{
    AnalysisStatus, ExtractionOutcome, ExtractionPlan, ExtractionPlanItem, OpsProgress,
    SkippedExtraction, SubtitleKind, SubtitleValidation, TrackReview, VideoEntry,
};
use crate::state::{acquire_batch_op, acquire_single_op, AppState};

use super::media::{apply_entry, get_entry, now_ms, resolve_tool, set_status_core};

/// هسته استخراج استاندارد به `<base>.fa.srt` — برای عملیات تکی و گروهی مشترک.
/// پرچم لغو از تماس‌گیرنده می‌آید (اسلات عملیات تکی یا پرچم عملیات گروهی).
/// State را به‌روز می‌کند؛ مستقل از Tauri.
pub fn run_extract_blocking(
    state: &AppState,
    path: &str,
    stream_index: u32,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<ExtractionOutcome, String> {
    let Some(entry) = get_entry(state, path) else {
        return Err("این ویدیو در کتابخانه نیست — ابتدا اسکن کنید.".into());
    };
    let Some(analysis) = entry.analysis.clone() else {
        return Err("ابتدا ویدیو را تحلیل کنید (Analyze).".into());
    };
    let Some(stream) = analysis
        .streams
        .iter()
        .find(|s| s.stream_index == stream_index)
    else {
        return Err(format!(
            "Track با شماره {stream_index} در تحلیل این ویدیو نیست."
        ));
    };
    if stream.kind != SubtitleKind::Text || !(stream.persian_candidate || stream.persian_confirmed)
    {
        return Err("فقط Track متنیِ فارسی (کاندید یا تأییدشده) قابل استخراج مستقیم است.".into());
    }

    let video = PathBuf::from(&entry.path);
    crate::subtitle::detection::check_extraction_target(&video, &entry.sibling_subtitles)?;

    set_status_core(state, path, AnalysisStatus::Extracting, None);
    crate::logging::info(
        "subtitle",
        &format!(
            "استخراج Track {stream_index} ({}) از {}",
            stream.codec_name, entry.file_name
        ),
    );

    let ffmpeg = resolve_tool(state, "ffmpeg")?;
    let result = crate::subtitle::extraction::extract_fa_srt(&ffmpeg, &video, stream_index, cancel);

    match result {
        Ok((target, validation)) => {
            finish_successful_extract(state, &entry, stream_index, target, validation)
        }
        Err(err) => {
            let message = err.message();
            let (fallback_status, status_message) = match &err {
                crate::subtitle::extraction::ExtractError::Cancelled => {
                    let (s, m) = crate::subtitle::detection::classify_analysis(&analysis.streams);
                    (s, m.unwrap_or_else(|| "عملیات لغو شد".into()))
                }
                crate::subtitle::extraction::ExtractError::Validation(_) => {
                    (AnalysisStatus::InvalidSubtitle, message.clone())
                }
                _ => (AnalysisStatus::Failed, message.clone()),
            };
            set_status_core(state, path, fallback_status, Some(status_message));
            crate::logging::error(
                "subtitle",
                &format!("استخراج {} ناموفق: {message}", entry.file_name),
            );
            Err(message)
        }
    }
}

fn finish_successful_extract(
    state: &AppState,
    entry: &VideoEntry,
    stream_index: u32,
    target: PathBuf,
    validation: SubtitleValidation,
) -> Result<ExtractionOutcome, String> {
    let persian_confirmed = validation.has_persian;
    crate::logging::info(
        "subtitle",
        &format!(
            "استخراج موفق: {} ({} cue، encoding: {}، فارسی: {})",
            target.file_name().unwrap_or_default().to_string_lossy(),
            validation.cue_count,
            validation.encoding,
            if persian_confirmed {
                "تأیید شد"
            } else {
                "تأیید نشد!"
            }
        ),
    );
    if !persian_confirmed {
        crate::logging::warn(
            "subtitle",
            "Track با Tag فارسی استخراج شد اما متن آن فارسی نیست — به‌عنوان فارسیِ تأییدشده ثبت نشد",
        );
    }

    let outcome = ExtractionOutcome {
        stream_index,
        target_path: target.to_string_lossy().to_string(),
        validation,
        persian_confirmed,
    };

    let mut updated = entry.clone();
    let a = updated.analysis.as_mut().expect("analysis exists");
    a.status = AnalysisStatus::Extracted;
    a.message = Some(if persian_confirmed {
        format!(
            "استخراج شد: {} ({} cue، فارسی تأیید شد)",
            target.file_name().unwrap_or_default().to_string_lossy(),
            outcome.validation.cue_count
        )
    } else {
        format!(
            "استخراج شد: {} — اما محتوای آن فارسی تأیید نشد",
            target.file_name().unwrap_or_default().to_string_lossy()
        )
    });
    a.last_extraction = Some(outcome.clone());
    a.analyzed_at_ms = now_ms();

    // فایل .fa.srt جدید کنار ویدیو است — فهرست زیرنویس‌های کناری تازه شود
    refresh_siblings(&mut updated);
    apply_entry(state, updated);
    Ok(outcome)
}

fn refresh_siblings(updated: &mut VideoEntry) {
    if let (Some(dir), Some(stem)) = (
        Some(Path::new(&updated.parent_dir)).filter(|d| d.is_dir()),
        Path::new(&updated.path).file_stem(),
    ) {
        updated.sibling_subtitles =
            crate::scanner::scan_sibling_subtitles(dir, &stem.to_string_lossy());
    }
}

#[tauri::command(async)]
pub async fn extract_track(
    app: AppHandle,
    path: String,
    stream_index: u32,
) -> Result<VideoEntry, String> {
    let app2 = app.clone();
    let path2 = path.clone();
    // رزرو اتمیک اسلات عملیات تکی با پرچم لغو تازه (RAII — در Drop آزاد می‌شود)
    let guard = acquire_single_op(&app.state::<AppState>().ops, &path)?;
    let flag = guard.cancel.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let st = app2.state::<AppState>();
        let r = run_extract_blocking(&st, &path2, stream_index, &flag);
        drop(guard); // آزادسازی در همهٔ مسیرها
        r
    })
    .await
    .map_err(|e| format!("خطای Thread: {e}"))?;

    result?;
    let state = app.state::<AppState>();
    let entry = get_entry(&state, &path).ok_or_else(|| "ویدیو در کتابخانه پیدا نشد".to_string())?;
    // وضعیت Extracted/Failed و زیرنویس‌های کناری تازه باید به UI برسد
    let _ = app.emit("library:entry", &entry);
    Ok(entry)
}

// ---------------------------------------------------------------------------
// استخراج آزمایشی برای بررسی (Trackهای متنی بدون Tag معتبر)
// ---------------------------------------------------------------------------

/// هسته استخراج آزمایشی: خروجی به `<base>.track-<N>.review.srt`، بدون بازنویسی.
/// وضعیت اصلی ویدیو را عوض نمی‌کند؛ فقط نتیجه را ثبت و Track را (در صورت تأیید
/// محتوا) به‌عنوان فارسیِ تأییدشده علامت می‌زند.
pub fn run_review_blocking(
    state: &AppState,
    path: &str,
    stream_index: u32,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<TrackReview, String> {
    let Some(entry) = get_entry(state, path) else {
        return Err("این ویدیو در کتابخانه نیست — ابتدا اسکن کنید.".into());
    };
    let Some(analysis) = entry.analysis.clone() else {
        return Err("ابتدا ویدیو را تحلیل کنید (Analyze).".into());
    };
    let Some(stream) = analysis
        .streams
        .iter()
        .find(|s| s.stream_index == stream_index)
    else {
        return Err(format!(
            "Track با شماره {stream_index} در تحلیل این ویدیو نیست."
        ));
    };
    if stream.kind != SubtitleKind::Text {
        return Err("استخراج آزمایشی فقط برای Trackهای متنی معنا دارد.".into());
    }

    let video = PathBuf::from(&entry.path);
    crate::logging::info(
        "subtitle",
        &format!(
            "استخراج آزمایشی Track {stream_index} ({}) از {} برای بررسی محتوا",
            stream.codec_name, entry.file_name
        ),
    );

    let ffmpeg = resolve_tool(state, "ffmpeg")?;
    let result =
        crate::subtitle::extraction::extract_for_review(&ffmpeg, &video, stream_index, cancel);

    match result {
        Ok((review_path, validation)) => {
            let persian_confirmed = validation.has_persian;
            let review = TrackReview {
                stream_index,
                review_path: review_path.to_string_lossy().to_string(),
                validation: validation.clone(),
                persian_confirmed,
                promoted: false,
            };
            crate::logging::info(
                "subtitle",
                &format!(
                    "فایل بررسی ساخته شد: {} ({} cue، فارسی: {})",
                    review_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    validation.cue_count,
                    if persian_confirmed {
                        "تأیید شد"
                    } else {
                        "تأیید نشد"
                    }
                ),
            );

            let mut updated = entry.clone();
            let a = updated.analysis.as_mut().expect("analysis exists");
            a.reviews.push(review.clone());

            // تأیید محتوا را روی خود Track ثبت کن و وضعیت را دوباره ارزیابی کن
            if let Some(s) = a
                .streams
                .iter_mut()
                .find(|s| s.stream_index == stream_index)
            {
                s.persian_confirmed = persian_confirmed;
            }
            let (new_status, new_message) =
                crate::subtitle::detection::classify_analysis(&a.streams);
            a.status = new_status;
            a.message = new_message;
            a.analyzed_at_ms = now_ms();

            // فایل بررسی کنار ویدیوست — زیرنویس‌های کناری تازه شوند
            refresh_siblings(&mut updated);
            apply_entry(state, updated);
            Ok(review)
        }
        Err(err) => {
            let message = err.message();
            crate::logging::error(
                "subtitle",
                &format!("استخراج آزمایشی {} ناموفق: {message}", entry.file_name),
            );
            Err(message)
        }
    }
}

#[tauri::command(async)]
pub async fn extract_track_for_review(
    app: AppHandle,
    path: String,
    stream_index: u32,
) -> Result<TrackReview, String> {
    let app2 = app.clone();
    let path2 = path.clone();
    let guard = acquire_single_op(&app.state::<AppState>().ops, &path)?;
    let flag = guard.cancel.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let st = app2.state::<AppState>();
        let r = run_review_blocking(&st, &path2, stream_index, &flag);
        drop(guard);
        r
    })
    .await
    .map_err(|e| format!("خطای Thread: {e}"))?;

    let review = result?;
    // وضعیت و Trackهای تأییدشده ممکن است تغییر کرده باشد
    let state = app.state::<AppState>();
    if let Some(entry) = get_entry(&state, &path) {
        let _ = app.emit("library:entry", &entry);
    }
    Ok(review)
}

/// هسته تبدیل فایل بررسی تأییدشده به `<base>.fa.srt` — مستقل از Tauri و
/// **بدون رزرو اسلات** تا pipeline (مالک گارد گروهی) هم بتواند مستقیماً
/// صدا بزند؛ رزرو در wrapper `run_promote_blocking` انجام می‌شود.
pub fn promote_review_core(
    state: &AppState,
    path: &str,
    stream_index: u32,
) -> Result<VideoEntry, String> {
    let Some(entry) = get_entry(state, path) else {
        return Err("این ویدیو در کتابخانه نیست.".into());
    };
    let Some(analysis) = entry.analysis.clone() else {
        return Err("ابتدا ویدیو را تحلیل کنید.".into());
    };
    // آخرین بررسیِ همین Track که تأیید شده و هنوز تبدیل نشده
    let Some(review) = analysis
        .reviews
        .iter()
        .rev()
        .find(|r| r.stream_index == stream_index && r.persian_confirmed && !r.promoted)
    else {
        return Err("بررسی تأییدشده‌ای برای این Track وجود ندارد (یا قبلاً تبدیل شده است).".into());
    };

    let review_path = PathBuf::from(&review.review_path);
    if !review_path.is_file() {
        return Err("فایل بررسی روی دیسک پیدا نشد — دوباره استخراج آزمایشی بزنید.".into());
    }

    // اعتبارسنجی مجدد محتوای فایل بررسی — اگر پس از Review تغییر کرده و
    // دیگر معتبر/فارسی نیست، Promote رد می‌شود و هیچ فایلی جابه‌جا نمی‌شود
    let revalidation = crate::subtitle::validation::validate_file(&review_path);
    if !revalidation.valid {
        return Err(format!(
            "فایل بررسی دیگر معتبر نیست ({}: {}) — Promote رد شد.",
            revalidation.encoding,
            revalidation.error.as_deref().unwrap_or("نامشخص")
        ));
    }
    if !revalidation.has_persian {
        return Err(
            "محتوای فارسی در فایل بررسی پیدا نشد — Promote رد شد و فایل دست‌نخورده ماند.".into(),
        );
    }

    let video = PathBuf::from(&entry.path);
    let target =
        crate::subtitle::detection::check_extraction_target(&video, &entry.sibling_subtitles)?;

    if target.exists() {
        return Err(format!(
            "مقصد «{}» موجود است — هیچ فایلی بازنویسی نمی‌شود.",
            target.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    std::fs::rename(&review_path, &target)
        .map_err(|e| format!("انتقال فایل بررسی به مقصد: {e}"))?;

    crate::logging::info(
        "subtitle",
        &format!(
            "فایل بررسی به زیرنویس استاندارد تبدیل شد: {}",
            target.file_name().unwrap_or_default().to_string_lossy()
        ),
    );

    let mut updated = entry.clone();
    let a = updated.analysis.as_mut().expect("analysis exists");
    if let Some(r) = a
        .reviews
        .iter_mut()
        .rev()
        .find(|r| r.stream_index == stream_index && !r.promoted)
    {
        r.promoted = true;
    }
    a.status = AnalysisStatus::Extracted;
    a.message = Some(format!(
        "استخراج شد: {} ({} cue، فارسی تأیید شد — از بررسی Track {stream_index})",
        target.file_name().unwrap_or_default().to_string_lossy(),
        revalidation.cue_count
    ));
    a.last_extraction = Some(ExtractionOutcome {
        stream_index,
        target_path: target.to_string_lossy().to_string(),
        validation: revalidation,
        persian_confirmed: true,
    });
    a.analyzed_at_ms = now_ms();
    refresh_siblings(&mut updated);
    apply_entry(state, updated.clone());
    Ok(updated)
}

/// wrapper سازگاری: رزرو اتمیک اسلات تکی + فراخوانی هستهٔ slot-free.
pub fn run_promote_blocking(
    state: &AppState,
    path: &str,
    stream_index: u32,
) -> Result<VideoEntry, String> {
    // رزرو اتمیک اسلات از ابتدا تا پایان (RAII — در همهٔ مسیرها آزاد می‌شود)
    let guard = acquire_single_op(&state.ops, path)?;
    let result = promote_review_core(state, path, stream_index);
    drop(guard);
    result
}

#[tauri::command]
pub fn promote_review(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    stream_index: u32,
) -> Result<VideoEntry, String> {
    let updated = run_promote_blocking(&state, &path, stream_index)?;
    let _ = app.emit("library:entry", &updated);
    Ok(updated)
}

// ---------------------------------------------------------------------------
// Extract All (پلن + اجرای گروهی)
// ---------------------------------------------------------------------------

/// هسته ساخت پلن — مستقل از Tauri؛ هم command و هم تست‌ها استفاده می‌کنند.
pub fn plan_extraction_from_library(library: &[VideoEntry]) -> ExtractionPlan {
    let mut items = Vec::new();
    let mut skipped = Vec::new();

    for e in library {
        let skip = |reason: String| SkippedExtraction {
            video_path: e.path.clone(),
            video_file_name: e.file_name.clone(),
            reason,
        };

        let Some(analysis) = &e.analysis else {
            skipped.push(skip("هنوز تحلیل نشده".into()));
            continue;
        };

        match analysis.status {
            AnalysisStatus::PersianTextTrackFound => {
                let Some(stream) = analysis.streams.iter().find(|s| {
                    s.kind == SubtitleKind::Text && (s.persian_candidate || s.persian_confirmed)
                }) else {
                    skipped.push(skip(
                        "وضعیت و Trackها هم‌خوان نیست — دوباره تحلیل کنید".into(),
                    ));
                    continue;
                };
                let video = Path::new(&e.path);
                match crate::subtitle::detection::check_extraction_target(
                    video,
                    &e.sibling_subtitles,
                ) {
                    Ok(target) => items.push(ExtractionPlanItem {
                        video_path: e.path.clone(),
                        video_file_name: e.file_name.clone(),
                        stream_index: stream.stream_index,
                        track_label: track_label(stream),
                        target_path: target.to_string_lossy().to_string(),
                    }),
                    Err(reason) => skipped.push(skip(reason)),
                }
            }
            AnalysisStatus::NeedsTrackSelection => {
                skipped.push(skip("چند Track فارسی دارد — انتخاب دستی لازم است".into()))
            }
            AnalysisStatus::ImageSubtitleFound => skipped.push(skip(
                "فقط Track تصویری دارد — استخراج متن به OCR نیاز دارد".into(),
            )),
            AnalysisStatus::NoEmbeddedSubtitle => skipped.push(skip("زیرنویس داخلی ندارد".into())),
            AnalysisStatus::NoPersianTrack => skipped.push(skip(
                "Track فارسی ندارد — در صورت شک می‌توانید استخراج آزمایشی بزنید".into(),
            )),
            AnalysisStatus::Extracted => skipped.push(skip("قبلاً استخراج شده".into())),
            AnalysisStatus::InvalidSubtitle => {
                skipped.push(skip("زیرنویس موجود نامعتبر است — بررسی دستی لازم".into()))
            }
            AnalysisStatus::Failed => {
                skipped.push(skip("تحلیل قبلی ناموفق بود — دوباره تحلیل کنید".into()))
            }
            AnalysisStatus::NotAnalyzed => skipped.push(skip("هنوز تحلیل نشده".into())),
            AnalysisStatus::Analyzing | AnalysisStatus::Extracting => {
                skipped.push(skip("در حال پردازش است".into()))
            }
        }
    }

    ExtractionPlan { items, skipped }
}

/// خلاصه عملیات Extract All برای تأیید کاربر — هیچ تغییری روی دیسک نمی‌دهد.
#[tauri::command]
pub fn plan_extraction_all(state: State<'_, AppState>) -> Result<ExtractionPlan, String> {
    let library = state.library.lock().unwrap().clone();
    Ok(plan_extraction_from_library(&library))
}

fn track_label(s: &crate::models::SubtitleStream) -> String {
    format!(
        "#{} {} [{}] {}",
        s.stream_index,
        s.codec_name,
        s.language
            .as_deref()
            .or(s.language_ietf.as_deref())
            .unwrap_or("?"),
        s.title.as_deref().unwrap_or("—")
    )
}

/// حلقه اجرای موارد پلن — مشترک بین command گروهی و تست‌ها.
/// `on_progress` برای هر مورد و `on_updated` بعد از تغییر وضعیت هر ویدیو صدا
/// زده می‌شود. پرچم لغو از عملیات گروهی می‌آید. خروجی: (موفق، ناموفق)
pub fn execute_plan_items(
    state: &AppState,
    total: u64,
    items: &[ExtractionPlanItem],
    cancel: &std::sync::atomic::AtomicBool,
    mut on_progress: impl FnMut(OpsProgress),
    mut on_updated: impl FnMut(VideoEntry),
) -> (u64, u64) {
    let mut success = 0u64;
    let mut failed = 0u64;
    for (i, item) in items.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            crate::logging::warn("subtitle", "پلن استخراج توسط کاربر لغو شد");
            break;
        }
        on_progress(OpsProgress {
            kind: "extract".into(),
            current: i as u64 + 1,
            total,
            video_path: item.video_path.clone(),
            message: item.track_label.clone(),
        });
        match run_extract_blocking(state, &item.video_path, item.stream_index, cancel) {
            Ok(_) => success += 1,
            Err(err) => {
                failed += 1;
                crate::logging::error(
                    "subtitle",
                    &format!("استخراج {} ناموفق: {err}", item.video_file_name),
                );
            }
        }
        // وضعیت جدید (Extracted یا Failed) به UI منتشر شود
        if let Some(updated) = get_entry(state, &item.video_path) {
            on_updated(updated);
        }
    }
    (success, failed)
}

/// اجرای پلن تأییدشده در پس‌زمینه؛ تعداد صف را برمی‌گرداند.
#[tauri::command]
pub fn execute_extraction_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    items: Vec<ExtractionPlanItem>,
) -> Result<u64, String> {
    if items.is_empty() {
        return Err("پلن استخراج خالی است.".into());
    }

    // ایمنی: در لحظه اجرا دوباره مطابقت پلن با کتابخانه بررسی می‌شود
    for item in &items {
        if get_entry(&state, &item.video_path).is_none() {
            return Err(format!(
                "«{}» در کتابخانه نیست — پلن را دوباره بسازید.",
                item.video_file_name
            ));
        }
    }

    // پرچم لغو تازه مخصوص این عملیات گروهی — گارد RAII با Drop ایمن
    let guard = acquire_batch_op(&state.ops)?;
    let cancel = Arc::clone(&guard.cancel);
    let total = items.len() as u64;
    crate::logging::info(
        "subtitle",
        &format!("اجرای پلن استخراج با {total} مورد شروع شد"),
    );

    let app2 = app.clone();
    std::thread::spawn(move || {
        let st = app2.state::<AppState>();
        let (success, failed) = execute_plan_items(
            &st,
            total,
            &items,
            &cancel,
            |p| {
                let _ = app2.emit("ops:progress", &p);
            },
            |updated| {
                let _ = app2.emit("library:entry", &updated);
            },
        );
        // پایان رزرو قبل از رویداد نهایی — همان ترتیب قبلی
        drop(guard);
        crate::logging::info(
            "subtitle",
            &format!("پلن استخراج تمام شد: {success} موفق، {failed} ناموفق"),
        );
        let _ = app2.emit(
            "ops:done",
            &OpsProgress {
                kind: "extract".into(),
                current: success,
                total,
                video_path: String::new(),
                message: format!("{success} موفق، {failed} ناموفق"),
            },
        );
    });

    Ok(total)
}

/// لغو همهٔ عملیات جاری پوشش‌داده‌شدهٔ قرارداد (گروهی + استخراج‌های تکی).
/// خروجی: آیا چیزی در جریان بود؟
///
/// poison-safe: قفل مسموم با recovery بازیابی می‌شود تا لغو همیشه کار کند.
/// رزرو اسکن عمداً لغو نمی‌شود — اسکن با عملیات‌های این قرارداد متقابلاً
/// انحصاری است و لغو لغوِ اسکن فقط از مسیر اختصاصی `stop_scan` انجام می‌شود.
/// اگر pipeline فعال باشد و cancel_all پرچم آن را ست کند، waiter آن بیدار
/// می‌شود (notify پس از رهایی قفل registry).
#[tauri::command]
pub fn cancel_subtitle_ops(state: State<'_, AppState>) -> bool {
    let any = match state.ops.lock() {
        Ok(mut ops) => ops.cancel_all(),
        Err(poisoned) => poisoned.into_inner().cancel_all(),
    };
    // رهایی قفل registry پیش از notify — هیچ قفل دیگری هم‌زمان نگه داشته نمی‌شود
    crate::pipeline::notify_cancelled_waiter(&state);
    if any {
        crate::logging::info("subtitle", "درخواست لغو عملیات ثبت شد");
    }
    any
}

/// اعتبارسنجی یک فایل زیرنویس دلخواه (برای دکمه «بررسی» در UI).
#[tauri::command]
pub fn validate_subtitle_file(path: String) -> Result<SubtitleValidation, String> {
    let p = Path::new(&path);
    if !p.is_file() {
        return Err("فایل وجود ندارد.".into());
    }
    Ok(crate::subtitle::validation::validate_file(p))
}
