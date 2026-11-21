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

// ===== افزودن زیرنویس نرم برای ویدیوهای هاردساب‌دار =====
//
// ویدیوهایی که زیرنویس‌شان داخل تصویر حک شده (قرارداد نام‌گذاری گروه‌های
// رلیز) پس از pipeline با دو گزینه به زیرنویس نرم مجهز می‌شوند:
// ۱. دانلود از SubSource → استخراج در staging → کپی هم‌نام کنار ویدیو
// ۲. فایل محلی کاربر → کپی هم‌نام کنار ویدیو
// هر دو مسیر هرگز بازنویسی نمی‌کنند و خروجی `<stem>.fa.srt` است تا سینک
// بعدی همان جریان عادی sidecar را ادامه دهد.

/// kind رویدادهای پیشرفت افزودن زیرنویس نرم — فرانت بر اساس آن فیلتر می‌کند.
pub const SOFTSUB_ADD_EVENT_KIND: &str = "softsub-add";

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
        &format!("زیرنویس نرم کنار ویدیو قرار گرفت: {}", target.display()),
    );
    Ok(target.to_string_lossy().into_owned())
}

/// افزودن زیرنویس نرم از فایل محلی کاربر — هم‌نام ویدیو، بدون بازنویسی.
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
fn download_and_place_core(
    state: &AppState,
    video_path: &str,
    api_key: &str,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut dyn FnMut(OpsProgress),
) -> Result<String, String> {
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
    if !staged_srt.is_file() {
        return Err(
            "دانلود انجام شد اما زیرنویس استخراج‌شده در staging نبود — با فایل محلی تلاش کنید".into(),
        );
    }
    validate_subtitle_for_sidecar(&staged_srt)?;
    let placed = place_sidecar_copy(&staged_srt, &video)?;

    // staging فقط حامل لینک/آرشیو موقت بود — پس از مصرف پاک می‌شود
    for root in &result.staging_paths {
        if let Err(e) = std::fs::remove_dir_all(root) {
            crate::logging::warn(
                "subsource",
                &format!("پاک‌سازی staging ناموفق بود ({}): {e}", root),
            );
        }
    }
    Ok(placed)
}

/// افزودن زیرنویس نرم با دانلود از SubSource برای یک یا چند ویدیوی
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

    let guard = acquire_batch_op(&state.ops)?;
    let cancel = Arc::clone(&guard.cancel);
    let total = paths.len() as u64;
    crate::logging::info(
        "subsource",
        &format!("افزودن زیرنویس نرم برای {total} ویدیو شروع شد"),
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
                    message: format!("در حال دانلود زیرنویس نرم: {name}"),
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
                        &format!("افزودن زیرنویس نرم «{name}» ناموفق: {reason}"),
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
        let summary = if failed > 0 {
            format!("افزودن زیرنویس نرم پایان یافت — موفق: {done}، ناموفق: {failed}")
        } else {
            format!("افزودن زیرنویس نرم پایان یافت — موفق: {done}")
        };
        crate::logging::info("subsource", &summary);
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
