//! دستور دریافت لاگ‌های اخیر.

use crate::models::LogEntry;

#[tauri::command]
pub fn get_logs(limit: Option<u32>) -> Vec<LogEntry> {
    crate::logging::recent(limit.unwrap_or(300).min(1000) as usize)
}
