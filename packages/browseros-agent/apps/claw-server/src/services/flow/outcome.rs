//! Turns what the prelude returned into the answer the agent reads: done, a handover packet
//! with everything needed to carry on, or a refusal.

use serde_json::{Value, json};

/// Whether the irreversible step ran: never guessed, only reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Committed {
    No,
    Yes,
    Unknown,
}

impl Committed {
    fn from_value(value: &Value) -> Self {
        match value {
            Value::Bool(true) => Self::Yes,
            Value::String(text) if text == "unknown" || text == "in_progress" => Self::Unknown,
            _ => Self::No,
        }
    }

    fn as_json(self) -> Value {
        match self {
            Self::No => json!(false),
            Self::Yes => json!(true),
            Self::Unknown => json!("unknown"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Handover {
    pub kind: String,
    pub step: Option<u32>,
    pub data: Value,
    pub done: Value,
    pub committed: Committed,
    pub page: Value,
    pub view: Option<String>,
    pub duration_ms: Option<i64>,
}

impl Handover {
    #[must_use]
    pub fn message(&self) -> String {
        self.data
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                let mut detail = self.data.clone();
                if let Some(map) = detail.as_object_mut() {
                    map.remove("step");
                }
                if detail.as_object().is_some_and(serde_json::Map::is_empty) {
                    self.kind.clone()
                } else {
                    format!("{}: {detail}", self.kind)
                }
            })
    }

    #[must_use]
    pub fn page_id(&self) -> Option<i64> {
        self.page.get("pageId").and_then(Value::as_i64)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Done {
        output: Value,
        done: Value,
        duration_ms: Option<i64>,
    },
    /// Verify mode reached the write and proved its target, without writing.
    VerifiedToCommit {
        dry: Value,
        duration_ms: Option<i64>,
    },
    /// Triage mode reached the step it was asked to stop after.
    Reached,
    Parsed,
    NeedsAgent(Handover),
    /// The `run` call itself failed (engine error, hard timeout, refused dispatch).
    RunFailed(String),
}

/// Reads the `value` the flow script returned.
#[must_use]
pub fn parse(value: &Value) -> Outcome {
    let status = value.get("status").and_then(Value::as_str).unwrap_or("");
    let duration_ms = value.get("durationMs").and_then(Value::as_i64);
    match status {
        "done" => Outcome::Done {
            output: value.get("output").cloned().unwrap_or(Value::Null),
            done: value.get("done").cloned().unwrap_or_else(|| json!([])),
            duration_ms,
        },
        "verified_to_commit" => Outcome::VerifiedToCommit {
            dry: value.get("dry").cloned().unwrap_or_else(|| json!([])),
            duration_ms,
        },
        "reached" => Outcome::Reached,
        "parsed" => Outcome::Parsed,
        "needs_agent" => Outcome::NeedsAgent(Handover {
            kind: value
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("exception")
                .to_string(),
            step: value
                .get("step")
                .and_then(Value::as_u64)
                .and_then(|step| u32::try_from(step).ok()),
            data: value.get("data").cloned().unwrap_or_else(|| json!({})),
            done: value.get("done").cloned().unwrap_or_else(|| json!([])),
            committed: Committed::from_value(value.get("committed").unwrap_or(&Value::Null)),
            page: value.get("page").cloned().unwrap_or(Value::Null),
            view: value
                .get("view")
                .and_then(Value::as_str)
                .map(str::to_string),
            duration_ms,
        }),
        _ => Outcome::RunFailed(format!("the flow returned no status: {value}")),
    }
}

/// The one-line instruction, generated from the packet so it always agrees with `committed`.
#[must_use]
pub fn next_instruction(handover: &Handover, commit_step: Option<u32>) -> String {
    let at = handover
        .step
        .map_or_else(|| "the start".to_string(), |step| format!("step {step}"));
    let on = handover
        .page_id()
        .map_or_else(String::new, |id| format!(" on page {id}"));
    match handover.committed {
        Committed::No => {
            format!("Continue from {at}{on} with your normal tools. Nothing was written yet.")
        }
        Committed::Yes => format!(
            "The write in step {} already happened. Check the result{on}; never redo that step.",
            commit_step.unwrap_or(0)
        ),
        Committed::Unknown => format!(
            "The write in step {} may or may not have happened. Check the page{on} (for example the cart or the list) before doing anything else.",
            commit_step.unwrap_or(0)
        ),
    }
}

/// The packet the agent reads after a hand-back.
#[must_use]
pub fn packet(
    handover: &Handover,
    labels: &[String],
    commit_step: Option<u32>,
    cause: &str,
    step_source: Option<&str>,
    repair: Option<String>,
) -> Value {
    let step = handover.step;
    let label = step
        .and_then(|n| labels.get(n as usize - 1))
        .cloned()
        .unwrap_or_default();
    let remaining: Vec<Value> = labels
        .iter()
        .enumerate()
        .filter(|(index, _)| step.is_none_or(|n| *index + 1 >= n as usize))
        .map(|(index, text)| json!({ "step": index + 1, "text": text }))
        .collect();
    let mut packet = json!({
        "status": "needs_agent",
        "failed": {
            "step": step,
            "label": label,
            "kind": handover.kind,
            "message": handover.message(),
        },
        "cause": cause,
        "effects": { "committed": handover.committed.as_json() },
        "done": handover.done,
        "partial": handover.data.get("partial").cloned().unwrap_or(Value::Null),
        "page": owned_page(&handover.page),
        "view": handover.view,
        "remaining": remaining,
        "next": next_instruction(handover, commit_step),
    });
    if let Some(source) = step_source {
        packet["stepSource"] = json!(source);
    }
    if let Some(repair) = repair {
        packet["repair"] = json!(repair);
    }
    packet
}

fn owned_page(page: &Value) -> Value {
    if page.is_null() {
        return Value::Null;
    }
    json!({
        "id": page.get("pageId").cloned().unwrap_or(Value::Null),
        "url": page.get("url").cloned().unwrap_or(Value::Null),
        "title": page.get("title").cloned().unwrap_or(Value::Null),
        "owned": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handover(committed: Value, step: u32) -> Handover {
        let outcome = parse(&json!({
            "status": "needs_agent",
            "kind": "not_found",
            "step": step,
            "data": { "step": step, "query": "unicorn" },
            "done": [{ "n": 1, "ms": 780 }],
            "committed": committed,
            "page": { "pageId": 7, "url": "http://shop/s?k=unicorn", "title": "Shop" },
            "view": "- button \"Add to cart\" [ref=e12]",
        }));
        match outcome {
            Outcome::NeedsAgent(handover) => handover,
            other => Handover {
                kind: format!("unexpected {other:?}"),
                step: None,
                data: json!({}),
                done: json!([]),
                committed: Committed::No,
                page: Value::Null,
                view: None,
                duration_ms: None,
            },
        }
    }

    #[test]
    fn statuses_parse() {
        assert!(matches!(
            parse(&json!({ "status": "done", "output": { "a": 1 } })),
            Outcome::Done { .. }
        ));
        assert_eq!(parse(&json!({ "status": "reached" })), Outcome::Reached);
        assert_eq!(parse(&json!({ "status": "parsed" })), Outcome::Parsed);
        assert!(matches!(parse(&json!({})), Outcome::RunFailed(_)));
        assert!(matches!(
            parse(&json!({ "status": "verified_to_commit", "dry": [] })),
            Outcome::VerifiedToCommit { .. }
        ));
    }

    #[test]
    fn next_follows_whether_the_write_happened() {
        assert_eq!(
            next_instruction(&handover(json!(false), 2), Some(3)),
            "Continue from step 2 on page 7 with your normal tools. Nothing was written yet."
        );
        assert!(next_instruction(&handover(json!(true), 4), Some(3)).contains("never redo"));
        assert!(
            next_instruction(&handover(json!("unknown"), 3), Some(3)).contains("may or may not")
        );
    }

    #[test]
    fn the_packet_names_the_step_and_what_is_left() {
        let labels = vec![
            "Open the search.".to_string(),
            "Find the card.".to_string(),
            "Add it.".to_string(),
        ];
        let packet = packet(
            &handover(json!(false), 2),
            &labels,
            Some(3),
            "input_only",
            Some("async () => {}"),
            None,
        );
        assert_eq!(packet["failed"]["label"], "Find the card.");
        assert_eq!(
            packet["failed"]["message"],
            "not_found: {\"query\":\"unicorn\"}"
        );
        assert_eq!(packet["remaining"].as_array().map(Vec::len), Some(2));
        assert_eq!(packet["page"]["owned"], true);
        assert_eq!(packet["page"]["id"], 7);
        assert_eq!(packet["effects"]["committed"], false);
        assert_eq!(packet["cause"], "input_only");
        assert!(packet.get("repair").is_none());
    }
}
