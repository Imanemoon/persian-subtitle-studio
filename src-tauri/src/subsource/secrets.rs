//! Secure storage for SubSource API key using OS credential manager.

use keyring::Entry;

const SERVICE_NAME: &str = "PersianSubtitleStudio";
const API_KEY_ACCOUNT: &str = "SubSourceApiKey";

/// ذخیره امن API key در Windows Credential Manager
///
/// # Arguments
/// * `api_key` - کلید API که باید ذخیره شود
///
/// # Returns
/// خطا اگر ذخیره‌سازی ناموفق باشد
pub fn store_api_key(api_key: &str) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, API_KEY_ACCOUNT)
        .map_err(|e| format!("ایجاد credential entry: {e}"))?;

    entry
        .set_password(api_key)
        .map_err(|e| format!("ذخیره API key: {e}"))
}

/// بازیابی API key از Windows Credential Manager
///
/// # Returns
/// API key اگر موجود باشد، None اگر تنظیم نشده باشد
pub fn retrieve_api_key() -> Result<Option<String>, String> {
    let entry = Entry::new(SERVICE_NAME, API_KEY_ACCOUNT)
        .map_err(|e| format!("ایجاد credential entry: {e}"))?;

    match entry.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("بازیابی API key: {e}")),
    }
}

/// حذف API key از Windows Credential Manager
pub fn delete_api_key() -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, API_KEY_ACCOUNT)
        .map_err(|e| format!("ایجاد credential entry: {e}"))?;

    match entry.delete_credential() {
        Ok(_) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()), // already deleted
        Err(e) => Err(format!("حذف API key: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use keyring::Entry;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// نام سرویس مختص تست — جدا از namespace تولیدی
    const TEST_SERVICE_NAME: &str = "PersianSubtitleStudio_Test";
    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    /// ساخت یک account name منحصربه‌فرد برای هر تست تا در اجرای موازی تداخل نداشته باشند.
    fn unique_test_account(test_name: &str) -> String {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        format!(
            "SubSourceApiKey_Test_{test_name}_{}_{}_{}",
            std::process::id(),
            nanos,
            id
        )
    }

    fn test_store(account: &str, key: &str) -> Result<(), String> {
        let entry = Entry::new(TEST_SERVICE_NAME, account)
            .map_err(|e| format!("ایجاد test credential entry: {e}"))?;
        entry
            .set_password(key)
            .map_err(|e| format!("ذخیره test API key: {e}"))
    }

    fn test_retrieve(account: &str) -> Result<Option<String>, String> {
        let entry = Entry::new(TEST_SERVICE_NAME, account)
            .map_err(|e| format!("ایجاد test credential entry: {e}"))?;
        match entry.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("بازیابی test API key: {e}")),
        }
    }

    fn test_delete(account: &str) -> Result<(), String> {
        let entry = Entry::new(TEST_SERVICE_NAME, account)
            .map_err(|e| format!("ایجاد test credential entry: {e}"))?;
        match entry.delete_credential() {
            Ok(_) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()), // already deleted
            Err(e) => Err(format!("حذف test API key: {e}")),
        }
    }

    #[test]
    fn retrieve_nonexistent_key_returns_none() {
        let account = unique_test_account("retrieve_nonexistent");
        // هیچ ورودی برای این account وجود ندارد — مستقیماً retrieve می‌کنیم
        let result = test_retrieve(&account).expect("retrieve should not error on missing key");
        assert_eq!(result, None);
        // پاک‌سازی (بی‌اثر اگر وجود نداشته باشد)
        let _ = test_delete(&account);
    }

    #[test]
    #[ignore] // نیاز به Windows Credential Manager واقعی دارد
    fn store_and_retrieve_roundtrip() {
        let account = unique_test_account("roundtrip");
        let test_key = "test-key-12345-abc";

        // ذخیره در test namespace با account منحصربه‌فرد
        test_store(&account, test_key).expect("store failed");

        // بازیابی
        let retrieved = test_retrieve(&account)
            .expect("retrieve failed")
            .expect("key should exist");

        assert_eq!(retrieved, test_key);

        // پاک‌سازی
        test_delete(&account).expect("delete failed");

        // تأیید حذف
        let after_delete = test_retrieve(&account).expect("retrieve after delete failed");
        assert_eq!(after_delete, None);
    }

    #[test]
    fn delete_nonexistent_key_succeeds() {
        let account = unique_test_account("delete_nonexistent");
        // account جدیدی که هرگز ذخیره نشده — حذف باید موفق باشد
        let result = test_delete(&account);
        assert!(result.is_ok());
    }
}
