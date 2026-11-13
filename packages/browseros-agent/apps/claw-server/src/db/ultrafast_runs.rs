//! What neo did on its own in a session, derived from the audit rows it already records. For an
//! ultrafast skill that ran without the agent: the `run_skill` dispatch, the `run` script it
//! dispatched internally, and the primitives that script recorded as its children. For ultrafast
//! browsing: each `pursue` dispatch, which keeps the pages it read and their token estimate.

use crate::db::audit_log::ToolDispatchRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UltrafastRunStats {
    pub duration_ms: i64,
    pub pages_read: i64,
    pub tokens_saved: i64,
}

/// Sums every `run_skill` call in the session that finished its flow without the agent, and every
/// `pursue` call that read at least one page. A skill's internal `run` is recorded as its own
/// top-level dispatch, so it is matched to the skill call by starting inside that call's time
/// span. Returns `None` when no such run exists.
#[must_use]
pub fn ultrafast_run_stats(dispatches: &[ToolDispatchRow]) -> Option<UltrafastRunStats> {
    let mut stats: Option<UltrafastRunStats> = None;
    for (row, (pages, tokens)) in dispatches
        .iter()
        .filter_map(|row| pursue_reads(row).map(|reads| (row, reads)))
    {
        let entry = stats.get_or_insert_with(UltrafastRunStats::default);
        entry.duration_ms = entry
            .duration_ms
            .saturating_add(row.duration_ms.unwrap_or(0).max(0));
        entry.pages_read = entry.pages_read.saturating_add(pages);
        entry.tokens_saved = entry.tokens_saved.saturating_add(tokens);
    }
    for skill in dispatches
        .iter()
        .filter(|row| row.tool_name == "run_skill" && finished_without_agent(row))
    {
        let duration = skill.duration_ms.unwrap_or(0).max(0);
        let span_end = skill.created_at.saturating_add(duration);
        let scripts = dispatches
            .iter()
            .filter(|row| {
                row.tool_name == "run"
                    && row.parent_dispatch_id.is_none()
                    && row.created_at >= skill.created_at
                    && row.created_at <= span_end
            })
            .filter_map(|row| row.dispatch_id.as_deref())
            .collect::<Vec<_>>();
        let reads = dispatches.iter().filter(|row| {
            row.tool_name == "read"
                && row
                    .parent_dispatch_id
                    .as_deref()
                    .is_some_and(|parent| scripts.contains(&parent))
        });
        let entry = stats.get_or_insert_with(UltrafastRunStats::default);
        entry.duration_ms = entry.duration_ms.saturating_add(duration);
        for read in reads {
            entry.pages_read += 1;
            entry.tokens_saved = entry
                .tokens_saved
                .saturating_add(read.tool_output_token_estimate.max(0));
        }
    }
    stats
}

/// The pages a `pursue` call read itself and their token estimate, when it read any. Calls
/// recorded before pursue kept these numbers have none, and count nothing.
pub(crate) fn pursue_reads(row: &ToolDispatchRow) -> Option<(i64, i64)> {
    if row.tool_name != "pursue" {
        return None;
    }
    let meta = serde_json::from_str::<serde_json::Value>(row.result_meta.as_deref()?).ok()?;
    let pages = meta
        .get("pagesRead")
        .and_then(serde_json::Value::as_i64)
        .filter(|pages| *pages > 0)?;
    let tokens = meta
        .get("pageTokens")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0)
        .max(0);
    Some((pages, tokens))
}

/// A finished flow returns `output` and `steps`; a refusal or a hand-back to the agent does not.
pub(crate) fn finished_without_agent(row: &ToolDispatchRow) -> bool {
    let Some(meta) = row
        .result_meta
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
    else {
        return false;
    };
    if meta.get("isError").and_then(serde_json::Value::as_bool) == Some(true)
        || meta.get("cancelled").and_then(serde_json::Value::as_bool) == Some(true)
    {
        return false;
    }
    let keys = meta
        .get("structuredKeys")
        .and_then(serde_json::Value::as_array)
        .map(|keys| {
            keys.iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    keys.contains(&"output") && keys.contains(&"steps")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DONE: &str = r#"{"cancelled":false,"contentSummary":"1 block(s)","isError":false,"structuredKeys":["durationMs","output","status","steps"]}"#;
    const SCRIPT: &str = r#"{"cancelled":false,"contentSummary":"2 block(s)","isError":false,"structuredKeys":["logs","ok","value"]}"#;
    const PRIMITIVE: &str =
        r#"{"cancelled":false,"contentSummary":"0 block(s)","isError":false,"structuredKeys":[]}"#;
    const HANDED_BACK: &str = r#"{"cancelled":false,"contentSummary":"1 block(s)","isError":false,"structuredKeys":["cause","done","effects","failed","next","status"]}"#;

    struct Spec<'a> {
        id: i64,
        at: i64,
        tool: &'a str,
        key: &'a str,
        parent: Option<&'a str>,
        duration_ms: i64,
        output_tokens: i64,
        meta: &'a str,
    }

    fn row(spec: &Spec<'_>) -> ToolDispatchRow {
        ToolDispatchRow {
            id: spec.id,
            created_at: spec.at,
            agent_id: "agent".into(),
            slug: "claude".into(),
            agent_label: "claude".into(),
            session_id: "f392a113".into(),
            tool_name: spec.tool.into(),
            page_id: None,
            tab_id: None,
            target_id: None,
            url: None,
            title: None,
            args_json: None,
            result_meta: Some(spec.meta.into()),
            duration_ms: Some(spec.duration_ms),
            tool_input_token_estimate: 5,
            tool_output_token_estimate: spec.output_tokens,
            token_estimator_version: 1,
            dispatch_id: Some(spec.key.into()),
            parent_dispatch_id: spec.parent.map(str::to_string),
            has_screenshot: false,
        }
    }

    /// The shape of a real run: nine page reads worth 61,942 tokens under one skill call.
    fn phone_deals_run(skill_meta: &str) -> Vec<ToolDispatchRow> {
        let reads = [6778, 7001, 8961, 9297, 9490, 8297, 4023, 4031, 4064];
        let mut rows = vec![
            row(&Spec {
                id: 289,
                at: 1_000,
                tool: "run_skill",
                key: "skill",
                parent: None,
                duration_ms: 15_354,
                output_tokens: 1_701,
                meta: skill_meta,
            }),
            row(&Spec {
                id: 288,
                at: 1_002,
                tool: "run",
                key: "script",
                parent: None,
                duration_ms: 15_349,
                output_tokens: 1_861,
                meta: SCRIPT,
            }),
            row(&Spec {
                id: 269,
                at: 3_000,
                tool: "pages.newPage",
                key: "page",
                parent: Some("script"),
                duration_ms: 2_000,
                output_tokens: 1,
                meta: PRIMITIVE,
            }),
        ];
        for (step, tokens) in (0_i64..).zip(reads) {
            let at = 3_100 + step * 1_000;
            rows.push(row(&Spec {
                id: 400 + step,
                at: at - 50,
                tool: "nav.goto",
                key: &format!("goto-{step}"),
                parent: Some("script"),
                duration_ms: 900,
                output_tokens: 5,
                meta: PRIMITIVE,
            }));
            rows.push(row(&Spec {
                id: 300 + step,
                at,
                tool: "read",
                key: &format!("read-{step}"),
                parent: Some("script"),
                duration_ms: 10,
                output_tokens: tokens,
                meta: PRIMITIVE,
            }));
        }
        rows
    }

    #[test]
    fn a_skill_run_without_the_agent_counts_its_page_reads() {
        assert_eq!(
            ultrafast_run_stats(&phone_deals_run(DONE)),
            Some(UltrafastRunStats {
                duration_ms: 15_354,
                pages_read: 9,
                tokens_saved: 61_942,
            })
        );
    }

    #[test]
    fn a_skill_handed_back_to_the_agent_is_not_an_ultrafast_run() {
        assert_eq!(ultrafast_run_stats(&phone_deals_run(HANDED_BACK)), None);
    }

    #[test]
    fn the_agents_own_scripts_outside_the_skill_call_are_not_counted() {
        let mut rows = phone_deals_run(DONE);
        rows.push(row(&Spec {
            id: 500,
            at: 40_000,
            tool: "run",
            key: "agent-script",
            parent: None,
            duration_ms: 3_000,
            output_tokens: 900,
            meta: SCRIPT,
        }));
        rows.push(row(&Spec {
            id: 501,
            at: 40_100,
            tool: "read",
            key: "agent-read",
            parent: Some("agent-script"),
            duration_ms: 10,
            output_tokens: 5_000,
            meta: PRIMITIVE,
        }));
        assert_eq!(
            ultrafast_run_stats(&rows).map(|stats| (stats.pages_read, stats.tokens_saved)),
            Some((9, 61_942))
        );
    }

    #[test]
    fn a_session_without_run_skill_has_no_ultrafast_stats() {
        let rows = vec![row(&Spec {
            id: 1,
            at: 0,
            tool: "run",
            key: "script",
            parent: None,
            duration_ms: 100,
            output_tokens: 10,
            meta: SCRIPT,
        })];
        assert_eq!(ultrafast_run_stats(&rows), None);
    }

    fn pursue(id: i64, at: i64, duration_ms: i64, meta: &str) -> ToolDispatchRow {
        row(&Spec {
            id,
            at,
            tool: "pursue",
            key: "pursue",
            parent: None,
            duration_ms,
            output_tokens: 120,
            meta,
        })
    }

    #[test]
    fn a_pursue_run_counts_the_pages_it_read_and_their_tokens() {
        let rows = vec![
            pursue(
                1,
                0,
                7_317,
                r#"{"isError":false,"outcome":"handed_back","pagesRead":5,"pageTokens":18204,"structuredKeys":[]}"#,
            ),
            pursue(
                2,
                9_000,
                2_000,
                r#"{"isError":false,"outcome":"completed","pagesRead":2,"pageTokens":4000,"structuredKeys":[]}"#,
            ),
        ];
        assert_eq!(
            ultrafast_run_stats(&rows),
            Some(UltrafastRunStats {
                duration_ms: 9_317,
                pages_read: 7,
                tokens_saved: 22_204,
            })
        );
    }

    #[test]
    fn a_pursue_run_without_page_counts_counts_nothing() {
        let older = pursue(
            1,
            0,
            4_000,
            r#"{"isError":false,"outcome":"completed","structuredKeys":[]}"#,
        );
        let read_nothing = pursue(
            2,
            5_000,
            30,
            r#"{"isError":false,"outcome":"handed_back","pagesRead":0,"pageTokens":0,"structuredKeys":[]}"#,
        );
        assert_eq!(ultrafast_run_stats(&[older, read_nothing]), None);
    }

    #[test]
    fn skill_and_pursue_runs_in_one_session_add_up() {
        let mut rows = phone_deals_run(DONE);
        rows.push(pursue(
            900,
            90_000,
            6_000,
            r#"{"isError":false,"outcome":"completed","pagesRead":3,"pageTokens":9000,"structuredKeys":[]}"#,
        ));
        assert_eq!(
            ultrafast_run_stats(&rows),
            Some(UltrafastRunStats {
                duration_ms: 21_354,
                pages_read: 12,
                tokens_saved: 70_942,
            })
        );
    }
}
