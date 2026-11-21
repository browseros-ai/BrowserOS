//! The committed contract (`contracts/server-api/openapi.json`) is the one the server generates,
//! and every `$ref` in it resolves. So `cargo test` alone catches a stale spec after a change to a
//! handler or a wire type, without waiting for `bun run codegen:server-api:check`, and catches a
//! reference to a schema utoipa never collected (a parameter type, for one), which makes
//! openapi-typescript refuse the whole document.

use serde_json::Value;

fn generated() -> Result<Value, serde_json::Error> {
    serde_json::to_value(browseros_server::http::openapi())
}

#[test]
fn the_committed_contract_is_the_generated_one() -> Result<(), Box<dyn std::error::Error>> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../contracts/server-api/openapi.json"
    );
    let committed: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    // Compared as JSON values: the codegen script owns the file's formatting.
    assert!(
        committed == generated()?,
        "contracts/server-api/openapi.json is stale; run `bun run codegen:server-api`"
    );
    Ok(())
}

fn refs<'a>(value: &'a Value, found: &mut Vec<&'a str>) {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(target)) = object.get("$ref") {
                found.push(target);
            }
            object.values().for_each(|child| refs(child, found));
        }
        Value::Array(items) => items.iter().for_each(|child| refs(child, found)),
        _ => {}
    }
}

#[test]
fn every_reference_in_the_contract_resolves() -> Result<(), serde_json::Error> {
    let document = generated()?;
    let mut found = Vec::new();
    refs(&document, &mut found);
    let dangling: Vec<&str> = found
        .into_iter()
        .filter(|target| {
            let name = target.trim_start_matches("#/components/schemas/");
            document["components"]["schemas"][name].is_null()
        })
        .collect();
    assert!(dangling.is_empty(), "references to no schema: {dangling:?}");
    Ok(())
}
