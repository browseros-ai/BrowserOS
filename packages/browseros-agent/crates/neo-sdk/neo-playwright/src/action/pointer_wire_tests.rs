//! Both halves of the pointer byte fixture: recorded prelude inputs still decode with the
//! typed Rust handlers, and their reply serialization keeps the recorded JSON spelling.

use std::collections::BTreeSet;
use std::error::Error;

use serde::Deserialize;
use serde_json::Value;

use super::{
    ActionOp, CheckInput, ClickInput, Clicked, DblclickInput, DragInput, DropInput, HoverInput,
    UncheckInput,
};
use crate::action::requested_navigations::NavigationCut;

#[derive(Deserialize)]
struct Recording {
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
struct Scenario {
    exchanges: Vec<Exchange>,
}

#[derive(Deserialize)]
struct Exchange {
    op: String,
    args: String,
    reply: String,
    body: String,
}

fn decode_pointer_input(op: ActionOp, args: Value) -> Result<(), Box<dyn Error>> {
    match op {
        ActionOp::Click => {
            let _: ClickInput = serde_json::from_value(args)?;
        }
        ActionOp::Dblclick => {
            let _: DblclickInput = serde_json::from_value(args)?;
        }
        ActionOp::Hover => {
            let _: HoverInput = serde_json::from_value(args)?;
        }
        ActionOp::Check => {
            let _: CheckInput = serde_json::from_value(args)?;
        }
        ActionOp::Uncheck => {
            let _: UncheckInput = serde_json::from_value(args)?;
        }
        ActionOp::DragTo => {
            let _: DragInput = serde_json::from_value(args)?;
        }
        ActionOp::Drop => {
            let _: DropInput = serde_json::from_value(args)?;
        }
        _ => return Err("recording unexpectedly contains an unmigrated action".into()),
    }
    Ok(())
}

/// The pre-migration bootstrap recording and the real Rust serde types share this fixture.
/// The JS test pins whole request/reply strings; here every pointer request still decodes,
/// and the handler's reply encoding keeps both navigation-cut presence and null absence.
#[test]
fn tests_that_pointer_inputs_and_reply_bytes_match_the_before_recording()
-> Result<(), Box<dyn Error>> {
    let recording: Recording =
        serde_json::from_str(include_str!("../../tests/data/pointer-wire.json"))?;
    let mut seen = BTreeSet::new();
    let mut click_replies = BTreeSet::new();
    for exchange in recording.scenarios.into_iter().flat_map(|s| s.exchanges) {
        let Some(op) = ActionOp::ALL
            .iter()
            .copied()
            .find(|op| op.spec().name == exchange.op)
        else {
            // The smooth-scroll script also records page setup and evaluation; their
            // unchanged boundaries are byte-checked by the JS half of this fixture.
            continue;
        };
        seen.insert(op.spec().name);
        let mut args: Value = serde_json::from_str(&exchange.args)?;
        args.as_object_mut()
            .ok_or("recorded engine arguments must be an object")?
            .remove("call")
            .ok_or("recorded arguments must carry their shared call header")?;
        decode_pointer_input(op, args)?;
        let reply: Value = serde_json::from_str(&exchange.reply)?;
        let body: Value = serde_json::from_str(&exchange.body)?;
        assert_eq!(
            reply.get("ok"),
            Some(&body),
            "{}: recorded reply body",
            exchange.op
        );
        let encoded = if op == ActionOp::Click {
            click_replies.insert(body.is_null());
            let clicked = if body.is_null() {
                None
            } else {
                let navigation = body.get("navigation").ok_or("click cut is missing")?;
                Some(Clicked {
                    navigation: NavigationCut {
                        seq: navigation
                            .get("seq")
                            .and_then(Value::as_u64)
                            .ok_or("cut seq")?,
                        at: navigation
                            .get("at")
                            .and_then(Value::as_u64)
                            .ok_or("cut time")?,
                    },
                })
            };
            serde_json::to_value(clicked)?
        } else {
            assert_eq!(op.wire().ok_or("pointer op is undescribed")?.output, "null");
            serde_json::to_value(())?
        };
        // Run's to_output serializes through Value, whose object ordering is the byte
        // ordering the engine records, rather than direct struct-to-string ordering.
        assert_eq!(
            serde_json::to_string(&encoded)?,
            exchange.body,
            "{}: reply bytes",
            exchange.op
        );
    }
    assert_eq!(click_replies, BTreeSet::from([false, true]));
    assert_eq!(
        seen,
        BTreeSet::from([
            "locator.click",
            "locator.dblclick",
            "locator.hover",
            "locator.check",
            "locator.uncheck",
            "locator.dragTo",
            "locator.drop",
        ])
    );
    Ok(())
}

/// Optional TypeScript keys must not widen the old serde bool contract: omission defaults,
/// but an explicitly supplied null was invalid before these fields became Option<bool>.
#[test]
fn tests_that_defaulted_pointer_flags_reject_explicit_null() -> Result<(), Box<dyn Error>> {
    let recording: Recording =
        serde_json::from_str(include_str!("../../tests/data/pointer-wire.json"))?;
    for (op, flags) in [
        (ActionOp::Click, &["force", "trial", "noWaitAfter"][..]),
        (ActionOp::Dblclick, &["force", "trial"][..]),
        (ActionOp::Hover, &["force", "trial"][..]),
        (ActionOp::Check, &["force", "trial"][..]),
        (ActionOp::Uncheck, &["force", "trial"][..]),
        (ActionOp::DragTo, &["force", "trial"][..]),
    ] {
        let exchange = recording
            .scenarios
            .iter()
            .flat_map(|s| &s.exchanges)
            .find(|exchange| exchange.op == op.spec().name)
            .ok_or("pointer recording is missing an op")?;
        for flag in flags {
            let mut args: Value = serde_json::from_str(&exchange.args)?;
            args.as_object_mut().ok_or("object args")?.remove("call");
            args.get_mut("options")
                .and_then(Value::as_object_mut)
                .ok_or("pointer options")?
                .insert((*flag).to_owned(), Value::Null);
            assert!(
                decode_pointer_input(op, args).is_err(),
                "{} accepted null for {flag}",
                op.spec().name
            );
        }
    }
    Ok(())
}
