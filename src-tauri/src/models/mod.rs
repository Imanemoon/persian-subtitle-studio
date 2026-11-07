use serde::{Deserialize, Serialize};

/// یک فایل ویدیویی شناسایی‌شده در اسکن کتابخانه.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VideoEntry {
    /// شناسه یکتا — همان مسیر کامل فایل.
    pub id: String,
    pub path: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub modified_ms: Option<u64>,
    pub parent_dir: String,
    pub show_name: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    /// زیرنویس‌های کنار ویدیو (همان پوشه) که بر اساس نام فایل تطبیق داده شده‌اند.
    pub sibling_subtitles: Vec<SubtitleFileInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleFileInfo {
    pub path: String,
    pub file_name: String,
    pub ext: String,
    pub size_bytes: u64,
    /// آیا در نام فایل نشانه فارسی (fa/farsi/persian/...) وجود دارد؟
    pub name_suggests_fa: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub dirs_scanned: u64,
    pub files_seen: u64,
    pub videos_found: u64,
    pub current_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub root: String,
    pub dirs_scanned: u64,
    pub files_seen: u64,
    pub video_count: u64,
    pub excluded_dir_hits: u64,
    /// ویدیوهایی که فصل/قسمت در نامشان تشخیص داده نشد.
    pub parse_failed: u64,
    /// ویدیوهایی که زیرنویس کناری با نشانه فارسی دارند.
    pub with_fa_sidecar: u64,
    /// ویدیوهایی که هر نوع زیرنویس کناری دارند.
    pub with_any_sidecar: u64,
    pub duration_ms: u64,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ToolStatus {
    Found,
    Missing,
    Broken,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    /// شناسه داخلی: ffmpeg | ffprobe | sevenZip | subsource | ffsubsync
    pub id: String,
    pub name: String,
    pub status: ToolStatus,
    pub path: Option<String>,
    pub version: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub id: u64,
    pub timestamp: String,
    pub level: LogLevel,
    pub source: String,
    pub message: String,
}
