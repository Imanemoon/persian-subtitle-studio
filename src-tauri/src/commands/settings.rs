//! دستورات دریافت و ذخیره تنظیمات.

use tauri::{AppHandle, Manager, State};

use crate::settings::Settings;
use crate::state::{AppPaths, AppState};

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Settings, String> {
    let clean = settings.sanitized();

    if let Some(root) = &clean.root_folder {
        if !std::path::Path::new(root).is_dir() {
            crate::logging::warn(
                "settings",
                &format!("پوشه ریشه تنظیم‌شده وجود ندارد: {root}"),
            );
        }
    }

    let paths = app.state::<AppPaths>();
    crate::settings::save(&paths.config_dir, &clean)?;
    *state.settings.lock().unwrap() = clean.clone();
    crate::logging::info("settings", "تنظیمات ذخیره شد");
    Ok(clean)
}
