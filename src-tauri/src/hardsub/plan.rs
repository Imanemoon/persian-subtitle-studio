//! ساخت پلن هاردساب — منبع زیرنویس و نام‌گذاری خروجی.
//!
//! منبع زیرنویس به‌ترتیب:
//! ۱. sidecar سینک‌شدهٔ فارسی (`<stem>.fa.synced.srt` سپس `<stem>.fa.srt`)
//! ۲. Track متنی فارسی داخلی ویدیو (استخراج در زمان اجرا)
//!
//! خروجی همیشه فایل جدید است: `<stem>.fa.hardsubbed.<ext>` — اگر موجود
//! باشد، مورد رد می‌شود و هرگز بازنویسی نمی‌شود.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::archive::listing::is_link_or_reparse;
use crate::hardsub::types::{HardsubPlan, HardsubPlanItem, HardsubPlanSkip};
use crate::media::ffprobe::{self, ProbeError};
use crate::models::SubtitleKind;
use crate::scanner::naming::is_video_file;
use crate::subtitle::validation::validate_file;

/// کانتینرهایی که خروجی با همان پسوند ساخته می‌شود.
/// webm اجازهٔ h264 نمی‌دهد؛ بقیه → mkv (سازگارترین کانتینر با copy صدا).
const KEEP_EXTENSION: [&str; 4] = ["mkv", "mp4", "m4v", "mov"];

/// پسوند خروجی هاردساب برای یک ویدیو.
pub fn output_extension(video: &Path) -> String {
    let ext = video
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if KEEP_EXTENSION.contains(&ext.as_str()) {
        ext
    } else {
        "mkv".into()
    }
}

/// مسیر خروجی نهایی هاردساب: `<stem>.fa.hardsubbed.<ext>` کنار ویدیو.
pub fn output_path_for(video: &Path) -> Option<PathBuf> {
    let stem = video.file_stem()?.to_string_lossy().into_owned();
    let dir = video.parent().filter(|dir| !dir.as_os_str().is_empty())?;
    Some(dir.join(format!("{stem}.fa.hardsubbed.{}", output_extension(video))))
}

/// منبع زیرنویس هر آیتم — تصمیم پلن؛ اجرا همان را مصرف می‌کند.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubtitleSource {
    /// فایل sidecar معتبر (سینک‌شده یا استخراج قبلی)
    File(PathBuf),
    /// Track متنی فارسی داخلی — stream index گلوبال
    EmbeddedStream { stream_index: u32 },
}

/// فایل ویدیوی قابل‌اعتماد؟ (معمولی، غیرخالی، بدون symlink/reparse)
fn validate_video_file(video: &Path) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(video).map_err(|error| format!("ویدیو قابل دسترسی نیست: {error}"))?;
    if !metadata.is_file() || is_link_or_reparse(&metadata) || metadata.len() == 0 {
        return Err("ویدیو باید یک فایل معمولیِ غیرخالی و بدون symlink باشد".into());
    }
    Ok(())
}

/// sidecar کاندیدای فارسی معتبر — synced اول، بعد fa.srt.
fn find_valid_sidecar(video: &Path) -> Option<PathBuf> {
    let stem = video.file_stem()?.to_string_lossy().into_owned();
    let dir = video.parent().filter(|dir| !dir.as_os_str().is_empty())?;
    for name in [format!("{stem}.fa.synced.srt"), format!("{stem}.fa.srt")] {
        let candidate = dir.join(name);
        let Ok(metadata) = fs::symlink_metadata(&candidate) else {
            continue;
        };
        if !metadata.is_file() || is_link_or_reparse(&metadata) || metadata.len() == 0 {
            continue;
        }
        let validation = validate_file(&candidate);
        if validation.valid && validation.has_persian {
            return Some(candidate);
        }
    }
    None
}

/// Track متنی فارسی داخلی — اولویت با Track با disposition=default؛ در
/// نبود آن اولین Track متنی کاندید فارسی. Track تصویری (PGS) برای burn
/// با فیلتر subtitles کار نمی‌کند و رد می‌شود.
fn find_embedded_fa_text(
    ffprobe: &Path,
    video: &Path,
    cancel: &AtomicBool,
) -> Result<Option<u32>, String> {
    let streams =
        ffprobe::probe_subtitle_streams(ffprobe, video, cancel).map_err(|error| match error {
            ProbeError::Cancelled => "پروب ویدیو لغو شد".to_string(),
            ProbeError::Failed(reason) => format!("پروب Trackهای داخلی ناموفق بود: {reason}"),
        })?;
    let text_fa = streams
        .iter()
        .filter(|stream| stream.persian_candidate && stream.kind == SubtitleKind::Text);
    let best = text_fa
        .clone()
        .find(|stream| stream.default_disposition)
        .or_else(|| text_fa.clone().next());
    Ok(best.map(|stream| stream.stream_index))
}

/// منبع زیرنویس یک ویدیو — Err = دلیل رد شدن این مورد.
pub fn find_subtitle_source(
    ffprobe: &Path,
    video: &Path,
    cancel: &AtomicBool,
) -> Result<SubtitleSource, String> {
    if let Some(path) = find_valid_sidecar(video) {
        return Ok(SubtitleSource::File(path));
    }
    match find_embedded_fa_text(ffprobe, video, cancel)? {
        Some(stream_index) => Ok(SubtitleSource::EmbeddedStream { stream_index }),
        None => Err("زیرنویس فارسی سینک‌شده (sidecar) و Track متنی فارسی داخلی یافت نشد".into()),
    }
}

/// پلن کامل برای مسیرهای انتخابی — ترتیب ورودی حفظ می‌شود؛ تکراری‌ها و
/// موارد غیرقابل‌اجرا با دلیل صادقانه در skipped می‌آیند.
pub fn build_plan(
    paths: &[String],
    ffprobe: &Path,
    cancel: &AtomicBool,
    encoder_label: &str,
) -> HardsubPlan {
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for raw in paths {
        let video = PathBuf::from(raw.trim());
        let file_name = video
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| raw.clone());

        let key = video.to_string_lossy().to_lowercase();
        if !seen.insert(key) {
            skipped.push(HardsubPlanSkip {
                path: raw.clone(),
                reason: "این ویدیو در همین انتخاب تکراری است".into(),
            });
            continue;
        }

        if let Err(reason) = run_single(&video, &file_name, ffprobe, cancel, &mut items) {
            skipped.push(HardsubPlanSkip {
                path: raw.clone(),
                reason,
            });
        }
    }

    HardsubPlan {
        items,
        skipped,
        encoder: encoder_label.to_string(),
    }
}

fn run_single(
    video: &Path,
    file_name: &str,
    ffprobe: &Path,
    cancel: &AtomicBool,
    items: &mut Vec<HardsubPlanItem>,
) -> Result<(), String> {
    if !is_video_file(file_name) {
        return Err("پسوند ویدیو پشتیبانی نمی‌شود".into());
    }
    validate_video_file(video)?;
    let output = output_path_for(video).ok_or_else(|| "نمی‌توان مسیر خروجی ساخت".to_string())?;
    if fs::symlink_metadata(&output).is_ok() {
        return Err(format!(
            "خروجی «{}» از قبل موجود است — برای جلوگیری از بازنویسی رد شد",
            output.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    let source = find_subtitle_source(ffprobe, video, cancel)?;
    let (subtitle_path, embedded_stream_index) = match source {
        SubtitleSource::File(path) => (Some(path.to_string_lossy().into_owned()), None),
        SubtitleSource::EmbeddedStream { stream_index } => (None, Some(stream_index)),
    };
    items.push(HardsubPlanItem {
        video_path: video.to_string_lossy().into_owned(),
        video_file_name: file_name.to_string(),
        subtitle_path,
        embedded_stream_index,
        output_path: output.to_string_lossy().into_owned(),
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "subsync-hardsub-plan-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }

    struct Fixture {
        dir: PathBuf,
        ffprobe: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn fixture(tag: &str) -> Fixture {
        let dir = unique_temp_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        let ffprobe = dir.join("fake-probe.cmd");
        // ffprobe جعلی: بدون Track داخلی (سناریوی sidecar محور)
        fs::write(&ffprobe, "@echo off\r\necho {\"streams\": []}\r\n").unwrap();
        Fixture { dir, ffprobe }
    }

    const SRT_FA: &str = "1\n00:00:01,000 --> 00:00:02,000\nسلام دنیا\n";

    #[test]
    fn output_naming_keeps_supported_containers_and_maps_others_to_mkv() {
        assert_eq!(output_extension(Path::new("x/m.mkv")), "mkv");
        assert_eq!(output_extension(Path::new("x/m.MP4")), "mp4");
        assert_eq!(output_extension(Path::new("x/m.m4v")), "m4v");
        assert_eq!(output_extension(Path::new("x/m.mov")), "mov");
        // webm اجازهٔ h264 ندارد و ts فصل/متادیتا را نگه نمی‌دارد — هر دو mkv
        assert_eq!(output_extension(Path::new("x/m.webm")), "mkv");
        assert_eq!(output_extension(Path::new("x/m.ts")), "mkv");
        assert_eq!(output_extension(Path::new("x/m.avi")), "mkv");
        assert_eq!(output_extension(Path::new("x/m")), "mkv");

        let video = Path::new("D:/Shows/Movie.S01E01.720p.mkv");
        let output = output_path_for(video).unwrap();
        assert_eq!(
            output,
            PathBuf::from("D:/Shows/Movie.S01E01.720p.fa.hardsubbed.mkv")
        );
        let mp4 = output_path_for(Path::new("D:/x/Movie.mp4")).unwrap();
        assert_eq!(mp4, PathBuf::from("D:/x/Movie.fa.hardsubbed.mp4"));
    }

    #[test]
    #[cfg(windows)]
    fn plan_prefers_synced_sidecar_then_fa_srt_then_skips_without_any() {
        let fx = fixture("sidecar");
        let video = fx.dir.join("Show.S01E01.720p.mkv");
        fs::write(&video, b"FAKE-VIDEO").unwrap();

        // بدون هیچ زیرنویسی → skip
        let cancel = AtomicBool::new(false);
        let plan = build_plan(
            &[video.to_string_lossy().into_owned()],
            &fx.ffprobe,
            &cancel,
            "CPU (libx264)",
        );
        assert!(plan.items.is_empty());
        assert_eq!(plan.skipped.len(), 1);
        assert!(
            plan.skipped[0].reason.contains("زیرنویس فارسی"),
            "دلیل باید صادقانه باشد: {}",
            plan.skipped[0].reason
        );

        // fa.srt معتبر → File
        let fa = fx.dir.join("Show.S01E01.720p.fa.srt");
        fs::write(&fa, SRT_FA).unwrap();
        let plan = build_plan(
            &[video.to_string_lossy().into_owned()],
            &fx.ffprobe,
            &cancel,
            "E",
        );
        assert_eq!(plan.items.len(), 1);
        assert_eq!(
            plan.items[0].subtitle_path.as_deref(),
            Some(fa.to_string_lossy().as_ref())
        );
        assert!(plan.items[0].embedded_stream_index.is_none());
        assert!(plan.items[0]
            .output_path
            .ends_with("Show.S01E01.720p.fa.hardsubbed.mkv"));

        // synced.srt معتبر → اولویت بر fa.srt
        let synced = fx.dir.join("Show.S01E01.720p.fa.synced.srt");
        fs::write(&synced, SRT_FA).unwrap();
        let plan = build_plan(
            &[video.to_string_lossy().into_owned()],
            &fx.ffprobe,
            &cancel,
            "E",
        );
        assert_eq!(plan.items.len(), 1);
        assert_eq!(
            plan.items[0].subtitle_path.as_deref(),
            Some(synced.to_string_lossy().as_ref()),
            "synced.srt باید بر fa.srt ترجیح داده شود"
        );
    }

    #[test]
    #[cfg(windows)]
    fn plan_skips_existing_output_and_invalid_sidecars() {
        let fx = fixture("conflict");
        let cancel = AtomicBool::new(false);

        // sidecar خالی/نامعتبر → skip (نه پسوند، نه فایل)
        let video = fx.dir.join("Movie.mkv");
        fs::write(&video, b"FAKE-VIDEO").unwrap();
        let fa = fx.dir.join("Movie.fa.srt");
        fs::write(&fa, "").unwrap();
        let plan = build_plan(
            &[video.to_string_lossy().into_owned()],
            &fx.ffprobe,
            &cancel,
            "E",
        );
        assert!(plan.items.is_empty(), "sidecar خالی نباید منبع شود");

        // پسوند غیر ویدیویی → skip
        let text = fx.dir.join("notes.txt");
        fs::write(&text, "hello").unwrap();
        let plan = build_plan(
            &[text.to_string_lossy().into_owned()],
            &fx.ffprobe,
            &cancel,
            "E",
        );
        assert!(plan.items.is_empty());
        assert!(plan.skipped[0].reason.contains("پسوند"));

        // خروجی موجود → skip با پیام تعارض
        let output = fx.dir.join("Movie.fa.hardsubbed.mkv");
        fs::write(&output, b"OLD-OUTPUT").unwrap();
        fs::write(&fa, SRT_FA).unwrap();
        let plan = build_plan(
            &[video.to_string_lossy().into_owned()],
            &fx.ffprobe,
            &cancel,
            "E",
        );
        assert!(plan.items.is_empty());
        assert!(
            plan.skipped[0].reason.contains("موجود است"),
            "دلیل تعارض باید شفاف باشد: {}",
            plan.skipped[0].reason
        );
    }

    #[test]
    #[cfg(windows)]
    fn plan_dedupes_duplicate_paths_and_preserves_order() {
        let fx = fixture("dedupe");
        let cancel = AtomicBool::new(false);
        let a = fx.dir.join("A.mkv");
        let b = fx.dir.join("B.mkv");
        fs::write(&a, b"VA").unwrap();
        fs::write(&b, b"VB").unwrap();
        fs::write(fx.dir.join("A.fa.srt"), SRT_FA).unwrap();
        fs::write(fx.dir.join("B.fa.srt"), SRT_FA).unwrap();

        let a_str = a.to_string_lossy().into_owned();
        let b_str = b.to_string_lossy().into_owned();
        let plan = build_plan(
            &[a_str.clone(), b_str.clone(), a_str.to_uppercase()],
            &fx.ffprobe,
            &cancel,
            "E",
        );
        assert_eq!(plan.items.len(), 2, "تکراری باید skip شود");
        assert_eq!(plan.items[0].video_file_name, "A.mkv");
        assert_eq!(plan.items[1].video_file_name, "B.mkv");
        assert_eq!(plan.skipped.len(), 1);
        assert!(plan.skipped[0].reason.contains("تکراری"));
    }
}
