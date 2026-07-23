use crate::{
    analytics::{AnalyticsSink, NoopAnalyticsSink, events},
    clock::now_epoch_ms,
    db::{
        Database, SessionEfficiencyStatsRepository,
        entities::session_efficiency_stats,
        session_efficiency_stats::{ELIGIBLE_TOKEN_ESTIMATOR_VERSION, SessionEfficiencySource},
    },
    error::AppResult,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::warn;

pub const EFFICIENCY_ESTIMATOR_VERSION: i64 = 2;
pub const SCREENSHOT_BASELINE_WIDTH: usize = 1920;
pub const SCREENSHOT_BASELINE_HEIGHT: usize = 1080;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const JS_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// Fixed provider-neutral baseline for one 1920x1080 screenshot. Anthropic documents
/// 2,691 visual tokens for Claude Sonnet 5 at this resolution and a 4,784-token
/// high-resolution ceiling; OpenAI's GPT-5.5 high/original rules retain all 2,040
/// 32px patches at this resolution within their documented patch budgets.
///
/// Anthropic: https://platform.claude.com/docs/en/build-with-claude/vision#resolution-and-token-cost
/// OpenAI: https://developers.openai.com/api/docs/guides/images-vision#calculating-costs
#[must_use]
pub const fn screenshot_tokens_per_dispatch() -> i64 {
    3_000
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedSessionEfficiency {
    pub stats: session_efficiency_stats::Model,
    pub end_kind: String,
    pub client_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEfficiencyAggregate {
    pub all_time: SessionEfficiencyWindow,
    pub last_30_days: SessionEfficiencyWindow,
    pub last_7_days: SessionEfficiencyWindow,
}

/// Cockpit-ready totals bounded to JavaScript-safe integers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionEfficiencyWindow {
    pub browser_claw_token_estimate: i64,
    pub screenshot_first_token_estimate: i64,
    pub raw_token_savings_estimate: i64,
    pub human_time_saved_ms: i64,
    pub session_count: i64,
    pub tool_call_count: i64,
}

/// Projects ended audit sessions once and aggregates their primitive measurements.
#[derive(Clone)]
pub struct SessionEfficiencyService {
    repository: SessionEfficiencyStatsRepository,
    analytics: Arc<dyn AnalyticsSink>,
    finalizers: TaskTracker,
    // TaskTracker::close permits later spawns, so enqueue and close need their own boundary.
    finalizer_gate: Arc<Mutex<()>>,
}

impl SessionEfficiencyService {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self::new_with_analytics(db, Arc::new(NoopAnalyticsSink))
    }

    #[must_use]
    pub fn new_with_analytics(db: Database, analytics: Arc<dyn AnalyticsSink>) -> Self {
        Self {
            repository: SessionEfficiencyStatsRepository::new(db),
            analytics,
            finalizers: TaskTracker::new(),
            finalizer_gate: Arc::new(Mutex::new(())),
        }
    }

    /// Queues projection work without extending the durable session-end path.
    pub fn queue_finalize(self: &Arc<Self>, session_id: String) -> bool {
        let _gate = self
            .finalizer_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.finalizers.is_closed() {
            return false;
        }
        let service = Arc::clone(self);
        self.finalizers.spawn(async move {
            if let Err(error) = service.finalize_session(&session_id).await {
                warn!(error = %error, "session efficiency finalization failed");
            }
        });
        true
    }

    /// Closes the shutdown barrier and waits for every tracked finalizer to finish.
    pub async fn drain(&self) {
        {
            let _gate = self
                .finalizer_gate
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.finalizers.close();
        }
        self.finalizers.wait().await;
    }

    pub async fn finalize_session(
        &self,
        session_id: &str,
    ) -> AppResult<Option<FinalizedSessionEfficiency>> {
        self.finalize_session_at(session_id, now_epoch_ms()).await
    }

    async fn finalize_session_at(
        &self,
        session_id: &str,
        computed_at: i64,
    ) -> AppResult<Option<FinalizedSessionEfficiency>> {
        let Some(source) = self.repository.source_for_session(session_id).await? else {
            return Ok(None);
        };
        let Some(stats) = calculate_session_efficiency(&source, computed_at) else {
            return Ok(None);
        };
        if !self.repository.insert_if_absent(&stats).await? {
            return Ok(None);
        }
        let finalized = FinalizedSessionEfficiency {
            stats,
            end_kind: source.end.kind,
            client_name: source.start.map(|start| start.client_name),
        };
        self.emit_computed(&finalized);
        Ok(Some(finalized))
    }

    pub async fn reconciliation_candidates(&self) -> AppResult<Vec<String>> {
        self.repository.reconciliation_candidates().await
    }

    pub async fn reconcile(&self, cancel: CancellationToken) -> AppResult<usize> {
        let candidates = tokio::select! {
            biased;
            () = cancel.cancelled() => return Ok(0),
            candidates = self.reconciliation_candidates() => candidates?,
        };
        let mut finalized = 0_usize;
        for session_id in candidates {
            if cancel.is_cancelled() {
                break;
            }
            if self.finalize_session(&session_id).await?.is_some() {
                finalized = finalized.saturating_add(1);
            }
        }
        Ok(finalized)
    }

    pub async fn aggregate(&self) -> AppResult<Option<SessionEfficiencyAggregate>> {
        self.aggregate_at(now_epoch_ms()).await
    }

    async fn aggregate_at(&self, now_ms: i64) -> AppResult<Option<SessionEfficiencyAggregate>> {
        let rows = self.repository.all_rows().await?;
        Ok(aggregate_rows(&rows, now_ms))
    }

    fn emit_computed(&self, finalized: &FinalizedSessionEfficiency) {
        let stats = &finalized.stats;
        let input = i128::from(stats.tool_input_token_estimate.max(0));
        let output = i128::from(stats.tool_output_token_estimate.max(0));
        let screenshot = i128::from(stats.screenshot_baseline_token_estimate.max(0));
        self.analytics.capture(
            events::AGENT_SESSION_EFFICIENCY_COMPUTED,
            json!({
                "kind": finalized.end_kind,
                "client_name": finalized.client_name.as_deref().unwrap_or_default(),
                "dispatch_count": js_safe_nonnegative(i128::from(stats.dispatch_count)),
                "active_duration_ms": js_safe_nonnegative(i128::from(stats.active_duration_ms)),
                "tool_input_token_estimate": js_safe_nonnegative(input),
                "tool_output_token_estimate": js_safe_nonnegative(output),
                "browserclaw_token_estimate": js_safe_nonnegative(input.saturating_add(output)),
                "screenshot_baseline_token_estimate": js_safe_nonnegative(screenshot),
                "screenshot_first_token_estimate": js_safe_nonnegative(input.saturating_add(screenshot)),
                "raw_token_savings_estimate": js_safe_signed(screenshot.saturating_sub(output)),
                "efficiency_estimator_version": EFFICIENCY_ESTIMATOR_VERSION,
                "screenshot_baseline_width": SCREENSHOT_BASELINE_WIDTH,
                "screenshot_baseline_height": SCREENSHOT_BASELINE_HEIGHT,
                "screenshot_tokens_per_dispatch": screenshot_tokens_per_dispatch(),
            }),
        );
    }
}

fn calculate_session_efficiency(
    source: &SessionEfficiencySource,
    computed_at: i64,
) -> Option<session_efficiency_stats::Model> {
    if source.dispatches.is_empty()
        || source
            .dispatches
            .iter()
            .any(|dispatch| dispatch.token_estimator_version != ELIGIBLE_TOKEN_ESTIMATOR_VERSION)
    {
        return None;
    }

    let mut tool_input_token_estimate = 0_i64;
    let mut tool_output_token_estimate = 0_i64;
    let mut earliest_start = i64::MAX;
    let mut latest_completion = i64::MIN;
    for dispatch in &source.dispatches {
        tool_input_token_estimate =
            tool_input_token_estimate.saturating_add(dispatch.tool_input_token_estimate.max(0));
        tool_output_token_estimate =
            tool_output_token_estimate.saturating_add(dispatch.tool_output_token_estimate.max(0));
        let safe_duration = dispatch.duration_ms.unwrap_or_default().max(0);
        earliest_start = earliest_start.min(dispatch.created_at.saturating_sub(safe_duration));
        latest_completion = latest_completion.max(dispatch.created_at);
    }

    let dispatch_count = i64::try_from(source.dispatches.len()).unwrap_or(i64::MAX);
    Some(session_efficiency_stats::Model {
        session_id: source.session_id.clone(),
        ended_at: source.end.created_at,
        dispatch_count,
        active_duration_ms: latest_completion.saturating_sub(earliest_start).max(0),
        tool_input_token_estimate,
        tool_output_token_estimate,
        screenshot_baseline_token_estimate: dispatch_count
            .saturating_mul(screenshot_tokens_per_dispatch()),
        efficiency_estimator_version: EFFICIENCY_ESTIMATOR_VERSION,
        computed_at,
    })
}

fn aggregate_rows(
    rows: &[session_efficiency_stats::Model],
    now_ms: i64,
) -> Option<SessionEfficiencyAggregate> {
    if rows.is_empty() {
        return None;
    }

    let last_30_days_lower_bound = now_ms.saturating_sub(30 * DAY_MS);
    let last_7_days_lower_bound = now_ms.saturating_sub(7 * DAY_MS);
    let mut all_time = WindowAccumulator::default();
    let mut last_30_days = WindowAccumulator::default();
    let mut last_7_days = WindowAccumulator::default();
    for row in rows {
        all_time.add(row);
        if row.ended_at >= last_30_days_lower_bound {
            last_30_days.add(row);
        }
        if row.ended_at >= last_7_days_lower_bound {
            last_7_days.add(row);
        }
    }

    Some(SessionEfficiencyAggregate {
        all_time: all_time.finish(),
        last_30_days: last_30_days.finish(),
        last_7_days: last_7_days.finish(),
    })
}

// Wider signed accumulation preserves cancellation and the sign of raw savings until the API clamp.
#[derive(Default)]
struct WindowAccumulator {
    browser_claw_token_estimate: i128,
    screenshot_first_token_estimate: i128,
    raw_token_savings_estimate: i128,
    human_time_saved_ms: i128,
    session_count: i128,
    tool_call_count: i128,
}

impl WindowAccumulator {
    fn add(&mut self, row: &session_efficiency_stats::Model) {
        let input = i128::from(row.tool_input_token_estimate.max(0));
        let output = i128::from(row.tool_output_token_estimate.max(0));
        let screenshot = i128::from(row.screenshot_baseline_token_estimate.max(0));
        self.browser_claw_token_estimate = self
            .browser_claw_token_estimate
            .saturating_add(input.saturating_add(output));
        self.screenshot_first_token_estimate = self
            .screenshot_first_token_estimate
            .saturating_add(input.saturating_add(screenshot));
        self.raw_token_savings_estimate = self
            .raw_token_savings_estimate
            .saturating_add(screenshot.saturating_sub(output));
        self.human_time_saved_ms = self
            .human_time_saved_ms
            .saturating_add(i128::from(row.active_duration_ms.max(0)));
        self.session_count = self.session_count.saturating_add(1);
        self.tool_call_count = self
            .tool_call_count
            .saturating_add(i128::from(row.dispatch_count.max(0)));
    }

    fn finish(self) -> SessionEfficiencyWindow {
        SessionEfficiencyWindow {
            browser_claw_token_estimate: js_safe_nonnegative(self.browser_claw_token_estimate),
            screenshot_first_token_estimate: js_safe_nonnegative(
                self.screenshot_first_token_estimate,
            ),
            raw_token_savings_estimate: js_safe_signed(self.raw_token_savings_estimate),
            human_time_saved_ms: js_safe_nonnegative(self.human_time_saved_ms),
            session_count: js_safe_nonnegative(self.session_count),
            tool_call_count: js_safe_nonnegative(self.tool_call_count),
        }
    }
}

fn js_safe_nonnegative(value: i128) -> i64 {
    i64::try_from(value.clamp(0, i128::from(JS_SAFE_INTEGER))).unwrap_or(JS_SAFE_INTEGER)
}

fn js_safe_signed(value: i128) -> i64 {
    let max = i128::from(JS_SAFE_INTEGER);
    i64::try_from(value.clamp(-max, max)).unwrap_or_else(|_| {
        if value.is_negative() {
            -JS_SAFE_INTEGER
        } else {
            JS_SAFE_INTEGER
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        EFFICIENCY_ESTIMATOR_VERSION, SCREENSHOT_BASELINE_HEIGHT, SCREENSHOT_BASELINE_WIDTH,
        SessionEfficiencyService, calculate_session_efficiency, screenshot_tokens_per_dispatch,
    };
    use crate::{
        analytics::{
            AnalyticsSink,
            events::{self, EventDefinition},
        },
        db::{
            AuditLog, DATABASE_FILENAME, Database, SessionEfficiencyStatsRepository,
            audit_log::{DispatchResultSummary, RecordToolDispatchInput},
            entities::{
                agent_session_ends, agent_session_starts, session_efficiency_stats, tool_dispatches,
            },
            session_efficiency_stats::SessionEfficiencySource,
        },
        ids::DispatchId,
    };
    use serde_json::{Value, json};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tempfile::{TempDir, tempdir};
    use tokio_util::sync::CancellationToken;

    const DAY_MS: i64 = 24 * 60 * 60 * 1000;
    const JS_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

    #[derive(Default)]
    struct RecordingAnalytics {
        events: Mutex<Vec<(&'static str, Option<Value>)>>,
    }

    impl AnalyticsSink for RecordingAnalytics {
        fn capture(&self, event: EventDefinition, properties: Value) {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((event.name(), event.sanitize(&properties)));
        }
    }

    impl RecordingAnalytics {
        fn events(&self) -> Vec<(&'static str, Option<Value>)> {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    struct CancelOnFirstCapture {
        cancel: CancellationToken,
        captures: AtomicUsize,
    }

    impl AnalyticsSink for CancelOnFirstCapture {
        fn capture(&self, _event: EventDefinition, _properties: Value) {
            if self.captures.fetch_add(1, Ordering::SeqCst) == 0 {
                self.cancel.cancel();
            }
        }
    }

    fn source(dispatches: Vec<tool_dispatches::Model>) -> SessionEfficiencySource {
        SessionEfficiencySource {
            session_id: "session".to_owned(),
            start: Some(agent_session_starts::Model {
                id: 1,
                created_at: 0,
                session_id: "session".to_owned(),
                agent_id: "agent".to_owned(),
                slug: "agent".to_owned(),
                agent_label: "Agent".to_owned(),
                client_name: "claude".to_owned(),
                client_version: "1".to_owned(),
            }),
            end: agent_session_ends::Model {
                id: 1,
                created_at: 10_000,
                session_id: "session".to_owned(),
                kind: "closed".to_owned(),
                reason: None,
            },
            dispatches,
        }
    }

    fn dispatch(
        id: i64,
        completed_at: i64,
        duration_ms: Option<i64>,
        input_tokens: i64,
        output_tokens: i64,
        estimator_version: i64,
    ) -> tool_dispatches::Model {
        tool_dispatches::Model {
            id,
            created_at: completed_at,
            agent_id: "agent".to_owned(),
            slug: "agent".to_owned(),
            agent_label: "Agent".to_owned(),
            session_id: "session".to_owned(),
            tool_name: if id == 1 { "name_session" } else { "navigate" }.to_owned(),
            page_id: None,
            tab_id: None,
            target_id: None,
            url: None,
            title: None,
            args_json: None,
            result_meta: Some(
                if id == 1 {
                    r#"{"isError":true,"cancelled":true}"#
                } else {
                    r#"{"isError":false,"cancelled":false}"#
                }
                .to_owned(),
            ),
            duration_ms,
            tool_input_token_estimate: input_tokens,
            tool_output_token_estimate: output_tokens,
            token_estimator_version: estimator_version,
            dispatch_id: None,
            has_screenshot: false,
        }
    }

    #[test]
    fn calculator_counts_every_v1_dispatch_and_uses_the_active_span() -> anyhow::Result<()> {
        let source = source(vec![
            dispatch(1, 1_000, Some(500), 10, 100, 1),
            dispatch(2, 1_200, Some(400), 20, 200, 1),
        ]);

        let stats = calculate_session_efficiency(&source, 12_000)
            .ok_or_else(|| anyhow::anyhow!("eligible session was skipped"))?;
        assert_eq!(stats.session_id, "session");
        assert_eq!(stats.ended_at, 10_000);
        assert_eq!(stats.dispatch_count, 2);
        assert_eq!(stats.active_duration_ms, 700);
        assert_eq!(stats.tool_input_token_estimate, 30);
        assert_eq!(stats.tool_output_token_estimate, 300);
        assert_eq!(stats.screenshot_baseline_token_estimate, 6_000);
        assert_eq!(stats.efficiency_estimator_version, 2);
        assert_eq!(stats.computed_at, 12_000);
        Ok(())
    }

    #[test]
    fn calculator_treats_missing_and_negative_durations_as_zero_and_saturates() -> anyhow::Result<()>
    {
        let source = source(vec![
            dispatch(1, 100, None, i64::MAX, i64::MAX, 1),
            dispatch(2, 200, Some(-10), 1, 1, 1),
        ]);

        let stats = calculate_session_efficiency(&source, 300)
            .ok_or_else(|| anyhow::anyhow!("eligible session was skipped"))?;
        assert_eq!(stats.active_duration_ms, 100);
        assert_eq!(stats.tool_input_token_estimate, i64::MAX);
        assert_eq!(stats.tool_output_token_estimate, i64::MAX);
        Ok(())
    }

    #[test]
    fn calculator_rejects_dispatchless_and_non_v1_sessions() {
        assert!(calculate_session_efficiency(&source(Vec::new()), 1).is_none());
        assert!(
            calculate_session_efficiency(
                &source(vec![
                    dispatch(1, 1, Some(1), 1, 1, 1),
                    dispatch(2, 2, Some(1), 0, 0, 0),
                ]),
                1,
            )
            .is_none()
        );
    }

    async fn test_services() -> anyhow::Result<(
        TempDir,
        AuditLog,
        SessionEfficiencyService,
        SessionEfficiencyStatsRepository,
    )> {
        let dir = tempdir()?;
        let db = Database::open(dir.path().join(DATABASE_FILENAME)).await?;
        Ok((
            dir,
            AuditLog::new(db.clone()),
            SessionEfficiencyService::new(db.clone()),
            SessionEfficiencyStatsRepository::new(db),
        ))
    }

    fn dispatch_input(
        session_id: &str,
        tool_name: &str,
        input_tokens: i64,
        output_tokens: i64,
        version: i64,
        is_error: bool,
        cancelled: bool,
    ) -> RecordToolDispatchInput {
        RecordToolDispatchInput {
            agent_id: "agent".to_owned(),
            slug: "agent".to_owned(),
            agent_label: "Agent".to_owned(),
            session_id: session_id.to_owned(),
            tool_name: tool_name.to_owned(),
            page_id: None,
            tab_id: None,
            target_id: None,
            url: None,
            title: None,
            raw_args: json!({}),
            duration_ms: 10,
            dispatch_id: DispatchId::new(),
            tool_input_token_estimate: input_tokens,
            tool_output_token_estimate: output_tokens,
            token_estimator_version: version,
            result: DispatchResultSummary {
                is_error,
                cancelled,
                structured_content: json!({}),
                content: json!([]),
            },
        }
    }

    async fn start(audit: &AuditLog, session_id: &str) -> anyhow::Result<()> {
        audit
            .record_session_start(session_id, "agent", "agent", "Agent", "claude", "1")
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn finalizing_an_eligible_session_inserts_exactly_once() -> anyhow::Result<()> {
        let (_dir, audit, service, repository) = test_services().await?;
        start(&audit, "eligible").await?;
        audit
            .record_tool_dispatch(dispatch_input(
                "eligible",
                "name_session",
                1,
                10,
                1,
                false,
                false,
            ))
            .await?;
        audit
            .record_tool_dispatch(dispatch_input(
                "eligible", "navigate", 2, 20, 1, true, false,
            ))
            .await?;
        audit
            .record_tool_dispatch(dispatch_input("eligible", "click", 3, 30, 1, true, true))
            .await?;
        audit.record_session_end("eligible", "closed", None).await?;

        let finalized = service
            .finalize_session_at("eligible", 50_000)
            .await?
            .ok_or_else(|| anyhow::anyhow!("first finalizer did not win the insert"))?;
        assert_eq!(finalized.end_kind, "closed");
        assert_eq!(finalized.client_name.as_deref(), Some("claude"));
        assert_eq!(finalized.stats.dispatch_count, 3);
        assert_eq!(finalized.stats.tool_input_token_estimate, 6);
        assert_eq!(finalized.stats.tool_output_token_estimate, 60);
        assert_eq!(
            finalized.stats.screenshot_baseline_token_estimate,
            3 * 3_000
        );
        assert_eq!(finalized.stats.computed_at, 50_000);
        assert_eq!(repository.find("eligible").await?, Some(finalized.stats));
        assert!(
            service
                .finalize_session_at("eligible", 60_000)
                .await?
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn finalization_skips_unused_unmeasured_and_live_sessions() -> anyhow::Result<()> {
        let (_dir, audit, service, repository) = test_services().await?;
        start(&audit, "unused").await?;
        audit.record_session_end("unused", "closed", None).await?;

        start(&audit, "unmeasured").await?;
        audit
            .record_tool_dispatch(dispatch_input(
                "unmeasured",
                "navigate",
                1,
                1,
                1,
                false,
                false,
            ))
            .await?;
        audit
            .record_tool_dispatch(dispatch_input(
                "unmeasured",
                "navigate",
                0,
                0,
                0,
                false,
                false,
            ))
            .await?;
        audit
            .record_session_end("unmeasured", "errored", None)
            .await?;

        start(&audit, "live").await?;
        audit
            .record_tool_dispatch(dispatch_input("live", "navigate", 1, 1, 1, false, false))
            .await?;

        for session_id in ["unused", "unmeasured", "live"] {
            assert!(service.finalize_session_at(session_id, 1).await?.is_none());
            assert!(repository.find(session_id).await?.is_none());
        }
        Ok(())
    }

    #[tokio::test]
    async fn racing_finalizers_report_one_insert_winner() -> anyhow::Result<()> {
        let (_dir, audit, service, repository) = test_services().await?;
        start(&audit, "race").await?;
        audit
            .record_tool_dispatch(dispatch_input("race", "navigate", 1, 1, 1, false, false))
            .await?;
        audit.record_session_end("race", "closed", None).await?;

        let (left, right) = tokio::join!(
            service.finalize_session_at("race", 1),
            service.finalize_session_at("race", 2),
        );
        let winners = [left?, right?].into_iter().filter(Option::is_some).count();
        assert_eq!(winners, 1);
        assert!(repository.find("race").await?.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn only_the_winning_finalizer_emits_the_bounded_efficiency_event() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let db = Database::open(dir.path().join(DATABASE_FILENAME)).await?;
        let audit = AuditLog::new(db.clone());
        let analytics = Arc::new(RecordingAnalytics::default());
        let service = SessionEfficiencyService::new_with_analytics(db, analytics.clone());
        audit
            .record_session_start("negative", "agent", "agent", "Agent", "Claude Code", "1")
            .await?;
        audit
            .record_tool_dispatch(dispatch_input(
                "negative", "navigate", 30, 4_000, 1, true, false,
            ))
            .await?;
        audit
            .record_session_end("negative", "errored", None)
            .await?;

        let (left, right) = tokio::join!(
            service.finalize_session_at("negative", 1),
            service.finalize_session_at("negative", 2),
        );
        assert_eq!(
            [left?, right?].into_iter().filter(Option::is_some).count(),
            1
        );
        assert_eq!(
            analytics.events(),
            vec![(
                events::AGENT_SESSION_EFFICIENCY_COMPUTED.name(),
                Some(json!({
                    "kind": "errored",
                    "client_name": "claude-code",
                    "dispatch_count": 1,
                    "active_duration_ms": 10,
                    "tool_input_token_estimate": 30,
                    "tool_output_token_estimate": 4_000,
                    "browserclaw_token_estimate": 4_030,
                    "screenshot_baseline_token_estimate": 3_000,
                    "screenshot_first_token_estimate": 3_030,
                    "raw_token_savings_estimate": -1_000,
                    "efficiency_estimator_version": 2,
                    "screenshot_baseline_width": 1_920,
                    "screenshot_baseline_height": 1_080,
                    "screenshot_tokens_per_dispatch": 3_000,
                })),
            )]
        );
        Ok(())
    }

    #[tokio::test]
    async fn tracked_finalizers_drain_without_emitting_for_unused_sessions() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let db = Database::open(dir.path().join(DATABASE_FILENAME)).await?;
        let audit = AuditLog::new(db.clone());
        let repository = SessionEfficiencyStatsRepository::new(db.clone());
        let analytics = Arc::new(RecordingAnalytics::default());
        let service = Arc::new(SessionEfficiencyService::new_with_analytics(
            db,
            analytics.clone(),
        ));
        start(&audit, "queued").await?;
        audit
            .record_tool_dispatch(dispatch_input("queued", "navigate", 1, 2, 1, false, false))
            .await?;
        audit.record_session_end("queued", "closed", None).await?;
        start(&audit, "unused").await?;
        audit.record_session_end("unused", "closed", None).await?;

        assert!(service.queue_finalize("queued".to_owned()));
        assert!(service.queue_finalize("unused".to_owned()));
        service.drain().await;

        assert!(repository.find("queued").await?.is_some());
        assert!(repository.find("unused").await?.is_none());
        assert_eq!(analytics.events().len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn completion_notification_after_drain_is_refused_and_remains_reconcilable()
    -> anyhow::Result<()> {
        let dir = tempdir()?;
        let db = Database::open(dir.path().join(DATABASE_FILENAME)).await?;
        let audit = AuditLog::new(db.clone());
        let repository = SessionEfficiencyStatsRepository::new(db.clone());
        let service = Arc::new(SessionEfficiencyService::new(db));
        start(&audit, "late-completion").await?;
        audit
            .record_tool_dispatch(dispatch_input(
                "late-completion",
                "navigate",
                1,
                2,
                1,
                false,
                false,
            ))
            .await?;
        audit
            .record_session_end("late-completion", "closed", None)
            .await?;

        service.drain().await;
        assert!(!service.queue_finalize("late-completion".to_owned()));

        assert!(repository.find("late-completion").await?.is_none());
        assert_eq!(
            service.reconciliation_candidates().await?,
            ["late-completion"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_reconciliation_resumes_from_the_idempotent_insert_boundary()
    -> anyhow::Result<()> {
        let dir = tempdir()?;
        let db = Database::open(dir.path().join(DATABASE_FILENAME)).await?;
        let audit = AuditLog::new(db.clone());
        let repository = SessionEfficiencyStatsRepository::new(db.clone());
        let analytics = Arc::new(RecordingAnalytics::default());
        let service = SessionEfficiencyService::new_with_analytics(db, analytics.clone());
        for session_id in ["first", "second"] {
            start(&audit, session_id).await?;
            audit
                .record_tool_dispatch(dispatch_input(
                    session_id, "navigate", 1, 2, 1, false, false,
                ))
                .await?;
            audit.record_session_end(session_id, "closed", None).await?;
        }

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert_eq!(service.reconcile(cancelled).await?, 0);
        assert!(repository.find("first").await?.is_none());
        assert!(repository.find("second").await?.is_none());

        assert_eq!(service.reconcile(CancellationToken::new()).await?, 2);
        assert!(repository.find("first").await?.is_some());
        assert!(repository.find("second").await?.is_some());
        assert_eq!(analytics.events().len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn reconciliation_cancellation_stops_after_partial_progress_and_resumes()
    -> anyhow::Result<()> {
        let dir = tempdir()?;
        let db = Database::open(dir.path().join(DATABASE_FILENAME)).await?;
        let audit = AuditLog::new(db.clone());
        let repository = SessionEfficiencyStatsRepository::new(db.clone());
        let cancel = CancellationToken::new();
        let analytics = Arc::new(CancelOnFirstCapture {
            cancel: cancel.clone(),
            captures: AtomicUsize::new(0),
        });
        let service = SessionEfficiencyService::new_with_analytics(db, analytics.clone());
        for session_id in ["first", "second", "third"] {
            start(&audit, session_id).await?;
            audit
                .record_tool_dispatch(dispatch_input(
                    session_id, "navigate", 1, 2, 1, false, false,
                ))
                .await?;
            audit.record_session_end(session_id, "closed", None).await?;
        }

        assert_eq!(service.reconcile(cancel).await?, 1);
        assert!(repository.find("first").await?.is_some());
        assert!(repository.find("second").await?.is_none());
        assert!(repository.find("third").await?.is_none());
        assert_eq!(
            service.reconciliation_candidates().await?,
            ["second", "third"]
        );

        assert_eq!(service.reconcile(CancellationToken::new()).await?, 2);
        assert!(service.reconciliation_candidates().await?.is_empty());
        assert_eq!(analytics.captures.load(Ordering::SeqCst), 3);
        Ok(())
    }

    #[tokio::test]
    async fn reconciliation_and_live_finalization_share_one_insert_and_event() -> anyhow::Result<()>
    {
        let dir = tempdir()?;
        let db = Database::open(dir.path().join(DATABASE_FILENAME)).await?;
        let audit = AuditLog::new(db.clone());
        let repository = SessionEfficiencyStatsRepository::new(db.clone());
        let analytics = Arc::new(RecordingAnalytics::default());
        let service = SessionEfficiencyService::new_with_analytics(db, analytics.clone());
        start(&audit, "reconcile-race").await?;
        audit
            .record_tool_dispatch(dispatch_input(
                "reconcile-race",
                "navigate",
                1,
                2,
                1,
                false,
                false,
            ))
            .await?;
        audit
            .record_session_end("reconcile-race", "closed", None)
            .await?;

        let (live, reconciled) = tokio::join!(
            service.finalize_session("reconcile-race"),
            service.reconcile(CancellationToken::new()),
        );
        let winners = usize::from(live?.is_some()).saturating_add(reconciled?);
        assert_eq!(winners, 1);
        assert!(repository.find("reconcile-race").await?.is_some());
        assert_eq!(analytics.events().len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn reconciliation_returns_only_eligible_missing_projections() -> anyhow::Result<()> {
        let (_dir, audit, service, _repository) = test_services().await?;
        for session_id in ["candidate", "projected", "unmeasured", "unused", "live"] {
            start(&audit, session_id).await?;
        }
        for session_id in ["candidate", "projected", "live"] {
            audit
                .record_tool_dispatch(dispatch_input(
                    session_id, "navigate", 1, 1, 1, false, false,
                ))
                .await?;
        }
        audit
            .record_tool_dispatch(dispatch_input(
                "unmeasured",
                "navigate",
                1,
                1,
                1,
                false,
                false,
            ))
            .await?;
        audit
            .record_tool_dispatch(dispatch_input(
                "unmeasured",
                "navigate",
                0,
                0,
                0,
                false,
                false,
            ))
            .await?;
        for session_id in ["candidate", "projected", "unmeasured", "unused"] {
            audit.record_session_end(session_id, "closed", None).await?;
        }
        assert!(service.finalize_session_at("projected", 1).await?.is_some());

        assert_eq!(service.reconciliation_candidates().await?, ["candidate"]);
        Ok(())
    }

    fn stats_row(
        session_id: &str,
        ended_at: i64,
        dispatch_count: i64,
        active_duration_ms: i64,
        input_tokens: i64,
        output_tokens: i64,
        screenshot_tokens: i64,
    ) -> session_efficiency_stats::Model {
        session_efficiency_stats::Model {
            session_id: session_id.to_owned(),
            ended_at,
            dispatch_count,
            active_duration_ms,
            tool_input_token_estimate: input_tokens,
            tool_output_token_estimate: output_tokens,
            screenshot_baseline_token_estimate: screenshot_tokens,
            efficiency_estimator_version: EFFICIENCY_ESTIMATOR_VERSION,
            computed_at: ended_at,
        }
    }

    #[tokio::test]
    async fn aggregation_uses_inclusive_windows_and_sums_session_spans() -> anyhow::Result<()> {
        let (_dir, _audit, service, repository) = test_services().await?;
        let now = 40 * DAY_MS;
        for row in [
            stats_row("old", now - 31 * DAY_MS, 1, 100, 10, 20, 30),
            stats_row("month", now - 30 * DAY_MS, 2, 200, 20, 50, 40),
            stats_row("week", now - 7 * DAY_MS, 3, 300, 30, 10, 60),
        ] {
            assert!(repository.insert_if_absent(&row).await?);
        }

        let aggregate = service
            .aggregate_at(now)
            .await?
            .ok_or_else(|| anyhow::anyhow!("aggregate missing"))?;
        assert_eq!(aggregate.all_time.browser_claw_token_estimate, 140);
        assert_eq!(aggregate.all_time.screenshot_first_token_estimate, 190);
        assert_eq!(aggregate.all_time.raw_token_savings_estimate, 50);
        assert_eq!(aggregate.all_time.human_time_saved_ms, 600);
        assert_eq!(aggregate.all_time.session_count, 3);
        assert_eq!(aggregate.all_time.tool_call_count, 6);

        assert_eq!(aggregate.last_30_days.browser_claw_token_estimate, 110);
        assert_eq!(aggregate.last_30_days.screenshot_first_token_estimate, 150);
        assert_eq!(aggregate.last_30_days.raw_token_savings_estimate, 40);
        assert_eq!(aggregate.last_30_days.human_time_saved_ms, 500);
        assert_eq!(aggregate.last_30_days.session_count, 2);
        assert_eq!(aggregate.last_30_days.tool_call_count, 5);

        assert_eq!(aggregate.last_7_days.browser_claw_token_estimate, 40);
        assert_eq!(aggregate.last_7_days.screenshot_first_token_estimate, 90);
        assert_eq!(aggregate.last_7_days.raw_token_savings_estimate, 50);
        assert_eq!(aggregate.last_7_days.human_time_saved_ms, 300);
        assert_eq!(aggregate.last_7_days.session_count, 1);
        assert_eq!(aggregate.last_7_days.tool_call_count, 3);
        Ok(())
    }

    #[tokio::test]
    async fn aggregation_returns_no_data_and_bounds_signed_outward_values() -> anyhow::Result<()> {
        let (_dir, _audit, service, repository) = test_services().await?;
        assert!(service.aggregate_at(1).await?.is_none());

        assert!(
            repository
                .insert_if_absent(&stats_row(
                    "large",
                    1,
                    i64::MAX,
                    i64::MAX,
                    i64::MAX,
                    i64::MAX,
                    0,
                ))
                .await?
        );
        let aggregate = service
            .aggregate_at(1)
            .await?
            .ok_or_else(|| anyhow::anyhow!("aggregate missing"))?;
        assert_eq!(
            aggregate.all_time.browser_claw_token_estimate,
            JS_SAFE_INTEGER
        );
        assert_eq!(
            aggregate.all_time.screenshot_first_token_estimate,
            JS_SAFE_INTEGER
        );
        assert_eq!(
            aggregate.all_time.raw_token_savings_estimate,
            -JS_SAFE_INTEGER
        );
        assert_eq!(aggregate.all_time.human_time_saved_ms, JS_SAFE_INTEGER);
        assert_eq!(aggregate.all_time.tool_call_count, JS_SAFE_INTEGER);
        Ok(())
    }

    #[test]
    fn baseline_is_fixed_for_1080p_screenshots() {
        assert_eq!(SCREENSHOT_BASELINE_WIDTH, 1_920);
        assert_eq!(SCREENSHOT_BASELINE_HEIGHT, 1_080);
        assert_eq!(screenshot_tokens_per_dispatch(), 3_000);
    }
}
