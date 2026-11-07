// ماژول‌ها public هستند تا تست‌های یکپارچه (tests/) بتوانند مستقیماً
// لایه command را با mock-runtime تست کنند.
pub mod commands;
pub mod logging;
pub mod media;
pub mod models;
pub mod scanner;
pub mod settings;
pub mod state;
pub mod subtitle;
pub mod tools;

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
                ops: Mutex::new(state::OpsRegistry::default()),
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
            commands::media::analyze_video,
            commands::media::analyze_all,
            commands::subtitle::extract_track,
            commands::subtitle::extract_track_for_review,
            commands::subtitle::promote_review,
            commands::subtitle::plan_extraction_all,
            commands::subtitle::execute_extraction_plan,
            commands::subtitle::cancel_subtitle_ops,
            commands::subtitle::validate_subtitle_file,
        ])
        .run(tauri::generate_context!())
        .expect("خطا در اجرای SubSync Studio");
}
