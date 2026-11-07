//! دستورات اسکن و دسترسی به کتابخانه.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, State};

use crate::models::{ScanProgress, ScanSummary, VideoEntry};
use crate::scanner;
use crate::state::AppState;

#[tauri::command]
pub fn start_scan(app: AppHandle, state: State<'_, AppState>, root: String) -> Result<(), String> {
    if state.scanning.load(Ordering::SeqCst) {
        return Err("یک اسکن در حال اجراست؛ ابتدا آن را متوقف کنید.".into());
    }

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

    state.cancel_scan.store(false, Ordering::SeqCst);
    state.scanning.store(true, Ordering::SeqCst);
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
        let out = scanner::scan_folder(
            &root_path,
            &settings.excluded_folders,
            &st.cancel_scan,
            &mut throttle,
        );

        {
            let mut lib = st.library.lock().unwrap();
            *lib = out.entries;
            *st.last_scan.lock().unwrap() = Some(out.summary.clone());
            st.scanning.store(false, Ordering::SeqCst);
        }

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
    let was_scanning = state.scanning.load(Ordering::SeqCst);
    state.cancel_scan.store(true, Ordering::SeqCst);
    if was_scanning {
        crate::logging::info("scan", "درخواست توقف اسکن ثبت شد");
    }
    was_scanning
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
    state.scanning.load(Ordering::SeqCst)
}
