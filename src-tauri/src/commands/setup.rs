//! commandهای معاونت راه‌اندازی — wrapperهای نازک؛ هسته در crate::setup است.
//! الگوی long-running: thread + emit رویداد، همان الگوی execute_extraction_plan.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::models::ToolStatus;
use crate::setup::{self, SetupProgress};
use crate::state::{AppPaths, AppState};

/// وضعیت یک ابزار در معاونت — از detect واقعی می‌آید؛ هیچ «حس می‌شود» نیست.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupToolStatus {
    pub id: String,
    pub label: String,
    pub required_for_entry: bool,
    pub size_hint: String,
    pub status: ToolStatus,
}

#[tauri::command]
pub fn setup_status(state: State<'_, AppState>) -> Vec<SetupToolStatus> {
    let overrides = state.settings.lock().unwrap().tool_paths.clone();
    let detected = crate::tools::detect_all(&overrides);
    let mut out = Vec::new();
    for spec in &setup::SETUP_TOOLS {
        let status = detected
            .iter()
            .find(|t| t.id == spec.id)
            .map(|t| t.status)
            .unwrap_or(ToolStatus::Missing);
        out.push(SetupToolStatus {
            id: spec.id.to_string(),
            label: spec.label.to_string(),
            required_for_entry: spec.required_for_entry,
            size_hint: spec.size_hint.to_string(),
            status,
        });
    }
    out
}

/// قفل تک‌نمونه‌ای نصب — هم‌زمان بیش از یک نصب جریان نمی‌گیرد (دانلودها سنگین‌اند).
static INSTALL_IN_FLIGHT: Mutex<Option<String>> = Mutex::new(None);

#[tauri::command]
pub fn install_tool(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: State<'_, AppPaths>,
    id: String,
) -> Result<(), String> {
    if setup::spec_by_id(&id).is_none() {
        return Err(format!("ابزار «{id}» جزو معاونت راه‌اندازی نیست"));
    }
    let mut flight = INSTALL_IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner());
    if flight.is_some() {
        return Err("یک نصب دیگر در جریان است — تا پایان آن صبر کنید یا لغو کنید".into());
    }
    *flight = Some(id.clone());
    drop(flight);

    let cancel = state.setup_cancel.clone();
    cancel.store(false, Ordering::SeqCst);
    let app2 = app.clone();
    let tools_dir = paths.inner().tools_dir();
    std::thread::spawn(move || {
        let result = run_install(&app2, &tools_dir, &cancel, &id);
        {
            let mut flight = INSTALL_IN_FLIGHT.lock().unwrap_or_else(|p| p.into_inner());
            *flight = None;
        }
        let (ok, error) = match &result {
            Ok(()) => (true, None),
            Err(e) => (false, Some(e.clone())),
        };
        let _ = app2.emit(
            "setup:done",
            serde_json::json!({ "toolId": id, "ok": ok, "error": error }),
        );
        match &result {
            Ok(()) => crate::logging::info("setup", &format!("نصب {id} کامل شد")),
            Err(e) => crate::logging::warn("setup", &format!("نصب {id} ناموفق: {e}")),
        }
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_setup(state: State<'_, AppState>) {
    state.setup_cancel.store(true, Ordering::SeqCst);
}

/// اجرای نصب واقعی یک ابزار — progress با emit به فرانت؛ ثبت نهایی در settings.
fn run_install(
    app: &AppHandle,
    tools_dir: &std::path::Path,
    cancel: &AtomicBool,
    id: &str,
) -> Result<(), String> {
    let app2 = app.clone();
    let mut on_progress = |p: SetupProgress| {
        let _ = app2.emit("setup:progress", &p);
    };
    match id {
        "ffmpeg" => {
            let mut fetch = http_fetch(app, cancel);
            setup::install_ffmpeg(
                tools_dir,
                &mut fetch,
                Some(setup::FFMPEG_SHA256),
                &mut on_progress,
                cancel,
            )
            .and_then(|inst| {
                update_tool_paths(app, |s| {
                    s.tool_paths.ffmpeg = Some(inst.ffmpeg_path.display().to_string());
                    s.tool_paths.ffprobe = Some(inst.ffprobe_path.display().to_string());
                })?;
                Ok(())
            })
        }
        "ffsubsync" => {
            let mut fetch = http_fetch(app, cancel);
            setup::install_ffsubsync(
                tools_dir,
                &mut fetch,
                Some(setup::PYTHON_SHA256),
                &mut on_progress,
                cancel,
            )
            .and_then(|inst| {
                update_tool_paths(app, |s| {
                    s.tool_paths.ffsubsync = Some(inst.ffs_path.display().to_string());
                })?;
                Ok(())
            })
        }
        "sevenZip" => {
            let installer = find_resource_file(app, "7z2603-x64.exe")
                .ok_or("نصاب 7-Zip در بستهٔ برنامه یافت نشد")?;
            setup::install_seven_zip(&installer, &mut on_progress).map(|_| ())
        }
        _ => Err(format!("ابزار «{id}» شناخته‌شده نیست")),
    }
}

/// fetch واقعی — دانلود HTTP با progress؛ تأیید hash در لایهٔ install انجام
/// می‌شود (verify_sha256) تا تست‌ها با fetch فیک همان مسیر را بروند.
fn http_fetch<'a>(
    app: &AppHandle,
    cancel: &'a AtomicBool,
) -> impl FnMut(&str, &std::path::Path) -> Result<(), String> + 'a {
    let app = app.clone();
    move |url: &str, dest: &std::path::Path| {
        let mut progress = |p: SetupProgress| {
            let _ = app.emit("setup:progress", &p);
        };
        let tool_id = if url.contains("python") {
            "ffsubsync"
        } else {
            "ffmpeg"
        };
        setup::download_file(tool_id, url, None, dest, &mut progress, cancel)
    }
}

/// به‌روزرسانی settings + ذخیرهٔ اتمیک — همان مسیر save موجود.
fn update_tool_paths(
    app: &AppHandle,
    patch: impl FnOnce(&mut crate::settings::Settings),
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let paths = app.state::<AppPaths>();
    let mut settings = state.settings.lock().unwrap_or_else(|p| p.into_inner());
    patch(&mut settings);
    let snapshot = settings.clone();
    drop(settings);
    crate::settings::save(&paths.inner().config_dir, &snapshot)
}

/// یافتن فایل باندل‌شده در resource_dir — چند چیدمان شناخته‌شده امتحان می‌شود.
fn find_resource_file(app: &AppHandle, file_name: &str) -> Option<PathBuf> {
    let dir = app.path().resource_dir().ok()?;
    let candidates = [
        dir.join("resources").join("tools").join(file_name),
        dir.join("tools").join(file_name),
        dir.join(file_name),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

// ---------------------------------------------------------------------
// امتیاز، Defender، میان‌بر — wrapper نازک روی هستهٔ setup
// ---------------------------------------------------------------------

/// مسیرهای مؤثر برای استثنای Defender — نصب برنامه و پوشهٔ ابزارها.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppDirHints {
    pub install_dir: String,
    pub tools_dir: String,
}

#[tauri::command]
pub fn get_app_dir_hints(
    app: AppHandle,
    paths: State<'_, AppPaths>,
) -> Result<AppDirHints, String> {
    let install_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.display().to_string()))
        .unwrap_or_default();
    let _ = &app;
    Ok(AppDirHints {
        install_dir,
        tools_dir: paths.inner().tools_dir().display().to_string(),
    })
}

#[tauri::command]
pub fn is_elevated() -> Result<bool, String> {
    setup::is_elevated()
}

#[tauri::command]
pub fn add_defender_exclusion(paths: Vec<String>) -> Result<String, String> {
    let result = setup::add_defender_exclusions(&paths);
    match &result {
        Ok(message) => crate::logging::info(
            "setup",
            &format!("استثنای Defender: {message} — مسیرها: {}", paths.join(", ")),
        ),
        Err(e) => crate::logging::warn(
            "setup",
            &format!(
                "استثنای Defender ناموفق: {e} — مسیرها: {}",
                paths.join(", ")
            ),
        ),
    }
    result
}

#[tauri::command]
pub fn create_admin_shortcut() -> Result<String, String> {
    let result = std::env::current_exe()
        .map_err(|e| format!("مسیر برنامهٔ جاری یافت نشد: {e}"))
        .and_then(|exe| setup::create_admin_shortcut(&exe, "SubSync Studio"));
    match &result {
        Ok(message) => crate::logging::info("setup", &format!("میان‌بر Administrator: {message}")),
        Err(e) => crate::logging::warn("setup", &format!("میان‌بر Administrator ناموفق: {e}")),
    }
    result
}

/// پیش‌آماده‌سازی ابزارهای باندل‌شده هنگام شروع — sub-rs از resources به
/// پوشهٔ tools کپی می‌شود فقط اگر ابزار در هیچ‌جا یافت نشود.
pub fn seed_bundled_tools(app: &AppHandle) {
    let state = app.state::<AppState>();
    let paths = app.state::<AppPaths>();
    let missing_subsource = {
        let settings = state.settings.lock().unwrap_or_else(|p| p.into_inner());
        crate::tools::resolve_tool_path("subsource", &settings.tool_paths).is_none()
    };
    if !missing_subsource {
        return;
    }
    let Some(source) = find_resource_file(app, "sub-rs.exe") else {
        return;
    };
    let dest_dir = paths.inner().tools_dir().join("sub-rs");
    match setup::seed_bundled_binary(&source, &dest_dir, "sub-rs.exe") {
        Ok(Some(dest)) => {
            let mut settings = state.settings.lock().unwrap_or_else(|p| p.into_inner());
            settings.tool_paths.subsource = Some(dest.display().to_string());
            let config_dir = paths.inner().config_dir.clone();
            let snapshot = settings.clone();
            drop(settings);
            if let Err(e) = crate::settings::save(&config_dir, &snapshot) {
                crate::logging::warn("setup", &format!("ثبت مسیر sub-rs باندل‌شده ناموفق: {e}"));
            } else {
                crate::logging::info(
                    "setup",
                    &format!("sub-rs از بستهٔ برنامه آماده شد — {}", dest.display()),
                );
            }
        }
        Ok(None) => {}
        Err(e) => crate::logging::warn("setup", &format!("پیش‌آماده‌سازی sub-rs ناموفق: {e}")),
    }
}
