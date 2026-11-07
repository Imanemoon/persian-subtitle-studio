//! دستورات تحلیل زیرنویس داخلی (FFprobe).
//!
//! هسته منطق (`run_analyze_blocking` و توابع core) مستقل از Tauri و فقط با
//! `&AppState` کار می‌کند تا تست‌پذیر باشد؛ commandها فقط wrapper رویدادند.

use std::path::PathBuf;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::models::{AnalysisStatus, OpsProgress, VideoAnalysis, VideoEntry};
use crate::state::AppState;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn get_entry(state: &AppState, path: &str) -> Option<VideoEntry> {
    state
        .library
        .lock()
        .unwrap()
        .iter()
        .find(|e| e.id == path)
        .cloned()
}

/// جایگزینی یک entry در State (بدون رخداد — لایه command رخداد می‌دهد).
pub fn apply_entry(state: &AppState, entry: VideoEntry) {
    let mut lib = state.library.lock().unwrap();
    if let Some(slot) = lib.iter_mut().find(|e| e.id == entry.id) {
        *slot = entry;
    }
}

pub(crate) fn set_status_core(
    state: &AppState,
    path: &str,
    status: AnalysisStatus,
    message: Option<String>,
) {
    let mut lib = state.library.lock().unwrap();
    let Some(entry) = lib.iter_mut().find(|e| e.id == path) else {
        return;
    };
    let analysis = entry.analysis.get_or_insert_with(|| VideoAnalysis {
        status,
        analyzed_at_ms: now_ms(),
        streams: Vec::new(),
        message: None,
        last_extraction: None,
    });
    analysis.status = status;
    if message.is_some() {
        analysis.message = message;
    }
}

/// مسیر ابزار خارجی از تنظیمات (override) یا تشخیص خودکار.
pub fn resolve_tool(state: &AppState, id: &str) -> Result<PathBuf, String> {
    let overrides = state.settings.lock().unwrap().tool_paths.clone();
    crate::tools::resolve_tool_path(id, &overrides).ok_or_else(|| {
        format!(
            "{} پیدا نشد — مسیر آن را در تنظیمات مشخص کنید",
            crate::tools::display_name(id)
        )
    })
}

/// هسته تحلیل یک ویدیو: State را به‌روز می‌کند و entry جدید را برمی‌گرداند.
/// مستقل از Tauri — فراخوانی command رخداد `library:entry` را منتشر می‌کند.
pub fn run_analyze_blocking(state: &AppState, path: &str) -> Result<VideoEntry, String> {
    let Some(entry) = get_entry(state, path) else {
        return Err("این ویدیو در کتابخانه نیست — ابتدا پوشه را اسکن کنید.".into());
    };

    set_status_core(state, path, AnalysisStatus::Analyzing, None);
    crate::logging::info("media", &format!("تحلیل زیرنویس داخلی: {path}"));

    let ffprobe = resolve_tool(state, "ffprobe")?;
    let video = PathBuf::from(path);
    let result = crate::media::ffprobe::probe_subtitle_streams(&ffprobe, &video);

    let (status, message, streams) = match result {
        Ok(streams) => {
            let (s, m) = crate::subtitle::detection::classify_analysis(&streams);
            (s, m, streams)
        }
        Err(err) => (AnalysisStatus::Failed, Some(err.clone()), Vec::new()),
    };

    crate::logging::info(
        "media",
        &format!(
            "نتیجه تحلیل {} → {:?} ({} Track)",
            entry.file_name,
            status,
            streams.len()
        ),
    );
    if let Some(m) = &message {
        crate::logging::info("media", m);
    }

    let mut updated = entry;
    let analysis = updated.analysis.get_or_insert_with(|| VideoAnalysis {
        status,
        analyzed_at_ms: now_ms(),
        streams: Vec::new(),
        message: None,
        last_extraction: None,
    });
    analysis.status = status;
    analysis.message = message;
    analysis.streams = streams;
    analysis.analyzed_at_ms = now_ms();
    apply_entry(state, updated.clone());
    Ok(updated)
}

#[tauri::command(async)]
pub async fn analyze_video(app: AppHandle, path: String) -> Result<VideoEntry, String> {
    let app2 = app.clone();
    let path2 = path.clone();
    let updated = tauri::async_runtime::spawn_blocking(move || {
        let st = app2.state::<AppState>();
        run_analyze_blocking(&st, &path2)
    })
    .await
    .map_err(|e| format!("خطای Thread: {e}"))??;

    let _ = app.emit("library:entry", &updated);
    Ok(updated)
}

/// تحلیل گروهی همه ویدیوهای کتابخانه در پس‌زمینه؛ تعداد صف را برمی‌گرداند.
#[tauri::command]
pub fn analyze_all(app: AppHandle, state: State<'_, AppState>) -> Result<u64, String> {
    if state.ops_running.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("یک عملیات گروهی در حال اجراست؛ ابتدا آن را متوقف کنید.".into());
    }
    let paths: Vec<String> = state
        .library
        .lock()
        .unwrap()
        .iter()
        .map(|e| e.id.clone())
        .collect();
    if paths.is_empty() {
        return Err("کتابخانه خالی است — ابتدا اسکن کنید.".into());
    }

    let total = paths.len() as u64;
    state
        .ops_running
        .store(true, std::sync::atomic::Ordering::SeqCst);
    state
        .cancel_ops
        .store(false, std::sync::atomic::Ordering::SeqCst);
    crate::logging::info("media", &format!("تحلیل گروهی {total} ویدیو شروع شد"));

    let app2 = app.clone();
    std::thread::spawn(move || {
        for (i, path) in paths.into_iter().enumerate() {
            let st = app2.state::<AppState>();
            if st.cancel_ops.load(std::sync::atomic::Ordering::SeqCst) {
                crate::logging::warn("media", "تحلیل گروهی لغو شد");
                break;
            }
            let _ = app2.emit(
                "ops:progress",
                &OpsProgress {
                    kind: "analyze".into(),
                    current: i as u64 + 1,
                    total,
                    video_path: path.clone(),
                    message: String::new(),
                },
            );
            match run_analyze_blocking(&st, &path) {
                Ok(updated) => {
                    let _ = app2.emit("library:entry", &updated);
                }
                Err(err) => {
                    crate::logging::error("media", &format!("تحلیل {path} ناموفق: {err}"));
                }
            }
        }
        let st = app2.state::<AppState>();
        st.ops_running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        crate::logging::info("media", "تحلیل گروهی پایان یافت");
        let _ = app2.emit(
            "ops:done",
            &OpsProgress {
                kind: "analyze".into(),
                ..Default::default()
            },
        );
    });

    Ok(total)
}
