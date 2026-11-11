//! اسکن بازگشتی پوشه‌ها برای یافتن ویدیوها و زیرنویس‌های کناری.

pub mod naming;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, UNIX_EPOCH};

use walkdir::WalkDir;

use crate::archive::listing::is_link_or_reparse;
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

/// آیا نام پوشه در فهرست مستثناهای پیکربندی‌شدهٔ کاربر است؟
pub fn is_user_excluded(name: &str, excluded: &[String]) -> bool {
    let lower = name.to_lowercase();
    excluded.iter().any(|e| e.trim().to_lowercase() == lower)
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
    is_user_excluded(name, excluded)
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
        entries.push(build_video_entry(
            &path,
            file_name,
            root,
            &mut sub_cache,
            &mut summary,
        ));

        on_progress(ScanProgress {
            dirs_scanned: summary.dirs_scanned,
            files_seen: summary.files_seen,
            videos_found: summary.video_count,
            current_path: path.to_string_lossy().to_string(),
        });
    }

    sort_entries(&mut entries);

    summary.duration_ms = started.elapsed().as_millis() as u64;
    ScanOutput {
        entries,
        summary,
        warnings,
    }
}

/// ساخت ورودی ویدیو از یک مسیر ویدیوی معتبر + به‌روزرسانی شمارنده‌های خلاصه.
///
/// `titles_root` مرز بالایی استخراج نام سریال است: در اسکن پوشه ریشهٔ اسکن و در
/// اسکن فایل‌های انتخابی خودِ پوشهٔ والد ویدیو (هیچ مرز بالاتری وجود ندارد).
fn build_video_entry(
    path: &Path,
    file_name: String,
    titles_root: &Path,
    sub_cache: &mut HashMap<PathBuf, Vec<SubtitleFileInfo>>,
    summary: &mut ScanSummary,
) -> VideoEntry {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file_name.clone());
    let parent = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| titles_root.to_path_buf());

    let (season, episode, show_name) = resolve_titles(titles_root, path, &stem);
    if season.is_none() && episode.is_none() {
        summary.parse_failed += 1;
    }

    let sibling_subtitles = sibling_subtitles(&parent, &stem, sub_cache);
    let has_fa = sibling_subtitles.iter().any(|s| s.name_suggests_fa);
    if has_fa {
        summary.with_fa_sidecar += 1;
    }
    if !sibling_subtitles.is_empty() {
        summary.with_any_sidecar += 1;
    }

    // symlink_metadata معادل lstat است — همان معنایی که WalkDir بدون follow_links می‌دهد.
    let meta = std::fs::symlink_metadata(path).ok();
    let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let modified_ms = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);

    summary.video_count += 1;
    VideoEntry {
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
        analysis: None,
    }
}

/// مرتب‌سازی مشترک ورودی‌ها (نام سریال، فصل، قسمت، نام فایل).
fn sort_entries(entries: &mut [VideoEntry]) {
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
}

/// پیشرفت واقعی اسکن فایل‌های انتخابی — هر مسیر پردازش‌شده یک رخداد.
fn selected_scan_progress(summary: &ScanSummary, current: &str) -> ScanProgress {
    ScanProgress {
        dirs_scanned: summary.dirs_scanned,
        files_seen: summary.files_seen,
        videos_found: summary.video_count,
        current_path: current.to_string(),
    }
}

/// اسکن دقیق فایل‌های ویدیویی انتخاب‌شدهٔ کاربر — پوشهٔ والد هرگز پیمایش نمی‌شود
/// و هیچ ویدیوی دیگری وارد نتیجه نمی‌شود.
///
/// مسیرهای نامعتبر تک‌به‌تک در `warnings` گزارش می‌شوند (دور انداخته نمی‌شوند).
/// پوشه‌های مستثنای پیکربندی‌شدهٔ کاربر برای انتخاب صریح پذیرفته می‌شوند (فقط در
/// `summary.excluded_dir_hits` شمرده می‌شوند)، اما staging (`_subsource_*`) و
/// پوشه‌های سیستمی محافظت‌شده در هر عمقی رد می‌شوند.
/// هیچ تغییری روی دیسک داده نمی‌شود؛ فقط خواندن متادیتا.
pub fn scan_selected_files<F: FnMut(ScanProgress)>(
    paths: &[String],
    excluded: &[String],
    cancel: &AtomicBool,
    on_progress: &mut F,
) -> ScanOutput {
    let started = Instant::now();
    // ریشهٔ واقعی معنا ندارد؛ `selected_files` مرجع صادقانهٔ ورودی‌های پذیرفته‌شده است.
    let mut summary = ScanSummary {
        root: String::new(),
        ..Default::default()
    };
    let mut entries: Vec<VideoEntry> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut sub_cache: HashMap<PathBuf, Vec<SubtitleFileInfo>> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();

    for raw in paths {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }
        summary.files_seen += 1;
        // هم‌راستا با start_scan: فاصله‌ها و گیومهٔ اضافه پاک می‌شوند.
        let path_text = raw.trim().trim_matches('"');
        let path = Path::new(path_text);

        if let Err(reason) = validate_selected_path(path) {
            warnings.push(format!("{path_text}: {reason}"));
            on_progress(selected_scan_progress(&summary, path_text));
            continue;
        }

        let key = selected_path_key(path_text);
        if !seen.insert(key) {
            warnings.push(format!("{path_text}: مسیر تکراری است"));
            on_progress(selected_scan_progress(&summary, path_text));
            continue;
        }

        // انتخاب صریح داخل پوشهٔ مستثنای کاربر پذیرفته می‌شود؛ فقط شمرده می‌شود.
        if path_in_user_excluded(path, excluded) {
            summary.excluded_dir_hits += 1;
        }

        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let titles_root = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(path_text));
        let entry = build_video_entry(path, file_name, &titles_root, &mut sub_cache, &mut summary);
        entries.push(entry);

        on_progress(selected_scan_progress(&summary, path_text));
    }

    sort_entries(&mut entries);
    summary.selected_files = entries.iter().map(|e| e.path.clone()).collect();
    summary.duration_ms = started.elapsed().as_millis() as u64;
    ScanOutput {
        entries,
        summary,
        warnings,
    }
}

/// اعتبارسنجی کامل یک مسیر انتخاب‌شدهٔ صریح — فقط خواندن، بدون هیچ تغییر روی دیسک.
fn validate_selected_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err("مسیر خالی است".into());
    }
    if !path.is_absolute() {
        return Err("مسیر مطلق نیست".into());
    }

    // اجزای پوشه‌ای (خود فایل نه) در هر عمقی باید خارج از staging و
    // پوشه‌های سیستمی محافظت‌شده باشند؛ مستثناهای کاربر اینجا نادیده گرفته می‌شوند.
    for anc in path.ancestors().skip(1) {
        let Some(name) = anc.file_name() else {
            continue;
        };
        let lower = name.to_string_lossy().to_lowercase();
        if lower.starts_with(STAGING_PREFIX) {
            return Err("مسیر درون پوشهٔ کاری staging است".into());
        }
        if ALWAYS_EXCLUDED_DIRS.contains(&lower.as_str()) {
            return Err("مسیر درون پوشهٔ سیستمی محافظت‌شده است".into());
        }
    }

    // والدین: موجود بودن + نبود symlink/reparse (از نزدیک‌ترین والد تا ریشه).
    for anc in path.ancestors().skip(1) {
        match std::fs::symlink_metadata(anc) {
            Ok(meta) => {
                if is_link_or_reparse(&meta) {
                    return Err(format!(
                        "پوشهٔ «{}» پیوند نمادین یا Reparse Point است",
                        anc.display()
                    ));
                }
            }
            Err(_) => return Err("مسیر روی دیسک پیدا نشد".into()),
        }
    }

    // خود فایل: موجود، پیوند/ری‌پارس نباشد، فایل معمولی باشد.
    let meta = std::fs::symlink_metadata(path).map_err(|_| "فایل روی دیسک پیدا نشد".to_string())?;
    if is_link_or_reparse(&meta) {
        return Err("پیوند نمادین یا Reparse Point پذیرفته نمی‌شود".into());
    }
    if !meta.is_file() {
        return Err("مسیر یک پوشه است، نه فایل ویدیویی".into());
    }

    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if !is_video_file(&file_name) {
        return Err("فرمت ویدیویی پشتیبانی نمی‌شود".into());
    }
    Ok(())
}

/// کلید مقایسهٔ مسیر برای تشخیص تکراری — ویندوز به بزرگی/کوچکی و نوع جداکننده حساس نیست.
fn selected_path_key(path_text: &str) -> String {
    if cfg!(windows) {
        path_text.replace('/', "\\").to_lowercase()
    } else {
        path_text.to_string()
    }
}

/// آیا یکی از پوشه‌های والد مسیر در فهرست مستثناهای کاربر است؟
fn path_in_user_excluded(path: &Path, excluded: &[String]) -> bool {
    path.ancestors()
        .skip(1)
        .filter_map(|anc| anc.file_name())
        .any(|name| is_user_excluded(&name.to_string_lossy(), excluded))
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

/// اسکن مجدد زیرنویس‌های کناری یک ویدیو (پس از استخراج، برای به‌روزرسانی UI).
pub fn scan_sibling_subtitles(dir: &Path, video_stem: &str) -> Vec<SubtitleFileInfo> {
    let mut cache = HashMap::new();
    sibling_subtitles(dir, video_stem, &mut cache)
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

    // ------------------------------------------------------------------
    // scan_selected_files — اسکن exact-path
    // ------------------------------------------------------------------

    fn mkv(dir: &Path, name: &str) -> PathBuf {
        touch(dir, name, "");
        dir.join(name)
    }

    fn sel(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }

    fn run_selected(paths: &[String]) -> ScanOutput {
        let cancel = AtomicBool::new(false);
        let mut no_progress = |_p: ScanProgress| {};
        scan_selected_files(paths, &[], &cancel, &mut no_progress)
    }

    #[test]
    fn selected_scan_single_video_produces_one_entry() {
        let root = unique_temp_dir("sel-single");
        let video = mkv(&root, "Movie.2023.1080p.mkv");

        let out = run_selected(&sel(std::slice::from_ref(&video)));

        assert_eq!(out.entries.len(), 1, "{:?}", out.warnings);
        assert_eq!(out.entries[0].path, video.to_string_lossy().to_string());
        assert_eq!(out.entries[0].file_name, "Movie.2023.1080p.mkv");
        assert_eq!(out.summary.video_count, 1);
        assert_eq!(out.summary.files_seen, 1);
        assert_eq!(out.summary.selected_files.len(), 1);
        assert!(out.warnings.is_empty());
        assert!(!out.summary.cancelled);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn selected_scan_multiple_videos_produce_exactly_those_entries() {
        let root = unique_temp_dir("sel-multi");
        let a = mkv(&root, "A.Show.S01E01.mkv");
        let b = mkv(&root, "B.Show.S01E01.mkv");
        // ویدیوی مزاحم هم‌پوشه — هرگز نباید وارد نتیجه شود
        mkv(&root, "Decoy.S01E01.mkv");

        let out = run_selected(&sel(&[b.clone(), a.clone()]));

        assert_eq!(out.summary.video_count, 2);
        let got: Vec<String> = out.entries.iter().map(|e| e.path.clone()).collect();
        let mut want = vec![
            a.to_string_lossy().into_owned(),
            b.to_string_lossy().into_owned(),
        ];
        want.sort();
        assert_eq!(got, want, "فقط فایل‌های انتخاب‌شده، مرتب‌شده");
        assert!(!got.iter().any(|p| p.contains("Decoy")));
        assert_eq!(out.summary.selected_files, got);
        assert!(out.warnings.is_empty());
        // ریشهٔ فریبنده نباید پر شود — selected_files مرجع صادقانه است
        assert_eq!(out.summary.root, "");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn selected_video_in_user_excluded_folder_is_accepted() {
        let root = unique_temp_dir("sel-excluded");
        let video = mkv(&root.join("Ted Lasso"), "Ted.Lasso.S01E01.mkv");
        let excluded = vec!["Ted Lasso".to_string()];

        let cancel = AtomicBool::new(false);
        let mut no_progress = |_p: ScanProgress| {};
        let out = scan_selected_files(&sel(&[video]), &excluded, &cancel, &mut no_progress);
        assert_eq!(out.entries.len(), 1, "{:?}", out.warnings);
        assert_eq!(out.summary.excluded_dir_hits, 1);

        // همان مسیر در اسکن پوشه همچنان مستثنا می‌ماند — رفتار پوشه دست‌نخورده
        let folder_out = scan_folder(&root, &excluded, &cancel, &mut no_progress);
        assert_eq!(folder_out.summary.video_count, 0);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn selected_unsupported_extension_is_rejected() {
        let root = unique_temp_dir("sel-ext");
        let txt = mkv(&root, "notes.txt");
        let no_ext = mkv(&root, "README");

        let out = run_selected(&sel(&[txt, no_ext]));

        assert!(out.entries.is_empty());
        assert_eq!(out.summary.video_count, 0);
        assert_eq!(out.summary.files_seen, 2);
        assert_eq!(out.warnings.len(), 2);
        assert!(
            out.warnings
                .iter()
                .all(|w| w.contains("فرمت ویدیویی پشتیبانی نمی‌شود")),
            "{:?}",
            out.warnings
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn selected_missing_paths_are_rejected() {
        let root = unique_temp_dir("sel-missing");
        std::fs::create_dir_all(&root).unwrap();
        let missing_file = root.join("nope.mkv");
        let missing_parent = root.join("ghost-dir").join("x.mkv");

        let out = run_selected(&sel(&[missing_file, missing_parent]));

        assert!(out.entries.is_empty());
        assert_eq!(out.warnings.len(), 2);
        assert!(out
            .warnings
            .iter()
            .any(|w| w.contains("فایل روی دیسک پیدا نشد")));
        assert!(out
            .warnings
            .iter()
            .any(|w| w.contains("مسیر روی دیسک پیدا نشد")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn selected_directory_is_rejected_as_file() {
        let root = unique_temp_dir("sel-dir");
        let dir = root.join("Season 01");
        std::fs::create_dir_all(&dir).unwrap();

        let out = run_selected(&sel(&[dir]));

        assert!(out.entries.is_empty());
        assert_eq!(out.warnings.len(), 1);
        assert!(out.warnings[0].contains("پوشه است"), "{:?}", out.warnings);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn selected_relative_and_empty_paths_are_rejected() {
        let out = run_selected(&["some/relative/video.mkv".to_string(), "   ".to_string()]);
        assert!(out.entries.is_empty());
        assert_eq!(out.warnings.len(), 2);
        assert!(out.warnings.iter().any(|w| w.contains("مسیر مطلق نیست")));
        assert!(out.warnings.iter().any(|w| w.contains("مسیر خالی است")));
    }

    #[test]
    fn selected_duplicate_paths_are_rejected_case_insensitively() {
        let root = unique_temp_dir("sel-dup");
        let video = mkv(&root, "Show.S01E01.mkv");
        let lower = video.to_string_lossy().into_owned();
        // ویندوز به بزرگی/کوچکی حساس نیست؛ در سایر پلتفرم‌ها همان متن عیناً تکرار می‌شود
        let variant = if cfg!(windows) {
            lower.to_uppercase()
        } else {
            lower.clone()
        };

        let out = run_selected(&[lower, variant]);

        assert_eq!(out.entries.len(), 1, "{:?}", out.warnings);
        assert_eq!(out.summary.video_count, 1);
        assert_eq!(out.summary.files_seen, 2);
        assert_eq!(out.warnings.len(), 1);
        assert!(
            out.warnings[0].contains("مسیر تکراری است"),
            "{:?}",
            out.warnings
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn selected_staging_path_is_rejected() {
        let root = unique_temp_dir("sel-staging");
        let staged = mkv(&root.join("_SubSource_Batch_abc"), "vid.mkv");

        let out = run_selected(&sel(&[staged]));

        assert!(out.entries.is_empty());
        assert_eq!(out.warnings.len(), 1);
        assert!(out.warnings[0].contains("staging"), "{:?}", out.warnings);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn selected_protected_system_paths_are_rejected() {
        let root = unique_temp_dir("sel-protected");
        let recycle = mkv(&root.join("$Recycle.Bin"), "v1.mkv");
        let svinfo = mkv(&root.join("System Volume Information"), "v2.mkv");

        let out = run_selected(&sel(&[recycle, svinfo]));

        assert!(out.entries.is_empty());
        assert_eq!(out.warnings.len(), 2);
        assert!(
            out.warnings.iter().all(|w| w.contains("سیستمی محافظت‌شده")),
            "{:?}",
            out.warnings
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn selected_junction_and_symlink_are_rejected() {
        let root = unique_temp_dir("sel-link");
        let real = root.join("real");
        std::fs::create_dir_all(&real).unwrap();
        touch(&real, "v.mkv", "");

        // Junction نیازمند دسترسی ویژه نیست و Reparse Point است
        let junction = root.join("junction");
        let out = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&real)
            .output()
            .expect("اجرای mklink");
        if out.status.success() {
            let via_junction = junction.join("v.mkv");
            let rejected = run_selected(&sel(&[via_junction]));
            assert!(rejected.entries.is_empty());
            assert!(
                rejected
                    .warnings
                    .iter()
                    .any(|w| w.contains("Reparse") || w.contains("پیوند نمادین")),
                "{:?}",
                rejected.warnings
            );
        } else {
            eprintln!("mklink /J در این محیط پشتیبانی نشد");
        }

        // پیوند نمادین فایل به دسترسی ویژه نیاز دارد؛ در نبودش فقط گزارش می‌شود
        let link = root.join("link.mkv");
        match std::os::windows::fs::symlink_file(real.join("v.mkv"), &link) {
            Ok(()) => {
                let rejected = run_selected(&sel(&[link]));
                assert!(rejected.entries.is_empty());
                assert!(
                    rejected
                        .warnings
                        .iter()
                        .any(|w| w.contains("Reparse") || w.contains("پیوند نمادین")),
                    "{:?}",
                    rejected.warnings
                );
            }
            Err(e) => eprintln!("symlink_file در این محیط پشتیبانی نشد: {e}"),
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(not(windows))]
    #[test]
    fn selected_symlink_is_rejected() {
        let root = unique_temp_dir("sel-link");
        std::fs::create_dir_all(&root).unwrap();
        touch(&root, "v.mkv", "");
        let link = root.join("link.mkv");
        std::os::unix::fs::symlink(root.join("v.mkv"), &link).expect("symlink");

        let out = run_selected(&sel(&[link]));
        assert!(out.entries.is_empty());
        assert!(out.warnings.iter().any(|w| w.contains("پیوند نمادین")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn selected_scan_matches_folder_scan_for_titles_and_siblings() {
        let root = unique_temp_dir("sel-parity");
        make_tree(&root);
        let cancel = AtomicBool::new(false);
        let excluded = vec!["Ted Lasso".to_string()];
        let mut no_progress = |_p: ScanProgress| {};

        let folder = scan_folder(&root, &excluded, &cancel, &mut no_progress);
        let dexter = root.join("Dexter.S04E01.720p.mkv");
        let other = root.join("Other Show").join("Season 02").join("S02E05.mkv");
        let movie = root.join("Movies").join("Movie.2023.1080p.mkv");
        let guarded = root.join("G").join("Show.S01E01.mkv");
        let selected = scan_selected_files(
            &sel(&[
                dexter.clone(),
                other.clone(),
                movie.clone(),
                guarded.clone(),
            ]),
            &excluded,
            &cancel,
            &mut no_progress,
        );

        assert_eq!(selected.entries.len(), 4);
        // هر ورودی باید با خروجی اسکن پوشه برای همان فایل عیناً برابر باشد
        for path in [&dexter, &other, &movie, &guarded] {
            let want = path.to_string_lossy();
            let from_folder = folder
                .entries
                .iter()
                .find(|e| e.path.as_str() == want.as_ref())
                .unwrap_or_else(|| panic!("folder scan باید {} را می‌یافت", path.display()));
            let from_selected = selected
                .entries
                .iter()
                .find(|e| e.path.as_str() == want.as_ref())
                .unwrap_or_else(|| panic!("selected scan باید {} را می‌یافت", path.display()));
            assert_eq!(from_folder, from_selected, "{}", path.display());
        }
        // گارد E010 در هر دو مسیر یکسان است
        let guarded_entry = selected
            .entries
            .iter()
            .find(|e| e.file_name == "Show.S01E01.mkv")
            .unwrap();
        assert!(guarded_entry.sibling_subtitles.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn selected_scan_cancellation_keeps_partial_results() {
        let root = unique_temp_dir("sel-cancel");
        let a = mkv(&root, "a.mkv");
        let b = mkv(&root, "b.mkv");
        let cancel = AtomicBool::new(false);
        let paths = sel(&[a.clone(), b]);

        let mut progress_calls = 0u32;
        let mut on_progress = |_p: ScanProgress| {
            progress_calls += 1;
            if progress_calls == 1 {
                // پس از پردازش اولین فایل، لغو درخواست می‌شود
                cancel.store(true, Ordering::Relaxed);
            }
        };

        let out = scan_selected_files(&paths, &[], &cancel, &mut on_progress);

        assert!(out.summary.cancelled);
        assert_eq!(out.entries.len(), 1);
        assert_eq!(out.entries[0].path, a.to_string_lossy().to_string());
        assert_eq!(out.summary.video_count, 1);
        // مسیر دوم پردازش نشده و شمرده نشده است
        assert_eq!(out.summary.files_seen, 1);
        // هیچ تغییری روی دیسک لازم نیست — فقط ورودی معتبرِ پیش از لغو حفظ شده

        let _ = std::fs::remove_dir_all(&root);
    }
}
