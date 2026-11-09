//! Neo: the JSON both sides of the hook realm agree on: the payload Rust hands the SDK's
//! `invoke(id, event, payload)` for each event, and the decision `invoke` resolves with, checked
//! here against what that event allows. Anything else fails closed (hooks design D8): an
//! unreadable decision, one with an unknown field, or one the event does not allow is a
//! [`HookFailure::BadDecision`], exactly as a hook that threw is a [`HookFailure::Threw`].

use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::events::CONTEXT_GUID;
use crate::hooks::{ActionEnd, ActionEvent, AfterDecision, HookEvent, HookFailure, Replaced};
use crate::run::RunOutcome;

/// What `invoke` resolved with, as the SDK writes it.
#[derive(Debug, Deserialize)]
#[serde(tag = "decision", rename_all = "camelCase", deny_unknown_fields)]
enum Wire {
    /// A struct variant, so `deny_unknown_fields` refuses extra fields on it too: serde lets a
    /// unit variant of an internally tagged enum carry any.
    Allow {},
    Block {
        reason: String,
    },
    ReplaceArgs {
        args: Value,
    },
    Keep {
        #[serde(default)]
        notes: Vec<String>,
    },
    Replace {
        /// Absent for `{ replace: undefined }`, which JSON cannot hold; `null` is `null`.
        #[serde(default, deserialize_with = "present")]
        value: Option<Value>,
        #[serde(default)]
        notes: Vec<String>,
    },
    Fail {
        error: String,
    },
}

/// A decision its event allows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Decided {
    Allow,
    Block(String),
    ReplaceArgs(Value),
    After(AfterDecision),
}

/// Reads `invoke`'s decision `text` for `event`. `beforeRun` may allow or block (never rewrite
/// the script, hooks design D12), `beforeAction` may also replace the arguments with an object,
/// and the after events keep or replace with notes. A block needs a reason, which the run's
/// outcome shows.
pub(crate) fn parse(event: HookEvent, text: &str) -> Result<Decided, HookFailure> {
    let wire: Wire = serde_json::from_str(text)
        .map_err(|error| HookFailure::BadDecision(format!("{}: {error}", event.name())))?;
    let before = matches!(event, HookEvent::BeforeRun | HookEvent::BeforeAction);
    match wire {
        Wire::Fail { error } => Err(HookFailure::Threw(error)),
        Wire::Allow {} if before => Ok(Decided::Allow),
        Wire::Block { reason } if before && !reason.is_empty() => Ok(Decided::Block(reason)),
        Wire::ReplaceArgs { args } if event == HookEvent::BeforeAction && args.is_object() => {
            Ok(Decided::ReplaceArgs(args))
        }
        Wire::Keep { notes } if !before => Ok(Decided::After(AfterDecision {
            replace: None,
            notes,
        })),
        Wire::Replace { value, notes } if !before => Ok(Decided::After(AfterDecision {
            replace: Some(Replaced {
                value,
                binaries: Vec::new(),
            }),
            notes,
        })),
        wire => Err(HookFailure::BadDecision(format!(
            "{} cannot decide {wire:?}",
            event.name()
        ))),
    }
}

/// `beforeRun`'s payload: the script's text, the run page and the run's context.
pub(crate) fn before_run(script: &str, page: &str) -> String {
    json!({ "script": script, "page": page, "context": CONTEXT_GUID }).to_string()
}

/// `afterRun`'s payload: how the script ended, its value or error, the run page and context.
/// A returned binary is the placeholder the outcome's `value` holds (`RunOutcome::binaries`).
pub(crate) fn after_run(outcome: &RunOutcome, page: &str) -> String {
    let mut payload = json!({ "status": outcome.status, "page": page, "context": CONTEXT_GUID });
    if let Some(value) = &outcome.value {
        payload["value"] = value.clone();
    }
    if let Some(error) = &outcome.error {
        payload["error"] = json!({ "name": error.name, "message": error.message });
    }
    payload.to_string()
}

/// `beforeAction`'s payload, and `afterAction`'s with how the action `ended`.
pub(crate) fn action(event: &ActionEvent, ended: Option<&ActionEnd>) -> String {
    #[derive(Serialize)]
    struct Payload<'a> {
        #[serde(flatten)]
        event: &'a ActionEvent,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<&'a Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<Value>,
    }
    let (result, error) = match ended {
        Some(ActionEnd::Result(value)) => (Some(value), None),
        Some(ActionEnd::Error(error)) => (
            None,
            Some(json!({ "name": error.name, "message": error.message })),
        ),
        None => (None, None),
    };
    // Every field serializes; a failure here would be a bug in this module's own types.
    serde_json::to_string(&Payload {
        event,
        result,
        error,
    })
    .unwrap_or_else(|_| "{}".to_string())
}

/// A field that is present, `null` included: `Some(Value::Null)` for `null`, where `Option`'s
/// own `Deserialize` reads `null` as `None`.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::hooks::LocatorRef;
    use crate::run::{CallLocation, RunError};

    fn decide(event: HookEvent, decision: &Value) -> Result<Decided, HookFailure> {
        parse(event, &decision.to_string())
    }

    /// An after decision; `replace` is `Some(None)` for a replacement with `undefined`.
    fn after(replace: Option<Value>, replaced: bool, notes: &[&str]) -> Decided {
        Decided::After(AfterDecision {
            replace: replaced.then(|| Replaced {
                value: replace,
                binaries: Vec::new(),
            }),
            notes: notes.iter().map(ToString::to_string).collect(),
        })
    }

    #[test]
    fn tests_that_each_event_reads_the_decisions_it_allows() {
        for event in [HookEvent::BeforeRun, HookEvent::BeforeAction] {
            assert_eq!(
                decide(event, &json!({"decision": "allow"})),
                Ok(Decided::Allow)
            );
            assert_eq!(
                decide(
                    event,
                    &json!({"decision": "block", "reason": "owner approval"})
                ),
                Ok(Decided::Block("owner approval".to_string()))
            );
        }
        assert_eq!(
            decide(
                HookEvent::BeforeAction,
                &json!({"decision": "replaceArgs", "args": {"value": "s3cret"}})
            ),
            Ok(Decided::ReplaceArgs(json!({"value": "s3cret"})))
        );
        for event in [HookEvent::AfterRun, HookEvent::AfterAction] {
            assert_eq!(
                decide(event, &json!({"decision": "keep", "notes": []})),
                Ok(after(None, false, &[]))
            );
            assert_eq!(
                decide(event, &json!({"decision": "keep", "notes": ["a", "b"]})),
                Ok(after(None, false, &["a", "b"]))
            );
            assert_eq!(
                decide(
                    event,
                    &json!({"decision": "replace", "value": {"x": 1}, "notes": ["n"]})
                ),
                Ok(after(Some(json!({"x": 1})), true, &["n"]))
            );
            assert_eq!(
                decide(event, &json!({"decision": "replace", "value": null})),
                Ok(after(Some(Value::Null), true, &[])),
                "null replaces with null"
            );
            assert_eq!(
                decide(event, &json!({"decision": "replace", "notes": []})),
                Ok(after(None, true, &[])),
                "an absent value replaces with undefined"
            );
        }
    }

    #[test]
    fn tests_that_a_decision_the_event_does_not_allow_fails() {
        let refused = [
            (
                HookEvent::BeforeRun,
                json!({"decision": "replaceArgs", "args": {}}),
            ),
            (
                HookEvent::BeforeRun,
                json!({"decision": "keep", "notes": []}),
            ),
            (
                HookEvent::BeforeRun,
                json!({"decision": "replace", "value": 1}),
            ),
            (HookEvent::AfterRun, json!({"decision": "allow"})),
            (
                HookEvent::AfterRun,
                json!({"decision": "block", "reason": "no"}),
            ),
            (
                HookEvent::AfterRun,
                json!({"decision": "replaceArgs", "args": {}}),
            ),
            (
                HookEvent::BeforeAction,
                json!({"decision": "keep", "notes": []}),
            ),
            (HookEvent::AfterAction, json!({"decision": "allow"})),
            (
                HookEvent::AfterAction,
                json!({"decision": "replaceArgs", "args": {}}),
            ),
        ];
        for (event, decision) in refused {
            assert!(
                matches!(decide(event, &decision), Err(HookFailure::BadDecision(_))),
                "{event:?} must refuse {decision}"
            );
        }
    }

    #[test]
    fn tests_that_every_failure_shape_fails_closed() {
        assert!(matches!(
            decide(HookEvent::BeforeRun, &json!({"decision": "fail", "error": "boom"})),
            Err(HookFailure::Threw(detail)) if detail == "boom"
        ));
        let bad = [
            "not json",
            "\"allow\"",
            "{}",
            r#"{"decision":"maybe"}"#,
            r#"{"decision":"allow","extra":1}"#,
            r#"{"decision":"block"}"#,
            r#"{"decision":"block","reason":""}"#,
            r#"{"decision":"block","reason":7}"#,
            r#"{"decision":"replaceArgs","args":"x"}"#,
            r#"{"decision":"keep","notes":[1]}"#,
            r#"{"decision":"keep","notes":"n"}"#,
        ];
        for text in bad {
            let event = if text.contains("keep") {
                HookEvent::AfterRun
            } else {
                HookEvent::BeforeAction
            };
            assert!(
                matches!(parse(event, text), Err(HookFailure::BadDecision(_))),
                "{text} must fail closed"
            );
        }
    }

    #[test]
    fn tests_that_payloads_carry_the_contract_fields() {
        let before: Value = serde_json::from_str(&before_run("return 1", "page@T")).unwrap();
        assert_eq!(
            before,
            json!({"script": "return 1", "page": "page@T", "context": "context@default"})
        );
        let outcome = RunOutcome::returned(Some(json!({"a": 1})), Vec::new());
        let ok: Value = serde_json::from_str(&after_run(&outcome, "page@T")).unwrap();
        assert_eq!(
            ok,
            json!({"status": "ok", "value": {"a": 1}, "page": "page@T", "context": "context@default"})
        );
        let failed = RunOutcome::failed(RunError::new("TypeError", "x".to_string()), Vec::new());
        let failed: Value = serde_json::from_str(&after_run(&failed, "page@T")).unwrap();
        assert_eq!(failed["status"], "error");
        assert_eq!(
            failed["error"],
            json!({"name": "TypeError", "message": "x"})
        );
        assert!(failed.get("value").is_none());

        let event = ActionEvent {
            kind: Some("fill"),
            group: None,
            method: "fill".to_string(),
            guid: "frame@T".to_string(),
            args: json!({"selector": "#p", "value": "{{password}}"}),
            location: Some(CallLocation {
                line: 3,
                column: Some(7),
            }),
            page: Some("page@T".to_string()),
            frame: Some("frame@T".to_string()),
            context: "context@default".to_string(),
            locator: Some(LocatorRef {
                frame: "frame@T".to_string(),
                selector: "#p".to_string(),
            }),
            element: None,
        };
        let before: Value = serde_json::from_str(&action(&event, None)).unwrap();
        assert_eq!(
            before,
            json!({
                "kind": "fill", "group": null, "method": "fill", "guid": "frame@T",
                "args": {"selector": "#p", "value": "{{password}}"},
                "location": {"line": 3, "column": 7}, "page": "page@T", "frame": "frame@T",
                "context": "context@default", "locator": {"frame": "frame@T", "selector": "#p"},
            })
        );
        let done: Value =
            serde_json::from_str(&action(&event, Some(&ActionEnd::Result(json!({}))))).unwrap();
        assert_eq!(done["result"], json!({}));
        let failed = ActionEnd::Error(RunError::new("TimeoutError", "late".to_string()));
        let failed: Value = serde_json::from_str(&action(&event, Some(&failed))).unwrap();
        assert_eq!(
            failed["error"],
            json!({"name": "TimeoutError", "message": "late"})
        );
        assert!(failed.get("result").is_none());
    }
}
