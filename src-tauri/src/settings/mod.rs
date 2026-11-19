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

/// مقصد نهایی زیرنویس سینک‌شده.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum SubtitleOutputMode {
    /// زیرنویس درون فایل ویدیو remux می‌شود و به‌صورت SoftSub در دسترس است.
    #[default]
    SoftSub,
    /// رفتار قدیمی: خروجی زیرنویس کنار فایل ویدیو ذخیره می‌شود.
    Sidecar,
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
    /// محل خروجی زیرنویس؛ SoftSub حالت پیش‌فرض نسخه‌های جدید است.
    pub subtitle_output_mode: SubtitleOutputMode,
}

/// تنظیمات SubSource API و دانلود
///
/// API key عمداً در این ساختار قرار ندارد؛ فقط در Credential Manager سیستم
/// ذخیره می‌شود تا هرگز داخل settings.json نوشته نشود.
///
/// تنظیمات پروکسی برداشته شده‌اند؛ SubSource از اتصال شبکه‌ای سیستم‌عامل
/// استفاده می‌کند. کاربران می‌توانند از VPN خارجی برای تغییر مسیر شبکه استفاده کنند.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct SubSourceSettings {
    /// حذف تبلیغات از زیرنویس‌های فارسی
    pub clean_ads: bool,

    /// رد کردن فایل‌هایی که زیرنویس دارند
    pub skip_existing: bool,
}

impl Default for SubSourceSettings {
    fn default() -> Self {
        Self {
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
            subtitle_output_mode: SubtitleOutputMode::SoftSub,
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
    /// معاونت راه‌اندازی (نصب خودکار پیش‌نیازها) یک‌بار دیده شده/پشت زده شده است —
    /// پس از true شدن، صفحهٔ setup دوباره اجباری نمی‌شود.
    #[serde(default)]
    pub setup_completed: bool,
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
            setup_completed: false,
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

    match take_legacy_api_key(&mut raw) {
        Ok(LegacyKeyAction::Migrate(key)) => match store_api_key(&key) {
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
        },
        Ok(LegacyKeyAction::RemoveOnly) => {
            if let Err(err) = save_raw(config_dir, &raw) {
                crate::logging::warn(
                    "settings",
                    &format!("حذف فیلد خالی apiKey قدیمی ناموفق بود: {err}"),
                );
            }
        }
        Ok(LegacyKeyAction::None) => {}
        Err(conflict_err) => {
            crate::logging::error(
                "settings",
                &format!(
                    "مهاجرت API key به دلیل تضاد در مقادیر متوقف شد؛ فایل اصلی دست‌نخورده باقی ماند: {conflict_err}"
                ),
            );
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

/// نتیجه استخراج و اعتبارسنجی کلید API قدیمی از JSON
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LegacyKeyAction {
    /// هیچ کلید قدیمی وجود ندارد
    None,
    /// کلید معتبر یافت شد که باید در Credential Manager ذخیره و از JSON حذف شود
    Migrate(String),
    /// فیلد(های) قدیمی وجود داشت اما خالی یا null بود؛ فقط از JSON حذف می‌شود
    RemoveOnly,
}

fn extract_clean_key(val: Option<&serde_json::Value>) -> Option<String> {
    val.and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// استخراج امن API key قدیمی از JSON (پشتیبانی از هر دو حالت camelCase و snake_case).
///
/// رفتار:
/// - اگر فیلدی وجود نداشت: `Ok(LegacyKeyAction::None)`
/// - اگر فقط یکی وجود داشت و معتبر بود: `Ok(LegacyKeyAction::Migrate(key))` و حذف از JSON
/// - اگر هر دو وجود داشتند و مقدار یکسان داشتند: `Ok(LegacyKeyAction::Migrate(key))` و حذف هر دو
/// - اگر یکی خالی/null و دیگری معتبر بود: کلید معتبر حفظ و هر دو از JSON حذف می‌شوند
/// - اگر هر دو خالی/null بودند: `Ok(LegacyKeyAction::RemoveOnly)` و هر دو از JSON حذف می‌شوند
/// - اگر هر دو دارای مقادیر معتبر ولی متفاوت بودند: خطای تضاد بازگردانده شده و JSON کاملاً دست‌نخورده می‌ماند
pub(crate) fn take_legacy_api_key(
    value: &mut serde_json::Value,
) -> Result<LegacyKeyAction, String> {
    let Some(subsource) = value.get_mut("subsource").and_then(|v| v.as_object_mut()) else {
        return Ok(LegacyKeyAction::None);
    };

    let has_camel = subsource.contains_key("apiKey");
    let has_snake = subsource.contains_key("api_key");

    if !has_camel && !has_snake {
        return Ok(LegacyKeyAction::None);
    }

    let camel_clean = extract_clean_key(subsource.get("apiKey"));
    let snake_clean = extract_clean_key(subsource.get("api_key"));

    match (camel_clean, snake_clean) {
        (Some(c), Some(s)) => {
            if c == s {
                subsource.remove("apiKey");
                subsource.remove("api_key");
                Ok(LegacyKeyAction::Migrate(c))
            } else {
                Err(
                    "تضاد در کلیدهای API قدیمی: دو مقدار apiKey و api_key با یکدیگر متفاوت هستند"
                        .to_string(),
                )
            }
        }
        (Some(c), None) => {
            subsource.remove("apiKey");
            subsource.remove("api_key");
            Ok(LegacyKeyAction::Migrate(c))
        }
        (None, Some(s)) => {
            subsource.remove("apiKey");
            subsource.remove("api_key");
            Ok(LegacyKeyAction::Migrate(s))
        }
        (None, None) => {
            subsource.remove("apiKey");
            subsource.remove("api_key");
            Ok(LegacyKeyAction::RemoveOnly)
        }
    }
}

/// ذخیره امن تنظیمات با بررسی و مهاجرت API key قدیمی.
///
/// اگر فایل فعلی settings.json حاوی `subsource.apiKey` یا `subsource.api_key` باشد،
/// ابتدا کلید به Credential Manager منتقل می‌شود. اگر مهاجرت ناموفق باشد، ذخیره‌سازی
/// لغو می‌شود تا داده‌ای از دست نرود.
pub fn save(config_dir: &Path, settings: &Settings) -> Result<(), String> {
    save_with_store(config_dir, settings, crate::subsource::store_api_key)
}

fn save_with_store(
    config_dir: &Path,
    settings: &Settings,
    store_api_key: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    let path = config_file_path(config_dir);

    // بررسی وجود API key قدیمی در فایل موجود
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(mut raw) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            match take_legacy_api_key(&mut raw) {
                Ok(LegacyKeyAction::Migrate(key)) => {
                    store_api_key(&key).map_err(|e| {
                        format!(
                            "مهاجرت API key به Credential Manager ناموفق بود؛ ذخیره لغو شد: {e}"
                        )
                    })?;
                }
                Ok(LegacyKeyAction::RemoveOnly) | Ok(LegacyKeyAction::None) => {
                    // بدون نیاز به ذخیره در Credential Manager
                }
                Err(conflict_err) => {
                    return Err(format!(
                        "تضاد در کلیدهای API قدیمی؛ ذخیره لغو شد تا فایل دست‌نخورده بماند: {conflict_err}"
                    ));
                }
            }
        }
    }

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

    /// تنظیمات قدیمی (بدون subtitleOutputMode) — فیلد جدید با پیش‌فرض SoftSub
    /// مهاجرت می‌شود و انتخاب صریح sidecar دست‌نخورده می‌ماند.
    #[test]
    fn legacy_settings_without_output_mode_default_to_softsub_and_keep_explicit_sidecar() {
        let dir = unique_temp_dir("output-mode-migration");
        std::fs::create_dir_all(&dir).unwrap();
        let json = r#"{
            "rootFolder": "D:\\Series",
            "processing": { "syncWorkers": 4, "segmentCount": 6 }
        }"#;
        std::fs::write(config_file_path(&dir), json).unwrap();

        let loaded = load(&dir);
        assert_eq!(
            loaded.processing.subtitle_output_mode,
            SubtitleOutputMode::SoftSub,
            "تنظیمات قدیمی باید به پیش‌فرض جدید مهاجرت شود"
        );

        // انتخاب صریح کاربر (sidecar) پس از ذخیره و بارگذاری مجدد حفظ می‌شود
        let mut s = loaded;
        s.processing.subtitle_output_mode = SubtitleOutputMode::Sidecar;
        save(&dir, &s).expect("save");
        let raw = std::fs::read_to_string(config_file_path(&dir)).unwrap();
        assert!(
            raw.contains("\"sidecar\""),
            "مقدار serialize باید camelCase باشد: {raw}"
        );
        let reloaded = load(&dir);
        assert_eq!(
            reloaded.processing.subtitle_output_mode,
            SubtitleOutputMode::Sidecar
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
        // فایل قدیمی ممکن است حاوی فیلدهای ناشناخته مانند connectionMode باشد —
        // این فیلدها هنگام بارگذاری نادیده گرفته می‌شوند (serde default).
        let legacy = serde_json::json!({
            "rootFolder": "D:\\Series",
            "subsource": {
                "apiKey": "legacy-secret-key",
                "connectionMode": "manual",
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

        assert!(loaded.subsource.clean_ads);
        assert!(loaded.subsource.skip_existing);
        let text = std::fs::read_to_string(config_file_path(&dir)).unwrap();
        assert!(!text.contains("apiKey"));
        assert!(!text.contains("api_key"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_settings_with_proxy_and_worker_load_and_resave_cleanly() {
        let dir = unique_temp_dir("legacy-compat");
        std::fs::create_dir_all(&dir).unwrap();
        let legacy = serde_json::json!({
            "rootFolder": "D:\\LegacyData",
            "subsource": {
                "connectionMode": "manual",
                "proxy": "http://127.0.0.1:8080",
                "worker": "https://subsource-proxy.worker.dev",
                "cleanAds": true,
                "skipExisting": false
            }
        });
        std::fs::write(
            config_file_path(&dir),
            serde_json::to_vec_pretty(&legacy).unwrap(),
        )
        .unwrap();

        // 1. بارگذاری بدون خطا انجام می‌شود و فیلدهای معتبر خوانده می‌شوند
        let loaded = load(&dir);
        assert_eq!(loaded.root_folder.as_deref(), Some("D:\\LegacyData"));
        assert!(loaded.subsource.clean_ads);
        assert!(!loaded.subsource.skip_existing);

        // 2. ذخیره مجدد باعث حذف دائمی فیلدهای منسوخ‌شده (connectionMode, proxy, worker) می‌شود
        save(&dir, &loaded).expect("ذخیره تنظیمات سازگار با فایل قدیمی باید موفق باشد");
        let saved_text = std::fs::read_to_string(config_file_path(&dir)).unwrap();
        assert!(!saved_text.contains("connectionMode"));
        assert!(!saved_text.contains("proxy"));
        assert!(!saved_text.contains("worker"));
        assert!(saved_text.contains("cleanAds"));
        assert!(saved_text.contains("skipExisting"));

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

    #[test]
    fn save_aborts_if_legacy_key_present_and_migration_fails() {
        let dir = unique_temp_dir("save-abort");
        std::fs::create_dir_all(&dir).unwrap();

        // ایجاد settings.json با apiKey قدیمی و تنظیمات دیگر
        let original = serde_json::json!({
            "rootFolder": "D:\\Existing",
            "subsource": {
                "apiKey": "old-secret-key",
                "connectionMode": "direct"
            }
        });
        let original_text = serde_json::to_string_pretty(&original).unwrap();
        std::fs::write(config_file_path(&dir), &original_text).unwrap();

        // ذخیره با mock store که ناموفق است
        let new_settings = Settings {
            root_folder: Some("D:\\NewFolder".to_string()),
            ..Default::default()
        };
        let result = save_with_store(&dir, &new_settings, |_| {
            Err("credential manager unavailable".into())
        });

        // ذخیره باید ناموفق باشد
        assert!(result.is_err());

        // فایل باید دست‌نخورده باشد
        let after = std::fs::read_to_string(config_file_path(&dir)).unwrap();
        assert_eq!(after, original_text);
        assert!(
            after.contains("old-secret-key"),
            "apiKey باید در فایل بماند"
        );
        assert!(
            after.contains("D:\\\\Existing"),
            "rootFolder قدیمی باید حفظ شود"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_migrates_legacy_key_then_saves_successfully() {
        let dir = unique_temp_dir("save-migrate-ok");
        std::fs::create_dir_all(&dir).unwrap();

        // ایجاد settings.json با apiKey قدیمی
        let original = serde_json::json!({
            "rootFolder": "D:\\OldFolder",
            "subsource": {
                "apiKey": "migrate-me-key",
                "connectionMode": "manual"
            }
        });
        std::fs::write(
            config_file_path(&dir),
            serde_json::to_vec_pretty(&original).unwrap(),
        )
        .unwrap();

        let stored = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let captured = stored.clone();

        // ذخیره با mock store که موفق است
        let new_settings = Settings {
            root_folder: Some("D:\\NewFolder".to_string()),
            ..Default::default()
        };
        let result = save_with_store(&dir, &new_settings, move |key| {
            *captured.lock().unwrap() = Some(key.to_string());
            Ok(())
        });

        // ذخیره باید موفق باشد
        assert!(result.is_ok());
        assert_eq!(
            *stored.lock().unwrap(),
            Some("migrate-me-key".to_string()),
            "کلید قدیمی باید به store منتقل شده باشد"
        );

        // فایل جدید نباید apiKey داشته باشد
        let text = std::fs::read_to_string(config_file_path(&dir)).unwrap();
        assert!(!text.contains("apiKey"), "apiKey نباید در فایل ذخیره شود");
        assert!(
            !text.contains("migrate-me-key"),
            "مقدار کلید نباید در فایل باشد"
        );

        // تنظیمات جدید باید ذخیره شده باشند
        let saved: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            saved["rootFolder"].as_str(),
            Some("D:\\NewFolder"),
            "rootFolder جدید باید ذخیره شده باشد"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn take_legacy_api_key_only_camel_case() {
        let mut json = serde_json::json!({
            "subsource": {
                "apiKey": "camel-only-key",
                "cleanAds": true
            }
        });
        let action = take_legacy_api_key(&mut json).expect("should succeed");
        assert_eq!(
            action,
            LegacyKeyAction::Migrate("camel-only-key".to_string())
        );
        assert!(!json["subsource"]
            .as_object()
            .unwrap()
            .contains_key("apiKey"));
    }

    #[test]
    fn take_legacy_api_key_only_snake_case() {
        let mut json = serde_json::json!({
            "subsource": {
                "api_key": "snake-only-key",
                "cleanAds": true
            }
        });
        let action = take_legacy_api_key(&mut json).expect("should succeed");
        assert_eq!(
            action,
            LegacyKeyAction::Migrate("snake-only-key".to_string())
        );
        assert!(!json["subsource"]
            .as_object()
            .unwrap()
            .contains_key("api_key"));
    }

    #[test]
    fn take_legacy_api_key_both_identical() {
        let mut json = serde_json::json!({
            "subsource": {
                "apiKey": "identical-key",
                "api_key": "identical-key"
            }
        });
        let action = take_legacy_api_key(&mut json).expect("should succeed");
        assert_eq!(
            action,
            LegacyKeyAction::Migrate("identical-key".to_string())
        );
        let sub = json["subsource"].as_object().unwrap();
        assert!(!sub.contains_key("apiKey"));
        assert!(!sub.contains_key("api_key"));
    }

    #[test]
    fn take_legacy_api_key_one_empty_one_valid_camel() {
        let mut json = serde_json::json!({
            "subsource": {
                "apiKey": "valid-camel-key",
                "api_key": ""
            }
        });
        let action = take_legacy_api_key(&mut json).expect("should succeed");
        assert_eq!(
            action,
            LegacyKeyAction::Migrate("valid-camel-key".to_string())
        );
        let sub = json["subsource"].as_object().unwrap();
        assert!(!sub.contains_key("apiKey"));
        assert!(!sub.contains_key("api_key"));
    }

    #[test]
    fn take_legacy_api_key_one_empty_one_valid_snake() {
        let mut json = serde_json::json!({
            "subsource": {
                "apiKey": null,
                "api_key": "valid-snake-key"
            }
        });
        let action = take_legacy_api_key(&mut json).expect("should succeed");
        assert_eq!(
            action,
            LegacyKeyAction::Migrate("valid-snake-key".to_string())
        );
        let sub = json["subsource"].as_object().unwrap();
        assert!(!sub.contains_key("apiKey"));
        assert!(!sub.contains_key("api_key"));
    }

    #[test]
    fn take_legacy_api_key_both_empty_or_null() {
        let mut json = serde_json::json!({
            "subsource": {
                "apiKey": null,
                "api_key": "   "
            }
        });
        let action = take_legacy_api_key(&mut json).expect("should succeed");
        assert_eq!(action, LegacyKeyAction::RemoveOnly);
        let sub = json["subsource"].as_object().unwrap();
        assert!(!sub.contains_key("apiKey"));
        assert!(!sub.contains_key("api_key"));
    }

    #[test]
    fn take_legacy_api_key_conflict_aborts_and_keeps_json_intact() {
        let mut json = serde_json::json!({
            "subsource": {
                "apiKey": "key-alpha",
                "api_key": "key-beta",
                "cleanAds": true
            }
        });
        let result = take_legacy_api_key(&mut json);
        assert!(result.is_err(), "different keys must cause conflict error");
        let err = result.unwrap_err();
        assert!(
            !err.contains("key-alpha"),
            "error string must not leak apiKey"
        );
        assert!(
            !err.contains("key-beta"),
            "error string must not leak api_key"
        );
        assert!(
            err.contains("متفاوت هستند"),
            "must return generic conflict message"
        );
        let sub = json["subsource"].as_object().unwrap();
        assert_eq!(sub.get("apiKey").unwrap().as_str(), Some("key-alpha"));
        assert_eq!(sub.get("api_key").unwrap().as_str(), Some("key-beta"));
    }

    #[test]
    fn load_aborts_migration_on_conflicting_legacy_keys_and_leaves_file_untouched() {
        let dir = unique_temp_dir("load-conflict");
        std::fs::create_dir_all(&dir).unwrap();

        let original = serde_json::json!({
            "rootFolder": "D:\\OriginalFolder",
            "subsource": {
                "apiKey": "key-first",
                "api_key": "key-second"
            }
        });
        let original_text = serde_json::to_string_pretty(&original).unwrap();
        std::fs::write(config_file_path(&dir), &original_text).unwrap();

        // بارگذاری با store_api_key که نباید هرگز صدا زده شود
        let loaded = load_with_store(&dir, |_| {
            panic!("در صورت تضاد کلیدها، مهاجرت نباید اجرا شود");
        });

        assert_eq!(loaded.root_folder.as_deref(), Some("D:\\OriginalFolder"));

        // فایل روی دیسک کاملاً دست‌نخورده باقی مانده است
        let text_after = std::fs::read_to_string(config_file_path(&dir)).unwrap();
        assert_eq!(text_after, original_text);
        assert!(text_after.contains("key-first"));
        assert!(text_after.contains("key-second"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_aborts_on_conflicting_legacy_keys_and_leaves_file_untouched() {
        let dir = unique_temp_dir("save-conflict");
        std::fs::create_dir_all(&dir).unwrap();

        let original = serde_json::json!({
            "rootFolder": "D:\\KeepMe",
            "subsource": {
                "apiKey": "key-apple",
                "api_key": "key-orange"
            }
        });
        let original_text = serde_json::to_string_pretty(&original).unwrap();
        std::fs::write(config_file_path(&dir), &original_text).unwrap();

        let new_settings = Settings {
            root_folder: Some("D:\\OverwrittenAttempt".to_string()),
            ..Default::default()
        };

        // تلاش برای ذخیره با mock store که نباید صدا زده شود
        let save_result = save_with_store(&dir, &new_settings, |_| {
            panic!("در صورت تضاد کلیدها، مهاجرت نباید اجرا شود");
        });

        // ذخیره باید با خطا متوقف شود
        assert!(save_result.is_err());
        let err_msg = save_result.unwrap_err();
        assert!(err_msg.contains("تضاد در کلیدهای API قدیمی"));
        assert!(!err_msg.contains("key-apple"), "error must not leak apiKey");
        assert!(
            !err_msg.contains("key-orange"),
            "error must not leak api_key"
        );

        // فایل روی دیسک نباید به هیچ وجه دست خورده باشد
        let text_after = std::fs::read_to_string(config_file_path(&dir)).unwrap();
        assert_eq!(text_after, original_text);
        assert!(text_after.contains("key-apple"));
        assert!(text_after.contains("key-orange"));
        assert!(!text_after.contains("OverwrittenAttempt"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn conflict_error_and_file_preservation_never_leak_secret_keys() {
        let secret_a = "SUPER_SECRET_KEY_ALPHA_9999";
        let secret_b = "SUPER_SECRET_KEY_BETA_8888";

        // 1. بررسی take_legacy_api_key: مقادیر کلید نباید در متن خطا نشت کنند
        let mut json = serde_json::json!({
            "subsource": {
                "apiKey": secret_a,
                "api_key": secret_b
            }
        });
        let err = take_legacy_api_key(&mut json).expect_err("must fail with conflict");
        assert!(
            !err.contains(secret_a),
            "take_legacy_api_key error message leaked secret_a"
        );
        assert!(
            !err.contains(secret_b),
            "take_legacy_api_key error message leaked secret_b"
        );
        assert!(
            err.contains("متفاوت هستند"),
            "must return generic conflict message"
        );

        // 2. بررسی save_with_store: متن خطای ذخیره نباید کلیدها را افشا کند
        let dir = unique_temp_dir("conflict-leak-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file_content = serde_json::to_string_pretty(&json).unwrap();
        std::fs::write(config_file_path(&dir), &file_content).unwrap();

        let s = Settings::default();
        let save_err = save_with_store(&dir, &s, |_| panic!("store should not be called"))
            .expect_err("save must fail on conflict");
        assert!(
            !save_err.contains(secret_a),
            "save error string leaked secret_a"
        );
        assert!(
            !save_err.contains(secret_b),
            "save error string leaked secret_b"
        );

        // 3. بررسی load_with_store: در زمان conflict نباید کرش کند یا migration ناقص انجام دهد
        let loaded = load_with_store(&dir, |_| panic!("store should not be called"));
        assert_eq!(loaded.subsource, SubSourceSettings::default());

        // 4. تأیید دست‌نخورده ماندن دقیق فایل کانفیگ روی دیسک
        let after_file = std::fs::read_to_string(config_file_path(&dir)).unwrap();
        assert_eq!(after_file, file_content);
        assert!(after_file.contains(secret_a));
        assert!(after_file.contains(secret_b));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
