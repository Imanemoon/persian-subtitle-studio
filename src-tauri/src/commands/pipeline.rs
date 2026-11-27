//! دستورات Tauri برای pipeline — admission، publish، لغو، resolve تصمیم و
//! snapshot؛ mapping رویدادها به فرانت.

use std::sync::{Arc, Mutex, PoisonError};

use tauri::{AppHandle, Emitter, Manager, State};

use crate::pipeline::{
    cancel_active_pipeline, clear_published_control, resolve_decision_core, take_guard_once,
    GuardHandoff, PipelineControl, PipelineEvent, PipelineInput, PipelineItemSnapshot,
    PipelineSnapshot,
};
use crate::state::{acquire_batch_op, AppState};

fn publish_state_sink(app: &AppHandle) -> crate::pipeline::SinkStore {
    let app = app.clone();
    Arc::new(Mutex::new(Box::new(move |event: PipelineEvent| {
        let _ = emit_event(&app, event);
    })))
}

fn emit_event(app: &AppHandle, event: PipelineEvent) -> tauri::Result<()> {
    match event {
        PipelineEvent::State { snapshot } => app.emit("pipeline:state", &snapshot),
        PipelineEvent::Item { item } => app.emit("pipeline:item", &item),
        PipelineEvent::Decision { decision } => app.emit("pipeline:decision", &decision),
        PipelineEvent::Done { summary } => {
            // اعلان سیستمی پایان سینک — طبق تنظیمات کاربر (حتی با پنجرهٔ مینیمایز)
            let counts = &summary.counts;
            let ok = counts.synced + counts.softsubbed + counts.already_synced;
            let title = if summary.cancelled {
                "سینک متوقف شد"
            } else if summary.status == crate::pipeline::PipelineStatus::Failed {
                "سینک با خطا پایان یافت"
            } else {
                "سینک کامل شد"
            };
            crate::notify::notify_task_done(
                app,
                crate::notify::KIND_PIPELINE,
                title,
                &format!(
                    "موفق: {} — ناموفق: {} — هاردساب: {}",
                    ok,
                    counts.failed + counts.skipped,
                    counts.hardsubbed
                ),
            );
            app.emit("pipeline:done", &summary)
        }
        PipelineEvent::LibraryReplaced { entries } => app.emit("library:replaced", &entries),
        PipelineEvent::LibraryEntry { entry } => app.emit("library:entry", &entry),
    }
}

fn pipeline_control(state: &State<'_, AppState>) -> Option<Arc<PipelineControl>> {
    let published = state
        .pipeline
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    published.as_ref().map(Arc::clone)
}

/// شروع pipeline — admission اتمیک + publish + انتقال گارد با handoff + spawn.
/// شکست spawn → بازیابی گارد → حذف control (چک run_id) → رهایی اسلات؛ Err.
#[tauri::command]
pub fn start_pipeline(
    app: AppHandle,
    state: State<'_, AppState>,
    input: PipelineInput,
) -> Result<PipelineSnapshot, String> {
    // ۱) admission اتمیک — تعارض با اسکن/گروهی/تکی رد می‌شود
    let guard = acquire_batch_op(&state.ops)?;
    let emit_app = app.clone();

    // فقط یک pipeline هم‌زمان (اسلات control تکی است) — تسک‌های دیگر انواع
    // (هاردساب/افزودن زیرنویس) می‌توانند هم‌زمان اجرا شوند
    {
        let published = state
            .pipeline
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if published.is_some() {
            drop(guard);
            return Err("یک pipeline در حال اجراست؛ صبر کنید تا تمام شود.".into());
        }
    }

    let run_id = crate::sync::new_pipeline_run_id();
    let control = Arc::new(PipelineControl::from_guard(
        &guard,
        run_id.clone(),
        "شروع pipeline…",
    ));

    // ۲) publish — قبل از spawn
    {
        let mut published = state
            .pipeline
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *published = Some(Arc::clone(&control));
    }

    // ۳) انتقال مالکیت گارد با handoff
    let handoff: GuardHandoff = Arc::new(Mutex::new(Some(guard)));

    // ۴) spawn صریح — شکست → بازیابی گارد + حذف control + Err
    let h2 = Arc::clone(&handoff);
    let c2 = Arc::clone(&control);
    let worker_run_id = run_id.clone();
    let spawned = std::thread::Builder::new()
        .name("pipeline-worker".into())
        .spawn(move || {
            let state = app.state::<AppState>();
            let sink = publish_state_sink(&app);
            crate::pipeline::enter_worker(&state, worker_run_id, &h2, c2, input, sink);
        });

    match spawned {
        Ok(_) => {
            let snapshot = control.snapshot();
            let _ = emit_app.emit("pipeline:state", &snapshot);
            Ok(snapshot)
        }
        Err(error) => {
            // caller مالکیت گارد را از handoff بازیابی می‌کند؛ ترتیب: حذف control → رهایی اسلات
            let guard = take_guard_once(&handoff)?;
            clear_published_control(&state, &run_id);
            drop(guard);
            Err(format!("شروع pipeline ناموفق بود: {error}"))
        }
    }
}

/// resolve یک تصمیم باز — گزینه مات backend-owned؛ هر مقدار دیگری رد می‌شود.
/// worker پس از wake، وضعیت آیتم/مرحله را به‌روزرسانی و رویدادها را منتشر می‌کند.
#[tauri::command]
pub fn resolve_pipeline_decision(
    state: State<'_, AppState>,
    decision_id: String,
    option_id: String,
) -> Result<(), String> {
    resolve_decision_core(&state, &decision_id, &option_id)?;
    Ok(())
}

/// پخش ویدیو با پخش‌کنندهٔ پیش‌فرض ویندوز — برای قضاوت سریع سینک توسط کاربر.
/// هیچ فرایند دیگری اجرا نمی‌شود؛ فقط `start` روی فایل با association خودش.
#[tauri::command]
pub fn play_video(path: String) -> Result<(), String> {
    let video = std::path::PathBuf::from(path.trim());
    if !video.is_file() {
        return Err("فایل ویدیو روی دیسک پیدا نشد.".into());
    }
    crate::logging::info(
        "pipeline",
        &format!("پخش ویدیو با پخش‌کنندهٔ پیش‌فرض: {}", video.display()),
    );
    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/C", "start", "", &video.display().to_string()]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("باز کردن ویدیو با پخش‌کنندهٔ پیش‌فرض ناموفق بود: {e}"))
}

/// باز کردن پوشهٔ محل ذخیره در Explorer با انتخاب خود فایل — برای دسترسی
/// سریع به خروجی (ویدیو/زیرنویس) کنار بقیهٔ فایل‌ها.
#[tauri::command]
pub fn reveal_in_explorer(path: String) -> Result<(), String> {
    let target = std::path::PathBuf::from(path.trim());
    if !target.is_file() {
        return Err("فایل روی دیسک پیدا نشد.".into());
    }
    crate::logging::info(
        "pipeline",
        &format!("نمایش محل ذخیره در Explorer: {}", target.display()),
    );
    let mut cmd = std::process::Command::new("explorer");
    cmd.args(["/select,", &target.display().to_string()]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("باز کردن Explorer ناموفق بود: {e}"))
}

/// لغو pipeline فعال — فقط control منتشرشده؛ بدون pipeline → false.
/// worker پس از wake، آیتم‌های باز را Cancelled می‌کند و pipeline:done می‌فرستد.
#[tauri::command]
pub fn cancel_pipeline(app: AppHandle, state: State<'_, AppState>) -> Result<bool, String> {
    let cancelled = cancel_active_pipeline(&state)?;
    if cancelled {
        // وضعیت «در حال توقف» را فوراً منتشر می‌کنیم — تا worker پاسخ دهد،
        // کاربر باید ببیند که درخواست ثبت شده و برنامه در حال توقف است.
        if let Some(control) = pipeline_control(&state) {
            let _ = emit_event(
                &app,
                PipelineEvent::State {
                    snapshot: control.snapshot(),
                },
            );
        }
    }
    Ok(cancelled)
}

/// snapshot کامل — برای بازیابی UI بدون اتکای انحصاری به رویدادها.
#[tauri::command]
pub fn get_pipeline_state(state: State<'_, AppState>) -> Option<PipelineSnapshot> {
    pipeline_control(&state).map(|control| control.snapshot())
}

/// آیتم‌های run فعال — بازیابی جدول فایل‌ها پس از remount/reload.
#[tauri::command]
pub fn get_pipeline_items(state: State<'_, AppState>) -> Vec<PipelineItemSnapshot> {
    pipeline_control(&state)
        .map(|control| control.item_snapshots())
        .unwrap_or_default()
}

/// آیا pipeline فعالی جریان دارد؟
#[tauri::command]
pub fn is_pipeline_active(state: State<'_, AppState>) -> bool {
    pipeline_control(&state).is_some()
}
