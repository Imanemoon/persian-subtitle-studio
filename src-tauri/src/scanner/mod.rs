//! اسکن بازگشتی پوشه‌ها برای یافتن ویدیوها و زیرنویس‌های کناری.

pub mod naming;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, UNIX_EPOCH};

use walkdir::WalkDir;

use crate::models::{ScanProgress, ScanSummary, SubtitleFileInfo, VideoEntry};
use naming::{
    clean_show_name, file_extension, is_subtitle_file, is_video_file, name_suggests_fa,
    parse_episode, parse_season_dir, sub_matches_video_stem,
};

/// پوشه‌هایی که همیشه نادیده گرفته می‌شوند (نام‌های حروف کوچک).
pub const ALWAYS_EXCLUDED_DIRS: &[&str] = &["$recycle.bin", "system volume information"];

/// پیشوند پوشه‌های کاری موقت که باید از اسکن مستثنا باشند.
const STAGING_PREFIX: &str = "_subsource_";

pub struct ScanOutput {
    pub entries: Vec<VideoEntry>,
    pub summary: ScanSummary,
    pub warnings: Vec<String>,
}

/// آیا نام پوشه باید از اسکن مستثنا شود؟
pub fn is_excluded_dir_name(name: &str, excluded: &[String]) -> bool {
    let lower = name.to_lowercase();
    if lower.starts_with('.') || lower.starts_with(STAGING_PREFIX) {
        return true;
    }
    if ALWAYS_EXCLUDED_DIRS.contains(&lower.as_str()) {
        return true;
    }
    excluded.iter().any(|e| e.trim().to_lowercase() == lower)
}

/// اگر مسیر ریشه (یا والدین آن) درون یکی از پوشه‌های مستثنا باشد، نام آن را برمی‌گرداند.
pub fn root_contains_excluded(root: &Path, excluded: &[String]) -> Option<String> {
    for anc in root.ancestors() {
        if let Some(name) = anc.file_name() {
            if is_excluded_dir_name(&name.to_string_lossy(), excluded) {
                return Some(name.to_string_lossy().to_string());
            }
        }
    }
    None
}

/// اسکن بازگشتی یک پوشه با گزارش پیشرفت و پشتیبانی از لغو.
pub fn scan_folder<F: FnMut(ScanProgress)>(
    root: &Path,
    excluded: &[String],
    cancel: &AtomicBool,
    on_progress: &mut F,
) -> ScanOutput {
    let started = Instant::now();
    let mut summary = ScanSummary {
        root: root.to_string_lossy().to_string(),
        ..Default::default()
    };
    let mut entries: Vec<VideoEntry> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    // کش زیرنویس‌های هر پوشه تا هر پوشه فقط یک‌بار خوانده شود.
    let mut sub_cache: HashMap<PathBuf, Vec<SubtitleFileInfo>> = HashMap::new();

    let mut it = WalkDir::new(root).follow_links(false).into_iter();

    loop {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }
        let entry = match it.next() {
            Some(Ok(e)) => e,
            Some(Err(err)) => {
                if warnings.len() < 50 {
                    warnings.push(format!("خطا در پیمایش: {err}"));
                }
                continue;
            }
            None => break,
        };

        if entry.depth() == 0 {
            continue; // خود ریشه
        }

        if entry.file_type().is_dir() {
            let name = entry.file_name().to_string_lossy();
            if is_excluded_dir_name(&name, excluded) {
                summary.excluded_dir_hits += 1;
                it.skip_current_dir();
                continue;
            }
            summary.dirs_scanned += 1;
            on_progress(ScanProgress {
                dirs_scanned: summary.dirs_scanned,
                files_seen: summary.files_seen,
                videos_found: summary.video_count,
                current_path: entry.path().to_string_lossy().to_string(),
            });
            continue;
        }

        if !entry.file_type().is_file() {
            continue;
        }
        summary.files_seen += 1;

        let file_name = entry.file_name().to_string_lossy().to_string();
        if !is_video_file(&file_name) {
            continue;
        }

        let path = entry.path().to_path_buf();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| file_name.clone());
        let parent = path.parent().unwrap_or(root).to_path_buf();

        let (season, episode, show_name) = resolve_titles(root, &path, &stem);
        if season.is_none() && episode.is_none() {
            summary.parse_failed += 1;
        }

        let sibling_subtitles = sibling_subtitles(&parent, &stem, &mut sub_cache);
        let has_fa = sibling_subtitles.iter().any(|s| s.name_suggests_fa);
        if has_fa {
            summary.with_fa_sidecar += 1;
        }
        if !sibling_subtitles.is_empty() {
            summary.with_any_sidecar += 1;
        }

        let meta = entry.metadata().ok();
        let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified_ms = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64);

        summary.video_count += 1;
        entries.push(VideoEntry {
            id: path.to_string_lossy().to_string(),
            path: path.to_string_lossy().to_string(),
            file_name,
            size_bytes,
            modified_ms,
            parent_dir: parent.to_string_lossy().to_string(),
            show_name,
            season,
            episode,
            sibling_subtitles,
        });

        on_progress(ScanProgress {
            dirs_scanned: summary.dirs_scanned,
            files_seen: summary.files_seen,
            videos_found: summary.video_count,
            current_path: path.to_string_lossy().to_string(),
        });
    }

    entries.sort_by(|a, b| {
        let ka = (
            a.show_name.as_deref().unwrap_or("").to_lowercase(),
            a.season.unwrap_or(0),
            a.episode.unwrap_or(0),
            a.file_name.to_lowercase(),
        );
        let kb = (
            b.show_name.as_deref().unwrap_or("").to_lowercase(),
            b.season.unwrap_or(0),
            b.episode.unwrap_or(0),
            b.file_name.to_lowercase(),
        );
        ka.cmp(&kb)
    });

    summary.duration_ms = started.elapsed().as_millis() as u64;
    ScanOutput {
        entries,
        summary,
        warnings,
    }
}

/// تشخیص نام سریال/فصل/قسمت با کمک ساختار پوشه‌ها.
fn resolve_titles(
    root: &Path,
    video_path: &Path,
    stem: &str,
) -> (Option<u32>, Option<u32>, Option<String>) {
    let parent = video_path.parent().unwrap_or(root);
    let parent_name = parent.file_name().map(|s| s.to_string_lossy().to_string());
    let parent_season = parent_name.as_deref().and_then(parse_season_dir);

    let ep_match = parse_episode(stem);
    let mut season = ep_match.as_ref().map(|m| m.season);
    let episode = ep_match.as_ref().map(|m| m.episode);

    // نام سریال از متنِ قبل از الگوی SxxEyy در نام فایل
    let mut show: Option<String> = ep_match.as_ref().and_then(|m| {
        (m.token_start > 0).then(|| {
            let raw = &stem[..m.token_start];
            let cleaned = clean_show_name(raw);
            (!cleaned.is_empty()).then_some(cleaned)
        })?
    });

    // در غیر این صورت از نام پوشه (و اگر پوشه، پوشه فصل باشد، از نام پدربزرگ)
    if show.is_none() {
        if parent_season.is_some() {
            if let (Some(grand), true) = (parent.parent(), parent.starts_with(root)) {
                if let Some(gn) = grand.file_name() {
                    let cleaned = clean_show_name(&gn.to_string_lossy());
                    if !cleaned.is_empty() {
                        show = Some(cleaned);
                    }
                }
            }
        } else if let Some(pn) = &parent_name {
            let cleaned = clean_show_name(pn);
            if !cleaned.is_empty() {
                show = Some(cleaned);
            }
        }
    }

    if season.is_none() {
        season = parent_season;
    }

    (season, episode, show)
}

/// زیرنویس‌های هم‌پوشهٔ یک ویدیو را برمی‌گرداند (هر پوشه حداکثر یک‌بار خوانده می‌شود).
fn sibling_subtitles(
    dir: &Path,
    video_stem: &str,
    cache: &mut HashMap<PathBuf, Vec<SubtitleFileInfo>>,
) -> Vec<SubtitleFileInfo> {
    let dir_subs = cache.entry(dir.to_path_buf()).or_insert_with(|| {
        let mut out = Vec::new();
        let Ok(read) = std::fs::read_dir(dir) else {
            return out;
        };
        for e in read.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if !is_subtitle_file(&name) {
                continue;
            }
            let ext = file_extension(&name);
            let sub_stem = std::path::Path::new(&name)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| name.clone());
            out.push(SubtitleFileInfo {
                path: p.to_string_lossy().to_string(),
                file_name: name,
                ext,
                size_bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
                name_suggests_fa: name_suggests_fa(&sub_stem),
            });
        }
        out.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        out
    });

    dir_subs
        .iter()
        .filter(|s| {
            let stem = std::path::Path::new(&s.file_name)
                .file_stem()
                .map(|x| x.to_string_lossy().to_string())
                .unwrap_or_else(|| s.file_name.clone());
            sub_matches_video_stem(&stem, video_stem)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("subsync-test-{tag}-{}-{nanos}", std::process::id()))
    }

    fn touch(dir: &Path, name: &str, content: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), content).unwrap();
    }

    fn make_tree(root: &Path) {
        // زیرنویس‌های واقعی stem کامل ویدیو را دارند (مطابق اسکریپت مرجع)
        touch(root, "Dexter.S04E01.720p.mkv", "");
        touch(root, "Dexter.S04E01.720p.fa.srt", "سلام");
        touch(root, "Dexter.S04E01.720p.synced.srt", "text");
        // نام فایل بدون نام سریال — نام سریال باید از پدربزرگ («Other Show») استخراج شود
        touch(&root.join("Other Show").join("Season 02"), "S02E05.mkv", "");
        touch(
            &root.join("Other Show").join("Season 02"),
            "S02E05.fa.ass",
            "فارسی",
        );
        touch(&root.join("Ted Lasso"), "Ted.Lasso.S01E01.mkv", "");
        touch(&root.join("_SubSource_Batch_abc"), "y.mkv", "");
        touch(&root.join(".hidden"), "z.mkv", "");
        touch(&root.join("Movies"), "Movie.2023.1080p.mkv", "");
        touch(&root.join("Movies"), "Movie.2023.1080p.srt", "english only");
        touch(&root.join("G"), "Show.S01E01.mkv", "");
        touch(&root.join("G"), "Show.S01E010.srt", "guard");
    }

    #[test]
    fn scans_nested_tree_and_respects_exclusions() {
        let root = unique_temp_dir("scan");
        make_tree(&root);
        let cancel = AtomicBool::new(false);
        let excluded = vec!["Ted Lasso".to_string(), "Better Call Saul".to_string()];
        let mut no_progress = |_p: ScanProgress| {};

        let out = scan_folder(&root, &excluded, &cancel, &mut no_progress);

        // ویدیوها: Dexter، Other.Show، Movie، Show.S01E01 (چهار مورد)
        assert_eq!(out.summary.video_count, 4, "{:?}", out.entries);
        // پوشه‌های مستثنا: Ted Lasso، _SubSource_Batch_abc، .hidden
        assert_eq!(out.summary.excluded_dir_hits, 3);
        assert_eq!(out.summary.parse_failed, 1); // Movie.2023
        assert_eq!(out.summary.with_any_sidecar, 3);
        assert_eq!(out.summary.with_fa_sidecar, 2);

        let dexter = out
            .entries
            .iter()
            .find(|e| e.file_name.starts_with("Dexter"))
            .expect("dexter entry");
        assert_eq!(dexter.show_name.as_deref(), Some("Dexter"));
        assert_eq!(dexter.season, Some(4));
        assert_eq!(dexter.episode, Some(1));
        assert_eq!(dexter.sibling_subtitles.len(), 2);
        // نسخه fa.srt نشانه فارسی دارد؛ نسخه synced.srt نشانه‌ای ندارد
        assert!(dexter
            .sibling_subtitles
            .iter()
            .any(|s| s.name_suggests_fa && s.file_name.ends_with("fa.srt")));
        assert!(dexter
            .sibling_subtitles
            .iter()
            .any(|s| s.file_name.ends_with("synced.srt") && !s.name_suggests_fa));

        let other = out
            .entries
            .iter()
            .find(|e| e.file_name == "S02E05.mkv")
            .expect("other entry");
        // نام سریال از پدربزرگ (چون پدر، پوشه فصل است)
        assert_eq!(other.show_name.as_deref(), Some("Other Show"));
        assert_eq!(other.season, Some(2));
        assert_eq!(other.episode, Some(5));

        let movie = out
            .entries
            .iter()
            .find(|e| e.file_name.starts_with("Movie."))
            .expect("movie entry");
        assert_eq!(movie.show_name.as_deref(), Some("Movies"));
        assert_eq!(movie.season, None);
        assert_eq!(movie.episode, None);
        assert_eq!(movie.sibling_subtitles.len(), 1);
        assert!(!movie.sibling_subtitles[0].name_suggests_fa);

        // گارد E010: زیرنویس Show.S01E010.srt نباید به Show.S01E01.mkv بخورد
        let guarded = out
            .entries
            .iter()
            .find(|e| e.file_name == "Show.S01E01.mkv")
            .expect("guard entry");
        assert!(guarded.sibling_subtitles.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cancellation_stops_scan() {
        let root = unique_temp_dir("cancel");
        touch(&root, "a.mkv", "");
        touch(&root, "b.mkv", "");
        let cancel = AtomicBool::new(true); // از قبل لغو شده
        let excluded: Vec<String> = vec![];
        let mut no_progress = |_p: ScanProgress| {};

        let out = scan_folder(&root, &excluded, &cancel, &mut no_progress);
        assert!(out.summary.cancelled);
        assert_eq!(out.summary.video_count, 0);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn root_inside_excluded_folder_is_detected() {
        let excluded = vec!["Ted Lasso".to_string()];
        let p = Path::new("D:/Media/Ted Lasso/Season 01");
        assert_eq!(
            root_contains_excluded(p, &excluded),
            Some("Ted Lasso".to_string())
        );
        assert_eq!(
            root_contains_excluded(Path::new("D:/Media"), &excluded),
            None
        );
    }

    #[test]
    fn always_excluded_dirs() {
        assert!(is_excluded_dir_name("$RECYCLE.BIN", &[]));
        assert!(is_excluded_dir_name("System Volume Information", &[]));
        assert!(is_excluded_dir_name("_SubSource_Batch_123", &[]));
        assert!(is_excluded_dir_name(".git", &[]));
        assert!(is_excluded_dir_name(
            "ted lasso",
            &["Ted Lasso".to_string()]
        ));
        assert!(!is_excluded_dir_name("Dexter", &["Ted Lasso".to_string()]));
    }
}
