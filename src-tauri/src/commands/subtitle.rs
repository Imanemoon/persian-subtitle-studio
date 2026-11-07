//! دستورات استخراج زیرنویس داخلی — همیشه دو مرحله‌ای برای «Extract All»:
//! اول `plan_extraction_all` (خلاصه برای تأیید کاربر)، بعد `execute_extraction_plan`.
//!
//! هسته منطق مستقل از Tauri است و فقط با `&AppState` کار می‌کند.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::models::{
    AnalysisStatus, ExtractionOutcome, ExtractionPlan, ExtractionPlanItem, OpsProgress,
    SkippedExtraction, SubtitleKind, SubtitleValidation, VideoEntry,
};
use crate::state::AppState;

use super::media::{apply_entry, get_entry, now_ms, resolve_tool, set_status_core};

/// هسته استخراج — برای عملیات تکی و گروهی مشترک.
/// State را به‌روز می‌کند و نتیجه را برمی‌گرداند؛ مستقل از Tauri.
pub fn run_extract_blocking(
    state: &AppState,
    path: &str,
    stream_index: u32,
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
    if !stream.persian_candidate || stream.kind != SubtitleKind::Text {
        return Err("فقط Track متنیِ کاندید فارسی قابل استخراج است.".into());
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
    let result = crate::subtitle::extraction::extract_fa_srt(
        &ffmpeg,
        &video,
        stream_index,
        &state.cancel_ops,
    );

    match result {
        Ok((target, validation)) => {
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
            if let (Some(dir), Some(stem)) = (
                Some(Path::new(&updated.parent_dir)).filter(|d| d.is_dir()),
                Path::new(&updated.path).file_stem(),
            ) {
                updated.sibling_subtitles =
                    crate::scanner::scan_sibling_subtitles(dir, &stem.to_string_lossy());
            }

            apply_entry(state, updated);
            Ok(outcome)
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

#[tauri::command(async)]
pub async fn extract_track(
    app: AppHandle,
    path: String,
    stream_index: u32,
) -> Result<VideoEntry, String> {
    let app2 = app.clone();
    let path2 = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let st = app2.state::<AppState>();
        run_extract_blocking(&st, &path2, stream_index)
    })
    .await
    .map_err(|e| format!("خطای Thread: {e}"))??;

    let state = app.state::<AppState>();
    let entry = get_entry(&state, &path).ok_or_else(|| "ویدیو در کتابخانه پیدا نشد".to_string())?;
    // وضعیت Extracted/Failed و زیرنویس‌های کناری تازه باید به UI برسد
    let _ = app.emit("library:entry", &entry);
    Ok(entry)
}

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
                let Some(stream) = analysis
                    .streams
                    .iter()
                    .find(|s| s.persian_candidate && s.kind == SubtitleKind::Text)
                else {
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
            AnalysisStatus::NoPersianTrack => skipped.push(skip("Track فارسی ندارد".into())),
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
/// زده می‌شود. خروجی: (موفق، ناموفق)
pub fn execute_plan_items(
    state: &AppState,
    total: u64,
    items: &[ExtractionPlanItem],
    mut on_progress: impl FnMut(OpsProgress),
    mut on_updated: impl FnMut(VideoEntry),
) -> (u64, u64) {
    let mut success = 0u64;
    let mut failed = 0u64;
    for (i, item) in items.iter().enumerate() {
        if state.cancel_ops.load(Ordering::SeqCst) {
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
        match run_extract_blocking(state, &item.video_path, item.stream_index) {
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
    if state.ops_running.load(Ordering::SeqCst) {
        return Err("یک عملیات گروهی در حال اجراست؛ ابتدا آن را متوقف کنید.".into());
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

    let total = items.len() as u64;
    state.ops_running.store(true, Ordering::SeqCst);
    state.cancel_ops.store(false, Ordering::SeqCst);
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
            |p| {
                let _ = app2.emit("ops:progress", &p);
            },
            |updated| {
                let _ = app2.emit("library:entry", &updated);
            },
        );
        st.ops_running.store(false, Ordering::SeqCst);
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

#[tauri::command]
pub fn cancel_subtitle_ops(state: State<'_, AppState>) -> bool {
    let running = state.ops_running.load(Ordering::SeqCst);
    state.cancel_ops.store(true, Ordering::SeqCst);
    if running {
        crate::logging::info("subtitle", "درخواست لغو عملیات ثبت شد");
    }
    running
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
