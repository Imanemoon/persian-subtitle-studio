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
    use super::*;

    #[cfg(test)]
    const TEST_SERVICE_NAME: &str = "PersianSubtitleStudio_Test";
    #[cfg(test)]
    const TEST_API_KEY_ACCOUNT: &str = "SubSourceApiKey_Test";

    fn test_store(key: &str) -> Result<(), String> {
        let entry = Entry::new(TEST_SERVICE_NAME, TEST_API_KEY_ACCOUNT)
            .map_err(|e| format!("ایجاد test credential entry: {e}"))?;
        entry
            .set_password(key)
            .map_err(|e| format!("ذخیره test API key: {e}"))
    }

    fn test_retrieve() -> Result<Option<String>, String> {
        let entry = Entry::new(TEST_SERVICE_NAME, TEST_API_KEY_ACCOUNT)
            .map_err(|e| format!("ایجاد test credential entry: {e}"))?;
        match entry.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("بازیابی test API key: {e}")),
        }
    }

    fn test_delete() -> Result<(), String> {
        let entry = Entry::new(TEST_SERVICE_NAME, TEST_API_KEY_ACCOUNT)
            .map_err(|e| format!("ایجاد test credential entry: {e}"))?;
        match entry.delete_credential() {
            Ok(_) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()), // already deleted
            Err(e) => Err(format!("حذف test API key: {e}")),
        }
    }

    #[test]
    fn retrieve_nonexistent_key_returns_none() {
        // اطمینان از حذف قبل از تست (فقط در فضای test namespace)
        let _ = test_delete();

        let result = test_retrieve().expect("retrieve should not error on missing key");
        assert_eq!(result, None);
    }

    #[test]
    #[ignore] // نیاز به Windows Credential Manager واقعی دارد
    fn store_and_retrieve_roundtrip() {
        let test_key = "test-key-12345-abc";

        // ذخیره در test namespace
        test_store(test_key).expect("store failed");

        // بازیابی
        let retrieved = test_retrieve()
            .expect("retrieve failed")
            .expect("key should exist");

        assert_eq!(retrieved, test_key);

        // پاک‌سازی
        test_delete().expect("delete failed");

        // تأیید حذف
        let after_delete = test_retrieve().expect("retrieve after delete failed");
        assert_eq!(after_delete, None);
    }

    #[test]
    fn delete_nonexistent_key_succeeds() {
        let _ = test_delete();
        let result = test_delete();
        assert!(result.is_ok());
    }
}
