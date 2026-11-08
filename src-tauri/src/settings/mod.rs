//! تنظیمات برنامه — ذخیره‌سازی JSON در پوشه config دائمی.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolPathOverrides {
    pub ffmpeg: Option<String>,
    pub ffprobe: Option<String>,
    pub subsource: Option<String>,
    pub seven_zip: Option<String>,
    pub ffsubsync: Option<String>,
}

/// مقادیر پیش‌فرض پردازش مطابق تجربه اسکریپت PowerShell قبلی.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProcessingSettings {
    pub sync_workers: u32,
    pub multi_segment: bool,
    pub segment_count: u32,
    pub skip_intro_outro: bool,
    pub skip_sync_on_low_quality: bool,
}

/// تنظیمات SubSource API و دانلود
///
/// API key عمداً در این ساختار قرار ندارد؛ فقط در Credential Manager سیستم
/// ذخیره می‌شود تا هرگز داخل settings.json نوشته نشود.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct SubSourceSettings {
    /// حالت اتصال: direct, system, manual, cloudflare
    pub connection_mode: String,

    /// آدرس پروکسی (فقط با manual mode)
    pub proxy: Option<String>,

    /// آدرس Cloudflare Worker (فقط با cloudflare mode)
    pub worker: Option<String>,

    /// حذف تبلیغات از زیرنویس‌های فارسی
    pub clean_ads: bool,

    /// رد کردن فایل‌هایی که زیرنویس دارند
    pub skip_existing: bool,
}

impl Default for SubSourceSettings {
    fn default() -> Self {
        Self {
            connection_mode: "direct".to_string(),
            proxy: None,
            worker: None,
            clean_ads: true,
            skip_existing: true,
        }
    }
}

impl Default for ProcessingSettings {
    fn default() -> Self {
        Self {
            sync_workers: 2,
            multi_segment: true,
            segment_count: 6,
            skip_intro_outro: true,
            skip_sync_on_low_quality: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum ExistingFileBehavior {
    /// فایل‌های موجود هرگز بدون اجازه کاربر بازنویسی نمی‌شوند.
    #[default]
    NeverOverwrite,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub root_folder: Option<String>,
    pub excluded_folders: Vec<String>,
    pub tool_paths: ToolPathOverrides,
    pub processing: ProcessingSettings,
    pub subsource: SubSourceSettings,
    pub existing_file_behavior: ExistingFileBehavior,
    /// اگر خالی باشد از پوشه لاگ پیش‌فرض سیستم استفاده می‌شود.
    pub logs_dir: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            root_folder: None,
            excluded_folders: vec!["Ted Lasso".to_string(), "Better Call Saul".to_string()],
            tool_paths: ToolPathOverrides::default(),
            processing: ProcessingSettings::default(),
            subsource: SubSourceSettings::default(),
            existing_file_behavior: ExistingFileBehavior::default(),
            logs_dir: None,
        }
    }
}

impl Settings {
    /// اعتبارسنجی و پاک‌سازی مقادیر ورودی کاربر.
    pub fn sanitized(mut self) -> Self {
        self.processing.sync_workers = self.processing.sync_workers.clamp(1, 16);
        self.processing.segment_count = self.processing.segment_count.clamp(1, 12);

        let mut seen = std::collections::HashSet::new();
        self.excluded_folders = self
            .excluded_folders
            .iter()
            .map(|f| f.trim().to_string())
            .filter(|t| !t.is_empty() && seen.insert(t.to_lowercase()))
            .collect();

        let clean = |p: &Option<String>| -> Option<String> {
            p.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        self.root_folder = clean(&self.root_folder);
        self.logs_dir = clean(&self.logs_dir);
        self.tool_paths.ffmpeg = clean(&self.tool_paths.ffmpeg);
        self.tool_paths.ffprobe = clean(&self.tool_paths.ffprobe);
        self.tool_paths.subsource = clean(&self.tool_paths.subsource);
        self.tool_paths.seven_zip = clean(&self.tool_paths.seven_zip);
        self.tool_paths.ffsubsync = clean(&self.tool_paths.ffsubsync);
        self
    }
}

pub fn config_file_path(config_dir: &Path) -> PathBuf {
    config_dir.join("settings.json")
}

/// بارگذاری تنظیمات؛ در صورت خرابی فایل، از آن پشتیبان گرفته و پیش‌فرض برمی‌گردد.
///
/// اگر فایل قدیمی حاوی `subsource.apiKey` باشد، ابتدا کلید به Credential Manager
/// منتقل می‌شود و تنها پس از ذخیرهٔ موفق، فیلد plaintext از JSON حذف می‌گردد.
/// در صورت شکست مهاجرت، فایل تنظیمات دست‌نخورده می‌ماند تا داده‌ای از دست نرود.
pub fn load(config_dir: &Path) -> Settings {
    load_with_store(config_dir, crate::subsource::store_api_key)
}

fn load_with_store(
    config_dir: &Path,
    store_api_key: impl FnOnce(&str) -> Result<(), String>,
) -> Settings {
    let path = config_file_path(config_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => return Settings::default(),
    };

    let mut raw: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(err) => {
            backup_corrupt(config_dir, &path, &err.to_string());
            return Settings::default();
        }
    };

    let settings = match serde_json::from_value::<Settings>(raw.clone()) {
        Ok(s) => s.sanitized(),
        Err(err) => {
            backup_corrupt(config_dir, &path, &err.to_string());
            return Settings::default();
        }
    };

    if let Some(legacy_value) = take_legacy_api_key(&mut raw) {
        let legacy_key = legacy_value
            .as_str()
            .map(str::trim)
            .filter(|key| !key.is_empty());

        let migration = match legacy_key {
            Some(key) => store_api_key(key),
            None => Ok(()),
        };

        match migration {
            Ok(()) => {
                if let Err(err) = save_raw(config_dir, &raw) {
                    crate::logging::error(
                        "settings",
                        &format!(
                            "حذف apiKey قدیمی از settings.json ناموفق بود؛ مقدار plaintext حفظ شد: {err}"
                        ),
                    );
                } else {
                    crate::logging::info(
                        "settings",
                        "API key قدیمی به Credential Manager منتقل و از settings.json حذف شد",
                    );
                }
            }
            Err(err) => {
                crate::logging::error(
                    "settings",
                    &format!(
                        "مهاجرت API key به Credential Manager ناموفق بود؛ تنظیمات موجود حفظ شد: {err}"
                    ),
                );
            }
        }
    }

    settings
}

fn backup_corrupt(config_dir: &Path, path: &Path, err: &str) {
    let backup = config_dir.join(format!(
        "settings.corrupt-{}.json",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    let _ = std::fs::copy(path, &backup);
    crate::logging::warn(
        "settings",
        &format!(
            "فایل تنظیمات قابل خواندن نبود ({err})؛ پشتیبان: {} — تنظیمات پیش‌فرض بارگذاری شد",
            backup.display()
        ),
    );
}

/// حذف فیلد قدیمی API key از JSON (هم camelCase و هم snake_case).
fn take_legacy_api_key(value: &mut serde_json::Value) -> Option<serde_json::Value> {
    let subsource = value.get_mut("subsource")?.as_object_mut()?;
    let camel_case = subsource.remove("apiKey");
    let snake_case = subsource.remove("api_key");
    camel_case.or(snake_case)
}

/// ذخیره امن تنظیمات: اول در فایل موقت، سپس جایگزینی.
pub fn save(config_dir: &Path, settings: &Settings) -> Result<(), String> {
    let clean = settings.clone().sanitized();
    let data = serde_json::to_vec_pretty(&clean).map_err(|e| format!("سریال‌سازی تنظیمات: {e}"))?;
    write_atomic(config_dir, &data)
}

fn save_raw(config_dir: &Path, value: &serde_json::Value) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(value)
        .map_err(|e| format!("سریال‌سازی تنظیمات برای مهاجرت: {e}"))?;
    write_atomic(config_dir, &data)
}

fn write_atomic(config_dir: &Path, data: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(config_dir).map_err(|e| format!("عدم امکان ساخت پوشه تنظیمات: {e}"))?;
    let tmp = config_dir.join("settings.json.tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("نوشتن تنظیمات ناموفق بود: {e}"))?;
    if let Err(err) = std::fs::rename(&tmp, config_file_path(config_dir)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("جایگزینی فایل تنظیمات ناموفق بود: {err}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "subsync-settings-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn roundtrip_save_and_load() {
        let dir = unique_temp_dir("rt");
        let s = Settings {
            root_folder: Some("D:\\Series".to_string()),
            excluded_folders: vec!["Ted Lasso".into(), "  X  ".into(), "ted lasso".into()],
            processing: ProcessingSettings {
                sync_workers: 4,
                ..Default::default()
            },
            ..Default::default()
        };

        save(&dir, &s).expect("save");
        let loaded = load(&dir);
        assert_eq!(loaded.root_folder.as_deref(), Some("D:\\Series"));
        // موارد تکراری و خالی حذف شده‌اند
        assert_eq!(
            loaded.excluded_folders,
            vec!["Ted Lasso".to_string(), "X".to_string()]
        );
        assert_eq!(loaded.processing.sync_workers, 4);
        // فایل موقت باقی نمانده باشد
        assert!(!dir.join("settings.json.tmp").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_returns_default() {
        let dir = unique_temp_dir("missing");
        let s = load(&dir);
        assert_eq!(s, Settings::default());
        assert_eq!(
            s.excluded_folders,
            vec!["Ted Lasso".to_string(), "Better Call Saul".to_string()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_corrupt_backs_up_and_defaults() {
        let dir = unique_temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(config_file_path(&dir), "{not valid json").unwrap();
        let s = load(&dir);
        assert_eq!(s, Settings::default());
        // پشتیبان خرابی ساخته شده است
        let has_backup = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("settings.corrupt-")
            });
        assert!(has_backup);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitized_clamps_ranges() {
        let s = Settings {
            processing: ProcessingSettings {
                sync_workers: 99,
                segment_count: 0,
                ..Default::default()
            },
            excluded_folders: vec!["  ".into()],
            ..Default::default()
        };
        let s = s.sanitized();
        assert_eq!(s.processing.sync_workers, 16);
        assert_eq!(s.processing.segment_count, 1);
        assert!(s.excluded_folders.is_empty());
    }

    #[test]
    fn legacy_api_key_migrates_and_is_removed_from_json() {
        let dir = unique_temp_dir("migrate-ok");
        std::fs::create_dir_all(&dir).unwrap();
        let legacy = serde_json::json!({
            "rootFolder": "D:\\Series",
            "subsource": {
                "apiKey": "legacy-secret-key",
                "connectionMode": "manual",
                "proxy": "http://user:pass@proxy.example.com:8080",
                "cleanAds": false,
                "skipExisting": false
            },
            "futureField": { "keep": true }
        });
        std::fs::write(
            config_file_path(&dir),
            serde_json::to_vec_pretty(&legacy).unwrap(),
        )
        .unwrap();

        let stored = std::sync::Arc::new(std::sync::Mutex::new(None));
        let captured = stored.clone();
        let loaded = load_with_store(&dir, move |key| {
            *captured.lock().unwrap() = Some(key.to_string());
            Ok(())
        });

        assert_eq!(
            *stored.lock().unwrap(),
            Some("legacy-secret-key".to_string())
        );
        assert_eq!(loaded.root_folder.as_deref(), Some("D:\\Series"));
        assert_eq!(loaded.subsource.connection_mode, "manual");
        assert_eq!(
            loaded.subsource.proxy.as_deref(),
            Some("http://user:pass@proxy.example.com:8080")
        );
        assert!(!loaded.subsource.clean_ads);

        let text = std::fs::read_to_string(config_file_path(&dir)).unwrap();
        assert!(!text.contains("legacy-secret-key"));
        assert!(!text.contains("apiKey"));
        assert!(!text.contains("api_key"));

        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["rootFolder"].as_str(), Some("D:\\Series"));
        assert_eq!(json["futureField"]["keep"].as_bool(), Some(true));
        assert!(!dir.join("settings.json.tmp").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_legacy_api_key_migration_keeps_settings_file_intact() {
        let dir = unique_temp_dir("migrate-fail");
        std::fs::create_dir_all(&dir).unwrap();
        let original = serde_json::json!({
            "rootFolder": "D:\\Keep",
            "excludedFolders": ["Keep Me"],
            "subsource": {
                "apiKey": "still-plaintext",
                "connectionMode": "direct"
            }
        });
        let original_text = serde_json::to_string_pretty(&original).unwrap();
        std::fs::write(config_file_path(&dir), &original_text).unwrap();

        let loaded = load_with_store(&dir, |_| Err("credential manager unavailable".into()));

        assert_eq!(loaded.root_folder.as_deref(), Some("D:\\Keep"));
        assert_eq!(loaded.excluded_folders, vec!["Keep Me".to_string()]);

        let after = std::fs::read_to_string(config_file_path(&dir)).unwrap();
        assert_eq!(after, original_text);
        assert!(after.contains("still-plaintext"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_null_api_key_is_removed_without_credential_write() {
        let dir = unique_temp_dir("migrate-null");
        std::fs::create_dir_all(&dir).unwrap();
        let legacy = serde_json::json!({
            "subsource": { "apiKey": null, "connectionMode": "direct" }
        });
        std::fs::write(
            config_file_path(&dir),
            serde_json::to_vec_pretty(&legacy).unwrap(),
        )
        .unwrap();

        let loaded = load_with_store(&dir, |_| panic!("نباید برای apiKey خالی فراخوانی شود"));

        assert_eq!(loaded.subsource.connection_mode, "direct");
        let text = std::fs::read_to_string(config_file_path(&dir)).unwrap();
        assert!(!text.contains("apiKey"));
        assert!(!text.contains("api_key"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saved_settings_never_contain_api_key() {
        let dir = unique_temp_dir("no-api-key");
        let s = Settings {
            root_folder: Some("D:\\Series".to_string()),
            ..Default::default()
        };

        save(&dir, &s).expect("save");
        let text = std::fs::read_to_string(config_file_path(&dir)).unwrap();
        assert!(!text.contains("apiKey"));
        assert!(!text.contains("api_key"));
        assert!(!text.contains("api-key"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
