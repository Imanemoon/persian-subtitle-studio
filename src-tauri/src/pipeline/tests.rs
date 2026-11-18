//! تست‌های PipelineControl و lifecycle worker — بدون Tauri runtime.

use super::*;
use crate::archive::provenance::ProvenanceRegistry;
use crate::state::OpsRegistry;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn make_state() -> AppState {
    AppState {
        settings: Mutex::new(crate::settings::Settings::default()),
        library: Mutex::new(Vec::new()),
        last_scan: Mutex::new(None),
        ops: Arc::new(Mutex::new(OpsRegistry::default())),
        archive_provenance: Mutex::new(ProvenanceRegistry::default()),
        pipeline: Mutex::new(None),
    }
}

fn make_control(state: &AppState, run_id: &str) -> (BatchOpGuard, Arc<PipelineControl>) {
    let guard = crate::state::acquire_batch_op(&state.ops).unwrap();
    let control = Arc::new(PipelineControl::from_guard(
        &guard,
        run_id.to_string(),
        "تست",
    ));
    (guard, control)
}

fn make_item(path: &str) -> PipelineItemSnapshot {
    PipelineItemSnapshot {
        video_path: path.to_string(),
        video_file_name: "Show.S01E01.mkv".into(),
        stage: PipelineStage::Scanning,
        stage_status: StageStatus::Done,
        outcome: None,
        artifacts: ItemArtifacts::default(),
    }
}

/// حذف control با run_id مطابق، قبل از رهایی اسلات — ترتیب واقعی Drop.
#[test]
fn cleanup_clears_control_before_releasing_slot() {
    let state = make_state();
    let (guard, control) = make_control(&state, "run-1");
    {
        let mut published = state.pipeline.lock().unwrap();
        *published = Some(Arc::clone(&control));
    }
    assert!(state.pipeline.lock().unwrap().is_some());
    let cleanup = WorkerCleanup::new(&state, "run-1".into(), guard);
    drop(cleanup);
    assert!(state.pipeline.lock().unwrap().is_none());
    assert!(crate::state::acquire_batch_op(&state.ops).is_ok());
}

/// حذف control با run_id ناهمسان — هیچ چیز پاک نمی‌شود؛ مالک جدید محفوظ می‌ماند.
#[test]
fn mismatched_run_id_clears_nothing() {
    let state = make_state();
    let (_guard_new, control_new) = make_control(&state, "run-new");
    {
        let mut published = state.pipeline.lock().unwrap();
        *published = Some(Arc::clone(&control_new));
    }
    // درخواست پاک‌سازی با run_id ناهمسان → بی‌اثر
    assert!(!clear_published_control(&state, "run-old"));
    assert!(state.pipeline.lock().unwrap().is_some());
    // run_id درست → پاک می‌شود
    assert!(clear_published_control(&state, "run-new"));
    assert!(state.pipeline.lock().unwrap().is_none());
}

/// take_guard_once: دقیقاً یک‌بار؛ خالی → Err صریح.
#[test]
fn guard_handoff_transfers_exactly_once() {
    let state = make_state();
    let (guard, _control) = make_control(&state, "run");
    let handoff: GuardHandoff = Arc::new(Mutex::new(Some(guard)));

    let first = take_guard_once(&handoff).expect("اولین انتقال باید موفق باشد");
    let second = take_guard_once(&handoff);
    assert!(second.is_err(), "دومین انتقال باید خطا بدهد");
    drop(first);
}

/// resolve تصمیم → waiter فوراً با payload بیدار می‌شود (بدون polling).
#[test]
fn decision_resolution_wakes_waiter_with_payload() {
    let state = make_state();
    let (_guard, control) = make_control(&state, "run-dec");

    control.register_decision(
        "dec-1".into(),
        DecisionKind::SyncSidecar,
        "C:\\v\\a.mkv",
        "a.mkv",
        vec![
            DecisionOptionPayload::SidecarPath {
                path: PathBuf::from("C:\\v\\a.fa.srt"),
            },
            DecisionOptionPayload::SidecarPath {
                path: PathBuf::from("C:\\v\\a.fa.ass"),
            },
        ],
    );

    let (result_tx, result_rx) = mpsc::channel();
    let control_for_waiter = Arc::clone(&control);
    let waiter = thread::spawn(move || {
        let exit = control_for_waiter.wait_for_decision("dec-1");
        let _ = result_tx.send(exit);
    });

    // resolve با گزینهٔ مات معتبر — همان مسیر resolve_decision_core
    thread::spawn(move || {
        let state = make_state();
        {
            let mut published = state.pipeline.lock().unwrap();
            *published = Some(Arc::clone(&control));
        }
        let payload = resolve_decision_core(&state, "dec-1", "dec-1-opt-1").unwrap();
        assert!(matches!(payload, DecisionOptionPayload::SidecarPath { .. }));
    })
    .join()
    .unwrap();

    match result_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(WaitExit::Resolved(payload)) => {
            assert!(matches!(payload, DecisionOptionPayload::SidecarPath { .. }));
        }
        other => panic!("باید Resolved باشد، نه {other:?}"),
    }
    waiter.join().unwrap();
}

/// گزینهٔ ناشناخته → Err صریح (بدون تغییر تصمیم).
#[test]
fn resolve_unknown_option_fails_closed() {
    let state = make_state();
    let (_guard, control) = make_control(&state, "run-unknown");
    control.register_decision(
        "dec-u".into(),
        DecisionKind::SyncSidecar,
        "C:\\v\\b.mkv",
        "b.mkv",
        vec![DecisionOptionPayload::SidecarPath {
            path: PathBuf::from("b.fa.srt"),
        }],
    );
    {
        let mut published = state.pipeline.lock().unwrap();
        *published = Some(Arc::clone(&control));
    }
    let err = resolve_decision_core(&state, "dec-u", "opt-unknown").unwrap_err();
    assert!(err.contains("معتبر نیست"), "{err}");
}

/// لغو → waiter فوراً Cancelled می‌گیرد. تصمیم واقعی ثبت می‌شود تا waiter
/// واقعاً در gate.wait بلاک شود؛ control هم باید منتشر شده باشد تا لغو هدفش را
/// پیدا کند. هر دو ترتیب زمانی (لغو قبل یا بعد از ورود waiter به wait) به
/// نتیجهٔ یکسان می‌رسند: پرچم در ابتدای هر دور حلقه بازبینی می‌شود.
#[test]
fn cancel_wakes_waiter_immediately() {
    let state = make_state();
    let (guard, control) = make_control(&state, "run-cancel");
    control.put_item_snapshot(make_item("C:\\v\\a.mkv"));
    {
        let mut published = state.pipeline.lock().unwrap();
        *published = Some(Arc::clone(&control));
    }
    control.register_decision(
        "dec-x".into(),
        DecisionKind::SyncSidecar,
        "C:\\v\\a.mkv",
        "a.mkv",
        vec![DecisionOptionPayload::SidecarPath {
            path: PathBuf::from("a.fa.srt"),
        }],
    );

    let (result_tx, result_rx) = mpsc::channel();
    let control_for_waiter = Arc::clone(&control);
    let waiter = thread::spawn(move || {
        let exit = control_for_waiter.wait_for_decision("dec-x");
        let _ = result_tx.send(exit);
    });
    thread::sleep(Duration::from_millis(50));
    assert!(cancel_active_pipeline(&state).unwrap());

    match result_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(WaitExit::Cancelled) => {}
        other => panic!("باید Cancelled باشد، نه {other:?}"),
    }
    waiter.join().unwrap();
    drop(guard);
}

/// cancel_pipeline بدون pipeline فعال → false بی‌ضرر.
#[test]
fn cancel_without_pipeline_is_harmless_false() {
    let state = make_state();
    assert!(!cancel_active_pipeline(&state).unwrap());
}

/// snapshot: completed فقط از outcomeهای نهایی؛ decisions_pending از pending.
#[test]
fn snapshot_counts_only_terminal_outcomes() {
    let state = make_state();
    let (_guard, control) = make_control(&state, "run-snap");
    control.put_item_snapshot(make_item("C:\\v\\a.mkv"));
    let mut done = make_item("C:\\v\\b.mkv");
    done.outcome = Some(ItemOutcome::AlreadySynced);
    control.put_item_snapshot(done);
    control.put_item_snapshot(make_item("C:\\v\\c.mkv"));
    control.register_decision(
        "dec-s".into(),
        DecisionKind::SyncSidecar,
        "C:\\v\\c.mkv",
        "c.mkv",
        vec![DecisionOptionPayload::SidecarPath {
            path: PathBuf::from("c.fa.srt"),
        }],
    );

    let snapshot = control.snapshot();
    assert_eq!(snapshot.total, 3);
    assert_eq!(snapshot.completed, 1);
    assert_eq!(snapshot.counts.decisions_pending, 1);
    assert_eq!(snapshot.status, PipelineStatus::AwaitingDecision);
    // بازیابی UI حین انتظار تصمیم فقط از طریق snapshot ممکن است
    let pending = snapshot
        .pending_decision
        .expect("تصمیم باز باید در snapshot باشد");
    assert_eq!(pending.decision_id, "dec-s");
    assert_eq!(pending.options.len(), 1);
    assert_eq!(pending.options[0].option_id, "dec-s-opt-0");
}
