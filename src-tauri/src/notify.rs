//! اعلان‌های سیستمی پایان تسک‌های طولانی — pipeline، هاردساب، افزودن زیرنویس.
//!
//! هر اعلان قبل از ارسال با تنظیمات کاربر سنجیده می‌شود (کلید اصلی + کلید
//! همان نوع تسک)؛ در حالت خاموش، مسیر کاملاً بی‌صدا است و هیچ هزینه‌ای ندارد.
//! ارسال از لایهٔ command انجام می‌شود که AppHandle در دسترس دارد — حتی وقتی
//! پنجره مینیمایز است اعلان روی می‌آید و کاربر متوجه پایان می‌شود.

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::state::AppState;

pub const KIND_PIPELINE: &str = "pipeline";
pub const KIND_HARDSUB: &str = "hardsub";
pub const KIND_SOFTSUB: &str = "softsub";

/// اعلان پایان یک تسک — اگر کاربر آن نوع را خاموش کرده باشد سکوت می‌شود.
pub fn notify_task_done(app: &AppHandle, kind: &str, title: &str, body: &str) {
    let allowed = {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let settings = state
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let n = &settings.notifications;
        let per_task = match kind {
            KIND_PIPELINE => n.pipeline_done,
            KIND_HARDSUB => n.hardsub_done,
            KIND_SOFTSUB => n.softsub_done,
            _ => false,
        };
        n.enabled && per_task
    };
    if !allowed {
        return;
    }
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        crate::logging::warn("notify", &format!("اعلان سیستمی ارسال نشد: {error}"));
    }
}
