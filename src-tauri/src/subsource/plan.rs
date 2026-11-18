//! Download Plan — طرح دانلود گروهی زیرنویس با SubSource (فاز ۳ PR ۲).
//!
//! ساخت پلن **هیچ تغییری روی دیسک نمی‌دهد**؛ فقط ویدیوها را بر اساس وضعیت
//! واقعی فایل‌ها غربال می‌کند و دلیل رد شدن هر مورد را ثبت می‌کند.
//!
//! قواعد غربال (قبل از اجرا و دوباره در لحظهٔ اجرا — هرگز به ورودی فرانت
//! اعتماد نمی‌شود):
//! - ویدیو باید در کتابخانه اسکن‌شده باشد.
//! - فایل ویدیو باید روی دیسک موجود و از پسوند پشتیبانی‌شده باشد.
//! - مسیر باید مطلق باشد.
//! - وضعیت sidecarهای فارسی از **فایل‌سیستم تازه** بررسی می‌شود — نه از
//!   `VideoEntry.sibling_subtitles` ذخیره‌شده. فقط زیرنویسی که ساختارش معتبر
//!   و محتوایش واقعاً فارسی باشد «موجود» شمرده می‌شود؛ فایل فارسی‌نامِ خراب
//!   یک تعارض است (حفظ می‌شود، هرگز بازنویسی/حذف نمی‌شود) و مانع دانلود است.
//! - مسیرهای تکراری فقط یک‌بار پذیرفته می‌شوند.
//!
//! همهٔ ردشدگی‌ها علاوه بر پیام انسانی، `SkipReasonCode` typed دارند؛ pipeline
//! فقط کدها را مصرف می‌کند و هرگز رشته‌ها را parse نمی‌کند.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::archive::listing::is_link_or_reparse;
use crate::models::VideoEntry;
use crate::scanner::naming::{
    is_subtitle_file, is_video_file, name_suggests_fa, sub_matches_video_stem,
};
use crate::subtitle::validation::validate_file;

/// یک ویدیوی واجد شرایط دانلود — در پلن تأییدشده کاربر.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadPlanItem {
    pub video_path: String,
    pub video_file_name: String,
}

/// کد پایدار دلیل رد شدن — برای مصرف typed در pipeline/UI.
/// wire format رشته‌های camelCase فعلی حفظ شده است.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SkipReasonCode {
    /// payload قدیمی یا کد ناشناختهٔ نسخه‌های آینده
    #[default]
    Unknown,
    NotInLibrary,
    NotAbsolute,
    MissingFile,
    UnsupportedFormat,
    ValidSidecarExists,
    SidecarConflict,
    DuplicatePath,
    /// ویدیو بین ساخت پلن و اجرا غیب شد
    MissingAtExecution,
    /// sidecar معتبر بین ساخت پلن و اجرا ظاهر شد
    SidecarAppearedAtExecution,
    /// وضعیت sidecarها قابل تشخیص نبود — fail-closed
    SidecarInspectionFailed,
    StagingVolumeUnavailable,
    StagingPathCollision,
    StagingBuildFailed,
}

impl SkipReasonCode {
    fn as_json_str(&self) -> &'static str {
        match self {
            SkipReasonCode::Unknown => "unknown",
            SkipReasonCode::NotInLibrary => "notInLibrary",
            SkipReasonCode::NotAbsolute => "notAbsolute",
            SkipReasonCode::MissingFile => "missingFile",
            SkipReasonCode::UnsupportedFormat => "unsupportedFormat",
            SkipReasonCode::ValidSidecarExists => "validSidecarExists",
            SkipReasonCode::SidecarConflict => "sidecarConflict",
            SkipReasonCode::DuplicatePath => "duplicatePath",
            SkipReasonCode::MissingAtExecution => "missingAtExecution",
            SkipReasonCode::SidecarAppearedAtExecution => "sidecarAppearedAtExecution",
            SkipReasonCode::SidecarInspectionFailed => "sidecarInspectionFailed",
            SkipReasonCode::StagingVolumeUnavailable => "stagingVolumeUnavailable",
            SkipReasonCode::StagingPathCollision => "stagingPathCollision",
            SkipReasonCode::StagingBuildFailed => "stagingBuildFailed",
        }
    }

    fn from_json_str(value: &str) -> Option<Self> {
        let code = match value {
            "unknown" => SkipReasonCode::Unknown,
            "notInLibrary" => SkipReasonCode::NotInLibrary,
            "notAbsolute" => SkipReasonCode::NotAbsolute,
            "missingFile" => SkipReasonCode::MissingFile,
            "unsupportedFormat" => SkipReasonCode::UnsupportedFormat,
            "validSidecarExists" => SkipReasonCode::ValidSidecarExists,
            "sidecarConflict" => SkipReasonCode::SidecarConflict,
            "duplicatePath" => SkipReasonCode::DuplicatePath,
            "missingAtExecution" => SkipReasonCode::MissingAtExecution,
            "sidecarAppearedAtExecution" => SkipReasonCode::SidecarAppearedAtExecution,
            "sidecarInspectionFailed" => SkipReasonCode::SidecarInspectionFailed,
            "stagingVolumeUnavailable" => SkipReasonCode::StagingVolumeUnavailable,
            "stagingPathCollision" => SkipReasonCode::StagingPathCollision,
            "stagingBuildFailed" => SkipReasonCode::StagingBuildFailed,
            _ => return None,
        };
        Some(code)
    }
}

impl Serialize for SkipReasonCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_json_str())
    }
}

impl<'de> Deserialize<'de> for SkipReasonCode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(SkipReasonCode::from_json_str(&value).unwrap_or(SkipReasonCode::Unknown))
    }
}

/// ویدیوی ردشده با دلیل — هم در ساخت پلن و هم در اعتبارسنجی مجدد اجرا.
/// `code` typed است (payload قدیمی بدون code → Unknown)؛ فیلدهای context اختیاری
/// فقط وقتی پر می‌شوند که معنا داشته باشند (sidecar معتبر/تعارض).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkippedDownload {
    pub video_path: String,
    pub video_file_name: String,
    pub reason: String,
    #[serde(default)]
    pub code: SkipReasonCode,
    /// مسیرهای sidecar معتبر وقتی کد `ValidSidecarExists` است — برای
    /// انتخاب صریح کاربر در مرحلهٔ سینک pipeline؛ هرگز اینجا انتخاب نمی‌شوند.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub valid_sidecar_paths: Vec<String>,
    /// تعارض‌های sidecar وقتی کد `SidecarConflict` است — حفظ و گزارش بدون تغییر فایل‌ها
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sidecar_conflicts: Vec<SidecarConflict>,
}

impl SkippedDownload {
    pub fn with_code(
        video_path: impl Into<String>,
        video_file_name: impl Into<String>,
        code: SkipReasonCode,
        reason: impl Into<String>,
    ) -> Self {
        SkippedDownload {
            video_path: video_path.into(),
            video_file_name: video_file_name.into(),
            reason: reason.into(),
            code,
            valid_sidecar_paths: Vec::new(),
            sidecar_conflicts: Vec::new(),
        }
    }
}

/// پلن دانلود — خروجی `plan_subsource_download` برای تأیید کاربر.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadPlan {
    pub items: Vec<DownloadPlanItem>,
    pub skipped: Vec<SkippedDownload>,
}

/// نگاشت پایدار «ویدیوی اصلی → مسیر staging همان ویدیو».
///
/// پوشهٔ آرشیو هر ویدیو دقیقاً از این اطلاعات ساخته می‌شود:
/// `staging_root / relative_video_dir / sub`
///
/// هیچ راز/API key در این ساختار نیست. این mapping پس از حذف hard linkهای
/// موقت هم معتبر می‌ماند، چون فقط به مسیرهای staging و ویدیوی اصلی اشاره
/// می‌کند.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSource {
    /// مسیر کامل ویدیوی اصلی در کتابخانه.
    pub video_path: String,
    pub video_file_name: String,
    /// ریشهٔ staging که خروجی SubSource در آن حفظ شده است.
    pub staging_root: String,
    /// مسیر نسبی ویدیو در staging (مثلاً `Season 01/Show.S01E01.mkv`).
    pub relative_video_path: String,
    /// پوشهٔ نسبی ویدیو در staging (مثلاً `Season 01`؛ برای ریشه خالی است).
    pub relative_video_dir: String,
    /// فصل/قسمت از `VideoEntry` اسکن‌شده، در صورت وجود.
    #[serde(default)]
    pub season: Option<u32>,
    #[serde(default)]
    pub episode: Option<u32>,
}

/// نتیجه نهایی عملیات دانلود — مناسب مصرف در فاز بعدی (Archive Extraction).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadResult {
    /// تعداد ویدیوهای واجد شرایط پس از اعتبارسنجی مجدد.
    pub planned: usize,
    /// تعداد ویدیوهایی که hard link آن‌ها موفق ساخته شد و وارد فاز دانلود شد.
    pub staged: usize,
    /// آیا اجرای SubSource در همه گروه‌ها موفق بود؟
    pub success: bool,
    /// آیا عملیات توسط کاربر لغو شد؟
    pub cancelled: bool,
    /// کد خروج آخرین اجرای SubSource (اگر اجرا رخ داده باشد).
    pub exit_code: Option<i32>,
    /// پیام قابل‌نمایش به کاربر.
    pub message: String,
    /// ریشه‌های staging حفظ‌شده (به ازای هر volume) — خروجی دانلود برای فاز بعد.
    pub staging_paths: Vec<String>,
    /// نگاشت هر ویدیوی staged به مسیر نسبی‌اش در staging — ورودی فاز
    /// Archive Extraction. برای payloadهای قدیمی به‌صورت خالی خوانده می‌شود.
    #[serde(default)]
    pub archive_sources: Vec<ArchiveSource>,
    /// هشدارهای پاک‌سازی (لینک‌های باقیمانده که حذف آن‌ها با خطا مواجه شد).
    pub cleanup_warnings: Vec<String>,
    /// ردشده‌ها (اعتبارسنجی، تکرار، خطای staging) با دلیل.
    pub skipped: Vec<SkippedDownload>,
    pub duration_secs: f64,
}

impl Default for DownloadResult {
    fn default() -> Self {
        Self {
            planned: 0,
            staged: 0,
            success: false,
            cancelled: false,
            exit_code: None,
            message: String::new(),
            staging_paths: Vec::new(),
            archive_sources: Vec::new(),
            cleanup_warnings: Vec::new(),
            skipped: Vec::new(),
            duration_secs: 0.0,
        }
    }
}

/// کلید یکتای مسیر برای تشخیص تکراری — ویندوز به بزرگی/کوچکی حساس نیست.
pub fn path_key(path: &str) -> String {
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path.to_string()
    }
}

/// یک sidecar فارسی‌نامِ نامعتبر — حفظ و گزارش می‌شود، هرگز تغییر نمی‌کند.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarConflict {
    pub path: PathBuf,
    pub reason: String,
}

/// نتیجهٔ بازبینی تازهٔ فایل‌سیستم برای sidecarهای فارسی کنار یک ویدیو.
/// «معتبر» یعنی ساختار زیرنویس معتبر **و** محتوای واقعاً فارسی؛ نام به‌تنهایی
/// هرگز کافی نیست.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SidecarInspection {
    pub valid: Vec<PathBuf>,
    pub conflicts: Vec<SidecarConflict>,
}

/// شکست بازبینی — وقتی وضعیت sidecarها قابل تشخیص نیست، تصمیم دانلود fail-closed است.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SidecarInspectionError {
    /// ویدیو stem یا پوشهٔ والد معتبر ندارد
    InvalidVideoPath { path: String },
    /// خواندن پوشهٔ والد ناموفق بود — نمی‌دانیم چه sidecarهایی وجود دارند
    ParentUnreadable { path: String, reason: String },
}

/// آیا نام این فایل، کاندیدای sidecar فارسی است؟
///
/// نام فقط غربال اولیه است — اعتبار واقعی با `validate_file` بررسی می‌شود.
/// زیرنویس انگلیسیِ بی‌نشانه فارسی نیست و کاندیدا محسوب نمی‌شود.
fn persian_sidecar_name(file_name: &str, sub_stem: &str, video_stem: &str) -> bool {
    if !is_subtitle_file(file_name) {
        return false;
    }
    if !sub_matches_video_stem(sub_stem, video_stem) {
        return false;
    }
    if name_suggests_fa(sub_stem) {
        return true;
    }
    // پسوندهای صریح فارسی: `<stem>.fa.<ext>` و `<stem>.fa.synced.<ext>`
    let lower = sub_stem.to_lowercase();
    lower.ends_with(".fa")
        || lower.ends_with(".fa.synced")
        || lower.contains(".fa.")
        || lower.contains(".fa.synced.")
}

/// بازبینی تازهٔ فایل‌سیستم برای sidecarهای فارسی کنار یک ویدیو.
///
/// - پوشهٔ والد همین الان با `read_dir` خوانده می‌شود — هیچ دادهٔ کش‌شدهٔ اسکن.
/// - نام‌ها با helperهای نام‌گذاری موجود به stem ویدیو سنجیده می‌شوند.
/// - symlink/reparse، پوشه و فایل غیرمعمولی = تعارض (بدون دنبال‌کردن پیوند).
/// - معتبر فقط با `validate_file` + محتوای فارسی.
/// - هیچ فایلی تغییر نمی‌کند؛ خروجی‌ها مرتب‌اند (قطعیت ترتیب).
/// - خواندن نشدن پوشهٔ والد = خطا (fail-closed)، نه «پوشهٔ خالی».
pub fn inspect_persian_sidecars(video: &Path) -> Result<SidecarInspection, SidecarInspectionError> {
    let (Some(stem), Some(parent)) = (video.file_stem(), video.parent()) else {
        return Err(SidecarInspectionError::InvalidVideoPath {
            path: video.to_string_lossy().into_owned(),
        });
    };
    let video_stem = stem.to_string_lossy().into_owned();

    let read = std::fs::read_dir(parent).map_err(|e| SidecarInspectionError::ParentUnreadable {
        path: parent.to_string_lossy().into_owned(),
        reason: e.to_string(),
    })?;

    let mut inspection = SidecarInspection::default();
    for entry in read {
        let entry = entry.map_err(|e| SidecarInspectionError::ParentUnreadable {
            path: parent.to_string_lossy().into_owned(),
            reason: e.to_string(),
        })?;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let sub_stem = Path::new(&file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !persian_sidecar_name(&file_name, &sub_stem, &video_stem) {
            continue;
        }
        let path = entry.path();

        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) => {
                inspection.conflicts.push(SidecarConflict {
                    path: path.clone(),
                    reason: format!("خواندن متادیتای فایل ناموفق بود: {e}"),
                });
                continue;
            }
        };
        if is_link_or_reparse(&meta) {
            inspection.conflicts.push(SidecarConflict {
                path: path.clone(),
                reason: "پیوند نمادین یا Reparse Point است".into(),
            });
            continue;
        }
        if !meta.is_file() {
            inspection.conflicts.push(SidecarConflict {
                path: path.clone(),
                reason: "پوشه است یا فایل معمولی نیست".into(),
            });
            continue;
        }
        let validation = validate_file(&path);
        if !validation.valid {
            inspection.conflicts.push(SidecarConflict {
                path: path.clone(),
                reason: format!(
                    "ساختار زیرنویس نامعتبر است: {}",
                    validation.error.unwrap_or_else(|| "دلیل نامشخص".into())
                ),
            });
            continue;
        }
        if !validation.has_persian {
            inspection.conflicts.push(SidecarConflict {
                path: path.clone(),
                reason: "محتوای فارسی ندارد".into(),
            });
            continue;
        }
        inspection.valid.push(path);
    }

    // ترتیب قطعی برای مصرف‌کننده‌ها
    inspection.valid.sort();
    inspection.conflicts.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(inspection)
}

/// نتیجهٔ typed اعتبارسنجی یک ویدیو برای دانلود — هم در ساخت پلن و هم در اجرا.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoSkipIssue {
    NotInLibrary,
    NotAbsolute,
    MissingFile,
    UnsupportedFormat,
    /// حداقل یک sidecar فارسی معتبر موجود است (همهٔ مسیرهای معتبر حمل می‌شوند)
    ValidSidecarExists {
        valid: Vec<PathBuf>,
    },
    /// sidecarهای فارسی‌نامِ نامعتبر موجودند — تعارض؛ فایل‌ها حفظ می‌شوند
    SidecarConflict {
        conflicts: Vec<SidecarConflict>,
    },
    /// وضعیت sidecarها قابل تشخیص نبود — fail-closed
    SidecarInspectionFailed,
}

impl VideoSkipIssue {
    /// کد پایدار برای `SkippedDownload.code`
    pub fn code(&self) -> SkipReasonCode {
        match self {
            VideoSkipIssue::NotInLibrary => SkipReasonCode::NotInLibrary,
            VideoSkipIssue::NotAbsolute => SkipReasonCode::NotAbsolute,
            VideoSkipIssue::MissingFile => SkipReasonCode::MissingFile,
            VideoSkipIssue::UnsupportedFormat => SkipReasonCode::UnsupportedFormat,
            VideoSkipIssue::ValidSidecarExists { .. } => SkipReasonCode::ValidSidecarExists,
            VideoSkipIssue::SidecarConflict { .. } => SkipReasonCode::SidecarConflict,
            VideoSkipIssue::SidecarInspectionFailed => SkipReasonCode::SidecarInspectionFailed,
        }
    }

    /// پیام انسانی پایدار (سازگار با رویدادها/UI فعلی)
    pub fn message(&self) -> String {
        match self {
            VideoSkipIssue::NotInLibrary => "در کتابخانه اسکن‌شده نیست — ابتدا اسکن کنید".into(),
            VideoSkipIssue::NotAbsolute => "مسیر مطلق نیست".into(),
            VideoSkipIssue::MissingFile => "فایل ویدیو روی دیسک پیدا نشد".into(),
            VideoSkipIssue::UnsupportedFormat => "فرمت ویدیویی پشتیبانی نمی‌شود".into(),
            VideoSkipIssue::ValidSidecarExists { valid } => format!(
                "زیرنویس فارسی معتبر کنار ویدیو موجود است — بازنویسی نمی‌شود ({})",
                valid.len()
            ),
            VideoSkipIssue::SidecarConflict { conflicts } => format!(
                "فایل کناری نامعتبر موجود است — تعارض؛ فایل حفظ می‌شود ({})",
                conflicts.len()
            ),
            VideoSkipIssue::SidecarInspectionFailed => {
                "بررسی sidecarهای کناری ناموفق بود — برای اطمینان، این ویدیو دانلود نمی‌شود".into()
            }
        }
    }
}

/// اعتبارسنجی typed یک ویدیو برای دانلود — هم در ساخت پلن و هم در لحظهٔ اجرا
/// (بازبینی sidecarها همیشه از فایل‌سیستم تازه انجام می‌شود، نه از دادهٔ اسکن).
///
/// `entry` نبودن یعنی ویدیو در کتابخانه اسکن‌شده نیست.
pub fn validate_video_typed(path: &str, entry: Option<&VideoEntry>) -> Result<(), VideoSkipIssue> {
    let Some(entry) = entry else {
        return Err(VideoSkipIssue::NotInLibrary);
    };

    let video = PathBuf::from(path);
    if !video.is_absolute() {
        return Err(VideoSkipIssue::NotAbsolute);
    }
    if !video.is_file() {
        return Err(VideoSkipIssue::MissingFile);
    }
    if !is_video_file(&entry.file_name) {
        return Err(VideoSkipIssue::UnsupportedFormat);
    }
    match inspect_persian_sidecars(&video) {
        Ok(inspection) => {
            if !inspection.valid.is_empty() {
                return Err(VideoSkipIssue::ValidSidecarExists {
                    valid: inspection.valid,
                });
            }
            if !inspection.conflicts.is_empty() {
                return Err(VideoSkipIssue::SidecarConflict {
                    conflicts: inspection.conflicts,
                });
            }
            Ok(())
        }
        Err(_) => Err(VideoSkipIssue::SidecarInspectionFailed),
    }
}

/// wrapper سازگاری با خروجی رشته‌ای برای callerهای فعلی — pipeline از این
/// نسخه استفاده نمی‌کند (فقط typed).
pub fn validate_video(path: &str, entry: Option<&VideoEntry>) -> Result<(), String> {
    validate_video_typed(path, entry).map_err(|issue| issue.message())
}

/// ساخت پلن دانلود از فهرست انتخابی کاربر در برابر کتابخانه — بدون تغییر دیسک.
pub fn create_download_plan(library: &[VideoEntry], requested: &[String]) -> DownloadPlan {
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for path in requested {
        let file_name = Path::new(path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        let mut skip = |issue: VideoSkipIssue| {
            let mut skipped_item = SkippedDownload::with_code(
                path.clone(),
                file_name.clone(),
                issue.code(),
                issue.message(),
            );
            match &issue {
                VideoSkipIssue::ValidSidecarExists { valid } => {
                    skipped_item.valid_sidecar_paths = valid
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                }
                VideoSkipIssue::SidecarConflict { conflicts } => {
                    skipped_item.sidecar_conflicts = conflicts.clone();
                }
                _ => {}
            }
            skipped.push(skipped_item);
        };

        let entry = library.iter().find(|e| &e.id == path || &e.path == path);
        // مسیر تکراری فقط وقتی «تکراری» است که قبلاً پذیرفته شده باشد؛
        // نسخه‌های تکراریِ یک مسیر نامعتبر دلیل واقعی رد شدن را می‌گیرند.
        if entry.is_some() && !seen.insert(path_key(path)) {
            skipped.push(SkippedDownload::with_code(
                path.clone(),
                file_name.clone(),
                SkipReasonCode::DuplicatePath,
                "مسیر تکراری است",
            ));
            continue;
        }
        match validate_video_typed(path, entry) {
            Ok(()) => items.push(DownloadPlanItem {
                video_path: entry.expect("entry بررسی شد").path.clone(),
                video_file_name: entry.expect("entry بررسی شد").file_name.clone(),
            }),
            Err(issue) => skip(issue),
        }
    }

    DownloadPlan { items, skipped }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> VideoEntry {
        let video = PathBuf::from(path);
        VideoEntry {
            id: path.to_string(),
            path: path.to_string(),
            file_name: video
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default(),
            size_bytes: 0,
            modified_ms: None,
            parent_dir: video
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            show_name: None,
            season: None,
            episode: None,
            sibling_subtitles: Vec::new(),
            analysis: None,
        }
    }

    fn unique_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "subsync-plan-test-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn validate_rejects_missing_file_and_non_video() {
        let dir = unique_dir("missing");
        let missing = entry(dir.join("Show.S01E01.mkv").to_string_lossy().as_ref());
        let err = validate_video(&missing.path, Some(&missing)).unwrap_err();
        assert!(err.contains("پیدا نشد"), "{err}");

        // فایل موجود با پسوند غیر ویدیویی
        let txt = dir.join("notes.txt");
        std::fs::write(&txt, b"x").unwrap();
        let e = entry(txt.to_string_lossy().as_ref());
        let err = validate_video(&e.path, Some(&e)).unwrap_err();
        assert!(err.contains("فرمت"), "{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_rejects_relative_paths_and_missing_entry() {
        let err = validate_video("relative/path.mkv", None).unwrap_err();
        assert!(err.contains("کتابخانه"), "{err}");

        let e = entry("relative/path.mkv");
        let err = validate_video(&e.path, Some(&e)).unwrap_err();
        assert!(err.contains("مطلق"), "{err}");
    }

    /// fixture زیرنویس فارسی معتبر (cue + متن فارسی)
    const FA_SRT: &str = "1\r\n00:00:01,000 --> 00:00:02,000\r\nسلام\r\n";
    /// ASS معتبر با متن فارسی — برای کاندیدای fa.ass
    const FA_ASS: &str = "[Script Info]\r\nTitle: t\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,سلام\r\n";

    #[test]
    fn validate_valid_fa_sidecar_blocks_and_invalid_becomes_conflict() {
        let dir = unique_dir("fa-sidecar");
        let video = dir.join("Show.S01E01.mkv");
        std::fs::write(&video, b"video").unwrap();
        let fa = dir.join("Show.S01E01.fa.srt");
        std::fs::write(&fa, FA_SRT).unwrap();

        let e = entry(video.to_string_lossy().as_ref());
        let issue = validate_video_typed(&e.path, Some(&e)).unwrap_err();
        // فقط زیرنویس واقعاً معتبرِ فارسی مانع دانلود است
        assert_eq!(issue.code(), SkipReasonCode::ValidSidecarExists);
        assert!(
            issue.message().contains("زیرنویس فارسی"),
            "{}",
            issue.message()
        );
        // فایل زیرنویس موجود دست‌نخورده
        assert_eq!(std::fs::read(&fa).unwrap(), FA_SRT.as_bytes());

        // فرمت synced هم با محتوای فارسی پوشش داده می‌شود
        std::fs::write(dir.join("Show.S01E02.mkv"), b"video").unwrap();
        std::fs::write(dir.join("Show.S01E02.fa.synced.srt"), FA_SRT).unwrap();
        let e2 = entry(dir.join("Show.S01E02.mkv").to_string_lossy().as_ref());
        assert_eq!(
            validate_video_typed(&e2.path, Some(&e2))
                .unwrap_err()
                .code(),
            SkipReasonCode::ValidSidecarExists
        );

        // زیرنویس غیر فارسی (انگلیسی) کاندیدای فارسی نیست و مانع دانلود نیست
        std::fs::write(dir.join("Show.S01E03.mkv"), b"video").unwrap();
        std::fs::write(dir.join("Show.S01E03.english.srt"), b"hello").unwrap();
        let e3 = entry(dir.join("Show.S01E03.mkv").to_string_lossy().as_ref());
        assert!(validate_video_typed(&e3.path, Some(&e3)).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_persian_named_sidecar_becomes_conflict_not_valid() {
        let dir = unique_dir("fa-malformed");
        let video = dir.join("Show.S01E01.mkv");
        std::fs::write(&video, b"video").unwrap();
        // فارسی‌نام ولی بدون ساختار معتبر
        let fa = dir.join("Show.S01E01.fa.srt");
        std::fs::write(&fa, b"EXISTING_PERSIAN_SUBTITLE").unwrap();

        let e = entry(video.to_string_lossy().as_ref());
        let issue = validate_video_typed(&e.path, Some(&e)).unwrap_err();
        assert_eq!(issue.code(), SkipReasonCode::SidecarConflict);
        assert!(issue.message().contains("تعارض"), "{}", issue.message());
        // فایل حفظ شده و دست‌نخورده
        assert_eq!(std::fs::read(&fa).unwrap(), b"EXISTING_PERSIAN_SUBTITLE");

        // ساختاری معتبر ولی بدون محتوای فارسی → باز هم تعارض، نه valid
        let fa2 = dir.join("Show.S01E02.fa.srt");
        std::fs::write(dir.join("Show.S01E02.mkv"), b"video").unwrap();
        std::fs::write(
            &fa2,
            "1\r\n00:00:01,000 --> 00:00:02,000\r\nenglish only\r\n",
        )
        .unwrap();
        let e2 = entry(dir.join("Show.S01E02.mkv").to_string_lossy().as_ref());
        let issue2 = validate_video_typed(&e2.path, Some(&e2)).unwrap_err();
        assert_eq!(issue2.code(), SkipReasonCode::SidecarConflict);
        if let VideoSkipIssue::SidecarConflict { conflicts } = &issue2 {
            assert_eq!(conflicts.len(), 1);
            assert!(conflicts[0].reason.contains("فارسی ندارد"));
        } else {
            panic!("باید SidecarConflict باشد");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // inspect_persian_sidecars — بازبینی تازهٔ فایل‌سیستم
    // ------------------------------------------------------------------

    #[test]
    fn inspector_detects_persian_variants_and_ignores_english() {
        let dir = unique_dir("inspector");
        let video = dir.join("Show.S01E01.mkv");
        std::fs::write(&video, b"video").unwrap();
        std::fs::write(dir.join("Show.S01E01.fa.srt"), FA_SRT).unwrap();
        // فارسی در فرمت ASS معتبر
        std::fs::write(
            dir.join("Show.S01E01.fa.ass"),
            "[Script Info]\r\nTitle: t\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,سلام\r\n",
        )
        .unwrap();
        // انگلیسیِ منطبق: نه کاندیداست نه تعارض
        std::fs::write(dir.join("Show.S01E01.english.srt"), b"hello").unwrap();
        // بی‌نشانه: کاندیدای فارسی نیست (نام نشانه ندارد)
        std::fs::write(dir.join("Show.S01E01.srt"), FA_SRT).unwrap();

        let inspection = inspect_persian_sidecars(&video).unwrap();
        assert_eq!(inspection.valid.len(), 2, "{:?}", inspection);
        assert!(inspection.conflicts.is_empty(), "{:?}", inspection);
        // هیچ انتخابی انجام نشده — همهٔ validها برگشته‌اند
        assert!(inspection.valid.contains(&dir.join("Show.S01E01.fa.srt")));
        assert!(inspection.valid.contains(&dir.join("Show.S01E01.fa.ass")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspector_returns_all_valid_and_all_conflicts_sorted() {
        let dir = unique_dir("inspector-mixed");
        let video = dir.join("Show.S01E01.mkv");
        std::fs::write(&video, b"video").unwrap();
        // دو معتبر — هر فایل با محتوای معتبرِ فرمت خودش
        std::fs::write(dir.join("Show.S01E01.fa.srt"), FA_SRT).unwrap();
        std::fs::write(dir.join("Show.S01E01.fa.ass"), FA_ASS).unwrap();
        // دو نامعتبر: خرابِ ساختاری + ساختار معتبرِ لاتینِ بی‌فارسی
        std::fs::write(dir.join("Show.S01E01.fa.synced.srt"), b"garbage").unwrap();
        std::fs::write(
            dir.join("Show.S01E01.fa.vtt"),
            b"WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nenglish only\n",
        )
        .unwrap();

        let inspection = inspect_persian_sidecars(&video).unwrap();
        assert_eq!(inspection.valid.len(), 2);
        assert_eq!(inspection.conflicts.len(), 2);
        // هیچ انتخابی نشده؛ ترتیب قطعی
        let sorted = {
            let mut v = inspection.valid.clone();
            v.sort();
            v == inspection.valid
        };
        assert!(sorted);
        assert!(inspection
            .conflicts
            .iter()
            .any(|c| c.reason.contains("ساختار")));
        assert!(inspection
            .conflicts
            .iter()
            .any(|c| c.reason.contains("فارسی ندارد")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspection_fails_closed_when_parent_unreadable() {
        // پوشهٔ والد وجود ندارد — read_dir شکست می‌خورد؛ «خالی» تفسیر نمی‌شود
        let missing = unique_dir("inspector-missing-root");
        let video = missing.join("nowhere").join("Show.S01E01.mkv");
        let err = inspect_persian_sidecars(&video).unwrap_err();
        assert!(matches!(
            err,
            SidecarInspectionError::ParentUnreadable { .. }
        ));
        // ویدیوی بدون stem معتبر
        let err = inspect_persian_sidecars(Path::new("")).unwrap_err();
        assert!(matches!(
            err,
            SidecarInspectionError::InvalidVideoPath { .. }
        ));
        let _ = std::fs::remove_dir_all(&missing);
    }

    #[test]
    fn inspection_detects_sidecars_created_after_previous_check() {
        // freshness: دادهٔ اسکن قبلی ملاک نیست — فقط فایل‌سیستم فعلی
        let dir = unique_dir("inspector-fresh");
        let video = dir.join("Show.S01E01.mkv");
        std::fs::write(&video, b"video").unwrap();

        // قبل از ساخت sidecar — هیچ کاندیدایی
        let before = inspect_persian_sidecars(&video).unwrap();
        assert!(before.valid.is_empty() && before.conflicts.is_empty());

        // بعد از ساخت sidecar — همان فراخوانی بدون هیچ اسکن مجدد آن را می‌بیند
        std::fs::write(dir.join("Show.S01E01.fa.srt"), FA_SRT).unwrap();
        let after = inspect_persian_sidecars(&video).unwrap();
        assert_eq!(after.valid.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspection_never_modifies_candidates() {
        let dir = unique_dir("inspector-readonly");
        let video = dir.join("Show.S01E01.mkv");
        std::fs::write(&video, b"video").unwrap();
        let fa = dir.join("Show.S01E01.fa.srt");
        std::fs::write(&fa, b"BYTES-1").unwrap();
        let bad = dir.join("Show.S01E01.fa.synced.srt");
        std::fs::write(&bad, b"BYTES-2").unwrap();

        let _ = inspect_persian_sidecars(&video).unwrap();

        // هر دو کاندیدا بایت‌به‌بایت دست‌نخورده
        assert_eq!(std::fs::read(&fa).unwrap(), b"BYTES-1");
        assert_eq!(std::fs::read(&bad).unwrap(), b"BYTES-2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn inspector_junction_sidecar_is_conflict_without_following() {
        let dir = unique_dir("inspector-junction");
        let video = dir.join("Show.S01E01.mkv");
        std::fs::write(&video, b"video").unwrap();
        // کاندیدای واقعی فارسی در مسیر دیگر
        let real_dir = dir.join("real");
        std::fs::create_dir_all(&real_dir).unwrap();
        let real_sidecar = real_dir.join("Show.S01E01.fa.srt");
        std::fs::write(&real_sidecar, FA_SRT).unwrap();

        // Junction با نامِ کاندیدای فارسی — نباید دنبال شود
        let junction = dir.join("Show.S01E01.fa.srt");
        let out = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&real_sidecar)
            .output()
            .expect("اجرای mklink");
        if !out.status.success() {
            eprintln!("mklink /J در این محیط پشتیبانی نشد");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }

        let inspection = inspect_persian_sidecars(&video).unwrap();
        assert!(inspection.valid.is_empty(), "{:?}", inspection);
        assert_eq!(inspection.conflicts.len(), 1);
        assert!(inspection.conflicts[0].reason.contains("پیوند"));
        // هدف junction دست‌نخورده
        assert_eq!(std::fs::read(&real_sidecar).unwrap(), FA_SRT.as_bytes());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------------
    // SkipReasonCode — سازگاری سریال‌سازی
    // ------------------------------------------------------------------

    #[test]
    fn skip_reason_code_roundtrips_all_variants() {
        let variants = [
            (SkipReasonCode::Unknown, "unknown"),
            (SkipReasonCode::NotInLibrary, "notInLibrary"),
            (SkipReasonCode::NotAbsolute, "notAbsolute"),
            (SkipReasonCode::MissingFile, "missingFile"),
            (SkipReasonCode::UnsupportedFormat, "unsupportedFormat"),
            (SkipReasonCode::ValidSidecarExists, "validSidecarExists"),
            (SkipReasonCode::SidecarConflict, "sidecarConflict"),
            (SkipReasonCode::DuplicatePath, "duplicatePath"),
            (SkipReasonCode::MissingAtExecution, "missingAtExecution"),
            (
                SkipReasonCode::SidecarAppearedAtExecution,
                "sidecarAppearedAtExecution",
            ),
            (
                SkipReasonCode::SidecarInspectionFailed,
                "sidecarInspectionFailed",
            ),
            (
                SkipReasonCode::StagingVolumeUnavailable,
                "stagingVolumeUnavailable",
            ),
            (SkipReasonCode::StagingPathCollision, "stagingPathCollision"),
            (SkipReasonCode::StagingBuildFailed, "stagingBuildFailed"),
        ];
        for (code, wire) in variants {
            let json = serde_json::to_string(&code).unwrap();
            assert_eq!(json, format!("\"{wire}\""));
            let back: SkipReasonCode = serde_json::from_str(&json).unwrap();
            assert_eq!(back, code);
        }
    }

    #[test]
    fn legacy_skipped_payload_without_code_deserializes_unknown() {
        let json = r#"{
            "videoPath": "C:\\x\\Show.S01E01.mkv",
            "videoFileName": "Show.S01E01.mkv",
            "reason": "دلیل قدیمی"
        }"#;
        let skipped: SkippedDownload = serde_json::from_str(json).unwrap();
        assert_eq!(skipped.code, SkipReasonCode::Unknown);
        assert!(skipped.valid_sidecar_paths.is_empty());
        assert!(skipped.sidecar_conflicts.is_empty());
    }

    #[test]
    fn unknown_future_code_string_deserializes_unknown() {
        let json = r#"{
            "videoPath": "C:\\x\\a.mkv",
            "videoFileName": "a.mkv",
            "reason": "x",
            "code": "someFutureCode"
        }"#;
        let skipped: SkippedDownload = serde_json::from_str(json).unwrap();
        assert_eq!(skipped.code, SkipReasonCode::Unknown);
    }

    #[test]
    fn skipped_download_context_fields_roundtrip_and_camel_case() {
        let skipped = SkippedDownload {
            video_path: "C:\\x\\a.mkv".into(),
            video_file_name: "a.mkv".into(),
            reason: "زیرنویس فارسی معتبر کنار ویدیو موجود است — بازنویسی نمی‌شود (1)".into(),
            code: SkipReasonCode::ValidSidecarExists,
            valid_sidecar_paths: vec!["C:\\x\\a.fa.srt".into()],
            sidecar_conflicts: Vec::new(),
        };
        let json = serde_json::to_string(&skipped).unwrap();
        assert!(json.contains("\"code\":\"validSidecarExists\""));
        assert!(json.contains("\"validSidecarPaths\""));
        assert!(!json.contains("\"sidecarConflicts\"")); // خالی → نمی‌آید
        let back: SkippedDownload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, skipped);
    }

    #[test]
    fn create_plan_rejects_duplicates_and_out_of_library() {
        let dir = unique_dir("plan-dup");
        let video = dir.join("Show.S01E01.mkv");
        std::fs::write(&video, b"v").unwrap();
        let e = entry(video.to_string_lossy().as_ref());
        let path_str = e.path.clone();
        let library = vec![e];

        // مسیر تکراری + مسیر خارج از کتابخانه
        let plan = create_download_plan(
            &library,
            &[
                path_str.clone(),
                path_str.clone(),
                dir.join("Other.mkv").to_string_lossy().to_string(),
            ],
        );
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].video_path, path_str);
        assert_eq!(plan.skipped.len(), 2);
        assert!(plan.skipped.iter().any(|s| s.reason.contains("تکراری")));
        assert!(plan.skipped.iter().any(|s| s.reason.contains("کتابخانه")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_plan_does_not_modify_disk() {
        // پلن فقط می‌خواند — هیچ مسیر جدیدی نباید ساخته شود
        let dir = unique_dir("plan-readonly");
        let before = std::fs::read_dir(&dir).unwrap().count();
        let _ = create_download_plan(&[], &[]);
        let after = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(before, after);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn path_key_is_case_insensitive_on_windows() {
        assert_eq!(path_key("C:\\A\\X.mkv"), path_key("c:\\a\\x.mkv"));
    }

    #[test]
    fn download_result_without_archive_sources_still_deserializes() {
        // payload قدیمی (قبل از PR آرشیو) نباید خطا بدهد
        let json = r#"{
            "planned": 1,
            "staged": 1,
            "success": true,
            "cancelled": false,
            "exitCode": 0,
            "message": "ok",
            "stagingPaths": ["C:\\staging\\_subsource_batch_x"],
            "cleanupWarnings": [],
            "skipped": [],
            "durationSecs": 1.5
        }"#;
        let result: DownloadResult = serde_json::from_str(json).unwrap();
        assert!(result.archive_sources.is_empty());
        assert_eq!(result.staging_paths.len(), 1);
    }

    #[test]
    fn archive_source_roundtrips_without_secrets() {
        let source = ArchiveSource {
            video_path: "C:\\lib\\Show\\S01\\Show.S01E01.mkv".into(),
            video_file_name: "Show.S01E01.mkv".into(),
            staging_root: "C:\\_subsource_batch_abc".into(),
            relative_video_path: "S01\\Show.S01E01.mkv".into(),
            relative_video_dir: "S01".into(),
            season: Some(1),
            episode: Some(1),
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(!json.to_lowercase().contains("apikey"));
        assert!(json.contains("relativeVideoDir"));
        let back: ArchiveSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, source);
    }
}
