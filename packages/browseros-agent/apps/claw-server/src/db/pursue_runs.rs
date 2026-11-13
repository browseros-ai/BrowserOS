//! What neo did with `pursue` in a session, derived from the audit rows it already records. Each
//! step neo takes while pursue runs is recorded as its own top-level `act` or `navigate` dispatch,
//! so a step belongs to a pursue call when it starts inside that call's time span. Calls recorded
//! before pursue counted pages still have their outcome and steps.

use serde_json::Value;

use crate::db::audit_log::ToolDispatchRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PursueOutcome {
    Completed,
    HandedBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PursueStepKind {
    Click,
    Type,
    Press,
    Check,
    Uncheck,
    Select,
    Scroll,
    Drag,
    Dialog,
    Open,
    Back,
    Reload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PursueStep {
    pub kind: PursueStepKind,
    /// The typed text, key, chosen option or opened site, when recorded and not redacted.
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PursueRun {
    /// How the session's last pursue that finished or handed back ended.
    pub outcome: PursueOutcome,
    pub duration_ms: i64,
    pub steps: Vec<PursueStep>,
}

/// The session's pursue calls that finished their goal or handed the page back, with the steps neo
/// took during them. Returns `None` when there was no such call.
#[must_use]
pub fn pursue_run(dispatches: &[ToolDispatchRow]) -> Option<PursueRun> {
    let mut run: Option<PursueRun> = None;
    for call in dispatches.iter().filter(|row| row.tool_name == "pursue") {
        let Some(outcome) = pursue_outcome(call) else {
            continue;
        };
        let duration = call.duration_ms.unwrap_or(0).max(0);
        let span_end = call.created_at.saturating_add(duration);
        let steps = dispatches
            .iter()
            .filter(|row| {
                row.parent_dispatch_id.is_none()
                    && row.created_at >= call.created_at
                    && row.created_at <= span_end
                    && !failed(row)
            })
            .filter_map(step);
        let entry = run.get_or_insert(PursueRun {
            outcome,
            duration_ms: 0,
            steps: Vec::new(),
        });
        entry.outcome = outcome;
        entry.duration_ms = entry.duration_ms.saturating_add(duration);
        entry.steps.extend(steps);
    }
    run
}

/// The outcome a pursue call kept in its result summary, when it finished or handed back.
pub(crate) fn pursue_outcome(row: &ToolDispatchRow) -> Option<PursueOutcome> {
    if row.tool_name != "pursue" || failed(row) {
        return None;
    }
    match meta(row)?.get("outcome").and_then(Value::as_str)? {
        "completed" => Some(PursueOutcome::Completed),
        "handed_back" => Some(PursueOutcome::HandedBack),
        _ => None,
    }
}

fn meta(row: &ToolDispatchRow) -> Option<Value> {
    serde_json::from_str(row.result_meta.as_deref()?).ok()
}

fn failed(row: &ToolDispatchRow) -> bool {
    meta(row).is_some_and(|meta| {
        meta.get("isError").and_then(Value::as_bool) == Some(true)
            || meta.get("cancelled").and_then(Value::as_bool) == Some(true)
    })
}

/// The step a dispatch recorded, or `None` for one that changes nothing on the page (focus, hover)
/// or isn't a page step at all.
fn step(row: &ToolDispatchRow) -> Option<PursueStep> {
    let args = row
        .args_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or(Value::Null);
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "[redacted]")
            .map(str::to_string)
    };
    let (kind, text) = match row.tool_name.as_str() {
        "act" => match args.get("kind").and_then(Value::as_str)? {
            "click" | "click_at" => (PursueStepKind::Click, None),
            "type" | "type_at" | "fill" => (PursueStepKind::Type, text("value").or(text("text"))),
            "press" => (PursueStepKind::Press, text("key")),
            "check" => (PursueStepKind::Check, None),
            "uncheck" => (PursueStepKind::Uncheck, None),
            "select" => (PursueStepKind::Select, text("value")),
            "scroll" => (PursueStepKind::Scroll, None),
            "drag" | "drag_at" => (PursueStepKind::Drag, None),
            "dialog_accept" | "dialog_dismiss" => (PursueStepKind::Dialog, None),
            _ => return None,
        },
        "navigate" => match args.get("action").and_then(Value::as_str) {
            Some("back") => (PursueStepKind::Back, None),
            Some("reload") => (PursueStepKind::Reload, None),
            _ => (
                PursueStepKind::Open,
                text("url").map(|url| site(&url).unwrap_or(url)),
            ),
        },
        _ => return None,
    };
    Some(PursueStep { kind, text })
}

fn site(url: &str) -> Option<String> {
    let host = url::Url::parse(url).ok()?.host_str()?.to_string();
    Some(
        host.strip_prefix("www.")
            .map_or(host.clone(), str::to_string),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const HANDED_BACK: &str = r#"{"cancelled":false,"contentSummary":"1 block(s)","isError":false,"outcome":"handed_back","structuredKeys":["actions","continueWith","decisions","outcome","page","remaining","status","story","url"]}"#;
    const COMPLETED: &str = r#"{"cancelled":false,"contentSummary":"1 block(s)","isError":false,"outcome":"completed","structuredKeys":["actions","decisions","outcome","page","status","story","url"]}"#;
    const ACTED: &str = r#"{"cancelled":false,"contentSummary":"1 block(s)","isError":false,"structuredKeys":["kind"]}"#;

    fn row(
        id: i64,
        at: i64,
        tool: &str,
        args: &str,
        meta: &str,
        duration_ms: i64,
    ) -> ToolDispatchRow {
        ToolDispatchRow {
            id,
            created_at: at,
            agent_id: "agent".into(),
            slug: "claude".into(),
            agent_label: "claude".into(),
            session_id: "2ca8fbb6".into(),
            tool_name: tool.into(),
            page_id: None,
            tab_id: None,
            target_id: None,
            url: None,
            title: None,
            args_json: Some(args.into()),
            result_meta: Some(meta.into()),
            duration_ms: Some(duration_ms),
            tool_input_token_estimate: 5,
            tool_output_token_estimate: 10,
            token_estimator_version: 1,
            dispatch_id: Some(format!("d{id}")),
            parent_dispatch_id: None,
            has_screenshot: false,
        }
    }

    fn act(id: i64, at: i64, args: &str) -> ToolDispatchRow {
        row(id, at, "act", args, ACTED, 200)
    }

    /// The recorded Google Flights run: a pursue that handed back after two clicks and a fill,
    /// each preceded by a focus, with the agent's own scripts after it.
    fn flights() -> Vec<ToolDispatchRow> {
        vec![
            row(1, 1_000, "tabs", "{}", ACTED, 50),
            row(
                2,
                1_100,
                "pursue",
                r#"{"goal":"Search flights","page":9}"#,
                HANDED_BACK,
                5_662,
            ),
            act(3, 2_100, r#"{"kind":"focus","page":9,"ref":"e12"}"#),
            act(
                4,
                2_300,
                r#"{"diff":"none","kind":"click","page":9,"ref":"e12"}"#,
            ),
            act(5, 2_600, r#"{"kind":"focus","page":9,"ref":"e81"}"#),
            act(
                6,
                2_800,
                r#"{"diff":"none","kind":"click","page":9,"ref":"e81"}"#,
            ),
            act(7, 4_000, r#"{"kind":"focus","page":9,"ref":"e17"}"#),
            act(
                8,
                4_800,
                r#"{"diff":"none","kind":"fill","page":9,"ref":"e17","value":"Singapore"}"#,
            ),
            row(9, 9_000, "run", r#"{"code":"..."}"#, ACTED, 900),
            act(10, 9_500, r#"{"kind":"click","page":9,"ref":"e40"}"#),
        ]
    }

    #[test]
    fn a_handed_back_pursue_lists_the_steps_neo_took_inside_it() {
        let Some(run) = pursue_run(&flights()) else {
            panic!("a pursue run");
        };
        assert_eq!(run.outcome, PursueOutcome::HandedBack);
        assert_eq!(run.duration_ms, 5_662);
        assert_eq!(
            run.steps,
            vec![
                PursueStep {
                    kind: PursueStepKind::Click,
                    text: None
                },
                PursueStep {
                    kind: PursueStepKind::Click,
                    text: None
                },
                PursueStep {
                    kind: PursueStepKind::Type,
                    text: Some("Singapore".into())
                },
            ]
        );
    }

    #[test]
    fn a_finished_pursue_without_page_counts_still_has_its_steps() {
        let rows = vec![
            row(
                1,
                1_000,
                "pursue",
                r#"{"goal":"Search amazon.in","page":3}"#,
                COMPLETED,
                7_317,
            ),
            act(2, 1_500, r#"{"kind":"focus","page":3,"ref":"e57"}"#),
            act(
                3,
                2_000,
                r#"{"diff":"none","kind":"fill","page":3,"ref":"e57","value":"colgate toothpaste"}"#,
            ),
            act(4, 3_000, r#"{"kind":"focus","page":3,"ref":"e57"}"#),
            act(
                5,
                3_500,
                r#"{"diff":"none","key":"Enter","kind":"press","page":3,"ref":"e57"}"#,
            ),
        ];
        let Some(run) = pursue_run(&rows) else {
            panic!("a pursue run");
        };
        assert_eq!(run.outcome, PursueOutcome::Completed);
        assert_eq!(run.duration_ms, 7_317);
        assert_eq!(
            run.steps,
            vec![
                PursueStep {
                    kind: PursueStepKind::Type,
                    text: Some("colgate toothpaste".into())
                },
                PursueStep {
                    kind: PursueStepKind::Press,
                    text: Some("Enter".into())
                },
            ]
        );
    }

    #[test]
    fn redacted_values_failed_steps_and_navigation_are_worded_from_what_was_recorded() {
        let rows = vec![
            row(1, 1_000, "pursue", "{}", COMPLETED, 5_000),
            row(
                2,
                1_200,
                "navigate",
                r#"{"page":3,"url":"https://www.example.com/login"}"#,
                ACTED,
                300,
            ),
            act(
                3,
                2_000,
                r#"{"kind":"fill","page":3,"ref":"e2","value":"[redacted]"}"#,
            ),
            row(
                4,
                2_500,
                "act",
                r#"{"kind":"click","page":3,"ref":"e9"}"#,
                r#"{"cancelled":false,"isError":true}"#,
                100,
            ),
            row(
                5,
                3_000,
                "navigate",
                r#"{"page":3,"action":"back"}"#,
                ACTED,
                300,
            ),
        ];
        let Some(run) = pursue_run(&rows) else {
            panic!("a pursue run");
        };
        assert_eq!(
            run.steps,
            vec![
                PursueStep {
                    kind: PursueStepKind::Open,
                    text: Some("example.com".into())
                },
                PursueStep {
                    kind: PursueStepKind::Type,
                    text: None
                },
                PursueStep {
                    kind: PursueStepKind::Back,
                    text: None
                },
            ]
        );
    }

    #[test]
    fn a_cancelled_or_unrecorded_pursue_is_not_a_pursue_run() {
        let cancelled = r#"{"cancelled":true,"isError":false,"outcome":"cancelled"}"#;
        let no_outcome = r#"{"cancelled":false,"isError":false,"structuredKeys":["status"]}"#;
        let rows = vec![
            row(1, 1_000, "pursue", "{}", cancelled, 2_000),
            act(2, 1_500, r#"{"kind":"click","page":3,"ref":"e1"}"#),
            row(3, 5_000, "pursue", "{}", no_outcome, 2_000),
        ];
        assert_eq!(pursue_run(&rows), None);
        assert_eq!(pursue_run(&[]), None);
    }

    #[test]
    fn several_pursue_calls_add_up_and_the_last_outcome_wins() {
        let mut rows = flights();
        rows.push(row(11, 20_000, "pursue", "{}", COMPLETED, 3_000));
        rows.push(act(
            12,
            21_000,
            r#"{"kind":"press","page":9,"key":"Enter"}"#,
        ));
        let Some(run) = pursue_run(&rows) else {
            panic!("a pursue run");
        };
        assert_eq!(run.outcome, PursueOutcome::Completed);
        assert_eq!(run.duration_ms, 8_662);
        assert_eq!(run.steps.len(), 4);
    }
}
