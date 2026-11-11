//! رجیستری Provenance سمت سرور — تنها مرجع معتبر برای نگاشت
//! «ویدیوی اسکن‌شده → خروجی staging ساخته‌شده توسط خود برنامه».
//!
//! فرانت فقط `ArchiveSource` را برمی‌گرداند؛ این ساختار به‌تنهایی قابل جعل
//! است. بنابراین هنگام ساخت staging، یک رکورد در همین رجیستری ثبت می‌شود و
//! هر دو دستور پلن/اجرا باید رکورد دقیقاً منطبق را پیدا کنند. علاوه بر آن:
//!
//! - ویدیو باید در کتابخانهٔ اسکن‌شدهٔ فعلی باشد (`AppState.library`).
//! - نام/مسیر و پسوند ویدیو از `VideoEntry` معتبر گرفته می‌شود، نه از فرانت.
//! - `season`/`episode` ارسالی فرانت نادیده گرفته و از `VideoEntry` جایگزین
//!   می‌شود.
//!
//! رجیستری در حافظه است و فقط رکوردهایی را نگه می‌دارد که خود بک‌اند هنگام
//! staging ساخته است. حذف/بازنویسی رکورد هرگز به حذف آرشیوهای staging منجر
//! نمی‌شود؛ ثبت مجدد فقط رکورد همان مسیر را جایگزین می‌کند.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::models::VideoEntry;
use crate::scanner::naming::is_video_file;
use crate::subsource::plan::{path_key, ArchiveSource};
use crate::subsource::staging::STAGING_DIR_PREFIX;

use super::listing::{is_link_or_reparse, validate_member_path};

fn normalize_sep(value: &str) -> String {
    value.replace('\\', "/")
}

fn same_path_text(a: &str, b: &str) -> bool {
    let a = normalize_sep(a);
    let b = normalize_sep(b);
    if cfg!(windows) {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

/// یک رکورد provenance که فقط در بک‌اند و هنگام ساخت staging ایجاد می‌شود.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceRecord {
    pub video_path: String,
    pub video_file_name: String,
    pub staging_root: String,
    pub relative_video_path: String,
    pub relative_video_dir: String,
}

/// رجیستری درون‌حافظه‌ای؛ کلید = مسیر نرمال‌شدهٔ ویدیوی اصلی.
/// برای هر ویدیو ممکن است چند staging معتبر (دانلودهای قبلی) حفظ شود.
#[derive(Debug, Default)]
pub struct ProvenanceRegistry {
    records: HashMap<String, Vec<ProvenanceRecord>>,
}

impl ProvenanceRegistry {
    /// ثبت یک stagingِ تازه‌ساخته. خروجی، `ArchiveSource` معتبر و
    /// authoritative برای قرار گرفتن در `DownloadResult.archive_sources` است.
    pub fn register(
        &mut self,
        entry: &VideoEntry,
        staging_root: &Path,
        relative_video_path: &str,
        relative_video_dir: &str,
    ) -> Result<ArchiveSource, String> {
        if entry.path.trim().is_empty() {
            return Err("مسیر ویدیوی کتابخانه خالی است — provenance ثبت نشد".into());
        }
        if !is_video_file(&entry.file_name) {
            return Err(format!(
                "پسوند ویدیوی «{}» پشتیبانی نمی‌شود — provenance ثبت نشد",
                entry.file_name
            ));
        }
        let actual_name = Path::new(&entry.path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if actual_name.is_empty() || !same_path_text(&actual_name, &entry.file_name) {
            return Err(format!(
                "نام فایل «{}» با مسیر ویدیوی «{}» هم‌خوان نیست",
                entry.file_name, entry.path
            ));
        }

        let staging_root_text = staging_root.to_string_lossy().to_string();
        validate_staging_root_text(&staging_root_text)?;

        let safe_rel = validate_member_path(&normalize_sep(relative_video_path))?;
        let rel_dir = if relative_video_dir.is_empty() {
            String::new()
        } else {
            validate_member_path(&normalize_sep(relative_video_dir))?
        };
        let expected_rel = if rel_dir.is_empty() {
            entry.file_name.clone()
        } else {
            format!("{rel_dir}/{}", entry.file_name)
        };
        if !same_path_text(
            &normalize_sep(relative_video_path),
            &normalize_sep(&expected_rel),
        ) {
            return Err(format!(
                "مسیر نسبی staging «{relative_video_path}» با پوشه/نام فایل هم‌خوان نیست"
            ));
        }
        let _ = safe_rel; // فقط برای اعتبارسنجی مسیر استفاده شد

        let record = ProvenanceRecord {
            video_path: entry.path.clone(),
            video_file_name: entry.file_name.clone(),
            staging_root: staging_root_text.clone(),
            relative_video_path: relative_video_path.to_string(),
            relative_video_dir: relative_video_dir.to_string(),
        };
        let key = path_key(&entry.path);
        let records = self.records.entry(key).or_default();
        // ثبت مجدد همان staging/مسیر فقط رکورد را جایگزین می‌کند؛ هیچ فایلی
        // روی دیسک حذف نمی‌شود و رکوردهای stagingهای قبلی حفظ می‌شوند.
        records.retain(|r| {
            !(same_path_text(&r.staging_root, &staging_root_text)
                && same_path_text(&r.relative_video_path, relative_video_path))
        });
        records.push(record);

        Ok(source_from(entry, &records_last(records)))
    }

    /// اعتبارسنجی یک منبع ارسالی و تبدیل آن به منبع authoritative.
    /// هرگونه ناسازگاری → خطا (fail-closed).
    pub fn validate(
        &self,
        submitted: &ArchiveSource,
        library: &[VideoEntry],
    ) -> Result<ArchiveSource, String> {
        let entry = library
            .iter()
            .find(|e| same_path_text(&e.path, &submitted.video_path))
            .ok_or_else(|| {
                format!(
                    "«{}» در کتابخانهٔ اسکن‌شده نیست — ابتدا اسکن کنید",
                    submitted.video_file_name
                )
            })?;

        if !is_video_file(&entry.file_name) {
            return Err(format!(
                "پسوند ویدیوی «{}» پشتیبانی نمی‌شود — منبع رد شد",
                entry.file_name
            ));
        }
        if !same_path_text(&entry.file_name, &submitted.video_file_name) {
            return Err(format!(
                "نام فایل «{}» با رکورد کتابخانه «{}» هم‌خوان نیست — منبع رد شد",
                submitted.video_file_name, entry.file_name
            ));
        }

        let key = path_key(&entry.path);
        let record = self
            .records
            .get(&key)
            .and_then(|records| {
                records.iter().find(|r| {
                    same_path_text(&r.video_path, &submitted.video_path)
                        && same_path_text(&r.video_file_name, &submitted.video_file_name)
                        && same_path_text(&r.staging_root, &submitted.staging_root)
                        && same_path_text(&r.relative_video_path, &submitted.relative_video_path)
                        && same_path_text(&r.relative_video_dir, &submitted.relative_video_dir)
                })
            })
            .ok_or_else(|| {
                format!(
                    "برای «{}» هیچ رکورد staging مورد اعتمادی ثبت نشده است — منبع رد شد",
                    submitted.video_file_name
                )
            })?;

        // ریشهٔ staging باید همچنان یک پوشهٔ واقعی و متعلق به برنامه باشد.
        validate_staging_root_text(&record.staging_root)?;

        // season/episode هرگز از فرانت پذیرفته نمی‌شود.
        Ok(source_from(entry, record))
    }
}

fn records_last(records: &[ProvenanceRecord]) -> ProvenanceRecord {
    records
        .last()
        .cloned()
        .expect("رجیستری تازه مقداردهی شده است")
}

fn source_from(entry: &VideoEntry, record: &ProvenanceRecord) -> ArchiveSource {
    ArchiveSource {
        video_path: entry.path.clone(),
        video_file_name: entry.file_name.clone(),
        staging_root: record.staging_root.clone(),
        relative_video_path: record.relative_video_path.clone(),
        relative_video_dir: record.relative_video_dir.clone(),
        season: entry.season,
        episode: entry.episode,
    }
}

/// اعتبارسنجی دسته‌ای منابع ورودی دستورات + رد تکراری‌ها.
pub fn validate_archive_sources(
    registry: &ProvenanceRegistry,
    library: &[VideoEntry],
    submitted: &[ArchiveSource],
) -> Result<Vec<ArchiveSource>, String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut trusted = Vec::with_capacity(submitted.len());
    for source in submitted {
        let canonical = registry.validate(source, library)?;
        if !seen.insert(path_key(&canonical.video_path)) {
            return Err(format!(
                "منبع تکراری برای «{}» رد شد",
                canonical.video_file_name
            ));
        }
        trusted.push(canonical);
    }
    Ok(trusted)
}

/// پیشوند `_subsource_batch_` به‌تنهایی مدرک نیست؛ این تابع فقط شرایط
/// فیزیکی ریشه را بررسی می‌کند. مدرک اصلی، وجود رکورد رجیستری است.
fn validate_staging_root_text(staging_root: &str) -> Result<(), String> {
    let path = Path::new(staging_root);
    if !path.is_absolute() {
        return Err(format!("ریشهٔ staging «{staging_root}» مطلق نیست"));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if !name.starts_with(STAGING_DIR_PREFIX) {
        return Err(format!(
            "پوشهٔ «{staging_root}» ریشهٔ staging معتبر برنامه نیست"
        ));
    }
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("ریشهٔ staging «{staging_root}» قابل دسترسی نیست: {e}"))?;
    if is_link_or_reparse(&meta) {
        return Err(format!(
            "ریشهٔ staging «{staging_root}» symlink/reparse است — رد شد"
        ));
    }
    if !meta.is_dir() {
        return Err(format!("ریشهٔ staging «{staging_root}» پوشه نیست"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::VideoEntry;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "subsync-provenance-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(video: &Path, season: Option<u32>, episode: Option<u32>) -> VideoEntry {
        VideoEntry {
            id: video.to_string_lossy().to_string(),
            path: video.to_string_lossy().to_string(),
            file_name: video
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default(),
            size_bytes: 1,
            modified_ms: None,
            parent_dir: video
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            show_name: Some("Show".into()),
            season,
            episode,
            sibling_subtitles: Vec::new(),
            analysis: None,
        }
    }

    fn staging(base: &Path, name: &str) -> PathBuf {
        let root = base.join(format!("{STAGING_DIR_PREFIX}{name}"));
        fs::create_dir_all(root.join("sub")).unwrap();
        root
    }

    fn submitted(entry: &VideoEntry, staging_root: &Path, rel_dir: &str) -> ArchiveSource {
        let rel = if rel_dir.is_empty() {
            entry.file_name.clone()
        } else {
            format!("{rel_dir}/{}", entry.file_name)
        };
        ArchiveSource {
            video_path: entry.path.clone(),
            video_file_name: entry.file_name.clone(),
            staging_root: staging_root.to_string_lossy().to_string(),
            relative_video_path: rel,
            relative_video_dir: rel_dir.to_string(),
            season: Some(9),
            episode: Some(9),
        }
    }

    #[test]
    fn arbitrary_staging_dir_with_prefix_is_not_trusted() {
        let base = unique_dir("untrusted");
        let video = base.join("Show.S01E01.mkv");
        fs::write(&video, b"video").unwrap();
        let e = entry(&video, Some(1), Some(1));
        let root = staging(&base, "forged");
        let source = submitted(&e, &root, "");
        let registry = ProvenanceRegistry::default();

        let err = registry.validate(&source, &[e]).unwrap_err();
        assert!(err.contains("رکورد"), "{err}");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn arbitrary_existing_file_outside_library_is_rejected() {
        let base = unique_dir("outside");
        let video = base.join("Show.S01E01.mkv");
        fs::write(&video, b"video").unwrap();
        let e = entry(&video, Some(1), Some(1));
        let root = staging(&base, "x");

        let other = base.join("Other.S01E01.mkv");
        fs::write(&other, b"video").unwrap();
        let other_entry = entry(&other, Some(1), Some(1));
        let registry = ProvenanceRegistry::default();
        let err = registry
            .validate(&submitted(&other_entry, &root, ""), &[e])
            .unwrap_err();
        assert!(err.contains("کتابخانه"), "{err}");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn unsupported_video_extension_is_rejected() {
        let base = unique_dir("ext");
        let file = base.join("Show.S01E01.txt");
        fs::write(&file, b"x").unwrap();
        let e = entry(&file, Some(1), Some(1));
        let root = staging(&base, "x");
        let mut registry = ProvenanceRegistry::default();
        let err = registry
            .register(&e, &root, "Show.S01E01.txt", "")
            .unwrap_err();
        assert!(err.contains("پسوند"), "{err}");
        let err = registry
            .validate(&submitted(&e, &root, ""), std::slice::from_ref(&e))
            .unwrap_err();
        assert!(err.contains("پسوند"), "{err}");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn forged_season_episode_is_replaced_by_authoritative_entry() {
        let base = unique_dir("season");
        let video = base.join("Episode 5.mkv");
        fs::write(&video, b"video").unwrap();
        let e = entry(&video, Some(2), Some(5));
        let root = staging(&base, "x");
        let mut registry = ProvenanceRegistry::default();
        let registered = registry.register(&e, &root, "Episode 5.mkv", "").unwrap();
        assert_eq!(registered.season, Some(2));
        assert_eq!(registered.episode, Some(5));

        let mut forged = submitted(&e, &root, "");
        forged.season = Some(1);
        forged.episode = Some(1);
        let canonical = registry.validate(&forged, &[e]).unwrap();
        assert_eq!((canonical.season, canonical.episode), (Some(2), Some(5)));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn valid_record_survives_reregistration_and_preserves_old_archive() {
        let base = unique_dir("valid");
        let video = base.join("Show.S01E01.mkv");
        fs::write(&video, b"video").unwrap();
        let e = entry(&video, Some(1), Some(1));
        let old_root = staging(&base, "old");
        let old_archive = old_root.join("sub").join("old.zip");
        fs::write(&old_archive, b"archive-bytes").unwrap();
        let new_root = staging(&base, "new");

        let mut registry = ProvenanceRegistry::default();
        let old_source = registry
            .register(&e, &old_root, "Show.S01E01.mkv", "")
            .unwrap();
        let new_source = registry
            .register(&e, &new_root, "Show.S01E01.mkv", "")
            .unwrap();

        assert!(
            old_archive.exists(),
            "ثبت رکورد جدید نباید آرشیو قبلی را حذف کند"
        );
        assert_eq!(
            registry
                .validate(&old_source, std::slice::from_ref(&e))
                .unwrap(),
            old_source
        );
        assert_eq!(registry.validate(&new_source, &[e]).unwrap(), new_source);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn filename_mismatch_and_duplicate_sources_are_rejected() {
        let base = unique_dir("mismatch");
        let video = base.join("Show.S01E01.mkv");
        fs::write(&video, b"video").unwrap();
        let e = entry(&video, Some(1), Some(1));
        let root = staging(&base, "x");
        let mut registry = ProvenanceRegistry::default();
        let source = registry.register(&e, &root, "Show.S01E01.mkv", "").unwrap();

        let mut wrong_name = source.clone();
        wrong_name.video_file_name = "Other.S01E01.mkv".into();
        assert!(registry
            .validate(&wrong_name, std::slice::from_ref(&e))
            .is_err());

        let mut wrong_rel = source.clone();
        wrong_rel.relative_video_path = "../Show.S01E01.mkv".into();
        assert!(registry
            .validate(&wrong_rel, std::slice::from_ref(&e))
            .is_err());

        let err = validate_archive_sources(&registry, &[e], &[source.clone(), source]).unwrap_err();
        assert!(err.contains("تکراری"), "{err}");
        let _ = fs::remove_dir_all(&base);
    }
}
