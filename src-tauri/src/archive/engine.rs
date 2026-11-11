//! موتور پلن/اجرای استخراج آرشیو.
//!
//! پلن: بازرسی امن همهٔ اعضا → استخراج در پوشهٔ موقت تحت مالکیت برنامه →
//! اعتبارسنجی ساختار/Encoding/محتوای فارسی → انتخاب/ابهام → پاک‌سازی موقت.
//!
//! اجرا: **همه‌چیز دوباره اعتبارسنجی می‌شود** (هرگز به دادهٔ پلن فرانت اعتماد
//! نمی‌شود)، سپس آرشیو انتخاب‌شده دوباره استخراج، محتوا دوباره اعتبارسنجی و
//! در نهایت با `create_new` و بدون بازنویسی جای‌گذاری می‌شود.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::models::OpsProgress;
use crate::scanner::naming::{file_extension, is_video_file};
use crate::subtitle::validation::validate_bytes_for_extension;

use super::detect::{discover_archives, DiscoveredArchive};
use super::listing::{
    is_link_or_reparse, validate_member_path, validate_members, verify_extracted_tree, SafeMember,
};
use super::matching::{member_matches_video, quality_of, sort_candidates, video_key, VideoKey};
use super::placement::{output_path, place_subtitle};
use super::tool::{
    extract_archive, list_archive, validate_staging_root, ExtractWorkspace, ToolError,
};
use super::types::{
    ArchiveExecuteItemResult, ArchiveExecuteRequest, ArchiveExecuteResult, ArchiveExtractionPlan,
    ArchiveFileInfo, ArchiveItemStatus, ArchivePlanItem, ArchivePlanStatus, ArchiveSelection,
    ArchiveSource, SubtitleCandidate, MAX_SUBTITLE_BYTES,
};

struct ValidSource {
    staging_root: PathBuf,
    archive_dir: PathBuf,
}

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

fn path_key(value: &str) -> String {
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value.to_string()
    }
}

fn join_relative(base: &Path, rel: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        out.push(part);
    }
    out
}

fn validate_relative_dir(raw: &str) -> Result<String, String> {
    if raw.is_empty() {
        return Ok(String::new());
    }
    if raw.trim().is_empty() {
        return Err("پوشهٔ نسبی ویدیو در staging نامعتبر است".into());
    }
    let normalized = normalize_sep(raw);
    validate_member_path(&normalized)
}

/// اعتبارسنجی منبع پیش از هر کاری: ویدیوی اصلی، ریشهٔ staging، زنجیرهٔ
/// پوشه‌های نسبی و پوشهٔ `sub`. هر symlink/reparse در مسیر رد می‌شود.
fn validate_source(source: &ArchiveSource) -> Result<ValidSource, String> {
    let video = Path::new(&source.video_path);
    if !video.is_absolute() {
        return Err(format!(
            "مسیر ویدیوی «{}» مطلق نیست — منبع رد شد",
            source.video_path
        ));
    }
    let video_meta = fs::metadata(video)
        .map_err(|e| format!("ویدیوی اصلی «{}» قابل دسترسی نیست: {e}", source.video_path))?;
    if !video_meta.is_file() {
        return Err(format!(
            "«{}» یک فایل ویدیویی معمولی نیست",
            source.video_path
        ));
    }
    if !is_video_file(&source.video_file_name) {
        return Err(format!(
            "پسوند ویدیوی «{}» پشتیبانی نمی‌شود — منبع رد شد",
            source.video_file_name
        ));
    }
    let actual_name = video
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| "نام فایل ویدیوی اصلی نامعتبر است".to_string())?;
    if !same_path_text(&actual_name, &source.video_file_name) {
        return Err(format!(
            "نام «{}» با مسیر ویدیوی «{}» هم‌خوان نیست — منبع رد شد",
            source.video_file_name, source.video_path
        ));
    }
    let safe_video_name = validate_member_path(&source.video_file_name)?;
    if safe_video_name.contains('/') {
        return Err("نام فایل ویدیو نباید جداکنندهٔ مسیر داشته باشد".into());
    }

    let staging_root =
        validate_staging_root(Path::new(&source.staging_root)).map_err(|e| e.to_string())?;
    let rel_dir = validate_relative_dir(&source.relative_video_dir)?;
    let expected_rel = if rel_dir.is_empty() {
        safe_video_name
    } else {
        format!("{rel_dir}/{safe_video_name}")
    };
    if !same_path_text(&normalize_sep(&source.relative_video_path), &expected_rel) {
        return Err(format!(
            "مسیر نسبی staging «{}» با پوشه/نام فایل هم‌خوان نیست — منبع رد شد",
            source.relative_video_path
        ));
    }

    let mut current = staging_root.clone();
    for part in rel_dir
        .split('/')
        .filter(|p| !p.is_empty())
        .chain(std::iter::once("sub"))
    {
        current.push(part);
        let meta = fs::symlink_metadata(&current).map_err(|e| {
            format!(
                "پوشهٔ «{}» در staging قابل دسترسی نیست: {e}",
                current.display()
            )
        })?;
        if is_link_or_reparse(&meta) {
            return Err(format!(
                "پوشهٔ «{}» symlink/reparse است — رد شد",
                current.display()
            ));
        }
        if !meta.is_dir() {
            return Err(format!("«{}» پوشه نیست", current.display()));
        }
    }
    let archive_dir = fs::canonicalize(&current).map_err(|e| {
        format!(
            "canonicalize پوشهٔ آرشیو «{}» ناموفق: {e}",
            current.display()
        )
    })?;
    if !archive_dir.starts_with(&staging_root) {
        return Err("پوشهٔ آرشیو از ریشهٔ staging خارج شده است — رد شد".into());
    }

    // ویدیوی اصلی هرگز نباید داخل staging باشد (محافظ در برابر payload دستکاری‌شده).
    if let Ok(canonical_video) = fs::canonicalize(video) {
        if canonical_video.starts_with(&staging_root) {
            return Err("ویدیوی اصلی داخل ریشهٔ staging است — منبع رد شد".into());
        }
    }

    Ok(ValidSource {
        staging_root,
        archive_dir,
    })
}

fn progress(message: &str, current: u64, total: u64, video_path: &str) -> OpsProgress {
    OpsProgress {
        kind: "archive".into(),
        current,
        total,
        video_path: video_path.into(),
        message: message.into(),
    }
}

fn summarize_warnings(warnings: &[String]) -> String {
    let mut out = warnings
        .iter()
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join(" | ");
    if warnings.len() > 3 {
        out.push_str(&format!(" | و {} هشدار دیگر", warnings.len() - 3));
    }
    out
}

fn with_warnings(base: &str, warnings: &[String]) -> String {
    if warnings.is_empty() {
        base.to_string()
    } else {
        format!("{base} — {}", summarize_warnings(warnings))
    }
}

#[derive(Default)]
struct ArchiveInspection {
    candidates: Vec<SubtitleCandidate>,
    warnings: Vec<String>,
    fatal_error: Option<String>,
    cancelled: bool,
    cleanup_error: Option<String>,
}

/// بازرسی یک آرشیو: فهرست و اعتبارسنجی امنیتی → استخراج → اعتبارسنجی
/// کامل همهٔ کاندیدهای نام‌تطبیق. پوشهٔ موقت در همهٔ مسیرها پاک می‌شود.
fn inspect_archive(
    seven_zip: &Path,
    staging_root: &Path,
    archive: &DiscoveredArchive,
    key: &VideoKey,
    cancel: &AtomicBool,
) -> ArchiveInspection {
    let mut workspace = match ExtractWorkspace::create(staging_root) {
        Ok(w) => w,
        Err(ToolError::Cancelled) => {
            return ArchiveInspection {
                cancelled: true,
                ..Default::default()
            };
        }
        Err(ToolError::Failed(e)) => {
            return ArchiveInspection {
                fatal_error: Some(e),
                ..Default::default()
            };
        }
    };
    let out_dir = workspace.out_dir().to_path_buf();
    let log_dir = workspace.log_dir().to_path_buf();

    let inner = (|| -> Result<(Vec<SubtitleCandidate>, Vec<String>), ToolError> {
        let mut warnings = Vec::new();
        let (members, mut list_warnings) =
            list_archive(seven_zip, &archive.path, &log_dir, cancel)?;
        warnings.append(&mut list_warnings);

        let safe = validate_members(&members).map_err(ToolError::Failed)?;
        let matching: Vec<&SafeMember> = safe
            .iter()
            .filter(|m| !m.is_dir && member_matches_video(key, &m.normalized))
            .collect();
        if matching.is_empty() {
            return Ok((Vec::new(), warnings));
        }

        let mut eligible = Vec::new();
        for member in matching {
            if member.size > MAX_SUBTITLE_BYTES {
                warnings.push(format!(
                    "«{}» بزرگ‌تر از حد مجاز زیرنویس است و نادیده گرفته شد",
                    member.normalized
                ));
            } else {
                eligible.push(member);
            }
        }
        if eligible.is_empty() {
            return Ok((Vec::new(), warnings));
        }

        let mut extract_warnings =
            extract_archive(seven_zip, &archive.path, &out_dir, &log_dir, cancel)?;
        warnings.append(&mut extract_warnings);
        verify_extracted_tree(&out_dir).map_err(ToolError::Failed)?;
        let canonical_root = fs::canonicalize(&out_dir).map_err(|e| {
            ToolError::Failed(format!(
                "canonicalize ریشهٔ استخراج «{}» ناموفق: {e}",
                out_dir.display()
            ))
        })?;

        let mut candidates = Vec::new();
        for member in eligible {
            match candidate_from_extracted(&canonical_root, member, &archive.file_name) {
                Ok(candidate) => candidates.push(candidate),
                Err(reason) => warnings.push(reason),
            }
        }
        sort_candidates(&mut candidates);
        Ok((candidates, warnings))
    })();

    let cleanup_error = workspace.cleanup().err();
    match inner {
        Ok((candidates, warnings)) => ArchiveInspection {
            candidates,
            warnings,
            fatal_error: None,
            cancelled: false,
            cleanup_error,
        },
        Err(ToolError::Cancelled) => ArchiveInspection {
            cancelled: true,
            cleanup_error,
            ..Default::default()
        },
        Err(ToolError::Failed(e)) => ArchiveInspection {
            fatal_error: Some(e),
            cleanup_error,
            ..Default::default()
        },
    }
}

fn candidate_from_extracted(
    canonical_root: &Path,
    member: &SafeMember,
    archive_file_name: &str,
) -> Result<SubtitleCandidate, String> {
    let path = canonical_root.join(member.relative_path());
    let canonical = fs::canonicalize(&path).map_err(|e| {
        format!(
            "زیرنویس استخراج‌شده «{}» قابل دسترسی نیست: {e}",
            member.normalized
        )
    })?;
    if !canonical.starts_with(canonical_root) {
        return Err(format!(
            "«{}» از ریشهٔ استخراج خارج شده است — رد شد",
            member.normalized
        ));
    }
    let meta = fs::symlink_metadata(&canonical)
        .map_err(|e| format!("متادیتای «{}» خوانده نشد: {e}", member.normalized))?;
    if !meta.is_file() || is_link_or_reparse(&meta) {
        return Err(format!("«{}» فایل معمولی نیست — رد شد", member.normalized));
    }
    if meta.len() == 0 {
        return Err(format!("«{}» خالی است — رد شد", member.normalized));
    }
    if meta.len() > MAX_SUBTITLE_BYTES {
        return Err(format!(
            "«{}» بزرگ‌تر از حد مجاز زیرنویس است — رد شد",
            member.normalized
        ));
    }

    let ext = file_extension(member.file_name());
    let bytes = fs::read(&canonical)
        .map_err(|e| format!("خواندن «{}» ناموفق بود: {e}", member.normalized))?;
    let validation = validate_bytes_for_extension(&bytes, &ext);
    if !validation.valid {
        return Err(format!(
            "«{}» زیرنویس معتبر نیست: {}",
            member.normalized,
            validation.error.unwrap_or_else(|| "خطای نامشخص".into())
        ));
    }
    if !validation.has_persian || validation.persian_char_count == 0 {
        return Err(format!(
            "«{}» محتوای فارسی ندارد — به‌عنوان زیرنویس فارسی پذیرفته نشد",
            member.normalized
        ));
    }

    Ok(SubtitleCandidate {
        archive_file_name: archive_file_name.to_string(),
        member_path: member.normalized.clone(),
        subtitle_extension: ext,
        size_bytes: meta.len(),
        quality: quality_of(archive_file_name, &member.normalized).to_string(),
    })
}

struct Inspection {
    plan: ArchivePlanItem,
    warnings: Vec<String>,
    cleanup_warnings: Vec<String>,
    cancelled: bool,
    /// حداقل یک خطای جدی (منبع نامعتبر، آرشیو خراب، کد خروج ۲+) رخ داده است.
    had_fatal: bool,
}

fn inspect_source(seven_zip: &Path, source: &ArchiveSource, cancel: &AtomicBool) -> Inspection {
    let fallback_dir = join_relative(
        &join_relative(
            Path::new(&source.staging_root),
            &normalize_sep(&source.relative_video_dir),
        ),
        "sub",
    );
    let valid = match validate_source(source) {
        Ok(v) => v,
        Err(e) => {
            return Inspection {
                plan: ArchivePlanItem::empty(source, fallback_dir.to_string_lossy().to_string(), e),
                warnings: Vec::new(),
                cleanup_warnings: Vec::new(),
                cancelled: false,
                had_fatal: true,
            };
        }
    };

    let archive_dir = valid.archive_dir.to_string_lossy().to_string();
    let (archives, discover_warnings) = discover_archives(&valid.archive_dir);
    let mut item = ArchivePlanItem {
        video_path: source.video_path.clone(),
        video_file_name: source.video_file_name.clone(),
        staging_root: source.staging_root.clone(),
        relative_video_dir: source.relative_video_dir.clone(),
        archive_dir,
        archives: archives
            .iter()
            .map(|a| ArchiveFileInfo {
                file_name: a.file_name.clone(),
                format: a.format.as_str().to_string(),
                size_bytes: a.size_bytes,
            })
            .collect(),
        candidates: Vec::new(),
        status: ArchivePlanStatus::NoMatch,
        selection: None,
        message: String::new(),
    };
    let mut warnings = discover_warnings;
    let mut cleanup_warnings = Vec::new();
    let mut cancelled = false;
    let mut had_fatal = false;

    if archives.is_empty() {
        item.message = with_warnings("هیچ آرشیو ZIP/RAR معتبری در پوشهٔ sub پیدا نشد", &warnings);
        return Inspection {
            plan: item,
            warnings,
            cleanup_warnings,
            cancelled,
            had_fatal: false,
        };
    }

    let key = video_key(&source.video_file_name, source.season, source.episode);
    let mut candidates = Vec::new();
    for archive in &archives {
        if cancel.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }
        let inspection = inspect_archive(seven_zip, &valid.staging_root, archive, &key, cancel);
        warnings.extend(inspection.warnings);
        if let Some(e) = inspection.fatal_error {
            had_fatal = true;
            warnings.push(format!("آرشیو «{}»: {e}", archive.file_name));
        }
        if let Some(e) = inspection.cleanup_error {
            cleanup_warnings.push(e);
        }
        candidates.extend(inspection.candidates);
        if inspection.cancelled {
            cancelled = true;
            break;
        }
    }

    sort_candidates(&mut candidates);
    item.status = match candidates.len() {
        0 => ArchivePlanStatus::NoMatch,
        1 => ArchivePlanStatus::Ready,
        _ => ArchivePlanStatus::Ambiguous,
    };
    item.selection = match item.status {
        ArchivePlanStatus::Ready => candidates.first().cloned(),
        _ => None,
    };
    let base = match item.status {
        ArchivePlanStatus::Ready => "دقیقاً یک زیرنویس معتبر پیدا شد — آمادهٔ جای‌گذاری".to_string(),
        ArchivePlanStatus::Ambiguous => format!(
            "{} کاندید معتبر پیدا شد — انتخاب صریح لازم است",
            candidates.len()
        ),
        ArchivePlanStatus::NoMatch => "هیچ زیرنویس معتبر و منطبقی در آرشیوها پیدا نشد".to_string(),
    };
    item.message = with_warnings(&base, &warnings);
    item.candidates = candidates;

    Inspection {
        plan: item,
        warnings,
        cleanup_warnings,
        cancelled,
        had_fatal,
    }
}

/// پلن کامل — هیچ فایل دائمی روی دیسک تغییر نمی‌کند.
pub fn plan_archives(
    seven_zip: &Path,
    sources: &[ArchiveSource],
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(OpsProgress),
) -> ArchiveExtractionPlan {
    let mut plan = ArchiveExtractionPlan::default();
    let mut seen = HashSet::new();
    let total = sources.len() as u64;

    for (idx, source) in sources.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            plan.cancelled = true;
            plan.warnings
                .push("پلن به‌دلیل لغو پیش از بررسی همهٔ منابع متوقف شد.".into());
            break;
        }
        if !seen.insert(path_key(&source.video_path)) {
            plan.warnings.push(format!(
                "منبع تکراری «{}» نادیده گرفته شد",
                source.video_file_name
            ));
            continue;
        }

        on_progress(progress(
            &format!("بررسی آرشیوهای «{}»", source.video_file_name),
            idx as u64,
            total,
            &source.video_path,
        ));
        let inspection = inspect_source(seven_zip, source, cancel);
        plan.warnings.extend(inspection.cleanup_warnings);
        match inspection.plan.status {
            ArchivePlanStatus::Ready => plan.ready += 1,
            ArchivePlanStatus::Ambiguous => plan.ambiguous += 1,
            ArchivePlanStatus::NoMatch => plan.no_match += 1,
        }
        let message = inspection.plan.message.clone();
        plan.items.push(inspection.plan);

        on_progress(progress(
            &format!("«{}»: {}", source.video_file_name, message),
            idx as u64 + 1,
            total,
            &source.video_path,
        ));
        if inspection.cancelled {
            plan.cancelled = true;
            plan.warnings
                .push("پلن به‌دلیل لغو در میانهٔ استخراج متوقف شد؛ پوشه‌های موقت پاک شدند.".into());
            break;
        }
    }

    plan
}

#[derive(Debug)]
enum ChoiceFailure {
    NoMatch(String),
    Ambiguous(String),
    Failed(String),
}

fn choose_candidate(
    candidates: &[SubtitleCandidate],
    selection: Option<&ArchiveSelection>,
) -> Result<SubtitleCandidate, ChoiceFailure> {
    if let Some(selection) = selection {
        let found = candidates.iter().find(|c| {
            same_path_text(&c.archive_file_name, &selection.archive_file_name)
                && same_path_text(&c.member_path, &selection.member_path)
        });
        return match found {
            Some(c) => Ok(c.clone()),
            None => Err(ChoiceFailure::Failed(format!(
                "انتخاب «{}/{}» دیگر یک کاندید معتبر نیست — پلن را دوباره بسازید",
                selection.archive_file_name, selection.member_path
            ))),
        };
    }

    match candidates.len() {
        0 => Err(ChoiceFailure::NoMatch(
            "هیچ زیرنویس معتبر و منطبقی پیدا نشد".into(),
        )),
        1 => Ok(candidates[0].clone()),
        n => Err(ChoiceFailure::Ambiguous(format!(
            "{n} کاندید معتبر وجود دارد؛ انتخاب صریح لازم است: {}",
            candidates
                .iter()
                .take(5)
                .map(|c| format!("{}::{}", c.archive_file_name, c.member_path))
                .collect::<Vec<_>>()
                .join(" | ")
        ))),
    }
}

enum PlaceFailure {
    Cancelled,
    Failed(String),
}

struct PlaceAttempt {
    outcome: Result<PathBuf, PlaceFailure>,
    warnings: Vec<String>,
    cleanup_error: Option<String>,
}

fn map_tool_error(error: ToolError) -> PlaceFailure {
    match error {
        ToolError::Cancelled => PlaceFailure::Cancelled,
        ToolError::Failed(e) => PlaceFailure::Failed(e),
    }
}

/// استخراج مجدد آرشیو انتخاب‌شده + اعتبارسنجی مجدد + جای‌گذاری بدون بازنویسی.
fn place_selected(
    seven_zip: &Path,
    source: &ArchiveSource,
    chosen: &SubtitleCandidate,
    cancel: &AtomicBool,
) -> PlaceAttempt {
    let mut warnings = Vec::new();
    let valid = match validate_source(source) {
        Ok(v) => v,
        Err(e) => {
            return PlaceAttempt {
                outcome: Err(PlaceFailure::Failed(e)),
                warnings,
                cleanup_error: None,
            };
        }
    };
    let (archives, _) = discover_archives(&valid.archive_dir);
    let Some(archive) = archives
        .iter()
        .find(|a| same_path_text(&a.file_name, &chosen.archive_file_name))
    else {
        return PlaceAttempt {
            outcome: Err(PlaceFailure::Failed(format!(
                "آرشیو انتخابی «{}» دیگر در پوشهٔ sub وجود ندارد — پلن را دوباره بسازید",
                chosen.archive_file_name
            ))),
            warnings,
            cleanup_error: None,
        };
    };

    let key = video_key(&source.video_file_name, source.season, source.episode);
    if !member_matches_video(&key, &chosen.member_path) {
        return PlaceAttempt {
            outcome: Err(PlaceFailure::Failed(
                "زیرنویس انتخابی با قسمت این ویدیو تطبیق ندارد — رد شد".into(),
            )),
            warnings,
            cleanup_error: None,
        };
    }

    let mut workspace = match ExtractWorkspace::create(&valid.staging_root) {
        Ok(w) => w,
        Err(ToolError::Cancelled) => {
            return PlaceAttempt {
                outcome: Err(PlaceFailure::Cancelled),
                warnings,
                cleanup_error: None,
            };
        }
        Err(ToolError::Failed(e)) => {
            return PlaceAttempt {
                outcome: Err(PlaceFailure::Failed(e)),
                warnings,
                cleanup_error: None,
            };
        }
    };
    let out_dir = workspace.out_dir().to_path_buf();
    let log_dir = workspace.log_dir().to_path_buf();

    let attempt = (|| -> Result<PathBuf, PlaceFailure> {
        if cancel.load(Ordering::SeqCst) {
            return Err(PlaceFailure::Cancelled);
        }
        let (members, mut list_warnings) =
            list_archive(seven_zip, &archive.path, &log_dir, cancel).map_err(map_tool_error)?;
        warnings.append(&mut list_warnings);
        let safe = validate_members(&members).map_err(PlaceFailure::Failed)?;
        let member = safe
            .iter()
            .find(|m| !m.is_dir && same_path_text(&m.normalized, &chosen.member_path))
            .ok_or_else(|| {
                PlaceFailure::Failed(format!(
                    "عضو «{}» دیگر در آرشیو وجود ندارد — پلن را دوباره بسازید",
                    chosen.member_path
                ))
            })?;
        if !member_matches_video(&key, &member.normalized) {
            return Err(PlaceFailure::Failed(
                "عضو انتخاب‌شده با قسمت این ویدیو تطبیق ندارد".into(),
            ));
        }

        let mut extract_warnings =
            extract_archive(seven_zip, &archive.path, &out_dir, &log_dir, cancel)
                .map_err(map_tool_error)?;
        warnings.append(&mut extract_warnings);
        verify_extracted_tree(&out_dir).map_err(PlaceFailure::Failed)?;
        let canonical_root = fs::canonicalize(&out_dir).map_err(|e| {
            PlaceFailure::Failed(format!(
                "canonicalize ریشهٔ استخراج «{}» ناموفق: {e}",
                out_dir.display()
            ))
        })?;

        // اعتبارسنجی مجدد محتوا — انتخاب کاربر هرگز جای بررسی محتوا را نمی‌گیرد.
        let candidate = candidate_from_extracted(&canonical_root, member, &archive.file_name)
            .map_err(PlaceFailure::Failed)?;
        let extracted = canonical_root.join(member.relative_path());
        let target = output_path(Path::new(&source.video_path), &candidate.subtitle_extension)
            .map_err(|e| PlaceFailure::Failed(e.to_string()))?;
        place_subtitle(&extracted, &target).map_err(|e| PlaceFailure::Failed(e.to_string()))?;
        Ok(target)
    })();

    let cleanup_error = workspace.cleanup().err();
    PlaceAttempt {
        outcome: attempt,
        warnings,
        cleanup_error,
    }
}

fn cancelled_item(source: &ArchiveSource) -> ArchiveExecuteItemResult {
    ArchiveExecuteItemResult {
        video_path: source.video_path.clone(),
        video_file_name: source.video_file_name.clone(),
        status: ArchiveItemStatus::Cancelled,
        output_path: None,
        warnings: Vec::new(),
        message: "عملیات پیش از پردازش این مورد لغو شد.".into(),
    }
}

/// اجرای استخراج با اعتبارسنجی کامل مجدد و جای‌گذاری بدون بازنویسی.
pub fn execute_archive_extraction(
    seven_zip: &Path,
    requests: &[ArchiveExecuteRequest],
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(OpsProgress),
) -> ArchiveExecuteResult {
    let start = Instant::now();
    let mut result = ArchiveExecuteResult::default();
    let total = requests.len() as u64;
    let mut seen = HashSet::new();

    for (idx, request) in requests.iter().enumerate() {
        let source = &request.source;
        if cancel.load(Ordering::SeqCst) {
            result.cancelled = true;
            result.items.push(cancelled_item(source));
            break;
        }

        on_progress(progress(
            &format!("بررسی مجدد آرشیوهای «{}»", source.video_file_name),
            idx as u64,
            total,
            &source.video_path,
        ));

        if !seen.insert(path_key(&source.video_path)) {
            result.items.push(ArchiveExecuteItemResult {
                video_path: source.video_path.clone(),
                video_file_name: source.video_file_name.clone(),
                status: ArchiveItemStatus::Failed,
                output_path: None,
                warnings: Vec::new(),
                message: "درخواست تکراری برای همین ویدیو رد شد.".into(),
            });
            continue;
        }

        let inspection = inspect_source(seven_zip, source, cancel);
        result
            .cleanup_warnings
            .extend(inspection.cleanup_warnings.iter().cloned());
        result.warnings.extend(inspection.warnings.iter().cloned());

        if inspection.cancelled || cancel.load(Ordering::SeqCst) {
            result.cancelled = true;
            result.items.push(cancelled_item(source));
            break;
        }

        let plan_item = inspection.plan;
        if plan_item.candidates.is_empty() && inspection.had_fatal {
            result.items.push(ArchiveExecuteItemResult {
                video_path: source.video_path.clone(),
                video_file_name: source.video_file_name.clone(),
                status: ArchiveItemStatus::Failed,
                output_path: None,
                warnings: Vec::new(),
                message: plan_item.message,
            });
            continue;
        }
        let choice = choose_candidate(&plan_item.candidates, request.selection.as_ref());
        let chosen = match choice {
            Ok(c) => c,
            Err(ChoiceFailure::NoMatch(message)) => {
                result.items.push(ArchiveExecuteItemResult {
                    video_path: source.video_path.clone(),
                    video_file_name: source.video_file_name.clone(),
                    status: ArchiveItemStatus::NoMatch,
                    output_path: None,
                    warnings: Vec::new(),
                    message: with_warnings(&message, &inspection.warnings),
                });
                continue;
            }
            Err(ChoiceFailure::Ambiguous(message)) => {
                result.items.push(ArchiveExecuteItemResult {
                    video_path: source.video_path.clone(),
                    video_file_name: source.video_file_name.clone(),
                    status: ArchiveItemStatus::Ambiguous,
                    output_path: None,
                    warnings: Vec::new(),
                    message,
                });
                continue;
            }
            Err(ChoiceFailure::Failed(message)) => {
                result.items.push(ArchiveExecuteItemResult {
                    video_path: source.video_path.clone(),
                    video_file_name: source.video_file_name.clone(),
                    status: ArchiveItemStatus::Failed,
                    output_path: None,
                    warnings: Vec::new(),
                    message,
                });
                continue;
            }
        };

        on_progress(progress(
            &format!(
                "استخراج و جای‌گذاری «{}» برای «{}»",
                chosen.member_path, source.video_file_name
            ),
            idx as u64,
            total,
            &source.video_path,
        ));
        let attempt = place_selected(seven_zip, source, &chosen, cancel);
        result.warnings.extend(attempt.warnings.iter().cloned());
        if let Some(e) = attempt.cleanup_error {
            result.cleanup_warnings.push(e);
        }

        let item = match attempt.outcome {
            Ok(output) => ArchiveExecuteItemResult {
                video_path: source.video_path.clone(),
                video_file_name: source.video_file_name.clone(),
                status: ArchiveItemStatus::Placed,
                output_path: Some(output.to_string_lossy().to_string()),
                warnings: attempt.warnings,
                message: "زیرنویس با موفقیت کنار ویدیو قرار گرفت.".into(),
            },
            Err(PlaceFailure::Cancelled) => {
                result.cancelled = true;
                canceled_with_warnings(source, attempt.warnings)
            }
            Err(PlaceFailure::Failed(message)) => ArchiveExecuteItemResult {
                video_path: source.video_path.clone(),
                video_file_name: source.video_file_name.clone(),
                status: ArchiveItemStatus::Failed,
                output_path: None,
                warnings: attempt.warnings,
                message,
            },
        };
        let is_cancelled = item.status == ArchiveItemStatus::Cancelled;
        let status_text = item.message.clone();
        result.items.push(item);
        on_progress(progress(
            &format!("«{}»: {}", source.video_file_name, status_text),
            idx as u64 + 1,
            total,
            &source.video_path,
        ));
        if is_cancelled {
            break;
        }
    }

    result.recalculate();
    result.message = if result.cancelled {
        "استخراج لغو شد؛ خروجی نیمه‌کارهٔ همین عملیات پاک شد و آرشیوها دست‌نخورده ماندند.".into()
    } else if result.success {
        format!(
            "استخراج تمام شد: {} زیرنویس فارسی کنار ویدیوها قرار گرفت.",
            result.placed
        )
    } else {
        format!(
            "استخراج کامل نشد: {} جای‌گذاری، {} خطا، {} ابهام، {} بدون تطبیق.",
            result.placed, result.failed, result.ambiguous, result.no_match
        )
    };
    result.duration_secs = start.elapsed().as_secs_f64();
    result
}

fn canceled_with_warnings(
    source: &ArchiveSource,
    warnings: Vec<String>,
) -> ArchiveExecuteItemResult {
    ArchiveExecuteItemResult {
        video_path: source.video_path.clone(),
        video_file_name: source.video_file_name.clone(),
        status: ArchiveItemStatus::Cancelled,
        output_path: None,
        warnings,
        message: "عملیات در میانهٔ استخراج لغو شد؛ پوشهٔ موقت پاک شد.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(archive: &str, member: &str) -> SubtitleCandidate {
        SubtitleCandidate {
            archive_file_name: archive.into(),
            member_path: member.into(),
            subtitle_extension: "srt".into(),
            size_bytes: 10,
            quality: "other".into(),
        }
    }

    #[test]
    fn choice_requires_explicit_selection_for_multiple_candidates() {
        let candidates = vec![
            candidate("a.zip", "Show.S01E01.fa.srt"),
            candidate("b.zip", "x"),
        ];
        assert!(matches!(
            choose_candidate(&candidates, None),
            Err(ChoiceFailure::Ambiguous(_))
        ));
        assert!(matches!(
            choose_candidate(
                &candidates,
                Some(&ArchiveSelection {
                    archive_file_name: "missing.zip".into(),
                    member_path: "x".into(),
                })
            ),
            Err(ChoiceFailure::Failed(_))
        ));
        let selected = choose_candidate(
            &candidates,
            Some(&ArchiveSelection {
                archive_file_name: "B.ZIP".into(),
                member_path: "x".into(),
            }),
        )
        .unwrap();
        assert_eq!(selected.archive_file_name, "b.zip");
    }

    #[test]
    fn empty_candidates_is_no_match_and_single_is_auto() {
        assert!(matches!(
            choose_candidate(&[], None),
            Err(ChoiceFailure::NoMatch(_))
        ));
        let only = candidate("a.zip", "Show.S01E01.fa.srt");
        assert_eq!(
            choose_candidate(std::slice::from_ref(&only), None).unwrap(),
            only
        );
    }

    #[test]
    fn source_validation_rejects_outside_staging_and_bad_relative_path() {
        let base = std::env::temp_dir().join(format!(
            "subsync-archive-engine-{}-{}",
            std::process::id(),
            crate::subsource::staging::new_uid()
        ));
        let staging = base.join("_subsource_batch_x");
        let library = base.join("library");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir_all(&library).unwrap();
        let video = library.join("Show.S01E01.mkv");
        std::fs::write(&video, b"video").unwrap();

        let mut source = ArchiveSource {
            video_path: video.to_string_lossy().to_string(),
            video_file_name: "Show.S01E01.mkv".into(),
            staging_root: staging.to_string_lossy().to_string(),
            relative_video_path: "Show.S01E01.mkv".into(),
            relative_video_dir: "".into(),
            season: Some(1),
            episode: Some(1),
        };
        // پوشهٔ sub هنوز نیست
        assert!(validate_source(&source).is_err());

        std::fs::create_dir_all(staging.join("sub")).unwrap();
        assert!(validate_source(&source).is_ok());

        source.relative_video_path = "../escape.mkv".into();
        assert!(validate_source(&source).is_err());

        let _ = std::fs::remove_dir_all(&base);
    }
}
