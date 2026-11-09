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
    create_download_plan, validate_video, DownloadPlan, DownloadPlanItem, DownloadResult,
    SkippedDownload,
};
use crate::subsource::staging::{self, VolumeGroup};
use crate::subsource::{
    retrieve_api_key, BatchOptions, CancellationToken, SubSourceCli, SubSourceError,
};

use super::media::{get_entry, resolve_tool};

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
/// `api_key = None` یعنی کلید موجود نیست → توقف کامل قبل از هر تغییر دیسک.
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
    let start = Instant::now();
    let mut result = run_download_inner(state, items, api_key, staging_base, cancel, on_progress);
    result.duration_secs = start.elapsed().as_secs_f64();
    result
}

fn run_download_inner(
    state: &AppState,
    items: &[DownloadPlanItem],
    api_key: Option<String>,
    staging_base: Option<&Path>,
    cancel: &Arc<AtomicBool>,
    on_progress: &mut dyn FnMut(OpsProgress),
) -> DownloadResult {
    let mut result = DownloadResult::default();

    // ۰) بررسی پرچم لغو پیش از هر کاری
    if cancel.load(Ordering::SeqCst) {
        result.cancelled = true;
        result.message = "عملیات توسط کاربر لغو شد.".into();
        return result;
    }

    // ۱) API key — پیش از هرگونه تغییر روی دیسک
    let Some(api_key) = api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
    else {
        result.message = "برای دانلود زیرنویس ابتدا API key را تنظیم کنید (ذخیره امن در Credential Manager سیستم).".into();
        return result;
    };

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
        match validate_video(&item.video_path, entry.as_ref()) {
            Ok(()) => valid.push(item.clone()),
            Err(reason) => result.skipped.push(SkippedDownload {
                video_path: item.video_path.clone(),
                video_file_name: item.video_file_name.clone(),
                reason,
            }),
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
    let group_cfg = GroupConfig {
        api_key: &api_key,
        clean_ads,
        skip_existing,
    };

    let mut all_success = true;
    let mut group_failures = 0usize;

    for group in &groups {
        if cancel.load(Ordering::SeqCst) {
            result.cancelled = true;
            result.message = "عملیات توسط کاربر لغو شد.".into();
            return result;
        }
        match run_group(&cli, group, &group_cfg, cancel, &mut result, on_progress) {
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

    result.success = all_success && group_failures == 0;
    if result.success {
        result.message = format!(
            "دانلود تمام شد: {} ویدیو پردازش شد؛ خروجی در staging حفظ شد.",
            result.staged
        );
    } else if result.message.is_empty() {
        result.message = "دانلود با خطا مواجه شد.".into();
    }
    result
}

struct GroupConfig<'a> {
    api_key: &'a str,
    clean_ads: bool,
    skip_existing: bool,
}

/// اجرای یک گروه volume: ساخت staging → اجرای SubSource → پاک‌سازی لینک‌ها.
fn run_group(
    cli: &SubSourceCli,
    group: &VolumeGroup,
    cfg: &GroupConfig<'_>,
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
    let mut session = match staging::build(
        group.staging_root.clone(),
        &group.placements,
        Some(cancel),
        &mut on_link,
    ) {
        Ok(s) => s,
        Err(e) => {
            let cancelled = cancel.load(Ordering::SeqCst);
            let message = if cancelled {
                format!("لغو در حین ساخت staging: {e}")
            } else {
                format!("ساخت staging روی {volume_key} ناموفق: {e}")
            };
            on_progress(stage_progress(&message, 0, 0, ""));
            // فایل‌های این گروه که لینک نشدند با دلیل ثبت شوند
            let skipped = group
                .placements
                .iter()
                .map(|p| SkippedDownload {
                    video_path: p.source.to_string_lossy().to_string(),
                    video_file_name: p.file_name.clone(),
                    reason: message.clone(),
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
        session.rollback();
        return GroupOutcome::CancelledBeforeCli {
            message: "عملیات پیش از اجرای SubSource لغو شد؛ ساختار staging برگردانده شد.".into(),
        };
    }

    result.staged += session.link_count();
    result
        .staging_paths
        .push(session.root().to_string_lossy().to_string());

    // اجرای SubSource روی staging این گروه — token همان پرچم رجیستری است
    on_progress(stage_progress(
        "اجرای SubSource — در حال دانلود زیرنویس‌ها (بدون درصد؛ CLI خروجی ساختاریافته ندارد)",
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
    match cli.run_batch_with_cancellation(&opts, &token) {
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
    let removed = session.cleanup_keep_output();
    crate::logging::info(
        "subsource",
        &format!(
            "staging گروه {}: {} لینک حذف شد؛ خروجی در «{}» حفظ شد",
            volume_key,
            removed,
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

    // API key پیش از هرگونه تغییر دیسک و پیش از رزرو اسلات
    let api_key = retrieve_api_key().map_err(|e| format!("خواندن API key: {e}"))?;
    if api_key.as_deref().map(str::trim).unwrap_or("").is_empty() {
        return Err(
            "برای دانلود زیرنویس ابتدا API key را تنظیم کنید (ذخیره امن در Credential Manager سیستم)."
                .into(),
        );
    }

    // رزرو اتمیک اسلات عملیات گروهی با گارد RAII — همان پرچمی که
    // cancel_subtitle_ops (cancel_all) لغو می‌کند
    let guard = acquire_batch_op(&state.ops)?;
    let cancel = guard.cancel.clone();
    let total = items.len() as u64;
    crate::logging::info("subsource", &format!("دانلود گروهی {total} ویدیو شروع شد"));

    let app2 = app.clone();
    std::thread::spawn(move || {
        let st = app2.state::<AppState>();
        let result = run_download_blocking(&st, &items, api_key, None, &cancel, &mut |p| {
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
    state.ops.lock().unwrap().batch_cancel.is_some()
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
            scanning: AtomicBool::new(false),
            cancel_scan: AtomicBool::new(false),
            ops: Arc::new(Mutex::new(crate::state::OpsRegistry::default())),
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

    /// نبود API key باید قبل از هر تغییر دیسک متوقف شود.
    #[test]
    fn missing_api_key_aborts_before_any_fs_change() {
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
        assert!(result.message.contains("API key"), "{}", result.message);
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
            "قبل از API key نباید staging ساخته شود"
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
