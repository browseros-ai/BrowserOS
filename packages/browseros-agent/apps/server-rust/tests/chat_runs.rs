//! `ConversationRuns` semantics, ported case by case from the TS suite
//! (`apps/server/tests/api/services/conversation-runs.test.ts`) plus the ordering guarantees the
//! Rust port adds: 409 before preparation finishes, `on_finish` before the stream end, stop after
//! a draining engine, and panic/rejection cleanup. Engines are hand-fed chunk channels.

use futures_util::StreamExt as _;
use server_api::{ConversationPanelAssignments, ConversationRunStatus};
use server_rust::{
    activity::ServerActivity,
    chat::{
        ChatRejection, ConversationRuns, FinishedTurn, OnFinish, PanelStream, Prepared, Resume,
        RunPresentation, RunStatus, stopped_chunk,
    },
};
use std::{
    error::Error,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use ui_message_stream::{ChunkStream, UiChunk, UiMessage};
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error>>;

fn text_start() -> UiChunk {
    UiChunk::TextStart {
        id: "answer".into(),
        provider_metadata: None,
    }
}

fn text_delta() -> UiChunk {
    UiChunk::TextDelta {
        id: "answer".into(),
        delta: "hello".into(),
        provider_metadata: None,
    }
}

/// An engine stream the test writes into; dropping the sender ends it.
fn controlled() -> (mpsc::UnboundedSender<UiChunk>, ChunkStream) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|chunk| (chunk, receiver))
    });
    (sender, Box::pin(stream))
}

fn noop_finish() -> OnFinish {
    Box::new(|_| Box::pin(async { None }))
}

fn presentation(tabs: &[u32]) -> RunPresentation {
    RunPresentation {
        panel_tab_ids: tabs.to_vec(),
        panels_visible: true,
        tab_group: None,
        keep_replay: true,
    }
}

fn runs() -> Arc<ConversationRuns> {
    ConversationRuns::new(Arc::new(ServerActivity::new()), Duration::from_secs(5))
}

async fn start(
    runs: &Arc<ConversationRuns>,
    conversation_id: Uuid,
    tabs: &[u32],
    chunks: ChunkStream,
) -> Result<Uuid, ChatRejection> {
    runs.start(conversation_id, presentation(tabs), move |_| async move {
        Ok(Prepared {
            chunks,
            on_finish: noop_finish(),
        })
    })
    .await
    .map(|started| started.run_id)
}

fn stream_of(resume: Resume) -> Result<ChunkStream, Box<dyn Error>> {
    match resume {
        Resume::Stream(stream) => Ok(stream),
        Resume::NoRun => Err("expected a stream, got NoRun".into()),
        Resume::RunChanged => Err("expected a stream, got RunChanged".into()),
    }
}

async fn next_panels(
    panels: &mut PanelStream,
) -> Result<ConversationPanelAssignments, Box<dyn Error>> {
    tokio::time::timeout(Duration::from_secs(5), panels.next())
        .await?
        .ok_or_else(|| "panel stream ended".into())
}

async fn current_panels(
    runs: &Arc<ConversationRuns>,
) -> Result<ConversationPanelAssignments, Box<dyn Error>> {
    next_panels(&mut runs.panel_assignments()).await
}

fn tab_ids(assignments: &ConversationPanelAssignments) -> Vec<i64> {
    assignments.assignments.iter().map(|a| a.tab_id).collect()
}

async fn eventually(mut check: impl FnMut() -> bool) -> TestResult {
    for _ in 0..200 {
        if check() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Err("condition never held".into())
}

#[tokio::test]
async fn publishes_one_canonical_run_to_streams_and_panel_assignments() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let mut panels = runs.panel_assignments();
    assert_eq!(next_panels(&mut panels).await?.assignments, vec![]);

    let (source, chunks) = controlled();
    let run_id = start(&runs, conversation, &[10, 11], chunks).await?;
    let attached = next_panels(&mut panels).await?;
    assert_eq!(tab_ids(&attached), vec![10, 11]);
    for assignment in &attached.assignments {
        assert_eq!(assignment.conversation_id, conversation.to_string());
        assert_eq!(assignment.run_id, run_id.to_string());
        assert_eq!(assignment.status, ConversationRunStatus::Running);
        assert_eq!(assignment.auto_open_id, None);
    }

    let pinned = runs.active_run(conversation).ok_or("no active run")?;
    assert!(runs.associate_tabs(&pinned, &[12]));
    assert_eq!(tab_ids(&next_panels(&mut panels).await?), vec![10, 11, 12]);

    let streamed = stream_of(runs.subscribe(conversation, None))?;
    source.send(text_start())?;
    drop(source);
    assert_eq!(streamed.collect::<Vec<_>>().await, vec![text_start()]);
    let completed = next_panels(&mut panels).await?;
    assert_eq!(tab_ids(&completed), vec![10, 11, 12]);
    assert!(
        completed
            .assignments
            .iter()
            .all(|a| a.status == ConversationRunStatus::Completed)
    );

    // A request pins the record that authorized it; late effects never reach a later turn.
    let (_next, chunks) = controlled();
    start(&runs, conversation, &[10], chunks).await?;
    assert!(!runs.associate_tabs(&pinned, &[13]));
    assert!(runs.stop(conversation).await);
    Ok(())
}

#[tokio::test]
async fn keeps_creation_identity_across_turns_and_removes_only_retired_bindings() -> TestResult {
    let runs = runs();
    let old = Uuid::new_v4();
    let (first, chunks) = controlled();
    start(&runs, old, &[10], chunks).await?;
    let run = runs.active_run(old).ok_or("no active run")?;
    runs.associate_tabs(&run, &[11, 12]);
    let initial = current_panels(&runs).await?.assignments;
    assert_eq!(initial[0].auto_open_id, None);
    assert!(initial[1].auto_open_id.is_some());

    drop(first);
    stream_of(runs.subscribe(old, None))?
        .collect::<Vec<_>>()
        .await;
    assert_eq!(tab_ids(&current_panels(&runs).await?), vec![10, 11, 12]);

    let (_second, chunks) = controlled();
    start(&runs, old, &[10], chunks).await?;
    assert_eq!(
        current_panels(&runs).await?.assignments[1].auto_open_id,
        initial[1].auto_open_id
    );
    assert!(matches!(
        runs.subscribe(old, Some(&run.run_id.to_string())),
        Resume::RunChanged
    ));

    let new = Uuid::new_v4();
    let (_third, chunks) = controlled();
    start(&runs, new, &[12], chunks).await?;
    assert!(runs.delete(old).await);
    let remaining = current_panels(&runs).await?.assignments;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].tab_id, 12);
    assert_eq!(remaining[0].conversation_id, new.to_string());
    runs.remove_panel_tab(12);
    assert_eq!(current_panels(&runs).await?.assignments, vec![]);
    runs.stop(new).await;
    Ok(())
}

#[tokio::test]
async fn retires_mappings_immediately_while_provider_preparation_is_still_unwinding() -> TestResult
{
    let runs = runs();
    let conversation = Uuid::new_v4();
    let (release, released) = oneshot::channel::<ChunkStream>();
    let starting = tokio::spawn({
        let runs = runs.clone();
        async move {
            runs.start(conversation, presentation(&[4]), move |_| async move {
                let chunks = released.await.map_err(|_| ChatRejection::Busy)?;
                Ok(Prepared {
                    chunks,
                    on_finish: noop_finish(),
                })
            })
            .await
        }
    });
    eventually(|| runs.active_run(conversation).is_some()).await?;
    let run = runs.active_run(conversation).ok_or("no active run")?;
    let retiring = tokio::spawn({
        let runs = runs.clone();
        async move { runs.delete(conversation).await }
    });
    eventually(|| !runs.associate_tabs(&run, &[5])).await?;
    assert_eq!(current_panels(&runs).await?.assignments, vec![]);

    let (_source, chunks) = controlled();
    let _ = release.send(chunks);
    starting.await??;
    assert!(retiring.await?);
    Ok(())
}

#[tokio::test]
async fn periodically_repeats_current_panel_assignments_for_client_healing() -> TestResult {
    let runs = ConversationRuns::new(Arc::new(ServerActivity::new()), Duration::from_millis(20));
    let conversation = Uuid::new_v4();
    let mut panels = runs.panel_assignments();
    assert_eq!(next_panels(&mut panels).await?.assignments, vec![]);

    let (_source, chunks) = controlled();
    let run_id = start(&runs, conversation, &[14], chunks).await?;
    let changed = next_panels(&mut panels).await?;
    let heartbeat = next_panels(&mut panels).await?;
    assert_eq!(changed.assignments.len(), 1);
    assert_eq!(changed.assignments[0].tab_id, 14);
    assert_eq!(changed.assignments[0].run_id, run_id.to_string());
    assert_eq!(heartbeat, changed);

    drop(panels);
    runs.stop(conversation).await;
    Ok(())
}

#[tokio::test]
async fn keeps_replay_multicast_explicit_cancellation_and_preparation_races_server_owned()
-> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let (provide, provided) = oneshot::channel::<ChunkStream>();
    let starting = tokio::spawn({
        let runs = runs.clone();
        async move {
            runs.start(conversation, presentation(&[]), move |_| async move {
                let chunks = provided.await.map_err(|_| ChatRejection::Busy)?;
                Ok(Prepared {
                    chunks,
                    on_finish: noop_finish(),
                })
            })
            .await
        }
    });
    eventually(|| runs.active_run(conversation).is_some()).await?;

    // Stopping mid-preparation answers at once; the registration still blocks a new turn.
    assert!(runs.stop(conversation).await);
    let (_unused, chunks) = controlled();
    assert!(matches!(
        start(&runs, conversation, &[], chunks).await,
        Err(ChatRejection::Busy)
    ));

    let (_late, chunks) = controlled();
    let _ = provide.send(chunks);
    starting.await??;
    eventually(|| {
        runs.snapshot(conversation)
            .is_some_and(|s| s.status == RunStatus::Aborted)
    })
    .await?;

    let (source, chunks) = controlled();
    start(&runs, conversation, &[], chunks).await?;
    let first = stream_of(runs.subscribe(conversation, None))?;
    source.send(text_start())?;
    eventually(|| {
        runs.snapshot(conversation)
            .is_some_and(|s| s.chunk_count == 1)
    })
    .await?;
    let late = stream_of(runs.subscribe(conversation, None))?;
    source.send(text_delta())?;
    drop(source);
    assert_eq!(
        first.collect::<Vec<_>>().await,
        vec![text_start(), text_delta()]
    );
    assert_eq!(
        late.collect::<Vec<_>>().await,
        vec![text_start(), text_delta()]
    );
    Ok(())
}

/// Drop-observing wrapper: tells whether the registry let go of the engine's stream early.
struct Watched {
    inner: ChunkStream,
    dropped: Arc<AtomicBool>,
}

impl futures_core::Stream for Watched {
    type Item = UiChunk;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<UiChunk>> {
        self.inner.as_mut().poll_next(cx)
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn does_not_let_a_subscriber_disconnect_cancel_execution() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let (source, chunks) = controlled();
    let dropped = Arc::new(AtomicBool::new(false));
    let watched: ChunkStream = Box::pin(Watched {
        inner: chunks,
        dropped: dropped.clone(),
    });
    start(&runs, conversation, &[], watched).await?;

    let abandoned = stream_of(runs.subscribe(conversation, None))?;
    let remaining = stream_of(runs.subscribe(conversation, None))?;
    drop(abandoned);
    source.send(text_delta())?;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !dropped.load(Ordering::SeqCst),
        "a viewer disconnect dropped the engine stream"
    );
    assert!(runs.active_run(conversation).is_some());
    drop(source);

    assert_eq!(remaining.collect::<Vec<_>>().await, vec![text_delta()]);
    eventually(|| {
        runs.snapshot(conversation)
            .is_some_and(|s| s.status == RunStatus::Completed)
    })
    .await?;
    Ok(())
}

#[tokio::test]
async fn marks_an_error_chunk_as_a_failed_run_and_panel_assignment() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let mut panels = runs.panel_assignments();
    next_panels(&mut panels).await?;
    let (source, chunks) = controlled();
    start(&runs, conversation, &[7], chunks).await?;
    next_panels(&mut panels).await?;

    source.send(UiChunk::Error {
        error_text: "provider unavailable".into(),
    })?;
    drop(source);

    assert_eq!(
        next_panels(&mut panels).await?.assignments[0].status,
        ConversationRunStatus::Failed
    );
    assert_eq!(
        runs.snapshot(conversation).map(|s| s.status),
        Some(RunStatus::Failed)
    );
    Ok(())
}

#[tokio::test]
async fn waits_for_preparation_cleanup_before_deletion_and_blocks_replacement() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let (provide, provided) = oneshot::channel::<ChunkStream>();
    let starting = tokio::spawn({
        let runs = runs.clone();
        async move {
            runs.start(conversation, presentation(&[]), move |_| async move {
                let chunks = provided.await.map_err(|_| ChatRejection::Busy)?;
                Ok(Prepared {
                    chunks,
                    on_finish: noop_finish(),
                })
            })
            .await
        }
    });
    eventually(|| runs.active_run(conversation).is_some()).await?;
    let deleting = tokio::spawn({
        let runs = runs.clone();
        async move { runs.delete(conversation).await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!deleting.is_finished());
    let (_unused, chunks) = controlled();
    assert!(matches!(
        start(&runs, conversation, &[], chunks).await,
        Err(ChatRejection::Busy)
    ));

    let (_late, chunks) = controlled();
    let _ = provide.send(chunks);
    starting.await??;
    assert!(deleting.await?);
    assert!(runs.snapshot(conversation).is_none());
    Ok(())
}

#[tokio::test]
async fn wakes_retained_panels_without_reclaiming_tabs_owned_by_another_conversation() -> TestResult
{
    let runs = runs();
    let older = Uuid::new_v4();
    let newer = Uuid::new_v4();
    let (first, chunks) = controlled();
    start(&runs, older, &[20], chunks).await?;
    let run = runs.active_run(older).ok_or("no active run")?;
    runs.associate_tabs(&run, &[21]);
    drop(first);
    eventually(|| {
        runs.snapshot(older)
            .is_some_and(|s| s.status == RunStatus::Completed)
    })
    .await?;

    let (_newer_source, chunks) = controlled();
    start(&runs, newer, &[21], chunks).await?;
    let (_second, chunks) = controlled();
    start(&runs, older, &[20], chunks).await?;

    let assignments = current_panels(&runs).await?.assignments;
    assert_eq!(assignments.len(), 2);
    assert_eq!(
        (
            assignments[0].tab_id,
            assignments[0].conversation_id.clone()
        ),
        (20, older.to_string())
    );
    assert_eq!(
        (
            assignments[1].tab_id,
            assignments[1].conversation_id.clone()
        ),
        (21, newer.to_string())
    );
    runs.stop(newer).await;
    runs.stop(older).await;
    Ok(())
}

#[tokio::test]
async fn keeps_scheduled_runs_out_of_panel_state() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let (_source, chunks) = controlled();
    runs.start(
        conversation,
        RunPresentation {
            panel_tab_ids: vec![30],
            panels_visible: false,
            tab_group: None,
            keep_replay: false,
        },
        move |_| async move {
            Ok(Prepared {
                chunks,
                on_finish: noop_finish(),
            })
        },
    )
    .await?;
    let run = runs.active_run(conversation).ok_or("no active run")?;
    assert!(!runs.associate_tabs(&run, &[31]));
    assert_eq!(current_panels(&runs).await?.assignments, vec![]);
    runs.stop(conversation).await;
    Ok(())
}

#[tokio::test]
async fn holds_panel_hydration_until_preparation_publishes_canonical_messages() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let (release, released) = oneshot::channel::<()>();
    let starting = tokio::spawn({
        let runs = runs.clone();
        async move {
            runs.start(
                conversation,
                presentation(&[32]),
                move |handle| async move {
                    let _ = released.await;
                    handle.publish_messages(vec![UiMessage::user(
                        "canonical-user".into(),
                        None,
                        vec![],
                    )]);
                    let (_source, chunks) = controlled();
                    Ok(Prepared {
                        chunks,
                        on_finish: noop_finish(),
                    })
                },
            )
            .await
        }
    });
    eventually(|| runs.active_run(conversation).is_some()).await?;
    let hydrating = tokio::spawn({
        let runs = runs.clone();
        async move { runs.prepared_snapshot(conversation).await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!hydrating.is_finished());

    let _ = release.send(());
    starting.await??;
    let snapshot = hydrating.await?.ok_or("no snapshot")?;
    assert_eq!(snapshot.messages[0].id, "canonical-user");
    runs.stop(conversation).await;
    Ok(())
}

#[tokio::test]
async fn rejects_stale_message_updates_from_a_replaced_run() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let (first, chunks) = controlled();
    let first_run = start(&runs, conversation, &[], chunks).await?;
    drop(first);
    eventually(|| {
        runs.snapshot(conversation)
            .is_some_and(|s| s.status == RunStatus::Completed)
    })
    .await?;
    let (_second, chunks) = controlled();
    let second_run = start(&runs, conversation, &[], chunks).await?;

    let message = |id: &str| UiMessage::user(id.into(), None, vec![]);
    assert!(!runs.update_messages(conversation, first_run, vec![message("stale")]));
    assert!(runs.update_messages(conversation, second_run, vec![message("current")]));
    assert_eq!(
        runs.snapshot(conversation)
            .map(|s| s.messages[0].id.clone()),
        Some("current".into())
    );
    runs.stop(conversation).await;
    Ok(())
}

// ───── ordering guarantees of the Rust port ─────

#[tokio::test]
async fn registers_the_run_before_preparation_so_a_concurrent_start_gets_409() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let (release, released) = oneshot::channel::<()>();
    let prepared = Arc::new(AtomicBool::new(false));
    let starting = tokio::spawn({
        let runs = runs.clone();
        let prepared = prepared.clone();
        async move {
            runs.start(conversation, presentation(&[]), move |_| async move {
                let _ = released.await;
                prepared.store(true, Ordering::SeqCst);
                let (_source, chunks) = controlled();
                Ok(Prepared {
                    chunks,
                    on_finish: noop_finish(),
                })
            })
            .await
        }
    });
    eventually(|| runs.active_run(conversation).is_some()).await?;
    let (_unused, chunks) = controlled();
    assert!(matches!(
        start(&runs, conversation, &[], chunks).await,
        Err(ChatRejection::Busy)
    ));
    assert!(!prepared.load(Ordering::SeqCst));
    let _ = release.send(());
    starting.await??;
    runs.stop(conversation).await;
    Ok(())
}

#[tokio::test]
async fn runs_on_finish_before_subscribers_see_the_end() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let persisted = Arc::new(AtomicBool::new(false));
    let observed: Arc<Mutex<Option<FinishedTurn>>> = Arc::new(Mutex::new(None));
    let (source, chunks) = controlled();
    let on_finish: OnFinish = {
        let persisted = persisted.clone();
        let observed = observed.clone();
        Box::new(move |finished| {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                persisted.store(true, Ordering::SeqCst);
                if let Ok(mut slot) = observed.lock() {
                    *slot = Some(finished);
                }
                None
            })
        })
    };
    runs.start(conversation, presentation(&[]), move |_| async move {
        Ok(Prepared { chunks, on_finish })
    })
    .await?;
    let subscriber = stream_of(runs.subscribe(conversation, None))?;
    source.send(text_start())?;
    drop(source);
    let chunks = subscriber.collect::<Vec<_>>().await;
    assert_eq!(chunks, vec![text_start()]);
    assert!(
        persisted.load(Ordering::SeqCst),
        "the stream ended before on_finish completed"
    );
    let status = observed
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|f| f.status));
    assert_eq!(status, Some(RunStatus::Completed));
    Ok(())
}

#[tokio::test]
async fn stop_cancels_the_engine_drains_its_abort_and_returns_after_the_end() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let finished_status = Arc::new(Mutex::new(None));
    runs.start(conversation, presentation(&[]), {
        let finished_status = finished_status.clone();
        move |handle| async move {
            // A well-behaved engine: streams until cancelled, then emits `abort` and ends.
            let cancel = handle.cancel().clone();
            let chunks: ChunkStream = Box::pin(futures_util::stream::unfold(0, move |step| {
                let cancel = cancel.clone();
                async move {
                    match step {
                        0 => Some((text_start(), 1)),
                        1 => {
                            cancel.cancelled().await;
                            Some((stopped_chunk(), 2))
                        }
                        _ => None,
                    }
                }
            }));
            let on_finish: OnFinish = Box::new(move |finished| {
                Box::pin(async move {
                    if let Ok(mut slot) = finished_status.lock() {
                        *slot = Some(finished.status);
                    }
                    None
                })
            });
            Ok(Prepared { chunks, on_finish })
        }
    })
    .await?;
    eventually(|| {
        runs.snapshot(conversation)
            .is_some_and(|s| s.chunk_count == 1)
    })
    .await?;
    assert!(runs.stop(conversation).await);
    let snapshot = runs.snapshot(conversation).ok_or("no snapshot")?;
    assert_eq!(snapshot.status, RunStatus::Aborted);
    assert_eq!(snapshot.chunk_count, 2);
    assert_eq!(
        *finished_status.lock().map_err(|_| "poisoned")?,
        Some(RunStatus::Aborted)
    );
    assert!(
        !runs.stop(conversation).await,
        "a finished run cannot be stopped again"
    );
    let replay = stream_of(runs.subscribe(conversation, None))?
        .collect::<Vec<_>>()
        .await;
    assert_eq!(replay, vec![text_start(), stopped_chunk()]);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn stop_bounds_an_engine_that_ignores_cancellation() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let (_never_closed, chunks) = controlled();
    start(&runs, conversation, &[], chunks).await?;
    assert!(runs.stop(conversation).await);
    assert_eq!(
        runs.snapshot(conversation).map(|s| s.status),
        Some(RunStatus::Aborted)
    );
    Ok(())
}

#[tokio::test]
async fn a_rejected_preparation_leaves_a_failed_record_and_releases_activity() -> TestResult {
    let activity = Arc::new(ServerActivity::new());
    let runs = ConversationRuns::new(activity.clone(), Duration::from_secs(5));
    let conversation = Uuid::new_v4();
    let rejected = runs
        .start(conversation, presentation(&[3]), |_| async {
            Err::<Prepared, _>(ChatRejection::UnknownAgent)
        })
        .await;
    assert!(matches!(rejected, Err(ChatRejection::UnknownAgent)));
    let snapshot = runs
        .prepared_snapshot(conversation)
        .await
        .ok_or("no snapshot")?;
    assert_eq!(snapshot.status, RunStatus::Failed);
    assert!(
        stream_of(runs.subscribe(conversation, None))?
            .collect::<Vec<_>>()
            .await
            .is_empty()
    );
    assert_eq!(
        current_panels(&runs).await?.assignments[0].status,
        ConversationRunStatus::Failed
    );
    eventually(|| activity.can_update()).await?;
    assert_eq!(runs.running_count(), 0);

    // The failed record does not block the next turn.
    let (_source, chunks) = controlled();
    start(&runs, conversation, &[], chunks).await?;
    runs.stop(conversation).await;
    Ok(())
}

#[tokio::test]
async fn a_panicking_preparation_answers_500_and_frees_the_conversation() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let crashed = runs
        .start(conversation, presentation(&[]), move |_| async move {
            if conversation != Uuid::nil() {
                panic!("engine bug");
            }
            Err::<Prepared, _>(ChatRejection::Busy)
        })
        .await;
    assert!(matches!(crashed, Err(ChatRejection::Internal(_))));
    eventually(|| {
        runs.snapshot(conversation)
            .is_some_and(|s| s.status == RunStatus::Failed)
    })
    .await?;
    let (_source, chunks) = controlled();
    start(&runs, conversation, &[], chunks).await?;
    runs.stop(conversation).await;
    Ok(())
}

#[tokio::test]
async fn reports_activity_while_running() -> TestResult {
    let activity = Arc::new(ServerActivity::new());
    let runs = ConversationRuns::new(activity.clone(), Duration::from_secs(5));
    let conversation = Uuid::new_v4();
    let (source, chunks) = controlled();
    start(&runs, conversation, &[], chunks).await?;
    eventually(|| !activity.can_update()).await?;
    assert_eq!(runs.running_count(), 1);
    drop(source);
    eventually(|| activity.can_update()).await?;
    assert_eq!(runs.running_count(), 0);
    Ok(())
}

#[tokio::test]
async fn resume_compares_the_run_id_as_the_client_sent_it() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    assert!(matches!(runs.subscribe(conversation, None), Resume::NoRun));
    assert!(matches!(
        runs.subscribe(conversation, Some("x")),
        Resume::NoRun
    ));
    let (_source, chunks) = controlled();
    let run_id = start(&runs, conversation, &[], chunks).await?;
    assert!(matches!(
        runs.subscribe(conversation, Some(&run_id.to_string())),
        Resume::Stream(_)
    ));
    assert!(matches!(
        runs.subscribe(conversation, Some(&run_id.to_string().to_uppercase())),
        Resume::RunChanged
    ));
    assert!(matches!(
        runs.subscribe(conversation, Some("")),
        Resume::RunChanged
    ));
    runs.stop(conversation).await;
    Ok(())
}

// ───── review regressions (P8-fix) ─────

/// The POST that started a run streams that run, even once the conversation has moved on to a
/// newer turn (it used to subscribe by conversation id, and could pick up the next run).
#[tokio::test]
async fn the_starter_streams_its_own_run_after_the_conversation_moved_on() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let (source, chunks) = controlled();
    let first = runs
        .start(conversation, presentation(&[]), move |_| async move {
            Ok(Prepared {
                chunks,
                on_finish: noop_finish(),
            })
        })
        .await?;
    source.send(text_start())?;
    drop(source);
    eventually(|| {
        runs.snapshot(conversation)
            .is_some_and(|s| s.status == RunStatus::Completed)
    })
    .await?;

    let (next, chunks) = controlled();
    let second = start(&runs, conversation, &[], chunks).await?;
    next.send(text_delta())?;
    assert_ne!(first.run_id, second);
    assert_eq!(first.chunks.collect::<Vec<_>>().await, vec![text_start()]);
    runs.stop(conversation).await;
    Ok(())
}

/// A later turn that fails before publishing its own history (unknown agent, credentials, a
/// history read) leaves the conversation looking as it did, not empty.
#[tokio::test]
async fn a_new_record_shows_the_previous_messages_until_its_turn_publishes() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let earlier = vec![UiMessage::user("earlier".into(), None, vec![])];
    let (source, chunks) = controlled();
    runs.start(conversation, presentation(&[]), {
        let earlier = earlier.clone();
        move |handle| async move {
            handle.publish_messages(earlier);
            Ok(Prepared {
                chunks,
                on_finish: noop_finish(),
            })
        }
    })
    .await?;
    drop(source);
    eventually(|| {
        runs.snapshot(conversation)
            .is_some_and(|s| s.status == RunStatus::Completed)
    })
    .await?;

    let rejected = runs
        .start(conversation, presentation(&[]), |_| async {
            Err::<Prepared, _>(ChatRejection::UnknownAgent)
        })
        .await;
    assert!(matches!(rejected, Err(ChatRejection::UnknownAgent)));
    let snapshot = runs.snapshot(conversation).ok_or("no snapshot")?;
    assert_eq!(snapshot.status, RunStatus::Failed);
    assert_eq!(snapshot.messages, earlier);
    Ok(())
}

/// `/state` never pairs `running` with the run's final history: a panel resuming from such a
/// snapshot would replay the whole turn onto the finished assistant message.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn final_messages_are_published_together_with_the_terminal_status() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let user = UiMessage::user("question".into(), None, vec![]);
    let assistant = UiMessage {
        id: "answer".into(),
        role: ui_message_stream::Role::Assistant,
        metadata: None,
        parts: vec![],
    };
    let (entered_tx, entered) = oneshot::channel::<()>();
    let (release, released) = oneshot::channel::<()>();
    let (source, chunks) = controlled();
    runs.start(conversation, presentation(&[]), {
        let user = user.clone();
        let assistant = assistant.clone();
        move |handle| async move {
            handle.publish_messages(vec![user.clone()]);
            let on_finish: OnFinish = Box::new(move |_| {
                Box::pin(async move {
                    // Stands in for the persistence await.
                    let _ = entered_tx.send(());
                    let _ = released.await;
                    Some(vec![user, assistant])
                })
            });
            Ok(Prepared { chunks, on_finish })
        }
    })
    .await?;
    source.send(text_start())?;
    drop(source);
    entered.await?;
    let persisting = runs.snapshot(conversation).ok_or("no snapshot")?;
    assert_eq!(persisting.status, RunStatus::Running);
    assert_eq!(persisting.messages, vec![user.clone()]);

    let watcher = tokio::spawn({
        let runs = runs.clone();
        async move {
            loop {
                let Some(snapshot) = runs.snapshot(conversation) else {
                    return Err("record vanished".to_owned());
                };
                match snapshot.status {
                    RunStatus::Running if snapshot.messages.len() > 1 => {
                        return Err("running with the final history".to_owned());
                    }
                    RunStatus::Running => tokio::task::yield_now().await,
                    _ => return Ok(snapshot),
                }
            }
        }
    });
    let _ = release.send(());
    let finished = watcher.await??;
    assert_eq!(finished.status, RunStatus::Completed);
    assert_eq!(finished.messages, vec![user, assistant]);
    Ok(())
}

/// `stop` must cancel before it reads whether the run reached its stream: read first, and the
/// pump could start in between, so `stopped: true` came back while chunks were still landing.
/// Invariant checked: once `stop` answers true, the replay log no longer grows. The interleaving
/// is a few instructions wide, so this is a probabilistic guard, not a deterministic repro.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_never_answers_while_the_turn_still_streams() -> TestResult {
    let runs = runs();
    for _ in 0..2000 {
        let conversation = Uuid::new_v4();
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let starting = tokio::spawn({
            let runs = runs.clone();
            let barrier = barrier.clone();
            async move {
                runs.start(conversation, presentation(&[]), move |_| async move {
                    barrier.wait().await;
                    // An engine that ignores cancellation: one chunk, then it ends.
                    let chunks: ChunkStream = Box::pin(futures_util::stream::iter([text_delta()]));
                    Ok(Prepared {
                        chunks,
                        on_finish: noop_finish(),
                    })
                })
                .await
                .map(|started| started.run_id)
            }
        });
        // Released together with `prepare`'s return, so the stop races the pump's start.
        barrier.wait().await;
        let stopped = runs.stop(conversation).await;
        let answered = runs.snapshot(conversation).ok_or("no snapshot")?;
        starting.await??;
        eventually(|| {
            runs.snapshot(conversation)
                .is_some_and(|s| s.status != RunStatus::Running)
        })
        .await?;
        let settled = runs.snapshot(conversation).ok_or("no snapshot")?;
        if stopped {
            assert_eq!(
                answered.chunk_count, settled.chunk_count,
                "a chunk landed after stop answered"
            );
        }
    }
    Ok(())
}

/// DELETE removes the record last: while the engines release the conversation a new turn is
/// 409, and a delete abandoned part-way (its client hung up) still frees the conversation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_removes_the_record_after_cleanup_and_even_when_abandoned() -> TestResult {
    let runs = runs();
    let conversation = Uuid::new_v4();
    let (source, chunks) = controlled();
    start(&runs, conversation, &[], chunks).await?;
    drop(source);
    eventually(|| {
        runs.snapshot(conversation)
            .is_some_and(|s| s.status == RunStatus::Completed)
    })
    .await?;

    let (entered_tx, entered) = oneshot::channel::<()>();
    let (release, released) = oneshot::channel::<()>();
    let deleting = tokio::spawn({
        let runs = runs.clone();
        async move {
            runs.delete_with(conversation, async move {
                let _ = entered_tx.send(());
                let _ = released.await;
                "released"
            })
            .await
        }
    });
    entered.await?;
    assert!(runs.snapshot(conversation).is_some());
    let (_unused, chunks) = controlled();
    assert!(matches!(
        start(&runs, conversation, &[], chunks).await,
        Err(ChatRejection::Busy)
    ));
    let _ = release.send(());
    assert_eq!(deleting.await?, (true, "released"));
    assert!(runs.snapshot(conversation).is_none());

    // Abandoned mid-cleanup: the record still leaves the registry.
    let (_source, chunks) = controlled();
    start(&runs, conversation, &[], chunks).await?;
    let abandoned = tokio::spawn({
        let runs = runs.clone();
        async move {
            runs.delete_with(conversation, std::future::pending::<()>())
                .await
        }
    });
    eventually(|| runs.active_run(conversation).is_none()).await?;
    abandoned.abort();
    let _ = abandoned.await;
    eventually(|| runs.snapshot(conversation).is_none()).await?;
    let (_next, chunks) = controlled();
    start(&runs, conversation, &[], chunks).await?;
    runs.stop(conversation).await;
    Ok(())
}

/// A `keep_replay: false` (scheduled) run keeps its replay log while anyone still reads it and
/// drops it once the last subscriber leaves, or at its end if nobody is watching.
#[tokio::test]
async fn a_run_without_keep_replay_drops_its_log_once_unwatched() -> TestResult {
    let runs = runs();
    let ephemeral = |tabs: &[u32]| RunPresentation {
        keep_replay: false,
        ..presentation(tabs)
    };
    let conversation = Uuid::new_v4();
    let (source, chunks) = controlled();
    let started = runs
        .start(conversation, ephemeral(&[]), move |_| async move {
            Ok(Prepared {
                chunks,
                on_finish: noop_finish(),
            })
        })
        .await?;
    let late = stream_of(runs.subscribe(conversation, None))?;
    source.send(text_start())?;
    source.send(text_delta())?;
    drop(source);
    assert_eq!(
        started.chunks.collect::<Vec<_>>().await,
        vec![text_start(), text_delta()]
    );
    let held = runs.snapshot(conversation).ok_or("no snapshot")?;
    assert_eq!(held.status, RunStatus::Completed);
    assert_eq!(held.chunk_count, 2, "a subscriber is still reading");
    assert_eq!(
        late.collect::<Vec<_>>().await,
        vec![text_start(), text_delta()]
    );
    assert_eq!(runs.snapshot(conversation).map(|s| s.chunk_count), Some(0));

    // Nobody watching at the end: released by `finish` itself.
    let unwatched = Uuid::new_v4();
    let (source, chunks) = controlled();
    drop(
        runs.start(unwatched, ephemeral(&[]), move |_| async move {
            Ok(Prepared {
                chunks,
                on_finish: noop_finish(),
            })
        })
        .await?,
    );
    source.send(text_start())?;
    drop(source);
    eventually(|| {
        runs.snapshot(unwatched)
            .is_some_and(|s| s.status == RunStatus::Completed && s.chunk_count == 0)
    })
    .await?;
    Ok(())
}
