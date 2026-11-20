//! هاردساب — کنترل اجرا و موتور worker.
//!
//! مالکیت lifecycle (قرینهٔ pipeline):
//! - `start_hardsub` (command): admission با `acquire_batch_op` → ساخت
//!   `HardsubControl` از **همان Arc پرچم گارد** → publish → انتقال گارد با
//!   handoff → spawn.
//! - کل lifecycle داخل thread است؛ پاک‌سازی با `Cleanup` (RAII) با ترتیب
//!   قطعی: حذف control با چک run_id → رهایی اسلات.
//! - رویدادها: ticker با گام 400ms و `hardsub:state` + یک `hardsub:done`.
//! - استخر worker موازی از صف `VecDeque` — لغو، آیتم‌های صف را Cancelled می‌کند.

pub mod burn;
pub mod encoder;
pub mod plan;
pub mod types;

#[cfg(test)]
mod real_world;

pub use types::{
    HardsubCounts, HardsubEvent, HardsubItemSnapshot, HardsubItemStatus, HardsubPlan,
    HardsubPlanItem, HardsubPlanSkip, HardsubSnapshot, HardsubSummary,
};

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::media::ffprobe::ProbeError;
use crate::state::{AppState, BatchOpGuard};

use burn::BurnError;

/// sink رویدادها — Send برای مصرف در thread pool.
pub type SinkStore = Arc<Mutex<Box<dyn FnMut(HardsubEvent) + Send>>>;

pub(crate) fn emit_to(sink: &SinkStore, event: HardsubEvent) {
    let mut sink = sink.lock().unwrap_or_else(PoisonError::into_inner);
    sink(event);
}

/// وضعیت درونی اجرا — همه به‌روزرسانی‌ها زیر `inner` Mutex.
pub(crate) struct ControlState {
    pub items: Vec<HardsubItemSnapshot>,
    /// درصد زندهٔ هر آیتم (هم‌index با items) — فقط برای Encoding پر می‌شود.
    pub slots: Vec<Option<Arc<Mutex<Option<u32>>>>>,
    pub counts: HardsubCounts,
    pub running: bool,
    pub cancelling: bool,
}

/// کنترل اجرا — `cancel` دقیقاً همان Arc پرچم `BatchOpGuard` است.
pub struct HardsubControl {
    run_id: String,
    cancel: Arc<AtomicBool>,
    seq: AtomicU64,
    started: Instant,
    inner: Mutex<ControlState>,
}

impl HardsubControl {
    pub(crate) fn from_guard(guard: &BatchOpGuard, run_id: String) -> Self {
        HardsubControl {
            run_id,
            cancel: Arc::clone(&guard.cancel),
            seq: AtomicU64::new(0),
            started: Instant::now(),
            inner: Mutex::new(ControlState {
                items: Vec::new(),
                slots: Vec::new(),
                counts: HardsubCounts::default(),
                running: true,
                cancelling: false,
            }),
        }
    }

    pub(crate) fn run_id(&self) -> &str {
        &self.run_id
    }

    /// پرچم لغو — همان Arc پرچم BatchOpGuard؛ هسته‌های فعال از همان poll می‌کنند.
    pub(crate) fn cancel_flag(&self) -> &Arc<AtomicBool> {
        &self.cancel
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, ControlState> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// snapshot کامل — درصدِ زندهٔ Encodingها از slots ادغام می‌شود.
    pub fn snapshot(&self) -> HardsubSnapshot {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let mut g = self.lock();
        let percents: Vec<Option<u32>> = g
            .slots
            .iter()
            .map(|slot| {
                slot.as_ref()
                    .and_then(|slot| *slot.lock().unwrap_or_else(PoisonError::into_inner))
            })
            .collect();
        for (index, item) in g.items.iter_mut().enumerate() {
            if matches!(item.status, HardsubItemStatus::Encoding { .. }) {
                item.status = HardsubItemStatus::Encoding {
                    percent: percents.get(index).cloned().flatten(),
                };
            }
        }
        let completed = g
            .items
            .iter()
            .filter(|item| item.status.is_terminal())
            .count();
        HardsubSnapshot {
            seq,
            run_id: self.run_id.clone(),
            running: g.running,
            cancelling: g.cancelling,
            total: g.items.len(),
            completed,
            items: g.items.clone(),
            counts: g.counts,
        }
    }

    /// به‌روزرسانی یک آیتم + بازشمارش صادقانهٔ counters — زیر همان قفل.
    pub(crate) fn update_item(&self, index: usize, update: impl FnOnce(&mut HardsubItemSnapshot)) {
        let mut g = self.lock();
        if let Some(item) = g.items.get_mut(index) {
            update(item);
            g.counts = counts_of(&g.items);
        }
    }

    pub(crate) fn set_slot(&self, index: usize, slot: Option<Arc<Mutex<Option<u32>>>>) {
        let mut g = self.lock();
        if let Some(entry) = g.slots.get_mut(index) {
            *entry = slot;
        }
    }

    pub(crate) fn set_cancelling(&self, cancelling: bool) {
        self.lock().cancelling = cancelling;
    }

    /// مسیر fail-closed handoff — فقط وضعیت می‌بندد؛ حذف control توسط
    /// مالک واقعی (Cleanup) انجام می‌شود.
    pub(crate) fn mark_failed(&self, message: impl Into<String>) {
        let message = message.into();
        let mut g = self.lock();
        g.running = false;
        crate::logging::error("hardsub", &message);
    }

    /// خلاصهٔ نهایی — منبع حقیقت پس از پایان.
    pub(crate) fn summary(&self, cancelled: bool) -> HardsubSummary {
        let g = self.lock();
        HardsubSummary {
            run_id: self.run_id.clone(),
            cancelled,
            counts: g.counts,
            items: g.items.clone(),
            duration_ms: self.started.elapsed().as_millis() as u64,
        }
    }
}

/// شمارنده‌ها فقط از شمارش واقعی وضعیت‌های پایانی — هیچ تخمینی.
fn counts_of(items: &[HardsubItemSnapshot]) -> HardsubCounts {
    let mut counts = HardsubCounts::default();
    for item in items {
        match &item.status {
            HardsubItemStatus::Done { .. } => counts.done += 1,
            HardsubItemStatus::Failed { .. } => counts.failed += 1,
            HardsubItemStatus::Conflict => counts.conflicts += 1,
            HardsubItemStatus::Cancelled => counts.cancelled += 1,
            _ => {}
        }
    }
    counts
}

/// حذف فقط control با run_id مطابق — poison-safe؛ خروجی: آیا حذف شد؟
pub(crate) fn clear_published_control(state: &AppState, run_id: &str) -> bool {
    let mut published = state.hardsub.lock().unwrap_or_else(PoisonError::into_inner);
    if published
        .as_ref()
        .map(|control| control.run_id() == run_id)
        .unwrap_or(false)
    {
        *published = None;
        true
    } else {
        false
    }
}

/// RAII پایان lifecycle worker — ترتیب قطعی در Drop:
/// ۱) حذف فقط controlِ هم-run_id ۲) رهایی اسلات (release batch guard).
pub(crate) struct Cleanup<'a> {
    state: &'a AppState,
    run_id: String,
    guard: Option<BatchOpGuard>,
}

impl<'a> Cleanup<'a> {
    pub(crate) fn new(state: &'a AppState, run_id: String, guard: BatchOpGuard) -> Self {
        Cleanup {
            state,
            run_id,
            guard: Some(guard),
        }
    }
}

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let _ = clear_published_control(self.state, &self.run_id);
        // رهایی اسلات بعد از حذف control — BatchOpGuard::drop با poison recovery
        self.guard = None;
    }
}

/// مسیرهای ابزار موتور — یک‌بار در command resolve می‌شوند.
#[derive(Clone)]
pub(crate) struct EnginePaths {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

/// پیکربندی موتور از تنظیمات کاربر.
#[derive(Clone, Copy)]
pub(crate) struct EngineConfig {
    pub encoder: encoder::VideoEncoder,
    pub quality: crate::settings::HardsubQuality,
    pub workers: usize,
}

/// ورود worker — گارد را از handoff می‌گیرد؛ خالی بودن handoff = fail-closed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn enter_worker(
    state: &AppState,
    run_id: String,
    handoff: &crate::pipeline::GuardHandoff,
    control: Arc<HardsubControl>,
    paths: EnginePaths,
    config: EngineConfig,
    plan: HardsubPlan,
    sink: SinkStore,
) {
    let guard = match crate::pipeline::take_guard_once(handoff) {
        Ok(guard) => guard,
        Err(err) => {
            control.mark_failed(format!(
                "انتقال batch guard ناموفق بود: {err} — control به‌صورت Failed علامت می‌خورد"
            ));
            return;
        }
    };
    let cleanup = Cleanup::new(state, run_id, guard);
    run_engine(&control, &paths, config, &plan, &sink);
    // cleanup در پایان scope به ترتیب قطعی Drop می‌شود
    drop(cleanup);
}

/// موتور اصلی — تیکر رویدادها + استخر worker + sweep پایانی + Done.
fn run_engine(
    control: &Arc<HardsubControl>,
    paths: &EnginePaths,
    config: EngineConfig,
    plan: &HardsubPlan,
    sink: &SinkStore,
) {
    let run_id = control.run_id().to_string();
    crate::logging::info(
        "hardsub",
        &format!(
            "[{run_id}] هاردساب {} ویدیو با انکودر {} ({} worker) شروع شد",
            plan.items.len(),
            plan.encoder,
            config.workers
        ),
    );

    // مقداردهی اولیهٔ آیتم‌ها و slots — هم‌index
    {
        let mut g = control.lock();
        g.items = plan
            .items
            .iter()
            .map(|item| HardsubItemSnapshot {
                video_path: item.video_path.clone(),
                video_file_name: item.video_file_name.clone(),
                status: HardsubItemStatus::Pending,
                encoder: None,
            })
            .collect();
        g.slots = vec![None; plan.items.len()];
        g.counts = HardsubCounts::default();
    }
    emit_to(
        sink,
        HardsubEvent::State {
            snapshot: control.snapshot(),
        },
    );

    // ticker — emits هر 400ms تا running=false؛ درصد زنده از slots ادغام می‌شود
    let ticker_control = Arc::clone(control);
    let ticker_sink = Arc::clone(sink);
    let ticker = std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(400));
        {
            let g = ticker_control.lock();
            if !g.running {
                break;
            }
        }
        emit_to(
            &ticker_sink,
            HardsubEvent::State {
                snapshot: ticker_control.snapshot(),
            },
        );
    });

    let cancel = Arc::clone(control.cancel_flag());
    let queue: Arc<Mutex<VecDeque<usize>>> =
        Arc::new(Mutex::new(VecDeque::from_iter(0..plan.items.len())));
    let workers = config.workers.min(plan.items.len()).max(1);
    let mut handles = Vec::with_capacity(workers);
    for worker_index in 0..workers {
        let queue = Arc::clone(&queue);
        let control = Arc::clone(control);
        let paths = paths.clone();
        let items = plan.items.clone();
        let worker_cancel = Arc::clone(&cancel);
        let spawned = std::thread::Builder::new()
            .name(format!("hardsub-w{worker_index}"))
            .spawn(move || loop {
                let index = {
                    queue
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .pop_front()
                };
                let Some(index) = index else { break };
                if worker_cancel.load(Ordering::SeqCst) {
                    control.update_item(index, |item| item.status = HardsubItemStatus::Cancelled);
                    continue;
                }
                let Some(plan_item) = items.get(index) else {
                    continue;
                };
                process_item(&control, &paths, config, plan_item, index, &worker_cancel);
            });
        match spawned {
            Ok(handle) => handles.push(handle),
            Err(error) => {
                // fail-closed: worker وجود ندارد؛ آیتم‌هایش در sweep نهایی Failed می‌شوند
                crate::logging::warn(
                    "hardsub",
                    &format!("ساخت worker {worker_index} ناموفق بود: {error}"),
                );
            }
        }
    }
    for handle in handles {
        let _ = handle.join();
    }

    let cancelled = control.is_cancelled();
    // sweep آیتم‌های پردازش‌نشده — فقط وضعیت‌های پایانی می‌مانند
    {
        let mut g = control.lock();
        for item in &mut g.items {
            if !item.status.is_terminal() {
                item.status = if cancelled {
                    HardsubItemStatus::Cancelled
                } else {
                    HardsubItemStatus::Failed {
                        reason: "این مورد پردازش نشد".into(),
                    }
                };
            }
        }
        g.counts = counts_of(&g.items);
        g.running = false;
    }

    let _ = ticker.join();
    emit_to(
        sink,
        HardsubEvent::State {
            snapshot: control.snapshot(),
        },
    );
    emit_to(
        sink,
        HardsubEvent::Done {
            summary: control.summary(cancelled),
        },
    );
    crate::logging::info(
        "hardsub",
        &format!(
            "[{run_id}] هاردساب پایان یافت — موفق: {}، شکست: {}، تعارض: {}، لغو: {}",
            control.lock().counts.done,
            control.lock().counts.failed,
            control.lock().counts.conflicts,
            control.lock().counts.cancelled,
        ),
    );
}

/// پردازش یک آیتم: مدت → burn (با fallback CPU) → وضعیت نهایی.
fn process_item(
    control: &Arc<HardsubControl>,
    paths: &EnginePaths,
    config: EngineConfig,
    plan_item: &HardsubPlanItem,
    index: usize,
    cancel: &AtomicBool,
) {
    let video = PathBuf::from(&plan_item.video_path);
    let output = PathBuf::from(&plan_item.output_path);
    let initial_status = if plan_item.embedded_stream_index.is_some() {
        HardsubItemStatus::Extracting
    } else {
        HardsubItemStatus::Encoding { percent: None }
    };
    control.update_item(index, |item| {
        item.status = initial_status;
        item.encoder = Some(config.encoder.label().to_string());
    });

    // مدت ویدیو — برای timeout و درصد واقعی؛ شکست پروب = شکست صادقانهٔ آیتم
    let duration = match crate::media::ffprobe::probe_duration(&paths.ffprobe, &video, cancel) {
        Ok(value) => value,
        Err(ProbeError::Cancelled) => {
            control.update_item(index, |item| item.status = HardsubItemStatus::Cancelled);
            return;
        }
        Err(ProbeError::Failed(reason)) => {
            control.update_item(index, |item| {
                item.status = HardsubItemStatus::Failed {
                    reason: format!("پروب مدت ویدیو ناموفق بود: {reason}"),
                };
            });
            return;
        }
    };

    let slot: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
    control.set_slot(index, Some(Arc::clone(&slot)));

    let source = match &plan_item.subtitle_path {
        Some(path) => plan::SubtitleSource::File(PathBuf::from(path)),
        None => plan::SubtitleSource::EmbeddedStream {
            stream_index: plan_item.embedded_stream_index.unwrap_or_default(),
        },
    };

    let do_burn = |encoder: encoder::VideoEncoder| {
        let request = burn::BurnRequest {
            ffmpeg: &paths.ffmpeg,
            ffprobe: &paths.ffprobe,
            video: &video,
            output: &output,
            source: source.clone(),
            encoder,
            quality: config.quality,
            duration_seconds: duration,
            timeout_override: None,
            cancel,
            progress_slot: Some(Arc::clone(&slot)),
        };
        burn::burn(&request)
    };

    let result = do_burn(config.encoder);
    // انکود HW شکست خورد → یک تلاش مجدد با CPU (شفاف از طریق encoder آیتم)
    let result = match result {
        Err(BurnError::Unsuccessful(_)) if config.encoder != encoder::VideoEncoder::Libx264 => {
            crate::logging::warn(
                "hardsub",
                &format!(
                    "انکودر {} برای «{}» شکست خورد؛ تلاش مجدد با CPU",
                    config.encoder.label(),
                    plan_item.video_file_name
                ),
            );
            control.update_item(index, |item| {
                item.status = HardsubItemStatus::Encoding { percent: None };
                item.encoder = Some(encoder::VideoEncoder::Libx264.label().to_string());
            });
            do_burn(encoder::VideoEncoder::Libx264)
        }
        other => other,
    };

    match result {
        Ok(outcome) => {
            if let Some(warning) = &outcome.cleanup_warning {
                crate::logging::warn("hardsub", warning);
            }
            crate::logging::info(
                "hardsub",
                &format!(
                    "«{}» هاردساب شد → {}",
                    plan_item.video_file_name,
                    outcome.output_path.display()
                ),
            );
            control.update_item(index, |item| {
                item.status = HardsubItemStatus::Done {
                    output_path: outcome.output_path.to_string_lossy().into_owned(),
                };
                item.encoder = Some(outcome.encoder_used.label().to_string());
            });
        }
        Err(BurnError::Cancelled) => {
            control.update_item(index, |item| item.status = HardsubItemStatus::Cancelled);
        }
        Err(BurnError::Conflict) => {
            control.update_item(index, |item| item.status = HardsubItemStatus::Conflict);
        }
        Err(other) => {
            let reason = other.message();
            crate::logging::error(
                "hardsub",
                &format!(
                    "هاردساب «{}» ناموفق بود: {reason}",
                    plan_item.video_file_name
                ),
            );
            control.update_item(index, |item| {
                item.status = HardsubItemStatus::Failed { reason };
            });
        }
    }
}

/// لغو هاردساب فعال — ست پرچم (همان Arc پرچم batch guard) + cancelling؛
/// workerها آیتم‌های باز را Cancelled می‌کنند و `hardsub:done` می‌فرستند.
pub fn cancel_active_hardsub(state: &AppState) -> bool {
    let control = {
        let published = state.hardsub.lock().unwrap_or_else(PoisonError::into_inner);
        published.as_ref().map(Arc::clone)
    };
    match control {
        Some(control) => {
            control.cancel.store(true, Ordering::SeqCst);
            control.set_cancelling(true);
            true
        }
        None => false,
    }
}
