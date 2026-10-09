use crate::analytics::events::MAX_SAFE_INTEGER;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionUsageSnapshot {
    pub(crate) client_name: String,
    pub(crate) dispatch_count: u64,
    pub(crate) distinct_tool_count: u64,
    pub(crate) max_concurrent_used_sessions: u64,
    pub(crate) tools: Vec<ToolUsageSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolUsageSnapshot {
    pub(crate) tool_name: String,
    pub(crate) dispatch_count: u64,
    pub(crate) total_duration_ms: u64,
    pub(crate) max_duration_ms: u64,
}

pub(crate) struct UsageTracker {
    client_name: String,
    used: AtomicBool,
    state: Mutex<UsageState>,
}

#[derive(Default)]
struct UsageState {
    dispatch_count: u64,
    max_concurrent_used_sessions: u64,
    tools: BTreeMap<String, ToolUsage>,
}

#[derive(Default)]
struct ToolUsage {
    dispatch_count: u64,
    total_duration_ms: u64,
    max_duration_ms: u64,
}

impl UsageTracker {
    pub(crate) fn new(client_name: String) -> Self {
        Self {
            client_name,
            used: AtomicBool::new(false),
            state: Mutex::new(UsageState::default()),
        }
    }

    pub(crate) fn client_name(&self) -> &str {
        &self.client_name
    }

    pub(crate) fn mark_used(&self) {
        self.used.store(true, Ordering::Relaxed);
    }

    pub(crate) fn is_used(&self) -> bool {
        self.used.load(Ordering::Relaxed)
    }

    pub(crate) async fn record(
        &self,
        tool_name: &str,
        elapsed: Duration,
        concurrent_used_sessions: usize,
    ) {
        self.mark_used();
        let elapsed_ms = capped_u128(elapsed.as_millis());
        let concurrent_used_sessions = capped_u128(concurrent_used_sessions as u128);
        let mut state = self.state.lock().await;
        state.dispatch_count = capped_add(state.dispatch_count, 1);
        state.max_concurrent_used_sessions = state
            .max_concurrent_used_sessions
            .max(concurrent_used_sessions);
        let tool = state.tools.entry(tool_name.to_string()).or_default();
        tool.dispatch_count = capped_add(tool.dispatch_count, 1);
        tool.total_duration_ms = capped_add(tool.total_duration_ms, elapsed_ms);
        tool.max_duration_ms = tool.max_duration_ms.max(elapsed_ms);
    }

    pub(crate) async fn snapshot(&self) -> SessionUsageSnapshot {
        let state = self.state.lock().await;
        SessionUsageSnapshot {
            client_name: self.client_name.clone(),
            dispatch_count: state.dispatch_count,
            distinct_tool_count: capped_u128(state.tools.len() as u128),
            max_concurrent_used_sessions: state.max_concurrent_used_sessions,
            tools: state
                .tools
                .iter()
                .map(|(tool_name, usage)| ToolUsageSnapshot {
                    tool_name: tool_name.clone(),
                    dispatch_count: usage.dispatch_count,
                    total_duration_ms: usage.total_duration_ms,
                    max_duration_ms: usage.max_duration_ms,
                })
                .collect(),
        }
    }
}

fn capped_add(current: u64, increment: u64) -> u64 {
    current.saturating_add(increment).min(MAX_SAFE_INTEGER)
}

fn capped_u128(value: u128) -> u64 {
    value.min(u128::from(MAX_SAFE_INTEGER)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytics::events::MAX_SAFE_INTEGER;
    use std::time::Duration;

    #[tokio::test]
    async fn untouched_tracker_has_a_zero_use_snapshot() {
        let tracker = UsageTracker::new("Codex".to_string());

        assert!(!tracker.is_used());
        assert_eq!(
            tracker.snapshot().await,
            SessionUsageSnapshot {
                client_name: "Codex".to_string(),
                dispatch_count: 0,
                distinct_tool_count: 0,
                max_concurrent_used_sessions: 0,
                tools: Vec::new(),
            }
        );
    }

    #[tokio::test]
    async fn tracker_aggregates_calls_in_deterministic_tool_order() {
        let tracker = UsageTracker::new("Codex".to_string());

        tracker
            .record("navigate", Duration::from_millis(120), 1)
            .await;
        tracker.record("tabs", Duration::from_millis(30), 2).await;
        tracker
            .record("navigate", Duration::from_millis(80), 2)
            .await;

        assert!(tracker.is_used());
        assert_eq!(
            tracker.snapshot().await,
            SessionUsageSnapshot {
                client_name: "Codex".to_string(),
                dispatch_count: 3,
                distinct_tool_count: 2,
                max_concurrent_used_sessions: 2,
                tools: vec![
                    ToolUsageSnapshot {
                        tool_name: "navigate".to_string(),
                        dispatch_count: 2,
                        total_duration_ms: 200,
                        max_duration_ms: 120,
                    },
                    ToolUsageSnapshot {
                        tool_name: "tabs".to_string(),
                        dispatch_count: 1,
                        total_duration_ms: 30,
                        max_duration_ms: 30,
                    },
                ],
            }
        );
    }

    #[test]
    fn numeric_helpers_cap_at_the_analytics_boundary() {
        assert_eq!(capped_add(MAX_SAFE_INTEGER - 1, 10), MAX_SAFE_INTEGER);
        assert_eq!(capped_u128(u128::MAX), MAX_SAFE_INTEGER);
    }
}
