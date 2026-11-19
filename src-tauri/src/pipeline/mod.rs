//! Pipeline — ارکستراسیون انتها-به-انتها (Task 4).
//!
//! مالکیت lifecycle:
//! - `start_pipeline`: admission با `acquire_batch_op` → ساخت `PipelineControl`
//!   از **همان Arc پرچم گارد** → publish → انتقال گارد با `GuardHandoff` → spawn.
//! - worker با `AppHandle` جابه‌جا می‌شود و `state` را داخل thread می‌گیرد.
//! - کل lifecycle داخل panic boundary است؛ پاک‌سازی با `WorkerCleanup` (RAII)
//!   با ترتیب قطعی: حذف control با چک run_id → رهایی اسلات.
//! - انتظار تصمیم‌ها: Mutex+Condvar روی `control.inner` — بدون polling.

pub mod engine;
pub mod types;

pub use types::{
    DecisionKind, DecisionOptionPayload, ItemArtifacts, ItemOutcome, PendingDecision,
    PipelineCounts, PipelineEvent, PipelineInput, PipelineItemSnapshot, PipelineSnapshot,
    PipelineStage, PipelineStatus, PipelineSummary, RegisteredOption, StageProgress, StageStatus,
};

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use crate::state::{AppState, BatchOpGuard};

/// دست دادن مالکیت گارد — guard تا وقتی worker نگرفته اینجاست.
pub(crate) type GuardHandoff = Arc<Mutex<Option<BatchOpGuard>>>;

/// sink رویدادها — Send برای مصرف در worker pool سینک.
pub(crate) type SinkStore = Arc<Mutex<Box<dyn FnMut(PipelineEvent) + Send>>>;

pub(crate) fn emit_to(sink: &SinkStore, event: PipelineEvent) {
    let mut sink = sink.lock().unwrap_or_else(PoisonError::into_inner);
    sink(event);
}

/// وضعیت درونی control — زیر `inner` Mutex محافظت می‌شود؛ predicateهای
/// Condvar (لغو + تصمیم) زیر همین قفل ارزیابی می‌شوند.
pub(crate) struct ControlState {
    pub stage: PipelineStage,
    pub status: PipelineStatus,
    pub items: Vec<PipelineItemSnapshot>,
    pub pending: Option<PendingDecisionState>,
    pub counts: PipelineCounts,
    pub current_file: Option<String>,
    pub active: usize,
    pub cancelling: bool,
    pub message: String,
    /// پیشرفت مرحلهٔ جاری — با هر set_stage صفر می‌شود؛ فقط شمارش واقعی.
    pub stage_progress: Option<StageProgress>,
    pub invalid_selections: Vec<crate::scanner::SelectionIssue>,
    pub staged_paths: Vec<String>,
    pub warnings: Vec<String>,
    pub key_store_error: Option<String>,
}

pub(crate) struct PendingDecisionState {
    pub decision: PendingDecision,
    pub resolved: Option<RegisteredOption>,
}

/// کنترل run — `cancel` دقیقاً همان Arc پرچم `BatchOpGuard` است (نه کپی جدا).
pub struct PipelineControl {
    run_id: String,
    cancel: Arc<AtomicBool>,
    /// شمارهٔ یکنوای snapshot — جلوگیری از overwrite شدن وضعیت جدید با رویداد کهنه.
    seq: AtomicU64,
    started: Instant,
    inner: Mutex<ControlState>,
    gate: Condvar,
}

use std::time::Instant;

impl PipelineControl {
    pub(crate) fn from_guard(
        guard: &BatchOpGuard,
        run_id: String,
        message: impl Into<String>,
    ) -> Self {
        PipelineControl {
            run_id,
            cancel: Arc::clone(&guard.cancel),
            seq: AtomicU64::new(0),
            started: Instant::now(),
            inner: Mutex::new(ControlState {
                stage: PipelineStage::Validating,
                status: PipelineStatus::Running,
                items: Vec::new(),
                pending: None,
                counts: PipelineCounts::default(),
                current_file: None,
                active: 0,
                cancelling: false,
                message: message.into(),
                stage_progress: None,
                invalid_selections: Vec::new(),
                staged_paths: Vec::new(),
                warnings: Vec::new(),
                key_store_error: None,
            }),
            gate: Condvar::new(),
        }
    }

    pub(crate) fn run_id(&self) -> &str {
        &self.run_id
    }

    /// پرچم لغو — همان Arc پرچم BatchOpGuard؛ هسته‌های فعال از همان poll می‌کنند.
    pub(crate) fn cancel_flag(&self) -> &Arc<AtomicBool> {
        &self.cancel
    }

    /// وضعیت کلی run — فقط در مسیرهای نهایی/بازگشت از تصمیم.
    pub(crate) fn set_status(&self, status: PipelineStatus) {
        self.lock_inner().status = status;
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    pub(crate) fn lock_inner(&self) -> MutexGuard<'_, ControlState> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// snapshot کامل — برای command و رویدادها. completed فقط از outcomeهای نهایی.
    pub(crate) fn snapshot(&self) -> PipelineSnapshot {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let mut g = self.lock_inner();
        g.counts.decisions_pending = if g.pending.is_some() { 1 } else { 0 };
        let completed = g.items.iter().filter(|i| i.outcome.is_some()).count();
        PipelineSnapshot {
            seq,
            run_id: self.run_id.clone(),
            stage: g.stage,
            status: g.status,
            total: g.items.len(),
            completed,
            active: g.active,
            cancelling: g.cancelling,
            current_file: g.current_file.clone(),
            counts: g.counts,
            stage_progress: g.stage_progress,
            pending_decision: g.pending.as_ref().map(|p| p.decision.clone()),
            message: g.message.clone(),
        }
    }

    /// snapshot آیتم‌های ثبت‌شده — برای بازیابی UI پس از remount/reload.
    pub(crate) fn item_snapshots(&self) -> Vec<PipelineItemSnapshot> {
        self.lock_inner().items.clone()
    }

    pub(crate) fn set_active(&self, active: usize) {
        self.lock_inner().active = active;
    }

    /// ثبت درخواست توقف — وضعیت پایانی همچنان از مسیر worker می‌آید.
    pub(crate) fn set_cancelling(&self, cancelling: bool) {
        self.lock_inner().cancelling = cancelling;
    }

    pub(crate) fn set_stage(&self, stage: PipelineStage) {
        let mut g = self.lock_inner();
        g.stage = stage;
        // شمارندهٔ مرحلهٔ قبلی به مرحلهٔ جدید سرایت نمی‌کند
        g.stage_progress = None;
    }

    /// ثبت پیشرفت مرحلهٔ جاری — فقط شمارش واقعی آیتم‌های تمام‌شده؛
    /// هر مرحله ابتدا total را اعلام می‌کند و پس از هر آیتم completed را بالا می‌برد.
    pub(crate) fn set_stage_progress(&self, completed: u32, total: u32) {
        self.lock_inner().stage_progress = Some(StageProgress { completed, total });
    }

    /// پاک‌سازی شمارنده — وقتی شمارش خطی مرحله دیگر معنا ندارد.
    pub(crate) fn clear_stage_progress(&self) {
        self.lock_inner().stage_progress = None;
    }

    pub(crate) fn set_message(&self, message: impl Into<String>) {
        self.lock_inner().message = message.into();
    }

    pub(crate) fn set_counts(&self, counts: PipelineCounts) {
        self.lock_inner().counts = counts;
    }

    pub(crate) fn set_current_file(&self, path: Option<String>) {
        self.lock_inner().current_file = path;
    }

    pub(crate) fn set_invalid_selections(&self, issues: Vec<crate::scanner::SelectionIssue>) {
        self.lock_inner().invalid_selections = issues;
    }

    pub(crate) fn set_staged_paths(&self, paths: Vec<String>) {
        self.lock_inner().staged_paths = paths;
    }

    pub(crate) fn push_warning(&self, warning: impl Into<String>) {
        self.lock_inner().warnings.push(warning.into());
    }

    pub(crate) fn set_key_store_error(&self, reason: Option<String>) {
        self.lock_inner().key_store_error = reason;
    }

    /// جایگزینی/افزودن snapshot آیتم بر اساس مسیر ویدیو.
    pub(crate) fn put_item_snapshot(&self, snapshot: PipelineItemSnapshot) {
        let mut g = self.lock_inner();
        match g
            .items
            .iter_mut()
            .find(|i| i.video_path == snapshot.video_path)
        {
            Some(existing) => *existing = snapshot,
            None => g.items.push(snapshot),
        }
    }

    /// ثبت تصمیم باز با گزینه‌های backend-owned (شناسه‌های مات opt-N).
    /// زیر inner lock — خروجی: تصمیم کامل برای emit.
    pub(crate) fn register_decision(
        &self,
        decision_id: String,
        kind: DecisionKind,
        video_path: &str,
        video_file_name: &str,
        payloads: Vec<DecisionOptionPayload>,
    ) -> PendingDecision {
        let mut g = self.lock_inner();
        let options = payloads
            .into_iter()
            .enumerate()
            .map(|(i, payload)| RegisteredOption {
                option_id: format!("{decision_id}-opt-{i}"),
                payload,
            })
            .collect();
        let decision = PendingDecision {
            decision_id,
            kind,
            video_path: video_path.to_string(),
            video_file_name: video_file_name.to_string(),
            options,
        };
        g.pending = Some(PendingDecisionState {
            decision: decision.clone(),
            resolved: None,
        });
        g.status = PipelineStatus::AwaitingDecision;
        decision
    }

    /// پاک‌سازی تصمیم پس از خواندن انتخاب — زیر inner.
    pub(crate) fn clear_pending_decision(&self, decision_id: &str) {
        let mut g = self.lock_inner();
        if g.pending
            .as_ref()
            .map(|p| p.decision.decision_id == decision_id)
            .unwrap_or(false)
        {
            g.pending = None;
            g.status = PipelineStatus::Running;
        }
    }

    /// علامت‌گذاری control به‌عنوان Failed بدون حذف آن (مسیر fail-closed
    /// empty-handoff — حذف control فقط توسط مالک واقعی انجام می‌شود).
    pub(crate) fn mark_failed(&self, message: impl Into<String>) {
        let mut g = self.lock_inner();
        g.status = PipelineStatus::Failed;
        g.message = message.into();
    }

    /// انتظار تصمیم — حلقهٔ صریح با predicate زیر inner؛ wake روی resolve/لغو.
    /// قفل مسموم با recovery ادامه می‌یابد (بدون panic، بدون گم‌شدن لغو).
    pub(crate) fn wait_for_decision(&self, decision_id: &str) -> WaitExit {
        let mut g = self.lock_inner();
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                return WaitExit::Cancelled;
            }
            match &g.pending {
                Some(pending) if pending.decision.decision_id == decision_id => {
                    if let Some(resolved) = &pending.resolved {
                        return WaitExit::Resolved(resolved.payload.clone());
                    }
                }
                _ => return WaitExit::Cancelled,
            }
            g = match self.gate.wait(g) {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
    }
}

#[derive(Debug)]
pub(crate) enum WaitExit {
    Cancelled,
    Resolved(DecisionOptionPayload),
}

/// حذف فقط control با run_id مطابق — poison-safe؛ خروجی: آیا حذف شد؟
pub(crate) fn clear_published_control(state: &AppState, run_id: &str) -> bool {
    let mut published = state
        .pipeline
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if published
        .as_ref()
        .map(|c| c.run_id() == run_id)
        .unwrap_or(false)
    {
        *published = None;
        true
    } else {
        false
    }
}

/// RAII پایان lifecycle worker — ترتیب قطعی در Drop:
/// ۱) حذف فقط controlِ هم-run_id (poison-safe) ۲) رهایی اسلات (release batch guard).
pub(crate) struct WorkerCleanup<'a> {
    state: &'a AppState,
    run_id: String,
    guard: Option<BatchOpGuard>,
}

impl<'a> WorkerCleanup<'a> {
    pub(crate) fn new(state: &'a AppState, run_id: String, guard: BatchOpGuard) -> Self {
        WorkerCleanup {
            state,
            run_id,
            guard: Some(guard),
        }
    }
}

impl Drop for WorkerCleanup<'_> {
    fn drop(&mut self) {
        // ۱) حذف control فقط با run_id مطابق — هیچ panic ای از این مسیر نمی‌آید
        let _ = clear_published_control(self.state, &self.run_id);
        // ۲) رهایی اسلات بعد از حذف control — BatchOpGuard::drop با poison recovery
        self.guard = None;
    }
}

/// انتقال دقیقاً یک‌بارهٔ گارد؛ خالی بودن handoff = خطای داخلی صریح.
pub(crate) fn take_guard_once(handoff: &GuardHandoff) -> Result<BatchOpGuard, String> {
    handoff
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .ok_or_else(|| "مالکیت batch guard قبلاً منتقل شده است (خطای داخلی handoff)".into())
}

/// ورود worker — گارد را از handoff می‌گیرد؛ خالی بودن handoff = fail-closed:
/// control پاک نمی‌شود (مالک واقعی نامعلوم)؛ فقط Failed علامت می‌خورد.
pub(crate) fn enter_worker(
    state: &AppState,
    run_id: String,
    handoff: &GuardHandoff,
    control: Arc<PipelineControl>,
    input: PipelineInput,
    sink: SinkStore,
) {
    let guard = match take_guard_once(handoff) {
        Ok(guard) => guard,
        Err(err) => {
            crate::logging::error(
                "pipeline",
                &format!("[{run_id}] انتقال batch guard ناموفق بود: {err} — control به‌صورت Failed علامت می‌خورد"),
            );
            control.mark_failed(format!("خطای داخلی انتقال guard: {err}"));
            return;
        }
    };
    let cleanup = WorkerCleanup::new(state, run_id, guard);
    let io = engine::PipelineIo::production();
    engine::run_worker(state, &control, cleanup, input, &sink, &io);
}

/// لغو pipeline فعال — فقط control منتشرشده را هدف می‌گیرد؛ هرگز
/// `cancel_all` صدا نمی‌زند و عملیات‌های بی‌ربط را لمس نمی‌کند.
pub fn cancel_active_pipeline(state: &AppState) -> Result<bool, String> {
    let control = {
        let published = state
            .pipeline
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match published.as_ref() {
            Some(control) => Arc::clone(control),
            None => return Ok(false),
        }
    };
    // ۱) ست پرچم (همان Arc پرچم batch guard — هسته‌های فعال از همان poll می‌کنند)
    control.cancel.store(true, Ordering::SeqCst);
    // ۲) بیدارسازی waiterها — notify زیر inner (پروتکل بدون lost wakeup)
    let mut g = control.lock_inner();
    g.cancelling = true;
    control.gate.notify_all();
    Ok(true)
}

/// بیدارسازی waiterهای pipeline پس از هر مسیری که پرچم batch را ست کرده
/// (cancel_subtitle_ops) — notify-only؛ predicate در wait بازبینی می‌شود.
pub fn notify_cancelled_waiter(state: &AppState) {
    let published = state
        .pipeline
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(control) = published.as_ref() {
        let _guard = control.lock_inner();
        control.gate.notify_all();
    }
}

/// هستهٔ resolve تصمیم — اعتبارسنجی گزینه زیر inner + notify با همان guard.
pub fn resolve_decision_core(
    state: &AppState,
    decision_id: &str,
    option_id: &str,
) -> Result<DecisionOptionPayload, String> {
    let control = {
        let published = state
            .pipeline
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match published.as_ref() {
            Some(control) => Arc::clone(control),
            None => return Err("pipeline فعالی وجود ندارد".into()),
        }
    };
    let mut g = control.lock_inner();
    let Some(pending) = g.pending.as_mut() else {
        return Err("هیچ تصمیم بازی در جریان نیست — snapshot را دوباره بگیرید.".into());
    };
    if pending.decision.decision_id != decision_id {
        return Err("این تصمیم دیگر باز نیست — snapshot را دوباره بگیرید.".into());
    }
    let Some(option) = pending
        .decision
        .options
        .iter()
        .find(|o| o.option_id == option_id)
    else {
        return Err("این گزینه معتبر نیست — یکی از گزینه‌های ثبت‌شده را انتخاب کنید.".into());
    };
    let payload = option.payload.clone();
    pending.resolved = Some(option.clone());
    control.gate.notify_all();
    Ok(payload)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod e2e;

#[cfg(test)]
mod real_world;
