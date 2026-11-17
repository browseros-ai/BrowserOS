//! Both halves of the pointer byte fixture: aligned flat inputs decode with the typed Rust
//! handlers, and reply serialization keeps the recorded JSON spelling.

use std::collections::BTreeSet;
use std::error::Error;
use std::iter;

use serde::Deserialize;
use serde_json::Value;

use super::{
    ActionOp, CheckInput, ClickInput, DblclickInput, DragInput, DropInput, HoverInput, UncheckInput,
};

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

/// The aligned bootstrap recording and the real Rust serde types share this fixture.
/// The JS test pins whole request/reply strings; here every pointer request decodes,
/// and the handler's reply encoding keeps both navigation-cut presence and null absence.
#[tokio::test]
async fn tests_that_pointer_inputs_and_reply_bytes_match_the_aligned_recording()
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
            .ok_or("recorded object args")?
            .remove("call")
            .ok_or("recorded call header")?;
        decode_pointer_input(op, args)?;
        let reply: Value = serde_json::from_str(&exchange.reply)?;
        let body: Value = serde_json::from_str(&exchange.body)?;
        assert_eq!(
            reply.get("ok"),
            Some(&body),
            "{}: recorded reply body",
            exchange.op
        );
        if op == ActionOp::Click {
            click_replies.insert(body.is_null());
        } else {
            assert_eq!(op.wire().output, "null");
        }
        let encoded =
            super::wire_reply::encode(op, serde_json::from_str(&exchange.args)?, body).await?;
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

/// Optional TypeScript keys must not widen the serde bool contract: omission defaults,
/// but an explicitly supplied null is invalid before any browser input is sent.
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
            args.as_object_mut()
                .ok_or("pointer input")?
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

#[test]
fn tests_that_flat_pointer_inputs_reject_old_nesting_and_other_commands_keys()
-> Result<(), Box<dyn Error>> {
    let recording: Recording =
        serde_json::from_str(include_str!("../../tests/data/pointer-wire.json"))?;
    for (op, unsupported) in [
        (ActionOp::Click, &[][..]),
        (ActionOp::Dblclick, &["noWaitAfter", "clickCount"][..]),
        (
            ActionOp::Hover,
            &["noWaitAfter", "button", "clickCount", "delay", "steps"][..],
        ),
        (
            ActionOp::Check,
            &[
                "noWaitAfter",
                "button",
                "clickCount",
                "delay",
                "steps",
                "modifiers",
            ][..],
        ),
        (
            ActionOp::Uncheck,
            &[
                "noWaitAfter",
                "button",
                "clickCount",
                "delay",
                "steps",
                "modifiers",
            ][..],
        ),
        (ActionOp::DragTo, &["noWaitAfter", "modifiers"][..]),
    ] {
        let exchange = recording
            .scenarios
            .iter()
            .flat_map(|s| &s.exchanges)
            .find(|exchange| exchange.op == op.spec().name)
            .ok_or("pointer recording is missing an op")?;
        let mut args: Value = serde_json::from_str(&exchange.args)?;
        args.as_object_mut().ok_or("object args")?.remove("call");
        for key in iter::once("options").chain(unsupported.iter().copied()) {
            let mut invalid = args.clone();
            invalid
                .as_object_mut()
                .ok_or("object args")?
                .insert(key.to_owned(), serde_json::json!({}));
            let error = decode_pointer_input(op, invalid)
                .err()
                .ok_or("unsupported pointer field decoded")?;
            assert!(
                error
                    .to_string()
                    .contains(&format!("unknown field `{key}`")),
                "{} accepted a field its command lacks: {error}",
                op.spec().name
            );
        }
    }
    Ok(())
}

#[test]
fn tests_that_drop_data_can_be_omitted_but_not_null() -> Result<(), Box<dyn Error>> {
    let recording: Recording =
        serde_json::from_str(include_str!("../../tests/data/pointer-wire.json"))?;
    let exchange = recording
        .scenarios
        .iter()
        .flat_map(|s| &s.exchanges)
        .find(|exchange| exchange.op == "locator.drop")
        .ok_or("pointer recording is missing drop")?;
    let mut args: Value = serde_json::from_str(&exchange.args)?;
    let fields = args.as_object_mut().ok_or("object args")?;
    fields.remove("call");
    fields.remove("data");
    let input: DropInput = serde_json::from_value(args.clone())?;
    assert!(
        input.data.is_none(),
        "files-only drops have no MIME entries"
    );
    args.as_object_mut()
        .ok_or("object args")?
        .insert("data".to_owned(), Value::Null);
    assert!(serde_json::from_value::<DropInput>(args).is_err());
    Ok(())
}
