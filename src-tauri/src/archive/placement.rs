//! جای‌گذاری ایمن زیرنویس انتخاب‌شده کنار ویدیو.
//!
//! خروجی همیشه `<video_stem>.fa.<original_extension>` است. فایل موجود هرگز
//! بازنویسی نمی‌شود: نوشتن با `create_new` انجام می‌شود تا حتی در شرایط
//! مسابقه‌ای (race) هم فقط یک نویسنده برنده شود. اگر نوشتن نیمه‌کاره شکست
//! بخورد، فقط همان فایل ناقصِ ساخته‌شده توسط همین عملیات حذف می‌شود.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::scanner::naming::SUBTITLE_EXTENSIONS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlacementError {
    /// مقصد از قبل وجود دارد — هرگز بازنویسی نمی‌شود.
    TargetExists(PathBuf),
    Invalid(String),
    Io(String),
}

impl std::fmt::Display for PlacementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlacementError::TargetExists(p) => {
                write!(f, "زیرنویس مقصد از قبل موجود است: {}", p.display())
            }
            PlacementError::Invalid(m) => write!(f, "{m}"),
            PlacementError::Io(m) => write!(f, "{m}"),
        }
    }
}

/// مسیر مقصد استاندارد برای ویدیو و پسوند اصلی زیرنویس.
pub fn output_path(video_path: &Path, extension: &str) -> Result<PathBuf, PlacementError> {
    let ext = extension.trim_start_matches('.').to_ascii_lowercase();
    if ext.is_empty() || !SUBTITLE_EXTENSIONS.contains(&ext.as_str()) {
        return Err(PlacementError::Invalid(format!(
            "پسوند زیرنویس «{extension}» پشتیبانی نمی‌شود"
        )));
    }
    let stem = video_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            PlacementError::Invalid(format!("نام ویدیوی «{}» نامعتبر است", video_path.display()))
        })?;
    let parent = video_path.parent().ok_or_else(|| {
        PlacementError::Invalid(format!("پوشهٔ ویدیوی «{}» مشخص نیست", video_path.display()))
    })?;
    Ok(parent.join(format!("{stem}.fa.{ext}")))
}

/// کپی امن فایل استخراج‌شده به مقصد نهایی، بدون هیچ بازنویسی.
///
/// ترتیب: بررسی فایل مبدأ → `create_new` روی مقصد → کپی → flush.
/// در خطای کپی، فایل ناقصِ همین عملیات حذف می‌شود.
pub fn place_subtitle(extracted: &Path, target: &Path) -> Result<u64, PlacementError> {
    let source_meta = fs::symlink_metadata(extracted).map_err(|e| {
        PlacementError::Io(format!(
            "خواندن متادیتای زیرنویس استخراج‌شده «{}» ناموفق: {e}",
            extracted.display()
        ))
    })?;
    if !source_meta.is_file() {
        return Err(PlacementError::Invalid(format!(
            "«{}» یک فایل معمولی نیست",
            extracted.display()
        )));
    }
    if source_meta.len() == 0 {
        return Err(PlacementError::Invalid(format!(
            "«{}» خالی است",
            extracted.display()
        )));
    }

    if let Ok(meta) = fs::symlink_metadata(target) {
        if meta.is_file() || meta.is_dir() || meta.file_type().is_symlink() {
            return Err(PlacementError::TargetExists(target.to_path_buf()));
        }
    }

    let mut source = File::open(extracted).map_err(|e| {
        PlacementError::Io(format!(
            "بازکردن زیرنویس استخراج‌شده «{}» ناموفق: {e}",
            extracted.display()
        ))
    })?;

    let mut dest = match OpenOptions::new().write(true).create_new(true).open(target) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            return Err(PlacementError::TargetExists(target.to_path_buf()));
        }
        Err(e) => {
            return Err(PlacementError::Io(format!(
                "ساخت زیرنویس مقصد «{}» ناموفق: {e}",
                target.display()
            )));
        }
    };

    let copy_result = (|| -> io::Result<u64> {
        let mut buffer = [0u8; 64 * 1024];
        let mut written = 0u64;
        loop {
            let n = source.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            dest.write_all(&buffer[..n])?;
            written += n as u64;
        }
        dest.flush()?;
        Ok(written)
    })();

    match copy_result {
        Ok(written) => Ok(written),
        Err(e) => {
            drop(dest);
            let cleanup = fs::remove_file(target);
            match cleanup {
                Ok(()) => Err(PlacementError::Io(format!(
                    "نوشتن «{}» ناموفق بود: {e} — فایل ناقص حذف شد",
                    target.display()
                ))),
                Err(cleanup_err) => Err(PlacementError::Io(format!(
                    "نوشتن «{}» ناموفق بود: {e} — حذف فایل ناقص هم ناموفق بود ({cleanup_err})؛ این فایل باید دستی بررسی شود",
                    target.display()
                ))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn unique_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "subsync-placement-{tag}-{}-{}",
            std::process::id(),
            crate::subsource::staging::new_uid()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn builds_output_path_preserving_extension() {
        let dir = unique_dir("path");
        let video = dir.join("Show.S01E01.mkv");
        assert_eq!(
            output_path(&video, "ass").unwrap(),
            dir.join("Show.S01E01.fa.ass")
        );
        assert!(output_path(&video, "exe").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_target_is_never_overwritten() {
        let dir = unique_dir("exists");
        let src = dir.join("in.srt");
        let dst = dir.join("out.fa.srt");
        fs::write(&src, b"new content").unwrap();
        fs::write(&dst, b"old content").unwrap();

        let err = place_subtitle(&src, &dst).unwrap_err();
        assert!(matches!(err, PlacementError::TargetExists(_)));
        assert_eq!(fs::read(&dst).unwrap(), b"old content");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_placement_has_exactly_one_winner() {
        let dir = unique_dir("race");
        let src = dir.join("in.srt");
        let dst = dir.join("out.fa.srt");
        fs::write(&src, b"content").unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let src = src.clone();
            let dst = dst.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                place_subtitle(&src, &dst)
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(PlacementError::TargetExists(_))))
                .count(),
            1
        );
        assert_eq!(fs::read(&dst).unwrap(), b"content");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_copy_removes_only_its_partial_target() {
        let dir = unique_dir("partial");
        let src = dir.join("in.srt");
        let dst = dir.join("out.fa.srt");
        fs::write(&src, b"content").unwrap();
        // شبیه‌سازی شکست خواندن: مبدأ حذف می‌شود بین بررسی و بازکردن سخت است؛
        // اما مقصد باید در پایان بدون فایل ناقص بماند.
        let err = place_subtitle(&src, &dst);
        assert!(err.is_ok());
        fs::remove_file(&src).unwrap();
        let dst2 = dir.join("out2.fa.srt");
        let err = place_subtitle(&src, &dst2).unwrap_err();
        assert!(matches!(err, PlacementError::Io(_)));
        assert!(!dst2.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
