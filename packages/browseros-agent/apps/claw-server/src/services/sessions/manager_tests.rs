#[cfg(test)]
mod tests {
    use crate::analytics::HostAnalyticsBridge;
    use crate::services::sessions::{
        ClawSessionJournal, RetainedGroupAction, RetirementCause, Session, Sessions, teardown_all,
    };
    use crate::{
        analytics::{AnalyticsSink, events},
        db::{AuditLog, DATABASE_FILENAME, Database, SessionTabLedger, audit_log::TaskStatus},
        identity::{ClientIdentity, ClientInfo, ConversationIdentity, generate_fun_name},
        ids::{ConvoId, DispatchId, SessionId},
    };
    use serde_json::{Value, json};
    use std::{
        sync::{
            Arc, Mutex as StdMutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tempfile::tempdir;
    use tokio::time::Instant;
    use tokio_util::sync::CancellationToken;

    #[derive(Default)]
    struct RecordingAnalytics {
        events: StdMutex<Vec<(events::EventDefinition, Value)>>,
    }

    impl AnalyticsSink for RecordingAnalytics {
        fn capture(&self, event: events::EventDefinition, properties: Value) {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((event, properties));
        }
    }

    impl RecordingAnalytics {
        fn snapshot(&self) -> Vec<(events::EventDefinition, Value)> {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    async fn repositories(
        dir: &tempfile::TempDir,
    ) -> anyhow::Result<(Arc<AuditLog>, Arc<SessionTabLedger>)> {
        let database = Database::open(dir.path().join(DATABASE_FILENAME)).await?;
        Ok((
            Arc::new(AuditLog::new(database.clone())),
            Arc::new(SessionTabLedger::new(database)),
        ))
    }

    #[tokio::test]
    async fn sweep_removes_idle_sessions_and_writes_end_row() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log.clone(), session_tabs.clone()),
            Duration::from_secs(5),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let session = Session::new(
            SessionId::new("s1"),
            ClientIdentity::Ephemeral {
                slug: "a1".to_string(),
                label: "A1".to_string(),
            },
            ConversationIdentity::new("a1", "agile-alpaca".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );
        registry.insert_for_testing(session).await;
        // Resume before the sweep writes to SQLite; its worker runs on wall time.
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(6)).await;
        tokio::time::resume();
        assert_eq!(registry.sweep_idle().await?, 1);
        assert_eq!(registry.count().await, 0);
        let detail = audit_log.get_task("s1").await?;
        assert!(detail.is_none());
        Ok(())
    }

    fn test_session(id: &str) -> Arc<Session> {
        Session::new(
            SessionId::new(id),
            ClientIdentity::Ephemeral {
                slug: "a1".to_string(),
                label: "A1".to_string(),
            },
            ConversationIdentity::new("a1", format!("tombstone-{id}")),
            "Codex".to_string(),
            Instant::now(),
        )
    }

    /// A tombstone is a hint for an agent still holding a dead handle, not a permanent
    /// record. Once it expires the handle is genuinely unknown and mints as any other
    /// stranger would, so an agent that ignores the refusal is never locked out for good.
    #[tokio::test]
    async fn a_retirement_expires_with_the_retention_window() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(600),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let id = SessionId::new("retiring");
        registry.insert_for_testing(test_session("retiring")).await;
        registry.cancel_by_session(&id).await?;
        assert_eq!(
            registry.retirement_of(&id).await,
            Some(RetirementCause::Cancelled)
        );

        let now = Instant::now();
        assert_eq!(
            registry.reap_retired(now + Duration::from_secs(59)).await,
            0
        );
        assert_eq!(
            registry.retirement_of(&id).await,
            Some(RetirementCause::Cancelled),
            "reaped before its retention elapsed"
        );
        assert_eq!(
            registry.reap_retired(now + Duration::from_secs(60)).await,
            1
        );
        assert_eq!(registry.retirement_of(&id).await, None);
        Ok(())
    }

    /// An idle sweep or a transport close is nobody's decision, so it retires as `Closed`
    /// and redirects. Only the cockpit Stop refuses.
    #[tokio::test]
    async fn an_unrequested_end_retires_as_closed_not_cancelled() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(600),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let id = SessionId::new("swept");
        registry.insert_for_testing(test_session("swept")).await;
        registry.remove(&id, "closed", Some("idle timeout")).await?;
        assert_eq!(
            registry.retirement_of(&id).await,
            Some(RetirementCause::Closed)
        );
        Ok(())
    }

    /// One successor per dead handle, decided under a single lock. Without this, two calls
    /// arriving together would each mint, and the burst would simply move from sequential
    /// to concurrent.
    #[tokio::test]
    async fn a_replacement_is_bound_once_and_then_reused() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(600),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let id = SessionId::new("closing");
        registry.insert_for_testing(test_session("closing")).await;
        registry
            .remove(&id, "closed", Some("transport closed"))
            .await?;

        let first = registry
            .adopt_replacement(&id, SessionId::new("first"))
            .await;
        let second = registry
            .adopt_replacement(&id, SessionId::new("second"))
            .await;
        assert_eq!(first.to_string(), "first");
        assert_eq!(
            second.to_string(),
            "first",
            "the second caller must adopt the first substitute, not its own"
        );

        // A handle the server has no record of binds just the same. This is the case a
        // restart creates for every handle at once, and leaving it unbound is what handed
        // one agent seven sessions for one handle.
        let stranger = SessionId::new("stranger");
        assert_eq!(
            registry
                .adopt_replacement(&stranger, SessionId::new("third"))
                .await
                .to_string(),
            "third"
        );
        assert_eq!(
            registry
                .adopt_replacement(&stranger, SessionId::new("fourth"))
                .await
                .to_string(),
            "third",
            "an unknown handle must bind once and then stay bound"
        );
        Ok(())
    }

    #[tokio::test]
    async fn session_teardown_releases_every_open_target_claim() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log.clone(), session_tabs.clone()),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let session = Session::new(
            SessionId::new("claim-session"),
            ClientIdentity::Ephemeral {
                slug: "agent".to_string(),
                label: "Agent".to_string(),
            },
            ConversationIdentity::new("agent", "agile-alpaca".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );
        registry.insert_for_testing(session.clone()).await;
        session_tabs
            .claim_target_for_session("target-a", session.id().as_str(), "agent", 1)
            .await?;
        session_tabs
            .claim_target_for_session("target-b", session.id().as_str(), "agent", 2)
            .await?;

        assert!(registry.remove(session.id(), "closed", None).await?);

        for _ in 0..100 {
            if session_tabs
                .all_legacy_claims_released_for_session(session.id().as_str())
                .await?
            {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
        anyhow::bail!("session teardown left claims open")
    }

    #[tokio::test]
    async fn mint_registers_live_session() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let session = registry
            .mint(
                ClientIdentity::Ephemeral {
                    slug: "agent".to_string(),
                    label: "Agent".to_string(),
                },
                ClientInfo {
                    name: "Agent".to_string(),
                    version: "1".to_string(),
                    title: None,
                },
            )
            .await?;
        assert!(registry.lookup(session.id()).await.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn terminal_cancellation_removes_session_and_records_end() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log.clone(), session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let session = registry
            .mint(
                ClientIdentity::Ephemeral {
                    slug: "agent".to_string(),
                    label: "Agent".to_string(),
                },
                ClientInfo {
                    name: "Agent".to_string(),
                    version: "1".to_string(),
                    title: None,
                },
            )
            .await?;
        let token = CancellationToken::new();
        let dispatch_id = DispatchId::new();
        assert!(
            session
                .try_register_dispatch(dispatch_id.clone(), token.clone())
                .await
        );

        let first_registry = registry.clone();
        let first_session_id = session.id().clone();
        let first =
            tokio::spawn(async move { first_registry.cancel_by_session(&first_session_id).await });
        token.cancelled().await;

        let retry_registry = registry.clone();
        let retry_session_id = session.id().clone();
        let retry =
            tokio::spawn(async move { retry_registry.cancel_by_session(&retry_session_id).await });
        tokio::task::yield_now().await;
        assert!(!retry.is_finished());
        assert!(!session.finish_dispatch(&dispatch_id).await);

        assert_eq!(first.await??, Some(1));
        assert_eq!(retry.await??, None);
        assert!(registry.lookup(session.id()).await.is_none());
        let summary = audit_log
            .get_task_summary(session.id().as_str())
            .await?
            .ok_or_else(|| anyhow::anyhow!("cancelled summary missing"))?;
        assert_eq!(summary.status, TaskStatus::Cancelled);
        assert!(
            !registry
                .remove(session.id(), "closed", Some("transport closed"))
                .await?
        );
        Ok(())
    }

    #[tokio::test]
    async fn stopping_one_session_does_not_block_another_session() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let first = registry
            .mint(
                ClientIdentity::Ephemeral {
                    slug: "agent".to_string(),
                    label: "Agent".to_string(),
                },
                ClientInfo {
                    name: "Agent".to_string(),
                    version: "1".to_string(),
                    title: None,
                },
            )
            .await?;
        let second = registry
            .mint(
                ClientIdentity::Ephemeral {
                    slug: "agent".to_string(),
                    label: "Agent".to_string(),
                },
                ClientInfo {
                    name: "Agent".to_string(),
                    version: "1".to_string(),
                    title: None,
                },
            )
            .await?;
        let dispatch_id = DispatchId::new();
        let token = CancellationToken::new();
        assert!(
            first
                .try_register_dispatch(dispatch_id.clone(), token.clone())
                .await
        );

        let first_registry = registry.clone();
        let first_session_id = first.id().clone();
        let first_stop =
            tokio::spawn(async move { first_registry.cancel_by_session(&first_session_id).await });
        token.cancelled().await;

        let second_stop = tokio::time::timeout(
            Duration::from_secs(1),
            registry.cancel_by_session(second.id()),
        )
        .await;
        assert_eq!(second_stop??, Some(0));

        assert!(!first.finish_dispatch(&dispatch_id).await);
        assert_eq!(first_stop.await??, Some(1));
        Ok(())
    }

    #[tokio::test]
    async fn pending_operator_stop_survives_later_transport_close() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log.clone(), session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let session = registry
            .mint(
                ClientIdentity::Ephemeral {
                    slug: "agent".to_string(),
                    label: "Agent".to_string(),
                },
                ClientInfo {
                    name: "Agent".to_string(),
                    version: "1".to_string(),
                    title: None,
                },
            )
            .await?;
        session.request_operator_stop();

        assert!(
            registry
                .remove(session.id(), "closed", Some("transport closed"))
                .await?
        );
        let summary = audit_log
            .get_task_summary(session.id().as_str())
            .await?
            .ok_or_else(|| anyhow::anyhow!("cancelled summary missing"))?;
        assert_eq!(summary.status, TaskStatus::Cancelled);
        Ok(())
    }

    #[tokio::test]
    async fn restores_pending_operator_stop_for_teardown_retry() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let session = Session::new(
            SessionId::new("pending-stop"),
            ClientIdentity::Ephemeral {
                slug: "agent".to_string(),
                label: "Agent".to_string(),
            },
            ConversationIdentity::new("agent", "agile-alpaca".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );
        session.request_operator_stop();
        registry
            .retire(session.id(), RetirementCause::Cancelled)
            .await;

        registry
            .restore_pending_operator_stop(session.id(), session.clone())
            .await;

        let restored = registry
            .lookup(session.id())
            .await
            .ok_or_else(|| anyhow::anyhow!("pending operator stop was not restored"))?;
        assert!(Arc::ptr_eq(&restored, &session));
        // The handle resolves again, so a tombstone left behind would refuse calls to a
        // session that is demonstrably alive.
        assert_eq!(registry.retirement_of(session.id()).await, None);
        Ok(())
    }

    #[tokio::test]
    async fn used_count_excludes_initialize_only_sessions() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let first = Session::new(
            SessionId::new("first"),
            ClientIdentity::Ephemeral {
                slug: "agent".to_string(),
                label: "Agent".to_string(),
            },
            ConversationIdentity::new("agent", "first-label".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );
        let second = Session::new(
            SessionId::new("second"),
            ClientIdentity::Ephemeral {
                slug: "agent".to_string(),
                label: "Agent".to_string(),
            },
            ConversationIdentity::new("agent", "second-label".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );
        registry.insert_for_testing(first.clone()).await;
        registry.insert_for_testing(second.clone()).await;

        assert_eq!(registry.used_count().await, 0);
        first.mark_used();
        assert_eq!(registry.used_count().await, 1);
        second.mark_used();
        assert_eq!(registry.used_count().await, 2);
        Ok(())
    }

    #[tokio::test]
    async fn session_lifecycle_emits_once_at_successful_state_transitions() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let analytics = Arc::new(RecordingAnalytics::default());
        let registry = Sessions::new_with_analytics(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
            HostAnalyticsBridge::new(analytics.clone()),
        );
        let session = registry
            .mint(
                ClientIdentity::Ephemeral {
                    slug: "agent".to_string(),
                    label: "Agent".to_string(),
                },
                ClientInfo {
                    name: "Claude Code".to_string(),
                    version: "1".to_string(),
                    title: None,
                },
            )
            .await?;
        assert!(registry.remove(session.id(), "closed", None).await?);
        assert!(!registry.remove(session.id(), "closed", None).await?);

        assert_eq!(
            analytics.snapshot(),
            vec![
                (
                    events::AGENT_SESSION_STARTED,
                    json!({ "client_name": "Claude Code" }),
                ),
                (
                    events::AGENT_SESSION_ENDED,
                    json!({
                        "kind": "closed",
                        "client_name": "Claude Code",
                        "dispatch_count": 0,
                        "distinct_tool_count": 0,
                        "max_concurrent_used_sessions": 0,
                    }),
                ),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn used_session_emits_sorted_tool_summaries_before_its_end_event() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let analytics = Arc::new(RecordingAnalytics::default());
        let registry = Sessions::new_with_analytics(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
            HostAnalyticsBridge::new(analytics.clone()),
        );
        let session = Session::new(
            SessionId::new("used-session"),
            ClientIdentity::Ephemeral {
                slug: "agent".to_string(),
                label: "Agent".to_string(),
            },
            ConversationIdentity::new("agent", "used-label".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );
        session
            .record_tool_usage("navigate", Duration::from_millis(100), 1)
            .await;
        session
            .record_tool_usage("tabs", Duration::from_millis(20), 2)
            .await;
        session
            .record_tool_usage("navigate", Duration::from_millis(50), 2)
            .await;
        registry.insert_for_testing(session.clone()).await;

        assert!(registry.remove(session.id(), "closed", None).await?);
        assert!(!registry.remove(session.id(), "closed", None).await?);
        assert_eq!(
            analytics.snapshot(),
            vec![
                (
                    events::AGENT_SESSION_TOOL_USAGE,
                    json!({
                        "client_name": "Codex",
                        "tool_name": "navigate",
                        "dispatch_count": 2,
                        "total_duration_ms": 150,
                        "max_duration_ms": 100,
                    }),
                ),
                (
                    events::AGENT_SESSION_TOOL_USAGE,
                    json!({
                        "client_name": "Codex",
                        "tool_name": "tabs",
                        "dispatch_count": 1,
                        "total_duration_ms": 20,
                        "max_duration_ms": 20,
                    }),
                ),
                (
                    events::AGENT_SESSION_ENDED,
                    json!({
                        "kind": "closed",
                        "client_name": "Codex",
                        "dispatch_count": 3,
                        "distinct_tool_count": 2,
                        "max_concurrent_used_sessions": 2,
                    }),
                ),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn failed_start_audit_emits_nothing_and_failed_end_audit_still_emits()
    -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let analytics = Arc::new(RecordingAnalytics::default());
        let registry = Sessions::new_with_analytics(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
            HostAnalyticsBridge::new(analytics.clone()),
        );
        let failed_start = Session::new(
            SessionId::new("failed-start"),
            ClientIdentity::Ephemeral {
                slug: "agent".to_string(),
                label: "Agent".to_string(),
            },
            ConversationIdentity::new("agent", "failed-start-label".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );
        registry
            .reserved_keys
            .lock()
            .await
            .insert(failed_start.convo_id().clone());
        assert!(
            registry
                .finish_mint(
                    failed_start.id().clone(),
                    failed_start.clone(),
                    Err(crate::error::AppError::Internal("audit failed".to_string()).into()),
                )
                .await
                .is_err()
        );
        assert!(registry.lookup(failed_start.id()).await.is_none());
        assert!(analytics.snapshot().is_empty());

        let failed_end = Session::new(
            SessionId::new("failed-end"),
            ClientIdentity::Ephemeral {
                slug: "agent".to_string(),
                label: "Agent".to_string(),
            },
            ConversationIdentity::new("agent", "failed-end-label".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );
        assert!(
            registry
                .finish_teardown(
                    failed_end,
                    "errored",
                    Err(crate::error::AppError::Internal("audit failed".to_string()).into()),
                )
                .await
                .is_err()
        );
        assert_eq!(
            analytics.snapshot(),
            vec![(
                events::AGENT_SESSION_ENDED,
                json!({
                    "kind": "errored",
                    "client_name": "Codex",
                    "dispatch_count": 0,
                    "distinct_tool_count": 0,
                    "max_concurrent_used_sessions": 0,
                }),
            )]
        );
        Ok(())
    }

    #[tokio::test]
    async fn completion_hook_runs_only_after_a_durable_end() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let completed = Arc::new(StdMutex::new(Vec::new()));
        registry.set_completion_hook(Arc::new({
            let completed = completed.clone();
            move |session_id| {
                completed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(session_id);
            }
        }));

        let session = |id: &'static str| {
            Session::new(
                SessionId::new(id),
                ClientIdentity::Ephemeral {
                    slug: "agent".to_string(),
                    label: "Agent".to_string(),
                },
                ConversationIdentity::new("agent", format!("{id}-label")),
                "Codex".to_string(),
                Instant::now(),
            )
        };
        assert!(
            registry
                .finish_teardown(
                    session("failed"),
                    "errored",
                    Err(crate::error::AppError::Internal("audit failed".to_string()).into()),
                )
                .await
                .is_err()
        );
        registry
            .finish_teardown(session("durable"), "closed", Ok(()))
            .await?;

        assert_eq!(
            *completed
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec!["durable".to_string()]
        );
        Ok(())
    }

    #[tokio::test]
    async fn durable_completion_hook_precedes_retained_group_work() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let collapse_entered = Arc::new(tokio::sync::Notify::new());
        let release_collapse = Arc::new(tokio::sync::Notify::new());
        registry.set_retained_group_hook(Arc::new({
            let collapse_entered = collapse_entered.clone();
            let release_collapse = release_collapse.clone();
            move |_, _, _| {
                let collapse_entered = collapse_entered.clone();
                let release_collapse = release_collapse.clone();
                Box::pin(async move {
                    collapse_entered.notify_one();
                    release_collapse.notified().await;
                    true
                })
            }
        }));
        let completion_called = Arc::new(AtomicBool::new(false));
        registry.set_completion_hook(Arc::new({
            let completion_called = completion_called.clone();
            move |_| completion_called.store(true, Ordering::SeqCst)
        }));
        let session = Session::new(
            SessionId::new("durable-before-collapse"),
            ClientIdentity::Ephemeral {
                slug: "agent".to_string(),
                label: "Agent".to_string(),
            },
            ConversationIdentity::new("agent", "durable-before-collapse-label".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );

        let teardown = tokio::spawn({
            let registry = registry.clone();
            async move { registry.finish_teardown(session, "closed", Ok(())).await }
        });
        tokio::time::timeout(Duration::from_secs(1), collapse_entered.notified()).await?;
        assert!(completion_called.load(Ordering::SeqCst));
        assert!(!teardown.is_finished());
        release_collapse.notify_one();
        teardown.await??;
        Ok(())
    }

    #[tokio::test]
    async fn idle_sweep_and_shutdown_emit_for_every_removed_session() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let analytics = Arc::new(RecordingAnalytics::default());
        let registry = Sessions::new_with_analytics(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::ZERO,
            Duration::from_secs(60),
            Duration::from_secs(1),
            HostAnalyticsBridge::new(analytics.clone()),
        );
        for id in ["idle-1", "idle-2"] {
            registry
                .insert_for_testing(Session::new(
                    SessionId::new(id),
                    ClientIdentity::Ephemeral {
                        slug: "agent".to_string(),
                        label: "Agent".to_string(),
                    },
                    ConversationIdentity::new("agent", format!("{id}-label")),
                    "Codex".to_string(),
                    Instant::now(),
                ))
                .await;
        }
        assert_eq!(registry.sweep_idle().await?, 2);

        registry
            .insert_for_testing(Session::new(
                SessionId::new("shutdown"),
                ClientIdentity::Ephemeral {
                    slug: "agent".to_string(),
                    label: "Agent".to_string(),
                },
                ConversationIdentity::new("agent", "shutdown-label".to_string()),
                "Codex".to_string(),
                Instant::now(),
            ))
            .await;
        assert_eq!(registry.shutdown().await?, 1);
        assert_eq!(
            analytics
                .snapshot()
                .into_iter()
                .filter(|(event, _)| *event == events::AGENT_SESSION_ENDED)
                .count(),
            3
        );
        Ok(())
    }

    #[tokio::test]
    async fn shutdown_teardown_continues_after_the_first_error() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let result = teardown_all([0, 1, 2], {
            let attempts = attempts.clone();
            move |index| {
                attempts.fetch_add(1, Ordering::SeqCst);
                async move {
                    if index == 0 {
                        Err(
                            crate::error::AppError::Internal("first teardown failed".to_string())
                                .into(),
                        )
                    } else {
                        Ok(())
                    }
                }
            }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn shutdown_waits_for_a_session_removed_by_transport_close() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let analytics = Arc::new(RecordingAnalytics::default());
        let registry = Sessions::new_with_analytics(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
            HostAnalyticsBridge::new(analytics.clone()),
        );
        let teardown_entered = Arc::new(tokio::sync::Semaphore::new(0));
        let teardown_release = Arc::new(tokio::sync::Semaphore::new(0));
        let hook_entered = teardown_entered.clone();
        let hook_release = teardown_release.clone();
        registry.set_retained_group_hook(Arc::new(move |_, _, action| {
            let hook_entered = hook_entered.clone();
            let hook_release = hook_release.clone();
            Box::pin(async move {
                if matches!(action, RetainedGroupAction::Collapse) {
                    hook_entered.add_permits(1);
                    let Ok(permit) = hook_release.acquire().await else {
                        return false;
                    };
                    permit.forget();
                }
                true
            })
        }));
        let session = Session::new(
            SessionId::new("transport-close"),
            ClientIdentity::Ephemeral {
                slug: "agent".to_string(),
                label: "Agent".to_string(),
            },
            ConversationIdentity::new("agent", "transport-close-label".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );
        registry.insert_for_testing(session.clone()).await;

        let remove_registry = registry.clone();
        let session_id = session.id().clone();
        let remove_task = tokio::spawn(async move {
            remove_registry
                .remove(&session_id, "closed", Some("transport closed"))
                .await
        });
        let entered = teardown_entered
            .acquire()
            .await
            .map_err(|_| anyhow::anyhow!("teardown semaphore closed"))?;
        entered.forget();

        let shutdown_registry = registry.clone();
        let mut shutdown_task = tokio::spawn(async move { shutdown_registry.shutdown().await });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut shutdown_task)
                .await
                .is_err()
        );

        teardown_release.add_permits(1);
        assert!(remove_task.await??);
        assert_eq!(shutdown_task.await??, 0);
        assert_eq!(
            analytics.snapshot(),
            vec![(
                events::AGENT_SESSION_ENDED,
                json!({
                    "kind": "closed",
                    "client_name": "Codex",
                    "dispatch_count": 0,
                    "distinct_tool_count": 0,
                    "max_concurrent_used_sessions": 0,
                }),
            )]
        );
        Ok(())
    }

    #[tokio::test]
    async fn same_client_sessions_get_distinct_identity_and_ownership() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(60),
            Duration::from_secs(1),
        );
        let client = ClientInfo {
            name: "Codex".to_string(),
            version: "1".to_string(),
            title: None,
        };
        let agent = ClientIdentity::Ephemeral {
            slug: "codex".to_string(),
            label: "Codex".to_string(),
        };
        let session1 = registry.mint(agent.clone(), client.clone()).await?;
        let key1 = session1.convo_id().clone();
        registry
            .ownership()
            .claim_page(key1.clone(), browseros_core::PageId(1))
            .await;
        let session2 = registry.mint(agent, client).await?;
        let key2 = session2.convo_id().clone();
        registry
            .ownership()
            .claim_page(key2.clone(), browseros_core::PageId(2))
            .await;

        assert_ne!(key1, key2);
        assert_eq!(
            registry
                .ownership()
                .owned_pages(&key1)
                .await
                .into_iter()
                .collect::<Vec<_>>(),
            vec![browseros_core::PageId(1)]
        );
        assert!(Arc::ptr_eq(
            &registry
                .lookup(session1.id())
                .await
                .ok_or_else(|| anyhow::anyhow!("session missing"))?,
            &session1
        ));
        assert_eq!(
            registry.ownership().owned_pages(&key2).await,
            std::collections::BTreeSet::from([browseros_core::PageId(2)])
        );
        Ok(())
    }

    #[tokio::test]
    async fn retained_session_collapses_then_closes_and_forgets_after_expiry() -> anyhow::Result<()>
    {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(10),
            Duration::from_secs(1),
        );
        let actions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let hook_actions = actions.clone();
        registry.set_retained_group_hook(Arc::new(move |_, _, action| {
            let hook_actions = hook_actions.clone();
            Box::pin(async move {
                hook_actions
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(action);
                true
            })
        }));
        let session_id = SessionId::new("s1");
        let session = Session::new(
            session_id.clone(),
            ClientIdentity::Ephemeral {
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            ConversationIdentity::new("codex", "agile-alpaca".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );
        let key = session.convo_id().clone();
        registry.insert_for_testing(session.clone()).await;
        registry
            .ownership()
            .claim_page(key.clone(), browseros_core::PageId(4))
            .await;
        registry
            .ownership()
            .set_tab_group_ref(key.clone(), Some("group-4".to_string()))
            .await;

        assert!(registry.remove(&session_id, "closed", None).await?);
        let retained_at = registry
            .retained
            .read()
            .await
            .get(&key)
            .map(|retained| retained.ended_at)
            .ok_or_else(|| anyhow::anyhow!("retained session missing"))?;
        assert_eq!(registry.retained.read().await.len(), 1);
        assert_eq!(
            actions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            &[RetainedGroupAction::Collapse]
        );
        assert_eq!(
            registry
                .ownership()
                .owner_of_page(&browseros_core::PageId(4))
                .await,
            Some(key.clone())
        );

        assert_eq!(
            registry
                .reap_retained(retained_at + Duration::from_secs(9))
                .await,
            0
        );
        assert_eq!(registry.retained.read().await.len(), 1);
        assert_eq!(
            registry
                .reap_retained(retained_at + Duration::from_secs(10))
                .await,
            1
        );
        assert_eq!(registry.retained.read().await.len(), 0);
        assert_eq!(registry.ownership().tab_group_ref(&key).await, None);
        assert!(!registry.reserved_keys.lock().await.contains(&key));
        assert_eq!(
            actions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            &[
                RetainedGroupAction::Collapse,
                RetainedGroupAction::Collapse,
                RetainedGroupAction::Close
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn failed_or_disconnected_close_retries_without_forgetting_state() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(10),
            Duration::from_secs(1),
        );
        let close_allowed = Arc::new(AtomicBool::new(false));
        let close_attempts = Arc::new(AtomicUsize::new(0));
        let hook_allowed = close_allowed.clone();
        let hook_attempts = close_attempts.clone();
        registry.set_retained_group_hook(Arc::new(move |_, _, action| {
            let hook_allowed = hook_allowed.clone();
            let hook_attempts = hook_attempts.clone();
            Box::pin(async move {
                if matches!(action, RetainedGroupAction::Close) {
                    hook_attempts.fetch_add(1, Ordering::SeqCst);
                    return hook_allowed.load(Ordering::SeqCst);
                }
                false
            })
        }));
        let session = Session::new(
            SessionId::new("s1"),
            ClientIdentity::Ephemeral {
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            ConversationIdentity::new("codex", "agile-alpaca".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );
        let key = session.convo_id().clone();
        registry.insert_for_testing(session.clone()).await;
        registry
            .ownership()
            .claim_page(key.clone(), browseros_core::PageId(7))
            .await;
        registry
            .ownership()
            .set_tab_group_ref(key.clone(), Some("group-7".to_string()))
            .await;
        registry.remove(session.id(), "closed", None).await?;
        let expired_at = Instant::now() + Duration::from_secs(10);

        assert_eq!(registry.reap_retained(expired_at).await, 0);
        assert_eq!(close_attempts.load(Ordering::SeqCst), 1);
        assert_eq!(registry.retained.read().await.len(), 1);
        assert_eq!(
            registry
                .ownership()
                .owner_of_page(&browseros_core::PageId(7))
                .await,
            Some(key.clone())
        );

        close_allowed.store(true, Ordering::SeqCst);
        assert_eq!(registry.reap_retained(expired_at).await, 1);
        assert_eq!(close_attempts.load(Ordering::SeqCst), 2);
        assert_eq!(registry.retained.read().await.len(), 0);
        assert_eq!(
            registry
                .ownership()
                .owner_of_page(&browseros_core::PageId(7))
                .await,
            None
        );
        Ok(())
    }

    #[tokio::test]
    async fn generated_key_stays_reserved_until_retained_cleanup() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(10),
            Duration::from_secs(1),
        );
        let session = Session::new(
            SessionId::new("s1"),
            ClientIdentity::Ephemeral {
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            ConversationIdentity::new("codex", "agile-alpaca".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );
        registry.insert_for_testing(session.clone()).await;
        registry.remove(session.id(), "closed", None).await?;
        let expired_at = Instant::now() + Duration::from_secs(10);

        let candidate_while_retained = {
            let reserved = registry.reserved_keys.lock().await;
            generate_fun_name(
                || 0.0,
                |label| !reserved.contains(&ConvoId::new(format!("codex-{label}"))),
            )?
        };
        assert_eq!(candidate_while_retained, "agile-alpaca-2");

        assert_eq!(registry.reap_retained(expired_at).await, 1);
        let candidate_after_cleanup = {
            let reserved = registry.reserved_keys.lock().await;
            generate_fun_name(
                || 0.0,
                |label| !reserved.contains(&ConvoId::new(format!("codex-{label}"))),
            )?
        };
        assert_eq!(candidate_after_cleanup, "agile-alpaca");
        Ok(())
    }

    #[tokio::test]
    async fn overlapping_retained_sweeps_issue_one_close() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let (audit_log, session_tabs) = repositories(&dir).await?;
        let registry = Sessions::new(
            ClawSessionJournal::new(audit_log, session_tabs),
            Duration::from_secs(60),
            Duration::from_secs(10),
            Duration::from_secs(1),
        );
        let close_attempts = Arc::new(AtomicUsize::new(0));
        let close_entered = Arc::new(tokio::sync::Notify::new());
        let close_release = Arc::new(tokio::sync::Notify::new());
        let hook_attempts = close_attempts.clone();
        let hook_entered = close_entered.clone();
        let hook_release = close_release.clone();
        registry.set_retained_group_hook(Arc::new(move |_, _, action| {
            let hook_attempts = hook_attempts.clone();
            let hook_entered = hook_entered.clone();
            let hook_release = hook_release.clone();
            Box::pin(async move {
                if matches!(action, RetainedGroupAction::Close) {
                    hook_attempts.fetch_add(1, Ordering::SeqCst);
                    hook_entered.notify_one();
                    hook_release.notified().await;
                }
                true
            })
        }));
        let session = Session::new(
            SessionId::new("s1"),
            ClientIdentity::Ephemeral {
                slug: "codex".to_string(),
                label: "Codex".to_string(),
            },
            ConversationIdentity::new("codex", "agile-alpaca".to_string()),
            "Codex".to_string(),
            Instant::now(),
        );
        registry.insert_for_testing(session.clone()).await;
        registry.remove(session.id(), "closed", None).await?;
        let now = Instant::now() + Duration::from_secs(10);
        let entered = close_entered.notified();
        let first_registry = registry.clone();
        let first = tokio::spawn(async move { first_registry.reap_retained(now).await });
        entered.await;

        assert_eq!(registry.reap_retained(now).await, 0);
        assert_eq!(close_attempts.load(Ordering::SeqCst), 1);
        close_release.notify_one();
        assert_eq!(first.await?, 1);
        assert_eq!(registry.reap_retained(now).await, 0);
        assert_eq!(close_attempts.load(Ordering::SeqCst), 1);
        Ok(())
    }
}
