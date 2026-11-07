use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use crate::models::{ScanSummary, VideoEntry};
use crate::settings::Settings;

/// وضعیت سراسری برنامه — از طریق `app.state::<AppState>()` در دسترس است.
pub struct AppState {
    pub settings: Mutex<Settings>,
    pub library: Mutex<Vec<VideoEntry>>,
    pub last_scan: Mutex<Option<ScanSummary>>,
    pub scanning: AtomicBool,
    pub cancel_scan: AtomicBool,
}

/// مسیرهای دائمی برنامه (تنظیمات و لاگ).
#[derive(Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
}
