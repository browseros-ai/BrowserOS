//! `ChunkRun` over a hand-built tree and a fake page that runs upstream's loop: every transfer,
//! the three joins from the page to the tree, nested mixed selectors, candidate errors and stale
//! refs reached or not, stale members, strict mode against the probe's recording, the one
//! pin a frame hop takes, and the options of a collapsed select, which reach the page through
//! their select.

use std::collections::HashMap;
use std::error::Error;

use neo_snapshot_apc::proto::{FormControlType, SelectOption};
use neo_snapshot_apc::select::{ChunkPlan, ParsedSelector, RefResolver, RoleQuery};
use neo_snapshot_apc::tree::{DocId, NodeBuilder, NodeId, NodeKey, OptionSlot, SnapshotTree};
use serde_json::{Value, json};

use super::fake_page::{El, FakePage, Sent, build, key, option, plain, tracked};
use super::{ChunkOutcome, ChunkRun, ChunkSet, First, Matched, QueryRoot};
use crate::probe::{ErrorClass, PinPreview, SetMember, StageSet};
use crate::target::Strictness;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// The probe's recorded strict reply (`neo-playwright-js/src/probe/protocol-examples.ts`), which
/// `neo-snapshot-apc/tests/select_match.rs` reproduces from the tree alone.
const PROBE_STRICT: &str = "strict mode violation: getByRole('button') resolved to 2 elements:\n    1) <button>Go</button> aka getByRole('button', { name: 'Go' })\n    2) <button>Stop</button> aka getByRole('button', { name: 'Stop' })\n";

/// The tab's ref table, standing in for `TabRefTable` (engine chunk 5).
#[derive(Default)]
struct Refs(HashMap<String, NodeKey>);

impl RefResolver for Refs {
    fn resolve(&self, body: &str) -> Option<NodeKey> {
        self.0.get(body).cloned()
    }
}

fn button(dom: i32, label: &str) -> El {
    tracked(
        NodeBuilder::element("button").child(NodeBuilder::text(label)),
        dom,
    )
}

fn element(tag: &str, dom: i32) -> El {
    tracked(NodeBuilder::element(tag), dom)
}

/// Two rows of a list, each with a plain cell around one button, a second button in the first
/// row, and a plain panel with a button of its own:
///
/// ```text
///  ul 10
///    li 11 [data-row=a]   span 12 [data-cell] (plain) > button 13 "Edit"; button 14 "Delete"
///    li 21 [data-row=b]   span 22 [data-cell] (plain) > button 23 "Remove"
///  div 30 [data-panel] (plain) > button 31 "Save"
/// ```
fn list_page() -> TestResult<(SnapshotTree, FakePage)> {
    build(vec![
        element("ul", 10).kids(vec![
            element("li", 11).attr("data-row", "a").kids(vec![
                plain(12)
                    .attr("data-cell", "")
                    .kids(vec![button(13, "Edit").attr("data-action", "edit")]),
                button(14, "Delete").attr("data-action", "delete"),
            ]),
            element("li", 21).attr("data-row", "b").kids(vec![
                plain(22)
                    .attr("data-cell", "")
                    .kids(vec![button(23, "Remove").attr("data-action", "remove")]),
            ]),
        ]),
        plain(30)
            .attr("data-panel", "")
            .kids(vec![button(31, "Save").attr("data-action", "save")]),
    ])
}

/// A collapsed select, whose two options the capture lists as entries of the select and the page
/// holds as `<option>` elements, then an ARIA option:
///
/// ```text
///  div 40 [data-form] (plain) > select 50 > option 51 "Hi" (page only), option 52 "Hello" (selected)
///  div 60 role=option aria-selected=true "Other"
/// ```
fn option_page() -> TestResult<(SnapshotTree, FakePage)> {
    let entry = |text: &str, selected: bool| SelectOption {
        value: Some(text.to_lowercase()),
        text: Some(text.to_owned()),
        is_selected: Some(selected),
        is_disabled: Some(false),
    };
    let select = NodeBuilder::form_control(FormControlType::SelectOne)
        .tag("select")
        .select_option(entry("Hi", false))
        .select_option(entry("Hello", true));
    build(vec![
        plain(40)
            .attr("data-form", "")
            .kids(vec![tracked(select, 50).kids(vec![
                option(51).words("<option>Hi</option>", "locator('option').first()"),
                option(52).words("<option>Hello</option>", "locator('option').nth(1)"),
            ])]),
        tracked(
            NodeBuilder::container()
                .role("option")
                .selected(true)
                .child(NodeBuilder::text("Other")),
            60,
        )
        .words(
            "<div role=\"option\" aria-selected=\"true\">Other</div>",
            "locator('div')",
        ),
    ])
}

/// What each node of a tree set is: an option slot's select and index, or another node's DOM id.
fn members(tree: &SnapshotTree, set: &ChunkSet) -> TestResult<Vec<String>> {
    let ChunkSet::Keys(matches) = set else {
        return Err("a tree stage ended the run".into());
    };
    matches
        .nodes()
        .iter()
        .map(|&node: &NodeId| match tree.option_slot(node) {
            Some(OptionSlot { select, index, .. }) => {
                let select = tree.node_key(select).ok_or("a select has a key")?;
                Ok(format!("option {index} of {}", select.node.raw()))
            }
            None => Ok(tree
                .node_key(node)
                .ok_or("an element with a DOM id")?
                .node
                .raw()
                .to_string()),
        })
        .collect()
}

/// The probe recording's page: two buttons, Go and Stop, in a plain container.
fn strict_page() -> TestResult<(SnapshotTree, FakePage)> {
    build(vec![plain(40).kids(vec![
        button(41, "Go").words("<button>Go</button>", "getByRole('button', { name: 'Go' })"),
        button(42, "Stop").words(
            "<button>Stop</button>",
            "getByRole('button', { name: 'Stop' })",
        ),
    ])])
}

fn part(name: &str, body: &str) -> Value {
    json!({ "name": name, "body": body, "source": body })
}

fn role(body: &str) -> Value {
    part("internal:role", body)
}

fn attr(body: &str) -> Value {
    part("internal:attr", body)
}

fn aria_ref(body: &str) -> Value {
    part("aria-ref", body)
}

fn nested(name: &str, parts: &[Value]) -> Value {
    json!({ "name": name, "body": { "parsed": { "parts": parts } }, "source": "nested" })
}

fn chunk(parts: &[Value]) -> Value {
    json!({ "parts": parts })
}

fn plan(chunk: &Value) -> TestResult<ChunkPlan> {
    let parsed: ParsedSelector = serde_json::from_value(chunk.clone())?;
    Ok(ChunkPlan::of(&parsed)?)
}

/// Runs `chunk` over `tree` and `page` from `root`.
async fn run_from(
    tree: &SnapshotTree,
    page: &mut FakePage,
    refs: &Refs,
    root: QueryRoot,
    chunk: &Value,
    strictness: Strictness,
) -> TestResult<ChunkOutcome> {
    let names = tree.name_cache();
    let run = ChunkRun::new(tree, &names, refs, DocId::MAIN, root)
        .ok_or("the capture holds the main document")?;
    Ok(run.run(&plan(chunk)?, strictness, page).await?)
}

async fn run(tree: &SnapshotTree, page: &mut FakePage, chunk: &Value) -> TestResult<ChunkOutcome> {
    run_with(tree, page, &Refs::default(), chunk).await
}

/// Runs `chunk` from the Document, taking the first match, with `refs` as the tab's refs.
async fn run_with(
    tree: &SnapshotTree,
    page: &mut FakePage,
    refs: &Refs,
    chunk: &Value,
) -> TestResult<ChunkOutcome> {
    run_from(
        tree,
        page,
        refs,
        QueryRoot::Document,
        chunk,
        Strictness::FirstMatch,
    )
    .await
}

fn matched(outcome: ChunkOutcome) -> TestResult<Matched> {
    match outcome {
        ChunkOutcome::Matched(matched) => Ok(matched),
        other => Err(format!("the run matched nothing it could report: {other:?}").into()),
    }
}

/// The DOM ids of a final set, from the tree for Keys and from the page for Pins.
fn doms(tree: &SnapshotTree, page: &FakePage, set: &ChunkSet) -> TestResult<Vec<i32>> {
    match set {
        ChunkSet::Keys(matches) => matches
            .nodes()
            .iter()
            .map(|&node| {
                tree.node_key(node)
                    .map(|key| key.node.raw())
                    .ok_or_else(|| "a match is an element with a DOM id".into())
            })
            .collect(),
        ChunkSet::Pins(matches) => Ok(page.doms_of(matches)),
    }
}

/// The DOM ids the run pinned through `hydrate`, call by call.
fn hydrated(page: &FakePage) -> Vec<i32> {
    page.sent
        .iter()
        .filter_map(|sent| match sent {
            Sent::Hydrate(dom) => Some(*dom),
            _ => None,
        })
        .collect()
}

/// The DOM ids each `stage` and `describePins` handed over as call nodes.
fn handed(page: &FakePage) -> Vec<Vec<i32>> {
    page.sent
        .iter()
        .filter_map(|sent| match sent {
            Sent::Stage { handed, .. } | Sent::Describe { handed, .. } => Some(handed.clone()),
            Sent::Hydrate(_) | Sent::Keys(_) => None,
        })
        .collect()
}

fn stage_calls(page: &FakePage) -> Vec<&Sent> {
    page.sent
        .iter()
        .filter(|sent| matches!(sent, Sent::Stage { .. }))
        .collect()
}

/// The candidate sets of the run's last `stage` call.
fn last_sets(page: &FakePage) -> TestResult<Vec<StageSet>> {
    match stage_calls(page).last() {
        Some(Sent::Stage { sets, .. }) => Ok(sets.clone()),
        _ => Err("the run made a stage call".into()),
    }
}

#[tokio::test]
async fn a_pure_tree_chain_runs_over_the_tree_and_makes_no_page_call() -> TestResult {
    let (tree, mut page) = list_page()?;
    let found = matched(
        run(
            &tree,
            &mut page,
            &chunk(&[role("listitem"), role("button")]),
        )
        .await?,
    )?;
    let ChunkSet::Keys(matches) = &found.set else {
        return Err("a tree stage from the Document gives Keys".into());
    };
    assert_eq!(matches.len(), 3, "count() reads the tree's answer");
    assert_eq!(
        doms(&tree, &page, &found.set)?,
        [13, 14, 23],
        "each row's buttons, plain cells and all"
    );
    assert!(page.sent.is_empty(), "no hydration and no probe call");
    Ok(())
}

#[tokio::test]
async fn a_tree_stage_from_an_untracked_root_runs_its_page_form() -> TestResult {
    let (tree, mut page) = list_page()?;
    page.root_at(30);
    let outcome = run_from(
        &tree,
        &mut page,
        &Refs::default(),
        QueryRoot::Untracked,
        &chunk(&[role("button")]),
        Strictness::FirstMatch,
    )
    .await?;
    let found = matched(outcome)?;
    assert!(
        matches!(found.set, ChunkSet::Pins(_)),
        "the page answered the stage"
    );
    assert_eq!(
        doms(&tree, &page, &found.set)?,
        [31],
        "the panel's button: the page scopes the document's candidates to its root"
    );
    assert_eq!(
        handed(&page),
        [vec![13, 14, 23, 31]],
        "candidates are the whole document's, handed over with the call"
    );
    assert!(
        hydrated(&page).is_empty(),
        "nothing pinned to hand them over"
    );
    let [Sent::Stage { from, sets, .. }] = stage_calls(&page)[..] else {
        return Err("one stage call".into());
    };
    assert_eq!(*from, None, "the stage starts from the query root");
    assert_eq!(sets.len(), 1, "one candidate set for the one leaf");
    Ok(())
}

#[tokio::test]
async fn an_element_handles_node_roots_every_tree_stage() -> TestResult {
    let (tree, mut page) = list_page()?;
    page.root_at(11);
    let row = tree
        .node_by_key(&key(11)?)
        .ok_or("row a is a tree node")?
        .id();
    let refs = Refs::default();
    let root = QueryRoot::Node(row);
    let buttons = chunk(&[role("button")]);
    let below = matched(
        run_from(
            &tree,
            &mut page,
            &refs,
            root,
            &buttons,
            Strictness::FirstMatch,
        )
        .await?,
    )?;
    assert_eq!(doms(&tree, &page, &below.set)?, [13, 14], "row a's buttons");
    assert!(page.sent.is_empty(), "the row's subtree is in the tree");

    // Path 2 runs the or over the tree from the query root: the row, not the Document.
    let edit_or_button = chunk(&[
        attr("[data-action=\"edit\"]"),
        nested("internal:or", &[role("button")]),
    ]);
    let or = matched(
        run_from(
            &tree,
            &mut page,
            &refs,
            root,
            &edit_or_button,
            Strictness::FirstMatch,
        )
        .await?,
    )?;
    assert!(
        matches!(or.set, ChunkSet::Keys(_)),
        "the or ran over the tree"
    );
    assert_eq!(
        doms(&tree, &page, &or.set)?,
        [13, 14],
        "internal:or adds the row's buttons, and none outside it"
    );
    Ok(())
}

#[tokio::test]
async fn candidates_are_the_whole_documents_since_a_stage_can_leave_its_query_root() -> TestResult {
    let (tree, mut page) = list_page()?;
    page.root_at(11);
    let row = tree
        .node_by_key(&key(11)?)
        .ok_or("row a is a tree node")?
        .id();
    // `..` climbs from row a to the list, and the chain then reaches row b's cell.
    let sent = chunk(&[
        part("xpath", ".."),
        nested("internal:chain", &[attr("[data-cell]"), role("button")]),
    ]);
    let found = matched(
        run_from(
            &tree,
            &mut page,
            &Refs::default(),
            QueryRoot::Node(row),
            &sent,
            Strictness::FirstMatch,
        )
        .await?,
    )?;
    assert_eq!(
        doms(&tree, &page, &found.set)?,
        [13, 23],
        "both cells' buttons, though row b's lies outside the query root"
    );
    Ok(())
}

#[tokio::test]
async fn a_tree_set_hands_the_document_over_as_its_marker() -> TestResult {
    let (tree, mut page) = list_page()?;
    // The tree run keeps the query root: `nth=0`, then an `or` that adds no dialog.
    let sent = chunk(&[
        part("nth", "0"),
        nested("internal:or", &[role("dialog")]),
        attr("[data-row=\"a\"]"),
    ]);
    let found = matched(run(&tree, &mut page, &sent).await?)?;
    assert_eq!(
        doms(&tree, &page, &found.set)?,
        [11],
        "row a, found from the Document"
    );
    let [Sent::Stage { from, handed, .. }] = stage_calls(&page)[..] else {
        return Err("one stage call".into());
    };
    assert_eq!(
        *from,
        Some(vec![SetMember::Document]),
        "the document's root node stands for the Document"
    );
    assert!(handed.is_empty(), "the Document is never hydrated");
    Ok(())
}

#[tokio::test]
async fn a_page_stage_from_the_root_sends_its_parts_as_they_are() -> TestResult {
    let (tree, mut page) = list_page()?;
    let sent = chunk(&[attr("[data-action=\"remove\"]")]);
    let found = matched(run(&tree, &mut page, &sent).await?)?;
    assert_eq!(
        doms(&tree, &page, &found.set)?,
        [23],
        "the page's own engine"
    );
    assert_eq!(
        page.sent,
        [Sent::Stage {
            selector: sent,
            from: None,
            sets: Vec::new(),
            strict: false,
            handed: Vec::new(),
        }],
        "one call, the chunk unchanged, nothing hydrated"
    );
    Ok(())
}

#[tokio::test]
async fn keys_to_pins_hydrates_only_the_tree_stages_matches() -> TestResult {
    let (tree, mut page) = list_page()?;
    let found = matched(
        run(
            &tree,
            &mut page,
            &chunk(&[role("listitem"), attr("[data-cell]")]),
        )
        .await?,
    )?;
    assert_eq!(
        doms(&tree, &page, &found.set)?,
        [12, 22],
        "each row's plain cell"
    );
    assert_eq!(
        handed(&page),
        [vec![11, 21]],
        "the two rows, and no other element"
    );
    let [
        Sent::Stage {
            from: Some(from), ..
        },
    ] = stage_calls(&page)[..]
    else {
        return Err("one stage call, from the rows".into());
    };
    assert_eq!(
        *from,
        [SetMember::Node(0), SetMember::Node(1)],
        "the rows start the stage as the call's nodes, in order"
    );
    Ok(())
}

#[tokio::test]
async fn pins_to_keys_joins_tree_node_roots_with_no_candidate_hydration() -> TestResult {
    let (tree, mut page) = list_page()?;
    let found = matched(
        run(
            &tree,
            &mut page,
            &chunk(&[attr("[data-row=\"a\"]"), role("button")]),
        )
        .await?,
    )?;
    assert!(
        matches!(found.set, ChunkSet::Keys(_)),
        "path 2: the row is a tree node, so the role stage ran over the tree"
    );
    assert_eq!(doms(&tree, &page, &found.set)?, [13, 14], "row a's buttons");
    assert!(
        matches!(page.sent[..], [Sent::Stage { .. }, Sent::Keys(_)]),
        "the attribute stage, then path 1's one keys call: nothing hydrated, no second stage"
    );
    Ok(())
}

#[tokio::test]
async fn a_document_match_joins_as_the_documents_root_with_no_call() -> TestResult {
    let (tree, mut page) = list_page()?;
    // `nth=0` keeps the Document, and `internal:or` adds row b after it.
    let sent = chunk(&[
        part("nth", "0"),
        nested("internal:or", &[attr("[data-row=\"b\"]")]),
        role("button"),
    ]);
    let found = matched(run(&tree, &mut page, &sent).await?)?;
    assert_eq!(
        doms(&tree, &page, &found.set)?,
        [13, 14, 23, 31],
        "the buttons below the Document, then row b's, already in the set"
    );
    let Some(Sent::Keys(pins)) = page.sent.get(1) else {
        return Err("path 1 asks the keys of the pinned roots".into());
    };
    assert_eq!(pins.len(), 1, "row b only: the Document needs no call");
    Ok(())
}

#[tokio::test]
async fn path_3_runs_the_page_form_from_plain_element_roots() -> TestResult {
    let (tree, mut page) = list_page()?;
    let found = matched(
        run(
            &tree,
            &mut page,
            &chunk(&[attr("[data-panel]"), role("button")]),
        )
        .await?,
    )?;
    assert!(matches!(found.set, ChunkSet::Pins(_)), "the page answered");
    assert_eq!(doms(&tree, &page, &found.set)?, [31], "the panel's button");
    assert!(
        matches!(
            page.sent[..],
            [
                Sent::Stage { from: None, .. },
                Sent::Keys(_),
                Sent::Stage { from: Some(_), .. }
            ]
        ),
        "the panel, its key (not a tree node), the page form from the panel: {:?}",
        page.sent
    );
    assert_eq!(
        handed(&page)[1],
        [13, 14, 23, 31],
        "the page form hands over the document's buttons"
    );
    Ok(())
}

#[tokio::test]
async fn a_page_stage_from_pins_hands_them_over_as_they_are() -> TestResult {
    let (tree, mut page) = list_page()?;
    let found = matched(
        run(
            &tree,
            &mut page,
            &chunk(&[
                attr("[data-panel]"),
                role("button"),
                nested("internal:and", &[attr("[data-action=\"save\"]")]),
            ]),
        )
        .await?,
    )?;
    assert_eq!(
        doms(&tree, &page, &found.set)?,
        [31],
        "the panel's button is the one with data-action=save"
    );
    let stages = stage_calls(&page);
    let [
        _,
        _,
        Sent::Stage {
            from: Some(from),
            sets,
            ..
        },
    ] = stages[..]
    else {
        return Err(format!("three stage calls: {:?}", page.sent).into());
    };
    assert!(sets.is_empty(), "the last stage has no candidates");
    assert!(
        matches!(from[..], [SetMember::Pin(_)]),
        "the panel's button, by the pin path 3 gave it"
    );
    assert_eq!(
        handed(&page)[2],
        Vec::<i32>::new(),
        "an and over CSS has no candidate to hand over"
    );
    Ok(())
}

#[tokio::test]
async fn nested_mixed_selectors_keep_upstreams_roots() -> TestResult {
    let (tree, mut page) = list_page()?;
    let cell_button = |name: &str| {
        vec![
            attr("[data-cell]"),
            role(&format!("button[name=\"{name}\"i]")),
        ]
    };
    let cases: [(Value, &[i32], &str); 5] = [
        (
            chunk(&[
                role("listitem"),
                nested("internal:has", &cell_button("Edit")),
            ]),
            &[11],
            "has: row a's cell holds Edit, row b's holds Remove",
        ),
        (
            chunk(&[
                role("listitem"),
                nested("internal:has-not", &cell_button("Edit")),
            ]),
            &[21],
            "has-not",
        ),
        (
            chunk(&[
                role("listitem"),
                nested("internal:chain", &[attr("[data-cell]"), role("button")]),
            ]),
            &[13, 23],
            "chain: from each row, its cell's buttons",
        ),
        (
            chunk(&[
                role("button"),
                nested("internal:and", &[attr("[data-row=\"a\"]"), role("button")]),
            ]),
            &[13, 14],
            "and: its nested selector runs from the query root",
        ),
        (
            chunk(&[
                attr("[data-action=\"save\"]"),
                nested("internal:or", &[attr("[data-row=\"b\"]"), role("button")]),
            ]),
            &[23, 31],
            "or: the union in document order",
        ),
    ];
    for (sent, expected, why) in cases {
        let found = matched(run(&tree, &mut page, &sent).await?)?;
        assert_eq!(doms(&tree, &page, &found.set)?, expected, "{why}");
    }
    Ok(())
}

#[tokio::test]
async fn a_candidate_error_surfaces_only_once_a_root_reaches_it() -> TestResult {
    let (tree, mut page) = list_page()?;
    let error_of = |body: &str| {
        RoleQuery::parse(body)
            .err()
            .map(|error| error.to_string())
            .ok_or(format!("{body} is not a role query"))
    };
    let invalid = "button[pressed=maybe]";
    let validation = error_of(invalid)?;
    let cell_with = |body: &str| nested("internal:has", &[attr("[data-cell]"), role(body)]);

    let unreached = matched(
        run(
            &tree,
            &mut page,
            &chunk(&[attr("[data-none]"), cell_with(invalid)]),
        )
        .await?,
    )?;
    assert_eq!(
        doms(&tree, &page, &unreached.set)?,
        Vec::<i32>::new(),
        "no root reaches the part, so nothing throws"
    );

    // The page throws a candidate's error with the class Rust sent, and the run relays the
    // page's error reply as it stands: each class below reaches the outcome only through the page.
    for (body, message, class, why) in [
        (
            invalid,
            validation,
            ErrorClass::Script,
            "a plain Error, as the role engine's validation throws it",
        ),
        (
            "button[",
            error_of("button[")?,
            ErrorClass::Selector,
            "upstream's InvalidSelectorError, from parseAttributeSelector",
        ),
    ] {
        page.sent.clear();
        let reached = run(
            &tree,
            &mut page,
            &chunk(&[attr("[data-row=\"a\"]"), cell_with(body)]),
        )
        .await?;
        assert_eq!(
            last_sets(&page)?,
            [StageSet::Error {
                message: message.clone(),
                class
            }],
            "the set carries the error and its class: {why}"
        );
        let ChunkOutcome::Thrown {
            message: thrown,
            class: thrown_class,
        } = reached
        else {
            return Err(format!("row a reaches the part: {reached:?}").into());
        };
        assert_eq!(
            (thrown, thrown_class),
            (message, class),
            "upstream's text and class: {why}"
        );
        assert_eq!(page.stages(), 1, "one call, and the page threw in it");
    }

    let over_the_tree = run(
        &tree,
        &mut page,
        &chunk(&[attr("[data-none]"), role(invalid)]),
    )
    .await?;
    assert!(
        matches!(over_the_tree, ChunkOutcome::Matched(_)),
        "the tree stage on an empty set reaches no root either"
    );
    let thrown_in_rust = run(
        &tree,
        &mut page,
        &chunk(&[attr("[data-row=\"a\"]"), role(invalid)]),
    )
    .await?;
    assert!(
        matches!(
            thrown_in_rust,
            ChunkOutcome::Thrown {
                class: ErrorClass::Script,
                ..
            }
        ),
        "path 2 reaches the part in Rust and throws the same: {thrown_in_rust:?}"
    );
    Ok(())
}

#[tokio::test]
async fn stale_members_make_the_run_stale() -> TestResult {
    let rows_then_cells = chunk(&[role("listitem"), attr("[data-cell]")]);

    let (tree, mut page) = list_page()?;
    page.remove(21);
    assert!(
        matches!(
            run(&tree, &mut page, &rows_then_cells).await?,
            ChunkOutcome::Stale
        ),
        "a matched row left the page after the capture: the page answers detached"
    );

    let (tree, mut page) = list_page()?;
    page.destroy(21);
    assert!(
        matches!(
            run(&tree, &mut page, &rows_then_cells).await?,
            ChunkOutcome::Stale
        ),
        "a matched row no longer resolves: the seam cannot hand it over"
    );

    let (tree, mut page) = list_page()?;
    page.remove_after(1, 11);
    assert!(
        matches!(
            run(
                &tree,
                &mut page,
                &chunk(&[attr("[data-row=\"a\"]"), role("button")])
            )
            .await?,
            ChunkOutcome::Stale
        ),
        "a pinned row left before path 1 asked its key"
    );
    Ok(())
}

#[tokio::test]
async fn a_candidate_that_left_the_page_is_only_filtered_out() -> TestResult {
    let (tree, mut page) = list_page()?;
    page.remove(13);
    let found = matched(
        run(
            &tree,
            &mut page,
            &chunk(&[
                role("listitem"),
                nested("internal:has", &[attr("[data-cell]"), role("button")]),
            ]),
        )
        .await?,
    )?;
    assert_eq!(
        doms(&tree, &page, &found.set)?,
        [21],
        "row a's cell lost its button, so only row b holds one; the run goes on"
    );
    Ok(())
}

#[tokio::test]
async fn a_stale_ref_counts_only_once_a_root_reaches_it() -> TestResult {
    let (tree, mut page) = list_page()?;
    let mut refs = Refs::default();
    refs.0.insert("e1".to_owned(), key(13)?);
    let cell_ref = |name: &str, body: &str| nested(name, &[attr("[data-cell]"), aria_ref(body)]);

    let in_tree = matched(
        run_with(
            &tree,
            &mut page,
            &refs,
            &chunk(&[role("listitem"), aria_ref("e99")]),
        )
        .await?,
    )?;
    assert_eq!(
        in_tree.stale_ref.as_deref(),
        Some("e99"),
        "a tree stage reports the ref its roots reached"
    );

    page.sent.clear();
    let unreached = matched(
        run_with(
            &tree,
            &mut page,
            &refs,
            &chunk(&[attr("[data-none]"), cell_ref("internal:has", "e99")]),
        )
        .await?,
    )?;
    assert_eq!(unreached.stale_ref, None, "no root reached the ref");
    assert_eq!(page.stages(), 1, "one call, which reached nothing");
    assert_eq!(
        last_sets(&page)?,
        [StageSet::StaleRef("e99".to_owned())],
        "the ref goes to the page as a stale-ref set"
    );

    page.sent.clear();
    let reached = matched(
        run_with(
            &tree,
            &mut page,
            &refs,
            &chunk(&[role("listitem"), cell_ref("internal:has", "e99")]),
        )
        .await?,
    )?;
    assert_eq!(
        reached.stale_ref.as_deref(),
        Some("e99"),
        "the rows reached it, and the page's reply says so"
    );
    assert_eq!(
        doms(&tree, &page, &reached.set)?,
        Vec::<i32>::new(),
        "nothing matched"
    );
    assert_eq!(page.stages(), 1, "one call, never sent again");

    let kept = matched(
        run_with(
            &tree,
            &mut page,
            &refs,
            &chunk(&[role("listitem"), cell_ref("internal:has-not", "e99")]),
        )
        .await?,
    )?;
    assert_eq!(
        doms(&tree, &page, &kept.set)?,
        [11, 21],
        "has-not over a ref that matches nothing keeps every row"
    );
    assert_eq!(kept.stale_ref.as_deref(), Some("e99"), "and reports it");

    let live = matched(
        run_with(
            &tree,
            &mut page,
            &refs,
            &chunk(&[role("listitem"), cell_ref("internal:has", "e1")]),
        )
        .await?,
    )?;
    assert_eq!(
        doms(&tree, &page, &live.set)?,
        [11],
        "e1 is row a's Edit button"
    );
    assert_eq!(live.stale_ref, None, "a live ref is not stale");
    Ok(())
}

#[tokio::test]
async fn a_run_keeps_the_first_stale_ref_of_all_its_stages() -> TestResult {
    let (tree, mut page) = list_page()?;
    // The tree stage reaches e98 first. The cells are plain, so the last stage runs in the page,
    // which reaches e99 and says so.
    let found = matched(
        run(
            &tree,
            &mut page,
            &chunk(&[
                role("listitem"),
                nested("internal:has-not", &[aria_ref("e98")]),
                attr("[data-cell]"),
                nested("internal:has", &[aria_ref("e99")]),
            ]),
        )
        .await?,
    )?;
    assert_eq!(
        last_sets(&page)?,
        [StageSet::StaleRef("e99".to_owned())],
        "the last stage took e99 to the page"
    );
    assert_eq!(
        found.stale_ref.as_deref(),
        Some("e98"),
        "the page's later stale ref does not replace the tree stage's"
    );
    Ok(())
}

#[tokio::test]
async fn strict_mode_on_a_tree_set_hydrates_only_the_previews() -> TestResult {
    let (tree, mut page) = strict_page()?;
    let sent = chunk(&[role("button")]);
    page.expect_chunk(sent.clone(), "getByRole('button')");
    let outcome = run_from(
        &tree,
        &mut page,
        &Refs::default(),
        QueryRoot::Document,
        &sent,
        Strictness::Strict,
    )
    .await?;
    let ChunkOutcome::Strict(message) = outcome else {
        return Err(format!("two buttons violate strict mode: {outcome:?}").into());
    };
    assert_eq!(message, PROBE_STRICT, "the recorded text, byte for byte");
    assert_eq!(
        page.sent,
        [Sent::Describe {
            members: vec![SetMember::Node(0), SetMember::Node(1)],
            handed: vec![41, 42],
        }],
        "decided in Rust: one call describes the previews, and no stage"
    );

    let eleven: Vec<El> = (1..=11)
        .map(|n| {
            button(100 + n, &format!("B{n}")).words(
                &format!("<button>B{n}</button>"),
                &format!("getByRole('button', {{ name: 'B{n}' }})"),
            )
        })
        .collect();
    let (tree, mut page) = build(eleven)?;
    page.expect_chunk(sent.clone(), "getByRole('button')");
    let outcome = run_from(
        &tree,
        &mut page,
        &Refs::default(),
        QueryRoot::Document,
        &sent,
        Strictness::Strict,
    )
    .await?;
    let ChunkOutcome::Strict(message) = outcome else {
        return Err("eleven buttons violate strict mode".into());
    };
    assert_eq!(
        handed(&page),
        [(101..=110).collect::<Vec<i32>>()],
        "the first ten, and not the eleventh"
    );
    assert!(
        message.starts_with("strict mode violation: getByRole('button') resolved to 11 elements:")
            && message.ends_with(
                "\n    10) <button>B10</button> aka getByRole('button', { name: 'B10' })\n    ...\n"
            ),
        "ten lines, then `...` for the eleventh: {message}"
    );
    Ok(())
}

#[tokio::test]
async fn strict_mode_on_a_page_set_relays_the_pages_message() -> TestResult {
    let (tree, mut page) = strict_page()?;
    page.root_at(40);
    let sent = chunk(&[role("button")]);
    page.expect_chunk(sent.clone(), "getByRole('button')");
    let outcome = run_from(
        &tree,
        &mut page,
        &Refs::default(),
        QueryRoot::Untracked,
        &sent,
        Strictness::Strict,
    )
    .await?;
    let ChunkOutcome::Strict(message) = outcome else {
        return Err(format!("two buttons violate strict mode: {outcome:?}").into());
    };
    assert_eq!(message, PROBE_STRICT, "the recorded text, byte for byte");
    assert!(
        matches!(stage_calls(&page)[..], [Sent::Stage { strict: true, .. }]),
        "the last page call carries the chunk"
    );
    assert_eq!(
        handed(&page),
        [vec![41, 42]],
        "the candidates, as call nodes"
    );

    let (tree, mut page) = list_page()?;
    let sent = chunk(&[attr("[data-panel]"), role("button")]);
    page.expect_chunk(sent.clone(), "locator('[data-panel]').getByRole('button')");
    let found = matched(
        run_from(
            &tree,
            &mut page,
            &Refs::default(),
            QueryRoot::Document,
            &sent,
            Strictness::Strict,
        )
        .await?,
    )?;
    assert_eq!(doms(&tree, &page, &found.set)?, [31], "one match passes");
    assert!(
        matches!(
            stage_calls(&page)[..],
            [
                Sent::Stage { strict: false, .. },
                Sent::Stage { strict: true, .. }
            ]
        ),
        "only the call that ends the chunk counts its matches"
    );
    Ok(())
}

#[tokio::test]
async fn first_match_takes_the_whole_set() -> TestResult {
    let (tree, mut page) = strict_page()?;
    let found = matched(run(&tree, &mut page, &chunk(&[role("button")])).await?)?;
    assert_eq!(doms(&tree, &page, &found.set)?, [41, 42], "both buttons");
    assert!(page.sent.is_empty(), "no violation to describe");
    Ok(())
}

#[tokio::test]
async fn a_frame_hop_takes_one_pin() -> TestResult {
    let (tree, mut page) = list_page()?;
    let names = tree.name_cache();
    let refs = Refs::default();
    let chunk_run = ChunkRun::new(&tree, &names, &refs, DocId::MAIN, QueryRoot::Document)
        .ok_or("the capture holds the main document")?;

    let keys = matched(
        chunk_run
            .run(
                &plan(&chunk(&[role("button")]))?,
                Strictness::FirstMatch,
                &mut page,
            )
            .await?,
    )?;
    let First::Pin(pin) = chunk_run.first(&keys.set, &mut page).await? else {
        return Err("the first button, pinned".into());
    };
    assert_eq!(page.dom_of(pin), 13, "the first in tree order");
    assert_eq!(hydrated(&page), [13], "a tree set hydrates that one key");

    let pins = matched(
        chunk_run
            .run(
                &plan(&chunk(&[attr("[data-action]")]))?,
                Strictness::FirstMatch,
                &mut page,
            )
            .await?,
    )?;
    let calls = page.sent.len();
    let First::Pin(pin) = chunk_run.first(&pins.set, &mut page).await? else {
        return Err("the first match, already pinned".into());
    };
    assert_eq!(page.dom_of(pin), 13, "the page's first match");
    assert_eq!(page.sent.len(), calls, "a pin needs no call");

    let none = matched(
        chunk_run
            .run(
                &plan(&chunk(&[role("dialog")]))?,
                Strictness::FirstMatch,
                &mut page,
            )
            .await?,
    )?;
    assert_eq!(
        chunk_run.first(&none.set, &mut page).await?,
        First::Missing,
        "nothing to hop into yet"
    );
    let document = matched(
        chunk_run
            .run(
                &plan(&chunk(&[part("nth", "0")]))?,
                Strictness::FirstMatch,
                &mut page,
            )
            .await?,
    )?;
    assert_eq!(
        chunk_run.first(&document.set, &mut page).await?,
        First::Document,
        "nth=0 keeps the Document, which is no iframe"
    );
    page.remove(13);
    assert_eq!(
        chunk_run.first(&keys.set, &mut page).await?,
        First::Stale,
        "the first key's element left the page since the capture"
    );
    Ok(())
}

#[tokio::test]
async fn nth_with_a_capture_throws_as_the_run_starts() -> TestResult {
    let (tree, mut page) = list_page()?;
    let mut sent = chunk(&[role("list"), part("nth", "0")]);
    sent["capture"] = json!(0);
    let outcome = run(&tree, &mut page, &sent).await?;
    let ChunkOutcome::Thrown { message, class } = outcome else {
        return Err(format!("upstream throws before any part runs: {outcome:?}").into());
    };
    assert_eq!(
        message, "Can't query n-th element in a request with the capture.",
        "upstream's text"
    );
    assert_eq!(
        class,
        ErrorClass::Script,
        "a stackless Error, which is `script`"
    );
    assert!(page.sent.is_empty(), "nothing reached the page");
    Ok(())
}

#[test]
fn page_text_in_an_outcome_or_a_preview_never_prints_through_debug() {
    let secret = "hunter2";
    let printed = [
        format!(
            "{:?}",
            ChunkOutcome::Strict(format!("strict mode violation: <input value=\"{secret}\">"))
        ),
        format!(
            "{:?}",
            ChunkOutcome::Thrown {
                message: format!("Unknown attribute \"{secret}\""),
                class: ErrorClass::Script,
            }
        ),
        format!(
            "{:?}",
            PinPreview {
                preview: format!("<input value=\"{secret}\">"),
                aka: format!("getByRole('textbox', {{ name: '{secret}' }})"),
            }
        ),
    ];
    for line in printed {
        assert!(
            !line.contains(secret),
            "a strict message, a thrown message and a preview print their lengths only: {line}"
        );
    }
}

#[tokio::test]
async fn a_tree_set_hands_an_option_over_as_its_select_and_index() -> TestResult {
    let (tree, mut page) = option_page()?;
    let names = tree.name_cache();
    let refs = Refs::default();
    let chunk_run = ChunkRun::new(&tree, &names, &refs, DocId::MAIN, QueryRoot::Document)
        .ok_or("the capture holds the main document")?;
    let found = matched(
        chunk_run
            .run(
                &plan(&chunk(&[role("option[selected=true]")]))?,
                Strictness::FirstMatch,
                &mut page,
            )
            .await?,
    )?;
    assert_eq!(
        members(&tree, &found.set)?,
        ["option 1 of 50", "60"],
        "the selected option and the aria-selected div, in tree order (SE-051)"
    );
    assert!(page.sent.is_empty(), "matched over the tree alone");

    let super::Pinned::Members(pins) = chunk_run.pinned(found.set, &mut page).await? else {
        return Err("both members are live".into());
    };
    assert_eq!(
        page.doms_of(&pins),
        [52, 60],
        "the page's own <option> and the div"
    );
    assert_eq!(
        page.sent,
        [Sent::Stage {
            selector: json!({ "parts": [] }),
            from: Some(vec![
                SetMember::Option {
                    select: 0,
                    index: 1,
                    count: 2
                },
                SetMember::Node(1),
            ]),
            sets: Vec::new(),
            strict: false,
            handed: vec![50, 60],
        }],
        "one stage: the option goes over as its select, its index and the list's length"
    );
    assert!(
        hydrated(&page).is_empty(),
        "an option has no key to hydrate"
    );
    Ok(())
}

#[tokio::test]
async fn the_first_option_is_pinned_through_one_stage() -> TestResult {
    let (tree, mut page) = option_page()?;
    let names = tree.name_cache();
    let refs = Refs::default();
    let chunk_run = ChunkRun::new(&tree, &names, &refs, DocId::MAIN, QueryRoot::Document)
        .ok_or("the capture holds the main document")?;
    let options = matched(
        chunk_run
            .run(
                &plan(&chunk(&[role("option")]))?,
                Strictness::FirstMatch,
                &mut page,
            )
            .await?,
    )?;
    let First::Pin(pin) = chunk_run.first(&options.set, &mut page).await? else {
        return Err("the first option, pinned".into());
    };
    assert_eq!(page.dom_of(pin), 51, "select.options[0]");
    assert!(
        hydrated(&page).is_empty(),
        "pinned by a stage, not hydrated"
    );
    assert_eq!(handed(&page), [vec![50]], "its select, handed over once");
    Ok(())
}

#[tokio::test]
async fn an_option_list_that_changed_since_the_capture_makes_the_run_stale() -> TestResult {
    let (tree, mut page) = option_page()?;
    let names = tree.name_cache();
    let refs = Refs::default();
    let chunk_run = ChunkRun::new(&tree, &names, &refs, DocId::MAIN, QueryRoot::Document)
        .ok_or("the capture holds the main document")?;
    let hello = chunk(&[role("option[name=\"Hello\"i]")]);
    let in_form = chunk(&[
        attr("[data-form]"),
        nested("internal:has", &[role("option[name=\"Hello\"i]")]),
    ]);
    let found = matched(
        chunk_run
            .run(&plan(&in_form)?, Strictness::FirstMatch, &mut page)
            .await?,
    )?;
    let ChunkSet::Pins(pins) = &found.set else {
        return Err("a page stage ended the run".into());
    };
    assert_eq!(page.doms_of(pins), [40], "the form holds the Hello option");

    let options = matched(
        chunk_run
            .run(&plan(&hello)?, Strictness::FirstMatch, &mut page)
            .await?,
    )?;
    // The page drops "Hi": select.options[1] no longer names the option the capture saw.
    page.remove(51);
    assert!(
        matches!(
            chunk_run.first(&options.set, &mut page).await?,
            First::Stale
        ),
        "the first option of a list that changed is stale"
    );
    assert!(
        matches!(
            chunk_run.pinned(options.set, &mut page).await?,
            super::Pinned::Stale
        ),
        "so is a whole set"
    );
    assert!(
        matches!(
            chunk_run
                .run(&plan(&in_form)?, Strictness::FirstMatch, &mut page)
                .await?,
            ChunkOutcome::Stale
        ),
        "and a candidate set naming it: the option may now be another, which no containment \
         test can tell"
    );
    Ok(())
}

#[tokio::test]
async fn strict_mode_previews_options_through_their_select() -> TestResult {
    let (tree, mut page) = option_page()?;
    let sent = chunk(&[role("option")]);
    page.expect_chunk(sent.clone(), "getByRole('option')");
    let outcome = run_from(
        &tree,
        &mut page,
        &Refs::default(),
        QueryRoot::Document,
        &sent,
        Strictness::Strict,
    )
    .await?;
    let ChunkOutcome::Strict(message) = outcome else {
        return Err(format!("three options violate strict mode: {outcome:?}").into());
    };
    assert_eq!(
        page.sent,
        [Sent::Describe {
            members: vec![
                SetMember::Option {
                    select: 0,
                    index: 0,
                    count: 2
                },
                SetMember::Option {
                    select: 0,
                    index: 1,
                    count: 2
                },
                SetMember::Node(1),
            ],
            handed: vec![50, 60],
        }],
        "one call describes the previews; the select goes over once for both options"
    );
    assert_eq!(
        message,
        "strict mode violation: getByRole('option') resolved to 3 elements:\n    \
         1) <option>Hi</option> aka getByRole('option', { name: 'Hi' })\n    \
         2) <option>Hello</option> aka getByRole('option', { name: 'Hello' })\n    \
         3) <div role=\"option\" aria-selected=\"true\">Other</div> aka getByRole('option', { name: 'Other' })\n",
        "the page's previews, with the role locator upstream's generator prefers"
    );
    Ok(())
}
