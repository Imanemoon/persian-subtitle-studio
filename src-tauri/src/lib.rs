// ماژول‌ها public هستند تا تست‌های یکپارچه (tests/) بتوانند مستقیماً
// لایه command را با mock-runtime تست کنند.
pub mod commands;
pub mod hardsub;
pub mod logging;
pub mod media;
pub mod models;
pub mod notify;
pub mod pipeline;
pub mod scanner;
pub mod settings;
pub mod setup;
pub mod state;
pub mod subsource;
pub mod subtitle;
pub mod sync;
pub mod tools;

pub mod archive;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tauri::Manager;

use crate::state::{AppPaths, AppState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // دادهٔ ماندگار در ProgramData — خارج از دسترسِ uninstaller per-user:
            // تیک «حذف اپ دیتا» در حذف برنامه، ابزارها و تنظیمات را نمی‌کشد.
            let data_root = persistent_data_root();
            let config_dir = data_root.join("config");
            let data_dir = data_root.join("data");
            let log_dir = data_root.join("logs");

            // مهاجرت یک‌باره از پوشه‌های identifier-محور قدیمی (Roaming/Local)
            let legacy_config = app.path().app_config_dir().ok();
            let legacy_data = app.path().app_local_data_dir().ok();
            migrate_legacy_data(
                legacy_config.as_deref(),
                legacy_data.as_deref(),
                &config_dir,
                &data_dir,
            );

            let _ = std::fs::create_dir_all(&config_dir);
            let _ = std::fs::create_dir_all(&data_dir);
            let _ = std::fs::create_dir_all(&log_dir);

            logging::init(Some(log_dir.join("subsync-studio.log")));
            logging::set_app_handle(app.handle().clone());
            logging::info("app", "SubSync Studio اجرا شد");

            let mut app_settings = settings::load(&config_dir);
            // مسیر ابزارهایی که به پوشه‌های قدیمی اشاره می‌کنند به ریشهٔ جدید
            // بازنویسی می‌شود تا بعد از حذف نسخهٔ قبلی بی‌اعتبار نشوند
            if let Some(legacy) = legacy_data.as_deref() {
                if rewrite_legacy_tool_paths(&mut app_settings, legacy, &data_dir) {
                    match settings::save(&config_dir, &app_settings) {
                        Ok(()) => {
                            logging::info("settings", "مسیر ابزارها به ریشهٔ ماندگار بازنویسی شد")
                        }
                        Err(e) => logging::warn(
                            "settings",
                            &format!("ذخیرهٔ مسیرهای بازنویسی‌شده ناموفق: {e}"),
                        ),
                    }
                }
            }
            // گزارش مهاجرت دادهٔ قدیمی — پس از init لاگ
            if let Some(report) = LEGACY_MIGRATION_REPORT
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .last()
                .cloned()
            {
                logging::info("app", &report);
            }
            if TOOLS_MIGRATION_FAILED.load(Ordering::Relaxed) {
                logging::warn(
                    "app",
                    "کپی ابزارها از پوشهٔ قدیمی ناقص ماند — از معاونت راه‌اندازی دوباره نصب کنید",
                );
            } else {
                let bytes = TOOLS_BYTES_MIGRATED.load(Ordering::Relaxed);
                if bytes > 0 {
                    logging::info(
                        "app",
                        &format!(
                            "ابزارها به ریشهٔ ماندگار منتقل شدند — {}",
                            crate::setup::fmt_bytes(bytes)
                        ),
                    );
                }
            }

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
                hardsub: Mutex::new(None),
                hardsub_encoder_cache: Mutex::new(None),
                setup_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                sync_attempts: Mutex::new(std::collections::HashMap::new()),
            });
            app.manage(AppPaths {
                config_dir,
                data_dir,
            });

            // ابزارهای باندل‌شده (sub-rs) — فقط اگر در هیچ‌جای سیستم نیستند
            commands::setup::seed_bundled_tools(app.handle());

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
            commands::pipeline::play_video,
            commands::pipeline::reveal_in_explorer,
            commands::hardsub::plan_hardsub,
            commands::hardsub::start_hardsub,
            commands::hardsub::cancel_hardsub,
            commands::hardsub::get_hardsub_state,
            commands::hardsub::is_hardsub_active,
            commands::secrets::set_subsource_api_key,
            commands::secrets::has_subsource_api_key,
            commands::secrets::delete_subsource_api_key,
            commands::subsource::plan_subsource_download,
            commands::subsource::execute_subsource_download,
            commands::subsource::is_download_active,
            commands::subsource::place_local_subtitle,
            commands::subsource::download_softsub_batch,
            commands::subsource::reset_subtitle,
            commands::archive::plan_archive_extraction,
            commands::archive::execute_archive_extraction,
            commands::archive::is_archive_active,
            commands::tools::detect_tools,
            commands::setup::setup_status,
            commands::setup::install_tool,
            commands::setup::cancel_setup,
            commands::setup::is_elevated,
            commands::setup::add_defender_exclusion,
            commands::setup::get_app_dir_hints,
            commands::setup::create_admin_shortcut,
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

/// ریشهٔ دادهٔ ماندگار — `%ProgramData%\SubSync Studio`.
///
/// پوشه‌های identifier-محور (Roaming/Local) توسط uninstaller per-user با تیک
/// «حذف اپ دیتا» پاک می‌شوند؛ ProgramData هرگز با حذف برنامه per-user پاک
/// نمی‌شود، پس ابزارها و تنظیمات از uninstall جان سالم می‌برند.
fn persistent_data_root() -> std::path::PathBuf {
    match std::env::var_os("ProgramData") {
        Some(pd) if !pd.is_empty() => std::path::PathBuf::from(pd).join("SubSync Studio"),
        // خارج از ویندوز/بدون ProgramData — رفتار قبلی
        _ => {
            if let Ok(home) = std::env::var("USERPROFILE") {
                std::path::PathBuf::from(home)
                    .join("AppData")
                    .join("Local")
                    .join("SubSyncStudio")
            } else {
                std::env::temp_dir().join("subsync-studio")
            }
        }
    }
}

/// کپی بازگشتی دیرکتوری — فقط فایل/پوشهٔ معمولی؛ خروجی: بایت‌های کپی‌شده.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<u64, String> {
    std::fs::create_dir_all(dst)
        .map_err(|e| format!("ساخت پوشهٔ «{}» ناموفق: {e}", dst.display()))?;
    let mut copied = 0u64;
    for entry in std::fs::read_dir(src).map_err(|e| format!("خواندن «{}»: {e}", src.display()))?
    {
        let entry = entry.map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let meta = entry
            .metadata()
            .map_err(|e| format!("متادیتای «{}»: {e}", from.display()))?;
        if meta.is_dir() {
            copied += copy_dir_recursive(&from, &to)?;
        } else if meta.is_file() {
            std::fs::copy(&from, &to)
                .map_err(|e| format!("کپی «{}» ناموفق: {e}", from.display()))?;
            copied += meta.len();
        }
    }
    Ok(copied)
}

/// مهاجرت یک‌بارهٔ داده از پوشه‌های قدیمی identifier-محور به ریشهٔ ماندگار.
/// فقط وقتی مقصد خالی است و منبع موجود است؛ خطاها ناهلاک‌شده لاگ نمی‌شوند
/// (logging هنوز init نشده) — بعد از init در صورت شکست گزارش می‌شود.
fn migrate_legacy_data(
    legacy_config: Option<&std::path::Path>,
    legacy_data: Option<&std::path::Path>,
    config_dir: &std::path::Path,
    data_dir: &std::path::Path,
) {
    let new_settings = config_dir.join("settings.json");
    if new_settings.exists() {
        return; // مهاجرت قبلاً انجام شده
    }
    let mut migrated = Vec::new();
    if let Some(legacy) = legacy_config {
        let old = legacy.join("settings.json");
        if old.exists() {
            let _ = std::fs::create_dir_all(config_dir);
            if std::fs::copy(&old, &new_settings).is_ok() {
                migrated.push("settings.json");
            }
        }
    }
    if let Some(legacy) = legacy_data {
        let old_tools = legacy.join("tools");
        if old_tools.is_dir() && !data_dir.join("tools").exists() {
            match copy_dir_recursive(&old_tools, &data_dir.join("tools")) {
                Ok(bytes) => {
                    migrated.push("tools");
                    TOOLS_BYTES_MIGRATED.store(bytes, Ordering::Relaxed);
                }
                Err(_) => TOOLS_MIGRATION_FAILED.store(true, Ordering::Relaxed),
            }
        }
    }
    if !migrated.is_empty() {
        LEGACY_MIGRATION_REPORT
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(format!(
                "مهاجرت دادهٔ قدیمی انجام شد: {}",
                migrated.join("، ")
            ));
    }
}

/// گزارش مهاجرت + بایت‌های کپی‌شده — بعد از init لاگ مصرف و پاک می‌شود.
static LEGACY_MIGRATION_REPORT: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
static TOOLS_BYTES_MIGRATED: AtomicU64 = AtomicU64::new(0);
static TOOLS_MIGRATION_FAILED: AtomicBool = AtomicBool::new(false);

/// بازنویسی مسیرهای ابزار که به ریشهٔ دادهٔ قدیمی اشاره می‌کنند — خروجی:
/// آیا تغییری رخ داد (برای save).
fn rewrite_legacy_tool_paths(
    settings: &mut crate::settings::Settings,
    legacy_data_dir: &std::path::Path,
    new_data_dir: &std::path::Path,
) -> bool {
    let old_prefix = legacy_data_dir.display().to_string();
    let mut changed = false;
    let slots = [
        &mut settings.tool_paths.ffmpeg,
        &mut settings.tool_paths.ffprobe,
        &mut settings.tool_paths.subsource,
        &mut settings.tool_paths.seven_zip,
        &mut settings.tool_paths.ffsubsync,
    ];
    for slot in slots {
        if let Some(value) = slot.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            let value = value.to_string();
            if value.len() > old_prefix.len()
                && value[..old_prefix.len()].eq_ignore_ascii_case(&old_prefix)
            {
                *slot = Some(format!(
                    "{}{}",
                    new_data_dir.display(),
                    &value[old_prefix.len()..]
                ));
                changed = true;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_legacy_tool_paths_rewrites_only_legacy_prefix() {
        let mut settings = crate::settings::Settings::default();
        let old = std::path::PathBuf::from("C:/Users/u/AppData/Local/com.subsync.studio");
        let new = std::path::PathBuf::from("C:/ProgramData/SubSync Studio");
        settings.tool_paths.ffmpeg =
            Some(old.join("tools/ffmpeg/ffmpeg.exe").display().to_string());
        settings.tool_paths.ffsubsync = Some("C:/other/ffs.exe".into());
        assert!(rewrite_legacy_tool_paths(&mut settings, &old, &new));
        assert_eq!(
            settings.tool_paths.ffmpeg.as_deref().unwrap(),
            new.join("tools/ffmpeg/ffmpeg.exe").display().to_string()
        );
        assert_eq!(
            settings.tool_paths.ffsubsync.as_deref(),
            Some("C:/other/ffs.exe")
        );
        // دوباره: دیگر چیزی برای بازنویسی نیست
        assert!(!rewrite_legacy_tool_paths(&mut settings, &old, &new));
    }

    #[test]
    fn copy_dir_recursive_copies_files_and_subdirs() {
        let src = std::env::temp_dir().join(format!("mig-src-{}", std::process::id()));
        let dst = std::env::temp_dir().join(format!("mig-dst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dst);
        std::fs::create_dir_all(src.join("a")).unwrap();
        std::fs::write(src.join("f.txt"), b"hello").unwrap();
        std::fs::write(src.join("a").join("g.txt"), b"world").unwrap();
        let bytes = copy_dir_recursive(&src, &dst).unwrap();
        assert_eq!(bytes, 10);
        assert!(dst.join("f.txt").is_file());
        assert!(dst.join("a").join("g.txt").is_file());
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dst);
    }

    #[test]
    fn persistent_data_root_is_programdata_on_windows() {
        if let Some(pd) = std::env::var_os("ProgramData") {
            let root = persistent_data_root();
            assert!(root.starts_with(std::path::PathBuf::from(pd)));
            assert!(root.ends_with("SubSync Studio"));
        }
    }
}
