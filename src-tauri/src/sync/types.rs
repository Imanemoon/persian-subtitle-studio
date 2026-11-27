//! انواع عمومی آداپتور FFsubsync — کاملاً مستقل از Tauri.
//!
//! این ماژول فقط هستهٔ اجراست: ورودی/خروجی ساختاریافته می‌گیرد و کاندیدای
//! موقتِ سینک‌شده را برای مرحلهٔ جانمایی بعدی (pipeline) برمی‌گرداند. هیچ
//! جانمایی کنار ویدیو، هیچ worker pool و هیچ وابستگی به AppHandle/State
//! اینجا وجود ندارد.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::ProcessingSettings;

/// markerهای لاگ ffsubsync 0.5.1 (تأییدشده از سورس نصب‌شده) — نسخه‌محور.
/// اگر نسخه‌ای این متن‌ها را عوض کند، طبقه‌بندی fail-closed می‌ماند:
/// بدون marker آفست، هیچ خروجی‌ای «سینک‌شده» شمرده نمی‌شود.
pub const LOW_QUALITY_MARKER: &str = "leaving subtitles unmodified";
pub const OFFSET_MARKER: &str = "offset seconds:";

/// سقف جستجوی آفست ffsubsync (ثانیه) — همان پیش‌فرض 0.5.1؛ صریح پین می‌شود
/// تا رفتار بین نسخه‌ها ثابت بماند.
pub(crate) const SEARCH_MAX_OFFSET_SECONDS: u32 = 60;

/// آستانهٔ گیت کیفیت `--skip-sync-on-low-quality` (ثانیه) — همان پیش‌فرض 0.5.1
/// (۳۰ ثانیه)؛ صریح پین می‌شود و عمداً کوچک نگه داشته می‌شود.
///
/// شواهد واقعی (رسانهٔ واقعی، ffsubsync 0.5.1، فایل 6CH پر موسیقی): چهار اجرای
/// صوتی چهار «بهترین آفست» متفاوت داد (+36.11، +32.6، ‎-8.64، ‎-297.39) — اسکور
/// خام FFT نرمال‌شده نیست و روی VAD کم‌بازده پیک‌های کاذبِ بزرگ برنده می‌شوند.
/// هم‌ترازیِ معتبرِ زیرنویس‌به‌زیرنویس (مرجع Track داخلی) برای همان فایل آفست
/// 0.000 با اسکور ۳ برابر داد — یعنی زیرنویس از قبل سینک بود و آفست 36.1s پیک
/// کاذب بود. پس آفست‌های ۳۰–۶۰ ثانیه‌ایِ صوتی به‌احتمال غلط‌اند؛ راه درستشان
/// استراتژی مرجع کامل است، نه بالا بردن این آستانه.
pub(crate) const QUALITY_MAX_OFFSET_SECONDS: u32 = 30;

/// سقف کاراکترهای diagnostic در خروجی — متن خام فرایند هرگز کامل به UI نمی‌رسد.
pub const DIAGNOSTIC_TAIL_CHARS: usize = 2000;

/// سقف کاراکترهای خلاصهٔ انسانی تلاش‌ها — خطوط کوتاهِ شمارش تلاش‌ها هستند و
/// عملاً خیلی زیر این سقف می‌مانند.
pub(crate) const SUMMARY_MAX_CHARS: usize = 600;

/// سقف بایت‌های ثبت‌شده از هر pipe (آخرین بخش نگه داشته می‌شود).
pub(crate) const CAPTURE_CAP_BYTES: usize = 128 * 1024;

/// سقف خواندن tail فایل لاگ ffsubsync — فایل کامل هرگز به حافظه کشیده نمی‌شود.
pub(crate) const LOG_TAIL_LIMIT_BYTES: u64 = 128 * 1024;

/// تنظیمات اجرای سینک — قرینهٔ `ProcessingSettings`.
/// (`sync_workers` در این سطح معنا ندارد؛ تعداد فرایندهای موازی را
/// pipeline آینده کنترل می‌کند و هر فرایند همیشه `--parallel-workers 1` می‌گیرد.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfig {
    pub multi_segment: bool,
    pub segment_count: u32,
    pub skip_intro_outro: bool,
    pub skip_sync_on_low_quality: bool,
}

impl SyncConfig {
    /// مقدار قطعیِ امن برای تعداد segmentها (تنظیمات sanitized کلامپ شده؛ دوباره محک می‌زنیم).
    pub(crate) fn effective_segment_count(&self) -> u32 {
        self.segment_count.max(1)
    }
}

impl From<&ProcessingSettings> for SyncConfig {
    fn from(settings: &ProcessingSettings) -> Self {
        SyncConfig {
            multi_segment: settings.multi_segment,
            segment_count: settings.segment_count,
            skip_intro_outro: settings.skip_intro_outro,
            skip_sync_on_low_quality: settings.skip_sync_on_low_quality,
        }
    }
}

/// درخواست اجرای سینک برای یک ویدیو — همهٔ مسیرها صریح و از قبل resolve‌شده.
/// هیچ مقدار secret (مثل SubSource API key) در این ساختار جایی ندارد.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRequest {
    /// ویدیوی مرجع (صوتی/VAD) — فقط خوانده می‌شود.
    pub video_path: PathBuf,
    /// زیرنویس فارسیِ از پیش اعتبارسنجی‌شده — هرگز تغییر نمی‌کند.
    pub subtitle_path: PathBuf,
    /// مسیر اجرایی ffs (از resolver ابزارها) — مستقیم اجرا می‌شود، بدون shell.
    pub ffsubsync_path: PathBuf,
    /// مسیر FFmpeg برای `--ffmpeg-path`؛ اگر None باشد ffsubsync از PATH می‌خواند.
    pub ffmpeg_path: Option<PathBuf>,
    pub config: SyncConfig,
    /// ریشهٔ ساخت workspace موقت؛ None = پوشهٔ temp سیستم.
    pub workspace_root: Option<PathBuf>,
    /// شمارهٔ تلاش — ۰/None = اجرای اولیه؛ ≥۱ = تلاشِ تنظیم دوباره. نردبان
    /// استراتژی‌ها طوری می‌چرخد که هر تنظیم دوباره نتیجه‌ای واقعاً متفاوت بدهد.
    #[serde(default)]
    pub attempt: Option<u32>,
}

/// وضعیت نهایی یک اجرا — fail-closed: بدون تأیید marker آفست + خروجی معتبر فارسی،
/// هیچ اجرایی «سینک‌شده» شمرده نمی‌شود.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncStatus {
    /// سینک تأیید شد؛ کاندیدای موقت معتبر در `output_path` برای جانمایی بعدی.
    Synced,
    /// کیفیت هم‌ترازی پایین بود — خروجی unmodified دور ریخته شد و ورودی دست‌نخورده ماند.
    UnchangedLowQuality,
    /// توسط کاربر لغو شد؛ هیچ کاندیدایی باقی نمی‌ماند.
    Cancelled,
    /// ورودی نامعتبر (مسیر/فرمت/محتوا) — فرایند اصلاً اجرا نشد.
    InvalidInput,
    /// ابزار (ffs یا ffmpeg) مسیر معتبری نداشت.
    ToolMissing,
    /// فرایند اجرا نشد یا با کد خروج غیرصفر خاتمه یافت.
    ToolExecutionFailed,
    /// کد خروج صفر بود اما marker آفست پیدا نشد — نتیجهٔ سینک نامشخص.
    UnknownOutcome,
    /// خروجی ساخته شد اما معتبر/فارسی نبود — دور ریخته شد.
    OutputValidationFailed,
    /// شکست داخلی (workspace/IO) که به خود ابزار مربوط نیست.
    Failed,
}

/// نتیجهٔ ساختاریافتهٔ یک اجرا — برای pipeline/رویدادها قابل serialize.
/// `Clone`/`PartialEq` ندارد: مالکیت workspace یکتاست و handle قابل تکثیر نیست.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutcome {
    pub status: SyncStatus,
    /// فقط برای `Synced` — مسیر کاندیدای تأییدشده داخل workspace موقت.
    pub output_path: Option<PathBuf>,
    /// فقط برای `Synced` — آفست گزارش‌شدهٔ ffsubsync (ثانیه) اگر از لاگ قابل استخراج بود.
    pub offset_seconds: Option<f64>,
    /// هشدارها — مثلاً از دست رفتن استایل ASS/SSA در تبدیل به SRT.
    pub warnings: Vec<String>,
    /// تشخیص کوتاه و محدودشده (tail لاگ) — برای لاگ/عیب‌یابی، نه نمایش کامل.
    pub diagnostic: String,
    /// خلاصهٔ کوتاه و انسانیِ تلاش‌های چندمرحله‌ای (بدون متن خام لاگ) — فقط
    /// وقتی بیش از یک استراتژی اجرا شده پر می‌شود؛ برای نمایش مستقیم به کاربر.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// مالکیت workspace فقط روی `Synced` — opaque، غیر-serialize، از فرانت
    /// قابل ساخت نیست؛ انتقال با `take_workspace` و حذف با `discard`.
    #[serde(skip)]
    workspace: Option<super::SyncWorkspaceHandle>,
}

impl SyncOutcome {
    pub(crate) fn bare(status: SyncStatus, diagnostic: impl Into<String>) -> Self {
        SyncOutcome {
            status,
            output_path: None,
            offset_seconds: None,
            warnings: Vec::new(),
            diagnostic: diagnostic.into(),
            summary: None,
            workspace: None,
        }
    }

    /// فقط `run_sync` (والد) پس از موفقیت مالکیت workspace را ضمیمه می‌کند.
    pub(super) fn attach_workspace(&mut self, handle: super::SyncWorkspaceHandle) {
        debug_assert!(self.workspace.is_none());
        self.workspace = Some(handle);
    }

    /// انتقال یک‌بارهٔ مالکیت به pipeline — دومین فراخوانی None می‌دهد.
    pub(crate) fn take_workspace(&mut self) -> Option<super::SyncWorkspaceHandle> {
        self.workspace.take()
    }

    /// مسیر workspace فقط برای نمایش/گزارش — هیچ توان حذفی از این مسیر نیست.
    pub(crate) fn workspace_display_root(&self) -> Option<&Path> {
        self.workspace.as_ref().map(|h| h.display_root())
    }
}

/// مراحل صادقانهٔ اجرا — هیچ درصدی وجود ندارد؛ ffsubsync پیشرفت درصدی معتبری نمی‌دهد.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncStage {
    Validating,
    Preparing,
    Running,
    Verifying,
}

/// رخداد پیشرفت — فقط مراحل واقعی و پیام متنی.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgress {
    pub stage: SyncStage,
    pub message: String,
}
