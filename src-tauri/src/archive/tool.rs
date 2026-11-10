//! اجرای امن 7-Zip و مدیریت پوشه‌های موقتِ تحت مالکیت برنامه.
//!
//! - بدون shell؛ مسیرها به‌صورت آرگومان جداگانه و بعد از `--` پاس می‌شوند.
//! - خروجی به فایل لاگ داخل پوشهٔ موقت خود برنامه هدایت می‌شود تا pipe پر
//!   نشود و لغو/تایم‌اوت قابل کنترل باشد.
//! - کد خروج ۰ موفق، ۱ هشدار (ادامه) و ۲+ شکست است.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::subsource::staging::STAGING_DIR_PREFIX;

use super::listing::{parse_listing, ArchiveMember};
use super::types::EXTRACT_DIR_PREFIX;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const TOOL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::Cancelled => write!(f, "عملیات لغو شد"),
            ToolError::Failed(m) => write!(f, "{m}"),
        }
    }
}

pub struct ToolRun {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// اجرای 7-Zip با هدایت خروجی به فایل و کنترل لغو/تایم‌اوت.
pub fn run_7z(
    seven_zip: &Path,
    args: &[OsString],
    log_dir: &Path,
    cancel: &AtomicBool,
) -> Result<ToolRun, ToolError> {
    fs::create_dir_all(log_dir)
        .map_err(|e| ToolError::Failed(format!("ساخت پوشهٔ لاگ ابزار ناموفق: {e}")))?;
    let out_path = log_dir.join("sevenzip.stdout.log");
    let err_path = log_dir.join("sevenzip.stderr.log");
    let out_file = fs::File::create(&out_path)
        .map_err(|e| ToolError::Failed(format!("ساخت فایل لاگ stdout ناموفق: {e}")))?;
    let err_file = fs::File::create(&err_path)
        .map_err(|e| ToolError::Failed(format!("ساخت فایل لاگ stderr ناموفق: {e}")))?;

    let mut command = Command::new(seven_zip);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command
        .spawn()
        .map_err(|e| ToolError::Failed(format!("اجرای 7-Zip ناموفق بود: {e}")))?;

    let started = Instant::now();
    let exit_status = loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ToolError::Cancelled);
        }
        if started.elapsed() > TOOL_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ToolError::Failed(
                "اجرای 7-Zip از حد زمانی مجاز بیشتر شد و متوقف شد".into(),
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ToolError::Failed(format!(
                    "انتظار برای پایان 7-Zip ناموفق بود: {e}"
                )));
            }
        }
    };

    let stdout = fs::read(&out_path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let stderr = fs::read(&err_path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();

    Ok(ToolRun {
        exit_code: exit_status.code(),
        stdout,
        stderr,
    })
}

/// فهرست اعضای آرشیو با اجرای `7z l -slt -ba`.
/// خروج ۱ → هشدار و در صورت وجود خروجی معتبر ادامه؛ ۲+ → شکست.
pub fn list_archive(
    seven_zip: &Path,
    archive: &Path,
    log_dir: &Path,
    cancel: &AtomicBool,
) -> Result<(Vec<ArchiveMember>, Vec<String>), ToolError> {
    let args = vec![
        OsString::from("l"),
        OsString::from("-slt"),
        OsString::from("-ba"),
        OsString::from("--"),
        archive.as_os_str().to_os_string(),
    ];
    let run = run_7z(seven_zip, &args, log_dir, cancel)?;
    let mut warnings = Vec::new();

    match run.exit_code {
        Some(0) => {}
        Some(1) => warnings.push(format!(
            "7-Zip هنگام فهرست‌کردن «{}» هشدار داد (خروج ۱){}",
            archive.display(),
            stderr_suffix(&run.stderr)
        )),
        Some(code) => {
            return Err(ToolError::Failed(format!(
                "فهرست‌کردن آرشیو «{}» با کد خروج {code} ناموفق بود{}",
                archive.display(),
                stderr_suffix(&run.stderr)
            )));
        }
        None => {
            return Err(ToolError::Failed(format!(
                "7-Zip هنگام فهرست‌کردن «{}» کد خروجی برنگرداند (فرایند به‌طور غیرعادی پایان یافت){}",
                archive.display(),
                stderr_suffix(&run.stderr)
            )));
        }
    }

    let members = parse_listing(&run.stdout).map_err(ToolError::Failed)?;
    Ok((members, warnings))
}

/// استخراج کامل یک آرشیو به `out_dir` با `7z x -y`.
/// خروج ۱ → هشدار؛ ۲+ → شکست.
pub fn extract_archive(
    seven_zip: &Path,
    archive: &Path,
    out_dir: &Path,
    log_dir: &Path,
    cancel: &AtomicBool,
) -> Result<Vec<String>, ToolError> {
    let mut out_switch = OsString::from("-o");
    out_switch.push(out_dir.as_os_str());
    let args = vec![
        OsString::from("x"),
        OsString::from("-y"),
        OsString::from("-bd"),
        out_switch,
        OsString::from("--"),
        archive.as_os_str().to_os_string(),
    ];
    let run = run_7z(seven_zip, &args, log_dir, cancel)?;

    match run.exit_code {
        Some(0) => Ok(Vec::new()),
        Some(1) => Ok(vec![format!(
            "7-Zip هنگام استخراج «{}» هشدار داد (خروج ۱) — فایل‌های استخراج‌شده اعتبارسنجی شدند{}",
            archive.display(),
            stderr_suffix(&run.stderr)
        )]),
        Some(code) => Err(ToolError::Failed(format!(
            "استخراج آرشیو «{}» با کد خروج {code} ناموفق بود{}",
            archive.display(),
            stderr_suffix(&run.stderr)
        ))),
        None => Err(ToolError::Failed(format!(
            "7-Zip هنگام استخراج «{}» کد خروجی برنگرداند{}",
            archive.display(),
            stderr_suffix(&run.stderr)
        ))),
    }
}

fn stderr_suffix(stderr: &str) -> String {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        let short: String = trimmed.chars().take(400).collect();
        format!(" — {short}")
    }
}

/// پوشهٔ موقت استخراج/لاگ — همیشه دقیقاً داخل ریشهٔ staging و با پیشوند
/// اختصاصی برنامه. پاک‌سازی فقط همین پوشه را هدف می‌گیرد.
pub struct ExtractWorkspace {
    root: PathBuf,
    staging_root: PathBuf,
    out_dir: PathBuf,
    log_dir: PathBuf,
    cleaned: bool,
}

impl ExtractWorkspace {
    pub fn create(staging_root: &Path) -> Result<Self, ToolError> {
        let staging_root = validate_staging_root(staging_root)?;
        let mut last_err = None;

        for _ in 0..8 {
            let name = format!(
                "{EXTRACT_DIR_PREFIX}{}",
                crate::subsource::staging::new_uid()
            );
            let root = staging_root.join(name);
            match fs::create_dir(&root) {
                Ok(()) => {
                    let canonical_root = fs::canonicalize(&root).map_err(|e| {
                        ToolError::Failed(format!(
                            "canonicalize پوشهٔ موقت «{}» ناموفق: {e}",
                            root.display()
                        ))
                    })?;
                    if !canonical_root.starts_with(&staging_root) {
                        let _ = fs::remove_dir_all(&root);
                        return Err(ToolError::Failed(
                            "پوشهٔ موقت از ریشهٔ staging خارج شد — متوقف شد".into(),
                        ));
                    }
                    let out_dir = canonical_root.join("out");
                    let log_dir = canonical_root.join("logs");
                    if let Err(e) = fs::create_dir(&out_dir).and_then(|_| fs::create_dir(&log_dir))
                    {
                        let _ = fs::remove_dir_all(&canonical_root);
                        return Err(ToolError::Failed(format!(
                            "ساخت زیرپوشه‌های موقت ناموفق: {e}"
                        )));
                    }
                    return Ok(Self {
                        root: canonical_root,
                        staging_root,
                        out_dir,
                        log_dir,
                        cleaned: false,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_err = Some(e);
                    continue;
                }
                Err(e) => {
                    return Err(ToolError::Failed(format!(
                        "ساخت پوشهٔ موقت استخراج ناموفق: {e}"
                    )));
                }
            }
        }

        Err(ToolError::Failed(format!(
            "ساخت پوشهٔ موقت یکتا ناموفق بود: {}",
            last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "برخورد نامشخص".into())
        )))
    }

    pub fn out_dir(&self) -> &Path {
        &self.out_dir
    }

    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    /// پاک‌سازی صریح؛ خطا برای گزارش به کاربر برگردانده می‌شود.
    /// فقط پوشهٔ موقتی که خودمان ساختیم حذف می‌شود.
    pub fn cleanup(&mut self) -> Result<(), String> {
        if self.cleaned {
            return Ok(());
        }
        self.cleaned = true;
        if !self.root.starts_with(&self.staging_root)
            || !self
                .root
                .file_name()
                .map(|n| n.to_string_lossy().starts_with(EXTRACT_DIR_PREFIX))
                .unwrap_or(false)
        {
            return Err(format!(
                "پاک‌سازی «{}» رد شد: پوشه در مالکیت برنامه نیست",
                self.root.display()
            ));
        }
        match fs::remove_dir_all(&self.root) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!(
                "حذف پوشهٔ موقت «{}» ناموفق بود: {e}",
                self.root.display()
            )),
        }
    }
}

impl Drop for ExtractWorkspace {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

/// اعتبارسنجی ریشهٔ staging: مطلق، موجود، پوشهٔ واقعی و با پیشوند اختصاصی.
pub fn validate_staging_root(path: &Path) -> Result<PathBuf, ToolError> {
    if !path.is_absolute() {
        return Err(ToolError::Failed(format!(
            "ریشهٔ staging «{}» مطلق نیست",
            path.display()
        )));
    }
    let meta = fs::symlink_metadata(path).map_err(|e| {
        ToolError::Failed(format!(
            "ریشهٔ staging «{}» قابل دسترسی نیست: {e}",
            path.display()
        ))
    })?;
    if super::listing::is_link_or_reparse(&meta) {
        return Err(ToolError::Failed(format!(
            "ریشهٔ staging «{}» symlink/reparse است — رد شد",
            path.display()
        )));
    }
    if !meta.is_dir() {
        return Err(ToolError::Failed(format!(
            "ریشهٔ staging «{}» یک پوشه نیست",
            path.display()
        )));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if !name.starts_with(STAGING_DIR_PREFIX) {
        return Err(ToolError::Failed(format!(
            "پوشهٔ «{}» ریشهٔ staging معتبر برنامه نیست (پیشوند {STAGING_DIR_PREFIX} ندارد)",
            path.display()
        )));
    }
    fs::canonicalize(path).map_err(|e| {
        ToolError::Failed(format!(
            "canonicalize ریشهٔ staging «{}» ناموفق: {e}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_root_requires_marker_prefix() {
        let base = std::env::temp_dir().join(format!(
            "subsync-archive-tool-{}-{}",
            std::process::id(),
            crate::subsource::staging::new_uid()
        ));
        fs::create_dir_all(&base).unwrap();
        assert!(validate_staging_root(&base).is_err());
        let good = base.join(format!("{STAGING_DIR_PREFIX}x"));
        fs::create_dir(&good).unwrap();
        assert!(validate_staging_root(&good).is_ok());
        let _ = fs::remove_dir_all(&base);
    }
}
