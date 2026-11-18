//! موتور اجرای pipeline — هر مرحله فقط هسته‌های موجود را صدا می‌زند؛
//! هیچ منطق استخراج/دانلود/آرشیو/سینک تکرار نمی‌شود و هیچ تصمیمی
//! خودکار گرفته نمی‌شود.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use crate::models::{AnalysisStatus, SubtitleKind, VideoEntry};
use crate::state::AppState;
use crate::subsource::plan::{
    create_download_plan, inspect_persian_sidecars, SkipReasonCode, SkippedDownload,
};

use super::types::{
    DecisionKind, DecisionOptionPayload, ItemArtifacts, ItemOutcome, ItemRuntime, PipelineCounts,
    PipelineEvent, PipelineInput, PipelineStage, PipelineStatus, PipelineSummary,
    PreSelectionArtifact, StageStatus,
};
use super::{PipelineControl, WaitExit};

/// خطای سطح run — پس از هر Err، آیتم‌های باز mark می‌شوند و run_worker
/// خلاصهٔ نهایی را از snapshot می‌سازد (nested result — panic جدا از Err).
#[derive(Debug)]
pub(crate) enum PipelineError {
    Cancelled,
    Fatal(String),
}

/// پایان lifecycle worker — خلاصهٔ نهایی داخل boundary؛ پاک‌سازی بیرون آن.
/// وابستگی‌های بیرونی pipeline — در تولید مقدار واقعی، در تست تزریق‌شدنی.
/// هیچ رفتار شاخه‌ای جدیدی به تولید اضافه نمی‌کند؛ فقط جای درز برای آزمون.
pub(crate) struct PipelineIo<'a> {
    /// بازیابی API key از مخزن کلید (همان `retrieve_api_key` در تولید).
    pub key_provider: &'a (dyn Fn() -> Result<Option<String>, String> + Send + Sync),
    /// ریشهٔ staging دانلود — `None` = ریشهٔ volume (رفتار تولید).
    pub staging_base: Option<&'a std::path::Path>,
}

impl PipelineIo<'static> {
    /// مقدار تولید: کلید از Credential Manager، staging روی ریشهٔ volume.
    pub(crate) fn production() -> Self {
        PipelineIo {
            key_provider: &crate::subsource::retrieve_api_key,
            staging_base: None,
        }
    }
}

pub(crate) fn run_worker(
    state: &AppState,
    control: &Arc<PipelineControl>,
    cleanup: super::WorkerCleanup<'_>,
    input: PipelineInput,
    sink: &super::SinkStore,
    io: &PipelineIo<'_>,
) {
    let lifecycle = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match run_pipeline_body(state, control, input, sink, io) {
            Ok(summary) => summary, // ۱) موفق
            Err(PipelineError::Cancelled) => {
                // ۲) لغو
                let mut summary = summary_from_control(
                    control,
                    true,
                    PipelineStatus::Cancelled,
                    "pipeline لغو شد.",
                );
                for item in &mut summary.items {
                    if item.outcome.is_none() {
                        item.outcome = Some(ItemOutcome::Cancelled);
                    }
                }
                summary.counts.decisions_pending = 0;
                summary
            }
            Err(PipelineError::Fatal(reason)) => {
                // ۳) خطای عادی → Failed
                let mut summary =
                    summary_from_control(control, false, PipelineStatus::Failed, reason.clone());
                summary.warnings.push(reason);
                summary
            }
        }
    }));
    match lifecycle {
        Ok(summary) => super::emit_to(sink, PipelineEvent::Done { summary }),
        // ۴) panic — خلاصهٔ Failed + fallback emit داخل boundary خودش
        Err(panic_payload) => {
            let mut summary = summary_from_control(
                control,
                false,
                PipelineStatus::Failed,
                "panic در اجرای pipeline — خلاصهٔ ناقص",
            );
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                super::emit_to(
                    sink,
                    PipelineEvent::Done {
                        summary: summary.clone(),
                    },
                );
            }));
            summary.warnings.push("panic در اجرای pipeline".into());
            super::emit_to(sink, PipelineEvent::Done { summary });
            drop(panic_payload);
        }
    }
    drop(cleanup); // ترتیب قطعی: حذف control (چک run_id) → رهایی اسلات — داخل Drop
}

fn summary_from_control(
    control: &PipelineControl,
    cancelled: bool,
    status: PipelineStatus,
    message: impl Into<String>,
) -> PipelineSummary {
    let g = control.lock_inner();
    PipelineSummary {
        run_id: control.run_id().to_string(),
        status,
        cancelled,
        counts: g.counts,
        items: g.items.clone(),
        invalid_selections: g.invalid_selections.clone(),
        staged_paths: g.staged_paths.clone(),
        warnings: g.warnings.clone(),
        key_store_error: g.key_store_error.clone(),
        duration_ms: control.started.elapsed().as_millis() as u64,
        ephemeral: true,
    }
    .with_message(message.into())
}

trait WithMessage {
    fn with_message(self, message: String) -> Self;
}
impl WithMessage for PipelineSummary {
    fn with_message(mut self, message: String) -> Self {
        self.warnings.push(message);
        self
    }
}

// ---------------------------------------------------------------------
// بدنهٔ pipeline — stage barriers با per-video snapshots
// ---------------------------------------------------------------------

pub(crate) fn run_pipeline_body(
    state: &AppState,
    control: &Arc<PipelineControl>,
    input: PipelineInput,
    sink: &super::SinkStore,
    io: &PipelineIo<'_>,
) -> Result<PipelineSummary, PipelineError> {
    let started = Instant::now();
    let cancel = control.cancel_flag().clone();
    let ensure_running = || -> Result<(), PipelineError> {
        if control.is_cancelled() || cancel.load(Ordering::SeqCst) {
            Err(PipelineError::Cancelled)
        } else {
            Ok(())
        }
    };
    let mut counts = PipelineCounts::default();
    let mut warnings: Vec<String> = Vec::new();
    let mut staged_paths: Vec<String> = Vec::new();
    let mut items: Vec<ItemRuntime> = Vec::new();

    macro_rules! push_state {
        ($msg:expr) => {{
            control.set_message($msg);
            control.set_counts(counts);
            super::emit_to(
                sink,
                PipelineEvent::State {
                    snapshot: control.snapshot(),
                },
            );
        }};
    }

    // ---- S0: Validating ----
    control.set_stage(PipelineStage::Validating);
    push_state!("اعتبارسنجی ورودی و ابزارها…");
    let paths = match &input {
        PipelineInput::Files { paths } if !paths.is_empty() => paths.clone(),
        _ => return Err(PipelineError::Fatal("هیچ ویدیویی انتخاب نشده است.".into())),
    };
    let settings = state.settings.lock().unwrap().clone();
    let sync_workers = settings.processing.sync_workers.max(1) as usize;
    let _tools = resolve_all_tools(state);
    ensure_running()?;

    // ---- S1: Scanning ----
    control.set_stage(PipelineStage::Scanning);
    push_state!("اسکن دقیق فایل‌های انتخاب‌شده…");
    let scan_out = crate::scanner::scan_selected_files(
        &paths,
        &settings.excluded_folders,
        &cancel,
        &mut |p| {
            control.set_current_file(Some(p.current_path.clone()));
            control.set_message(format!("اسکن: {}", p.current_path));
            super::emit_to(
                sink,
                PipelineEvent::State {
                    snapshot: control.snapshot(),
                },
            );
        },
    );
    if scan_out.summary.cancelled {
        mark_all_cancelled(control);
        return Err(PipelineError::Cancelled);
    }
    control.set_invalid_selections(scan_out.issues.clone());
    warnings.extend(scan_out.warnings.iter().cloned());
    for entry in &scan_out.entries {
        items.push(ItemRuntime {
            video_path: entry.path.clone(),
            video_file_name: entry.file_name.clone(),
            stage: PipelineStage::Scanning,
            stage_status: StageStatus::Done,
            outcome: None,
            artifacts: ItemArtifacts::default(),
            sidecar_valid: Vec::new(),
            sidecar_conflicts: Vec::new(),
            inspection_failed: false,
            has_sidecar: false,
            archive_sources: Vec::new(),
        });
    }
    // جایگزینی کامل کتابخانه (قرارداد اسکن) + رویداد
    {
        let mut lib = state.library.lock().unwrap();
        *lib = scan_out.entries.clone();
    }
    super::emit_to(
        sink,
        PipelineEvent::LibraryReplaced {
            entries: scan_out.entries.clone(),
        },
    );
    sync_all(control, sink, &items);
    ensure_running()?;

    // ---- S2: EvaluatingExisting ----
    control.set_stage(PipelineStage::EvaluatingExisting);
    push_state!("بررسی زیرنویس‌های موجود (بازبینی تازهٔ فایل‌سیستم)…");
    for rt in items.iter_mut() {
        if cancel.load(Ordering::SeqCst) {
            mark_all_cancelled(control);
            return Err(PipelineError::Cancelled);
        }
        rt.stage = PipelineStage::EvaluatingExisting;
        let video = PathBuf::from(&rt.video_path);
        match inspect_persian_sidecars(&video) {
            Ok(inspection) => {
                let synced = inspection
                    .valid
                    .iter()
                    .any(|p| is_synced_name(&p.to_string_lossy()));
                rt.sidecar_valid = inspection.valid.clone();
                rt.sidecar_conflicts = inspection.conflicts.clone();
                rt.artifacts.sidecar_conflicts = inspection.conflicts.clone();
                rt.artifacts.valid_sidecar_paths = inspection
                    .valid
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                if synced {
                    rt.outcome = Some(ItemOutcome::AlreadySynced);
                    counts.already_synced += 1;
                    rt.stage_status = StageStatus::Done;
                } else {
                    rt.stage_status = StageStatus::Done;
                }
            }
            Err(_) => {
                rt.inspection_failed = true;
                rt.stage_status = StageStatus::Skipped {
                    reason: "بازبینی sidecarها ناموفق بود — fail-closed".into(),
                };
            }
        }
        emit_item(control, sink, rt);
    }
    set_counts_snapshot(control, &mut counts);
    ensure_running()?;

    // ---- S3: Analyzing ----
    control.set_stage(PipelineStage::Analyzing);
    push_state!("تحلیل زیرنویس‌های داخلی…");
    for rt in items.iter_mut() {
        if rt.outcome.is_some() || rt.inspection_failed || !rt.sidecar_valid.is_empty() {
            continue; // این‌ها نیازی به تحلیل ندارند
        }
        if cancel.load(Ordering::SeqCst) {
            mark_all_cancelled(control);
            return Err(PipelineError::Cancelled);
        }
        rt.stage = PipelineStage::Analyzing;
        rt.stage_status = StageStatus::InProgress;
        control.set_current_file(Some(rt.video_file_name.clone()));
        emit_item(control, sink, rt);

        match crate::commands::media::run_analyze_blocking(state, &rt.video_path, &cancel) {
            Ok(_) => {
                rt.stage_status = StageStatus::Done;
                counts.analyzed += 1;
            }
            Err(err) => {
                rt.stage_status = StageStatus::Failed {
                    reason: err.clone(),
                };
            }
        }
        emit_item(control, sink, rt);
    }
    set_counts_snapshot(control, &mut counts);
    ensure_running()?;

    // ---- S4: Extracting ----
    control.set_stage(PipelineStage::Extracting);
    push_state!("استخراج زیرنویس‌های داخلی…");
    let library: Vec<VideoEntry> = state.library.lock().unwrap().clone();
    let extraction_plan = crate::commands::subtitle::plan_extraction_from_library(&library);
    // آماده‌ها (دقیقاً یک Track فارسی) — هستهٔ موجود execute_plan_items
    if !extraction_plan.items.is_empty() {
        let total = extraction_plan.items.len() as u64;
        let (success, _failed) = crate::commands::subtitle::execute_plan_items(
            state,
            total,
            &extraction_plan.items,
            &cancel,
            |p| {
                control.set_message(format!(
                    "استخراج {}/{}: {}",
                    p.current, p.total, p.video_path
                ));
                super::emit_to(
                    sink,
                    PipelineEvent::State {
                        snapshot: control.snapshot(),
                    },
                );
            },
            |updated| super::emit_to(sink, PipelineEvent::LibraryEntry { entry: updated }),
        );
        counts.extracted += success;
        if cancel.load(Ordering::SeqCst) {
            mark_all_cancelled(control);
            return Err(PipelineError::Cancelled);
        }
    }
    // وضعیت‌های باقی بر اساس AnalysisStatus تایپ‌شده — تصمیم‌های صریح کاربر
    for rt in items.iter_mut() {
        if rt.outcome.is_some() || rt.has_sidecar {
            continue;
        }
        if cancel.load(Ordering::SeqCst) {
            mark_all_cancelled(control);
            return Err(PipelineError::Cancelled);
        }
        rt.stage = PipelineStage::Extracting;
        let Some(entry) = crate::commands::media::get_entry(state, &rt.video_path) else {
            continue;
        };
        let Some(analysis) = entry.analysis.as_ref() else {
            rt.stage_status = StageStatus::Skipped {
                reason: "تحلیل در دسترس نیست".into(),
            };
            emit_item(control, sink, rt);
            continue;
        };
        match analysis.status {
            AnalysisStatus::NeedsTrackSelection => {
                // D1: چند Track متنی فارسی → انتخاب صریح کاربر
                let payloads = text_track_options(&entry, true);
                if payloads.is_empty() {
                    rt.stage_status = StageStatus::Skipped {
                        reason: "Track متنی کاندیدا یافت نشد".into(),
                    };
                    emit_item(control, sink, rt);
                    continue;
                }
                let (vpath, vname) = (rt.video_path.clone(), rt.video_file_name.clone());
                let payload = register_and_wait(
                    control,
                    sink,
                    rt,
                    DecisionKind::EmbeddedTrack,
                    &vpath,
                    &vname,
                    payloads,
                )?;
                let stream_index = match &payload {
                    DecisionOptionPayload::EmbeddedTrack { stream_index, .. } => *stream_index,
                    _ => unreachable!("نوع تصمیم ثابت است"),
                };
                // revalidation تازه — انتخاب فقط اگر Track هنوز معتبر و فارسی‌مانند است
                let stream_ok = crate::commands::media::get_entry(state, &rt.video_path)
                    .and_then(|e| e.analysis)
                    .map(|a| {
                        a.streams.iter().any(|s| {
                            s.stream_index == stream_index
                                && s.kind == SubtitleKind::Text
                                && (s.persian_candidate || s.persian_confirmed)
                        })
                    })
                    .unwrap_or(false);
                if !stream_ok {
                    rt.stage_status = StageStatus::Failed {
                        reason: "Track انتخاب‌شده دیگر معتبر نیست".into(),
                    };
                    emit_item(control, sink, rt);
                    continue;
                }
                match crate::commands::subtitle::run_extract_blocking(
                    state,
                    &rt.video_path,
                    stream_index,
                    &cancel,
                ) {
                    Ok(extracted) => {
                        rt.has_sidecar = true;
                        rt.artifacts.sync_input_path = Some(extracted.target_path.clone());
                        rt.stage_status = StageStatus::Done;
                        counts.extracted += 1;
                    }
                    Err(err) => {
                        rt.stage_status = StageStatus::Failed {
                            reason: err.clone(),
                        };
                    }
                }
                emit_item(control, sink, rt);
            }
            AnalysisStatus::NoPersianTrack => {
                // مسیر review: کاربر Track متنی را برای بررسی محتوا انتخاب می‌کند
                let payloads = text_track_options(&entry, false);
                if payloads.is_empty() {
                    rt.stage_status = StageStatus::Skipped {
                        reason: "Track متنی برای بررسی نیست".into(),
                    };
                    emit_item(control, sink, rt);
                    continue;
                }
                let (vpath, vname) = (rt.video_path.clone(), rt.video_file_name.clone());
                let payload = register_and_wait(
                    control,
                    sink,
                    rt,
                    DecisionKind::ReviewTrack,
                    &vpath,
                    &vname,
                    payloads,
                )?;
                let stream_index = match &payload {
                    DecisionOptionPayload::ReviewTrack { stream_index, .. } => *stream_index,
                    _ => unreachable!("نوع تصمیم ثابت است"),
                };
                match crate::commands::subtitle::run_review_blocking(
                    state,
                    &rt.video_path,
                    stream_index,
                    &cancel,
                ) {
                    Ok(review) if review.persian_confirmed => {
                        match crate::commands::subtitle::promote_review_core(
                            state,
                            &rt.video_path,
                            stream_index,
                        ) {
                            Ok(updated) => {
                                rt.has_sidecar = true;
                                rt.artifacts.sync_input_path = updated
                                    .analysis
                                    .as_ref()
                                    .and_then(|a| a.last_extraction.as_ref())
                                    .map(|e| e.target_path.clone());
                                rt.stage_status = StageStatus::Done;
                                counts.extracted += 1;
                                super::emit_to(
                                    sink,
                                    PipelineEvent::LibraryEntry { entry: updated },
                                );
                            }
                            Err(err) => {
                                rt.stage_status = StageStatus::Failed {
                                    reason: err.clone(),
                                };
                            }
                        }
                    }
                    Ok(_review) => {
                        rt.stage_status = StageStatus::Skipped {
                            reason: "بررسی محتوا فارسی تأیید نکرد".into(),
                        };
                    }
                    Err(err) => {
                        rt.stage_status = StageStatus::Failed {
                            reason: err.clone(),
                        };
                    }
                }
                emit_item(control, sink, rt);
            }
            AnalysisStatus::ImageSubtitleFound => {
                rt.stage_status = StageStatus::Skipped {
                    reason: "زیرنویس تصویری — OCR پشتیبانی نمی‌شود".into(),
                };
                emit_item(control, sink, rt);
            }
            AnalysisStatus::NoEmbeddedSubtitle => {
                rt.stage_status = StageStatus::Skipped {
                    reason: "زیرنویس داخلی ندارد".into(),
                };
                emit_item(control, sink, rt);
            }
            AnalysisStatus::Extracted => {
                rt.has_sidecar = true;
                // ورودی سینک از استخراجِ همین اجرا — بدون این، ویدیو بدون
                // outcome از مرحلهٔ سینک عبور می‌کند
                if let Some(extraction) = entry
                    .analysis
                    .as_ref()
                    .and_then(|a| a.last_extraction.as_ref())
                {
                    rt.artifacts.sync_input_path = Some(extraction.target_path.clone());
                }
                rt.stage_status = StageStatus::Done;
                emit_item(control, sink, rt);
            }
            _ => {}
        }
        ensure_running()?;
    }
    refresh_sidecar_flags(state, &mut items, control, sink);
    set_counts_snapshot(control, &mut counts);
    ensure_running()?;

    // ---- S5: Downloading ----
    control.set_stage(PipelineStage::Downloading);
    push_state!("دانلود زیرنویس‌های جاافتاده…");
    let needer_idxs: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, rt)| {
            rt.outcome.is_none()
                && !rt.has_sidecar
                && rt.sidecar_valid.is_empty()
                && !rt.inspection_failed
        })
        .map(|(i, _)| i)
        .collect();

    if !needer_idxs.is_empty() {
        match (io.key_provider)() {
            Err(err) => {
                // خطای مخزن کلید ≠ «کلید تنظیم نشده» — جدا و صادقانه گزارش می‌شود
                control.set_key_store_error(Some(err.clone()));
                for &i in &needer_idxs {
                    items[i].stage = PipelineStage::Downloading;
                    items[i].outcome = Some(ItemOutcome::KeyStoreUnavailable {
                        reason: err.clone(),
                    });
                    items[i].stage_status = StageStatus::Done;
                    counts.key_store_unavailable += 1;
                }
            }
            Ok(None) => {
                for &i in &needer_idxs {
                    items[i].stage = PipelineStage::Downloading;
                    items[i].outcome = Some(ItemOutcome::NeedsApiKey);
                    items[i].stage_status = StageStatus::Done;
                    counts.needs_api_key += 1;
                }
            }
            Ok(Some(key)) if key.trim().is_empty() => {
                for &i in &needer_idxs {
                    items[i].stage = PipelineStage::Downloading;
                    items[i].outcome = Some(ItemOutcome::NeedsApiKey);
                    items[i].stage_status = StageStatus::Done;
                    counts.needs_api_key += 1;
                }
            }
            Ok(Some(key)) => {
                match crate::commands::media::resolve_tool(state, "subsource") {
                    Err(err) => {
                        for &i in &needer_idxs {
                            items[i].stage = PipelineStage::Downloading;
                            items[i].outcome = Some(ItemOutcome::Skipped {
                                reason: format!("ابزار SubSource در دسترس نیست: {err}"),
                            });
                            counts.skipped += 1;
                        }
                    }
                    Ok(_sub_exe) => {
                        let needer_paths: Vec<String> = needer_idxs
                            .iter()
                            .map(|&i| items[i].video_path.clone())
                            .collect();
                        let library = state.library.lock().unwrap().clone();
                        let plan = create_download_plan(&library, &needer_paths);
                        for skipped in &plan.skipped {
                            if let Some(rt) = items
                                .iter_mut()
                                .find(|rt| rt.video_path == skipped.video_path)
                            {
                                map_plan_skip(rt, skipped, &mut counts);
                            }
                        }
                        let result = crate::commands::subsource::run_download_blocking(
                            state,
                            &plan.items,
                            Some(key),
                            io.staging_base,
                            &cancel,
                            &mut |p| {
                                control.set_message(p.message.clone());
                                super::emit_to(
                                    sink,
                                    PipelineEvent::State {
                                        snapshot: control.snapshot(),
                                    },
                                );
                            },
                        );
                        control.set_staged_paths(result.staging_paths.clone());
                        staged_paths.extend(result.staging_paths.iter().cloned());
                        counts.downloaded += result.staged as u64;
                        for skipped in &result.skipped {
                            if let Some(rt) = items
                                .iter_mut()
                                .find(|rt| rt.video_path == skipped.video_path)
                            {
                                map_runtime_skip(rt, skipped, &mut counts);
                            }
                        }
                        for source in &result.archive_sources {
                            if let Some(rt) = items
                                .iter_mut()
                                .find(|rt| rt.video_path == source.video_path)
                            {
                                rt.archive_sources.push(source.clone());
                                rt.stage_status = StageStatus::Done;
                            }
                        }
                        // صداقت گزارش: آیتم‌های پلن‌شده که بعد از دانلود بدون
                        // نتیجه ماندند — دلیل واقعی دانلود را حمل می‌کنند، نه
                        // یک پیام گیج‌کننده از فاز آرشیو.
                        if !result.cancelled {
                            for item in &plan.items {
                                let Some(rt) =
                                    items.iter_mut().find(|rt| rt.video_path == item.video_path)
                                else {
                                    continue;
                                };
                                if rt.outcome.is_some() {
                                    continue;
                                }
                                let has_real_output =
                                    rt.archive_sources.iter().any(archive_output_dir_exists);
                                if !result.success && !has_real_output {
                                    // دانلود شکست خورده و هیچ خروجی واقعی نداشته —
                                    // فاز آرشیو برایش بی‌معناست؛ دلیل اصلی حفظ می‌شود
                                    rt.outcome = Some(ItemOutcome::Failed {
                                        reason: format!(
                                            "دانلود ناموفق بود: {} (کد خروج: {})",
                                            result.message,
                                            result
                                                .exit_code
                                                .map(|c| c.to_string())
                                                .unwrap_or_else(|| "—".into()),
                                        ),
                                    });
                                    counts.failed += 1;
                                    rt.archive_sources.clear();
                                } else if result.success && !has_real_output {
                                    rt.outcome = Some(ItemOutcome::Skipped {
                                        reason:
                                            "دانلود انجام شد اما زیرنویسی برای این ویدیو یافت نشد."
                                                .into(),
                                    });
                                    counts.skipped += 1;
                                }
                                rt.stage_status = StageStatus::Done;
                            }
                            if !result.success {
                                control.push_warning(format!(
                                    "دانلود با خطا مواجه شد: {}",
                                    result.message
                                ));
                            }
                        }
                    }
                }
            }
        }
        // emit همهٔ شاخه‌ها — outcomeهای ست‌شدهٔ این مرحله باید به snapshot/خلاصه برسند
        sync_all(control, sink, &items);
        if cancel.load(Ordering::SeqCst) {
            mark_all_cancelled(control);
            return Err(PipelineError::Cancelled);
        }
        set_counts_snapshot(control, &mut counts);
    }
    ensure_running()?;

    // ---- S6: PlanningArchives ----
    control.set_stage(PipelineStage::PlanningArchives);
    push_state!("بررسی آرشیوهای دانلودشده…");
    let all_sources: Vec<_> = items
        .iter()
        .flat_map(|rt| rt.archive_sources.iter().cloned())
        .collect();
    let mut seven_zip: Option<PathBuf> = None;
    if !all_sources.is_empty() {
        match crate::commands::media::resolve_tool(state, "sevenZip") {
            Err(err) => {
                for rt in items.iter_mut() {
                    if !rt.archive_sources.is_empty() {
                        rt.stage = PipelineStage::PlanningArchives;
                        rt.stage_status = StageStatus::Skipped {
                            reason: format!("ابزار 7-Zip در دسترس نیست: {err}"),
                        };
                    }
                }
                sync_all(control, sink, &items);
            }
            Ok(exe) => {
                seven_zip = Some(exe.clone());
                let plan =
                    crate::archive::engine::plan_archives(&exe, &all_sources, &cancel, &mut |p| {
                        control.set_message(p.message.clone());
                        super::emit_to(
                            sink,
                            PipelineEvent::State {
                                snapshot: control.snapshot(),
                            },
                        );
                    });
                for plan_item in &plan.items {
                    let Some(pos) = items
                        .iter()
                        .position(|rt| rt.video_path == plan_item.video_path)
                    else {
                        continue;
                    };
                    items[pos].stage = PipelineStage::PlanningArchives;
                    use crate::archive::types::ArchivePlanStatus as APS;
                    match plan_item.status {
                        APS::Ready => {
                            items[pos].stage_status = StageStatus::Done;
                            if let Some(sel) = &plan_item.selection {
                                items[pos].artifacts.pre_selection = Some(PreSelectionArtifact {
                                    archive_file_name: sel.archive_file_name.clone(),
                                    member_path: sel.member_path.clone(),
                                });
                            }
                        }
                        APS::Ambiguous => {
                            // D2: چند کاندیدای آرشیو → انتخاب صریح کاربر
                            let payloads: Vec<DecisionOptionPayload> = plan_item
                                .candidates
                                .iter()
                                .map(|c| DecisionOptionPayload::ArchiveCandidate {
                                    archive_file_name: c.archive_file_name.clone(),
                                    member_path: c.member_path.clone(),
                                })
                                .collect();
                            let (vpath, vname) = (
                                plan_item.video_path.clone(),
                                plan_item.video_file_name.clone(),
                            );
                            let resolved = register_and_wait(
                                control,
                                sink,
                                &mut items[pos],
                                DecisionKind::ArchiveCandidate,
                                &vpath,
                                &vname,
                                payloads,
                            )?;
                            if let DecisionOptionPayload::ArchiveCandidate {
                                archive_file_name,
                                member_path,
                            } = &resolved
                            {
                                items[pos].artifacts.pre_selection = Some(PreSelectionArtifact {
                                    archive_file_name: archive_file_name.clone(),
                                    member_path: member_path.clone(),
                                });
                            }
                            items[pos].stage_status = StageStatus::Done;
                        }
                        APS::NoMatch => {
                            items[pos].stage_status = StageStatus::Skipped {
                                reason: "در آرشیوهای دانلودشده منطبق فارسی یافت نشد".into(),
                            };
                        }
                    }
                    sync_one(control, sink, &items, pos);
                }
            }
        }
        if cancel.load(Ordering::SeqCst) {
            mark_all_cancelled(control);
            return Err(PipelineError::Cancelled);
        }
        sync_all(control, sink, &items);
    }
    ensure_running()?;

    // ---- S7: ExtractingArchives ----
    control.set_stage(PipelineStage::ExtractingArchives);
    push_state!("استخراج زیرنویس از آرشیوها…");
    if let Some(exe) = &seven_zip {
        let requests: Vec<crate::archive::types::ArchiveExecuteRequest> = items
            .iter()
            .filter(|rt| !rt.archive_sources.is_empty())
            .filter_map(|rt| {
                let source = rt.archive_sources.first()?.clone();
                let selection = rt.artifacts.pre_selection.as_ref().map(|pre| {
                    crate::archive::types::ArchiveSelection {
                        archive_file_name: pre.archive_file_name.clone(),
                        member_path: pre.member_path.clone(),
                    }
                });
                Some(crate::archive::types::ArchiveExecuteRequest { source, selection })
            })
            .collect();
        if !requests.is_empty() {
            let result = crate::archive::engine::execute_archive_extraction(
                exe,
                &requests,
                &cancel,
                &mut |p| {
                    control.set_message(p.message.clone());
                    super::emit_to(
                        sink,
                        PipelineEvent::State {
                            snapshot: control.snapshot(),
                        },
                    );
                },
            );
            for item in &result.items {
                let Some(pos) = items.iter().position(|rt| rt.video_path == item.video_path) else {
                    continue;
                };
                items[pos].stage = PipelineStage::ExtractingArchives;
                use crate::archive::types::ArchiveItemStatus as AIS;
                match item.status {
                    AIS::Placed => {
                        items[pos].stage_status = StageStatus::Done;
                        items[pos].has_sidecar = true;
                        counts.archives_placed += 1;
                        if let Some(out) = &item.output_path {
                            items[pos].artifacts.sync_input_path = Some(out.clone());
                        }
                        refresh_entry_siblings(state, &items[pos].video_path, sink);
                    }
                    AIS::Conflict => {
                        items[pos].outcome = Some(ItemOutcome::OutputConflict {
                            preserved_path: item.output_path.clone().unwrap_or_default(),
                        });
                        items[pos].stage_status = StageStatus::Done;
                        counts.conflicts += 1;
                    }
                    AIS::Ambiguous => {
                        // نتیجهٔ پایانی صادقانه — نه Skipped عمومی از مرحلهٔ بعد
                        items[pos].outcome = Some(ItemOutcome::Skipped {
                            reason: "کاندیداهای آرشیو همچنان مبهم‌اند".into(),
                        });
                        items[pos].stage_status = StageStatus::Done;
                        counts.skipped += 1;
                    }
                    AIS::NoMatch => {
                        items[pos].outcome = Some(ItemOutcome::Skipped {
                            reason: "منطبقی یافت نشد".into(),
                        });
                        items[pos].stage_status = StageStatus::Done;
                        counts.skipped += 1;
                    }
                    AIS::Failed => {
                        // شکست استخراج آرشیو = شکست واقعی آیتم؛ دلیل اصلی حفظ می‌شود
                        items[pos].outcome = Some(ItemOutcome::Failed {
                            reason: item.message.clone(),
                        });
                        items[pos].stage_status = StageStatus::Done;
                        counts.failed += 1;
                    }
                    AIS::Cancelled => {}
                }
                sync_one(control, sink, &items, pos);
            }
        }
        sync_all(control, sink, &items);
        if cancel.load(Ordering::SeqCst) {
            mark_all_cancelled(control);
            return Err(PipelineError::Cancelled);
        }
    }
    ensure_running()?;

    // ---- S8: SelectingSyncInput ----
    control.set_stage(PipelineStage::SelectingSyncInput);
    push_state!("انتخاب ورودی سینک…");
    for rt in items.iter_mut() {
        if rt.outcome.is_some() || rt.has_sidecar || rt.artifacts.sync_input_path.is_some() {
            continue; // ورودی معلوم دارد یا نهایی است
        }
        if cancel.load(Ordering::SeqCst) {
            mark_all_cancelled(control);
            return Err(PipelineError::Cancelled);
        }
        rt.stage = PipelineStage::SelectingSyncInput;
        let video = PathBuf::from(&rt.video_path);
        match inspect_persian_sidecars(&video) {
            Err(_) => {
                rt.outcome = Some(ItemOutcome::Skipped {
                    reason: "بازبینی sidecarها ناموفق بود — fail-closed".into(),
                });
                counts.skipped += 1;
            }
            Ok(inspection) => {
                rt.artifacts.sidecar_conflicts = inspection.conflicts.clone();
                rt.artifacts.valid_sidecar_paths = inspection
                    .valid
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                let synced = inspection
                    .valid
                    .iter()
                    .any(|p| is_synced_name(&p.to_string_lossy()));
                if synced {
                    rt.outcome = Some(ItemOutcome::AlreadySynced);
                    counts.already_synced += 1;
                    rt.stage_status = StageStatus::Done;
                } else {
                    let normal: Vec<PathBuf> = inspection
                        .valid
                        .iter()
                        .filter(|p| !is_synced_name(&p.to_string_lossy()))
                        .cloned()
                        .collect();
                    match normal.len() {
                        1 => {
                            // دقیقاً یک کاندیدا — انتخاب خودکار مجاز
                            rt.artifacts.sync_input_path =
                                Some(normal[0].to_string_lossy().into_owned());
                            rt.stage_status = StageStatus::Done;
                        }
                        0 => {
                            rt.outcome = Some(ItemOutcome::Skipped {
                                reason: "ورودی سینک معتبری یافت نشد".into(),
                            });
                            counts.skipped += 1;
                        }
                        _ => {
                            // ≥۲ → تصمیم صریح کاربر — هیچ انتخاب خودکار
                            rt.stage_status = StageStatus::NeedsDecision {
                                decision_id: String::new(),
                            };
                            emit_item(control, sink, rt);
                            let payloads = normal
                                .iter()
                                .map(|p| DecisionOptionPayload::SidecarPath { path: p.clone() })
                                .collect();
                            let (vpath, vname) =
                                (rt.video_path.clone(), rt.video_file_name.clone());
                            let resolved = register_and_wait(
                                control,
                                sink,
                                rt,
                                DecisionKind::SyncSidecar,
                                &vpath,
                                &vname,
                                payloads,
                            )?;
                            let chosen = match &resolved {
                                DecisionOptionPayload::SidecarPath { path } => path.clone(),
                                _ => unreachable!("نوع تصمیم ثابت است"),
                            };
                            // revalidation تازه — fail-closed
                            let validation = crate::subtitle::validation::validate_file(&chosen);
                            if chosen.is_file() && validation.valid && validation.has_persian {
                                rt.artifacts.sync_input_path =
                                    Some(chosen.to_string_lossy().into_owned());
                                rt.stage_status = StageStatus::Done;
                            } else {
                                rt.stage_status = StageStatus::Failed {
                                    reason: "کاندیدای انتخاب‌شده دیگر معتبر نیست".into(),
                                };
                            }
                        }
                    }
                }
            }
        }
        emit_item(control, sink, rt);
        ensure_running()?;
    }

    // ---- S9: Syncing — pool باند به syncWorkers ----
    control.set_stage(PipelineStage::Syncing);
    push_state!("همگام‌سازی زیرنویس‌ها…");
    let ffs_res = crate::commands::media::resolve_tool(state, "ffsubsync");
    let ffmpeg_res = crate::commands::media::resolve_tool(state, "ffmpeg");
    let jobs: Vec<(usize, PathBuf, PathBuf)> = items
        .iter()
        .enumerate()
        .filter(|(_, rt)| {
            rt.outcome.is_none()
                && rt
                    .artifacts
                    .sync_input_path
                    .as_deref()
                    .map(|p| !p.is_empty())
                    .unwrap_or(false)
        })
        .map(|(i, rt)| {
            (
                i,
                PathBuf::from(&rt.video_path),
                PathBuf::from(rt.artifacts.sync_input_path.clone().unwrap()),
            )
        })
        .collect();

    let mut sync_outcomes: HashMap<usize, crate::sync::SyncOutcome> = HashMap::new();
    match (&ffs_res, &ffmpeg_res) {
        (Ok(ffs_exe), Ok(ffmpeg_exe)) if !jobs.is_empty() => {
            let config = crate::sync::SyncConfig::from(&state.settings.lock().unwrap().processing);
            let queue: Arc<Mutex<VecDeque<usize>>> =
                Arc::new(Mutex::new(jobs.iter().map(|(i, _, _)| *i).collect()));
            let requests: Arc<Mutex<HashMap<usize, crate::sync::SyncRequest>>> =
                Arc::new(Mutex::new(
                    jobs.iter()
                        .map(|(i, video, sub)| {
                            (
                                *i,
                                crate::sync::SyncRequest {
                                    video_path: video.clone(),
                                    subtitle_path: sub.clone(),
                                    ffsubsync_path: ffs_exe.clone(),
                                    ffmpeg_path: Some(ffmpeg_exe.clone()),
                                    config,
                                    workspace_root: None,
                                },
                            )
                        })
                        .collect(),
                ));
            let outcomes: Arc<Mutex<HashMap<usize, crate::sync::SyncOutcome>>> = Default::default();
            let workers = sync_workers.min(jobs.len());
            let mut pool = Vec::new();
            let _ = (&ffs_res, &ffmpeg_res);
            for _ in 0..workers {
                let queue = Arc::clone(&queue);
                let requests = Arc::clone(&requests);
                let outcomes = Arc::clone(&outcomes);
                let cancel = Arc::clone(&cancel);
                let control = Arc::clone(control);
                pool.push(std::thread::spawn(move || loop {
                    let Some(idx) = queue
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .pop_front()
                    else {
                        break;
                    };
                    if cancel.load(Ordering::SeqCst) {
                        outcomes
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .insert(
                                idx,
                                crate::sync::SyncOutcome::bare(
                                    crate::sync::SyncStatus::Cancelled,
                                    "لغو شد",
                                ),
                            );
                        continue;
                    }
                    let Some(request) = requests
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&idx)
                        .cloned()
                    else {
                        break;
                    };
                    let outcome = crate::sync::run_sync(&request, &cancel, &mut |p| {
                        control.set_message(p.message.clone());
                    });
                    outcomes
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(idx, outcome);
                }));
            }
            for handle in pool {
                let _ = handle.join();
            }
            {
                let mut map = outcomes.lock().unwrap_or_else(PoisonError::into_inner);
                for (idx, outcome) in map.drain() {
                    sync_outcomes.insert(idx, outcome);
                }
            }
        }
        (Err(err), _) | (_, Err(err)) if !jobs.is_empty() => {
            for (idx, _, _) in &jobs {
                items[*idx].stage_status = StageStatus::Skipped {
                    reason: format!("ابزار سینک در دسترس نیست: {err}"),
                };
            }
            sync_all(control, sink, &items);
        }
        _ => {}
    }
    if cancel.load(Ordering::SeqCst) {
        mark_all_cancelled(control);
        return Err(PipelineError::Cancelled);
    }
    ensure_running()?;

    // ---- S10: Finalizing — جانمایی no-overwrite ----
    control.set_stage(PipelineStage::Finalizing);
    push_state!("جانمایی خروجی‌های سینک‌شده…");
    for (idx, outcome) in sync_outcomes {
        use crate::sync::SyncStatus as SS;
        let rt = &mut items[idx];
        rt.stage = PipelineStage::Finalizing;
        match outcome.status {
            SS::Synced => {
                let Some(candidate) = outcome.output_path.clone() else {
                    rt.outcome = Some(ItemOutcome::Failed {
                        reason: "سینک موفق بود اما مسیر کاندیدا گم شد".into(),
                    });
                    counts.failed += 1;
                    rt.stage_status = StageStatus::Done;
                    continue;
                };
                let video = PathBuf::from(&rt.video_path);
                let target = video.parent().unwrap_or(Path::new(".")).join(format!(
                    "{}.fa.synced.srt",
                    video.file_stem().unwrap_or_default().to_string_lossy()
                ));
                match crate::archive::placement::place_subtitle(&candidate, &target) {
                    Ok(_) => {
                        // workspace فقط پس از جانمایی موفق حذف می‌شود
                        let mut placed_outcome = outcome;
                        let discard_warnings = placed_outcome
                            .take_workspace()
                            .map(|handle| handle.discard())
                            .unwrap_or_default();
                        for warning in discard_warnings {
                            control.push_warning(warning);
                        }
                        rt.outcome = Some(ItemOutcome::Synced {
                            output_path: target.to_string_lossy().into_owned(),
                        });
                        rt.artifacts.placed_path = Some(target.to_string_lossy().into_owned());
                        rt.artifacts.offset_seconds = placed_outcome.offset_seconds;
                        counts.synced += 1;
                        refresh_entry_siblings(state, &rt.video_path, sink);
                    }
                    Err(crate::archive::placement::PlacementError::TargetExists(preserved)) => {
                        // تعارض: مقصد + کاندیدا هر دو حفظ می‌شوند؛ هیچ حذفی
                        rt.outcome = Some(ItemOutcome::OutputConflict {
                            preserved_path: preserved.to_string_lossy().into_owned(),
                        });
                        rt.artifacts.conflict_path = Some(preserved.to_string_lossy().into_owned());
                        rt.artifacts.sync_candidate_path = outcome
                            .workspace_display_root()
                            .map(|p| p.to_string_lossy().into_owned());
                        counts.conflicts += 1;
                        rt.stage_status = StageStatus::Done;
                    }
                    Err(err) => {
                        rt.outcome = Some(ItemOutcome::Failed {
                            reason: format!("جانمایی ناموفق: {err}"),
                        });
                        rt.artifacts.sync_candidate_path = outcome
                            .workspace_display_root()
                            .map(|p| p.to_string_lossy().into_owned());
                        counts.failed += 1;
                        rt.stage_status = StageStatus::Done;
                    }
                }
            }
            SS::UnchangedLowQuality => {
                rt.outcome = Some(ItemOutcome::UnchangedLowQuality);
                counts.unchanged_low_quality += 1;
                rt.stage_status = StageStatus::Done;
            }
            SS::Cancelled => {} // لغو سراسری پایین‌تر مدیریت می‌شود
            _ => {
                rt.outcome = Some(ItemOutcome::Failed {
                    reason: outcome.diagnostic.clone(),
                });
                counts.failed += 1;
                rt.stage_status = StageStatus::Done;
            }
        }
        emit_item(control, sink, rt);
    }

    // ---- Done ----
    control.set_stage(PipelineStage::Done);
    control.set_status(PipelineStatus::Completed);
    set_counts_snapshot(control, &mut counts);
    let duration_ms = started.elapsed().as_millis() as u64;
    control.set_message("pipeline کامل شد.");
    let summary = {
        let g = control.lock_inner();
        PipelineSummary {
            run_id: control.run_id().to_string(),
            status: PipelineStatus::Completed,
            cancelled: false,
            counts: g.counts,
            items: g.items.clone(),
            invalid_selections: g.invalid_selections.clone(),
            staged_paths: g.staged_paths.clone(),
            warnings: g.warnings.clone(),
            key_store_error: g.key_store_error.clone(),
            duration_ms,
            ephemeral: true,
        }
    };
    super::emit_to(
        sink,
        PipelineEvent::State {
            snapshot: control.snapshot(),
        },
    );
    Ok(summary)
}

// ---------------------------------------------------------------------
// helperها
// ---------------------------------------------------------------------

fn resolve_all_tools(state: &AppState) -> HashMap<&'static str, Option<PathBuf>> {
    let mut map = HashMap::new();
    for id in ["ffprobe", "ffmpeg", "subsource", "sevenZip", "ffsubsync"] {
        map.insert(id, crate::commands::media::resolve_tool(state, id).ok());
    }
    map
}

fn set_counts_snapshot(control: &PipelineControl, counts: &mut PipelineCounts) {
    counts.decisions_pending = 0;
    control.set_counts(*counts);
}

/// آیا خروجی دانلود برای این منبع واقعاً روی دیسک هست؟ ثبت provenance پیش
/// از اجرای CLI انجام می‌شود؛ پس وجود «پوشهٔ sub» در staging تنها نشانهٔ
/// خروجی واقعی است — نه خود رکورد provenance.
fn archive_output_dir_exists(source: &crate::subsource::plan::ArchiveSource) -> bool {
    let dir = Path::new(&source.staging_root)
        .join(&source.relative_video_dir)
        .join("sub");
    dir.is_dir()
}

fn is_synced_name(file_name: &str) -> bool {
    let lower = file_name.to_lowercase();
    lower.contains(".fa.synced.") || lower.ends_with(".fa.synced.srt")
}

/// emit آیتم مستقیماً از rt قابل-mut — برای حلقه‌های iter_mut.
fn emit_item(control: &PipelineControl, sink: &super::SinkStore, rt: &ItemRuntime) {
    let snapshot = rt.snapshot();
    control.put_item_snapshot(snapshot.clone());
    super::emit_to(sink, PipelineEvent::Item { item: snapshot });
}

fn sync_all(control: &PipelineControl, sink: &super::SinkStore, items: &[ItemRuntime]) {
    for rt in items {
        emit_item(control, sink, rt);
    }
}

fn sync_one(control: &PipelineControl, sink: &super::SinkStore, items: &[ItemRuntime], idx: usize) {
    let snapshot = items[idx].snapshot();
    control.put_item_snapshot(snapshot.clone());
    super::emit_to(sink, PipelineEvent::Item { item: snapshot });
}

fn mark_all_cancelled(control: &PipelineControl) {
    let mut g = control.lock_inner();
    for item in &mut g.items {
        if item.outcome.is_none() {
            item.outcome = Some(ItemOutcome::Cancelled);
            item.stage_status = StageStatus::Done;
        }
    }
    g.status = PipelineStatus::Cancelled;
    g.counts.decisions_pending = 0;
    drop(g);
    control.set_message("pipeline لغو شد.");
}

/// فقط وضعیت واقعی کتابخانه را می‌بیند — Extracted → sidecar تازه موجود است.
fn refresh_sidecar_flags(
    state: &AppState,
    items: &mut [ItemRuntime],
    control: &PipelineControl,
    sink: &super::SinkStore,
) {
    for rt in items.iter_mut() {
        if rt.outcome.is_some() {
            continue;
        }
        // آیتم‌های has_sidecar بدون ورودی سینک هم باید ورودی‌شان از
        // last_extraction پر شود (نه فقط آیتم‌های بدون sidecar)
        if rt.has_sidecar && rt.artifacts.sync_input_path.is_some() {
            continue;
        }
        let Some(entry) = crate::commands::media::get_entry(state, &rt.video_path) else {
            continue;
        };
        if entry
            .analysis
            .as_ref()
            .map(|a| a.status == AnalysisStatus::Extracted)
            .unwrap_or(false)
        {
            rt.has_sidecar = true;
            rt.stage_status = StageStatus::Done;
            if let Some(extraction) = entry
                .analysis
                .as_ref()
                .and_then(|a| a.last_extraction.as_ref())
            {
                rt.artifacts.sync_input_path = Some(extraction.target_path.clone());
            }
            emit_item(control, sink, rt);
        }
    }
}

fn refresh_entry_siblings(state: &AppState, video_path: &str, sink: &super::SinkStore) {
    let Some(mut entry) = crate::commands::media::get_entry(state, video_path) else {
        return;
    };
    let parent = PathBuf::from(&entry.parent_dir);
    let stem = Path::new(&entry.file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    entry.sibling_subtitles = crate::scanner::scan_sibling_subtitles(&parent, &stem);
    crate::commands::media::apply_entry(state, entry.clone());
    super::emit_to(sink, PipelineEvent::LibraryEntry { entry });
}

/// نگاشت skipهای پلن (typed) به آیتم — هیچ parse رشته‌ای.
fn map_plan_skip(rt: &mut ItemRuntime, skipped: &SkippedDownload, counts: &mut PipelineCounts) {
    rt.stage = PipelineStage::Downloading;
    match skipped.code {
        SkipReasonCode::ValidSidecarExists => {
            // sidecar معتبر موجود → به SelectingSyncInput می‌رود (نه دانلود)
            rt.sidecar_valid = skipped
                .valid_sidecar_paths
                .iter()
                .map(PathBuf::from)
                .collect();
            rt.stage_status = StageStatus::Done;
        }
        SkipReasonCode::SidecarConflict => {
            rt.outcome = Some(ItemOutcome::Skipped {
                reason: skipped.reason.clone(),
            });
            rt.artifacts.sidecar_conflicts = skipped.sidecar_conflicts.clone();
            counts.skipped += 1;
            rt.stage_status = StageStatus::Done;
        }
        SkipReasonCode::SidecarInspectionFailed => {
            rt.outcome = Some(ItemOutcome::Skipped {
                reason: skipped.reason.clone(),
            });
            counts.skipped += 1;
            rt.stage_status = StageStatus::Done;
        }
        SkipReasonCode::MissingAtExecution => {
            rt.outcome = Some(ItemOutcome::Skipped {
                reason: skipped.reason.clone(),
            });
            counts.skipped += 1;
            rt.stage_status = StageStatus::Done;
        }
        _ => {
            rt.outcome = Some(ItemOutcome::Skipped {
                reason: skipped.reason.clone(),
            });
            counts.skipped += 1;
            rt.stage_status = StageStatus::Done;
        }
    }
}

/// نگاشت skipهای runtime (بین پلن و اجرا).
fn map_runtime_skip(rt: &mut ItemRuntime, skipped: &SkippedDownload, counts: &mut PipelineCounts) {
    rt.stage = PipelineStage::Downloading;
    match skipped.code {
        SkipReasonCode::SidecarAppearedAtExecution => {
            // sidecar معتبر ظاهر شد → به SelectingSyncInput می‌رود، نه skip نهایی
            rt.sidecar_valid = skipped
                .valid_sidecar_paths
                .iter()
                .map(PathBuf::from)
                .collect();
            rt.stage_status = StageStatus::Done;
        }
        SkipReasonCode::SidecarConflict => {
            rt.outcome = Some(ItemOutcome::Skipped {
                reason: skipped.reason.clone(),
            });
            rt.artifacts.sidecar_conflicts = skipped.sidecar_conflicts.clone();
            counts.skipped += 1;
            rt.stage_status = StageStatus::Done;
        }
        SkipReasonCode::MissingAtExecution => {
            rt.outcome = Some(ItemOutcome::Skipped {
                reason: skipped.reason.clone(),
            });
            counts.skipped += 1;
            rt.stage_status = StageStatus::Done;
        }
        _ => {
            rt.outcome = Some(ItemOutcome::Skipped {
                reason: skipped.reason.clone(),
            });
            counts.skipped += 1;
            rt.stage_status = StageStatus::Done;
        }
    }
}

/// گزینه‌های Track متنی — fa_only برای انتخاب استخراج، غیر آن برای review.
fn text_track_options(entry: &VideoEntry, fa_only: bool) -> Vec<DecisionOptionPayload> {
    entry
        .analysis
        .as_ref()
        .map(|analysis| {
            analysis
                .streams
                .iter()
                .filter(|stream| stream.kind == SubtitleKind::Text)
                .filter(|stream| !fa_only || (stream.persian_candidate || stream.persian_confirmed))
                .map(|stream| {
                    let label = format!(
                        "#{} {} [{}] {}",
                        stream.stream_index,
                        stream.codec_name,
                        stream.language.clone().unwrap_or_else(|| "?".into()),
                        stream.title.clone().unwrap_or_else(|| "—".into())
                    );
                    if fa_only {
                        DecisionOptionPayload::EmbeddedTrack {
                            stream_index: stream.stream_index,
                            label,
                        }
                    } else {
                        DecisionOptionPayload::ReviewTrack {
                            stream_index: stream.stream_index,
                            label,
                        }
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// ثبت تصمیم + انتظار — با به‌روزرسانی وضعیت آیتم و رویدادها.
fn register_and_wait(
    control: &PipelineControl,
    sink: &super::SinkStore,
    rt: &mut ItemRuntime,
    kind: DecisionKind,
    video_path: &str,
    video_file_name: &str,
    payloads: Vec<DecisionOptionPayload>,
) -> Result<DecisionOptionPayload, PipelineError> {
    let decision_id = format!("dec-{}-{}", next_decision_id(), control.run_id());
    let decision = control.register_decision(
        decision_id.clone(),
        kind,
        video_path,
        video_file_name,
        payloads,
    );
    rt.stage_status = StageStatus::NeedsDecision {
        decision_id: decision.decision_id.clone(),
    };
    emit_item(control, sink, rt);
    super::emit_to(
        sink,
        PipelineEvent::State {
            snapshot: control.snapshot(),
        },
    );
    match control.wait_for_decision(&decision.decision_id) {
        WaitExit::Resolved(payload) => {
            control.clear_pending_decision(&decision.decision_id);
            Ok(payload)
        }
        WaitExit::Cancelled => Err(PipelineError::Cancelled),
    }
}

fn next_decision_id() -> u64 {
    use std::sync::atomic::AtomicU64;
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod engine_tests;
