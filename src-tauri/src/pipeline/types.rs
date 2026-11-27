//! انواع عمومی Pipeline — قرارداد بک‌اند/فرانت‌اند (همه camelCase).
//!
//! تفکیک مفاهیم:
//! - `PipelineStage`: الان کدام مرحله است.
//! - `StageStatus`: وضعیت همین مرحله برای یک ویدیو (اطلاعاتی؛ نهایی نیست).
//! - `ItemOutcome`: وضعیت پایانی ویدیو (فقط یک‌بار ست می‌شود و `completed` را می‌سازد).
//! - `ItemArtifacts`: مسیرها/داده‌های قابل گزارش بدون parse رشته.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::models::VideoEntry;
use crate::subsource::plan::SidecarConflict;

/// ورودی pipeline — v1 فقط انتخاب دقیق فایل‌ها (هرگز والد اسکن نمی‌شود).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PipelineInput {
    Files { paths: Vec<String> },
}

/// مراحل pipeline — barrier: هر مرحله تا همهٔ ویدیوها resolved نشوند جلوتر نمی‌رود.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PipelineStage {
    Validating,
    Scanning,
    EvaluatingExisting,
    Analyzing,
    Extracting,
    Downloading,
    PlanningArchives,
    ExtractingArchives,
    SelectingSyncInput,
    Syncing,
    Finalizing,
    Done,
}

/// وضعیت مرحلهٔ جاری برای یک ویدیو — اطلاعاتی؛ نهایی نیست.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum StageStatus {
    Pending,
    InProgress,
    /// این مرحله برای این ویدیو با دلیل پرش شد (اطلاعاتی — نتیجه نهایی نیست)
    Skipped {
        reason: String,
    },
    /// شکست این مرحله برای این ویدیو (بقیهٔ مراحل ممکن است ادامه یابد)
    Failed {
        reason: String,
    },
    /// ویدیو منتظر تصمیم کاربر است (decision_id مات backend-owned)
    NeedsDecision {
        decision_id: String,
    },
    Done,
}

/// وضعیت پایانی یک ویدیو — فقط یک‌بار ست می‌شود و `completed` می‌سازد.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum ItemOutcome {
    /// زیرنویس سینک‌شدهٔ معتبر از قبل موجود است (بازبینی تازهٔ فایل‌سیستم)
    AlreadySynced,
    /// فایل مقصد/کناری نامعتبر موجود است — حفظ شده؛ مسیرش گزارش می‌شود
    OutputConflict {
        preserved_path: String,
    },
    /// سینک موفق و جانمایی شد
    Synced {
        output_path: String,
    },
    /// زیرنویس سینک‌شده با موفقیت داخل خود ویدیو قرار گرفت.
    SoftSubbed {
        video_path: String,
    },
    /// زیرنویس فارسی داخل تصویر حک شده است (قرارداد نام‌گذاری گروه‌های رلیز
    /// مثل Film2Media) — دانلود خودکار بی‌معناست؛ کاربر می‌تواند پس از اجرا
    /// از اقدام «افزودن زیرنویس چسبیده» دانلود یا فایل محلی را هم‌نام ویدیو بگذارد.
    Hardsubbed,
    /// کیفیت هم‌ترازی پایین بود — ورودی دست‌نخورده ماند؛ `reason` خلاصهٔ
    /// انسانی تلاش‌هاست (کدام استراتژی‌ها، چه آفست‌هایی رد شدند)
    UnchangedLowQuality {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// برای دانلود به API key نیاز دارد (فقط ویدیوهای واقعاً نیازمند دانلود)
    NeedsApiKey,
    /// مخزن کلید (Credential Manager) خطا داد — جدا از «کلید نیست»
    KeyStoreUnavailable {
        reason: String,
    },
    Skipped {
        reason: String,
    },
    Failed {
        reason: String,
    },
    Cancelled,
}

/// مسیرها/داده‌های قابل گزارش هر ویدیو — بدون هیچ parse رشته‌ای.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemArtifacts {
    /// ورودی سینک انتخاب/رزروشده (از بازبینی تازه یا استخراج/جانمایی موفق)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_input_path: Option<String>,
    /// زیرنویس سینک‌شدهٔ جانمایی‌شده
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placed_path: Option<String>,
    /// فایل تعارض حفظ‌شده (دست‌نخورده)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict_path: Option<String>,
    /// کاندیدای موقت سینک داخل workspace (در تعارض حفظ می‌ماند)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_candidate_path: Option<String>,
    /// ریشه‌های staging دانلود — برای بازیابی دستی
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub staged_paths: Vec<String>,
    /// مسیرهای sidecar فارسی معتبر (چندتایی → تصمیم کاربر در آینده)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub valid_sidecar_paths: Vec<String>,
    /// تعارض‌های sidecar حفظ‌شده
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sidecar_conflicts: Vec<SidecarConflict>,
    /// آفست گزارش‌شدهٔ سینک
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset_seconds: Option<f64>,
    /// انتخاب آرشیو (از Ready پلن یا resolve تصمیم کاربر) برای مرحلهٔ اجرا
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_selection: Option<PreSelectionArtifact>,
}

/// انتخاب آرشیو نهایی هر ویدیو — از پلن Ready یا تصمیم کاربر.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreSelectionArtifact {
    pub archive_file_name: String,
    pub member_path: String,
}

/// snapshot یک ویدیو در pipeline — stage/status/outcome جداست.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineItemSnapshot {
    pub video_path: String,
    pub video_file_name: String,
    pub stage: PipelineStage,
    pub stage_status: StageStatus,
    pub outcome: Option<ItemOutcome>,
    #[serde(flatten)]
    pub artifacts: ItemArtifacts,
}

/// وضعیت کلی run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PipelineStatus {
    Running,
    AwaitingDecision,
    Completed,
    Cancelled,
    Failed,
}

/// شمارنده‌های واقعی — هیچ درصد ساختگی.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineCounts {
    pub analyzed: u64,
    pub extracted: u64,
    pub downloaded: u64,
    pub archives_placed: u64,
    pub synced: u64,
    pub softsubbed: u64,
    pub hardsubbed: u64,
    pub unchanged_low_quality: u64,
    pub already_synced: u64,
    pub conflicts: u64,
    pub needs_api_key: u64,
    pub key_store_unavailable: u64,
    pub skipped: u64,
    pub failed: u64,
    pub decisions_pending: u64,
}

/// snapshot کامل run — برای `get_pipeline_state` و رویداد `pipeline:state`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSnapshot {
    /// شمارهٔ یکنوای snapshot — فرانت رویدادهای قدیمی‌تر را دور می‌ریزد.
    #[serde(default)]
    pub seq: u64,
    pub run_id: String,
    pub stage: PipelineStage,
    pub status: PipelineStatus,
    pub total: usize,
    pub completed: usize,
    /// تعداد آیتم‌هایی که همین حالا در حال پردازش‌اند (مثلاً workerهای سینک).
    #[serde(default)]
    pub active: usize,
    /// درخواست توقف ثبت شده و worker هنوز به وضعیت پایانی نرسیده است.
    #[serde(default)]
    pub cancelling: bool,
    pub current_file: Option<String>,
    pub counts: PipelineCounts,
    pub message: String,
    /// پیشرفت واقعی مرحلهٔ جاری (شمارندهٔ آیتم‌های تمام‌شدهٔ همین مرحله) —
    /// شمارش صادقانهٔ بک‌اند است؛ وقتی مرحله پیشرفت آیتمی ندارد حاضر نیست.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_progress: Option<StageProgress>,
    /// تصمیم باز در صورت انتظار — رویداد `pipeline:decision` فقط یک‌بار منتشر
    /// می‌شود؛ بدون این فیلد، بازگشایی UI حین انتظار تصمیم نمی‌تواند دیالوگ را
    /// بازیابی کند و کاربر راهی برای پاسخ ندارد.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_decision: Option<PendingDecision>,
}

/// شمارندهٔ مرحلهٔ جاری — فقط از شمارش واقعی آیتم‌های تمام‌شدهٔ همان مرحله
/// ساخته می‌شود؛ هیچ تخمین یا درصد ساختگی‌ای در این مسیر وجود ندارد.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageProgress {
    pub completed: u32,
    pub total: u32,
}

/// گزینهٔ ثبت‌شدهٔ backend-owned برای یک تصمیم — شناسه مات؛ فرانت هرگز
/// مسیر/metadata خام نمی‌فرستد و فقط همین idها را برمی‌گرداند.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredOption {
    pub option_id: String,
    #[serde(flatten)]
    pub payload: DecisionOptionPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DecisionOptionPayload {
    /// چند Track متنی فارسی کاندید — انتخاب استخراج مستقیم
    EmbeddedTrack { stream_index: u32, label: String },
    /// کاندیدای آرشیو (نام آرشیو + مسیر عضو)
    ArchiveCandidate {
        archive_file_name: String,
        member_path: String,
    },
    /// مسیر sidecar معتبر (چند کاندیدا → کاربر انتخاب می‌کند)
    SidecarPath { path: PathBuf },
    /// Track متنی مشکوک/بدون نشانه → مسیر استخراج آزمایشی برای بررسی محتوا
    ReviewTrack { stream_index: u32, label: String },
}

/// تصمیم بازِ منتشرشده به فرانت — در snapshot هم می‌آید.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDecision {
    pub decision_id: String,
    pub kind: DecisionKind,
    pub video_path: String,
    pub video_file_name: String,
    pub options: Vec<RegisteredOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DecisionKind {
    EmbeddedTrack,
    ReviewTrack,
    ArchiveCandidate,
    SyncSidecar,
}

/// خلاصهٔ نهایی run — صادقانه: شمارش‌ها + موارد + مسیرهای قابل بازیابی.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSummary {
    pub run_id: String,
    pub status: PipelineStatus,
    pub cancelled: bool,
    pub counts: PipelineCounts,
    pub items: Vec<PipelineItemSnapshot>,
    /// مسیرهای انتخابی نامعتبر (ساختاریافته — از ScanOutput.issues)
    pub invalid_selections: Vec<crate::scanner::SelectionIssue>,
    /// ریشه‌های staging دانلود — بعد از restart منبع حقیقت نیستند (فقط برای بازیابی دستی)
    pub staged_paths: Vec<String>,
    pub warnings: Vec<String>,
    /// خطای مخزن کلید (جدا از «کلید تنظیم نشده») در صورت وقوع
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_store_error: Option<String>,
    pub duration_ms: u64,
    /// همهٔ منابع درون‌حافظه‌ای (provenance/workspace) با پایان process از بین می‌روند
    pub ephemeral: bool,
}

/// رویدادهای pipeline — mapping مستقیم به رویدادهای Tauri در command layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "event")]
pub enum PipelineEvent {
    State { snapshot: PipelineSnapshot },
    Item { item: PipelineItemSnapshot },
    Decision { decision: PendingDecision },
    Done { summary: PipelineSummary },
    LibraryReplaced { entries: Vec<VideoEntry> },
    LibraryEntry { entry: VideoEntry },
}

/// snapshot آیتم از دید engine — نسخهٔ کامل شامل داده‌های غیر-serialize.
#[derive(Debug, Clone)]
pub(crate) struct ItemRuntime {
    pub video_path: String,
    pub video_file_name: String,
    pub stage: PipelineStage,
    pub stage_status: StageStatus,
    pub outcome: Option<ItemOutcome>,
    pub artifacts: ItemArtifacts,
    /// نتیجهٔ بازبینی sidecar از EvaluatingExisting — مرجع مراحل بعدی
    pub sidecar_valid: Vec<PathBuf>,
    pub sidecar_conflicts: Vec<SidecarConflict>,
    /// کد skip دانلود در صورت شکست بازبینی (SidecarInspectionFailed)
    pub inspection_failed: bool,
    /// آیا استخراج/بررسی/جانمایی موفق sidecar تازه ساخته است
    pub has_sidecar: bool,
    /// stream داخلی فارسی‌ای که در SoftSub با نسخهٔ سینک‌شده جایگزین می‌شود.
    pub embedded_stream_index: Option<u32>,
    /// ورودی sync در همین اجرای pipeline ساخته شده و پس از mux موفق قابل حذف است.
    pub generated_sync_input: bool,
    /// آرشیوهای دانلودشدهٔ متناظر این ویدیو
    pub archive_sources: Vec<crate::subsource::plan::ArchiveSource>,
}

impl ItemRuntime {
    pub(crate) fn snapshot(&self) -> PipelineItemSnapshot {
        PipelineItemSnapshot {
            video_path: self.video_path.clone(),
            video_file_name: self.video_file_name.clone(),
            stage: self.stage,
            stage_status: self.stage_status.clone(),
            outcome: self.outcome.clone(),
            artifacts: self.artifacts.clone(),
        }
    }
}
