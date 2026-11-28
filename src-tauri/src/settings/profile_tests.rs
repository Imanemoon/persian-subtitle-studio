//! تست‌های پروفایل‌ها، مهاجرت و اعمال — مستقل از Tauri.

use super::*;

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "subsync-profile-tests-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn default_settings_carry_builtin_profile_active() {
    let s = Settings::default();
    assert_eq!(s.profiles.len(), 1);
    assert_eq!(s.profiles[0].id, BUILTIN_DEFAULT_PROFILE_ID);
    assert!(s.profiles[0].built_in);
    assert_eq!(
        s.active_profile_id.as_deref(),
        Some(BUILTIN_DEFAULT_PROFILE_ID)
    );
    // مقادیر مؤثر = مقادیر پیش‌فرض — هیچ چیز جابه‌جا نشده
    assert_eq!(s.processing, ProcessingSettings::default());
    assert_eq!(s.automation, AutomationSettings::default());
    assert_eq!(
        s.naming.synced_sidecar_base,
        crate::output_naming::DEFAULT_BASE_TEMPLATE
    );
}

#[test]
fn legacy_settings_without_profiles_get_default_profile_from_effective_values() {
    // تنظیمات قدیمی: بدون profiles/activeProfileId/naming — مقادیر مؤثر سفارشی
    let json = r#"{
        "processing": {"syncWorkers": 5, "multiSegment": false, "segmentCount": 9,
            "skipIntroOutro": false, "skipSyncOnLowQuality": false,
            "subtitleOutputMode": "sidecar"},
        "automation": {"mode": "auto", "maxSyncAttempts": 4}
    }"#;
    let parsed: Settings = serde_json::from_str(json).unwrap();
    assert!(parsed.profiles.is_empty());
    assert!(parsed.active_profile_id.is_none());

    let migrated = parsed.sanitized();
    // پروفایل پیش‌فرض از مقادیر مؤثر کاربر snapshot می‌شود — مهاجرت بدون تغییر رفتار
    assert_eq!(migrated.profiles.len(), 1);
    let builtin = &migrated.profiles[0];
    assert_eq!(builtin.id, BUILTIN_DEFAULT_PROFILE_ID);
    assert_eq!(builtin.processing.sync_workers, 5);
    assert!(!builtin.processing.multi_segment);
    assert_eq!(
        builtin.processing.subtitle_output_mode,
        SubtitleOutputMode::Sidecar
    );
    assert_eq!(builtin.automation.mode, AutomationMode::Auto);
    assert_eq!(builtin.automation.max_sync_attempts, 4);
    // فعال‌شدن پروفایل فقط نمایشی است — مقادیر مؤثر همان قبلی‌اند
    assert_eq!(
        migrated.active_profile_id.as_deref(),
        Some(BUILTIN_DEFAULT_PROFILE_ID)
    );
    assert_eq!(migrated.processing.sync_workers, 5);
    assert_eq!(migrated.automation.mode, AutomationMode::Auto);
}

#[test]
fn deleted_builtin_profile_is_restored_on_sanitize() {
    let mut s = Settings::default();
    s.profiles.clear();
    s = s.sanitized();
    assert_eq!(s.profiles.len(), 1);
    assert_eq!(s.profiles[0].id, BUILTIN_DEFAULT_PROFILE_ID);
}

#[test]
fn stale_active_profile_id_is_cleared_not_fatal() {
    let mut s = Settings {
        active_profile_id: Some("ghost-profile".into()),
        ..Default::default()
    };
    s = s.sanitized();
    assert_eq!(s.active_profile_id, None);
    // مقادیر مؤثر دست‌نخورده — رفتار فعلی حفظ می‌شود
    assert_eq!(s.processing, ProcessingSettings::default());
}

#[test]
fn normalize_profiles_accepts_builtin_plus_custom() {
    let builtin = ProcessingProfile::default();
    let custom = ProcessingProfile {
        id: String::new(),
        name: "  سریال‌های جدید  ".into(),
        built_in: true,
        ..Default::default()
    }; // ورودی فرانت نادیده گرفته می‌شود

    let out = normalize_profiles(vec![builtin, custom]).unwrap();
    assert_eq!(out.len(), 2);
    let created = out
        .iter()
        .find(|p| p.name == "سریال‌های جدید")
        .expect("نام باید trim شده باشد");
    assert!(!created.id.is_empty(), "شناسه باید backend-generated باشد");
    assert_ne!(created.id, BUILTIN_DEFAULT_PROFILE_ID);
    assert!(
        !created.built_in,
        "فلگ ساختی فقط برای پیش‌فرض رسمی معنا دارد"
    );
}

#[test]
fn normalize_profiles_rejects_duplicate_names_and_empty() {
    let builtin = ProcessingProfile::default();
    let a = ProcessingProfile {
        id: String::new(),
        name: "حجاب".into(),
        ..Default::default()
    };
    let b = ProcessingProfile {
        id: String::new(),
        name: "  حجاب ".into(),
        ..Default::default()
    };
    // نام یکتا بدون حساسیت به فاصله/اندازه — دو «حجاب» رد می‌شوند
    assert!(normalize_profiles(vec![builtin.clone(), a, b]).is_err());
    // نام متفاوت از builtin مشکلی ندارد
    let builtin2 = ProcessingProfile::default();
    let c = ProcessingProfile {
        id: String::new(),
        name: "سریال".into(),
        ..Default::default()
    };
    assert!(normalize_profiles(vec![builtin2, c]).is_ok());
    // نام خالی رد می‌شود
    let builtin3 = ProcessingProfile::default();
    let d = ProcessingProfile {
        id: String::new(),
        name: "   ".into(),
        ..Default::default()
    };
    assert!(normalize_profiles(vec![builtin3, d]).is_err());
}

#[test]
fn normalize_profiles_rejects_deleting_builtin() {
    assert!(
        normalize_profiles(vec![]).is_err(),
        "حذف پروفایل پیش‌فرض ساختی باید رد شود"
    );
}

#[test]
fn normalize_profiles_rejects_invalid_templates() {
    let builtin = ProcessingProfile::default();
    let custom = ProcessingProfile {
        id: String::new(),
        name: "بد".into(),
        naming: NamingSettings {
            synced_sidecar_base: "{name}-{title}".into(),
        },
        ..Default::default()
    };
    let err = normalize_profiles(vec![builtin, custom]).unwrap_err();
    assert!(err.contains("ناشناخته"), "{err}");
}

#[test]
fn normalize_profiles_rejects_duplicate_ids() {
    let builtin = ProcessingProfile::default();
    let a = ProcessingProfile {
        id: "same-id".into(),
        name: "اول".into(),
        ..Default::default()
    };
    let b = ProcessingProfile {
        id: "same-id".into(),
        name: "دوم".into(),
        ..Default::default()
    };
    let err = normalize_profiles(vec![builtin, a, b]).unwrap_err();
    assert!(err.contains("تکراری"), "{err}");
}

#[test]
fn apply_profile_copies_effective_values_and_clears_on_invalid() {
    let mut s = Settings::default();
    let builtin = ProcessingProfile::default();
    let custom = ProcessingProfile {
        id: String::new(),
        name: "سینک سنگین".into(),
        processing: ProcessingSettings {
            sync_workers: 8,
            ..Default::default()
        },
        automation: AutomationSettings {
            mode: AutomationMode::Auto,
            ..Default::default()
        },
        naming: NamingSettings {
            synced_sidecar_base: "{show} S{season}E{episode}".into(),
        },
        ..Default::default()
    };
    s.profiles = normalize_profiles(vec![builtin, custom]).unwrap();

    let new_id = s
        .profiles
        .iter()
        .find(|p| p.id != BUILTIN_DEFAULT_PROFILE_ID)
        .unwrap()
        .id
        .clone();
    s.apply_profile_core(Some(&new_id)).unwrap();
    assert_eq!(s.active_profile_id.as_deref(), Some(new_id.as_str()));
    assert_eq!(s.processing.sync_workers, 8);
    assert_eq!(s.automation.mode, AutomationMode::Auto);
    assert_eq!(s.naming.synced_sidecar_base, "{show} S{season}E{episode}");

    // شناسهٔ نامعتبر → Err صریح و بدون تغییر
    let before_workers = s.processing.sync_workers;
    let before_active = s.active_profile_id.clone();
    assert!(s.apply_profile_core(Some("ghost")).is_err());
    assert_eq!(s.processing.sync_workers, before_workers);
    assert_eq!(s.active_profile_id, before_active);

    // None → فقط جداسازی؛ مقادیر مؤثر همان اعمال‌شده می‌مانند
    s.apply_profile_core(None).unwrap();
    assert_eq!(s.active_profile_id, None);
    assert_eq!(s.processing.sync_workers, 8);
}

#[test]
fn profile_settings_roundtrip_through_json() {
    let dir = unique_dir("roundtrip");
    let mut s = Settings::default();
    let builtin = ProcessingProfile::default();
    let custom = ProcessingProfile {
        id: String::new(),
        name: "فیلم‌های قدیمی".into(),
        processing: ProcessingSettings {
            subtitle_output_mode: SubtitleOutputMode::Sidecar,
            ..Default::default()
        },
        naming: NamingSettings {
            synced_sidecar_base: "{show} - S{season}E{episode}".into(),
        },
        ..Default::default()
    };
    s.profiles = normalize_profiles(vec![builtin, custom]).unwrap();
    let created_id = s
        .profiles
        .iter()
        .find(|p| p.name == "فیلم‌های قدیمی")
        .unwrap()
        .id
        .clone();
    s.apply_profile_core(Some(&created_id)).unwrap();

    save(&dir, &s).expect("save");
    let loaded = load(&dir);
    assert_eq!(loaded.profiles.len(), s.profiles.len());
    assert_eq!(
        loaded.naming.synced_sidecar_base, "{show} - S{season}E{episode}",
        "مقادیر مؤثر اعمال‌شده هم ماندگارند"
    );
    // پروفایل سفارشی با شناسهٔ پایدار همان مانده
    let original = s
        .profiles
        .iter()
        .find(|p| p.name == "فیلم‌های قدیمی")
        .unwrap();
    let loaded_custom = loaded
        .profiles
        .iter()
        .find(|p| p.name == "فیلم‌های قدیمی")
        .expect("پروفایل سفارشی باید ماندگار باشد");
    assert_eq!(original.id, loaded_custom.id);
    assert_eq!(
        loaded_custom.naming.synced_sidecar_base,
        "{show} - S{season}E{episode}"
    );
    assert_eq!(
        loaded.active_profile_id.as_deref(),
        Some(created_id.as_str())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_settings_json_roundtrip_preserves_profile_fields() {
    // تنظیمات قدیمی بدون فیلد پروفایل → بارگذاری → save → دوباره بارگذاری:
    // پروفایل seedشده ماندگار می‌شود و مقادیر مؤثر ثابت می‌مانند
    let dir = unique_dir("legacy-migrate");
    let legacy = r#"{
        "processing": {"syncWorkers": 4, "multiSegment": true, "segmentCount": 6,
            "skipIntroOutro": true, "skipSyncOnLowQuality": true,
            "subtitleOutputMode": "softSub"}
    }"#;
    std::fs::write(crate::settings::config_file_path(&dir), legacy).unwrap();
    let first = load(&dir);
    assert_eq!(first.profiles.len(), 1);
    assert_eq!(first.processing.sync_workers, 4);
    save(&dir, &first).unwrap();
    let second = load(&dir);
    assert_eq!(second.profiles.len(), 1);
    assert_eq!(second.profiles[0].id, BUILTIN_DEFAULT_PROFILE_ID);
    assert_eq!(second.profiles[0].processing.sync_workers, 4);
    assert_eq!(
        second.active_profile_id.as_deref(),
        Some(BUILTIN_DEFAULT_PROFILE_ID)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
