mod commands;
mod logging;
mod models;
mod scanner;
mod settings;
mod state;
mod tools;

use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use tauri::Manager;

use crate::state::{AppPaths, AppState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("subsync-studio"));
            let log_dir = app
                .path()
                .app_log_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("subsync-studio").join("logs"));
            let _ = std::fs::create_dir_all(&config_dir);
            let _ = std::fs::create_dir_all(&log_dir);

            logging::init(Some(log_dir.join("subsync-studio.log")));
            logging::set_app_handle(app.handle().clone());
            logging::info("app", "SubSync Studio اجرا شد");

            let app_settings = settings::load(&config_dir);

            // بررسی سلامت ابزارهای خارجی در شروع اجرا
            let detected = tools::detect_all(&app_settings.tool_paths);
            for t in &detected {
                match t.status {
                    models::ToolStatus::Found => logging::info(
                        "tools",
                        &format!(
                            "{} آماده است — {} ({})",
                            t.name,
                            t.version.as_deref().unwrap_or("نسخه نامشخص"),
                            t.path.as_deref().unwrap_or("-")
                        ),
                    ),
                    _ => logging::warn(
                        "tools",
                        &format!(
                            "{} در دسترس نیست ({})",
                            t.name,
                            t.detail.as_deref().unwrap_or("-")
                        ),
                    ),
                }
            }

            app.manage(AppState {
                settings: Mutex::new(app_settings),
                library: Mutex::new(Vec::new()),
                last_scan: Mutex::new(None),
                scanning: AtomicBool::new(false),
                cancel_scan: AtomicBool::new(false),
            });
            app.manage(AppPaths { config_dir });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::scan::start_scan,
            commands::scan::stop_scan,
            commands::scan::get_library,
            commands::scan::get_last_scan,
            commands::scan::is_scanning,
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::tools::detect_tools,
            commands::logs::get_logs,
        ])
        .run(tauri::generate_context!())
        .expect("خطا در اجرای SubSync Studio");
}
