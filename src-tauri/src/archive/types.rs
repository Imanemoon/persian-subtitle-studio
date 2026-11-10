//! انواع داده‌ای پلن/اجرای استخراج آرشیو — سریال‌سازی camelCase برای فرانت.

use serde::{Deserialize, Serialize};

pub use crate::subsource::plan::ArchiveSource;

/// پیشوند پوشه‌های استخراج موقت — همیشه داخل ریشهٔ staging ساخته می‌شوند و
/// فقط همان‌ها توسط برنامه پاک می‌شوند.
pub const EXTRACT_DIR_PREFIX: &str = "_subsource_extract_";

/// حداکثر اندازهٔ مجاز یک زیرنویس (جلوگیری از فایل‌های غول‌آسا/بمب).
pub const MAX_SUBTITLE_BYTES: u64 = 32 * 1024 * 1024;

/// حداکثر مجموع اندازهٔ بازشدهٔ یک آرشیو پیش از استخراج.
pub const MAX_ARCHIVE_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;

/// یک آرشیو شناسایی‌شده با magic bytes داخل `sub/`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveFileInfo {
    pub file_name: String,
    /// `zip` یا `rar4` یا `rar5`
    pub format: String,
    pub size_bytes: u64,
}

/// کاندید زیرنویس معتبر (ساختار درست + محتوای فارسی + تطبیق نام).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleCandidate {
    pub archive_file_name: String,
    /// مسیر عضو داخل آرشیو (با `/` نرمال‌شده).
    pub member_path: String,
    /// پسوند اصلی زیرنویس — هرگز تغییر نمی‌کند (ASS/SSA/SUB به SRT تبدیل نمی‌شود).
    pub subtitle_extension: String,
    pub size_bytes: u64,
    /// `1080p` / `720p` / `other`
    pub quality: String,
}

/// انتخاب صریح کاربر برای رفع ابهام (از پلن قبلی).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSelection {
    pub archive_file_name: String,
    pub member_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ArchivePlanStatus {
    /// دقیقاً یک کاندید معتبر — قابل ارتقای خودکار.
    Ready,
    /// چند کاندید معتبر — بدون انتخاب کاربر اجرا نمی‌شود.
    Ambiguous,
    /// هیچ کاندید معتبری نیست (دلیل در message).
    NoMatch,
}

/// نتیجهٔ پلن برای یک ویدیو.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArchivePlanItem {
    pub video_path: String,
    pub video_file_name: String,
    pub staging_root: String,
    pub relative_video_dir: String,
    pub archive_dir: String,
    pub archives: Vec<ArchiveFileInfo>,
    /// کاندیدهای معتبر، مرتب‌شده بر اساس کیفیت/اندازه.
    pub candidates: Vec<SubtitleCandidate>,
    pub status: ArchivePlanStatus,
    /// فقط برای `Ready` پر است.
    pub selection: Option<SubtitleCandidate>,
    pub message: String,
}

impl ArchivePlanItem {
    pub fn empty(source: &ArchiveSource, archive_dir: String, message: String) -> Self {
        Self {
            video_path: source.video_path.clone(),
            video_file_name: source.video_file_name.clone(),
            staging_root: source.staging_root.clone(),
            relative_video_dir: source.relative_video_dir.clone(),
            archive_dir,
            archives: Vec::new(),
            candidates: Vec::new(),
            status: ArchivePlanStatus::NoMatch,
            selection: None,
            message,
        }
    }
}

/// پلن کامل استخراج — خروجی `plan_archive_extraction`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveExtractionPlan {
    pub items: Vec<ArchivePlanItem>,
    pub ready: usize,
    pub ambiguous: usize,
    pub no_match: usize,
    /// پلن به‌دلیل لغو پیش از پردازش همهٔ منابع ناقص ماند.
    #[serde(default)]
    pub cancelled: bool,
    /// هشدارهای پاک‌سازی مسیرهای موقت (شکست واقعی).
    pub warnings: Vec<String>,
}

/// یک درخواست اجرا: منبع staging + انتخاب اختیاری (برای موارد Ambiguous).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveExecuteRequest {
    pub source: ArchiveSource,
    #[serde(default)]
    pub selection: Option<ArchiveSelection>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ArchiveItemStatus {
    Placed,
    Ambiguous,
    NoMatch,
    Failed,
    Cancelled,
}

/// نتیجهٔ اجرا برای یک ویدیو.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveExecuteItemResult {
    pub video_path: String,
    pub video_file_name: String,
    pub status: ArchiveItemStatus,
    pub output_path: Option<String>,
    /// هشدارهای غیرکشنده (مثلاً خروج ۱ از 7-Zip).
    pub warnings: Vec<String>,
    pub message: String,
}

/// نتیجهٔ کامل اجرای استخراج.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveExecuteResult {
    pub items: Vec<ArchiveExecuteItemResult>,
    pub placed: usize,
    pub ambiguous: usize,
    pub no_match: usize,
    pub failed: usize,
    pub cancelled: bool,
    pub success: bool,
    /// هشدارهای غیرکشنده (خروج ۱ از 7-Zip).
    pub warnings: Vec<String>,
    /// خطاهای پاک‌سازی پوشه‌های موقتِ خود برنامه.
    pub cleanup_warnings: Vec<String>,
    pub message: String,
    pub duration_secs: f64,
}

impl ArchiveExecuteResult {
    pub fn recalculate(&mut self) {
        self.placed = self
            .items
            .iter()
            .filter(|i| i.status == ArchiveItemStatus::Placed)
            .count();
        self.ambiguous = self
            .items
            .iter()
            .filter(|i| i.status == ArchiveItemStatus::Ambiguous)
            .count();
        self.no_match = self
            .items
            .iter()
            .filter(|i| i.status == ArchiveItemStatus::NoMatch)
            .count();
        self.failed = self
            .items
            .iter()
            .filter(|i| i.status == ArchiveItemStatus::Failed)
            .count();
        self.success = !self.cancelled
            && self.failed == 0
            && self.ambiguous == 0
            && self.placed > 0
            && self.cleanup_warnings.is_empty();
    }
}
