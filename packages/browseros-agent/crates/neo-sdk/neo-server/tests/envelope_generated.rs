//! The run envelope's contract with the prelude: the text a run's realm receives (its init, every
//! op's reply) and the TypeScript the prelude reads it with.
//!
//! The bytes are pinned: every example `neo_server::test_support::envelope_examples` renders
//! must equal its entry in `tests/data/envelope-examples.json`, which the prelude's
//! `tests/prelude-envelope.test.ts` reads too. This test never rewrites that file: a byte change
//! is a change to the contract, made by hand on both sides.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::Path;

use neo_server::test_support::envelope_examples;

/// From this crate's manifest directory.
const PINNED: &str = "tests/data/envelope-examples.json";

#[test]
fn tests_that_the_envelope_bytes_match_the_pinned_examples() -> Result<(), Box<dyn Error>> {
    let rendered: BTreeMap<String, String> = envelope_examples()?
        .into_iter()
        .map(|example| (example.name.to_owned(), example.text))
        .collect();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(PINNED);
    let pinned: BTreeMap<String, String> = match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)?,
        Err(error) => {
            return Err(format!(
                "{}: {error}; the examples render as:\n{}",
                path.display(),
                serde_json::to_string_pretty(&rendered)?
            )
            .into());
        }
    };
    let mut differences = Vec::new();
    for (name, text) in &rendered {
        match pinned.get(name) {
            Some(expected) if expected == text => {}
            Some(expected) => {
                differences.push(format!("{name}\n    pinned:   {expected}\n    rendered: {text}"));
            }
            None => differences.push(format!("{name} is not pinned; it renders as {text}")),
        }
    }
    for name in pinned.keys().filter(|name| !rendered.contains_key(*name)) {
        differences.push(format!("{name} is pinned and no longer rendered"));
    }
    assert!(
        differences.is_empty(),
        "the run envelope's bytes changed ({}); a change on purpose updates that file and the \
         prelude's tests/prelude-envelope.test.ts with it:\n  {}",
        path.display(),
        differences.join("\n  ")
    );
    Ok(())
}
