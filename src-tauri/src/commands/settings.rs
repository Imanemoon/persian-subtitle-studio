//! دستورات دریافت و ذخیره تنظیمات + مدیریت پروفایل‌ها.

use tauri::{AppHandle, Manager, State};

use crate::models::NamingPreview;
use crate::settings::{ProcessingProfile, Settings};
use crate::state::{AppPaths, AppState};

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Settings, String> {
    let clean = settings.sanitized();

    if let Some(root) = &clean.root_folder {
        if !std::path::Path::new(root).is_dir() {
            crate::logging::warn(
                "settings",
                &format!("پوشه ریشه تنظیم‌شده وجود ندارد: {root}"),
            );
        }
    }

    let paths = app.state::<AppPaths>();
    crate::settings::save(&paths.config_dir, &clean)?;
    *state.settings.lock().unwrap() = clean.clone();
    crate::logging::info("settings", "تنظیمات ذخیره شد");
    Ok(clean)
}

/// ذخیرهٔ کل فهرست پروفایل‌ها — ایجاد/ویرایش/تکثیر/حذف/تغییر نام همه از
/// همین مرز می‌گذرند تا اعتبارسنجی و شناسه‌گذاری backend-owned بماند.
/// پروفایل فعالِ حذف‌شده پاک می‌شود؛ مقادیر مؤثر هرگز اینجا تغییر نمی‌کنند
/// (اعمال پروفایل کار `apply_profile` است).
#[tauri::command]
pub fn save_profiles(
    app: AppHandle,
    state: State<'_, AppState>,
    profiles: Vec<ProcessingProfile>,
) -> Result<Settings, String> {
    let normalized = crate::settings::normalize_profiles(profiles)?;

    let paths = app.state::<AppPaths>();
    let mut clean = state.settings.lock().unwrap().clone();
    clean.profiles = normalized;
    clean = clean.sanitized();
    crate::settings::save(&paths.config_dir, &clean)?;
    *state.settings.lock().unwrap() = clean.clone();
    crate::logging::info(
        "settings",
        &format!(
            "فهرست پروفایل‌ها ذخیره شد — {} پروفایل",
            clean.profiles.len()
        ),
    );
    Ok(clean)
}

/// اعمال پروفایل روی مقادیر مؤثر — `None` فقط انتخاب را جداسازی می‌کند
/// (مقادیر فعلی دست‌نخورده). شناسهٔ نامعتبر → Err صریح.
#[tauri::command]
pub fn apply_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: Option<String>,
) -> Result<Settings, String> {
    let paths = app.state::<AppPaths>();
    let mut clean = state.settings.lock().unwrap().clone();
    clean.apply_profile_core(profile_id.as_deref())?;
    clean = clean.sanitized();
    crate::settings::save(&paths.config_dir, &clean)?;
    *state.settings.lock().unwrap() = clean.clone();
    crate::logging::info(
        "settings",
        &match profile_id.as_deref() {
            Some(id) => format!("پروفایل اعمال شد: {id}"),
            None => "انتخاب پروفایل جداسازی شد — مقادیر مؤثر دست‌نخورده ماندند".into(),
        },
    );
    Ok(clean)
}

/// پیش‌نمایش قطعی قالب نام روی متادیتای نمونه — فقط رندر رشته؛ هیچ
/// مسیری ساخته نمی‌شود و هیچ فایلی لمس نمی‌شود. قالب نامعتبر → Err.
#[tauri::command]
pub fn preview_sidecar_name(
    template: String,
    video_name: String,
    show: Option<String>,
    season: Option<u32>,
    episode: Option<u32>,
) -> Result<NamingPreview, String> {
    crate::output_naming::validate_template(&template)?;
    let stem = std::path::Path::new(video_name.trim())
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| video_name.trim().to_string());
    let ctx = crate::output_naming::OutputNameContext {
        name: stem.clone(),
        show,
        season,
        episode,
        parent: "SampleFolder".into(),
    };
    match crate::output_naming::render_base(&template, &ctx) {
        Some(base) => Ok(NamingPreview {
            file_name: format!("{base}.fa.synced.srt"),
            fell_back: false,
        }),
        // متادیتای گم‌شده → پیش‌نمایش همان fallback قطعی موتور
        None => Ok(NamingPreview {
            file_name: format!("{stem}.fa.synced.srt"),
            fell_back: true,
        }),
    }
}
