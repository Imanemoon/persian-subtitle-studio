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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct SubSourceSettings {
    /// API key برای SubSource (حساس — هرگز لاگ نمی‌شود)
    pub api_key: Option<String>,

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
            api_key: None,
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
pub fn load(config_dir: &Path) -> Settings {
    let path = config_file_path(config_dir);
    match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<Settings>(&bytes) {
            Ok(s) => s.sanitized(),
            Err(err) => {
                let backup = config_dir.join(format!(
                    "settings.corrupt-{}.json",
                    chrono::Local::now().format("%Y%m%d-%H%M%S")
                ));
                let _ = std::fs::copy(&path, &backup);
                crate::logging::warn(
                    "settings",
                    &format!(
                        "فایل تنظیمات قابل خواندن نبود ({err})؛ پشتیبان: {} — تنظیمات پیش‌فرض بارگذاری شد",
                        backup.display()
                    ),
                );
                Settings::default()
            }
        },
        Err(_) => Settings::default(),
    }
}

/// ذخیره امن تنظیمات: اول در فایل موقت، سپس جایگزینی.
pub fn save(config_dir: &Path, settings: &Settings) -> Result<(), String> {
    let clean = settings.clone().sanitized();
    std::fs::create_dir_all(config_dir).map_err(|e| format!("عدم امکان ساخت پوشه تنظیمات: {e}"))?;
    let tmp = config_dir.join("settings.json.tmp");
    let data = serde_json::to_vec_pretty(&clean).map_err(|e| format!("سریال‌سازی تنظیمات: {e}"))?;
    std::fs::write(&tmp, data).map_err(|e| format!("نوشتن تنظیمات ناموفق بود: {e}"))?;
    std::fs::rename(&tmp, config_file_path(config_dir))
        .map_err(|e| format!("جایگزینی فایل تنظیمات ناموفق بود: {e}"))?;
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
}
