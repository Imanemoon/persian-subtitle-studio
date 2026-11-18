//! دستورات اسکن و دسترسی به کتابخانه.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, State};

use crate::models::{ScanProgress, ScanSummary, VideoEntry};
use crate::scanner;
use crate::state::{acquire_scan_op, request_scan_cancel, AppState};

#[tauri::command]
pub fn start_scan(app: AppHandle, state: State<'_, AppState>, root: String) -> Result<(), String> {
    // admission اتمیک — تعارض با اسکن دیگر، عملیات گروهی و عملیات تکی‌ها
    // زیر همان قفل Registry رد می‌شود. گارد RAII است: هر Err بعدی (اعتبارسنجی
    // مسیر/مستثناها) با Drop فوری گارد، رزرو را آزاد می‌کند.
    let scan_guard = acquire_scan_op(&state.ops)?;

    let root_path = PathBuf::from(root.trim().trim_matches('"'));
    if root_path.as_os_str().is_empty() || !root_path.is_dir() {
        return Err("مسیر انتخاب‌شده یک پوشه معتبر نیست.".into());
    }

    let settings = state.settings.lock().unwrap().clone();
    if let Some(hit) = scanner::root_contains_excluded(&root_path, &settings.excluded_folders) {
        return Err(format!(
            "مسیر انتخاب‌شده درون پوشه مستثنا «{hit}» قرار دارد."
        ));
    }

    crate::logging::info("scan", &format!("شروع اسکن: {}", root_path.display()));

    let app2 = app.clone();
    std::thread::spawn(move || {
        let mut last_emit = Instant::now() - Duration::from_millis(500);
        let mut throttle = |p: ScanProgress| {
            if last_emit.elapsed() >= Duration::from_millis(200) {
                last_emit = Instant::now();
                let _ = app2.emit("scan:progress", &p);
            }
        };

        let st = app2.state::<AppState>();
        // پرچم لغو مخصوص همین نسل اسکن — از گارد رزرو می‌آید
        let out = scanner::scan_folder(
            &root_path,
            &settings.excluded_folders,
            &scan_guard.cancel,
            &mut throttle,
        );

        {
            let mut lib = st.library.lock().unwrap();
            *lib = out.entries;
            *st.last_scan.lock().unwrap() = Some(out.summary.clone());
        }
        // پایان عمر رزرو: رزرو همین‌جا آزاد می‌شود تا رویداد scan:done به UI برسد
        // و شروع اسکن جدید بلافاصله ممکن باشد.
        drop(scan_guard);

        if out.summary.cancelled {
            crate::logging::warn(
                "scan",
                &format!(
                    "اسکن لغو شد — {} ویدیو تا این لحظه در {} ms",
                    out.summary.video_count, out.summary.duration_ms
                ),
            );
        } else {
            crate::logging::info(
                "scan",
                &format!(
                    "اسکن کامل شد: {} ویدیو، {} پوشه، {} ms",
                    out.summary.video_count, out.summary.dirs_scanned, out.summary.duration_ms
                ),
            );
        }
        for w in out.warnings.iter().take(20) {
            crate::logging::warn("scan", w);
        }

        let _ = app2.emit("scan:done", &out.summary);
    });

    Ok(())
}

#[tauri::command]
pub fn stop_scan(state: State<'_, AppState>) -> bool {
    // اتمیک زیر قفل Registry: چک رزرو فعال + ست پرچمِ «همان نسل» یکجا.
    // بدون اسکن فعال → false و هیچ پرچمی ست نمی‌شود؛ هر نسل اسکن پرچم تازه
    // دارد، پس درخواست قدیمی هرگز اسکن بعدی را لغو نمی‌کند.
    match request_scan_cancel(&state.ops) {
        Ok(was_scanning) => {
            if was_scanning {
                crate::logging::info("scan", "درخواست توقف اسکن ثبت شد");
            }
            was_scanning
        }
        Err(err) => {
            crate::logging::warn("scan", &format!("درخواست توقف اسکن ناموفق بود: {err}"));
            false
        }
    }
}

#[tauri::command]
pub fn get_library(state: State<'_, AppState>) -> Vec<VideoEntry> {
    state.library.lock().unwrap().clone()
}

#[tauri::command]
pub fn get_last_scan(state: State<'_, AppState>) -> Option<ScanSummary> {
    state.last_scan.lock().unwrap().clone()
}

#[tauri::command]
pub fn is_scanning(state: State<'_, AppState>) -> bool {
    // از Registry می‌خواند — منبع حقیقت یکتا برای وضعیت اسکن
    // قفل مسموم: وضعیت نامشخص → false (بدون panic در مرز command)
    crate::state::is_scan_active(&state.ops).unwrap_or(false)
}
