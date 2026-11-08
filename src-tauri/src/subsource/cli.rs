//! SubSource CLI execution adapter — safe process management with testability.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::logging;
use crate::subsource::error::{SubSourceError, SubSourceResult};
use crate::subsource::types::{BatchOptions, BatchResult};

/// Token برای لغو عملیات در حال اجرا
#[derive(Clone)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// لغو عملیات
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// بررسی اینکه آیا لغو شده است
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

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

    /// اجرای batch download (بدون cancellation)
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
        let token = CancellationToken::new();
        self.run_batch_with_cancellation(options, &token)
    }

    /// اجرای batch download با قابلیت cancellation
    ///
    /// # Arguments
    /// * `options` - تنظیمات اجرا
    /// * `cancel_token` - token برای لغو عملیات
    ///
    /// # Returns
    /// نتیجه اجرا یا خطای `ProcessTerminated` در صورت لغو
    pub fn run_batch_with_cancellation(
        &self,
        options: &BatchOptions,
        cancel_token: &CancellationToken,
    ) -> SubSourceResult<BatchResult> {
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

        // بررسی cancellation قبل از شروع wait
        if cancel_token.is_cancelled() {
            Self::terminate_child(&mut child);
            return Err(SubSourceError::ProcessTerminated(
                "عملیات قبل از شروع لغو شد".into(),
            ));
        }

        // خواندن stdout و stderr در threadهای جدا
        let mut stdout_pipe = match child.stdout.take() {
            Some(pipe) => pipe,
            None => {
                Self::terminate_child(&mut child);
                return Err(SubSourceError::ProcessError(
                    "stdout pipe در دسترس نیست".into(),
                ));
            }
        };

        let mut stderr_pipe = match child.stderr.take() {
            Some(pipe) => pipe,
            None => {
                Self::terminate_child(&mut child);
                return Err(SubSourceError::ProcessError(
                    "stderr pipe در دسترس نیست".into(),
                ));
            }
        };

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

        // انتظار برای اتمام با پشتیبانی از cancellation.
        // در خطا، wait_with_cancellation خودش Child را kill/wait می‌کند؛
        // بنابراین reader threadها همیشه به EOF می‌رسند و join بدون deadlock انجام می‌شود.
        let status_result = Self::wait_with_cancellation(&mut child, cancel_token);
        let stdout_raw = stdout_thread.join().unwrap_or_default();
        let stderr_raw = stderr_thread.join().unwrap_or_default();
        let status = status_result?;
        let duration = start.elapsed().as_secs_f64();

        // Redact credentials از stdout/stderr
        let stdout = Self::redact_output(
            &stdout_raw,
            &options.api_key,
            &options.proxy,
            &options.worker,
        );
        let stderr = Self::redact_output(
            &stderr_raw,
            &options.api_key,
            &options.proxy,
            &options.worker,
        );

        // تحلیل نتیجه
        let exit_code = status.code().unwrap_or(-1);

        if status.success() {
            logging::info(
                "subsource",
                &format!("SubSource batch موفق ({:.1}s)", duration),
            );
            Ok(BatchResult::success(stdout, stderr, duration))
        } else {
            let result = BatchResult::failure(exit_code, stdout.clone(), stderr.clone(), duration);

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
                stderr,
            })
        }
    }

    /// انتظار برای اتمام process با پشتیبانی از cancellation
    fn wait_with_cancellation(
        child: &mut Child,
        cancel_token: &CancellationToken,
    ) -> SubSourceResult<std::process::ExitStatus> {
        use std::time::Duration;

        loop {
            // بررسی cancellation
            if cancel_token.is_cancelled() {
                Self::terminate_child(child);
                return Err(SubSourceError::ProcessTerminated("عملیات لغو شد".into()));
            }

            // تلاش برای بررسی وضعیت process (non-blocking)
            match child.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) => {
                    // هنوز در حال اجراست، کمی صبر کن
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    // حتی در خطای try_wait نباید Child رها شود.
                    Self::terminate_child(child);
                    return Err(SubSourceError::ProcessError(format!("wait error: {e}")));
                }
            }
        }
    }

    /// خاتمهٔ امن Child: ابتدا kill و سپس wait تا process و منابعش آزاد شوند.
    fn terminate_child(child: &mut Child) {
        match child.kill() {
            Ok(()) => {}
            // اگر process بین try_wait و kill خودش تمام شده باشد، wait زیر آن را reap می‌کند.
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {}
            Err(e) => logging::warn("subsource", &format!("خطا در kill process: {e}")),
        }

        if let Err(e) = child.wait() {
            logging::warn("subsource", &format!("خطا در wait پس از kill: {e}"));
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

    /// Redact کردن credentialها از متن خروجی (stdout/stderr)
    fn redact_output(
        text: &str,
        api_key: &str,
        proxy: &Option<String>,
        worker: &Option<String>,
    ) -> String {
        let mut result = text.to_string();

        // API key
        if !api_key.is_empty() {
            result = result.replace(api_key, "<REDACTED_API_KEY>");
        }

        // Proxy URL با username:password
        if let Some(proxy_url) = proxy {
            result = result.replace(proxy_url, "<REDACTED_PROXY>");
            // Redact username:password@ pattern در URLs
            if let Some(at_pos) = proxy_url.find('@') {
                if let Some(scheme_end) = proxy_url.find("://") {
                    let credentials = &proxy_url[scheme_end + 3..at_pos];
                    result = result.replace(credentials, "<REDACTED_CREDENTIALS>");
                }
            }
        }

        // Worker URL
        if let Some(worker_url) = worker {
            result = result.replace(worker_url, "<REDACTED_WORKER>");
        }

        result
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
