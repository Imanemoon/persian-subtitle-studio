// ماژول‌ها public هستند تا تست‌های یکپارچه (tests/) بتوانند مستقیماً
// لایه command را با mock-runtime تست کنند.
pub mod commands;
pub mod logging;
pub mod media;
pub mod models;
pub mod pipeline;
pub mod scanner;
pub mod settings;
pub mod state;
pub mod subsource;
pub mod subtitle;
pub mod sync;
pub mod tools;

pub mod archive;

use std::sync::{Arc, Mutex};

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
                ops: Arc::new(Mutex::new(state::OpsRegistry::default())),
                archive_provenance: Mutex::new(
                    crate::archive::provenance::ProvenanceRegistry::default(),
                ),
                pipeline: Mutex::new(None),
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
            commands::pipeline::start_pipeline,
            commands::pipeline::resolve_pipeline_decision,
            commands::pipeline::cancel_pipeline,
            commands::pipeline::get_pipeline_state,
            commands::pipeline::get_pipeline_items,
            commands::pipeline::is_pipeline_active,
            commands::secrets::set_subsource_api_key,
            commands::secrets::has_subsource_api_key,
            commands::secrets::delete_subsource_api_key,
            commands::subsource::plan_subsource_download,
            commands::subsource::execute_subsource_download,
            commands::subsource::is_download_active,
            commands::archive::plan_archive_extraction,
            commands::archive::execute_archive_extraction,
            commands::archive::is_archive_active,
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
