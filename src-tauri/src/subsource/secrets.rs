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

    #[test]
    fn retrieve_nonexistent_key_returns_none() {
        // اطمینان از حذف قبل از تست
        let _ = delete_api_key();

        let result = retrieve_api_key().expect("retrieve should not error on missing key");
        assert_eq!(result, None);
    }

    #[test]
    #[ignore] // نیاز به Windows Credential Manager واقعی دارد
    fn store_and_retrieve_roundtrip() {
        let test_key = "test-key-12345-abc";

        // ذخیره
        store_api_key(test_key).expect("store failed");

        // بازیابی
        let retrieved = retrieve_api_key()
            .expect("retrieve failed")
            .expect("key should exist");

        assert_eq!(retrieved, test_key);

        // پاک‌سازی
        delete_api_key().expect("delete failed");

        // تأیید حذف
        let after_delete = retrieve_api_key().expect("retrieve after delete failed");
        assert_eq!(after_delete, None);
    }

    #[test]
    fn delete_nonexistent_key_succeeds() {
        let _ = delete_api_key();
        let result = delete_api_key();
        assert!(result.is_ok());
    }
}
