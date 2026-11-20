//! انواع هاردساب — قرارداد بک‌اند/فرانت‌اند (همه camelCase).
//!
//! هاردساب یک عملیات گروهی مستقل پس از سینک است: خروجی همیشه **فایل جدید**
//! کنار ویدیو است (`<stem>.fa.hardsubbed.<ext>`) و فایل اصلی هرگز تغییر
//! نمی‌کند؛ اگر خروجی از قبل موجود باشد آن مورد رد می‌شود (بدون بازنویسی).

use serde::{Deserialize, Serialize};

/// آیتم پلن هاردساب — یک ویدیو با منبع زیرنویس مشخص.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardsubPlanItem {
    pub video_path: String,
    pub video_file_name: String,
    /// زیرنویس sidecar روی فایل‌سیستم؛ None = منبع داخلی.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle_path: Option<String>,
    /// Track متنی فارسی داخلی (index گلوبال) — استخراج در زمان اجرا.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedded_stream_index: Option<u32>,
    /// مسیر خروجی نهایی — هنگام شروع هرگز موجود نیست.
    pub output_path: String,
}

/// مورد ردشده در پلن — با دلیل صادقانه.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardsubPlanSkip {
    pub path: String,
    pub reason: String,
}

/// پلن هاردساب — نتیجهٔ بازبینی فایل‌سیستم/ffprobe؛ توصیه است و اجرا
/// پلن تازهٔ خودش را می‌سازد (منبع حقیقت).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardsubPlan {
    pub items: Vec<HardsubPlanItem>,
    pub skipped: Vec<HardsubPlanSkip>,
    /// انکودر نهایی انتخاب‌شده (اثبات‌شده با probe در حالت Auto).
    pub encoder: String,
}

/// وضعیت یک آیتم در طول اجرا — فقط یک‌بار به وضعیت پایانی می‌رسد.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum HardsubItemStatus {
    Pending,
    /// استخراج Track متنی داخلی به زیرنویس موقت (سورس SoftSub).
    Extracting,
    /// در حال انکود — درصد فقط وقتی مدت ویدیو معلوم است؛ هرگز ساختگی نیست.
    Encoding {
        percent: Option<u32>,
    },
    /// خروجی ساخته، اعتبارسنجی و با نام نهایی نصب شد.
    Done {
        output_path: String,
    },
    /// خروجی مقصد در فاصلهٔ پلن تا نصب ساخته شد — هرگز بازنویسی نمی‌کنیم.
    Conflict,
    Failed {
        reason: String,
    },
    Cancelled,
}

impl HardsubItemStatus {
    /// آیا وضعیت پایانی است؟ (شمارندهٔ completed فقط از این‌ها ساخته می‌شود)
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            HardsubItemStatus::Done { .. }
                | HardsubItemStatus::Conflict
                | HardsubItemStatus::Failed { .. }
                | HardsubItemStatus::Cancelled
        )
    }
}

/// snapshot یک آیتم — انکودر واقعی مصرف‌شدهٔ همان آیتم (بعد از fallback CPU).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardsubItemSnapshot {
    pub video_path: String,
    pub video_file_name: String,
    pub status: HardsubItemStatus,
    /// انکودر مصرف‌شدهٔ این آیتم (None تا شروع انکود).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder: Option<String>,
}

/// شمارنده‌های واقعی — هیچ درصد ساختگی.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardsubCounts {
    pub done: u64,
    pub failed: u64,
    pub conflicts: u64,
    pub cancelled: u64,
}

/// snapshot کامل اجرا — برای `get_hardsub_state` و رویداد `hardsub:state`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardsubSnapshot {
    /// شمارهٔ یکنوای snapshot — فرانت رویدادهای قدیمی‌تر را دور می‌ریزد.
    #[serde(default)]
    pub seq: u64,
    pub run_id: String,
    pub running: bool,
    /// درخواست توقف ثبت شده و workerها هنوز به وضعیت پایانی نرسیده‌اند.
    #[serde(default)]
    pub cancelling: bool,
    pub total: usize,
    pub completed: usize,
    pub items: Vec<HardsubItemSnapshot>,
    pub counts: HardsubCounts,
}

/// خلاصهٔ نهایی اجرا — event نهایی و منبع حقیقت پس از پایان.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardsubSummary {
    pub run_id: String,
    pub cancelled: bool,
    pub counts: HardsubCounts,
    pub items: Vec<HardsubItemSnapshot>,
    pub duration_ms: u64,
}

/// رویدادهای هاردساب — mapping مستقیم به رویدادهای Tauri در command layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "event")]
pub enum HardsubEvent {
    State { snapshot: HardsubSnapshot },
    Done { summary: HardsubSummary },
}
