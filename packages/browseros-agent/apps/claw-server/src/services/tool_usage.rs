//! Daily tool call totals for this install, reported once each UTC day is over.

use crate::{
    analytics::{AnalyticsService, events::TOOL_CALLS_DAILY},
    db::{
        run_error_budget::utc_day,
        tool_calls_daily::{ToolCallCount, ToolCallsDailyRepository},
    },
    error::AppResult,
};
use serde_json::{Map, Value, json};

const NOON_MS: i64 = 12 * 60 * 60 * 1_000;

/// Counts one finished tool call toward today. Nothing is kept while analytics consent is off.
pub async fn record_tool_call(
    repo: &ToolCallsDailyRepository,
    analytics: &AnalyticsService,
    tool_name: &str,
    client_name: &str,
    failed: bool,
    now_ms: i64,
) {
    if !analytics.get_state().await.consent {
        return;
    }
    if let Err(error) = repo
        .record(&utc_day(now_ms), tool_name, client_name, failed)
        .await
    {
        tracing::warn!(%error, "could not count a tool call");
    }
}

/// Sends one event per finished UTC day, then forgets that day. When events cannot leave this
/// process (opted out, no project key, hosted), the held counts are dropped instead.
pub async fn flush_finished_days(
    repo: &ToolCallsDailyRepository,
    analytics: &AnalyticsService,
    now_ms: i64,
) -> AppResult<usize> {
    if !analytics.delivers() {
        repo.delete_all().await?;
        return Ok(0);
    }
    let mut sent = 0;
    for day in repo.days_before(&utc_day(now_ms)).await? {
        let counts = repo.counts(&day).await?;
        // The event is dated noon of its day, so it lands on that date in any project timezone
        // within twelve hours of UTC.
        if let Some(midnight_ms) = utc_midnight_ms(&day)
            && analytics.capture_once(
                TOOL_CALLS_DAILY,
                daily_properties(&day, &counts),
                midnight_ms + NOON_MS,
                &day,
            )
        {
            sent += 1;
        }
        repo.delete_day(&day).await?;
    }
    Ok(sent)
}

fn daily_properties(day: &str, counts: &[ToolCallCount]) -> Value {
    let mut by_tool = Map::new();
    let mut by_client = Map::new();
    let (mut total_calls, mut error_calls) = (0_u64, 0_u64);
    for count in counts {
        let calls = u64::try_from(count.calls).unwrap_or(0);
        total_calls = total_calls.saturating_add(calls);
        error_calls = error_calls.saturating_add(u64::try_from(count.error_calls).unwrap_or(0));
        add_count(&mut by_tool, &count.tool_name, calls);
        add_count(&mut by_client, &count.client_name, calls);
    }
    json!({
        "day": day,
        "total_calls": total_calls,
        "error_calls": error_calls,
        "distinct_tools": by_tool.len(),
        "by_tool": by_tool,
        "by_client": by_client,
    })
}

fn add_count(counts: &mut Map<String, Value>, key: &str, calls: u64) {
    let current = counts.get(key).and_then(Value::as_u64).unwrap_or(0);
    counts.insert(key.to_string(), Value::from(current.saturating_add(calls)));
}

fn utc_midnight_ms(day: &str) -> Option<i64> {
    let mut parts = day.splitn(3, '-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = time::Month::try_from(parts.next()?.parse::<u8>().ok()?).ok()?;
    let day = parts.next()?.parse::<u8>().ok()?;
    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    Some(date.midnight().assume_utc().unix_timestamp() * 1_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use axum::{Router, body::Bytes, http::StatusCode, routing::any};
    use std::time::Duration;
    use tempfile::{TempDir, tempdir};
    use tokio::{
        net::TcpListener,
        sync::mpsc::{UnboundedReceiver, unbounded_channel},
    };

    const NOV_10_NOON: i64 = 1_794_312_000_000; // 2026-11-10T12:00:00Z
    const DAY_MS: i64 = 86_400_000;

    struct Harness {
        repo: ToolCallsDailyRepository,
        analytics: AnalyticsService,
        received: UnboundedReceiver<Value>,
        _dir: TempDir,
    }

    /// A real analytics client pointed at a local PostHog stand-in that records every request.
    async fn harness(delivery_enabled: bool) -> anyhow::Result<Harness> {
        let dir = tempdir()?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let host = format!("http://{}", listener.local_addr()?);
        let (sender, received) = unbounded_channel();
        let app = Router::new().fallback(any(move |body: Bytes| {
            let sender = sender.clone();
            async move {
                if let Ok(value) = serde_json::from_slice::<Value>(&body) {
                    let _ = sender.send(value);
                }
                StatusCode::OK
            }
        }));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let analytics =
            AnalyticsService::new_for_test(dir.path(), Some("test-key"), host, delivery_enabled)
                .await?;
        let db = Database::open(dir.path().join("browserclaw.sqlite")).await?;
        Ok(Harness {
            repo: ToolCallsDailyRepository::new(db),
            analytics,
            received,
            _dir: dir,
        })
    }

    impl Harness {
        async fn call(&self, tool: &str, client: &str, failed: bool, at_ms: i64) {
            record_tool_call(&self.repo, &self.analytics, tool, client, failed, at_ms).await;
        }

        async fn next_event(&mut self) -> anyhow::Result<Value> {
            let body = tokio::time::timeout(Duration::from_secs(5), self.received.recv())
                .await?
                .ok_or_else(|| anyhow::anyhow!("collector closed"))?;
            Ok(body["batch"][0].clone())
        }

        async fn nothing_more_is_sent(&mut self) {
            assert!(
                tokio::time::timeout(Duration::from_millis(300), self.received.recv())
                    .await
                    .is_err(),
                "no further request was expected"
            );
        }
    }

    #[test]
    fn days_are_dated_at_utc_midnight() {
        assert_eq!(utc_midnight_ms("2026-11-10"), Some(NOV_10_NOON - NOON_MS));
        assert_eq!(utc_midnight_ms("2026-02-30"), None);
        assert_eq!(utc_midnight_ms("someday"), None);
    }

    #[tokio::test]
    async fn a_finished_day_is_sent_once_with_its_totals() -> anyhow::Result<()> {
        let mut harness = harness(true).await?;
        harness.call("navigate", "Codex", false, NOV_10_NOON).await;
        harness
            .call("navigate", "Codex", true, NOV_10_NOON + 1)
            .await;
        harness
            .call("tabs", "Claude Code", false, NOV_10_NOON + 2)
            .await;
        harness
            .call("tabs", "Codex", false, NOV_10_NOON + DAY_MS)
            .await;

        let sent =
            flush_finished_days(&harness.repo, &harness.analytics, NOV_10_NOON + DAY_MS).await?;
        assert_eq!(sent, 1, "only the finished day is sent");

        let body = harness.next_event().await?;
        assert_eq!(body["event"], "tool_calls_daily");
        assert_eq!(body["timestamp"], "2026-11-10T12:00:00");
        let properties = &body["properties"];
        assert_eq!(properties["day"], "2026-11-10");
        assert_eq!(properties["total_calls"], 3);
        assert_eq!(properties["error_calls"], 1);
        assert_eq!(properties["distinct_tools"], 2);
        assert_eq!(properties["by_tool"], json!({ "navigate": 2, "tabs": 1 }));
        assert_eq!(
            properties["by_client"],
            json!({ "claude-code": 1, "codex": 2 })
        );
        assert_eq!(properties["$process_person_profile"], false);
        assert!(body["uuid"].as_str().is_some());

        assert!(harness.repo.counts("2026-11-10").await?.is_empty());
        assert_eq!(harness.repo.counts("2026-11-11").await?.len(), 1);
        assert_eq!(
            flush_finished_days(&harness.repo, &harness.analytics, NOV_10_NOON + DAY_MS).await?,
            0,
            "a sent day is not sent again"
        );
        harness.nothing_more_is_sent().await;
        Ok(())
    }

    /// A crash after the send but before the day is forgotten resends the day. It carries the
    /// same id, so PostHog keeps one copy.
    #[tokio::test]
    async fn resending_a_day_reuses_its_event_id() -> anyhow::Result<()> {
        let mut harness = harness(true).await?;
        let tomorrow = NOV_10_NOON + DAY_MS;

        harness.call("tabs", "Codex", false, NOV_10_NOON).await;
        flush_finished_days(&harness.repo, &harness.analytics, tomorrow).await?;
        let first = harness.next_event().await?;

        harness.call("tabs", "Codex", false, NOV_10_NOON).await;
        flush_finished_days(&harness.repo, &harness.analytics, tomorrow).await?;
        let second = harness.next_event().await?;

        assert_eq!(first["uuid"], second["uuid"]);
        assert_eq!(first["timestamp"], second["timestamp"]);
        Ok(())
    }

    #[tokio::test]
    async fn opting_out_counts_nothing_and_drops_held_days() -> anyhow::Result<()> {
        let mut harness = harness(true).await?;
        harness.call("tabs", "Codex", false, NOV_10_NOON).await;

        harness.analytics.set_consent(false).await?;
        harness.call("tabs", "Codex", false, NOV_10_NOON + 1).await;
        assert_eq!(harness.repo.counts("2026-11-10").await?[0].calls, 1);

        let sent =
            flush_finished_days(&harness.repo, &harness.analytics, NOV_10_NOON + DAY_MS).await?;
        assert_eq!(sent, 0);
        assert!(harness.repo.counts("2026-11-10").await?.is_empty());
        harness.nothing_more_is_sent().await;
        Ok(())
    }

    #[tokio::test]
    async fn without_delivery_nothing_is_sent_and_counts_are_dropped() -> anyhow::Result<()> {
        let mut harness = harness(false).await?;
        harness.call("tabs", "Codex", false, NOV_10_NOON).await;

        let sent =
            flush_finished_days(&harness.repo, &harness.analytics, NOV_10_NOON + DAY_MS).await?;
        assert_eq!(sent, 0);
        assert!(harness.repo.days_before("2026-11-12").await?.is_empty());
        harness.nothing_more_is_sent().await;
        Ok(())
    }
}
