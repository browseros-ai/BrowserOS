//! Saved snapshot trees from real pages, decoded into [`AriaNode`] and written back.
//!
//! The trees are real pages' content, so they live outside the repository and never go to the
//! remote pool. The test reads the folders named in `NEO_SNAPSHOT_SAVED_TREES` (joined with the
//! platform's path separator) and is run by hand where they are kept. Each file holds one snapshot
//! operation's result, `{ "json": [...], ... }`. Without the variable the test passes and prints
//! `saved trees: SKIPPED`, so a landing that needs the proof pastes the counts from a run with it.

use std::error::Error;
use std::path::PathBuf;
use std::{env, fs};

use neo_snapshot::AriaNode;
use serde::Deserialize;
use serde_json::Value;

/// The production decode goes through an internally tagged reply, which hands the tree to
/// `AriaNode` through serde's buffered content rather than straight from the text. Both paths
/// must read every saved tree alike.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Reply {
    Snapshot { json: Vec<AriaNode> },
}

#[test]
#[expect(
    clippy::print_stdout,
    reason = "the counts are the evidence a reviewer pastes; a silent skip must not read as a pass"
)]
fn tests_that_saved_real_trees_decode_and_write_back_the_same_json() -> Result<(), Box<dyn Error>> {
    let Some(folders) = env::var_os("NEO_SNAPSHOT_SAVED_TREES") else {
        println!("saved trees: SKIPPED (NEO_SNAPSHOT_SAVED_TREES names no folder)");
        return Ok(());
    };
    let mut differing: Vec<PathBuf> = Vec::new();
    let mut read = 0;
    for folder in env::split_paths(&folders) {
        let mut files = Vec::new();
        for entry in fs::read_dir(&folder)? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                files.push(path);
            }
        }
        files.sort();
        let mut identical = 0;
        for file in &files {
            let saved: Value = serde_json::from_str(&fs::read_to_string(file)?)?;
            let Some(tree) = saved.get("json") else {
                differing.push(file.clone());
                continue;
            };
            let text = serde_json::to_string(tree)?;
            let direct: Vec<AriaNode> = serde_json::from_str(&text)?;
            let Reply::Snapshot { json: buffered } =
                serde_json::from_str(&format!("{{\"kind\":\"snapshot\",\"json\":{text}}}"))?;
            let written = serde_json::to_value(&direct)?;
            // Equal as JSON values (keys, strings, number spellings) and, with the map's sorted
            // keys, as text: the tree reads and writes the same JSON.
            if written == *tree
                && serde_json::to_string(&written)? == text
                && serde_json::to_value(&buffered)? == written
            {
                identical += 1;
            } else {
                differing.push(file.clone());
            }
        }
        read += files.len();
        println!(
            "saved trees: {identical} of {} decoded and written back identical",
            files.len()
        );
    }
    assert!(
        read > 0,
        "the named folders hold no tree, which proves nothing"
    );
    assert!(
        differing.is_empty(),
        "{} saved trees do not round-trip: {differing:?}",
        differing.len()
    );
    Ok(())
}
