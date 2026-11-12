//! Coverage of real prelude recordings, using Rust's nesting metadata to find each logical
//! option. TypeScript's checker supplies optional input fields and generated option tuples;
//! build --check keeps that metadata fresh without a second hand-written input schema.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use neo_playwright::{Op, OpWire, OptionNesting};
use serde::Deserialize;
use serde_json::{Value, json};

/// Upstream's shared Frame.expect params include fields Page's public matchers cannot send.
/// Each exception is checked both ways: it must remain optional and must remain unrecorded.
const UNREACHABLE: &[(&str, &str, &str)] = &[
    (
        "expect.page",
        "expressionArg",
        "only locator attribute/CSS/JS-property matchers supply expressionArg; Page receiver validation rejects them",
    ),
    (
        "expect.page",
        "pseudo",
        "only locator.toHaveCSS supplies pseudo; Page receiver validation rejects that matcher",
    ),
    (
        "expect.page",
        "expectedNumber",
        "only locator.toHaveCount supplies expectedNumber; Page receiver validation rejects that matcher",
    ),
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Examples {
    optional_keys: BTreeMap<String, BTreeSet<String>>,
    ops: BTreeMap<String, Vec<Value>>,
}

fn observed(wire: &OpWire, inputs: &[Value], key: &str) -> bool {
    inputs.iter().any(|input| {
        let fields =
            if wire.option_nesting == OptionNesting::Nested && wire.option_keys.contains(&key) {
                input.get("options")
            } else {
                Some(input)
            };
        fields
            .and_then(Value::as_object)
            .is_some_and(|fields| fields.contains_key(key))
    })
}

fn gaps(
    wire: &OpWire,
    keys: &BTreeSet<String>,
    inputs: &[Value],
    exceptions: &BTreeSet<&str>,
) -> Vec<String> {
    let mut failures = Vec::new();
    for key in keys {
        let present = observed(wire, inputs, key);
        if present && exceptions.contains(key.as_str()) {
            failures.push(format!(
                "{key}: stale exception, the caller produced this key"
            ));
        } else if !present && !exceptions.contains(key.as_str()) {
            failures.push(format!("{key}: no recorded example"));
        }
    }
    for key in exceptions {
        if !keys.contains(*key) {
            failures.push(format!("{key}: stale exception, no longer an optional key"));
        }
    }
    failures
}

#[test]
fn tests_that_every_optional_input_key_has_a_recorded_example() -> Result<(), Box<dyn Error>> {
    let examples: Examples = serde_json::from_str(include_str!("data/wire-examples.json"))?;
    let registered: BTreeSet<_> = Op::all().map(|op| op.spec().name.to_owned()).collect();
    assert!(examples.ops.keys().all(|name| registered.contains(name)));
    assert_eq!(registered, examples.optional_keys.keys().cloned().collect());
    let mut failures = Vec::new();
    for op in Op::all() {
        let name = op.spec().name;
        let Some(wire) = op.wire() else {
            failures.push(format!("{name}: no OpWire yet"));
            continue;
        };
        let keys = examples
            .optional_keys
            .get(name)
            .ok_or("missing key metadata")?;
        let inputs = examples.ops.get(name).map_or(&[][..], Vec::as_slice);
        // OPTION_KEYS and OpWire come from the same declaration. The checker also adds flat
        // optional fields on ops without a bag, such as goto.referer and selectOption.force.
        for key in &wire.option_keys {
            assert!(
                keys.contains(*key),
                "{name}.{key}: missing generated option metadata"
            );
        }
        for address in ["target", "strict", "page", "frame", "call"] {
            assert!(
                !keys.contains(address),
                "{name}.{address}: addressing is not an option"
            );
        }
        let exceptions = UNREACHABLE
            .iter()
            .filter_map(|(owner, key, _reason)| (*owner == name).then_some(*key))
            .collect();
        failures.extend(
            gaps(&wire, keys, inputs, &exceptions)
                .into_iter()
                .map(|gap| format!("{name}.{gap}")),
        );
    }
    for (op, _key, reason) in UNREACHABLE {
        assert!(registered.contains(*op), "an exception names an unknown op");
        assert!(!reason.is_empty(), "every exception needs a reason");
    }
    assert!(
        failures.is_empty(),
        "optional input coverage:\n{}",
        failures.join("\n")
    );
    Ok(())
}

#[test]
fn tests_that_option_coverage_survives_flattening_and_rejects_stale_exceptions()
-> Result<(), Box<dyn Error>> {
    let mut wire = Op::from_name("locator.fill")
        .ok_or("fill op")?
        .wire()
        .ok_or("fill wire")?;
    // Keep exercising both shapes after the last real family has opted into flat options.
    // These are the same real force key and values, with only their container changed.
    wire.option_nesting = OptionNesting::Nested;
    let keys = BTreeSet::from(["force".to_owned()]);
    let none = BTreeSet::new();
    let present = vec![json!({"options": {"force": false}})];
    let absent = vec![json!({"options": {}})];
    let covered = gaps(&wire, &keys, &present, &none);
    let missing = gaps(&wire, &keys, &absent, &none);
    wire.option_nesting = OptionNesting::Flat;
    assert_eq!(
        covered,
        gaps(&wire, &keys, &[json!({"force": false})], &none)
    );
    assert_eq!(missing, gaps(&wire, &keys, &[json!({})], &none));
    assert!(covered.is_empty());
    assert_eq!(missing.len(), 1);
    let allowed = BTreeSet::from(["force"]);
    assert!(gaps(&wire, &keys, &[json!({})], &allowed).is_empty());
    assert_eq!(
        gaps(&wire, &keys, &[json!({"force": false})], &allowed).len(),
        1
    );
    assert_eq!(
        gaps(&wire, &keys, &[json!({})], &BTreeSet::from(["removedKey"])).len(),
        2
    );
    Ok(())
}
