//! SubSource CLI Adapter — Phase 3 PR 1
//!
//! این ماژول لایه ارتباطی قابل‌اعتماد با sub-rs.exe (SubSource CLI) را فراهم می‌کند.
//!
//! ## معماری
//!
//! - **CLI Adapter**: اجرای امن batch command با `std::process::Command`
//! - **Types**: ساختارهای type-safe برای ورودی/خروجی
//! - **Error Handling**: مدیریت دقیق خطاهای CLI و عملیاتی
//! - **Testability**: طراحی قابل mock برای تست‌های بدون وابستگی به ابزار واقعی
//!
//! ## محدودیت‌های CLI واقعی (از کشف)
//!
//! - فقط batch mode: دستورات search/download مستقل ندارد
//! - آرشیوها در `<video_dir>/sub/` دانلود می‌شوند
//! - نیاز به API key برای احراز هویت
//! - خروجی متنی (JSON mode وجود ندارد)
//!
//! ## PR Scope
//!
//! این PR فقط CLI adapter را پیاده‌سازی می‌کند؛ بدون:
//! - Staging directory یا hard links
//! - Archive extraction (7-Zip)
//! - Episode matching
//! - UI integration
//!
//! PRهای بعدی این قابلیت‌ها را اضافه می‌کنند.

pub mod cli;
pub mod error;
pub mod secrets;
pub mod types;

pub use cli::{CancellationToken, SubSourceCli};
pub use error::{SubSourceError, SubSourceResult};
pub use secrets::{delete_api_key, retrieve_api_key, store_api_key};
pub use types::{BatchOptions, BatchResult, ConnectionMode};
