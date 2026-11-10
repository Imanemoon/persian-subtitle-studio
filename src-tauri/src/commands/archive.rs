//! دستورات Tauri برای استخراج آرشیو و تطبیق قسمت (فاز ۳ PR ۳).
//!
//! پلن در `spawn_blocking` اجرا می‌شود تا IPC قفل نشود؛ اجرا مانند دانلود
//! گروهی در یک thread پس‌زمینه با گارد RAII در `OpsRegistry` انجام می‌شود.
//! پیشرفت فقط بر اساس مراحل واقعی (تعداد منابع پردازش‌شده) گزارش می‌شود.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::archive::engine::{execute_archive_extraction as run_archive_extraction, plan_archives};
use crate::archive::types::{
    ArchiveExecuteRequest, ArchiveExecuteResult, ArchiveExtractionPlan, ArchiveSource,
};
use crate::models::OpsProgress;
use crate::state::{acquire_batch_op, AppState};

use super::media::resolve_tool;

/// رویداد پایان استخراج — payload کامل و ساختاریافته.
pub const ARCHIVE_DONE_EVENT: &str = "archive:done";

pub fn archive_done_payload(result: &ArchiveExecuteResult) -> serde_json::Value {
    serde_json::to_value(result).unwrap_or(serde_json::Value::Null)
}

/// پلن استخراج: بازرسی/اعتبارسنجی کامل بدون تغییر دائمی روی دیسک.
#[tauri::command]
pub async fn plan_archive_extraction(
    app: AppHandle,
    state: State<'_, AppState>,
    sources: Vec<ArchiveSource>,
) -> Result<ArchiveExtractionPlan, String> {
    if sources.is_empty() {
        return Err("هیچ منبع آرشیوی برای بررسی وجود ندارد.".into());
    }
    let seven_zip = resolve_tool(&state, "sevenZip")?;
    let cancel = Arc::new(AtomicBool::new(false));

    tauri::async_runtime::spawn_blocking(move || {
        plan_archives(&seven_zip, &sources, &cancel, &mut |p: OpsProgress| {
            let _ = app.emit("ops:progress", &p);
        })
    })
    .await
    .map_err(|e| format!("اجرای پلن استخراج ناموفق بود: {e}"))
}

/// اجرای استخراج در پس‌زمینه؛ تعداد درخواست‌های صف را برمی‌گرداند.
#[tauri::command]
pub fn execute_archive_extraction(
    app: AppHandle,
    state: State<'_, AppState>,
    requests: Vec<ArchiveExecuteRequest>,
) -> Result<u64, String> {
    if requests.is_empty() {
        return Err("فهرست اجرای استخراج خالی است.".into());
    }
    let seven_zip = resolve_tool(&state, "sevenZip")?;

    // رزرو اتمیک اسلات عملیات گروهی — همان اسلات دانلود/استخراج داخلی.
    let guard = acquire_batch_op(&state.ops)?;
    let cancel = guard.cancel.clone();
    let total = requests.len() as u64;
    crate::logging::info(
        "archive",
        &format!("استخراج آرشیو برای {total} ویدیو شروع شد"),
    );

    let app2 = app.clone();
    std::thread::spawn(move || {
        let result =
            run_archive_extraction(&seven_zip, &requests, &cancel, &mut |p: OpsProgress| {
                let _ = app2.emit("ops:progress", &p);
            });
        drop(guard); // آزادسازی اسلات در همهٔ مسیرها (RAII)

        crate::logging::info(
            "archive",
            &format!(
                "استخراج آرشیو تمام شد: موفق={}، لغو={}، جای‌گذاری={}، خطا={}، ابهام={}، بدون‌تطبیق={} ({:.1}s) — {}",
                result.success,
                result.cancelled,
                result.placed,
                result.failed,
                result.ambiguous,
                result.no_match,
                result.duration_secs,
                result.message
            ),
        );
        let _ = app2.emit(ARCHIVE_DONE_EVENT, archive_done_payload(&result));
        let _ = app2.emit(
            "ops:done",
            &OpsProgress {
                kind: "archive".into(),
                current: result.placed as u64,
                total,
                video_path: String::new(),
                message: result.message,
            },
        );
    });

    Ok(total)
}

/// وضعیت عملیات استخراج/دانلود گروهی برای UI.
#[tauri::command]
pub fn is_archive_active(state: State<'_, AppState>) -> bool {
    state.ops.lock().unwrap().batch_cancel.is_some()
}
