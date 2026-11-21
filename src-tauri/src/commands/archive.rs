//! دستورات Tauri برای استخراج آرشیو و تطبیق قسمت (فاز ۳ PR ۳).
//!
//! پلن و اجرا هر دو ابتدا provenance ارسالی فرانت را در برابر کتابخانهٔ
//! اسکن‌شده و رجیستری Provenance سمت سرور اعتبارسنجی می‌کنند. سپس پلن با
//! `spawn_blocking` و اجرا در یک thread پس‌زمینه با گارد RAII در `OpsRegistry`
//! انجام می‌شود. پیشرفت فقط بر اساس مراحل واقعی (تعداد منابع پردازش‌شده)
//! گزارش می‌شود.

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, State};

use crate::archive::engine::{execute_archive_extraction as run_archive_extraction, plan_archives};
use crate::archive::provenance::validate_archive_sources;
use crate::archive::types::{
    ArchiveExecuteRequest, ArchiveExecuteResult, ArchiveExtractionPlan, ArchiveSource,
};
use crate::models::OpsProgress;
use crate::state::{acquire_batch_op, AppState, OpsRegistry};

use super::media::resolve_tool;

/// رویداد پایان استخراج — payload کامل و ساختاریافته.
pub const ARCHIVE_DONE_EVENT: &str = "archive:done";

pub fn archive_done_payload(result: &ArchiveExecuteResult) -> serde_json::Value {
    serde_json::to_value(result).unwrap_or(serde_json::Value::Null)
}

/// اعتبارسنجی منابع با استفاده از کتابخانهٔ اسکن‌شده و رجیستری Provenance.
/// `season`/`episode` خروجی همیشه از `VideoEntry` معتبر می‌آید.
fn validate_requested_sources(
    state: &AppState,
    sources: &[ArchiveSource],
) -> Result<Vec<ArchiveSource>, String> {
    let library = state.library.lock().unwrap().clone();
    let registry = state.archive_provenance.lock().unwrap();
    validate_archive_sources(&registry, &library, sources)
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

    // ۱) منبع فقط وقتی معتبر است که خود بک‌اند آن را ساخته باشد.
    let trusted = validate_requested_sources(&state, &sources)?;
    let seven_zip = resolve_tool(&state, "sevenZip")?;

    // ۲) پلن هم یک عملیات گروهی است: همان اسلات/پرچم لغو دانلود و استخراج.
    let guard = acquire_batch_op(&state.ops)?;
    let cancel = guard.cancel.clone();
    let total = trusted.len() as u64;
    let app2 = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let plan = plan_archives(&seven_zip, &trusted, &cancel, &mut |p: OpsProgress| {
            let _ = app2.emit("ops:progress", &p);
        });
        // آزادسازی صریح پیش از اعلام پایان؛ Drop هم در panic این کار را می‌کند.
        drop(guard);

        let message = if plan.cancelled {
            "پلن استخراج لغو شد؛ پوشه‌های موقت پاک شدند.".to_string()
        } else {
            format!(
                "پلن استخراج: {} آماده، {} ابهام، {} بدون تطبیق.",
                plan.ready, plan.ambiguous, plan.no_match
            )
        };
        let _ = app2.emit(
            "ops:done",
            &OpsProgress {
                kind: "archive".into(),
                current: plan.items.len() as u64,
                total,
                video_path: String::new(),
                message,
            },
        );
        plan
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

    // ۱) هر منبع باید رکورد Provenance سمت سرور و ویدیوی معتبر در کتابخانه
    // داشته باشد. انتخاب کاربر فقط برای رفع ابهام است، نه منبع حقیقت.
    let submitted: Vec<ArchiveSource> = requests.iter().map(|r| r.source.clone()).collect();
    let trusted = validate_requested_sources(&state, &submitted)?;
    let requests: Vec<ArchiveExecuteRequest> = requests
        .into_iter()
        .zip(trusted)
        .map(|(request, source)| ArchiveExecuteRequest {
            source,
            selection: request.selection,
        })
        .collect();

    let seven_zip = resolve_tool(&state, "sevenZip")?;

    // ۲) رزرو اتمیک اسلات عملیات گروهی — همان اسلات دانلود/استخراج داخلی.
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
    archive_ops_active(&state.ops)
}

/// آیا یک عملیات گروهی (دانلود، پلن آرشیو یا اجرای آرشیو) در جریان است؟
pub fn archive_ops_active(ops: &Arc<Mutex<OpsRegistry>>) -> bool {
    // poison-safe: قفل مسموم با recovery بازیابی می‌شود
    match ops.lock() {
        Ok(ops) => ops.has_active_batch(),
        Err(poisoned) => poisoned.into_inner().has_active_batch(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::acquire_single_op;
    use std::sync::atomic::Ordering;

    #[test]
    fn archive_batch_slot_rejects_conflicts_and_cancels_through_registry() {
        let ops = Arc::new(Mutex::new(OpsRegistry::default()));
        assert!(!archive_ops_active(&ops));

        let guard = acquire_batch_op(&ops).unwrap();
        assert!(archive_ops_active(&ops), "پلن باید به‌عنوان فعالیت دیده شود");
        // تسک گروهی دوم و سوم اکنون مجازند (تا سقف ۳ هم‌زمان)؛ چهارم رد می‌شود.
        let g2 = acquire_batch_op(&ops).expect("دوم");
        let g3 = acquire_batch_op(&ops).expect("سوم");
        assert!(
            acquire_batch_op(&ops).is_err(),
            "چهارم باید با سقف ۳ رد شود"
        );
        // عملیات تکی هم حین هر تسک گروهی مجاز نیست.
        assert!(acquire_single_op(&ops, "video").is_err());

        // cancel_subtitle_ops از همین Registry استفاده می‌کند.
        assert!(ops.lock().unwrap().cancel_all());
        assert!(guard.cancel.load(Ordering::SeqCst));
        assert!(g2.cancel.load(Ordering::SeqCst));
        assert!(g3.cancel.load(Ordering::SeqCst));

        // آزادسازی یکی → جا برای تسک بعدی با پرچم تازه
        drop(g2);
        let next_slot = acquire_batch_op(&ops).unwrap();
        assert!(!next_slot.cancel.load(Ordering::SeqCst));
        drop(next_slot);
        drop(g3);
        drop(guard);
        assert!(!archive_ops_active(&ops));
        let next = acquire_batch_op(&ops).unwrap();
        assert!(!next.cancel.load(Ordering::SeqCst));
    }
}
