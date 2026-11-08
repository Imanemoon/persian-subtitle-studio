//! SubSource CLI execution adapter — safe process management with testability.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::logging;
use crate::subsource::error::{SubSourceError, SubSourceResult};
use crate::subsource::types::{BatchOptions, BatchResult};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// CLI adapter برای اجرای sub-rs.exe
///
/// این ساختار لایه‌ای امن روی `std::process::Command` است که:
/// - آرگومان‌ها را به‌صورت جداگانه می‌فرستد (نه shell string)
/// - API key را از لاگ‌ها redact می‌کند
/// - خطاها را دسته‌بندی می‌کند
/// - قابل mock است (trait-based design برای PRهای بعدی)
pub struct SubSourceCli {
    exe_path: String,
}

impl SubSourceCli {
    /// ساخت adapter با مسیر executable
    ///
    /// # Arguments
    /// * `exe_path` - مسیر کامل sub-rs.exe
    ///
    /// # Returns
    /// خطا اگر فایل وجود نداشته باشد یا قابل اجرا نباشد
    pub fn new(exe_path: impl Into<String>) -> SubSourceResult<Self> {
        let path = exe_path.into();
        let p = Path::new(&path);

        if !p.exists() {
            return Err(SubSourceError::ToolNotFound(format!(
                "فایل وجود ندارد: {path}"
            )));
        }

        if !p.is_file() {
            return Err(SubSourceError::ToolNotFound(format!(
                "مسیر یک پوشه است، نه فایل: {path}"
            )));
        }

        Ok(Self { exe_path: path })
    }

    /// اجرای batch download
    ///
    /// # Arguments
    /// * `options` - تنظیمات اجرا (directory, API key, etc.)
    ///
    /// # Returns
    /// نتیجه اجرا شامل exit code، stdout، stderr و مدت زمان
    ///
    /// # Errors
    /// - `InvalidDirectory` اگر directory وجود نداشته باشد
    /// - `ProcessError` اگر spawn ناموفق باشد
    /// - `CliExitCode` اگر CLI با کد غیر صفر خارج شود
    pub fn run_batch(&self, options: &BatchOptions) -> SubSourceResult<BatchResult> {
        // اعتبارسنجی
        options.validate().map_err(SubSourceError::Other)?;

        let dir = Path::new(&options.directory);
        if !dir.exists() {
            return Err(SubSourceError::InvalidDirectory(format!(
                "پوشه وجود ندارد: {}",
                options.directory
            )));
        }
        if !dir.is_dir() {
            return Err(SubSourceError::InvalidDirectory(format!(
                "مسیر یک فایل است، نه پوشه: {}",
                options.directory
            )));
        }

        // ساخت دستور
        let args = options.to_cli_args();

        // لاگ با redact کردن API key
        let safe_args = Self::redact_sensitive_args(&args);
        logging::info(
            "subsource",
            &format!(
                "اجرای SubSource batch: {} {}",
                self.exe_path,
                safe_args.join(" ")
            ),
        );

        let start = Instant::now();

        // اجرای فرایند
        let mut cmd = Command::new(&self.exe_path);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(dir); // اجرا در directory هدف

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| SubSourceError::ProcessError(format!("spawn: {e}")))?;

        // خواندن stdout و stderr در threadهای جدا
        let mut stdout_pipe = child.stdout.take().expect("stdout piped");
        let mut stderr_pipe = child.stderr.take().expect("stderr piped");

        let stdout_thread = std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            let _ = stdout_pipe.read_to_string(&mut s);
            s
        });

        let stderr_thread = std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            let _ = stderr_pipe.read_to_string(&mut s);
            s
        });

        // انتظار برای اتمام
        let status = child
            .wait()
            .map_err(|e| SubSourceError::ProcessError(format!("wait: {e}")))?;

        let stdout = stdout_thread.join().unwrap_or_default();
        let stderr = stderr_thread.join().unwrap_or_default();
        let duration = start.elapsed().as_secs_f64();

        // تحلیل نتیجه
        let exit_code = status.code().unwrap_or(-1);

        if status.success() {
            logging::info(
                "subsource",
                &format!("SubSource batch موفق ({:.1}s)", duration),
            );
            Ok(BatchResult::success(stdout, stderr, duration))
        } else {
            let result = BatchResult::failure(exit_code, stdout, stderr, duration);

            // تشخیص نوع خطا
            if result.is_auth_error() {
                logging::error("subsource", "خطای احراز هویت SubSource API");
                return Err(SubSourceError::AuthenticationRequired(
                    "API key نامعتبر یا منقضی شده".into(),
                ));
            }

            if result.is_network_error() {
                logging::warn(
                    "subsource",
                    &format!("خطای شبکه SubSource (کد {exit_code})"),
                );
            } else {
                logging::warn(
                    "subsource",
                    &format!("SubSource batch ناموفق (کد {exit_code})"),
                );
            }

            Err(SubSourceError::CliExitCode {
                code: exit_code,
                stderr: result.stderr.clone(),
            })
        }
    }

    /// Redact کردن API key و credentialهای حساس از آرگومان‌ها
    fn redact_sensitive_args(args: &[String]) -> Vec<String> {
        let mut redacted = Vec::with_capacity(args.len());
        let mut redact_next = false;

        for arg in args {
            if redact_next {
                redacted.push("<REDACTED>".to_string());
                redact_next = false;
            } else if arg == "--api-key" || arg == "--proxy" || arg == "--worker" {
                redacted.push(arg.clone());
                redact_next = true;
            } else {
                redacted.push(arg.clone());
            }
        }

        redacted
    }

    /// دریافت مسیر executable
    pub fn exe_path(&self) -> &str {
        &self.exe_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_nonexistent_path() {
        let result = SubSourceCli::new("C:\\not\\a\\real\\path\\sub-rs.exe");
        assert!(matches!(result, Err(SubSourceError::ToolNotFound(_))));
    }

    #[test]
    fn redact_sensitive_args_hides_api_key() {
        let args = vec![
            "--directory".to_string(),
            "/videos".to_string(),
            "--api-key".to_string(),
            "secret-key-12345".to_string(),
            "--top".to_string(),
            "5".to_string(),
            "--proxy".to_string(),
            "http://proxy:8080".to_string(),
        ];

        let redacted = SubSourceCli::redact_sensitive_args(&args);
        assert_eq!(redacted[0], "--directory");
        assert_eq!(redacted[1], "/videos");
        assert_eq!(redacted[2], "--api-key");
        assert_eq!(redacted[3], "<REDACTED>");
        assert_eq!(redacted[4], "--top");
        assert_eq!(redacted[5], "5");
        assert_eq!(redacted[6], "--proxy");
        assert_eq!(redacted[7], "<REDACTED>");

        // تأیید اینکه secret واقعی در خروجی نیست
        let combined = redacted.join(" ");
        assert!(!combined.contains("secret-key-12345"));
        assert!(!combined.contains("proxy:8080"));
    }

    #[test]
    fn batch_options_validation_in_run_batch() {
        #[cfg(windows)]
        {
            // از cmd.exe به‌عنوان fake executable استفاده می‌کنیم
            let cli = SubSourceCli::new("C:\\Windows\\System32\\cmd.exe").unwrap();

            let opts = BatchOptions {
                directory: "C:\\this_directory_definitely_does_not_exist_xyz123".into(),
                api_key: "test".into(),
                ..Default::default()
            };

            let result = cli.run_batch(&opts);
            assert!(matches!(result, Err(SubSourceError::InvalidDirectory(_))));
        }
    }

    /// تست واقعی با sub-rs.exe (فقط با `cargo test -- --ignored`)
    #[test]
    #[ignore]
    fn live_subsource_help_succeeds() {
        // این تست فقط روی ماشینی که sub-rs نصب است اجرا می‌شود
        let exe = std::env::var("LOCALAPPDATA")
            .map(|la| format!("{la}\\Programs\\SubSource\\sub-rs.exe"))
            .expect("LOCALAPPDATA not set");

        if !Path::new(&exe).exists() {
            eprintln!("⚠️  sub-rs.exe not found, skipping live test");
            return;
        }

        // اجرای --help (بدون API key، dry run)
        let mut cmd = Command::new(&exe);
        cmd.arg("--help")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let output = cmd.output().expect("failed to run sub-rs --help");
        let stdout = String::from_utf8_lossy(&output.stdout);

        // خروجی help باید حاوی Usage باشد
        assert!(
            stdout.contains("Usage") || stdout.contains("Download subtitles"),
            "Unexpected --help output: {stdout}"
        );
    }
}
