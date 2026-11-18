//! دستورات Tauri برای مدیریت امن SubSource API key.
//!
//! کلید فقط در Windows Credential Manager ذخیره می‌شود (سرویس
//! «PersianSubtitleStudio» / حساب «SubSourceApiKey») و هرگز به settings.json،
//! لاگ‌ها، رویدادها یا خروجی هیچ commandی نمی‌رسد. frontend فقط وضعیت
//! وجود/نبودن کلید را به‌صورت boolean می‌بیند؛ هیچ commandی مقدار واقعی را
//! برنمی‌گرداند.
//!
//! هسته‌ها با تزریق عملیات credential نوشته شده‌اند تا تست‌ها هرگز
//! Credential Manager واقعی کاربر را ذخیره/حذف نکنند.

use crate::subsource::{delete_api_key, retrieve_api_key, store_api_key};

/// ذخیره یا به‌روزرسانی SubSource API key.
///
/// مقدار ابتدا trim می‌شود؛ مقدار خالی/فقط فاصله با خطای روشن رد می‌شود و
/// هرگز به لایهٔ ذخیره‌سازی نمی‌رسد. کلید لاگ، format یا برگردانده نمی‌شود.
#[tauri::command]
pub fn set_subsource_api_key(api_key: String) -> Result<(), String> {
    set_key_core(&api_key, store_api_key)
}

/// آیا یک کلید غیرخالی ذخیره شده است؟
///
/// فقط true/false برمی‌گرداند؛ مقدار بازیابی‌شده serialize یا لاگ نمی‌شود.
/// مقدار ذخیره‌شدهٔ خالی هم «ذخیره‌نشده» شمرده می‌شود.
#[tauri::command]
pub fn has_subsource_api_key() -> Result<bool, String> {
    has_key_core(retrieve_api_key)
}

/// حذف کلید ذخیره‌شده — حذف کلیدِ ناموجود هم موفقیت است (idempotent،
/// رفتار `delete_api_key` حفظ می‌شود). هیچ مقداری برنمی‌گرداند.
#[tauri::command]
pub fn delete_subsource_api_key() -> Result<(), String> {
    delete_key_core(delete_api_key)
}

/// هستهٔ ذخیره با تزریق تابع ذخیره‌سازی — برای تست بدون Credential Manager واقعی.
pub(crate) fn set_key_core<S>(api_key: &str, store: S) -> Result<(), String>
where
    S: FnOnce(&str) -> Result<(), String>,
{
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("مقدار API key خالی است — یک کلید معتبر وارد کنید.".into());
    }
    store(trimmed)
}

/// هستهٔ بررسی وجود کلید با تزریق تابع بازیابی — برای تست بدون Credential Manager واقعی.
pub(crate) fn has_key_core<R>(retrieve: R) -> Result<bool, String>
where
    R: FnOnce() -> Result<Option<String>, String>,
{
    let stored = retrieve()?;
    Ok(stored.map(|key| !key.trim().is_empty()).unwrap_or(false))
}

/// هستهٔ حذف کلید با تزریق تابع حذف — برای تست بدون Credential Manager واقعی.
pub(crate) fn delete_key_core<D>(delete: D) -> Result<(), String>
where
    D: FnOnce() -> Result<(), String>,
{
    delete()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::cell::RefCell;

    /// این ماژول هیچ‌وقت Credential Manager واقعی را لمس نمی‌کند:
    /// همهٔ تست‌ها با تزریق فیک اجرا می‌شوند و namespace تست موجود در
    /// `subsource::secrets` (PersianSubtitleStudio_Test) فقط برای همان
    /// تست‌های زندهٔ #[ignore] این ماژول است.
    #[test]
    fn empty_and_whitespace_only_keys_are_rejected_before_storage() {
        for bad in ["", "   ", "\t \n"] {
            let called = RefCell::new(false);
            let store = |_: &str| -> Result<(), String> {
                *called.borrow_mut() = true;
                Ok(())
            };
            let result = set_key_core(bad, store);
            assert!(result.is_err(), "مقدار {bad:?} باید رد شود");
            assert!(!called.into_inner(), "storage نباید فراخوانی شده باشد");
        }
    }

    #[test]
    fn valid_key_is_trimmed_and_passed_to_storage() {
        let captured = RefCell::new(None::<String>);
        let store = |key: &str| -> Result<(), String> {
            *captured.borrow_mut() = Some(key.to_string());
            Ok(())
        };
        set_key_core("  sk-test-123  ", store).expect("ذخیره باید موفق باشد");
        assert_eq!(captured.into_inner().as_deref(), Some("sk-test-123"));
    }

    #[test]
    fn storage_failure_is_propagated() {
        let store = |_: &str| -> Result<(), String> { Err("storage boom".into()) };
        let result = set_key_core("valid-key", store);
        assert_eq!(result.unwrap_err(), "storage boom");
    }

    #[test]
    fn has_key_returns_only_boolean_for_missing_empty_and_present() {
        assert!(!has_key_core(|| Ok(None)).expect("موجود نبودن باید false بدهد"));
        assert!(!has_key_core(|| Ok(Some(String::new()))).expect("خالی باید false بدهد"));
        assert!(!has_key_core(|| Ok(Some("   ".to_string()))).expect("فقط فاصله باید false بدهد"),);
        assert!(
            has_key_core(|| Ok(Some("stored-value".to_string()))).expect("موجود باید true بدهد"),
        );
        // خطای زیرساخت منتقل می‌شود — false ساختگی برنمی‌گردد
        assert!(has_key_core(|| Err("keyring boom".into())).is_err());
    }

    #[test]
    fn deleting_missing_key_succeeds_and_errors_propagate() {
        delete_key_core(|| Ok(())).expect("حذف کلید ناموجود باید موفق باشد (idempotent)");
        assert!(delete_key_core(|| Err("keyring boom".into())).is_err());
    }

    #[test]
    fn settings_serialization_never_contains_api_key() {
        // کلید باید فقط در Credential Manager باشد؛ هیچ فیلدی در Settings نیست
        let json = serde_json::to_string(&Settings::default()).expect("serialize settings");
        let lowered = json.to_lowercase();
        assert!(!lowered.contains("apikey"), "{json}");
        assert!(!lowered.contains("api_key"), "{json}");
    }
}
