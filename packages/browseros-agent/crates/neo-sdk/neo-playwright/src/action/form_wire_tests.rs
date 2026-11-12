//! The saved before-build boundary fixture exercises typed decoders and reply encoding. The
//! prelude test checks full JSON strings and option getter order; live suites cover effects.

use std::collections::BTreeSet;
use std::error::Error;

use serde::Deserialize;
use serde_json::Value;

use super::{
    ActionOp, DispatchEventInput, FillInput, PressInput, SelectOptionInput, SelectTextInput,
    TargetInput, TypeInput,
};
use crate::action::input_files::SetInputFilesInput;

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
    body: String,
}

fn decode_form(op: ActionOp, args: Value) -> Result<(), Box<dyn Error>> {
    match op {
        ActionOp::Fill => {
            let _: FillInput = serde_json::from_value(args)?;
        }
        ActionOp::Press => {
            let _: PressInput = serde_json::from_value(args)?;
        }
        ActionOp::PressSequentially => {
            let _: TypeInput = serde_json::from_value(args)?;
        }
        ActionOp::SelectOption => {
            let _: SelectOptionInput = serde_json::from_value(args)?;
        }
        ActionOp::SelectText => {
            let _: SelectTextInput = serde_json::from_value(args)?;
        }
        ActionOp::Focus | ActionOp::Blur | ActionOp::ScrollIntoViewIfNeeded => {
            let _: TargetInput = serde_json::from_value(args)?;
        }
        ActionOp::SetInputFiles => {
            let _: SetInputFilesInput = serde_json::from_value(args)?;
        }
        ActionOp::DispatchEvent => {
            let _: DispatchEventInput = serde_json::from_value(args)?;
        }
        _ => return Err("unexpected op in form recording".into()),
    }
    Ok(())
}

fn input(exchange: &Exchange) -> Result<Value, Box<dyn Error>> {
    let mut args: Value = serde_json::from_str(&exchange.args)?;
    args.as_object_mut().ok_or("object args")?.remove("call");
    Ok(args)
}

#[tokio::test]
async fn tests_that_form_inputs_and_reply_bytes_match_the_before_recording()
-> Result<(), Box<dyn Error>> {
    let recording: Recording =
        serde_json::from_str(include_str!("../../tests/data/form-wire.json"))?;
    let mut seen = BTreeSet::new();
    for exchange in recording.scenarios.into_iter().flat_map(|s| s.exchanges) {
        let Some(op) = ActionOp::ALL
            .iter()
            .copied()
            .find(|op| op.spec().name == exchange.op)
        else {
            continue;
        };
        decode_form(op, input(&exchange)?)?;
        seen.insert(op.spec().name);
        let wire = op.wire().ok_or("form op is not described")?;
        if op != ActionOp::SelectOption {
            assert_eq!(
                wire.output, "null",
                "{} must keep its null reply",
                exchange.op
            );
        }
        let encoded = super::wire_reply::encode(
            op,
            serde_json::from_str(&exchange.args)?,
            serde_json::from_str(&exchange.body)?,
        )
        .await?;
        assert_eq!(
            serde_json::to_string(&encoded)?,
            exchange.body,
            "{} recorded reply bytes",
            exchange.op
        );
    }
    assert_eq!(seen.len(), 10, "all ten form operations are pinned");
    Ok(())
}

/// Omission and false remain valid, but converting defaulted booleans to optional TS keys
/// must not allow null to reach a page effect.
#[test]
fn tests_that_defaulted_form_flags_reject_null_and_allow_omission() -> Result<(), Box<dyn Error>> {
    let recording: Recording =
        serde_json::from_str(include_str!("../../tests/data/form-wire.json"))?;
    for (op, bag, flag) in [
        (ActionOp::Fill, Some("options"), "force"),
        (ActionOp::Press, Some("options"), "noWaitAfter"),
        (ActionOp::SelectOption, None, "force"),
        (ActionOp::SelectText, None, "force"),
    ] {
        let exchange = recording
            .scenarios
            .iter()
            .flat_map(|s| &s.exchanges)
            .find(|e| e.op == op.spec().name)
            .ok_or("missing recorded op")?;
        let mut args = input(exchange)?;
        let fields = match bag {
            Some(key) => args.get_mut(key).ok_or("missing bag")?,
            None => &mut args,
        }
        .as_object_mut()
        .ok_or("object fields")?;
        fields.insert(flag.to_owned(), Value::Null);
        assert!(
            decode_form(op, args.clone()).is_err(),
            "{} accepted null for {flag}",
            op.spec().name
        );
        let fields = match bag {
            Some(key) => args.get_mut(key).ok_or("missing bag")?,
            None => &mut args,
        }
        .as_object_mut()
        .ok_or("object fields")?;
        fields.remove(flag);
        decode_form(op, args)?;
    }
    Ok(())
}
