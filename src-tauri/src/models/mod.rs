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
    /// نتیجه تحلیل زیرنویس‌های داخلی (فاز ۲) — `None` یعنی هنوز تحلیل نشده.
    pub analysis: Option<VideoAnalysis>,
}

/// نوع کدک زیرنویس.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubtitleKind {
    Text,
    Image,
    Unknown,
}

/// وضعیت تحلیل/استخراج زیرنویس داخلی هر ویدیو.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AnalysisStatus {
    NotAnalyzed,
    Analyzing,
    /// دقیقاً یک Track متنی فارسی — استخراج بدون ابهام
    PersianTextTrackFound,
    /// فقط Track تصویری فارسی (PGS/VobSub/DVB) — استخراج متن به OCR نیاز دارد
    ImageSubtitleFound,
    /// هیچ Stream زیرنویسی داخل فایل نیست
    NoEmbeddedSubtitle,
    /// زیرنویس داخلی هست اما Track فارسی در آن نیست
    NoPersianTrack,
    Extracting,
    Extracted,
    /// زیرنویس موجود/خروجی خالی یا فاقد ساختار معتبر SRT
    InvalidSubtitle,
    /// بیش از یک Track فارسی — انتخاب کاربر لازم است
    NeedsTrackSelection,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleStream {
    /// Global stream index در فایل — برای `-map 0:<index>`
    pub stream_index: u32,
    pub codec_name: String,
    pub kind: SubtitleKind,
    pub language: Option<String>,
    pub language_ietf: Option<String>,
    pub title: Option<String>,
    pub default_disposition: bool,
    pub forced_disposition: bool,
    /// زبان/عنوان نشانه فارسی دارد (محتوا هنوز تأیید نشده)
    pub persian_candidate: bool,
    /// محتوای این Track با استخراج آزمایشی به‌عنوان فارسی تأیید شده است
    /// (برای Trackهای بدون Tag معتبر — فقط جریان review این را set می‌کند)
    pub persian_confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VideoAnalysis {
    pub status: AnalysisStatus,
    pub analyzed_at_ms: u64,
    pub streams: Vec<SubtitleStream>,
    /// توضیح/خطا برای نمایش در UI و لاگ
    pub message: Option<String>,
    /// نتیجه آخرین استخراج این ویدیو
    pub last_extraction: Option<ExtractionOutcome>,
    /// استخراج‌های آزمایشی (بررسی محتوا) انجام‌شده روی این ویدیو
    #[serde(default)]
    pub reviews: Vec<TrackReview>,
}

/// نتیجه «استخراج آزمایشی برای بررسی» یک Track متنی — مستقل از نام‌گذاری استاندارد.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrackReview {
    pub stream_index: u32,
    /// مسیر فایل بررسی: `<base>.track-<N>.review.srt`
    pub review_path: String,
    pub validation: SubtitleValidation,
    /// آیا محتوای بررسی‌شده واقعاً فارسی بود؟
    pub persian_confirmed: bool,
    /// آیا بعداً به `<base>.fa.srt` استاندارد تبدیل شده؟
    pub promoted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionOutcome {
    pub stream_index: u32,
    pub target_path: String,
    pub validation: SubtitleValidation,
    /// آیا محتوای خروجی واقعاً فارسی بود؟ (Tag فارسی به‌تنهایی کافی نیست)
    pub persian_confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleValidation {
    pub valid: bool,
    pub size_bytes: u64,
    pub encoding: String,
    pub line_count: usize,
    /// تعداد خطوط تایم‌کد معتبر (`00:00:01,000 --> ...`)
    pub cue_count: usize,
    pub has_persian: bool,
    pub persian_char_count: usize,
    pub error: Option<String>,
}

/// یک عملیات استخراج تأییدشده توسط کاربر (Extract All).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionPlanItem {
    pub video_path: String,
    pub video_file_name: String,
    pub stream_index: u32,
    pub track_label: String,
    pub target_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkippedExtraction {
    pub video_path: String,
    pub video_file_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionPlan {
    pub items: Vec<ExtractionPlanItem>,
    pub skipped: Vec<SkippedExtraction>,
}

/// پیشرفت عملیات گروهی (Analyze All / Extract Plan).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OpsProgress {
    /// "analyze" | "extract"
    pub kind: String,
    pub current: u64,
    pub total: u64,
    pub video_path: String,
    pub message: String,
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
    /// مسیرهای پذیرفته‌شده در اسکنِ «فایل‌های انتخابی» (برای اسکن پوشه خالی).
    #[serde(default)]
    pub selected_files: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_summary_selected_files_defaults_for_old_payloads() {
        // payload قدیمی (پیش از افزودن selectedFiles) باید بدون خطا deserialize شود
        let json = r#"{
            "root": "D:/Media",
            "dirsScanned": 3,
            "filesSeen": 5,
            "videoCount": 2,
            "excludedDirHits": 0,
            "parseFailed": 1,
            "withFaSidecar": 1,
            "withAnySidecar": 1,
            "durationMs": 42,
            "cancelled": false
        }"#;
        let summary: ScanSummary = serde_json::from_str(json).unwrap();
        assert_eq!(summary.root, "D:/Media");
        assert!(summary.selected_files.is_empty());

        // راندتریپ کامل هم باید فیلد جدید را حفظ کند
        let full = ScanSummary {
            selected_files: vec!["D:/Media/a.mkv".into()],
            ..Default::default()
        };
        let back: ScanSummary =
            serde_json::from_str(&serde_json::to_string(&full).unwrap()).unwrap();
        assert_eq!(back.selected_files, vec!["D:/Media/a.mkv".to_string()]);
    }
}
