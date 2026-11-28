//! دستورات زیرنویس لوکال فاز ۳ — تحلیل/تبدیل encoding و ویرایشگر SRT.
//!
//! قراردادهای امنیتی:
//! - فایل اصلی هرگز بازنویسی/تغییر نمی‌کند؛ همهٔ خروجی‌ها فایل جدید با
//!   create_new اتمیک (tmp + rename) هستند.
//! - تشخیص encoding نامطمئن → تبدیل رد می‌شود (بایت‌های دلخواه هرگز به
//!   فارسی تفسیر نمی‌شوند) — همان گیت ۱۰-حرفِ مسیر دانلود.
//! - ویرایش فقط SRT؛ ذخیره fail-closed روی خطاهای اعتبارسنجی.
//! - گارد فایل لوکال: مطلق + معمولی + بدون symlink/reparse.

use crate::models::{EditorLoadResult, EditorSaveOutcome, EncodingConvertOutcome};
/// گارد امنیتی مشترک فایل لوکال — معمولی، موجود، بدون symlink/reparse.
fn ensure_local_plain_file(path: &str) -> Result<std::path::PathBuf, String> {
    let p = std::path::PathBuf::from(path.trim());
    if !p.is_absolute() {
        return Err("مسیر فایل زیرنویس باید مطلق باشد.".into());
    }
    let meta = std::fs::symlink_metadata(&p).map_err(|_| format!("فایل «{path}» پیدا نشد."))?;
    if !meta.is_file() || crate::archive::listing::is_link_or_reparse(&meta) {
        return Err("فایل زیرنویس باید یک فایل معمولی باشد (بدون پیوند/Reparse).".into());
    }
    Ok(p)
}

/// تحلیل encoding یک زیرنویس لوکال — فقط خواندن؛ هیچ تغییری.
#[tauri::command]
pub fn analyze_subtitle_encoding(
    path: String,
) -> Result<crate::subtitle::encoding::SubtitleEncodingReport, String> {
    let p = ensure_local_plain_file(&path)?;
    let bytes = std::fs::read(&p).map_err(|e| format!("خواندن فایل ناموفق بود: {e}"))?;
    Ok(crate::subtitle::encoding::analyze_subtitle_bytes(&bytes))
}

/// تبدیل زیرنویس لوکال به UTF-8 — خروجی جدید `<stem>.utf8.<ext>` کنار فایل
/// اصلی؛ اصل هرگز بازنویسی/تغییر نمی‌کند؛ atomic با tmp + rename.
#[tauri::command]
pub fn convert_subtitle_to_utf8(path: String) -> Result<EncodingConvertOutcome, String> {
    use crate::subtitle::encoding::{analyze_subtitle_bytes, convert_to_utf8_text};

    let p = ensure_local_plain_file(&path)?;
    let bytes = std::fs::read(&p).map_err(|e| format!("خواندن فایل ناموفق بود: {e}"))?;
    let report = analyze_subtitle_bytes(&bytes);
    if !report.convertible || !report.confident {
        return Err(report.detail.unwrap_or_else(|| {
            "این فایل یا از قبل UTF-8 است یا تشخیص encoding قابل‌اعتماد ندارد — تبدیل لازم نیست/ممکن نیست.".into()
        }));
    }
    let text = convert_to_utf8_text(&bytes, report.kind)?;
    // خروجی باید ساختار معتبر با متن حفظ‌شده داشته باشد — قبل از نوشتن
    let ext = p
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let validation =
        crate::subtitle::validation::validate_bytes_for_extension(text.as_bytes(), &ext);
    if !validation.valid {
        return Err(format!(
            "خروجی تبدیل اعتبارسنجی نشد — تبدیل دور ریخته شد: {}",
            validation.error.unwrap_or_else(|| "نامشخص".into())
        ));
    }

    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dest = p.with_file_name(format!("{}.utf8.{}", stem, ext));
    if dest.exists() {
        return Err(format!(
            "خروجی «{}» از قبل موجود است — برای جلوگیری از بازنویسی هیچ فایلی دست نخورد؛ ابتدا آن را حذف/تغییر نام دهید.",
            dest.display()
        ));
    }
    let tmp = p.with_file_name(format!(
        "{}.utf8.converting-{}.tmp",
        stem,
        crate::subtitle::extraction::unique_probe_token()
    ));
    std::fs::write(&tmp, text.as_bytes()).map_err(|e| format!("نوشتن موقت تبدیل: {e}"))?;
    if let Err(err) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("جایگزینی خروجی تبدیل: {err}"));
    }
    crate::logging::info(
        "subtitle",
        &format!("تبدیل encoding: {} → {}", p.display(), dest.display()),
    );
    Ok(EncodingConvertOutcome {
        output_path: dest.to_string_lossy().into_owned(),
        from_label: report.label,
        persian_chars: report.persian_chars,
    })
}

/// بازکردن زیرنویس لوکال برای ویرایش — فقط SRT در این فاز؛ encoding غیر
/// UTF-8/16 ابتدا باید با «تبدیل encoding» تبدیل شود.
#[tauri::command]
pub fn load_subtitle_for_edit(path: String) -> Result<EditorLoadResult, String> {
    use crate::subtitle::encoding::{analyze_subtitle_bytes, decode_bytes, SubtitleEncodingKind};

    let p = ensure_local_plain_file(&path)?;
    let ext = p
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if ext != "srt" {
        return Err(format!(
            "ویرایشگر در این فاز فقط SRT را باز می‌کند (پسوند فعلی: {ext}) — فرمت‌های استایل‌دار مثل ASS ساختار حفظ‌کردنی با ویرایش سبک ندارند."
        ));
    }
    let bytes = std::fs::read(&p).map_err(|e| format!("خواندن فایل ناموفق بود: {e}"))?;
    let kind = analyze_subtitle_bytes(&bytes).kind;
    if !matches!(
        kind,
        SubtitleEncodingKind::Utf8
            | SubtitleEncodingKind::Utf8Bom
            | SubtitleEncodingKind::Utf16Le
            | SubtitleEncodingKind::Utf16Be
    ) {
        return Err(
            "این فایل UTF-8/UTF-16 معتبر نیست (احتمالاً cp1256) — ابتدا از بخش «تبدیل encoding» به UTF-8 تبدیلش کنید.".into(),
        );
    }
    let (_, text) = decode_bytes(&bytes);
    let mut result = crate::subtitle::editor::parse_srt_for_edit(&text)
        .map_err(|e| format!("پارس SRT ناموفق بود: {e}"))?;
    result.path = p.to_string_lossy().into_owned();
    Ok(result)
}

/// اعتبارسنجی زندهٔ cueها — خطاها Err می‌شوند، هشدارها در Ok برمی‌گردند؛
/// هیچ فایلی لمس نمی‌شود.
#[tauri::command]
pub fn validate_subtitle_cues(
    cues: Vec<crate::models::SubtitleCue>,
) -> Result<EditorLoadResult, String> {
    let (errors, warnings) = crate::subtitle::editor::validate_cues(&cues);
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(EditorLoadResult {
        path: String::new(),
        cues: Vec::new(),
        warnings,
    })
}

/// ذخیرهٔ ویرایش — مقصد جدید `<stem>.edited.srt` کنار اصل؛ اصل هرگز
/// بازنویسی نمی‌شود؛ atomic با tmp + rename؛ اعتبارسنجی fail-closed.
#[tauri::command]
pub fn save_subtitle_edited(
    path: String,
    cues: Vec<crate::models::SubtitleCue>,
) -> Result<EditorSaveOutcome, String> {
    let p = ensure_local_plain_file(&path)?;
    let (errors, warnings) = crate::subtitle::editor::validate_cues(&cues);
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    if cues.is_empty() {
        return Err("هیچ cueای برای ذخیره نیست — لیست خالی است.".into());
    }
    let content = crate::subtitle::editor::write_srt(&cues);
    // خروجی باید خودش validator فعلی پروژه را رد نکند (ساختار SRT سالم)
    let validation =
        crate::subtitle::validation::validate_bytes_for_extension(content.as_bytes(), "srt");
    if !validation.valid {
        return Err(format!(
            "خروجی ویرایش اعتبارسنجی نشد — ذخیره لغو شد: {}",
            validation.error.unwrap_or_else(|| "نامشخص".into())
        ));
    }
    let Some(dest) = crate::subtitle::editor::edited_target(&p) else {
        return Err("ساخت مسیر مقصد ذخیره ممکن نبود.".into());
    };
    if dest.exists() {
        return Err(format!(
            "خروجی «{}» از قبل موجود است — برای جلوگیری از بازنویسی هیچ فایلی دست نخورد.",
            dest.display()
        ));
    }
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = p.with_file_name(format!(
        "{}.edited-{}.tmp",
        stem,
        crate::subtitle::extraction::unique_probe_token()
    ));
    std::fs::write(&tmp, content.as_bytes()).map_err(|e| format!("نوشتن موقت ویرایش: {e}"))?;
    if let Err(err) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("جایگزینی خروجی ویرایش: {err}"));
    }
    crate::logging::info(
        "subtitle",
        &format!("ویرایش ذخیره شد: {} ({} cue)", dest.display(), cues.len()),
    );
    Ok(EditorSaveOutcome {
        output_path: dest.to_string_lossy().into_owned(),
        cue_count: cues.len(),
        warnings,
    })
}
