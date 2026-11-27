//! آداپتور اجرای FFsubsync — هستهٔ مستقل از Tauri برای PR #6 گام ۳.
//!
//! قرارداد: ورودی (ویدیو + زیرنویس فارسیِ از پیش اعتبارسنجی‌شده) + مسیرهای
//! ابزارهای resolve‌شده + تنظیمات پردازش → اجرای امن `ffs` بدون shell →
//! تشخیص fail-closed موفقیت (marker آفست + اعتبارسنجی خروجی فارسی) →
//! کاندیدای موقتِ سینک‌شده در workspace اختصاصی برای جانمایی بعدی.
//!
//! امنیت و صداقت:
//! - کد خروج 0 و وجود فایل خروجی به‌تنهایی هرگز «سینک‌شده» نیست — marker
//!   `offset seconds:` لازم است و marker `leaving subtitles unmodified`
//!   بر همه‌چیز مقدم است (رفتار واقعی ffsubsync 0.5.1).
//! - ورودی‌ها هرگز تغییر نمی‌کنند؛ خروجی فقط در workspace یکتای خود اجرا
//!   ساخته می‌شود و هیچ مسیر موجودی بازنویسی نمی‌شود.
//! - لغو با خاتمهٔ درخت فرایند (taskkill /T /F روی ویندوز) انجام می‌شود؛
//!   در نبودِ تأیید، هرگز ادعای قطعی خاتمهٔ فرزندان نمی‌کنیم.
//! - diagnosticها محدودشده‌اند و هیچ secretای به این ماژول نمی‌رسد.

mod cli;
pub mod types;

pub use types::{SyncConfig, SyncOutcome, SyncProgress, SyncRequest, SyncStage, SyncStatus};

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::archive::listing::is_link_or_reparse;
use crate::scanner::naming::{file_extension, SUBTITLE_EXTENSIONS};
use crate::subtitle::validation::validate_file;
use types::{DIAGNOSTIC_TAIL_CHARS, SUMMARY_MAX_CHARS};

/// هشدار صریح از دست رفتن استایل — فقط برای ورودی ASS/SSA که به SRT تبدیل می‌شود.
pub(crate) fn styling_loss_warning(ext: &str) -> Option<String> {
    match ext {
        "ass" | "ssa" => {
            Some("تبدیل ASS/SSA به SRT باعث از دست رفتن استایل‌ها (فونت/موقعیت/رنگ) می‌شود.".into())
        }
        _ => None,
    }
}

/// فایل معمولی موجود و بدون symlink/reparse — همان معیار امنیتی سایر ماژول‌ها.
fn is_plain_file(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => meta.is_file() && !is_link_or_reparse(&meta),
        Err(_) => false,
    }
}

/// اعتبارسنجی کامل درخواست قبل از هر اجرا — خروجی: (پسوند زیرنویس، stem ویدیو).
// SyncOutcome عمداً به‌صورت مقدار در Err حمل می‌شود (سراسری fail-closed)؛
// اندازهٔ آن آگاهانه پذیرفته شده است.
#[allow(clippy::result_large_err)]
fn validate_request(request: &SyncRequest) -> Result<(String, String), SyncOutcome> {
    if !is_plain_file(&request.ffsubsync_path) {
        return Err(SyncOutcome::bare(
            SyncStatus::ToolMissing,
            format!(
                "مسیر FFsubsync «{}» فایل اجرایی معتبری نیست",
                request.ffsubsync_path.display()
            ),
        ));
    }
    if let Some(ffmpeg) = &request.ffmpeg_path {
        if !is_plain_file(ffmpeg) {
            return Err(SyncOutcome::bare(
                SyncStatus::ToolMissing,
                format!("مسیر FFmpeg «{}» فایل اجرایی معتبری نیست", ffmpeg.display()),
            ));
        }
    }

    if !request.video_path.is_absolute() {
        return Err(SyncOutcome::bare(
            SyncStatus::InvalidInput,
            "مسیر ویدیو مطلق نیست",
        ));
    }
    if !is_plain_file(&request.video_path) {
        return Err(SyncOutcome::bare(
            SyncStatus::InvalidInput,
            format!(
                "ویدیوی ورودی «{}» فایل معمولی موجودی نیست یا پیوند/Reparse است",
                request.video_path.display()
            ),
        ));
    }
    let video_stem = request
        .video_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if video_stem.is_empty() {
        return Err(SyncOutcome::bare(
            SyncStatus::InvalidInput,
            "نام ویدیو برای ساخت خروجی سینک معتبر نیست",
        ));
    }

    if !request.subtitle_path.is_absolute() {
        return Err(SyncOutcome::bare(
            SyncStatus::InvalidInput,
            "مسیر زیرنویس مطلق نیست",
        ));
    }
    if !is_plain_file(&request.subtitle_path) {
        return Err(SyncOutcome::bare(
            SyncStatus::InvalidInput,
            format!(
                "زیرنویس ورودی «{}» فایل معمولی موجودی نیست یا پیوند/Reparse است",
                request.subtitle_path.display()
            ),
        ));
    }
    let sub_file_name = request
        .subtitle_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = file_extension(&sub_file_name);
    if !SUBTITLE_EXTENSIONS.contains(&ext.as_str()) {
        return Err(SyncOutcome::bare(
            SyncStatus::InvalidInput,
            format!("فرمت «{ext}» برای سینک پشتیبانی نمی‌شود — فقط srt/ass/ssa/vtt/sub"),
        ));
    }

    // همان اعتبارسنجی موجود پروژه — قاعدهٔ تکراری نوشته نمی‌شود
    let validation = validate_file(&request.subtitle_path);
    if !validation.valid {
        return Err(SyncOutcome::bare(
            SyncStatus::InvalidInput,
            format!(
                "زیرنویس ورودی ساختار معتبر ندارد: {}",
                validation.error.unwrap_or_else(|| "نامشخص".into())
            ),
        ));
    }
    if !validation.has_persian {
        return Err(SyncOutcome::bare(
            SyncStatus::InvalidInput,
            "زیرنویس ورودی محتوای فارسی ندارد — سینک فقط برای زیرنویس فارسی معنا دارد",
        ));
    }
    Ok((ext, video_stem))
}

/// workspace موقت اختصاصی یک اجرا — یکتا، collision-safe و فقط از آنِ همین اجرا.
pub(crate) struct SyncWorkspace {
    pub root: std::path::PathBuf,
    pub out_dir: std::path::PathBuf,
    pub log_dir: std::path::PathBuf,
}

const WORKSPACE_PREFIX: &str = "subsync-sync-";

impl SyncWorkspace {
    /// ساخت با ۸ تلاش برای رد شدن از هر برخورد نام — `create_dir` روی نام موجود
    /// شکست می‌خورد (برخلاف create_dir_all). ریشهٔ پایه در صورت نبود ساخته می‌شود.
    pub(crate) fn create(base: Option<&Path>) -> Result<SyncWorkspace, String> {
        let base = base
            .map(Path::to_path_buf)
            .unwrap_or_else(std::env::temp_dir);
        if let Err(e) = std::fs::create_dir_all(&base) {
            return Err(format!("ساخت ریشهٔ workspace موقت: {e}"));
        }
        let mut last_error = String::new();
        for _ in 0..8 {
            let root = base.join(format!("{WORKSPACE_PREFIX}{}", cli::unique_token()));
            if let Err(e) = std::fs::create_dir(&root) {
                last_error = format!("ساخت workspace موقت: {e}");
                continue;
            }
            let out_dir = root.join("out");
            let log_dir = root.join("logs");
            if let Err(e) = std::fs::create_dir(&out_dir) {
                let _ = std::fs::remove_dir_all(&root);
                return Err(format!("ساخت پوشهٔ خروجی workspace: {e}"));
            }
            if let Err(e) = std::fs::create_dir(&log_dir) {
                let _ = std::fs::remove_dir_all(&root);
                return Err(format!("ساخت پوشهٔ لاگ workspace: {e}"));
            }
            return Ok(SyncWorkspace {
                root,
                out_dir,
                log_dir,
            });
        }
        Err(last_error)
    }

    /// نام کاندیدای خروجی — همان قرارداد نام نهایی آینده، داخل workspace موقت.
    pub(crate) fn output_path(&self, video_stem: &str) -> std::path::PathBuf {
        self.out_dir.join(format!("{video_stem}.fa.synced.srt"))
    }

    /// خروجی اختصاصی هر تلاش (استراتژی) — کاندیدای برنده فقط در انتها به مسیر
    /// قراردادی `output_path` منتقل می‌شود.
    pub(crate) fn pass_output(&self, video_stem: &str, idx: usize) -> std::path::PathBuf {
        self.out_dir
            .join(format!("{video_stem}.pass{idx}.fa.synced.srt"))
    }

    /// لاگ اختصاصی هر تلاش — ffsubsync لاگش را append می‌کند (logging.FileHandler
    /// با mode='a')؛ اگر تلاش‌ها log_dir مشترک می‌گرفتند، markerهای تلاش قبلی
    /// در طبقه‌بندی تلاش بعدی خوانده می‌شدند.
    pub(crate) fn pass_log_dir(&self, idx: usize) -> std::path::PathBuf {
        self.log_dir.join(format!("pass-{idx}"))
    }

    /// پاک‌سازی کامل workspace (فقط متعلق به همین اجرا) — خطاها گزارش می‌شوند، هرگز panic.
    pub(crate) fn remove(&self) -> Vec<String> {
        match std::fs::remove_dir_all(&self.root) {
            Ok(()) => Vec::new(),
            Err(e) => vec![format!("پاک‌سازی workspace موقت ناموفق بود: {e}")],
        }
    }
}

/// مالکیت workspace موقتِ یک اجرای موفق — opaque و backend-only.
///
/// فقط `run_sync` روی `Synced` می‌سازد (سازنده private همین ماژول)؛
/// Serialize/Deserialize/Clone ندارد؛ فقط نمایش مسیر و حذف مصرف‌کننده.
pub struct SyncWorkspaceHandle {
    workspace: SyncWorkspace,
}

impl std::fmt::Debug for SyncWorkspaceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // مسیر موقت هرگز در لاگ/رویدادها لو نمی‌رود
        f.write_str("SyncWorkspaceHandle(<owned temporary workspace>)")
    }
}

impl SyncWorkspaceHandle {
    /// مسیر برای نمایش/گزارش — فقط خواندنی؛ هیچ توان حذفی از این مسیر نیست.
    pub fn display_root(&self) -> &Path {
        &self.workspace.root
    }

    /// حذف مصرف‌کننده (by-value → دوباره‌کاری type-level ناممکن).
    /// گاردهای دفاعی: مطلق + پیشوند خود adapter + بدون symlink/reparse.
    /// خطاها warning برمی‌گردند؛ هرگز panic نمی‌کند.
    pub fn discard(self) -> Vec<String> {
        let root = &self.workspace.root;
        let safe = root.is_absolute()
            && root
                .file_name()
                .map(|n| n.to_string_lossy().starts_with(WORKSPACE_PREFIX))
                .unwrap_or(false)
            && std::fs::symlink_metadata(root)
                .map(|meta| meta.is_dir() && !crate::archive::listing::is_link_or_reparse(&meta))
                .unwrap_or(false);
        if !safe {
            return vec![format!(
                "مسیر workspace برای پاک‌سازی معتبر نیست: {}",
                root.display()
            )];
        }
        self.workspace.remove()
    }
}

/// سازندهٔ خصوصی — تنها نقطهٔ ساخت handle در کل crate.
fn from_workspace(workspace: SyncWorkspace) -> SyncWorkspaceHandle {
    SyncWorkspaceHandle { workspace }
}

/// شناسهٔ یکتای run برای pipeline — wrapper باریک روی توکن یکتا.
pub(crate) fn new_pipeline_run_id() -> String {
    cli::unique_token()
}

/// اجرای کامل سینک برای یک درخواست — ورودی‌ها را اعتبارسنجی، در workspace یکتا
/// اجرا و طبق قواعد fail-closed طبقه‌بندی می‌کند.
///
/// قرارداد خروجی:
/// - `Synced`: workspace و کاندیدا **نگه داشته می‌شوند**، مالکیت workspace با
///   `attach_workspace` به outcome منتقل و برای مرحلهٔ جانمایی بعدی با
///   `take_workspace` تحویل می‌شود.
/// - هر وضعیت دیگر: workspace و خروجی جزئی پاک می‌شوند؛ ورودی‌ها همیشه دست‌نخورده‌اند.
pub fn run_sync<F: FnMut(SyncProgress)>(
    request: &SyncRequest,
    cancel: &AtomicBool,
    on_progress: &mut F,
) -> SyncOutcome {
    on_progress(SyncProgress {
        stage: SyncStage::Validating,
        message: "اعتبارسنجی ویدیو و زیرنویس ورودی…".into(),
    });
    let (input_ext, video_stem) = match validate_request(request) {
        Ok(valid) => valid,
        Err(outcome) => return outcome,
    };

    on_progress(SyncProgress {
        stage: SyncStage::Preparing,
        message: "ساخت workspace موقت…".into(),
    });
    let workspace = match SyncWorkspace::create(request.workspace_root.as_deref()) {
        Ok(workspace) => workspace,
        Err(e) => return SyncOutcome::bare(SyncStatus::Failed, e),
    };

    let mut outcome = execute_in_workspace(
        request,
        &workspace,
        &input_ext,
        &video_stem,
        cancel,
        on_progress,
    );

    // فقط موفقیت workspace را نگه می‌دارد (مالکیت به فراخواننده منتقل می‌شود)؛
    // هر وضعیت دیگر، هیچ کاندیدایی نمی‌تواند نگه دارد
    if outcome.status == SyncStatus::Synced {
        outcome.attach_workspace(from_workspace(workspace));
    } else {
        for note in workspace.remove() {
            if note.is_empty() {
                continue;
            }
            if !outcome.diagnostic.is_empty() {
                outcome.diagnostic.push(' ');
            }
            outcome.diagnostic.push_str(&note);
        }
    }
    outcome
}

/// اجرا داخل workspace آماده — جدا از run_sync تا سناریوی «خروجی از قبل موجود»
/// مستقیماً قابل تست باشد (بازنویسی ممنوع).
///
/// اجرا چندمرحله‌ای است (`cli::sync_strategies`): اول مرجع کامل (Track داخلی/
/// VAD کامل صوت) و در صورت فعال بودن multi_segment، نمونه‌برداری صوت به‌عنوان
/// تلاش دوم. اولین استراتژی‌ای که کاندیدای تأییدشده بدهد برنده است؛ اگر هیچ‌کدام
/// نشدند، وضعیت آخرین تلاش با خلاصهٔ همهٔ تلاش‌ها برگردانده می‌شود.
pub(crate) fn execute_in_workspace(
    request: &SyncRequest,
    workspace: &SyncWorkspace,
    input_ext: &str,
    video_stem: &str,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(SyncProgress),
) -> SyncOutcome {
    let output = workspace.output_path(video_stem);
    // workspace تازه و یکتاست؛ اگر به هر دلیلی خروجی از قبل موجود باشد، بازنویسی ممنوع
    if output.exists() {
        return SyncOutcome::bare(
            SyncStatus::Failed,
            format!(
                "مسیر خروجی موقت «{}» از قبل موجود است — بازنویسی ممنوع",
                output.display()
            ),
        );
    }
    if cancel.load(Ordering::Relaxed) {
        return SyncOutcome::bare(SyncStatus::Cancelled, "عملیات پیش از اجرا لغو شد.");
    }

    let strategies = cli::sync_strategies(&request.config);
    let mut attempts: Vec<String> = Vec::new();
    let mut last_failure: Option<SyncOutcome> = None;

    for (idx, strategy) in strategies.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return SyncOutcome::bare(SyncStatus::Cancelled, "عملیات پیش از اجرا لغو شد.");
        }

        on_progress(SyncProgress {
            stage: SyncStage::Running,
            message: format!("اجرای FFsubsync — {}…", strategy.label()),
        });

        let pass_output = workspace.pass_output(video_stem, idx);
        // workspace تازه است؛ خروجی هر تلاش هم باید از قبل موجود نباشد
        if pass_output.exists() {
            return SyncOutcome::bare(
                SyncStatus::Failed,
                format!(
                    "مسیر خروجی موقت تلاش «{}» از قبل موجود است — بازنویسی ممنوع",
                    pass_output.display()
                ),
            );
        }
        let pass_log_dir = workspace.pass_log_dir(idx);
        if let Err(e) = std::fs::create_dir_all(&pass_log_dir) {
            return SyncOutcome::bare(SyncStatus::Failed, format!("ساخت پوشهٔ لاگ تلاش: {e}"));
        }

        let args = cli::build_cli_args(request, *strategy, &pass_output, &pass_log_dir);
        let run = match cli::run_process(&request.ffsubsync_path, &args, &pass_log_dir, cancel) {
            Ok(run) => run,
            // خطای spawn به استراتژی ربط ندارد — تلاش دوباره بی‌فایده است
            Err(e) => return SyncOutcome::bare(SyncStatus::ToolExecutionFailed, e),
        };
        if run.cancelled {
            let mut diagnostic = run.termination_notes.join(" ");
            if !diagnostic.is_empty() {
                diagnostic.push(' ');
            }
            diagnostic.push_str("خروجی موقت حذف می‌شود و ورودی‌ها دست‌نخورده می‌مانند.");
            return SyncOutcome::bare(SyncStatus::Cancelled, diagnostic);
        }

        let diagnostic_tail = cli::bounded_tail(&run.logs, DIAGNOSTIC_TAIL_CHARS);
        match cli::classify_run(run.exit_code, &run.logs) {
            cli::RunClassification::Synchronized { offset_seconds } => {
                on_progress(SyncProgress {
                    stage: SyncStage::Verifying,
                    message: "اعتبارسنجی خروجی سینک‌شده…".into(),
                });
                match validate_candidate(&pass_output, &diagnostic_tail, input_ext) {
                    Ok(mut outcome) => {
                        // کاندیدای تأییدشده به مسیر خروجی قراردادی منتقل می‌شود
                        if let Err(e) = std::fs::copy(&pass_output, &output) {
                            return SyncOutcome::bare(
                                SyncStatus::Failed,
                                format!("انتقال کاندیدای سینک‌شده به مسیر خروجی: {e}"),
                            );
                        }
                        let _ = std::fs::remove_file(&pass_output);
                        outcome.output_path = Some(output.clone());
                        // آفستِ validated در طبقه‌بندی حمل شده — همیشه Some برای Synced
                        outcome.offset_seconds = Some(offset_seconds);
                        if !attempts.is_empty() {
                            attempts.push(format!("تلاش {} [{}]: موفق.", idx + 1, strategy.label()));
                            let summary = attempts.join(" ");
                            outcome.summary = Some(cli::bounded_tail(&summary, SUMMARY_MAX_CHARS));
                            outcome.diagnostic = cli::bounded_tail(
                                &format!("{summary} {}", outcome.diagnostic),
                                DIAGNOSTIC_TAIL_CHARS,
                            );
                        }
                        return outcome;
                    }
                    Err(mut failed) => {
                        let _ = std::fs::remove_file(&pass_output);
                        attempts.push(format!(
                            "تلاش {} [{}]: خروجی نامعتبر — دور ریخته شد.",
                            idx + 1,
                            strategy.label()
                        ));
                        failed.summary = Some(cli::bounded_tail(&attempts.join(" "), SUMMARY_MAX_CHARS));
                        last_failure = Some(failed);
                        // تلاش بعدی ممکن است استراتژیِ متفاوت خروجی معتبر بدهد
                    }
                }
            }
            classification => {
                let _ = std::fs::remove_file(&pass_output);
                let (status, note) = classify_failure(classification, &run.logs);
                attempts.push(format!(
                    "تلاش {} [{}]: {note}",
                    idx + 1,
                    strategy.label()
                ));
                let mut failed = SyncOutcome::bare(
                    status,
                    format!("زیرنویس هم‌ترازیِ مورد اعتماد پیدا نکرد. {diagnostic_tail}"),
                );
                failed.summary = Some(cli::bounded_tail(&attempts.join(" "), SUMMARY_MAX_CHARS));
                last_failure = Some(failed);
            }
        }
    }

    // هیچ استراتژی‌ای کاندیدای معتبر نداد — وضعیت آخرین تلاش + خلاصهٔ همهٔ تلاش‌ها
    let Some(mut final_outcome) = last_failure else {
        return SyncOutcome::bare(SyncStatus::Failed, "هیچ تلاشی برای سینک ثبت نشد.");
    };
    let summary = attempts.join(" ");
    final_outcome.summary = Some(cli::bounded_tail(&summary, SUMMARY_MAX_CHARS));
    final_outcome.diagnostic = cli::bounded_tail(
        &format!("{summary} {}", final_outcome.diagnostic),
        DIAGNOSTIC_TAIL_CHARS,
    );
    final_outcome
}

/// اعتبارسنجی کاندیدای سینک‌شدهٔ یک تلاش — ساختار + محتوای فارسی.
/// خروجی Ok هنوز در مسیر موقت تلاش است؛ منتقل/تمیزکاری با فراخواننده.
// SyncOutcome عمداً به‌صورت مقدار در Err حمل می‌شود (سراسری fail-closed)؛
// اندازهٔ آن آگاهانه پذیرفته شده است — همان قرارداد validate_request.
#[allow(clippy::result_large_err)]
fn validate_candidate(
    candidate: &Path,
    diagnostic_tail: &str,
    input_ext: &str,
) -> Result<SyncOutcome, SyncOutcome> {
    if !candidate.is_file() {
        return Err(SyncOutcome::bare(
            SyncStatus::OutputValidationFailed,
            format!("marker آفست دیده شد اما فایل خروجی ساخته نشد. {diagnostic_tail}"),
        ));
    }
    let validation = validate_file(candidate);
    if !validation.valid || !validation.has_persian {
        let reason = if validation.valid {
            "خروجی سینک‌شده محتوای فارسی ندارد"
        } else {
            "خروجی سینک‌شده ساختار معتبر SRT ندارد"
        };
        return Err(SyncOutcome::bare(
            SyncStatus::OutputValidationFailed,
            format!("{reason} — خروجی دور ریخته می‌شود. {diagnostic_tail}"),
        ));
    }
    let mut outcome = SyncOutcome::bare(SyncStatus::Synced, diagnostic_tail.to_string());
    outcome.output_path = Some(candidate.to_path_buf());
    outcome.warnings = styling_loss_warning(input_ext).into_iter().collect();
    Ok(outcome)
}

/// نگاشت طبقه‌بندی شکست یک تلاش به (وضعیت، یادداشت کوتاه برای خلاصهٔ تلاش‌ها).
fn classify_failure(
    classification: cli::RunClassification,
    logs: &str,
) -> (SyncStatus, String) {
    match classification {
        // قانون ۱: marker کیفیت پایین بر کد خروج و marker آفست مقدم است
        cli::RunClassification::LowQuality => {
            let offset_note = cli::extract_offset_seconds(logs)
                .map(|offset| format!(" (آفست یافت‌شده: {offset:.3}s)"))
                .unwrap_or_default();
            (
                SyncStatus::UnchangedLowQuality,
                format!("کیفیت هم‌ترازی پایین — دست‌نخورده ماند{offset_note}"),
            )
        }
        cli::RunClassification::FailedExit(code) => {
            let code_text = code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "نامشخص".into());
            (
                SyncStatus::ToolExecutionFailed,
                format!("شکست اجرا با کد خروج {code_text}"),
            )
        }
        cli::RunClassification::Unknown => (
            SyncStatus::UnknownOutcome,
            "کد خروج صفر اما marker آفست معتبر نبود — نتیجه نامشخص".into(),
        ),
        cli::RunClassification::Synchronized { .. } => {
            unreachable!("Synchronized در مسیر شکست بررسی نمی‌شود")
        }
    }
}

// ---------------------------------------------------------------------
// تست‌ها — pure + فیک ffs (بدون ابزار واقعی؛ فقط cmd.exe خود ویندوز)
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "subsync-sync-test-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn sync_config() -> SyncConfig {
        SyncConfig {
            multi_segment: true,
            segment_count: 6,
            skip_intro_outro: true,
            skip_sync_on_low_quality: true,
        }
    }

    fn make_video(dir: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, b"fake-video-bytes").unwrap();
        path
    }

    fn make_fa_srt(dir: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, "1\r\n00:00:01,000 --> 00:00:02,000\r\nسلام دنیا\r\n").unwrap();
        path
    }

    /// محتوای فیک ffs — فقط ASCII با CRLF؛ مود از فایل کنارش خوانده می‌شود.
    /// مودهای flag-aware برای تست اجرای چندمرحله‌ای: `fullref-ok` بدون
    /// `--multi-segment-sync` موفق می‌شود و با آن marker کیفیت پایین می‌دهد
    /// (شبیه‌سازی: مرجع کامل جواب می‌دهد، نمونه‌برداری نه)؛ `fullref-lq` برعکس؛
    /// `all-lq` همیشه رد می‌کند. بقیه مودها سناریوهای fail-closed همیشگی‌اند.
    #[cfg(windows)]
    const FAKE_FFS_SCRIPT: &str = "@echo off\r\nsetlocal enabledelayedexpansion\r\nset MODE=\r\nif exist \"%~dp0.fake_ffs_mode\" set /p MODE=<\"%~dp0.fake_ffs_mode\"\r\nset OUT=\r\nset IN=\r\nset PREV=\r\nset MSS=\r\nfor %%A in (%*) do (\r\n  if \"!PREV!\"==\"-i\" set \"IN=%%~A\"\r\n  if \"!PREV!\"==\"-o\" set \"OUT=%%~A\"\r\n  if \"%%A\"==\"--multi-segment-sync\" set \"MSS=1\"\r\n  set \"PREV=%%A\"\r\n)\r\nif \"%MODE%\"==\"success\" goto success\r\nif \"%MODE%\"==\"fullref-ok\" goto fullref-ok\r\nif \"%MODE%\"==\"fullref-lq\" goto fullref-lq\r\nif \"%MODE%\"==\"all-lq\" goto all-lq\r\nif \"%MODE%\"==\"convert\" goto convert\r\nif \"%MODE%\"==\"latinfix\" goto latinfix\r\nif \"%MODE%\"==\"lowquality\" goto lowquality\r\nif \"%MODE%\"==\"nomarker\" goto nomarker\r\nif \"%MODE%\"==\"fail\" goto fail\r\nif \"%MODE%\"==\"nooutput\" goto nooutput\r\nif \"%MODE%\"==\"invalid\" goto invalid\r\nif \"%MODE%\"==\"slow\" goto slow\r\nexit /b 9\r\n\r\n:success\r\ncopy /y \"%IN%\" \"%OUT%\" >nul\r\necho score: 4.20 1>&2\r\necho offset seconds: 1.234 1>&2\r\nexit /b 0\r\n\r\n:fullref-ok\r\nif \"%MSS%\"==\"1\" goto all-lq\r\ncopy /y \"%IN%\" \"%OUT%\" >nul\r\necho score: 565358.000 1>&2\r\necho offset seconds: 0.000 1>&2\r\nexit /b 0\r\n\r\n:fullref-lq\r\nif \"%MSS%\"==\"1\" goto success_small\r\necho score: 210216.901 1>&2\r\necho offset seconds: 36.110 1>&2\r\necho low-quality alignment (^|offset^| 36.1s ^> 30.0s); leaving subtitles unmodified 1>&2\r\necho writing original (unsynced) output to \"%OUT%\" 1>&2\r\nexit /b 0\r\n\r\n:success_small\r\ncopy /y \"%IN%\" \"%OUT%\" >nul\r\necho score: 4.20 1>&2\r\necho offset seconds: 1.500 1>&2\r\nexit /b 0\r\n\r\n:all-lq\r\necho score: 210216.901 1>&2\r\necho offset seconds: 36.110 1>&2\r\necho low-quality alignment (^|offset^| 36.1s ^> 30.0s); leaving subtitles unmodified 1>&2\r\necho writing original (unsynced) output to \"%OUT%\" 1>&2\r\nexit /b 0\r\n\r\n:convert\r\nif not exist \"%~dp0fixture.srt\" exit /b 8\r\ncopy /y \"%~dp0fixture.srt\" \"%OUT%\" >nul\r\necho offset seconds: 0.500 1>&2\r\nexit /b 0\r\n\r\n:latinfix\r\nif not exist \"%~dp0fixture_latin.srt\" exit /b 8\r\ncopy /y \"%~dp0fixture_latin.srt\" \"%OUT%\" >nul\r\necho offset seconds: 2.000 1>&2\r\nexit /b 0\r\n\r\n:lowquality\r\ncopy /y \"%IN%\" \"%OUT%\" >nul\r\necho low-quality alignment (score too low); leaving subtitles unmodified 1>&2\r\necho writing original (unsynced) output to \"%OUT%\" 1>&2\r\nexit /b 0\r\n\r\n:nomarker\r\ncopy /y \"%IN%\" \"%OUT%\" >nul\r\necho sync finished 1>&2\r\nexit /b 0\r\n\r\n:fail\r\necho ffmpeg crashed badly 1>&2\r\nexit /b 3\r\n\r\n:nooutput\r\necho offset seconds: 1.500 1>&2\r\nexit /b 0\r\n\r\n:invalid\r\necho english only>\"%OUT%\"\r\necho offset seconds: 2.000 1>&2\r\nexit /b 0\r\n\r\n:slow\r\ncopy /y \"%IN%\" \"%OUT%\" >nul\r\necho offset seconds: 1.234 1>&2\r\nping -n 31 127.0.0.1 >nul\r\nexit /b 0\r\n";

    #[cfg(windows)]
    fn write_fake_ffs(dir: &Path, tag: &str, mode: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let exe = dir.join(format!("fake-ffs-{tag}.cmd"));
        std::fs::write(&exe, FAKE_FFS_SCRIPT).unwrap();
        std::fs::write(dir.join(".fake_ffs_mode"), mode).unwrap();
        exe
    }

    fn request_for(
        video: &Path,
        subtitle: &Path,
        ffs: &Path,
        workspace_root: Option<PathBuf>,
    ) -> SyncRequest {
        SyncRequest {
            video_path: video.to_path_buf(),
            subtitle_path: subtitle.to_path_buf(),
            ffsubsync_path: ffs.to_path_buf(),
            ffmpeg_path: None,
            config: sync_config(),
            workspace_root,
        }
    }

    /// هیچ workspaceای نباید بعد از شکست/لغو باقی بماند.
    fn assert_no_workspace_left(base: &Path) {
        let Ok(entries) = std::fs::read_dir(base) else {
            return; // ریشهٔ workspace اصلاً ساخته نشده
        };
        let leftovers: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(WORKSPACE_PREFIX)
            })
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn workspace_creation_is_collision_safe_and_removable() {
        let tmp = unique_temp_dir("ws");
        let a = SyncWorkspace::create(Some(&tmp)).expect("workspace اول");
        let b = SyncWorkspace::create(Some(&tmp)).expect("workspace دوم");
        assert_ne!(a.root, b.root);
        assert!(a.out_dir.is_dir() && a.log_dir.is_dir());
        assert!(a
            .output_path("V.S01E01")
            .ends_with("out/V.S01E01.fa.synced.srt"));
        assert!(a.remove().is_empty());
        assert!(b.remove().is_empty());
        assert!(!a.root.exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn styling_loss_warning_matches_ass_and_ssa_only() {
        assert!(styling_loss_warning("srt").is_none());
        assert!(styling_loss_warning("sub").is_none());
        assert!(styling_loss_warning("ass").is_some());
        assert!(styling_loss_warning("ssa").is_some());
    }

    #[test]
    fn invalid_input_is_rejected_before_any_execution() {
        let tmp = unique_temp_dir("invalid");
        let tools = tmp.join("tools");
        let fake = write_fake_ffs(&tools, "never-run", "success");
        let video = make_video(&tmp, "Video.S01E01.mkv");
        let sub = make_fa_srt(&tmp, "Video.S01E01.fa.srt");

        // زیرنویس لاتین — فارسی نیست
        let latin = tmp.join("Video.S01E02.fa.srt");
        std::fs::write(
            &latin,
            "1\r\n00:00:01,000 --> 00:00:02,000\r\nenglish only\r\n",
        )
        .unwrap();
        let outcome = run_sync(
            &request_for(&video, &latin, &fake, Some(tmp.join("ws"))),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(
            outcome.status,
            SyncStatus::InvalidInput,
            "{}",
            outcome.diagnostic
        );

        // فرمت پشتیبانی‌نشده
        let txt = tmp.join("notes.txt");
        std::fs::write(&txt, "hello").unwrap();
        let outcome = run_sync(
            &request_for(&video, &txt, &fake, Some(tmp.join("ws"))),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.status, SyncStatus::InvalidInput);
        assert!(outcome.diagnostic.contains("پشتیبانی نمی‌شود"));

        // ویدیوی ناموجود
        let outcome = run_sync(
            &request_for(&tmp.join("ghost.mkv"), &sub, &fake, Some(tmp.join("ws"))),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.status, SyncStatus::InvalidInput);

        // مسیر نسبی
        let outcome = run_sync(
            &SyncRequest {
                video_path: PathBuf::from("relative.mkv"),
                subtitle_path: sub.clone(),
                ffsubsync_path: fake.clone(),
                ffmpeg_path: None,
                config: sync_config(),
                workspace_root: None,
            },
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.status, SyncStatus::InvalidInput);

        // ابزار ناموجود — فرایند هرگز اجرا نشده
        let outcome = run_sync(
            &request_for(&video, &sub, &tmp.join("nope.exe"), Some(tmp.join("ws"))),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.status, SyncStatus::ToolMissing);

        assert_no_workspace_left(&tmp.join("ws"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(windows)]
    #[test]
    fn run_sync_success_produces_verified_candidate_and_keeps_inputs() {
        let tmp = unique_temp_dir("ok");
        let tools = tmp.join("tools");
        let fake = write_fake_ffs(&tools, "ok", "success");
        let video = make_video(&tmp, "Video.S01E01.mkv");
        let sub = make_fa_srt(&tmp, "Video.S01E01.fa.srt");
        let video_before = std::fs::read(&video).unwrap();
        let sub_before = std::fs::read(&sub).unwrap();

        let outcome = run_sync(
            &request_for(&video, &sub, &fake, Some(tmp.join("ws"))),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.status, SyncStatus::Synced, "{}", outcome.diagnostic);
        let output = outcome
            .output_path
            .expect("کاندیدای موفق باید مسیر داشته باشد");
        assert!(output.ends_with("out/Video.S01E01.fa.synced.srt"));
        assert!(output.is_file());
        // کاندیدا = محتوای سینک‌شده؛ اینجا فیک همان ورودی را کپی می‌کند
        assert_eq!(std::fs::read(&output).unwrap(), sub_before);
        assert!((outcome.offset_seconds.unwrap() - 1.234).abs() < 1e-9);
        assert!(outcome.warnings.is_empty());

        // workspace و لاگ برای عیب‌یابی نگه داشته می‌شوند
        assert!(output.parent().unwrap().parent().unwrap().is_dir());

        // ورودی‌ها دست‌نخورده
        assert_eq!(std::fs::read(&video).unwrap(), video_before);
        assert_eq!(std::fs::read(&sub).unwrap(), sub_before);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(windows)]
    #[test]
    fn full_reference_pass_succeeds_and_multi_segment_never_runs() {
        let tmp = unique_temp_dir("fullrefok");
        let tools = tmp.join("tools");
        let fake = write_fake_ffs(&tools, "fullrefok", "fullref-ok");
        let video = make_video(&tmp, "Video.S01E01.mkv");
        let sub = make_fa_srt(&tmp, "Video.S01E01.fa.srt");

        // config پیش‌فرض (multi_segment روشن): تلاش اول مرجع کامل موفق می‌شود؛
        // نمونه‌برداری صوت (که در این مود marker کیفیت پایین می‌دهد) نباید اجرا شود.
        let outcome = run_sync(
            &request_for(&video, &sub, &fake, Some(tmp.join("ws"))),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.status, SyncStatus::Synced, "{}", outcome.diagnostic);
        assert!((outcome.offset_seconds.unwrap() - 0.000).abs() < 1e-9);
        // تلاش اول مستقیم موفق شده — خلاصه‌ای از تلاش‌های قبلی وجود ندارد
        assert!(outcome.summary.is_none(), "{:?}", outcome.summary);
        let output = outcome.output_path.expect("کاندیدای موفق");
        assert!(output.is_file());
        // پوشهٔ لاگ تلاش اول ساخته شده؛ تلاش دوم هرگز اجرا نشده
        let ws_root = tmp.join("ws");
        let logs_dir = ws_root
            .read_dir()
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.file_name().to_string_lossy().starts_with(WORKSPACE_PREFIX))
            .expect("workspace موفق نگه داشته می‌شود")
            .path()
            .join("logs");
        assert!(logs_dir.join("pass-0").is_dir(), "{}", logs_dir.display());
        assert!(
            !logs_dir.join("pass-1").exists(),
            "تلاش دوم نباید اجرا شده باشد"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(windows)]
    #[test]
    fn multi_segment_fallback_rescues_when_full_reference_is_low_quality() {
        let tmp = unique_temp_dir("fullreflq");
        let tools = tmp.join("tools");
        let fake = write_fake_ffs(&tools, "fullreflq", "fullref-lq");
        let video = make_video(&tmp, "Video.S01E01.mkv");
        let sub = make_fa_srt(&tmp, "Video.S01E01.fa.srt");
        let sub_before = std::fs::read(&sub).unwrap();

        // تلاش اول (مرجع کامل) در این مود marker کیفیت پایین با آفست 36.110 می‌دهد؛
        // تلاش دوم (نمونه‌برداری صوت) موفق می‌شود.
        let outcome = run_sync(
            &request_for(&video, &sub, &fake, Some(tmp.join("ws"))),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.status, SyncStatus::Synced, "{}", outcome.diagnostic);
        assert!((outcome.offset_seconds.unwrap() - 1.500).abs() < 1e-9);
        let summary = outcome.summary.expect("خلاصهٔ تلاش‌های چندمرحله‌ای");
        assert!(summary.contains("مرجع کامل"), "{summary}");
        assert!(summary.contains("نمونه‌برداری صوت"), "{summary}");
        assert!(summary.contains("36.110"), "{summary}");
        assert!(summary.contains("موفق"), "{summary}");
        // ورودی دست‌نخورده
        assert_eq!(std::fs::read(&sub).unwrap(), sub_before);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(windows)]
    #[test]
    fn all_strategies_rejected_reports_lowquality_with_attempt_summary() {
        let tmp = unique_temp_dir("alllq");
        let tools = tmp.join("tools");
        let fake = write_fake_ffs(&tools, "alllq", "all-lq");
        let video = make_video(&tmp, "Video.S01E01.mkv");
        let sub = make_fa_srt(&tmp, "Video.S01E01.fa.srt");
        let sub_before = std::fs::read(&sub).unwrap();

        // هر دو استراتژی marker کیفیت پایین می‌دهند — نتیجه نهایی همان
        // UnchangedLowQuality است اما خلاصهٔ هر دو تلاش با آفست‌های یافت‌شده
        // در summary و diagnostic می‌آید تا «سینک دوبارهٔ بی‌نتیجه» قابل فهم باشد.
        let outcome = run_sync(
            &request_for(&video, &sub, &fake, Some(tmp.join("ws"))),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(
            outcome.status,
            SyncStatus::UnchangedLowQuality,
            "{}",
            outcome.diagnostic
        );
        assert!(outcome.output_path.is_none());
        let summary = outcome.summary.expect("خلاصهٔ تلاش‌ها");
        assert!(summary.contains("تلاش 1"), "{summary}");
        assert!(summary.contains("تلاش 2"), "{summary}");
        assert!(summary.contains("مرجع کامل"), "{summary}");
        assert!(summary.contains("نمونه‌برداری صوت"), "{summary}");
        assert!(summary.contains("36.110"), "{summary}");
        assert!(outcome.diagnostic.contains("leaving subtitles unmodified"));
        // ورودی دست‌نخورده و هیچ workspaceای باقی نمانده
        assert_eq!(std::fs::read(&sub).unwrap(), sub_before);
        assert_no_workspace_left(&tmp.join("ws"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(windows)]
    #[test]
    fn low_quality_output_is_removed_and_inputs_kept() {
        let tmp = unique_temp_dir("lq");
        let tools = tmp.join("tools");
        let fake = write_fake_ffs(&tools, "lq", "lowquality");
        let video = make_video(&tmp, "Video.S01E01.mkv");
        let sub = make_fa_srt(&tmp, "Video.S01E01.fa.srt");
        let sub_before = std::fs::read(&sub).unwrap();

        let outcome = run_sync(
            &request_for(&video, &sub, &fake, Some(tmp.join("ws"))),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.status, SyncStatus::UnchangedLowQuality);
        assert!(outcome.diagnostic.contains("leaving subtitles unmodified"));
        assert!(outcome.output_path.is_none());
        // هیچ کاندیدایی باقی نمانده و ورودی دست‌نخورده است
        assert_no_workspace_left(&tmp.join("ws"));
        assert_eq!(std::fs::read(&sub).unwrap(), sub_before);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(windows)]
    #[test]
    fn zero_exit_without_marker_is_unknown_and_cleaned() {
        let tmp = unique_temp_dir("nomarker");
        let tools = tmp.join("tools");
        let fake = write_fake_ffs(&tools, "nm", "nomarker");
        let video = make_video(&tmp, "Video.S01E01.mkv");
        let sub = make_fa_srt(&tmp, "Video.S01E01.fa.srt");

        let outcome = run_sync(
            &request_for(&video, &sub, &fake, Some(tmp.join("ws"))),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.status, SyncStatus::UnknownOutcome);
        assert!(outcome.output_path.is_none());
        assert_no_workspace_left(&tmp.join("ws"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(windows)]
    #[test]
    fn non_zero_exit_is_tool_execution_failure() {
        let tmp = unique_temp_dir("fail");
        let tools = tmp.join("tools");
        let fake = write_fake_ffs(&tools, "fail", "fail");
        let video = make_video(&tmp, "Video.S01E01.mkv");
        let sub = make_fa_srt(&tmp, "Video.S01E01.fa.srt");

        let outcome = run_sync(
            &request_for(&video, &sub, &fake, Some(tmp.join("ws"))),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.status, SyncStatus::ToolExecutionFailed);
        assert!(outcome.diagnostic.contains("کد خروج 3"));
        assert!(outcome.output_path.is_none());
        assert_no_workspace_left(&tmp.join("ws"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(windows)]
    #[test]
    fn missing_and_invalid_outputs_are_rejected() {
        let tmp = unique_temp_dir("badout");
        let tools = tmp.join("tools");

        // marker آفست هست اما فایل ساخته نشده
        let fake = write_fake_ffs(&tools, "noout", "nooutput");
        let video = make_video(&tmp, "Video.S01E01.mkv");
        let sub = make_fa_srt(&tmp, "Video.S01E01.fa.srt");
        let outcome = run_sync(
            &request_for(&video, &sub, &fake, Some(tmp.join("ws"))),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.status, SyncStatus::OutputValidationFailed);
        assert!(outcome.diagnostic.contains("ساخته نشد"));

        // خروجی ساختاری معتبر اما بی‌محتوای فارسی — دور ریخته می‌شود
        std::fs::write(
            tools.join("fixture_latin.srt"),
            "1\r\n00:00:01,000 --> 00:00:02,000\r\nenglish only\r\n",
        )
        .unwrap();
        let fake = write_fake_ffs(&tools, "latin", "latinfix");
        let outcome = run_sync(
            &request_for(&video, &sub, &fake, Some(tmp.join("ws"))),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.status, SyncStatus::OutputValidationFailed);
        assert!(outcome.diagnostic.contains("فارسی ندارد"));

        assert_no_workspace_left(&tmp.join("ws"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(windows)]
    #[test]
    fn ass_input_success_carries_styling_loss_warning() {
        let tmp = unique_temp_dir("ass");
        let tools = tmp.join("tools");
        let fake = write_fake_ffs(&tools, "convert", "convert");
        // فیک خروجی SRT معتبر فارسی را از fixture می‌سازد (شبیه‌سازی تبدیل pysubs2)
        std::fs::write(
            tools.join("fixture.srt"),
            "1\r\n00:00:01,000 --> 00:00:02,000\r\nسلام دنیا\r\n",
        )
        .unwrap();
        let video = make_video(&tmp, "Video.S01E01.mkv");
        let ass = tmp.join("Video.S01E01.fa.ass");
        std::fs::write(
            &ass,
            "[Script Info]\r\nTitle: t\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\r\nStyle: Default,Arial,20,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,سلام\r\n",
        )
        .unwrap();

        let outcome = run_sync(
            &request_for(&video, &ass, &fake, Some(tmp.join("ws"))),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.status, SyncStatus::Synced, "{}", outcome.diagnostic);
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("استایل"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(windows)]
    #[test]
    fn cancellation_kills_tree_and_leaves_no_candidate() {
        let tmp = unique_temp_dir("cancel");
        let tools = tmp.join("tools");
        let fake = write_fake_ffs(&tools, "slow", "slow");
        let video = make_video(&tmp, "Video.S01E01.mkv");
        let sub = make_fa_srt(&tmp, "Video.S01E01.fa.srt");

        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        // لغو درست هنگام ورود به مرحلهٔ اجرا — قطعی، بدون sleep
        let mut on_progress = |p: SyncProgress| {
            if p.stage == SyncStage::Running {
                cancel.store(true, Ordering::Relaxed);
            }
        };
        let outcome = run_sync(
            &request_for(&video, &sub, &fake, Some(tmp.join("ws"))),
            &cancel,
            &mut on_progress,
        );
        let elapsed = started.elapsed();

        assert_eq!(
            outcome.status,
            SyncStatus::Cancelled,
            "{}",
            outcome.diagnostic
        );
        assert!(outcome.output_path.is_none());
        // خاتمهٔ درخت نباید منتظر ping ۳۰ ثانیه‌ای فیک بماند
        assert!(elapsed < std::time::Duration::from_secs(20), "{elapsed:?}");
        assert_no_workspace_left(&tmp.join("ws"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn preexisting_candidate_is_never_overwritten() {
        let tmp = unique_temp_dir("guard");
        let video = make_video(&tmp, "Video.S01E01.mkv");
        let sub = make_fa_srt(&tmp, "Video.S01E01.fa.srt");
        let workspace = SyncWorkspace::create(Some(&tmp.join("ws"))).unwrap();
        let output = workspace.output_path("Video.S01E01");
        std::fs::write(&output, b"PREEXISTING").unwrap();

        // حتی اگر «ابزار» همه‌چیز را موفق گزارش می‌کرد، نباید اجرا شود
        let fake = write_fake_ffs(&tmp.join("tools"), "would-win", "success");
        let outcome = execute_in_workspace(
            &request_for(&video, &sub, &fake, None),
            &workspace,
            "srt",
            "Video.S01E01",
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.status, SyncStatus::Failed);
        assert!(outcome.diagnostic.contains("بازنویسی ممنوع"));
        // محتوای قبلی دست‌نخورده
        assert_eq!(std::fs::read(&output).unwrap(), b"PREEXISTING");
        workspace.remove();
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// تست زندهٔ نصب‌شده: ffmpeg واقعی + ffs واقعی لازم است (مثل سایر تست‌های
    /// `#[ignore]`؛ با `cargo test --lib -- --ignored sync` اجرا شود).
    /// ویدیوی سینتتیک (رنگ + sine) می‌سازد و کل آداپتور را روی ابزار واقعی می‌سنجد.
    #[test]
    #[ignore]
    fn live_ffsubsync_syncs_synthetic_video() {
        use crate::tools::resolve_tool_path;
        use std::process::Command;

        let ffmpeg = resolve_tool_path("ffmpeg", &Default::default()).expect("ffmpeg در PATH");
        let ffs = resolve_tool_path("ffsubsync", &Default::default()).expect("ffs نصب‌شده");

        let tmp = unique_temp_dir("live");
        std::fs::create_dir_all(&tmp).unwrap();
        let video = tmp.join("Live.S01E01.mkv");
        let status = Command::new(&ffmpeg)
            .args([
                "-y",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=320x240:d=6",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440",
                "-shortest",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac",
                "-map",
                "0:v",
                "-map",
                "1:a",
            ])
            .arg(&video)
            .status()
            .expect("اجرای ffmpeg");
        assert!(status.success(), "ساخت ویدیوی سینتتیک ناموفق بود");

        let sub = tmp.join("Live.S01E01.fa.srt");
        std::fs::write(
            &sub,
            "1\r\n00:00:01,000 --> 00:00:02,000\r\nسلام\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\nدنیا\r\n",
        )
        .unwrap();

        let request = SyncRequest {
            video_path: video,
            subtitle_path: sub,
            ffsubsync_path: ffs,
            ffmpeg_path: Some(ffmpeg),
            config: sync_config(),
            workspace_root: None,
        };
        let outcome = run_sync(&request, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(outcome.status, SyncStatus::Synced, "{}", outcome.diagnostic);
        assert!(outcome.output_path.expect("candidate").is_file());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -------------------------------------------------------------------
    // تست‌های زندهٔ real-world — فقط اجرای دستی؛ CI هرگز اجرایشان نمی‌کند.
    // مسیرها از متغیرهای محیطی می‌آیند و هرگز در ریپو کامیت نمی‌شوند:
    //   SUBSYNC_REAL_SYNC_VIDEO  = ویدیوی واقعی دارای Track داخلی فارسی
    //   SUBSYNC_REAL_SYNC_SUB    = sidecar فارسی هم‌نام آن ویدیو
    //   SUBSYNC_REAL_FFSUBSYNC   = مسیر ffs.exe واقعی (fallback: resolver)
    //   SUBSYNC_REAL_FFMPEG      = مسیر ffmpeg.exe واقعی (fallback: resolver)
    // ورودی‌ها هرگز تغییر نمی‌کنند (hash قبل/بعد) و خروجی در temp پاک می‌شود.
    // -------------------------------------------------------------------

    fn live_tool(env_key: &str, tool_id: &str) -> PathBuf {
        std::env::var_os(env_key)
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                crate::tools::resolve_tool_path(tool_id, &Default::default())
                    .unwrap_or_else(|| {
                        panic!("مسیر ابزار واقعی یافت نشد — {env_key} تنظیم شود یا در PATH باشد")
                    })
            })
    }

    fn live_media(env_key: &str) -> PathBuf {
        let path = PathBuf::from(
            std::env::var_os(env_key)
                .unwrap_or_else(|| panic!("{env_key} تنظیم نشده — اجرای دستی real-world است")),
        );
        assert!(
            path.is_file(),
            "«{}» فایل موجودی نیست",
            path.display()
        );
        path
    }

    /// hash استریمی فایل — فقط برای زیرنویس (کوچک)؛ برای ویدیوی بزرگ از
    /// `video_fingerprint` استفاده می‌شود تا تست در build debug هم سریع بماند.
    fn stream_sha256(path: &Path) -> String {
        use sha2::{Digest, Sha256};
        let mut file = std::fs::File::open(path).expect("باز کردن برای hash");
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            use std::io::Read;
            let n = file.read(&mut buf).expect("خواندن برای hash");
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        format!("{:x}", hasher.finalize())
    }

    /// اثر انگشت ویدیوی بزرگ (اندازه + mtime دقیق) — sync هرگز ورودی را
    /// نمی‌نویسد؛ تغییر اندازه یا مُهر زمان کافی است که دست‌خوردگی را لو بدهد.
    fn video_fingerprint(path: &Path) -> String {
        let meta = std::fs::symlink_metadata(path).expect("متادیتا برای hash");
        format!("{}|{:?}", meta.len(), meta.modified())
    }

    /// همهٔ تایم‌استمپ‌های SRT را delta ثانیه جابه‌جا می‌کند — برای ساخت
    /// کنترلِ مثبتِ «زیرنویس واقعاً ناهماهنگ».
    fn shift_srt_seconds(content: &str, delta_seconds: f64) -> String {
        let re = regex::Regex::new(r"(\d{2}):(\d{2}):(\d{2})[,.](\d{3})").unwrap();
        re.replace_all(content, |caps: &regex::Captures| {
            let h: f64 = caps[1].parse().unwrap();
            let m: f64 = caps[2].parse().unwrap();
            let s: f64 = caps[3].parse().unwrap();
            let ms: f64 = caps[4].parse().unwrap();
            let total = (h * 3600.0 + m * 60.0 + s + ms / 1000.0 + delta_seconds).max(0.0);
            let th = (total / 3600.0).floor();
            let tm = ((total - th * 3600.0) / 60.0).floor();
            let ts = (total - th * 3600.0 - tm * 60.0).floor();
            let tms = ((total - total.floor()) * 1000.0).round();
            format!("{th:02}:{tm:02}:{ts:02},{tms:03}")
        })
        .into_owned()
    }

    fn discard_workspace(outcome: &mut SyncOutcome) {
        if let Some(handle) = outcome.take_workspace() {
            for warning in handle.discard() {
                assert!(warning.is_empty(), "{warning}");
            }
        }
    }

    /// سناریوی واقعی کاربر: sidecar هم‌نامِ یک ویدیو که Track داخلیِ سینک‌شده
    /// دارد (مثل رلیزهای Film2Media) — مرجع کامل باید آفست ≈0 پیدا کند و سینک
    /// با موفقیت تمام شود، نه marker «کیفیت پایین».
    #[test]
    #[ignore = "اجرای دستی روی رسانهٔ واقعی — SUBSYNC_REAL_SYNC_VIDEO/SUB لازم است"]
    fn live_real_already_synced_sidecar_yields_near_zero_offset() {
        let video = live_media("SUBSYNC_REAL_SYNC_VIDEO");
        let sub = live_media("SUBSYNC_REAL_SYNC_SUB");
        let ffs = live_tool("SUBSYNC_REAL_FFSUBSYNC", "ffsubsync");
        let ffmpeg = live_tool("SUBSYNC_REAL_FFMPEG", "ffmpeg");

        let video_hash = video_fingerprint(&video);
        let sub_hash = stream_sha256(&sub);

        let request = SyncRequest {
            video_path: video.clone(),
            subtitle_path: sub.clone(),
            ffsubsync_path: ffs,
            ffmpeg_path: Some(ffmpeg),
            config: sync_config(),
            workspace_root: None,
        };
        let mut outcome = run_sync(&request, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(outcome.status, SyncStatus::Synced, "{}", outcome.diagnostic);
        let offset = outcome.offset_seconds.expect("آفست برای Synced");
        assert!(
            offset.abs() < 0.05,
            "زیرنویس از قبل سینک است؛ آفست انتظار ≈0 بود: {offset} — {summary}",
            summary = outcome.summary.as_deref().unwrap_or("-")
        );
        assert!(outcome.output_path.as_ref().expect("کاندیدا").is_file());
        discard_workspace(&mut outcome);

        // ورودی‌ها دست‌نخورده
        assert_eq!(video_fingerprint(&video), video_hash, "ویدیو تغییر کرده!");
        assert_eq!(stream_sha256(&sub), sub_hash, "زیرنویس تغییر کرده!");
    }

    /// کنترل مثبت: همان زیرنویس با شیفت واقعی ‎+۲۵ ثانیه — مرجع کامل باید
    /// اصلاحِ ≈−۲۵ ثانیه را پیدا کند (قرارداد ffsubsync: «offset seconds» =
    /// اصلاحی که به ورودی اعمال می‌شود؛ ورودیِ ۲۵ ثانیه جلوتر → اصلاح −25s).
    #[test]
    #[ignore = "اجرای دستی روی رسانهٔ واقعی — SUBSYNC_REAL_SYNC_VIDEO/SUB لازم است"]
    fn live_real_shifted_sidecar_is_corrected_by_about_25_seconds() {
        let video = live_media("SUBSYNC_REAL_SYNC_VIDEO");
        let sub = live_media("SUBSYNC_REAL_SYNC_SUB");
        let ffs = live_tool("SUBSYNC_REAL_FFSUBSYNC", "ffsubsync");
        let ffmpeg = live_tool("SUBSYNC_REAL_FFMPEG", "ffmpeg");

        let video_hash = video_fingerprint(&video);
        let sub_hash = stream_sha256(&sub);

        let tmp = unique_temp_dir("liveshift");
        std::fs::create_dir_all(&tmp).unwrap();
        let shifted = tmp.join("Live.S01E01.fa.srt");
        std::fs::write(
            &shifted,
            shift_srt_seconds(&std::fs::read_to_string(&sub).unwrap(), 25.0),
        )
        .unwrap();

        let request = SyncRequest {
            video_path: video.clone(),
            subtitle_path: shifted,
            ffsubsync_path: ffs,
            ffmpeg_path: Some(ffmpeg),
            config: sync_config(),
            workspace_root: None,
        };
        let mut outcome = run_sync(&request, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(outcome.status, SyncStatus::Synced, "{}", outcome.diagnostic);
        let offset = outcome.offset_seconds.expect("آفست برای Synced");
        assert!(
            (offset + 25.0).abs() <= 0.5,
            "اصلاح انتظار ≈−25s بود: {offset} — {summary}",
            summary = outcome.summary.as_deref().unwrap_or("-")
        );
        discard_workspace(&mut outcome);

        // زیرنویس اصلی دست‌نخورده (کپیِ شیفت‌شده در tmp ورودی بود)
        assert_eq!(video_fingerprint(&video), video_hash, "ویدیو تغییر کرده!");
        assert_eq!(stream_sha256(&sub), sub_hash, "زیرنویس اصلی تغییر کرده!");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
