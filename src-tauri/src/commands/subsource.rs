//! دانلود گروهی زیرنویس با SubSource — Staging + Batch Download (فاز ۳ PR ۲).
//!
//! جریان: پلن (بدون تغییر دیسک) → تأیید کاربر → اجرا:
//! ۱. API key قبل از هرگونه تغییر روی دیسک خوانده می‌شود (نبود → توقف).
//! ۲. پلن در لحظهٔ اجرا دوباره اعتبارسنجی می‌شود (هرگز به ورودی فرانت اعتماد نمی‌شود).
//! ۳. ویدیوها بر اساس volume گروه می‌شوند؛ هر گروه staging مستقل با hard link.
//! ۴. SubSource به‌ترتیب برای هر گروه اجرا می‌شود (adapter موجود + همان
//!    `CancellationToken` ثبت‌شده در `OpsRegistry`).
//! ۵. پس از پایان هر گروه فقط لینک‌های برنامه حذف می‌شوند؛ خروجی دانلود
//!    (آرشیوهای `sub/`) و ریشه staging برای فاز بعدی (Archive Extraction)
//!    حفظ می‌شود.
//!
//! درباره پیشرفت: SubSource CLI خروجی ساختاریافته/درصدی ندارد — فقط وضعیت
//! واقعی مراحل گزارش می‌شود و هیچ درصد ساختگی نمایش داده نمی‌شود.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::models::{OpsProgress, VideoEntry};
use crate::state::{acquire_batch_op, AppState};
use crate::subsource::plan::{
    create_download_plan, validate_video_typed, DownloadPlan, DownloadPlanItem, DownloadResult,
    SkipReasonCode, SkippedDownload, VideoSkipIssue,
};
use crate::subsource::staging::{self, VolumeGroup};
use crate::subsource::{
    retrieve_api_key, BatchOptions, CancellationToken, SubSourceCli, SubSourceError,
};

use super::media::{get_entry, resolve_tool};

/// رویداد پایان دانلود گروهی — payload آن `DownloadResult` کامل است تا فاز
/// بعدی (Archive Extraction) دقیقاً بداند کدام ریشه‌های staging خروجی دارند.
pub const SUBSOURCE_DONE_EVENT: &str = "subsource:done";

/// payload رویداد `subsource:done` — فقط نتیجهٔ ساختاریافتهٔ کامل.
/// این ساختار هیچ API key یا راز دیگری ندارد.
pub fn download_done_payload(result: &DownloadResult) -> serde_json::Value {
    serde_json::to_value(result).unwrap_or(serde_json::Value::Null)
}

/// رویداد پیشرفت مرحله‌دار — هیچ عدد ساختگی؛ current/total فقط برای شمارش
/// واقعی لینک‌ها معنا دارد.
fn stage_progress(message: &str, current: u64, total: u64, video_path: &str) -> OpsProgress {
    OpsProgress {
        kind: "download".into(),
        current,
        total,
        video_path: video_path.into(),
        message: message.into(),
    }
}

/// نتیجه اجرای یک گروه volume.
enum GroupOutcome {
    /// اجرای CLI تمام شد (موفق یا ناموفق) و پاک‌سازی انجام شد؛ فیلدهای
    /// `staged`/`staging_paths`/`exit_code`/`message` در result به‌روز شده‌اند.
    CliDone { success: bool, cancelled: bool },
    /// ساخت staging ناموفق بود — rollback کامل انجام شده، خروجی‌ای از دست
    /// نرفته؛ دلیل فایل‌های گروه در skipped برگشته است.
    BuildFailed {
        message: String,
        skipped: Vec<SkippedDownload>,
    },
    /// پیش از اجرای SubSource لغو شد — rollback کامل انجام شده.
    CancelledBeforeCli { message: String },
}

/// هسته دانلود گروهی — مستقل از Tauri و قابل تست با Fake SubSource.
///
/// `api_key` اختیاری است: None یا خالی = اجرای ناشناس (sub-rs بدون کلید
/// جستجو و دانلود می‌کند؛ کلید فقط برای حساب شخصی SubSource است).
/// `staging_base` فقط برای تست‌هاست؛ در تولید ریشه staging روی ریشه volume
/// هر گروه ساخته می‌شود.
///
/// این تابع هرگز `Err` برنمی‌گرداند — هر نتیجه (موفق، جزئی، لغو، خطا)
/// به‌صورت صادقانه در `DownloadResult` گزارش می‌شود.
pub fn run_download_blocking(
    state: &AppState,
    items: &[DownloadPlanItem],
    api_key: Option<String>,
    staging_base: Option<&Path>,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut dyn FnMut(OpsProgress),
) -> DownloadResult {
    run_download_blocking_with_remover(
        state,
        items,
        api_key,
        staging_base,
        cancel,
        on_progress,
        |p| std::fs::remove_file(p),
    )
}

/// همان `run_download_blocking` با امکان تزریق تابع حذف لینک برای تست‌های قطعی.
pub fn run_download_blocking_with_remover<F>(
    state: &AppState,
    items: &[DownloadPlanItem],
    api_key: Option<String>,
    staging_base: Option<&Path>,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut dyn FnMut(OpsProgress),
    remover: F,
) -> DownloadResult
where
    F: FnMut(&Path) -> std::io::Result<()>,
{
    run_download_blocking_with_removers(
        state,
        items,
        api_key,
        staging_base,
        cancel,
        on_progress,
        remover,
        |p| std::fs::remove_dir(p),
    )
}

/// همان `run_download_blocking_with_remover` با تزریق تابع حذف پوشه نیز —
/// برای تست قطعی خطاهای پاک‌سازی پوشه در rollback و cleanup.
#[allow(clippy::too_many_arguments)]
pub fn run_download_blocking_with_removers<F, D>(
    state: &AppState,
    items: &[DownloadPlanItem],
    api_key: Option<String>,
    staging_base: Option<&Path>,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut dyn FnMut(OpsProgress),
    remover: F,
    dir_remover: D,
) -> DownloadResult
where
    F: FnMut(&Path) -> std::io::Result<()>,
    D: FnMut(&Path) -> std::io::Result<()>,
{
    let start = Instant::now();
    let mut result = run_download_inner(
        state,
        items,
        api_key,
        staging_base,
        cancel,
        on_progress,
        remover,
        dir_remover,
    );
    result.duration_secs = start.elapsed().as_secs_f64();
    result
}

#[allow(clippy::too_many_arguments)]
fn run_download_inner<F, D>(
    state: &AppState,
    items: &[DownloadPlanItem],
    api_key: Option<String>,
    staging_base: Option<&Path>,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut dyn FnMut(OpsProgress),
    mut remover: F,
    mut dir_remover: D,
) -> DownloadResult
where
    F: FnMut(&Path) -> std::io::Result<()>,
    D: FnMut(&Path) -> std::io::Result<()>,
{
    let mut result = DownloadResult::default();

    // ۰) بررسی پرچم لغو پیش از هر کاری
    if cancel.load(Ordering::SeqCst) {
        result.cancelled = true;
        result.message = "عملیات توسط کاربر لغو شد.".into();
        return result;
    }

    // ۱) API key اختیاری — خالی/None = دانلود ناشناس با sub-rs
    let api_key = api_key.map(|k| k.trim().to_string()).unwrap_or_default();
    if api_key.is_empty() {
        crate::logging::info("subsource", "کلید API تنظیم نشده — دانلود ناشناس SubSource");
    }

    // ۲) اعتبارسنجی مجدد پلن در لحظهٔ اجرا — وضعیت دیسک ممکن است از زمان
    //    ساخت پلن تغییر کرده باشد
    let total_requested = items.len() as u64;
    on_progress(stage_progress(
        "در حال آماده‌سازی...",
        0,
        total_requested,
        "",
    ));
    let mut valid: Vec<DownloadPlanItem> = Vec::new();
    for item in items {
        let entry = get_entry(state, &item.video_path);
        match validate_video_typed(&item.video_path, entry.as_ref()) {
            Ok(()) => valid.push(item.clone()),
            Err(issue) => {
                // نگاشت اجرا: وضعیت‌های بین پلن و اجرا کد مخصوص خودشان را می‌گیرند
                let (code, reason) = match &issue {
                    VideoSkipIssue::MissingFile => (
                        SkipReasonCode::MissingAtExecution,
                        "ویدیو در لحظهٔ اجرا روی دیسک پیدا نشد".to_string(),
                    ),
                    VideoSkipIssue::ValidSidecarExists { .. } => (
                        SkipReasonCode::SidecarAppearedAtExecution,
                        "زیرنویس فارسی معتبر در لحظهٔ اجرا کنار ویدیو ظاهر شد — بازنویسی نمی‌شود"
                            .to_string(),
                    ),
                    VideoSkipIssue::SidecarConflict { .. } => (
                        SkipReasonCode::SidecarConflict,
                        "فایل کناری نامعتبر در لحظهٔ اجرا — تعارض؛ فایل حفظ می‌شود".to_string(),
                    ),
                    VideoSkipIssue::SidecarInspectionFailed => {
                        (SkipReasonCode::SidecarInspectionFailed, issue.message())
                    }
                    other => (other.code(), other.message()),
                };
                let mut skipped_item = SkippedDownload::with_code(
                    item.video_path.clone(),
                    item.video_file_name.clone(),
                    code,
                    reason,
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
                result.skipped.push(skipped_item);
            }
        }
    }
    result.planned = valid.len();
    if valid.is_empty() {
        result.message = "هیچ ویدیوی واجد شرایطی برای دانلود نماند.".into();
        return result;
    }
    on_progress(stage_progress(
        &format!(
            "{} ویدیو واجد شرایط، {} رد شد",
            valid.len(),
            result.skipped.len()
        ),
        valid.len() as u64,
        total_requested,
        "",
    ));

    // ۳) ابزار SubSource از Tool Resolver موجود — پیش از ساخت staging
    let cli = match resolve_tool(state, "subsource").and_then(|exe| {
        SubSourceCli::new(exe.to_string_lossy().to_string()).map_err(|e| e.to_string())
    }) {
        Ok(c) => c,
        Err(e) => {
            result.message = e;
            return result;
        }
    };

    // ۴) گروه‌بندی volume + مسیرهای staging
    let (groups, group_skips) = staging::group_by_volume(&valid, staging_base);
    result.skipped.extend(group_skips);
    if groups.is_empty() {
        result.message = "هیچ گروه staging قابل ساختی وجود ندارد.".into();
        return result;
    }

    // ۵) تنظیمات اجرا از Settings موجود — بدون پروکسی، شبکه سیستم‌عامل
    let (clean_ads, skip_existing) = {
        let s = state.settings.lock().unwrap();
        (s.subsource.clean_ads, s.subsource.skip_existing)
    };
    let mut group_cfg = GroupConfig {
        api_key: &api_key,
        clean_ads,
        skip_existing,
        remover: &mut remover,
        dir_remover: &mut dir_remover,
    };

    let mut all_success = true;
    let mut group_failures = 0usize;

    for group in &groups {
        if cancel.load(Ordering::SeqCst) {
            result.cancelled = true;
            result.message = "عملیات توسط کاربر لغو شد.".into();
            return result;
        }
        match run_group(
            state,
            &cli,
            group,
            &mut group_cfg,
            cancel,
            &mut result,
            on_progress,
        ) {
            GroupOutcome::CliDone { success, cancelled } => {
                all_success &= success;
                if cancelled {
                    result.cancelled = true;
                    if result.message.is_empty() {
                        result.message = "دانلود لغو شد؛ خروجی جزئی حفظ شد.".into();
                    }
                    return result;
                }
            }
            GroupOutcome::BuildFailed { message, skipped } => {
                group_failures += 1;
                result.skipped.extend(skipped);
                result.message = message;
                // لغو در حین ساخت staging؟ (build خودش rollback می‌کند)
                if cancel.load(Ordering::SeqCst) {
                    result.cancelled = true;
                    return result;
                }
            }
            GroupOutcome::CancelledBeforeCli { message } => {
                result.cancelled = true;
                result.message = message;
                return result;
            }
        }
    }

    result.success = all_success && group_failures == 0 && result.cleanup_warnings.is_empty();
    if result.success {
        result.message = format!(
            "دانلود تمام شد: {} ویدیو پردازش شد؛ خروجی در staging حفظ شد.",
            result.staged
        );
    } else if result.message.is_empty() {
        if !result.cleanup_warnings.is_empty() {
            result.message = format!(
                "دانلود انجام شد اما پاک‌سازی {} مسیر موقت ناموفق بود؛ مسیرهای staging برای بررسی دستی حفظ شدند.",
                result.cleanup_warnings.len()
            );
        } else {
            result.message = "دانلود با خطا مواجه شد.".into();
        }
    } else if !result.cleanup_warnings.is_empty() {
        result.message.push_str(&format!(
            " (توجه: پاک‌سازی {} مسیر موقت ناموفق بود — مسیر staging برای بررسی دستی حفظ شد)",
            result.cleanup_warnings.len()
        ));
    }
    result
}

struct GroupConfig<'a> {
    api_key: &'a str,
    clean_ads: bool,
    skip_existing: bool,
    remover: &'a mut (dyn FnMut(&Path) -> std::io::Result<()> + 'a),
    dir_remover: &'a mut (dyn FnMut(&Path) -> std::io::Result<()> + 'a),
}

/// اجرای یک گروه volume: ساخت staging → اجرای SubSource → پاک‌سازی لینک‌ها.
#[allow(clippy::too_many_arguments)]
fn run_group(
    state: &AppState,
    cli: &SubSourceCli,
    group: &VolumeGroup,
    cfg: &mut GroupConfig<'_>,
    cancel: &Arc<AtomicBool>,
    result: &mut DownloadResult,
    on_progress: &mut dyn FnMut(OpsProgress),
) -> GroupOutcome {
    // ساخت staging با گزارش پیشرفت واقعی لینک‌ها
    let staged_before = result.staged as u64;
    let planned = result.planned as u64;
    let volume_key = group.volume_key.clone();
    let mut on_link = |done: usize, total: usize, p: &staging::StagedPlacement| {
        on_progress(stage_progress(
            &format!("لینک {done}/{total}: {}", p.file_name),
            staged_before + done as u64,
            planned,
            &p.source.to_string_lossy(),
        ));
    };
    let mut session = match staging::build_with_removers(
        group.staging_root.clone(),
        &group.placements,
        Some(cancel),
        &mut on_link,
        &mut *cfg.remover,
        &mut *cfg.dir_remover,
    ) {
        Ok(s) => s,
        Err(err) => {
            let cancelled = cancel.load(Ordering::SeqCst);
            let root_str = err.staging_root.to_string_lossy().to_string();

            // در صورت بروز خطا در rollback، مسیر staging و هشدارهای پاک‌سازی ثبت می‌شوند
            if !err.cleanup.is_success() {
                if !result.staging_paths.contains(&root_str) {
                    result.staging_paths.push(root_str.clone());
                }
                for (p, e) in &err.cleanup.failed {
                    result.cleanup_warnings.push(format!(
                        "حذف لینک موقت «{}» در حین rollback ساخت staging ناموفق بود ({e}) — برای پاک‌سازی دستی مسیر «{root_str}» را بررسی کنید",
                        p.display()
                    ));
                }
                for (p, e) in &err.cleanup.failed_dirs {
                    result.cleanup_warnings.push(format!(
                        "حذف پوشه موقت «{}» در حین rollback ساخت staging ناموفق بود ({e}) — برای پاک‌سازی دستی مسیر «{root_str}» را بررسی کنید",
                        p.display()
                    ));
                }
                crate::logging::warn(
                    "subsource",
                    &format!(
                        "ساخت staging روی {volume_key} با شکست rollback مواجه شد: {} مسیر باقیمانده",
                        err.cleanup.failure_count()
                    ),
                );
            }

            let message = if cancelled {
                if err.cleanup.is_success() {
                    format!("لغو در حین ساخت staging: {}", err.message)
                } else {
                    format!(
                        "لغو در حین ساخت staging (اما پاک‌سازی {} مسیر موقت ناموفق بود): {}",
                        err.cleanup.failure_count(),
                        err.message
                    )
                }
            } else if err.cleanup.is_success() {
                format!("ساخت staging روی {volume_key} ناموفق: {}", err.message)
            } else {
                format!(
                    "ساخت staging روی {volume_key} ناموفق و پاک‌سازی {} مسیر ناقص ماند: {}",
                    err.cleanup.failure_count(),
                    err.message
                )
            };

            on_progress(stage_progress(&message, 0, 0, ""));
            // فایل‌های این گروه که لینک نشدند با دلیل ثبت شوند
            let skipped = group
                .placements
                .iter()
                .map(|p| {
                    SkippedDownload::with_code(
                        p.source.to_string_lossy().to_string(),
                        p.file_name.clone(),
                        SkipReasonCode::StagingBuildFailed,
                        message.clone(),
                    )
                })
                .collect();
            if cancelled {
                return GroupOutcome::CancelledBeforeCli { message };
            }
            return GroupOutcome::BuildFailed { message, skipped };
        }
    };

    // لغو بین ساخت staging و اجرای SubSource — rollback کامل (خروجی‌ای هنوز
    // تولید نشده و از دست نمی‌رود)
    if cancel.load(Ordering::SeqCst) {
        let cleanup = session.rollback_with_removers(&mut *cfg.remover, &mut *cfg.dir_remover);
        if !cleanup.is_success() {
            let staging_root_str = session.root().to_string_lossy().to_string();
            if !result.staging_paths.contains(&staging_root_str) {
                result.staging_paths.push(staging_root_str.clone());
            }
            for (p, err) in &cleanup.failed {
                result.cleanup_warnings.push(format!(
                    "حذف لینک موقت «{}» در حین rollback ناموفق بود ({err}) — برای بررسی دستی به «{staging_root_str}» مراجعه کنید",
                    p.display()
                ));
            }
            for (p, err) in &cleanup.failed_dirs {
                result.cleanup_warnings.push(format!(
                    "حذف پوشه موقت «{}» در حین rollback ناموفق بود ({err}) — برای بررسی دستی به «{staging_root_str}» مراجعه کنید",
                    p.display()
                ));
            }
            return GroupOutcome::CancelledBeforeCli {
                message: format!(
                    "عملیات پیش از اجرای SubSource لغو شد، اما پاک‌سازی {} مسیر موقت ناموفق بود؛ مسیر staging برای بررسی دستی حفظ شد.",
                    cleanup.failure_count()
                ),
            };
        }
        return GroupOutcome::CancelledBeforeCli {
            message: "عملیات پیش از اجرای SubSource لغو شد؛ ساختار staging برگردانده شد.".into(),
        };
    }

    result.staged += session.link_count();
    let staging_root_str = session.root().to_string_lossy().to_string();
    if !result.staging_paths.contains(&staging_root_str) {
        result.staging_paths.push(staging_root_str.clone());
    }

    // نگاشت پایدار ویدیوی اصلی → مسیر staging، برای فاز Archive Extraction.
    // این کار پیش از اجرای CLI انجام می‌شود تا حتی در صورت خطا/لغو بعدی،
    // آرشیوهای دانلودشده قابل نسبت‌دادن به ویدیوی درست باشند. هیچ رازی در
    // این ساختار نیست. علاوه بر خروجی سریال‌شده، رکورد در رجیستری Provenance
    // سمت سرور ثبت می‌شود تا پلن/اجرا نتوانند staging جعلی را بپذیرند.
    for placement in &group.placements {
        // فقط ویدیوهایی که واقعاً به staging لینک شدند (همهٔ placements در
        // جلسهٔ موفق لینک می‌شوند؛ بررسی وجود فایل یک گارد محافظ است).
        let relative_video_path = if placement.rel_dir.as_os_str().is_empty() {
            placement.file_name.clone()
        } else {
            placement
                .rel_dir
                .join(&placement.file_name)
                .to_string_lossy()
                .to_string()
        };
        let staged_path = session.root().join(&relative_video_path);
        if !staged_path.exists() {
            continue;
        }
        let video_path = placement.source.to_string_lossy().to_string();
        let Some(entry) = get_entry(state, &video_path) else {
            crate::logging::warn(
                "subsource",
                &format!(
                    "provenance برای «{}» ثبت نشد: ویدیو در کتابخانه نیست",
                    placement.file_name
                ),
            );
            continue;
        };
        let rel_dir_str = placement.rel_dir.to_string_lossy().to_string();
        match state.archive_provenance.lock().unwrap().register(
            &entry,
            session.root(),
            &relative_video_path,
            &rel_dir_str,
        ) {
            Ok(source) => result.archive_sources.push(source),
            Err(e) => crate::logging::warn(
                "subsource",
                &format!("provenance برای «{}» ثبت نشد: {e}", placement.file_name),
            ),
        }
    }

    // اجرای SubSource روی staging این گروه — token همان پرچم رجیستری است
    on_progress(stage_progress(
        "اجرای SubSource — در حال دانلود زیرنویس‌ها…",
        result.staged as u64,
        result.planned as u64,
        "",
    ));
    let opts = BatchOptions {
        directory: session.root().to_string_lossy().to_string(),
        api_key: cfg.api_key.to_string(),
        top: 1,
        skip_existing: cfg.skip_existing,
        no_recursive: false,
        dry_run: false,
        clean_ads: cfg.clean_ads,
    };
    let token = CancellationToken::from_flag(Arc::clone(cancel));
    let mut group_success = true;
    // پیشرفت زنده: CLI خروجی ساختاریافته ندارد؛ خطوط [FILE] Processing نشان
    // می‌دهند الان سر کدام ویدیوست تا مرحلهٔ دانلود «گیرکرده» به نظر نرسد
    let mut cli_processed: u64 = 0;
    let mut on_line = |line: &str| {
        if let Some(name) = line.trim().strip_prefix("[FILE] Processing: ") {
            let name = name.trim();
            on_progress(stage_progress(
                &format!("در حال دانلود: {name}"),
                cli_processed,
                result.planned as u64,
                "",
            ));
        } else if line.contains("[SUCCESS] Processed (") {
            cli_processed += 1;
        }
    };
    match cli.run_batch_with_cancellation(&opts, &token, &mut on_line) {
        Ok(batch) => {
            result.exit_code = Some(batch.exit_code);
            crate::logging::info(
                "subsource",
                &format!(
                    "اجرای گروه {} موفق ({:.1}s)",
                    volume_key, batch.duration_secs
                ),
            );
        }
        Err(SubSourceError::ProcessTerminated(msg)) => {
            group_success = false;
            result.exit_code = None;
            result.message = format!("فرایند SubSource متوقف شد: {msg}");
        }
        Err(SubSourceError::CliExitCode { code, stderr }) => {
            group_success = false;
            result.exit_code = Some(code);
            result.message = format!("SubSource با کد خروج {code} خاتمه یافت — خروجی جزئی حفظ شد");
            crate::logging::warn(
                "subsource",
                &format!("اجرای گروه {volume_key} با کد {code} ناموفق: {stderr}"),
            );
        }
        Err(e) => {
            group_success = false;
            result.message = e.to_string();
        }
    }

    // پاک‌سازی لینک‌های موقت — خروجی دانلود و ریشه staging حفظ می‌شود
    on_progress(stage_progress(
        "پاک‌سازی لینک‌های موقت staging...",
        result.staged as u64,
        result.planned as u64,
        "",
    ));
    let cleanup =
        session.cleanup_keep_output_with_removers(&mut *cfg.remover, &mut *cfg.dir_remover);
    if !cleanup.is_success() {
        group_success = false;
        for (path, err) in &cleanup.failed {
            result.cleanup_warnings.push(format!(
                "حذف لینک موقت «{}» ناموفق بود ({err}) — برای پاک‌سازی دستی مسیر staging «{staging_root_str}» را بررسی کنید",
                path.display()
            ));
        }
        for (path, err) in &cleanup.failed_dirs {
            result.cleanup_warnings.push(format!(
                "حذف پوشه موقت «{}» ناموفق بود ({err}) — برای پاک‌سازی دستی مسیر staging «{staging_root_str}» را بررسی کنید",
                path.display()
            ));
        }
        crate::logging::warn(
            "subsource",
            &format!(
                "staging گروه {volume_key}: {} مسیر حذف نشد و در مالکیت باقی ماند",
                cleanup.failure_count()
            ),
        );
    }
    crate::logging::info(
        "subsource",
        &format!(
            "staging گروه {}: {} لینک حذف شد؛ خروجی در «{}» حفظ شد",
            volume_key,
            cleanup.removed,
            session.root().display()
        ),
    );

    GroupOutcome::CliDone {
        success: group_success,
        cancelled: cancel.load(Ordering::SeqCst),
    }
}

/// پلن دانلود برای تأیید کاربر — هیچ تغییری روی دیسک نمی‌دهد.
#[tauri::command]
pub fn plan_subsource_download(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<DownloadPlan, String> {
    if paths.is_empty() {
        return Err("هیچ ویدیویی انتخاب نشده است.".into());
    }
    let library: Vec<VideoEntry> = state.library.lock().unwrap().clone();
    Ok(create_download_plan(&library, &paths))
}

/// اجرای پلن تأییدشده در پس‌زمینه؛ تعداد موارد صف را برمی‌گرداند.
#[tauri::command]
pub fn execute_subsource_download(
    app: AppHandle,
    state: State<'_, AppState>,
    items: Vec<DownloadPlanItem>,
) -> Result<u64, String> {
    if items.is_empty() {
        return Err("فهرست دانلود خالی است.".into());
    }

    // ایمنی: تعلق هر مورد به کتابخانه در لحظهٔ اجرا
    for item in &items {
        if get_entry(&state, &item.video_path).is_none() {
            return Err(format!(
                "«{}» در کتابخانه نیست — پلن را دوباره بسازید.",
                item.video_file_name
            ));
        }
    }

    // API key اختیاری — خطای مخزن هم دانلود را متوقف نمی‌کند؛ ناشناس ادامه می‌دهیم
    let api_key = match retrieve_api_key() {
        Ok(key) => key.unwrap_or_default().trim().to_string(),
        Err(e) => {
            crate::logging::warn(
                "subsource",
                &format!("خواندن API key ناموفق بود — دانلود ناشناس ادامه می‌یابد: {e}"),
            );
            String::new()
        }
    };

    // رزرو اتمیک اسلات عملیات گروهی با گارد RAII — همان پرچمی که
    // cancel_subtitle_ops (cancel_all) لغو می‌کند
    let guard = acquire_batch_op(&state.ops)?;
    let cancel = guard.cancel.clone();
    let total = items.len() as u64;
    crate::logging::info("subsource", &format!("دانلود گروهی {total} ویدیو شروع شد"));

    let app2 = app.clone();
    std::thread::spawn(move || {
        let st = app2.state::<AppState>();
        let result = run_download_blocking(&st, &items, Some(api_key), None, &cancel, &mut |p| {
            let _ = app2.emit("ops:progress", &p);
        });
        drop(guard); // آزادسازی اسلات در همهٔ مسیرها (RAII)
        crate::logging::info(
            "subsource",
            &format!(
                "دانلود گروهی تمام شد: موفق={}، لغو={}، staged={}/{}، کد خروج={:?} ({:.1}s) — {}",
                result.success,
                result.cancelled,
                result.staged,
                result.planned,
                result.exit_code,
                result.duration_secs,
                result.message
            ),
        );
        // نتیجهٔ ساختاریافتهٔ کامل برای فاز بعدی (مسیرهای staging و هشدارهای
        // پاک‌سازی) — رویداد قدیمی `ops:done` برای سازگاری حفظ می‌شود.
        let _ = app2.emit(SUBSOURCE_DONE_EVENT, download_done_payload(&result));
        let _ = app2.emit(
            "ops:done",
            &OpsProgress {
                kind: "download".into(),
                current: result.staged as u64,
                total,
                video_path: String::new(),
                message: result.message,
            },
        );
    });

    Ok(total)
}

/// وضعیت عملیات دانلود برای UI — از State رجیستری می‌خواند.
#[tauri::command]
pub fn is_download_active(state: State<'_, AppState>) -> bool {
    // poison-safe: قفل مسموم با recovery بازیابی می‌شود
    match state.ops.lock() {
        Ok(ops) => ops.has_active_batch(),
        Err(poisoned) => poisoned.into_inner().has_active_batch(),
    }
}

// ===== افزودن زیرنویس چسبیده برای ویدیوهای هاردساب‌دار =====
//
// ویدیوهایی که زیرنویس‌شان داخل تصویر حک شده (قرارداد نام‌گذاری گروه‌های
// رلیز) پس از pipeline با دو گزینه به زیرنویس چسبیده مجهز می‌شوند:
// ۱. دانلود از SubSource → استخراج در staging → کپی هم‌نام کنار ویدیو
// ۲. فایل محلی کاربر → کپی هم‌نام کنار ویدیو
// هر دو مسیر هرگز بازنویسی نمی‌کنند و خروجی `<stem>.fa.srt` است تا سینک
// بعدی همان جریان عادی sidecar را ادامه دهد.

/// kind رویدادهای پیشرفت افزودن زیرنویس چسبیده — فرانت بر اساس آن فیلتر می‌کند.
pub const SOFTSUB_ADD_EVENT_KIND: &str = "softsub-add";

/// فقط یک عملیات افزودن زیرنویس چسبیده هم‌زمان — static تک‌نمونه‌ای (قرینهٔ
/// INSTALL_IN_FLIGHT معاونت راه‌اندازی)؛ بدون تغییر AppState.
static SOFTSUB_BATCH_IN_FLIGHT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// ویدیوی معمولیِ غیرخالی با پسوند ویدیویی معتبر.
fn validate_video_for_sidecar(video: &Path) -> Result<(), String> {
    let file_name = video
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !crate::scanner::naming::is_video_file(&file_name) {
        return Err(format!("پسوند ویدیو پشتیبانی نمی‌شود: {file_name}"));
    }
    let metadata =
        std::fs::symlink_metadata(video).map_err(|e| format!("ویدیو قابل دسترسی نیست: {e}"))?;
    if !metadata.is_file()
        || crate::archive::listing::is_link_or_reparse(&metadata)
        || metadata.len() == 0
    {
        return Err("ویدیو باید فایل معمولیِ غیرخالی و بدون symlink باشد".into());
    }
    Ok(())
}

/// زیرنویس معمولیِ معتبر فارسی — خروجی اعتبارسنجی محتوا.
fn validate_subtitle_for_sidecar(
    subtitle: &Path,
) -> Result<crate::models::SubtitleValidation, String> {
    let metadata = std::fs::symlink_metadata(subtitle)
        .map_err(|e| format!("زیرنویس قابل دسترسی نیست: {e}"))?;
    if !metadata.is_file()
        || crate::archive::listing::is_link_or_reparse(&metadata)
        || metadata.len() == 0
    {
        return Err("زیرنویس باید فایل معمولیِ غیرخالی و بدون symlink باشد".into());
    }
    let validation = crate::subtitle::validation::validate_file(subtitle);
    if !validation.valid || !validation.has_persian {
        return Err(format!(
            "زیرنویس معتبر و فارسی نیست: {}",
            validation
                .error
                .unwrap_or_else(|| "محتوای فارسی تأیید نشد".into())
        ));
    }
    Ok(validation)
}

/// کپی create-new زیرنویس به `<stem>.fa.srt` کنار ویدیو — هم‌نام ویدیو،
/// بدون بازنویسی؛ کپی به temp هم‌دیرکت و rename اتمیک روی همان volume.
pub(crate) fn place_sidecar_copy(subtitle: &Path, video: &Path) -> Result<String, String> {
    let target = crate::archive::placement::output_path(video, "srt")
        .map_err(|e| format!("ساخت مسیر مقصد ناموفق بود: {e}"))?;
    if target.exists() {
        return Err(format!(
            "«{}» از قبل موجود است — برای جلوگیری از بازنویسی هیچ فایلی دست نخورد",
            target.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    let parent = target
        .parent()
        .ok_or_else(|| "مقصد پوشهٔ والد ندارد".to_string())?;
    let token = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let temp = parent.join(format!(
        "{}.fa.placing-{token:x}.srt",
        target.file_stem().unwrap_or_default().to_string_lossy()
    ));
    let install = || -> Result<(), String> {
        std::fs::copy(subtitle, &temp).map_err(|e| format!("کپی به فضای موقت ناموفق بود: {e}"))?;
        std::fs::rename(&temp, &target).map_err(|e| format!("نصب کنار ویدیو ناموفق بود: {e}"))
    };
    if let Err(error) = install() {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    crate::logging::info(
        "subsource",
        &format!("زیرنویس چسبیده کنار ویدیو قرار گرفت: {}", target.display()),
    );
    Ok(target.to_string_lossy().into_owned())
}

/// افزودن زیرنویس چسبیده از فایل محلی کاربر — هم‌نام ویدیو، بدون بازنویسی.
#[tauri::command]
pub fn place_local_subtitle(
    state: State<'_, AppState>,
    video_path: String,
    subtitle_path: String,
) -> Result<String, String> {
    let video = std::path::PathBuf::from(video_path.trim());
    let subtitle = std::path::PathBuf::from(subtitle_path.trim());
    validate_video_for_sidecar(&video)?;
    validate_subtitle_for_sidecar(&subtitle)?;
    // اسلات تکی همان ویدیو — تعارض با pipeline/گروهی رد می‌شود
    let guard = crate::state::acquire_single_op(&state.ops, &video.to_string_lossy())?;
    let placed = place_sidecar_copy(&subtitle, &video);
    drop(guard);
    placed
}

/// هستهٔ دانلود + جانمایی برای یک ویدیو: run_download_blocking روی همان
/// ویدیو → استخراج‌شدهٔ sub-rs در staging → کپی هم‌نام کنار ویدیو واقعی →
/// پاک‌سازی staging. خروجی: مسیر زیرنویس جانمایی‌شده.
/// زیرنویس دانلودشده در staging — یا استخراج‌شدهٔ خود sub-rs یا نیازمند
/// استخراج از ZIP پک-فصل با موتور آرشیو.
enum StagedSubtitle {
    Extracted(std::path::PathBuf),
    NeedsArchiveExtraction,
}

/// زیرنویس دانلودی ممکن است cp1256 (ANSI فارسی) باشد — قبل از اعتبارسنجی،
/// فایل staging به UTF-8 نرمال می‌شود؛ شکست تبدیل = همان خطای قبلی با
/// توضیح. فقط فایل‌های staging متعلق به خود برنامه بازنویسی می‌شوند.
fn normalize_downloaded_subtitle_file(path: &Path) -> Result<(), String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("خواندن زیرنویس دانلودشده ناموفق بود: {e}"))?;
    if bytes.len() as u64 > crate::archive::types::MAX_SUBTITLE_BYTES {
        return Err("زیرنویس دانلودشده بیش از حد مجاز بزرگ است".into());
    }
    let validation = crate::subtitle::validation::validate_file(path);
    if validation.valid && validation.has_persian {
        return Ok(()); // از قبل سالم
    }
    let Some(text) = crate::subtitle::encoding::convert_windows1256_if_needed(&bytes) else {
        return Err(format!(
            "زیرنویس معتبر و فارسی نیست: {}",
            validation
                .error
                .unwrap_or_else(|| "محتوای فارسی تأیید نشد".into())
        ));
    };
    std::fs::write(path, text.as_bytes())
        .map_err(|e| format!("بازنویسی زیرنویس با UTF-8 ناموفق بود: {e}"))?;
    let revalidated = crate::subtitle::validation::validate_file(path);
    if !revalidated.valid || !revalidated.has_persian {
        return Err(format!(
            "زیرنویس پس از تبدیل encoding هم معتبر نشد: {}",
            revalidated
                .error
                .unwrap_or_else(|| "محتوای فارسی تأیید نشد".into())
        ));
    }
    crate::logging::info(
        "subsource",
        &format!(
            "زیرنویس دانلودشده cp1256 بود و به UTF-8 تبدیل شد: {}",
            path.display()
        ),
    );
    Ok(())
}

/// نرمال‌سازی ZIPهای پک-فصلِ staging: اعضای متنی با encoding legacy
/// (cp1256) به UTF-8 بازنویسی می‌شوند تا فاز کاندیدای آرشیو آن‌ها را رد نکند.
/// بهترین تلاش — خطا فقط لاگ می‌شود (پلن به هر حال مسیر شکست خودش را دارد).
fn normalize_staged_zip_archives(source: &crate::subsource::plan::ArchiveSource) {
    let sub_dir = std::path::Path::new(&source.staging_root)
        .join(&source.relative_video_dir)
        .join("sub");
    let Ok(entries) = std::fs::read_dir(&sub_dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let is_zip = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("zip"))
            .unwrap_or(false);
        if !is_zip {
            continue;
        }
        match normalize_zip_file(&path) {
            Ok(true) => crate::logging::info(
                "subsource",
                &format!("ZIP پک-فصل به UTF-8 نرمال شد: {}", path.display()),
            ),
            Ok(false) => {}
            Err(e) => crate::logging::warn(
                "subsource",
                &format!("نرمال‌سازی ZIP ناموفق بود ({}): {e}", path.display()),
            ),
        }
    }
}

/// بازنویسی اعضای متنی یک ZIP پس از تبدیل cp1256 — خروجی: آیا تغییری کرد؟
fn normalize_zip_file(path: &Path) -> Result<bool, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("باز کردن ZIP ناموفق بود: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("خواندن ZIP ناموفق بود: {e}"))?;
    if archive.is_empty() {
        return Ok(false);
    }
    let mut entries: Vec<(String, Vec<u8>)> = Vec::with_capacity(archive.len());
    let mut changed = false;
    for index in 0..archive.len() {
        let mut member = archive
            .by_index(index)
            .map_err(|e| format!("خواندن عضو ZIP ناموفق بود: {e}"))?;
        let name = member.name().to_string();
        if name.ends_with('/') {
            continue; // پوشه — در بازنویسی بازساخته نمی‌شود
        }
        let mut data = Vec::new();
        member
            .read_to_end(&mut data)
            .map_err(|e| format!("خواندن محتوای عضو ناموفق بود: {e}"))?;
        let is_text = matches!(
            name.rsplit('.').next().unwrap_or(""),
            "srt" | "ass" | "ssa" | "vtt" | "txt"
        );
        if is_text {
            if let Some(text) = crate::subtitle::encoding::convert_windows1256_if_needed(&data) {
                data = text.into_bytes();
                changed = true;
            }
        }
        entries.push((name, data));
    }
    if !changed {
        return Ok(false);
    }
    drop(archive);
    let temp = path.with_extension("zip.norm");
    let out_file =
        std::fs::File::create(&temp).map_err(|e| format!("ساخت ZIP موقت ناموفق بود: {e}"))?;
    let mut writer = zip::ZipWriter::new(out_file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in &entries {
        writer
            .start_file(name.clone(), options)
            .map_err(|e| format!("نوشتن عضو ZIP ناموفق بود: {e}"))?;
        std::io::Write::write_all(&mut writer, data)
            .map_err(|e| format!("نوشتن محتوای عضو ناموفق بود: {e}"))?;
    }
    writer
        .finish()
        .map_err(|e| format!("پایان ZIP موقت ناموفق بود: {e}"))?;
    std::fs::rename(&temp, path).map_err(|e| format!("جایگزینی ZIP نرمال‌شده ناموفق بود: {e}"))?;
    Ok(true)
}

/// هستهٔ دانلود برای یک ویدیو — خروجی: زیرنویس در staging + ریشه‌های
/// staging برای پاک‌سازی بعدی. هیچ چیزی کنار ویدیوی واقعی نمی‌گذارد.
fn fetch_staged_subtitle(
    state: &AppState,
    video_path: &str,
    api_key: &str,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut dyn FnMut(OpsProgress),
) -> Result<
    (
        StagedSubtitle,
        crate::subsource::plan::ArchiveSource,
        Vec<String>,
    ),
    String,
> {
    let video = std::path::PathBuf::from(video_path);
    let file_name = video
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| video_path.to_string());
    let item = DownloadPlanItem {
        video_path: video_path.to_string(),
        video_file_name: file_name.clone(),
    };

    let result = run_download_blocking(
        state,
        &[item],
        Some(api_key.to_string()),
        None,
        cancel,
        on_progress,
    );
    if result.cancelled {
        return Err("عملیات توسط کاربر لغو شد".into());
    }
    if let Some(skip) = result.skipped.first() {
        // ValidSidecarExists و بقیهٔ کدها با دلیل صادقانه گزارش می‌شوند
        return Err(format!("دانلود انجام نشد: {}", skip.reason));
    }
    if result.staged == 0 || result.archive_sources.is_empty() {
        return Err(format!(
            "هیچ زیرنویسی دانلود نشد — {}",
            if result.message.is_empty() {
                "SubSource موردی پیدا نکرد".to_string()
            } else {
                result.message.clone()
            }
        ));
    }

    // زیرنویس استخراج‌شدهٔ sub-rs کنار ویدیوی staged — هم‌نام همان ویدیو
    let source = &result.archive_sources[0];
    let staged_video = std::path::Path::new(&source.staging_root).join(&source.relative_video_path);
    let staged_srt = staged_video
        .parent()
        .map(|dir| {
            dir.join(format!(
                "{}.fa.srt",
                staged_video
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
            ))
        })
        .ok_or_else(|| "مسیر staging نامعتبر است".to_string())?;

    let staged = if staged_srt.is_file() {
        StagedSubtitle::Extracted(staged_srt)
    } else {
        // ZIP چند-اپیزودی (پک فصل مثل «all_episods.zip»): sub-rs از اعضای
        // بی‌نامِ سریال (`S01E02.srt`) اپیزود را نمی‌سازد — موتور آرشیو خودمان
        // با mچ اپیزود دقیقاً همین را حل می‌کند (همان مسیر pipeline).
        StagedSubtitle::NeedsArchiveExtraction
    };
    Ok((staged, source.clone(), result.staging_paths))
}

/// پاک‌سازی ریشه‌های staging پس از مصرف — بهترین تلاش، خطا فقط لاگ.
fn cleanup_staging(staging_paths: &[String]) {
    for root in staging_paths {
        if let Err(e) = std::fs::remove_dir_all(root) {
            crate::logging::warn(
                "subsource",
                &format!("پاک‌سازی staging ناموفق بود ({}): {e}", root),
            );
        }
    }
}

fn download_and_place_core(
    state: &AppState,
    video_path: &str,
    api_key: &str,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut dyn FnMut(OpsProgress),
) -> Result<String, String> {
    let video = std::path::PathBuf::from(video_path);
    let (staged, source, staging_paths) =
        fetch_staged_subtitle(state, video_path, api_key, cancel, on_progress)?;
    let placed = match staged {
        StagedSubtitle::Extracted(staged_srt) => {
            normalize_downloaded_subtitle_file(&staged_srt)?;
            validate_subtitle_for_sidecar(&staged_srt)?;
            place_sidecar_copy(&staged_srt, &video)?
        }
        StagedSubtitle::NeedsArchiveExtraction => {
            normalize_staged_zip_archives(&source);
            extract_from_staged_archive(state, &source, cancel, on_progress)?
        }
    };
    cleanup_staging(&staging_paths);
    Ok(placed)
}

/// استخراج زیرنویس از ZIPهای staging با موتور آرشیو خودمان — پلن + اجرا؛
/// وضعیت هر حالت (Ready/Ambiguous/NoMatch) صادقانه به خطای فارسی می‌رود.
fn extract_from_staged_archive(
    state: &AppState,
    source: &crate::subsource::plan::ArchiveSource,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut dyn FnMut(OpsProgress),
) -> Result<String, String> {
    use crate::archive::engine::{execute_archive_extraction, plan_archives};
    use crate::archive::types::{ArchiveExecuteRequest, ArchiveItemStatus, ArchiveSelection};

    let seven_zip = resolve_tool(state, "sevenZip")?;
    let plan = plan_archives(
        &seven_zip,
        std::slice::from_ref(source),
        cancel,
        &mut |mut p| {
            p.kind = SOFTSUB_ADD_EVENT_KIND.into();
            on_progress(p);
        },
    );
    let Some(item) = plan.items.first() else {
        return Err("آرشیوی در staging پیدا نشد تا زیرنویس از آن استخراج شود".into());
    };
    match item.status {
        crate::archive::types::ArchivePlanStatus::Ready => {
            let selection = item.selection.as_ref().map(|sel| ArchiveSelection {
                archive_file_name: sel.archive_file_name.clone(),
                member_path: sel.member_path.clone(),
            });
            let request = ArchiveExecuteRequest {
                source: source.clone(),
                selection,
            };
            let exec = execute_archive_extraction(
                &seven_zip,
                std::slice::from_ref(&request),
                cancel,
                &mut |mut p| {
                    p.kind = SOFTSUB_ADD_EVENT_KIND.into();
                    on_progress(p);
                },
            );
            let Some(result_item) = exec.items.first() else {
                return Err("اجرای استخراج آرشیو نتیجه‌ای نداشت".into());
            };
            if result_item.status == ArchiveItemStatus::Placed {
                let output = result_item
                    .output_path
                    .clone()
                    .ok_or_else(|| "زیرنویس جانمایی شد اما مسیر خروجی گزارش نشد".to_string())?;
                crate::logging::info(
                    "subsource",
                    &format!("زیرنویس از پک چند-اپیزودی استخراج و جانمایی شد: {output}"),
                );
                Ok(output)
            } else {
                Err(format!(
                    "استخراج از پک دانلودشده ناموفق بود: {}",
                    result_item.message
                ))
            }
        }
        crate::archive::types::ArchivePlanStatus::Ambiguous => Err(format!(
            "پک دانلودشده چند کاندیدای اپیزود دارد — {} گزینهٔ فایل محلی را استفاده کنید",
            item.candidates.len()
        )),
        crate::archive::types::ArchivePlanStatus::NoMatch => Err(format!(
            "هیچ عضو زیرنویسی از پک دانلودشده با اپیزود این ویدیو مچ نشد — {}",
            item.message
        )),
    }
}

/// افزودن زیرنویس چسبیده با دانلود از SubSource برای یک یا چند ویدیوی
/// هاردساب‌دار — هر ویدیو جدا دانلود و هم‌نام کنارش جانمایی می‌شود.
/// رویدادها با kind `softsub-add` منتشر می‌شوند (ops:progress / ops:done).
#[tauri::command]
pub fn download_softsub_batch(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<u64, String> {
    if paths.is_empty() {
        return Err("هیچ ویدیویی انتخاب نشده است.".into());
    }
    for path in &paths {
        validate_video_for_sidecar(Path::new(path.trim())).map_err(|e| {
            format!(
                "«{}»: {e}",
                Path::new(path.trim())
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            )
        })?;
    }

    // کلید اختیاری — خطای مخزن دانلود ناشناس را نمی‌بندد
    let api_key = match retrieve_api_key() {
        Ok(key) => key.unwrap_or_default().trim().to_string(),
        Err(e) => {
            crate::logging::warn(
                "subsource",
                &format!("خواندن API key ناموفق بود — دانلود ناشناس ادامه می‌یابد: {e}"),
            );
            String::new()
        }
    };
    let sub_exe = resolve_tool(&state, "subsource")?;
    let _ = sub_exe; // resolve فقط برای پیام فارسی زودهنگام نبود ابزار است

    // تک‌نمونه‌ای: عملیات افزودن زیرنویس چسبیده دوم رد می‌شود
    if SOFTSUB_BATCH_IN_FLIGHT
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_err()
    {
        return Err("یک عملیات افزودن زیرنویس چسبیده در جریان است؛ صبر کنید تا تمام شود.".into());
    }
    let guard = acquire_batch_op(&state.ops)?;
    let cancel = Arc::clone(&guard.cancel);
    let total = paths.len() as u64;
    crate::logging::info(
        "subsource",
        &format!("افزودن زیرنویس چسبیده برای {total} ویدیو شروع شد"),
    );

    let app2 = app.clone();
    std::thread::spawn(move || {
        let st = app2.state::<AppState>();
        let mut done: u64 = 0;
        let mut failed: u64 = 0;
        for (index, path) in paths.iter().enumerate() {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            let name = Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            let _ = app2.emit(
                "ops:progress",
                &OpsProgress {
                    kind: SOFTSUB_ADD_EVENT_KIND.into(),
                    current: index as u64 + 1,
                    total,
                    video_path: path.clone(),
                    message: format!("در حال دانلود زیرنویس چسبیده: {name}"),
                },
            );
            let mut progress_events: Vec<OpsProgress> = Vec::new();
            let outcome = download_and_place_core(&st, path, &api_key, &cancel, &mut |mut p| {
                p.kind = SOFTSUB_ADD_EVENT_KIND.into();
                p.current = index as u64 + 1;
                p.total = total;
                p.video_path = path.clone();
                progress_events.push(p);
            });
            for p in progress_events {
                let _ = app2.emit("ops:progress", &p);
            }
            match outcome {
                Ok(placed) => {
                    done += 1;
                    let _ = app2.emit(
                        "ops:progress",
                        &OpsProgress {
                            kind: SOFTSUB_ADD_EVENT_KIND.into(),
                            current: index as u64 + 1,
                            total,
                            video_path: path.clone(),
                            message: format!("انجام شد: {placed}"),
                        },
                    );
                }
                Err(reason) => {
                    failed += 1;
                    crate::logging::warn(
                        "subsource",
                        &format!("افزودن زیرنویس چسبیده «{name}» ناموفق: {reason}"),
                    );
                    let _ = app2.emit(
                        "ops:progress",
                        &OpsProgress {
                            kind: SOFTSUB_ADD_EVENT_KIND.into(),
                            current: index as u64 + 1,
                            total,
                            video_path: path.clone(),
                            message: format!("ناموفق: {reason}"),
                        },
                    );
                }
            }
        }
        drop(guard);
        // پرچم تک‌نمونه‌ای در پایان thread آزاد می‌شود
        SOFTSUB_BATCH_IN_FLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
        let summary = if failed > 0 {
            format!("افزودن زیرنویس چسبیده پایان یافت — موفق: {done}، ناموفق: {failed}")
        } else {
            format!("افزودن زیرنویس چسبیده پایان یافت — موفق: {done}")
        };
        crate::logging::info("subsource", &summary);
        crate::notify::notify_task_done(
            &app2,
            crate::notify::KIND_SOFTSUB,
            if failed > 0 {
                "افزودن زیرنویس چسبیده پایان یافت"
            } else {
                "زیرنویس چسبیده همه اضافه شد"
            },
            &summary,
        );
        let _ = app2.emit(
            "ops:done",
            &OpsProgress {
                kind: SOFTSUB_ADD_EVENT_KIND.into(),
                current: done,
                total,
                video_path: String::new(),
                message: summary,
            },
        );
    });

    Ok(total)
}

// ===== تنظیم دوباره زیرنویس — از اول: حذف قدیمی + دانلود تازه + سینک =====
//
// وقتی کاربر می‌بیند زیرنویس ویدیو خراب/نامرتبط است، با یک دکمه:
// ۱. زیرنویس تازه از SubSource دانلود می‌شود (اول به staging — منبع فعلی
//    تا موفقیت دانلود دست‌نخورده می‌ماند)
// ۲. sidecarهای ساخته‌شدهٔ قبلی برنامه (`<stem>.fa.srt` / `.fa.synced.srt`)
//    حذف و زیرنویس تازه هم‌نام جانمایی می‌شود
// ۳. pipeline روی همان ویدیو به‌صورت خودکار اجرا می‌شود → سینک واقعی از
//    اول → SoftSub (جایگزینی Track قبلی) یا جانمایی sidecar

pub const SUBTITLE_RESET_EVENT_KIND: &str = "subtitle-reset";

/// فقط یک تنظیم دوباره هم‌زمان.
static SUBTITLE_RESET_IN_FLIGHT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// حذف sidecarهای ساخته‌شدهٔ قبلی برنامه برای یک ویدیو — خروجی: تعداد حذف.
/// فقط نام‌های متعلق به برنامه حذف می‌شوند؛ هیچ فایل دیگری لمس نمی‌شود.
fn delete_stale_sidecars(video: &Path) -> usize {
    let Some(stem) = video.file_stem() else {
        return 0;
    };
    let Some(dir) = video.parent() else {
        return 0;
    };
    let mut removed = 0;
    for name in [
        format!("{}.fa.srt", stem.to_string_lossy()),
        format!("{}.fa.synced.srt", stem.to_string_lossy()),
    ] {
        let path = dir.join(&name);
        if path.is_file() {
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    removed += 1;
                    crate::logging::info(
                        "subsource",
                        &format!("زیرنویس قبلی برای تنظیم دوباره حذف شد: {}", path.display()),
                    );
                }
                Err(e) => crate::logging::warn(
                    "subsource",
                    &format!("حذف «{}» ناموفق بود: {e}", path.display()),
                ),
            }
        }
    }
    removed
}

/// تنظیم دوباره زیرنویس یک ویدیو — پس از پایان، pipeline همان ویدیو به‌صورت
/// خودکار اجرا می‌شود (سینک واقعی از اول). رویدادها با kind `subtitle-reset`.
#[tauri::command]
pub fn reset_subtitle(
    app: AppHandle,
    state: State<'_, AppState>,
    video_path: String,
) -> Result<(), String> {
    let video = std::path::PathBuf::from(video_path.trim());
    validate_video_for_sidecar(&video)?;
    if get_entry(&state, &video.to_string_lossy()).is_none() {
        return Err(
            "این ویدیو در کتابخانه اسکن‌شده نیست — ابتدا پوشه را انتخاب و سینک کنید.".into(),
        );
    }

    if SUBTITLE_RESET_IN_FLIGHT
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_err()
    {
        return Err("یک تنظیم دوباره در جریان است؛ صبر کنید تا تمام شود.".into());
    }

    let guard = acquire_batch_op(&state.ops)?;
    let cancel = Arc::clone(&guard.cancel);

    // کلید اختیاری — ناشناس هم دانلود می‌شود
    let api_key = match retrieve_api_key() {
        Ok(key) => key.unwrap_or_default().trim().to_string(),
        Err(e) => {
            crate::logging::warn(
                "subsource",
                &format!("خواندن API key ناموفق بود — دانلود ناشناس ادامه می‌یابد: {e}"),
            );
            String::new()
        }
    };
    let _ = resolve_tool(&state, "subsource")?; // پیام فارسی زودهنگام نبود ابزار

    let app2 = app.clone();
    let video2 = video.clone();
    std::thread::spawn(move || {
        let video_path = video2.to_string_lossy().into_owned();
        let emit = |message: &str| {
            let _ = app2.emit(
                "ops:progress",
                &OpsProgress {
                    kind: SUBTITLE_RESET_EVENT_KIND.into(),
                    current: 0,
                    total: 0,
                    video_path: video_path.clone(),
                    message: message.into(),
                },
            );
        };
        let finish = |message: &str| {
            let _ = app2.emit(
                "ops:done",
                &OpsProgress {
                    kind: SUBTITLE_RESET_EVENT_KIND.into(),
                    current: 0,
                    total: 0,
                    video_path: String::new(),
                    message: message.into(),
                },
            );
        };

        let outcome = (|| -> Result<String, String> {
            // ۱) دانلود زیرنویس تازه به staging — منبع فعلی هنوز دست‌نخورده
            emit("دانلود زیرنویس تازه از SubSource…");
            let st = app2.state::<AppState>();
            let (staged, source, staging_paths) =
                fetch_staged_subtitle(&st, &video_path, &api_key, &cancel, &mut |mut p| {
                    p.kind = SUBTITLE_RESET_EVENT_KIND.into();
                    p.current = 0;
                    p.total = 0;
                    p.video_path = video_path.clone();
                    emit(&p.message);
                })?;

            // ۲) حذف زیرنویس‌های قبلی — فقط پس از موفقیت دانلود
            emit("حذف زیرنویس قبلی…");
            let removed = delete_stale_sidecars(&video2);

            // ۳) جانمایی زیرنویس تازه هم‌نام ویدیو
            emit("جانمایی زیرنویس تازه…");
            let st = app2.state::<AppState>();
            let placed = match staged {
                StagedSubtitle::Extracted(staged_srt) => {
                    normalize_downloaded_subtitle_file(&staged_srt)?;
                    validate_subtitle_for_sidecar(&staged_srt)?;
                    place_sidecar_copy(&staged_srt, &video2)?
                }
                StagedSubtitle::NeedsArchiveExtraction => {
                    normalize_staged_zip_archives(&source);
                    extract_from_staged_archive(&st, &source, &cancel, &mut |p| {
                        // پیشرفت آرشیو هم با همان kind می‌رود
                        let mut p = p;
                        p.kind = SUBTITLE_RESET_EVENT_KIND.into();
                        p.video_path = video_path.clone();
                        emit(&p.message);
                    })?;
                    video2
                        .parent()
                        .map(|dir| {
                            dir.join(format!(
                                "{}.fa.srt",
                                video2.file_stem().unwrap_or_default().to_string_lossy()
                            ))
                            .to_string_lossy()
                            .into_owned()
                        })
                        .unwrap_or_default()
                }
            };
            cleanup_staging(&staging_paths);
            crate::logging::info(
                "subsource",
                &format!(
                    "تنظیم دوباره: زیرنویس تازه جانمایی شد ({placed}) — {removed} زیرنویس قبلی حذف شد"
                ),
            );
            Ok(placed)
        })();

        match &outcome {
            Ok(placed) => emit(&format!("زیرنویس تازه آماده است: {placed} — سینک از نو…")),
            Err(reason) => {
                finish(&format!("تنظیم دوباره ناموفق بود: {reason}"));
                crate::logging::warn("subsource", &format!("تنظیم دوباره ناموفق: {reason}"));
                SUBTITLE_RESET_IN_FLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
                drop(guard);
                return;
            }
        }

        // ۴) سینک از اول — اجرای خودکار pipeline روی همین ویدیو
        drop(guard); // اسلات reset آزاد تا pipeline اسلات تازه بگیرد
        let input = crate::pipeline::PipelineInput::Files {
            paths: vec![video_path.clone()],
        };
        let st = app2.state::<AppState>();
        // شمارهٔ تلاش این ویدیو بالا می‌رود تا استراتژی سینک با تلاش قبل
        // واقعاً فرق کند (حلقهٔ سینک → پخش → قضاوت کاربر → تنظیم دوباره)
        {
            let mut attempts = st.sync_attempts.lock().unwrap_or_else(|p| p.into_inner());
            *attempts.entry(video_path.clone()).or_insert(0) += 1;
            crate::logging::info(
                "subsource",
                &format!(
                    "تنظیم دوبارهٔ شمارهٔ {} برای این ویدیو — استراتژی سینک می‌چرخد",
                    attempts.get(&video_path).copied().unwrap_or(1)
                ),
            );
        }
        let started = crate::commands::pipeline::start_pipeline(app2.clone(), st, input);
        SUBTITLE_RESET_IN_FLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
        match started {
            Ok(_) => finish("زیرنویس تازه جانمایی شد و سینک از نو شروع شد."),
            Err(e) => {
                // مثلاً pipeline دیگری در جریان است — تنظیم دوباره خودش موفق بوده
                finish(&format!(
                    "زیرنویس تازه جانمایی شد اما سینک خودکار شروع نشد ({e}) — دستی سینک را اجرا کنید."
                ));
            }
        }
    });

    Ok(())
}

/// مجموعه کوچک تست‌های هسته (بدون فرایند واقعی — برای اجرای فرایند واقعی
/// `tests/integration_subsource_download.rs` را ببینید).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::sync::Mutex;

    fn make_state() -> AppState {
        AppState {
            settings: Mutex::new(Settings::default()),
            library: Mutex::new(Vec::new()),
            last_scan: Mutex::new(None),
            ops: Arc::new(Mutex::new(crate::state::OpsRegistry::default())),
            pipeline: Mutex::new(None),
            hardsub: Mutex::new(None),
            hardsub_encoder_cache: Mutex::new(None),
            archive_provenance: Mutex::new(
                crate::archive::provenance::ProvenanceRegistry::default(),
            ),
            setup_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            sync_attempts: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn unique_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "subsync-dl-core-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// بایت‌های cp1256 نمونه — همان فیکسچر تست encoding (زیرنویس فارسی)
    const SAMPLE_CP1256_BYTES_FOR_ZIP: &[u8] = &[
        0x31, 0x0A, 0x30, 0x30, 0x3A, 0x30, 0x30, 0x3A, 0x30, 0x31, 0x2C, 0x30, 0x30, 0x30, 0x20,
        0x2D, 0x2D, 0x3E, 0x20, 0x30, 0x30, 0x3A, 0x30, 0x30, 0x3A, 0x30, 0x32, 0x2C, 0x30, 0x30,
        0x30, 0x0A, 0xD3, 0xE1, 0xC7, 0xE3, 0x20, 0xCF, 0xE4, 0xED, 0xC7, 0x20, 0x97, 0x20, 0xD2,
        0xED, 0xD1, 0xE4, 0xE6, 0xED, 0xD3, 0x20, 0xDD, 0xC7, 0xD1, 0xD3, 0xED, 0x20, 0xC8, 0xD1,
        0xC7, 0xED, 0x20, 0xCA, 0xD3, 0xCA, 0x20, 0xCA, 0xC8, 0xCF, 0xED, 0xE1, 0x20, 0xC7, 0xE4,
        0xDF, 0xE6, 0xCF, 0xED, 0xE4, 0x90, 0x0A, 0x0A, 0x32, 0x0A, 0x30, 0x30, 0x3A, 0x30, 0x30,
        0x3A, 0x30, 0x32, 0x2C, 0x31, 0x30, 0x30, 0x20, 0x2D, 0x2D, 0x3E, 0x20, 0x30, 0x30, 0x3A,
        0x30, 0x30, 0x3A, 0x30, 0x33, 0x2C, 0x35, 0x30, 0x30, 0x0A, 0xCE, 0xD8, 0x20, 0xCF, 0xE6,
        0xE3, 0x20, 0xC8, 0xD1, 0xC7, 0xED, 0x20, 0xC7, 0xD8, 0xE3, 0xED, 0xE4, 0xC7, 0xE4, 0x20,
        0xC7, 0xD2, 0x20, 0xCA, 0xDA, 0xCF, 0xC7, 0xCF, 0x20, 0xDF, 0xC7, 0xDD, 0xED, 0x20, 0xCD,
        0xD1, 0xE6, 0xDD, 0x0A,
    ];

    /// نرمال‌سازی ZIP پک-فصل: عضو cp1256 → UTF-8 (بدون تغییر عضو سالم)
    #[test]
    fn normalize_zip_file_converts_legacy_members() {
        use std::io::Write;
        let base = unique_dir("zip-norm");
        let sub_dir = base.join("sub");
        std::fs::create_dir_all(&sub_dir).unwrap();
        let zip_path = sub_dir.join("pack.zip");

        // ساخت ZIP با دو عضو: ANSI فارسی + باینری خام
        let out_file = std::fs::File::create(&zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(out_file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer
            .start_file("S01E02.srt".to_string(), options)
            .unwrap();
        writer.write_all(SAMPLE_CP1256_BYTES_FOR_ZIP).unwrap();
        writer.start_file("cover.bin".to_string(), options).unwrap();
        writer.write_all(&[0x00, 0xFF, 0x00, 0xFF]).unwrap();
        drop(writer.finish().unwrap());

        let source = crate::subsource::plan::ArchiveSource {
            video_path: base.join("Show.S01E02.mkv").to_string_lossy().into_owned(),
            video_file_name: "Show.S01E02.mkv".into(),
            staging_root: base.to_string_lossy().into_owned(),
            relative_video_path: "Show.S01E02.mkv".into(),
            relative_video_dir: String::new(),
            season: Some(1),
            episode: Some(2),
        };
        normalize_staged_zip_archives(&source);

        // عضو متنی باید UTF-8 فارسی شده باشد؛ باینری دست‌نخورده
        let file = std::fs::File::open(&zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut text = Vec::new();
        {
            let mut srt = archive.by_name("S01E02.srt").unwrap();
            std::io::Read::read_to_end(&mut srt, &mut text).unwrap();
        }
        let text = String::from_utf8(text).expect("عضو متنی باید UTF-8 باشد");
        assert!(text.contains("سلام"), "محتوای تبدیل‌شده: {text}");
        let mut raw = Vec::new();
        {
            let mut bin = archive.by_name("cover.bin").unwrap();
            std::io::Read::read_to_end(&mut bin, &mut raw).unwrap();
        }
        assert_eq!(raw, vec![0x00, 0xFF, 0x00, 0xFF]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// تست واقعی با فایل ANSI حفظ‌شده از دانلود کاربر (Resident Evil —
    /// همان شکست گزارش‌شده): normalize باید cp1256 → UTF-8 کند و فارسی تأیید شود.
    /// ورودی: SUBSYNC_REAL_ANSI_SRT (مسیر یک srt واقعی cp1256).
    #[test]
    #[ignore]
    fn live_normalize_converts_real_ansi_subtitle() {
        let Ok(src) = std::env::var("SUBSYNC_REAL_ANSI_SRT") else {
            eprintln!("SKIP: SUBSYNC_REAL_ANSI_SRT تنظیم نشده");
            return;
        };
        let base = unique_dir("ansi-real");
        let target =
            base.join("Resident.Evil.2026.1080p.WEB-DL.6CH.YIFY.Farsi.Sub.Film2Media.fa.srt");
        std::fs::copy(&src, &target).expect("کپی فایل واقعی");

        // قبل از normalize: validate شکست می‌خورد (encoding)
        let before = crate::subtitle::validation::validate_file(&target);
        assert!(
            !before.valid,
            "فایل اصلی باید غیرمعتبر (ANSI) باشد: {:?}",
            before
        );

        normalize_downloaded_subtitle_file(&target).expect("تبدیل باید موفق شود");

        let after = crate::subtitle::validation::validate_file(&target);
        assert!(
            after.valid && after.has_persian,
            "پس از تبدیل باید فارسی معتبر باشد: {:?}",
            after
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// حذف sidecarهای قبلی — فقط فایل‌های متعلق به برنامه؛ غایب‌ها نادیده
    #[test]
    fn delete_stale_sidecars_removes_only_app_generated_files() {
        let base = unique_dir("stale-sidecars");
        let video = base.join("Show.S01E01.mkv");
        std::fs::write(&video, b"VIDEO").unwrap();
        let fa = base.join("Show.S01E01.fa.srt");
        std::fs::write(&fa, b"OLD").unwrap();
        let synced = base.join("Show.S01E01.fa.synced.srt");
        std::fs::write(&synced, b"OLD-SYNCED").unwrap();
        let other = base.join("Show.S01E01.english.srt");
        std::fs::write(&other, b"KEEP-ME").unwrap();

        let removed = delete_stale_sidecars(&video);
        assert_eq!(removed, 2, "هر دو sidecar برنامه باید حذف شوند");
        assert!(!fa.exists());
        assert!(!synced.exists());
        assert!(other.exists(), "زیرنویس غیر برنامه نباید حذف شود");

        // دوباره → صفر (غایب‌ها نادیده)
        assert_eq!(delete_stale_sidecars(&video), 0);
        let _ = std::fs::remove_dir_all(&base);
    }

    const SRT_FA: &str = "1
00:00:01,000 --> 00:00:02,000
سلام دنیا
";

    /// جانمایی هم‌نام: خروجی `<stem>.fa.srt` کنار ویدیو — create-new
    #[test]
    fn place_sidecar_copies_next_to_video_with_same_name() {
        let base = unique_dir("place-ok");
        let video = base.join("Movie.2023.Farsi.Sub.Film2Media.mkv");
        std::fs::write(&video, b"VIDEO").unwrap();
        let sub = base.join("other.fa.srt");
        std::fs::write(&sub, SRT_FA).unwrap();

        let placed = place_sidecar_copy(&sub, &video).expect("place");
        let target = base.join("Movie.2023.Farsi.Sub.Film2Media.fa.srt");
        assert_eq!(placed, target.to_string_lossy());
        assert_eq!(std::fs::read(&target).unwrap(), SRT_FA.as_bytes());
        // فایل موقت placing پاک شده باشد
        let leftovers: Vec<_> = std::fs::read_dir(&base)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".placing-"))
            .collect();
        assert!(leftovers.is_empty(), "temp باقی مانده: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// تعارض: مقصد موجود → خطا و دست‌نخوردن فایل موجود
    #[test]
    fn place_sidecar_never_overwrites_existing_target() {
        let base = unique_dir("place-conflict");
        let video = base.join("Show.S01E01.Farsi.Sub.mkv");
        std::fs::write(&video, b"VIDEO").unwrap();
        let target = base.join("Show.S01E01.Farsi.Sub.fa.srt");
        std::fs::write(&target, b"EXISTING").unwrap();
        let sub = base.join("new.fa.srt");
        std::fs::write(&sub, SRT_FA).unwrap();

        let err = place_sidecar_copy(&sub, &video).expect_err("conflict");
        assert!(err.contains("موجود است"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"EXISTING");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// زیرنویس غیر فارسی در جانمایی محلی رد می‌شود
    #[test]
    fn local_placement_rejects_non_persian_subtitle() {
        let base = unique_dir("place-invalid");
        let video = base.join("Show.S01E02.Farsi.Sub.mkv");
        std::fs::write(&video, b"VIDEO").unwrap();
        let sub = base.join("english.srt");
        std::fs::write(
            &sub,
            "1
00:00:01,000 --> 00:00:02,000
hello world
",
        )
        .unwrap();

        let err = validate_subtitle_for_sidecar(&sub).expect_err("not persian");
        assert!(err.contains("فارسی"), "{err}");
        let _ = std::fs::remove_dir_all(&base);
    }

    /// نبود کلید دیگر گِیت نیست — رفتار با فهرست خالی همان مسیر صادقانهٔ
    /// «هیچ ویدیوی واجد شرایط» است؛ هیچ stagingای ساخته نمی‌شود.
    #[test]
    fn missing_api_key_no_longer_gates_download() {
        let state = make_state();
        let base = unique_dir("key");

        let cancel = Arc::new(AtomicBool::new(false));
        let result = run_download_blocking(
            &state,
            &[],
            None,
            Some(&base),
            &cancel,
            &mut |_p: OpsProgress| {},
        );

        assert!(!result.success);
        assert!(!result.cancelled);
        assert!(
            !result.message.contains("API key"),
            "پیام قدیمی کلید باید حذف شده باشد: {}",
            result.message
        );
        assert!(result.message.contains("واجد شرایط"), "{}", result.message);
        assert_eq!(result.staged, 0);
        assert!(result.staging_paths.is_empty());
        // هیچ stagingای ساخته نشده باشد
        let leftovers: Vec<_> = std::fs::read_dir(&base)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(staging::STAGING_DIR_PREFIX)
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "قبل از شروع گروه نباید staging ساخته شود"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// فهرست خالی → نتیجه با پیام روشن و هیچ تغییر دیسک.
    #[test]
    fn empty_items_produce_clear_result() {
        let state = make_state();
        let cancel = Arc::new(AtomicBool::new(false));
        let result = run_download_blocking(
            &state,
            &[],
            Some("key".into()),
            None,
            &cancel,
            &mut |_p: OpsProgress| {},
        );
        assert!(!result.success);
        assert!(result.message.contains("واجد شرایط"), "{}", result.message);
    }

    /// تست زنده (#[ignore]): فیکس واقعی پک فصل — قسمت‌هایی که sub-rs از
    /// ZIP چند-اپیزودی (`all_episods.zip`) اپیزودش را نمی‌سازد، با موتور
    /// آرشیو خودمان استخراج و هم‌نام جانمایی می‌شوند.
    /// پیش‌نیاز: sub-rs/7z واقعی + شبکه + SUBSYNC_REAL_HARDSUB_VIDEO.
    #[test]
    #[ignore]
    fn live_softsub_add_extracts_from_season_pack() {
        let Ok(src) = std::env::var("SUBSYNC_REAL_HARDSUB_VIDEO") else {
            eprintln!("SKIP: SUBSYNC_REAL_HARDSUB_VIDEO تنظیم نشده");
            return;
        };
        let sub_rs = crate::tools::resolve_tool_path("subsource", &Default::default())
            .expect("sub-rs واقعی لازم است");
        let seven_zip = crate::tools::resolve_tool_path("sevenZip", &Default::default())
            .expect("7-Zip واقعی لازم است");
        let ffprobe = crate::tools::resolve_tool_path("ffprobe", &Default::default())
            .expect("ffprobe واقعی لازم است");

        let state = make_state();
        state.settings.lock().unwrap().tool_paths.subsource =
            Some(sub_rs.to_string_lossy().into_owned());
        state.settings.lock().unwrap().tool_paths.seven_zip =
            Some(seven_zip.to_string_lossy().into_owned());
        state.settings.lock().unwrap().tool_paths.ffprobe =
            Some(ffprobe.to_string_lossy().into_owned());

        // دو نسخه از همان قسمتِ مشکل‌دار در پوشهٔ موقت — استخراج باید موفق شود
        let dir = unique_dir("season-pack");
        let src = std::path::PathBuf::from(&src);
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Overlord.S01E02.1080p.WEB-DL.Farsi.Sub.Film2Media.mkv".into());
        let v1 = dir.join(&name);
        std::fs::copy(&src, &v1).expect("کپی نمونه");

        // ویدیو باید در کتابخانه باشد (قرارداد دانلود) — مثل جریان واقعی پس از pipeline
        let (season, episode) = crate::scanner::naming::parse_episode(&name)
            .map(|m| (Some(m.season), Some(m.episode)))
            .unwrap_or((None, None));
        state
            .library
            .lock()
            .unwrap()
            .push(crate::models::VideoEntry {
                id: v1.to_string_lossy().into_owned(),
                path: v1.to_string_lossy().into_owned(),
                file_name: name.clone(),
                size_bytes: std::fs::metadata(&v1).map(|m| m.len()).unwrap_or(0),
                modified_ms: None,
                parent_dir: dir.to_string_lossy().into_owned(),
                show_name: Some("Overlord".into()),
                season,
                episode,
                sibling_subtitles: Vec::new(),
                analysis: None,
            });

        let cancel = Arc::new(AtomicBool::new(false));
        let outcome =
            download_and_place_core(&state, &v1.to_string_lossy(), "", &cancel, &mut |_p| {});
        let placed = outcome.unwrap_or_else(|e| panic!("پک فصل باید از مسیر آرشیو حل شود: {e}"));

        let expected = dir.join(format!(
            "{}.fa.srt",
            v1.file_stem().unwrap().to_string_lossy()
        ));
        assert_eq!(
            std::path::Path::new(&placed),
            expected,
            "جانمایی باید دقیقاً هم‌نام ویدیو باشد"
        );
        assert!(expected.is_file());
        let validation = crate::subtitle::validation::validate_file(&expected);
        assert!(
            validation.valid && validation.has_persian,
            "زیرنویس جانمایی باید فارسی معتبر باشد"
        );

        // staging پاک شده باشد
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(staging::STAGING_DIR_PREFIX)
            })
            .collect();
        assert!(leftovers.is_empty(), "staging باید پاک شود: {leftovers:?}");

        // تعارض: اجرای دوباره → خطای «از قبل موجود» نه بازنویسی
        let again =
            download_and_place_core(&state, &v1.to_string_lossy(), "", &cancel, &mut |_p| {});
        assert!(again.is_err(), "اجرای دوباره باید تعارض بدهد");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// پرچم لغو از قبل فعال → بدون هیچ اجرای SubSource متوقف شود.
    #[test]
    fn precancelled_operation_does_not_run() {
        let state = make_state();
        let base = unique_dir("precancel");
        let cancel = Arc::new(AtomicBool::new(true));
        let result = run_download_blocking(
            &state,
            &[],
            Some("key".into()),
            Some(&base),
            &cancel,
            &mut |_p: OpsProgress| {},
        );
        assert!(result.cancelled);
        assert_eq!(result.staged, 0);
        let _ = std::fs::remove_dir_all(&base);
    }
}
