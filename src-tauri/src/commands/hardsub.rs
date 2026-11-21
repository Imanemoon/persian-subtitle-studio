//! دستورات Tauri برای هاردساب — پلن، شروع، لغو و snapshot؛
//! mapping رویدادها به فرانت (`hardsub:state` / `hardsub:done`).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};

use tauri::{AppHandle, Emitter, Manager, State};

use crate::hardsub::{
    cancel_active_hardsub, clear_published_control, encoder, plan, types::*, EngineConfig,
    EnginePaths, HardsubControl, SinkStore,
};
use crate::settings::HardsubEncoder;
use crate::state::{acquire_batch_op, AppState};

fn publish_sink(app: &AppHandle) -> SinkStore {
    let app = app.clone();
    Arc::new(Mutex::new(Box::new(move |event: HardsubEvent| {
        let _ = emit_event(&app, event);
    })))
}

fn emit_event(app: &AppHandle, event: HardsubEvent) -> tauri::Result<()> {
    match event {
        HardsubEvent::State { snapshot } => app.emit("hardsub:state", &snapshot),
        HardsubEvent::Done { summary } => {
            // اعلان سیستمی پایان هاردساب — طبق تنظیمات کاربر
            let counts = &summary.counts;
            let title = if summary.cancelled {
                "هاردساب متوقف شد"
            } else {
                "هاردساب کامل شد"
            };
            crate::notify::notify_task_done(
                app,
                crate::notify::KIND_HARDSUB,
                title,
                &format!(
                    "موفق: {} — ناموفق: {} — تعارض: {}",
                    counts.done, counts.failed, counts.conflicts
                ),
            );
            app.emit("hardsub:done", &summary)
        }
    }
}

fn hardsub_control(state: &State<'_, AppState>) -> Option<Arc<HardsubControl>> {
    let published = state.hardsub.lock().unwrap_or_else(PoisonError::into_inner);
    published.as_ref().map(Arc::clone)
}

fn new_run_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("hardsub-{nanos:x}")
}

/// resolve_tool مشترک با media — پیام فارسی در نبود ابزار.
fn resolve_tool(state: &AppState, id: &str) -> Result<PathBuf, String> {
    crate::commands::media::resolve_tool(state, id)
}

/// انتخاب انکودر با کش سشن — کلید = تنظیم انکودر + مسیر ffmpeg؛
/// probe انکود آزمایشی (~۱-۲ ثانیه) فقط یک‌بار انجام می‌شود.
fn resolve_encoder_cached(
    state: &AppState,
    ffmpeg: &Path,
    setting: HardsubEncoder,
) -> encoder::VideoEncoder {
    let key = format!("{setting:?}|{}", ffmpeg.display());
    {
        let cache = state
            .hardsub_encoder_cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some((cached_key, encoder)) = cache.as_ref() {
            if *cached_key == key {
                return *encoder;
            }
        }
    }
    let no_cancel = AtomicBool::new(false);
    let resolved = encoder::resolve_encoder(ffmpeg, setting, &no_cancel);
    let mut cache = state
        .hardsub_encoder_cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    *cache = Some((key, resolved));
    resolved
}

fn encoder_label(state: &AppState, ffmpeg: &Path) -> String {
    let setting = state
        .settings
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .hardsub
        .encoder;
    resolve_encoder_cached(state, ffmpeg, setting)
        .label()
        .to_string()
}

/// پلن هاردساب — بازبینی فایل‌سیستم/ffprobe؛ فقط توصیه است و اجرا پلن
/// تازهٔ خودش را می‌سازد (منبع حقیقت).
#[tauri::command]
pub fn plan_hardsub(state: State<'_, AppState>, paths: Vec<String>) -> Result<HardsubPlan, String> {
    let ffprobe = resolve_tool(&state, "ffprobe")?;
    let ffmpeg = resolve_tool(&state, "ffmpeg")?;
    let encoder = encoder_label(&state, &ffmpeg);
    let no_cancel = AtomicBool::new(false);
    Ok(plan::build_plan(&paths, &ffprobe, &no_cancel, &encoder))
}

/// شروع هاردساب — admission اتمیک + پلن تازه + publish + handoff + spawn.
/// الگوی همان `start_pipeline`؛ شکست spawn → بازیابی گارد → حذف control.
#[tauri::command]
pub fn start_hardsub(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<HardsubSnapshot, String> {
    // ۱) admission اتمیک — تعارض با اسکن/pipeline/گروهی/تکی رد می‌شود
    let guard = acquire_batch_op(&state.ops)?;
    let emit_app = app.clone();

    // فقط یک هاردساب هم‌زمان (اسلات control تکی است)
    {
        let published = state.hardsub.lock().unwrap_or_else(PoisonError::into_inner);
        if published.is_some() {
            drop(guard);
            return Err("یک عملیات هاردساب در حال اجراست؛ صبر کنید تا تمام شود.".into());
        }
    }

    let settings = state
        .settings
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let ffmpeg = resolve_tool(&state, "ffmpeg")?;
    let ffprobe = resolve_tool(&state, "ffprobe")?;
    let encoder = resolve_encoder_cached(&state, &ffmpeg, settings.hardsub.encoder);

    // ۲) پلن تازه — منبع حقیقت اجرا (پلن نمایشی ممکن است کهنه باشد)
    let no_cancel = AtomicBool::new(false);
    let plan = plan::build_plan(&paths, &ffprobe, &no_cancel, encoder.label());
    if plan.items.is_empty() {
        drop(guard);
        let reason = if plan.skipped.is_empty() {
            "هیچ ویدیویی برای هاردساب یافت نشد".to_string()
        } else {
            let details: Vec<String> = plan
                .skipped
                .iter()
                .take(3)
                .map(|skip| format!("«{}»: {}", skip.path, skip.reason))
                .collect();
            format!("هیچ ویدیویی قابل هاردساب نبود — {}", details.join(" | "))
        };
        return Err(reason);
    }

    let run_id = new_run_id();
    let control = Arc::new(HardsubControl::from_guard(&guard, run_id.clone()));

    // ۳) publish — قبل از spawn
    {
        let mut published = state.hardsub.lock().unwrap_or_else(PoisonError::into_inner);
        *published = Some(Arc::clone(&control));
    }

    // ۴) انتقال مالکیت گارد با handoff + spawn
    let handoff: crate::pipeline::GuardHandoff = Arc::new(Mutex::new(Some(guard)));
    let engine_paths = EnginePaths { ffmpeg, ffprobe };
    let workers = (settings.hardsub.workers as usize)
        .clamp(1, 4)
        .min(plan.items.len())
        .max(1);
    let config = EngineConfig {
        encoder,
        quality: settings.hardsub.quality,
        workers,
    };

    let h2 = Arc::clone(&handoff);
    let c2 = Arc::clone(&control);
    let worker_run_id = run_id.clone();
    let p2 = plan.clone();
    let spawned = std::thread::Builder::new()
        .name("hardsub-worker".into())
        .spawn(move || {
            let state = app.state::<AppState>();
            let sink = publish_sink(&app);
            crate::hardsub::enter_worker(
                &state,
                worker_run_id,
                &h2,
                c2,
                engine_paths,
                config,
                p2,
                sink,
            );
        });

    match spawned {
        Ok(_) => {
            let snapshot = control.snapshot();
            let _ = emit_app.emit("hardsub:state", &snapshot);
            Ok(snapshot)
        }
        Err(error) => {
            // caller مالکیت گارد را از handoff بازیابی می‌کند؛ ترتیب: حذف control → رهایی اسلات
            let guard = crate::pipeline::take_guard_once(&handoff)?;
            clear_published_control(&state, &run_id);
            drop(guard);
            Err(format!("شروع هاردساب ناموفق بود: {error}"))
        }
    }
}

/// لغو هاردساب فعال — workerها آیتم‌های باز را Cancelled می‌کنند.
#[tauri::command]
pub fn cancel_hardsub(app: AppHandle, state: State<'_, AppState>) -> Result<bool, String> {
    let cancelled = cancel_active_hardsub(&state);
    if cancelled {
        if let Some(control) = hardsub_control(&state) {
            let _ = app.emit("hardsub:state", &control.snapshot());
        }
    }
    Ok(cancelled)
}

/// snapshot کامل — برای بازیابی UI بدون اتکای انحصاری به رویدادها.
#[tauri::command]
pub fn get_hardsub_state(state: State<'_, AppState>) -> Option<HardsubSnapshot> {
    hardsub_control(&state).map(|control| control.snapshot())
}

/// آیا هاردساب فعالی جریان دارد؟
#[tauri::command]
pub fn is_hardsub_active(state: State<'_, AppState>) -> bool {
    hardsub_control(&state).is_some()
}
