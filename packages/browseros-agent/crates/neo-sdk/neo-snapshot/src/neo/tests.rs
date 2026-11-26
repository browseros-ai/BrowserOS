//! Neo's default format in Rust against its judges (`crate::judges`), through `render` with
//! `Call::Element` and `Call::Page`: the committed cases; the private sets of saved pages, where
//! a folder names them; and today's TypeScript renderer in QuickJS on seeded random trees with
//! every fact and every combination of options. Each is held to its text and to what it keeps.

use std::error::Error;
use std::{env, fs};

use serde_json::json;

use crate::call::Call;
use crate::cap::MaxTokens;
use crate::cover_block::cover_block;
use crate::form::Passed;
use crate::judges::{Bundles, CaseFile, Generator, NEO_CASES, Shown, count, seed};
use crate::render::render;
use crate::snapshot::{Layer, Snapshot};

use super::neo_text_as;

/// How many random trees the gate's run draws.
const GATE_TREES: usize = 300;

/// A call in Neo's default format, a locator's or a whole page's.
fn neos(call: &Call<'_>) -> bool {
    matches!(call, Call::Element(_) | Call::Page(_))
}

#[test]
fn tests_that_neos_format_prints_every_committed_case() -> Result<(), Box<dyn Error>> {
    let cases: CaseFile = serde_json::from_str(NEO_CASES)?;
    let mut judged = 0;
    let mut differing = Vec::new();
    for found in cases.rendered_cases(neos) {
        let (case, call, name) = found?;
        let answer = render(&cases.snapshot(name)?, call);
        if let Some(difference) = case.expected().difference(&call, &answer, Shown::Lines)? {
            differing.push(format!("{}: {difference}", case.name));
        }
        judged += 1;
    }
    assert!(
        judged > 0,
        "neo.json holds cases in Neo's format, or this test judges nothing"
    );
    assert!(
        differing.is_empty(),
        "{} of {judged} cases in Neo's format differ from the committed text:\n{}",
        differing.len(),
        differing.join("\n")
    );
    Ok(())
}

/// `getByRole` reaches an element under an `aria-hidden` node only with `includeHidden`, which an
/// agent asks for only where the line, or a line it is nested under, says `[aria-hidden]`. So
/// each line printed where such a node printed no line around it carries the mark, in both forms;
/// a section's content, nested under its marked line, does not need it.
#[test]
fn tests_that_a_line_printed_in_place_of_an_aria_hidden_node_carries_its_mark()
-> Result<(), Box<dyn Error>> {
    let tree = serde_json::from_value(json!([
        { "role": "generic", "ariaHidden": true, "children": [
            { "role": "button", "name": "Next slide", "ref": "e1" },
            { "role": "heading", "name": "Slide 2", "level": 2, "children": [
                { "role": "link", "name": "Read more", "ref": "e2" },
            ] },
            { "role": "main", "ref": "e3", "children": [
                { "role": "button", "name": "Inside main", "ref": "e4" },
            ] },
        ] },
        { "role": "button", "name": "Menu", "ref": "e5", "ariaHidden": true, "children": [
            { "role": "link", "name": "Settings", "ref": "e6" },
        ] },
        { "role": "combobox", "name": "Plan", "ref": "e8", "children": [
            { "role": "option", "name": "Basic" },
            { "role": "group", "name": "Old", "ariaHidden": true, "children": [
                { "role": "option", "name": "Legacy", "selected": true },
            ] },
        ] },
        { "role": "button", "name": "After", "ref": "e7" },
    ]))?;
    let snapshot = Snapshot {
        tree,
        ..Snapshot::default()
    };
    let marked = [
        "- button \"Next slide\" [aria-hidden] [ref=e1]",
        "- heading \"Slide 2\" [level=2] [aria-hidden]",
        "- link \"Read more\" [aria-hidden] [ref=e2]",
        "- main [aria-hidden] [ref=e3]:",
        "  - button \"Inside main\" [ref=e4]",
        "- button \"Menu\" [aria-hidden] [ref=e5]",
        "- link \"Settings\" [aria-hidden] [ref=e6]",
        "- combobox \"Plan\" [ref=e8]:",
        "  - option \"Basic\"",
        "  - option \"Legacy\" [selected] [aria-hidden]",
        "- button \"After\" [ref=e7]",
    ]
    .join("\n");
    for text in [true, false] {
        let passed = Passed {
            text: Some(text),
            ..Passed::default()
        };
        assert_eq!(
            render(&snapshot, Call::Page(&passed)).text(),
            marked,
            "with text: {text}, every line in place of the hidden wrapper, the hidden button and \
             the hidden option group says [aria-hidden], the section's content, the group's \
             sibling option and the button after them do not"
        );
    }
    Ok(())
}

/// A cut prints each kept block after it at the left edge, outside the section lines around it,
/// so a kept block's own line says a mark that only those lines show. A child document is not
/// hidden by a mark above its iframe.
#[test]
fn tests_that_a_kept_block_printed_after_a_cut_keeps_the_mark_of_a_section_around_it()
-> Result<(), Box<dyn Error>> {
    let mut main = vec![];
    for index in 1..=6 {
        main.push(json!({ "role": "button", "name": format!("Item {index}"), "ref": format!("e{}", index + 9) }));
    }
    main.push(
        json!({ "role": "dialog", "name": "Confirm", "ref": "e2", "children": [
        { "role": "button", "name": "OK", "ref": "e3" },
    ] }),
    );
    main.push(json!({ "role": "iframe", "ref": "e4", "children": [
        { "role": "dialog", "name": "Pay", "ref": "f1e1", "children": [
            { "role": "button", "name": "Card", "ref": "f1e2" },
        ] },
    ] }));
    let snapshot = Snapshot {
        tree: serde_json::from_value(json!([
            { "role": "main", "ref": "e1", "ariaHidden": true, "children": main },
        ]))?,
        ..Snapshot::default()
    };
    let items: Vec<String> = (1..=6)
        .map(|index| format!("  - button \"Item {index}\" [ref=e{}]", index + 9))
        .collect();
    let in_place = [
        vec!["- main [aria-hidden] [ref=e1]:".to_owned()],
        items.clone(),
        [
            "  - dialog \"Confirm\" [aria-hidden] [ref=e2]:",
            "    - button \"OK\" [ref=e3]",
            "  - iframe [ref=e4]:",
            "    - dialog \"Pay\" [ref=f1e1]:",
            "      - button \"Card\" [ref=f1e2]",
        ]
        .map(str::to_owned)
        .to_vec(),
    ]
    .concat()
    .join("\n");
    let cut = [
        vec!["- main [aria-hidden] [ref=e1]:".to_owned()],
        items.get(..3).ok_or("six items")?.to_vec(),
        [
            "# cut: 4 more lines (4 with refs); the whole snapshot is about 100 tokens. \
             page.getByRef('e4').ariaSnapshot() shows the iframe section, which is not shown \
             above. Or pass a larger maxTokens.",
            "- dialog \"Confirm\" [aria-hidden] [ref=e2]:",
            "  - button \"OK\" [ref=e3]",
            "- dialog \"Pay\" [ref=f1e1]:",
            "  - button \"Card\" [ref=f1e2]",
        ]
        .map(str::to_owned)
        .to_vec(),
    ]
    .concat()
    .join("\n");
    for (max_tokens, expected) in [(None, in_place), (Some(30.0), cut)] {
        let passed = Passed {
            text: Some(false),
            max_tokens: max_tokens.and_then(MaxTokens::new),
            ..Passed::default()
        };
        assert_eq!(
            render(&snapshot, Call::Page(&passed)).text(),
            expected,
            "with maxTokens {max_tokens:?}, the dialog in the hidden main says [aria-hidden] in \
             place and at the left edge, the one in the iframe's document does not"
        );
    }
    Ok(())
}

/// What a whole-view layer holds prints first, outside every line around it in the page, so it
/// says the mark of an `aria-hidden` node above it. A child document is not hidden by a mark on
/// its iframe.
#[test]
fn tests_that_what_a_whole_view_layer_holds_keeps_the_mark_of_a_node_above_it()
-> Result<(), Box<dyn Error>> {
    let snapshot = Snapshot {
        tree: serde_json::from_value(json!([
            { "role": "generic", "ariaHidden": true, "children": [
                { "role": "generic", "children": [
                    { "role": "dialog", "name": "Promo", "ref": "e5", "onTop": "e5", "children": [
                        { "role": "button", "name": "Close", "ref": "e6" },
                    ] },
                ] },
            ] },
            { "role": "iframe", "ref": "e8", "ariaHidden": true, "children": [
                { "role": "button", "name": "Framed", "ref": "f1e1", "onTop": "e5" },
            ] },
            { "role": "button", "name": "Page", "ref": "e7" },
        ]))?,
        layers: vec![Layer {
            r#ref: "e5".to_owned(),
            line: "# on top: dialog \"Promo\" [ref=e5] over the whole view".to_owned(),
            whole_view: true,
        }],
        ..Snapshot::default()
    };
    let expected = [
        "# on top: dialog \"Promo\" [ref=e5] over the whole view",
        "- dialog \"Promo\" [aria-hidden] [ref=e5]:",
        "  - button \"Close\" [ref=e6]",
        "- button \"Framed\" [ref=f1e1]",
        "- iframe [aria-hidden] [ref=e8]",
        "- button \"Page\" [ref=e7]",
    ]
    .join("\n");
    assert_eq!(
        render(&snapshot, Call::Page(&Passed::default())).text(),
        expected,
        "the dialog lifted out of the hidden wrapper says [aria-hidden], the button lifted out \
         of the hidden iframe's document does not"
    );
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
fn tests_that_neos_format_prints_every_private_case() -> Result<(), Box<dyn Error>> {
    let Some(folders) = env::var_os("NEO_SNAPSHOT_PRIVATE_CASES") else {
        println!(
            "private sets, Neo's format: SKIPPED (NEO_SNAPSHOT_PRIVATE_CASES names no folder)"
        );
        return Ok(());
    };
    let bundles = Bundles::new()?;
    let mut differing: Vec<String> = Vec::new();
    let (mut texts, mut trees) = (Tally::default(), Tally::default());
    for file in &CaseFile::private_files(&folders)? {
        let shown = file.file_name().unwrap_or_default().to_string_lossy();
        let cases: CaseFile = serde_json::from_str(&fs::read_to_string(file)?)?;
        let file_texts = saved_texts(&cases, &shown, &mut differing)?;
        let file_trees = saved_trees(&cases, &bundles, &shown, &mut differing)?;
        println!(
            "private sets, Neo's format: {shown}: {} of {} saved texts identical, the kept \
             baseline too; {} of {} trees print today's renderer's text and covered block",
            file_texts.identical, file_texts.judged, file_trees.identical, file_trees.judged
        );
        texts.add(file_texts);
        trees.add(file_trees);
    }
    println!(
        "private sets, Neo's format: {} of {} identical, {} of {} trees as today's renderer \
         prints them",
        texts.identical, texts.judged, trees.identical, trees.judged
    );
    assert!(
        trees.judged > 0,
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

/// How many of a set's texts were judged, and how many of those were identical.
#[derive(Clone, Copy, Default)]
struct Tally {
    judged: usize,
    identical: usize,
}

impl Tally {
    fn count(&mut self, identical: bool) {
        self.judged += 1;
        if identical {
            self.identical += 1;
        }
    }

    fn add(&mut self, other: Self) {
        self.judged += other.judged;
        self.identical += other.identical;
    }
}

/// Every saved call in Neo's format in `cases`, against its saved text and kept baseline. A
/// difference is named by `shown`, the case's number, its line and the two lengths.
fn saved_texts(
    cases: &CaseFile,
    shown: &str,
    differing: &mut Vec<String>,
) -> Result<Tally, Box<dyn Error>> {
    let mut tally = Tally::default();
    for (index, found) in cases.rendered_cases(neos).enumerate() {
        let (case, call, name) = found?;
        let answer = render(&cases.snapshot(name)?, call);
        let difference = case.expected().difference(&call, &answer, Shown::Lengths)?;
        if let Some(difference) = &difference {
            differing.push(format!("{shown}: Neo-format case {index}: {difference}"));
        }
        tally.count(difference.is_none());
    }
    Ok(tally)
}

/// Every saved tree in `cases`, not only those a Neo-format call was saved for: Rust's whole page
/// with every default, its text and what it keeps, and its covered block, beside today's
/// renderer's in QuickJS.
fn saved_trees(
    cases: &CaseFile,
    bundles: &Bundles,
    shown: &str,
    differing: &mut Vec<String>,
) -> Result<Tally, Box<dyn Error>> {
    let mut tally = Tally::default();
    let passed = Passed::default();
    let call = Call::Page(&passed);
    for (index, name) in cases.snapshot_names().enumerate() {
        let snapshot = cases.snapshot(name)?;
        let answer = render(&snapshot, call);
        let todays = bundles.todays(&call, &snapshot)?;
        let block = cover_block(snapshot.tree.clone());
        let todays_block = bundles
            .todays_answer(&json!({ "cover": {} }), &snapshot)?
            .text;
        let difference = if let Some(difference) =
            todays
                .expected()
                .difference(&call, &answer, Shown::Lengths)?
        {
            Some(format!("against today's renderer: {difference}"))
        } else if block != todays_block {
            Some(format!(
                "the covered block differs; {:?} bytes printed, {:?} by today's renderer",
                block.as_deref().map(str::len),
                todays_block.as_deref().map(str::len)
            ))
        } else {
            None
        };
        if let Some(difference) = &difference {
            differing.push(format!("{shown}: tree {index}: {difference}"));
        }
        tally.count(difference.is_none());
    }
    Ok(tally)
}

/// Seeded random trees, each in Neo's format as a whole page and as a locator's snapshot with
/// the same options, against today's renderer: the text, and what each keeps.
#[test]
fn tests_that_neos_format_is_todays_on_generated_trees() -> Result<(), Box<dyn Error>> {
    let seed = seed()?;
    let bundles = Bundles::new()?;
    let mut generator = Generator::new(seed);
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_TREES", GATE_TREES)? {
        let snapshot = generator.snapshot();
        let passed = generator.passed();
        for call in [Call::Page(&passed), Call::Element(&passed)] {
            let answer = render(&snapshot, call);
            let todays = bundles.todays(&call, &snapshot)?;
            if let Some(difference) = todays.expected().difference(&call, &answer, Shown::Lines)? {
                panic!("seed {seed:#x}, tree {index}, {call:?}: {difference}\n{snapshot:?}");
            }
        }
    }
    Ok(())
}

/// What a delta stands on: a whole page's text rendered again in the form it keeps is the same
/// text, the length rule not asked again and the text form walked again for the notice.
#[test]
fn tests_that_a_text_rendered_again_in_its_own_form_is_the_same_text() -> Result<(), Box<dyn Error>>
{
    let cases: CaseFile = serde_json::from_str(NEO_CASES)?;
    let mut judged = 0;
    let rendered_again = |snapshot: &Snapshot, passed: &Passed| -> Result<(), String> {
        let answer = render(snapshot, Call::Page(passed));
        let kept = answer.pending().ok_or("a whole page keeps itself")?;
        (neo_text_as(snapshot, &kept.form) == kept.text)
            .then_some(())
            .ok_or_else(|| format!("renders again in its form {:?} as it was", kept.form))
    };
    for found in cases.rendered_cases(|call| matches!(call, Call::Page(_))) {
        let (case, call, name) = found?;
        let Call::Page(passed) = call else {
            continue;
        };
        rendered_again(&cases.snapshot(name)?, passed)
            .map_err(|failed| format!("{}: {failed}", case.name))?;
        judged += 1;
    }
    let seed = seed()?;
    let mut generator = Generator::new(seed);
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_TREES", GATE_TREES)? {
        let snapshot: Snapshot = generator.snapshot();
        rendered_again(&snapshot, &generator.passed())
            .map_err(|failed| format!("seed {seed:#x}, tree {index}: {failed}"))?;
        judged += 1;
    }
    assert!(judged > 0, "texts were rendered again");
    Ok(())
}
