//! Playwright's text in Rust against its judges (`crate::judges`), through `render` with
//! `Call::Playwright`: the committed cases; the private sets of saved pages, where a folder names
//! them; and today's TypeScript renderer in QuickJS on seeded random trees. Upstream's printer
//! judges `node_text` below the interface, one generated node at a time, since it prints one node
//! and `render` never does.

use std::error::Error;
use std::{env, fs};

use serde_json::json;

use crate::aria_node::AriaNode;
use crate::call::Call;
use crate::judges::{
    Bundles, CaseFile, Generator, NEO_CASES, PLAYWRIGHT_CASES, Shown, count, seed,
};
use crate::render::render;
use crate::snapshot::Snapshot;

use super::{NodeAlone, node_text};

/// How many random nodes and trees the gate's run draws.
const GATE_NODES: usize = 3_000;
const GATE_TREES: usize = 300;

fn playwright(call: &Call<'_>) -> bool {
    matches!(call, Call::Playwright)
}

#[test]
fn tests_that_playwrights_format_prints_every_committed_case() -> Result<(), Box<dyn Error>> {
    for (file, text) in [
        ("playwright.json", PLAYWRIGHT_CASES),
        ("neo.json", NEO_CASES),
    ] {
        let cases: CaseFile = serde_json::from_str(text)?;
        let mut printed = 0;
        let mut differing = Vec::new();
        for found in cases.rendered_cases(playwright) {
            let (case, call, name) = found?;
            let answer = render(&cases.snapshot(name)?, call);
            if let Some(difference) = case.expected().difference(&call, &answer, Shown::Lines)? {
                differing.push(format!("{}: {difference}", case.name));
            }
            printed += 1;
        }
        assert!(
            printed > 0,
            "{file} holds Playwright-format cases, or this test judges nothing"
        );
        assert!(
            differing.is_empty(),
            "{file}: {} of {printed} Playwright-format cases differ from the committed text:\n{}",
            differing.len(),
            differing.join("\n")
        );
    }
    Ok(())
}

/// The private sets are real pages' content, so they live outside the repository and never go
/// to the remote pool: the test reads the folders `NEO_SNAPSHOT_PRIVATE_CASES` names (joined
/// with the platform's path separator), each `*.json` file in the committed cases' shape, and is
/// run by hand where they are kept. Without the variable it passes and says it was skipped, so a
/// landing that needs the proof pastes the counts from a run with it. Nothing it prints holds a
/// page's text: a difference is named by its case's number, its line and the two lengths.
#[test]
#[expect(
    clippy::print_stdout,
    reason = "the counts are the evidence a reviewer pastes; a silent skip must not read as a pass"
)]
fn tests_that_playwrights_format_prints_every_private_case() -> Result<(), Box<dyn Error>> {
    let Some(folders) = env::var_os("NEO_SNAPSHOT_PRIVATE_CASES") else {
        println!("private sets: SKIPPED (NEO_SNAPSHOT_PRIVATE_CASES names no folder)");
        return Ok(());
    };
    let bundles = Bundles::new()?;
    let mut differing: Vec<String> = Vec::new();
    let (mut texts, mut texts_identical, mut trees, mut trees_identical) = (0, 0, 0, 0);
    for file in &CaseFile::private_files(&folders)? {
        let shown = file.file_name().unwrap_or_default().to_string_lossy();
        let cases: CaseFile = serde_json::from_str(&fs::read_to_string(file)?)?;
        let (mut file_texts, mut file_texts_identical) = (0, 0);
        for (index, found) in cases.rendered_cases(playwright).enumerate() {
            let (case, call, name) = found?;
            let answer = render(&cases.snapshot(name)?, call);
            match case.expected().difference(&call, &answer, Shown::Lengths)? {
                None => file_texts_identical += 1,
                Some(difference) => differing.push(format!(
                    "{shown}: Playwright-format case {index}: {difference}"
                )),
            }
            file_texts += 1;
        }
        // Every saved tree, not only those a Playwright-format call was saved for: Rust's
        // text beside today's renderer's, in QuickJS.
        let (mut file_trees, mut file_trees_identical) = (0, 0);
        for (index, name) in cases.snapshot_names().enumerate() {
            let snapshot = cases.snapshot(name)?;
            let answer = render(&snapshot, Call::Playwright);
            let todays = bundles.todays(&Call::Playwright, &snapshot)?;
            match todays
                .expected()
                .difference(&Call::Playwright, &answer, Shown::Lengths)?
            {
                None => file_trees_identical += 1,
                Some(difference) => differing.push(format!(
                    "{shown}: tree {index}, against today's renderer: {difference}"
                )),
            }
            file_trees += 1;
        }
        println!(
            "private sets: {shown}: {file_texts_identical} of {file_texts} saved \
             Playwright-format texts identical; {file_trees_identical} of {file_trees} trees \
             print today's renderer's text"
        );
        texts += file_texts;
        texts_identical += file_texts_identical;
        trees += file_trees;
        trees_identical += file_trees_identical;
    }
    println!(
        "private sets: {texts_identical} of {texts} identical (Playwright's format), \
         {trees_identical} of {trees} trees as today's renderer prints them"
    );
    assert!(
        trees > 0,
        "the named folders hold no case file, which proves nothing"
    );
    assert!(
        differing.is_empty(),
        "{} differences:\n{}",
        differing.len(),
        differing.join("\n")
    );
    Ok(())
}

#[test]
fn tests_that_each_generated_node_prints_upstreams_own_lines() -> Result<(), Box<dyn Error>> {
    let seed = seed()?;
    let bundles = Bundles::new()?;
    let mut generator = Generator::new(seed);
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_NODES", GATE_NODES)? {
        let node = generator.node();
        let mut rust = String::new();
        node_text(&NodeAlone::from(&node), &mut rust);
        assert_eq!(
            rust,
            bundles.vendored_node_text(&node)?,
            "seed {seed:#x}, node {index} prints upstream's own text for it: {}",
            serde_json::to_string(&node)?
        );
    }
    Ok(())
}

#[test]
fn tests_that_playwrights_format_is_todays_on_generated_trees() -> Result<(), Box<dyn Error>> {
    let seed = seed()?;
    let bundles = Bundles::new()?;
    let mut generator = Generator::new(seed);
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_TREES", GATE_TREES)? {
        let snapshot = generator.snapshot();
        let answer = render(&snapshot, Call::Playwright);
        let todays = bundles.todays(&Call::Playwright, &snapshot)?;
        if let Some(difference) =
            todays
                .expected()
                .difference(&Call::Playwright, &answer, Shown::Lines)?
        {
            panic!("seed {seed:#x}, tree {index}: {difference}\n{snapshot:?}");
        }
    }
    Ok(())
}

/// The one place today's renderer and upstream's whole-tree text part, which the differential's
/// trees reach only by chance: a role or a ref with a line break. No page writes one, but the
/// tree's type admits it, and Rust prints today's text.
#[test]
fn tests_that_a_line_break_in_a_key_moves_its_colon_as_todays_renderer_does()
-> Result<(), Box<dyn Error>> {
    let tree: Vec<AriaNode> = serde_json::from_value(json!([{
        "role": "button",
        "ref": "a\nb",
        "children": [{ "role": "img" }],
    }]))?;
    let snapshot = Snapshot {
        tree,
        ..Snapshot::default()
    };
    let todays = "- 'button [ref=a:\nb]'\n  - img";
    assert_eq!(
        render(&snapshot, Call::Playwright).text(),
        todays,
        "the key's first line takes the colon, as the TypeScript splits a node's text at line breaks"
    );
    let bundles = Bundles::new()?;
    assert_eq!(
        bundles.todays_playwright_text(&snapshot)?,
        todays,
        "today's renderer prints the same"
    );
    assert_eq!(
        bundles.vendored_text(&snapshot.tree)?,
        "- 'button [ref=a\nb]':\n  - img",
        "upstream's own walk keeps the colon at the end of the quoted key"
    );
    Ok(())
}
