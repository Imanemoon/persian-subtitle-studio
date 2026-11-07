//! دستور تشخیص ابزارهای خارجی.

use tauri::State;

use crate::models::ToolInfo;
use crate::state::AppState;

#[tauri::command]
pub fn detect_tools(state: State<'_, AppState>) -> Vec<ToolInfo> {
    let overrides = state.settings.lock().unwrap().tool_paths.clone();
    let results = crate::tools::detect_all(&overrides);
    for t in &results {
        let msg = format!(
            "{}: {} {} ({})",
            t.name,
            match t.status {
                crate::models::ToolStatus::Found => "پیدا شد",
                crate::models::ToolStatus::Missing => "پیدا نشد",
                crate::models::ToolStatus::Broken => "خراب/اجرا نمی‌شود",
            },
            t.version.as_deref().unwrap_or(""),
            t.path.as_deref().unwrap_or("-"),
        );
        match t.status {
            crate::models::ToolStatus::Found => crate::logging::info("tools", &msg),
            _ => crate::logging::warn("tools", &msg),
        }
    }
    results
}
