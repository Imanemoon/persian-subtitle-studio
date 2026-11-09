//! Types for SubSource operations — inputs, outputs, and configuration.

/// تنظیمات اجرای batch download
///
/// SubSource از تنظیمات شبکه‌ای سیستم‌عامل استفاده می‌کند.
/// کاربران می‌توانند از VPN خارجی برای تغییر مسیر شبکه استفاده کنند.
#[derive(Debug, Clone)]
pub struct BatchOptions {
    /// مسیر directory حاوی فایل‌های ویدیویی
    pub directory: String,

    /// API key برای SubSource
    pub api_key: String,

    /// حداکثر تعداد نتایج برای هر ویدیو [1..10]
    pub top: u8,

    /// رد کردن فایل‌هایی که زیرنویس فارسی دارند
    pub skip_existing: bool,

    /// عدم جستجو در زیرپوشه‌ها
    pub no_recursive: bool,

    /// شبیه‌سازی (بدون دانلود واقعی)
    pub dry_run: bool,

    /// حذف تبلیغات از زیرنویس‌های فارسی
    pub clean_ads: bool,
}

impl Default for BatchOptions {
    fn default() -> Self {
        Self {
            directory: String::new(),
            api_key: String::new(),
            top: 1,
            skip_existing: true,
            no_recursive: false,
            dry_run: false,
            clean_ads: true,
        }
    }
}

impl BatchOptions {
    /// اعتبارسنجی تنظیمات
    pub fn validate(&self) -> Result<(), String> {
        if self.directory.trim().is_empty() {
            return Err("directory خالی است".into());
        }

        if self.api_key.trim().is_empty() {
            return Err("API key تنظیم نشده است".into());
        }

        if self.top == 0 || self.top > 10 {
            return Err("top باید بین 1 تا 10 باشد".into());
        }

        Ok(())
    }

    /// تبدیل به آرگومان‌های CLI
    pub fn to_cli_args(&self) -> Vec<String> {
        let mut args = vec![
            "--directory".to_string(),
            self.directory.clone(),
            "--api-key".to_string(),
            self.api_key.clone(),
            "--top".to_string(),
            self.top.to_string(),
            "--lang".to_string(),
            "fa".to_string(),
        ];

        if self.skip_existing {
            args.push("--skip-existing".to_string());
        }

        if self.no_recursive {
            args.push("--no-recursive".to_string());
        }

        if self.dry_run {
            args.push("--dry-run".to_string());
        }

        if self.clean_ads {
            args.push("--clean-ads".to_string());
        }

        args
    }
}

/// نتیجه اجرای batch operation
#[derive(Debug, Clone)]
pub struct BatchResult {
    /// کد خروج CLI (0 = موفقیت)
    pub exit_code: i32,

    /// خروجی stdout
    pub stdout: String,

    /// خروجی stderr
    pub stderr: String,

    /// آیا عملیات موفق بود؟
    pub success: bool,

    /// زمان اجرا (ثانیه)
    pub duration_secs: f64,
}

impl BatchResult {
    /// ایجاد نتیجه موفق
    pub fn success(stdout: String, stderr: String, duration_secs: f64) -> Self {
        Self {
            exit_code: 0,
            stdout,
            stderr,
            success: true,
            duration_secs,
        }
    }

    /// ایجاد نتیجه ناموفق
    pub fn failure(exit_code: i32, stdout: String, stderr: String, duration_secs: f64) -> Self {
        Self {
            exit_code,
            stdout,
            stderr,
            success: false,
            duration_secs,
        }
    }

    /// آیا خطایی رخ داد که نشان‌دهنده مشکل احراز هویت است؟
    pub fn is_auth_error(&self) -> bool {
        let combined = format!("{} {}", self.stdout, self.stderr).to_lowercase();
        combined.contains("api key")
            || combined.contains("unauthorized")
            || combined.contains("authentication")
            || combined.contains("401")
    }

    /// آیا خطایی رخ داد که نشان‌دهنده مشکل شبکه است؟
    pub fn is_network_error(&self) -> bool {
        let combined = format!("{} {}", self.stdout, self.stderr).to_lowercase();
        combined.contains("network")
            || combined.contains("connection")
            || combined.contains("timeout")
            || combined.contains("dns")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_options_validation() {
        let mut opts = BatchOptions::default();
        assert!(opts.validate().is_err()); // directory خالی

        opts.directory = "/tmp/test".into();
        assert!(opts.validate().is_err()); // api_key خالی

        opts.api_key = "test-key".into();
        assert!(opts.validate().is_ok());

        opts.top = 0;
        assert!(opts.validate().is_err()); // top نامعتبر

        opts.top = 11;
        assert!(opts.validate().is_err());

        opts.top = 5;
        assert!(opts.validate().is_ok());
    }

    #[test]
    fn batch_options_to_cli_args() {
        let opts = BatchOptions {
            directory: "/videos".into(),
            api_key: "secret".into(),
            top: 3,
            skip_existing: true,
            no_recursive: true,
            dry_run: true,
            clean_ads: false,
        };

        let args = opts.to_cli_args();
        assert!(args.contains(&"--directory".to_string()));
        assert!(args.contains(&"/videos".to_string()));
        assert!(args.contains(&"--api-key".to_string()));
        assert!(args.contains(&"secret".to_string()));
        assert!(args.contains(&"--top".to_string()));
        assert!(args.contains(&"3".to_string()));
        assert!(args.contains(&"--skip-existing".to_string()));
        assert!(args.contains(&"--no-recursive".to_string()));
        assert!(args.contains(&"--dry-run".to_string()));
        assert!(!args.contains(&"--clean-ads".to_string()));
        // بدون --connect، --proxy، --worker
        assert!(!args.contains(&"--connect".to_string()));
        assert!(!args.contains(&"--proxy".to_string()));
        assert!(!args.contains(&"--worker".to_string()));
    }

    #[test]
    fn batch_result_error_detection() {
        let auth_err = BatchResult::failure(1, "".into(), "Error: Invalid API key".into(), 1.0);
        assert!(auth_err.is_auth_error());
        assert!(!auth_err.is_network_error());

        let net_err = BatchResult::failure(1, "".into(), "Error: Connection timeout".into(), 5.0);
        assert!(!net_err.is_auth_error());
        assert!(net_err.is_network_error());

        let success = BatchResult::success("Downloaded 5 files".into(), "".into(), 10.0);
        assert!(!success.is_auth_error());
        assert!(!success.is_network_error());
    }

    #[test]
    fn api_key_included_in_args_but_not_logged() {
        let opts = BatchOptions {
            directory: "/videos".into(),
            api_key: "secret-key-12345".into(),
            top: 1,
            ..Default::default()
        };

        let args = opts.to_cli_args();
        // API key باید در args باشد (برای CLI)
        assert!(args.contains(&"secret-key-12345".to_string()));

        // اما در لاگ‌ها باید redacted شود (این مسئولیت CLI adapter است)
        // این تست فقط تأیید می‌کند که args صحیح تولید می‌شود
    }
}
