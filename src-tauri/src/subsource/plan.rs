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
//! - اگر زیرنویس فارسی نهایی (مثل `<video>.fa.srt`) کنار ویدیو باشد، ویدیو
//!   رد می‌شود — زیرنویس موجود هرگز بازنویسی نمی‌شود.
//! - مسیرهای تکراری فقط یک‌بار پذیرفته می‌شوند.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::models::VideoEntry;
use crate::scanner::naming::is_video_file;

/// یک ویدیوی واجد شرایط دانلود — در پلن تأییدشده کاربر.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadPlanItem {
    pub video_path: String,
    pub video_file_name: String,
}

/// ویدیوی ردشده با دلیل — هم در ساخت پلن و هم در اعتبارسنجی مجدد اجرا.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkippedDownload {
    pub video_path: String,
    pub video_file_name: String,
    pub reason: String,
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

/// آیا زیرنویس فارسی نهایی کنار ویدیو موجود است؟
///
/// دو منبع بررسی می‌شود: فهرست زیرنویس‌های کناری ثبت‌شده در اسکن (با نشانهٔ
/// فارسی در نام) و بررسی مستقیم دیسک برای `<video>.fa.srt` و
/// `<video>.fa.synced.srt` (مرجع نهایی در لحظهٔ اجرا).
pub fn has_final_persian_sidecar(video: &Path, entry: Option<&VideoEntry>) -> bool {
    if let Some(e) = entry {
        if e.sibling_subtitles.iter().any(|s| s.name_suggests_fa) {
            return true;
        }
    }
    let (Some(stem), Some(parent)) = (video.file_stem(), video.parent()) else {
        return false;
    };
    for suffix in ["fa.srt", "fa.synced.srt"] {
        if parent
            .join(format!("{}.{}", stem.to_string_lossy(), suffix))
            .is_file()
        {
            return true;
        }
    }
    false
}

/// اعتبارسنجی کامل یک ویدیو برای دانلود — هم در ساخت پلن و هم در لحظهٔ اجرا.
///
/// `entry` نبودن یعنی ویدیو در کتابخانه اسکن‌شده نیست.
pub fn validate_video(path: &str, entry: Option<&VideoEntry>) -> Result<(), String> {
    let Some(entry) = entry else {
        return Err("در کتابخانه اسکن‌شده نیست — ابتدا اسکن کنید".into());
    };

    let video = PathBuf::from(path);
    if !video.is_absolute() {
        return Err("مسیر مطلق نیست".into());
    }
    if !video.is_file() {
        return Err("فایل ویدیو روی دیسک پیدا نشد".into());
    }
    if !is_video_file(&entry.file_name) {
        return Err("فرمت ویدیویی پشتیبانی نمی‌شود".into());
    }
    if has_final_persian_sidecar(&video, Some(entry)) {
        return Err("زیرنویس فارسی کنار ویدیو موجود است — بازنویسی نمی‌شود".into());
    }
    Ok(())
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
        let mut skip = |reason: String| {
            skipped.push(SkippedDownload {
                video_path: path.clone(),
                video_file_name: file_name.clone(),
                reason,
            })
        };

        let entry = library.iter().find(|e| &e.id == path || &e.path == path);
        // مسیر تکراری فقط وقتی «تکراری» است که قبلاً پذیرفته شده باشد؛
        // نسخه‌های تکراریِ یک مسیر نامعتبر دلیل واقعی رد شدن را می‌گیرند.
        if entry.is_some() && !seen.insert(path_key(path)) {
            skip("مسیر تکراری است".into());
            continue;
        }
        match validate_video(path, entry) {
            Ok(()) => items.push(DownloadPlanItem {
                video_path: entry.expect("entry بررسی شد").path.clone(),
                video_file_name: entry.expect("entry بررسی شد").file_name.clone(),
            }),
            Err(reason) => skip(reason),
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

    #[test]
    fn validate_rejects_existing_fa_sidecar_and_never_touches_it() {
        let dir = unique_dir("fa-sidecar");
        let video = dir.join("Show.S01E01.mkv");
        std::fs::write(&video, b"video").unwrap();
        let fa = dir.join("Show.S01E01.fa.srt");
        std::fs::write(&fa, "زیرنویس موجود").unwrap();

        let e = entry(video.to_string_lossy().as_ref());
        let err = validate_video(&e.path, Some(&e)).unwrap_err();
        assert!(err.contains("زیرنویس فارسی"), "{err}");
        // فایل زیرنویس موجود دست‌نخورده
        assert_eq!(std::fs::read_to_string(&fa).unwrap(), "زیرنویس موجود");

        // فرمت synced هم پوشش داده می‌شود
        let fa_synced = dir.join("Show.S01E02.fa.synced.srt");
        std::fs::write(dir.join("Show.S01E02.mkv"), b"video").unwrap();
        std::fs::write(&fa_synced, "synced").unwrap();
        let e2 = entry(dir.join("Show.S01E02.mkv").to_string_lossy().as_ref());
        assert!(validate_video(&e2.path, Some(&e2)).is_err());

        // زیرنویس غیر فارسی (انگلیسی) مانع دانلود نیست
        std::fs::write(dir.join("Show.S01E03.mkv"), b"video").unwrap();
        std::fs::write(dir.join("Show.S01E03.english.srt"), b"hello").unwrap();
        let e3 = entry(dir.join("Show.S01E03.mkv").to_string_lossy().as_ref());
        assert!(validate_video(&e3.path, Some(&e3)).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
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
