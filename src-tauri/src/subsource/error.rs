//! Error types for SubSource operations.

use std::fmt;

/// نتیجه عملیات SubSource
pub type SubSourceResult<T> = Result<T, SubSourceError>;

/// خطاهای SubSource — دسته‌بندی دقیق برای مدیریت و نمایش مناسب
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubSourceError {
    /// ابزار sub-rs.exe پیدا نشد یا قابل اجرا نیست
    ToolNotFound(String),

    /// API key تنظیم نشده یا نامعتبر
    AuthenticationRequired(String),

    /// مسیر directory نامعتبر یا غیرقابل دسترسی
    InvalidDirectory(String),

    /// خطای اجرای فرایند (spawn/wait)
    ProcessError(String),

    /// CLI با کد خروج غیر صفر خاتمه یافت
    CliExitCode { code: i32, stderr: String },

    /// CLI timeout یا کشته شد
    ProcessTerminated(String),

    /// خطای parse خروجی CLI
    OutputParseError(String),

    /// خطای عمومی
    Other(String),
}

impl fmt::Display for SubSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ToolNotFound(msg) => write!(f, "ابزار SubSource پیدا نشد: {msg}"),
            Self::AuthenticationRequired(msg) => write!(f, "احراز هویت لازم است: {msg}"),
            Self::InvalidDirectory(msg) => write!(f, "مسیر نامعتبر: {msg}"),
            Self::ProcessError(msg) => write!(f, "خطای اجرای فرایند: {msg}"),
            Self::CliExitCode { code, stderr } => {
                write!(f, "SubSource با کد خروج {code} خاتمه یافت: {stderr}")
            }
            Self::ProcessTerminated(msg) => write!(f, "فرایند متوقف شد: {msg}"),
            Self::OutputParseError(msg) => write!(f, "خطای تفسیر خروجی: {msg}"),
            Self::Other(msg) => write!(f, "خطا: {msg}"),
        }
    }
}

impl std::error::Error for SubSourceError {}

impl From<std::io::Error> for SubSourceError {
    fn from(err: std::io::Error) -> Self {
        Self::ProcessError(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_formats_correctly() {
        let err = SubSourceError::ToolNotFound("در PATH پیدا نشد".into());
        assert!(err.to_string().contains("پیدا نشد"));

        let err = SubSourceError::CliExitCode {
            code: 1,
            stderr: "network error".into(),
        };
        assert!(err.to_string().contains("کد خروج 1"));
        assert!(err.to_string().contains("network error"));
    }

    #[test]
    fn io_error_converts_to_process_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let sub_err: SubSourceError = io_err.into();
        matches!(sub_err, SubSourceError::ProcessError(_));
    }
}
