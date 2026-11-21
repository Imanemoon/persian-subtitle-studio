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

    /// ساخت token روی یک پرچم موجود — برای اتصال به همان `AtomicBool`
    /// که در `OpsRegistry` ثبت شده (بدون سیستم لغو دوم).
    pub fn from_flag(flag: Arc<AtomicBool>) -> Self {
        Self { cancelled: flag }
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
        self.run_batch_with_cancellation(options, &token, &mut |_| {})
    }

    /// اجرای batch download با قابلیت cancellation
    ///
    /// # Arguments
    /// * `options` - تنظیمات اجرا
    /// * `cancel_token` - token برای لغو عملیات
    /// * `on_stdout_line` - هر خط stdout لحظه‌ای اینجا می‌آید (پیشرفت زندهٔ
    ///   CLI بدون خروجی ساختاریافته دارد؛ فراخوانی روی thread صداکننده است)
    ///
    /// # Returns
    /// نتیجه اجرا یا خطای `ProcessTerminated` در صورت لغو
    pub fn run_batch_with_cancellation(
        &self,
        options: &BatchOptions,
        cancel_token: &CancellationToken,
        on_stdout_line: &mut dyn FnMut(&str),
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
        let stdout_pipe = match child.stdout.take() {
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

        // stdout خط‌به‌خط از thread خواننده به کانال می‌رود؛ حلقهٔ انتظارِ
        // لغو هر تیک خطوط رسیده را تخلیه و به callback می‌دهد — پیشرفت زندهٔ
        // CLI بدون خروجی ساختاریافته همین‌طور به UI می‌رسد.
        let (line_tx, line_rx) = std::sync::mpsc::channel::<String>();
        let stdout_thread = std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stdout_pipe);
            for line in reader.lines().map_while(Result::ok) {
                if line_tx.send(line).is_err() {
                    break; // گیرنده بسته شده (لغو/خطا) — خواندن بی‌معناست
                }
            }
        });

        let stderr_thread = std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            let _ = stderr_pipe.read_to_string(&mut s);
            s
        });

        // انتظار برای اتمام با پشتیبانی از cancellation + تخلیهٔ خطوط stdout.
        // در خطا، حلقهٔ زیر خودش Child را kill/wait می‌کند؛ بنابراین reader
        // threadها همیشه به EOF می‌رسند و join بدون deadlock انجام می‌شود.
        let mut stdout_raw = String::new();
        let status_result = loop {
            while let Ok(line) = line_rx.try_recv() {
                on_stdout_line(&line);
                stdout_raw.push_str(&line);
                stdout_raw.push('\n');
            }
            if cancel_token.is_cancelled() {
                Self::terminate_child(&mut child);
                break Err(SubSourceError::ProcessTerminated("عملیات لغو شد".into()));
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    // خطوط باقی‌ماندهٔ بافر تا EOF — بعد از مرگ فرایند لوله بسته می‌شود
                    while let Ok(line) = line_rx.try_recv() {
                        on_stdout_line(&line);
                        stdout_raw.push_str(&line);
                        stdout_raw.push('\n');
                    }
                    break Ok(status);
                }
                Ok(None) => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => {
                    Self::terminate_child(&mut child);
                    break Err(SubSourceError::ProcessError(format!("wait error: {e}")));
                }
            }
        };
        let _ = stdout_thread.join();
        while let Ok(line) = line_rx.try_recv() {
            on_stdout_line(&line);
            stdout_raw.push_str(&line);
            stdout_raw.push('\n');
        }
        let stderr_raw = stderr_thread.join().unwrap_or_default();
        let status = status_result?;
        let duration = start.elapsed().as_secs_f64();

        // Redact credentials از stdout/stderr
        let stdout = Self::redact_output(&stdout_raw, &options.api_key);
        let stderr = Self::redact_output(&stderr_raw, &options.api_key);

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

    /// Redact کردن API key از آرگومان‌ها
    fn redact_sensitive_args(args: &[String]) -> Vec<String> {
        let mut redacted = Vec::with_capacity(args.len());
        let mut redact_next = false;

        for arg in args {
            if redact_next {
                redacted.push("<REDACTED>".to_string());
                redact_next = false;
            } else if arg == "--api-key" {
                redacted.push(arg.clone());
                redact_next = true;
            } else {
                redacted.push(arg.clone());
            }
        }

        redacted
    }

    /// Redact کردن API key از متن خروجی (stdout/stderr)
    fn redact_output(text: &str, api_key: &str) -> String {
        let mut result = text.to_string();

        // API key
        if !api_key.is_empty() {
            result = result.replace(api_key, "<REDACTED_API_KEY>");
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
        ];

        let redacted = SubSourceCli::redact_sensitive_args(&args);
        assert_eq!(redacted[0], "--directory");
        assert_eq!(redacted[1], "/videos");
        assert_eq!(redacted[2], "--api-key");
        assert_eq!(redacted[3], "<REDACTED>");
        assert_eq!(redacted[4], "--top");
        assert_eq!(redacted[5], "5");

        // تأیید اینکه secret واقعی در خروجی نیست
        let combined = redacted.join(" ");
        assert!(!combined.contains("secret-key-12345"));
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
