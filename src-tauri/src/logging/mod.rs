//! لاگ برنامه: بافر در حافظه برای UI + فایل متنی روی دیسک + ارسال رخداد به فرانت‌اند.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Emitter};

use crate::models::{LogEntry, LogLevel};

const BUFFER_CAP: usize = 1000;
const MAX_LOG_FILE_BYTES: u64 = 10 * 1024 * 1024;

struct LoggerInner {
    file: Mutex<Option<File>>,
    buffer: Mutex<VecDeque<LogEntry>>,
    next_id: AtomicU64,
    app: OnceLock<AppHandle>,
}

static LOGGER: OnceLock<LoggerInner> = OnceLock::new();

pub fn init(file_path: Option<PathBuf>) {
    let _ = LOGGER.set(LoggerInner {
        file: Mutex::new(open_log_file(file_path.as_deref())),
        buffer: Mutex::new(VecDeque::with_capacity(BUFFER_CAP)),
        next_id: AtomicU64::new(1),
        app: OnceLock::new(),
    });
}

fn open_log_file(path: Option<&Path>) -> Option<File> {
    let path = path?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // جلوگیری از بی‌نهایت شدن حجم: چرخش ساده روی ۱۰ مگابایت
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > MAX_LOG_FILE_BYTES {
            let _ = std::fs::rename(path, path.with_extension("log.old"));
        }
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

pub fn set_app_handle(app: AppHandle) {
    if let Some(l) = LOGGER.get() {
        let _ = l.app.set(app);
    }
}

pub fn log(level: LogLevel, source: &str, message: &str) {
    let Some(l) = LOGGER.get() else { return };
    let entry = LogEntry {
        id: l.next_id.fetch_add(1, Ordering::Relaxed),
        timestamp: chrono::Local::now()
            .format("%Y-%m-%d %H:%M:%S%.3f")
            .to_string(),
        level,
        source: source.to_string(),
        message: message.to_string(),
    };

    {
        let mut buf = l.buffer.lock().unwrap();
        if buf.len() >= BUFFER_CAP {
            buf.pop_front();
        }
        buf.push_back(entry.clone());
    }

    if let Ok(mut guard) = l.file.lock() {
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(
                f,
                "[{}] [{:<5}] [{}] {}",
                entry.timestamp,
                level_label(&entry.level),
                entry.source,
                entry.message
            );
        }
    }

    if let Some(app) = l.app.get() {
        let _ = app.emit("log:new", &entry);
    }
}

pub fn info(source: &str, message: &str) {
    log(LogLevel::Info, source, message);
}

pub fn warn(source: &str, message: &str) {
    log(LogLevel::Warn, source, message);
}

// از فاز ۲ به بعد (موتور استخراج/دانلود/سینک) در کد اصلی استفاده می‌شود.
#[allow(dead_code)]
pub fn error(source: &str, message: &str) {
    log(LogLevel::Error, source, message);
}

/// آخرین `limit` لاگ (به ترتیب قدیم به جدید).
pub fn recent(limit: usize) -> Vec<LogEntry> {
    let Some(l) = LOGGER.get() else {
        return Vec::new();
    };
    let buf = l.buffer.lock().unwrap();
    let skip = buf.len().saturating_sub(limit);
    buf.iter().skip(skip).cloned().collect()
}

fn level_label(level: &LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "DEBUG",
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
    }
}
