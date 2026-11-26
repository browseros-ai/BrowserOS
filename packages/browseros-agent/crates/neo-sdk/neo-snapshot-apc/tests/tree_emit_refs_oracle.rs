//! Offline parity checks comparing [`neo_snapshot_apc::tree::SnapshotTree::aria`] emission,
//! [`neo_snapshot_apc::refs::TabRefTable`] ref assignment/resolution, and [`neo_snapshot::render`]
//! formatting against the 16 recorded Playwright oracles (`tests/fixtures/<name>.{pb,oracle.json}`)
//! plus contract-v4 mirrors of five key edge fixtures.

#[path = "support/fixture_pages.rs"]
#[expect(
    dead_code,
    reason = "only FIXTURES and CANARY_PREFIX are read here; page loaders serve the recorder"
)]
mod fixture_pages;
#[path = "support/fixture_server.rs"]
#[expect(
    dead_code,
    reason = "fixture_pages names fixture_server::Page; the server serves the recorder"
)]
mod fixture_server;
#[path = "support/oracle.rs"]
#[expect(
    dead_code,
    reason = "oracle::recorded_url serves tests/oracles.rs; only Oracle types are read here"
)]
mod oracle;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::path::PathBuf;
use std::{fs, mem};

use fixture_pages::{CANARY_PREFIX, FIXTURES};
use neo_snapshot::{
    AriaChild, AriaNode, Baseline, Call, Earlier, Invalid, Passed, Tristate, json_depth, render,
};
use neo_snapshot_apc::proto::{
    AnnotatedPageContent, AnnotatedRole, BrowserOsPageContentData, BrowserOsRef, BrowserOsTriState,
    ClickabilityReason, FormControlType, RedactionDecision, SelectOption,
};
use neo_snapshot_apc::refs::{RefLookup, RefTarget, TabRefTable};
use neo_snapshot_apc::tree::{
    ApcBuilder, AriaAsk, AriaTree, BuildError, BuildOptions, DocId, DocToken, DomNodeId,
    FrameBuilder, InvalidState, MAX_JSON_DEPTH, MIN_CONTRACT_VERSION, NameCache, NameVisibility,
    NodeBuilder, NodeKey, Role, SnapshotTree, State, TriState,
};
use oracle::{Oracle, OracleElement, OracleFrame, OracleStates};
use serde::Deserialize;
use serde_json::{Deserializer, Number, Value};

const CONTRACT_V4: u32 = 4;
const NO_CHANGE_LINE: &str = "# no change since your last snapshot of this tab";
const NAME_PROHIBITED_ROLES: &[&str] = &[
    "caption",
    "code",
    "definition",
    "deletion",
    "emphasis",
    "generic",
    "insertion",
    "mark",
    "paragraph",
    "presentation",
    "strong",
    "subscript",
    "suggestion",
    "superscript",
    "term",
    "time",
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn load_pb(name: &str) -> Result<AnnotatedPageContent, Box<dyn Error>> {
    let path = fixtures_dir().join(format!("{name}.pb"));
    let bytes = fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(neo_snapshot_apc::decode_protobuf(&bytes)?)
}

fn load_oracle(name: &str) -> Result<Oracle, Box<dyn Error>> {
    let path = fixtures_dir().join(format!("{name}.oracle.json"));
    let text = fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut reader = Deserializer::from_str(&text);
    reader.disable_recursion_limit();
    let oracle =
        Oracle::deserialize(&mut reader).map_err(|err| format!("{}: {err}", path.display()))?;
    reader.end()?;
    Ok(oracle)
}

fn parse_oracle_aria(value: &Value) -> Result<Vec<AriaNode>, Box<dyn Error>> {
    let text = serde_json::to_string(value)?;
    let mut reader = Deserializer::from_str(&text);
    reader.disable_recursion_limit();
    let nodes = Vec::<AriaNode>::deserialize(&mut reader)?;
    reader.end()?;
    Ok(nodes)
}

fn normalize_oracle_d20(nodes: &mut [AriaNode]) {
    let mut stack: Vec<&mut AriaNode> = nodes.iter_mut().collect();
    while let Some(node) = stack.pop() {
        if NAME_PROHIBITED_ROLES.contains(&node.role.as_str()) {
            let taken = node.name.take();
            if node.text.is_none() && node.children.is_empty() {
                node.text = taken;
            }
        }
        for child in &mut node.children {
            if let AriaChild::Node(child_node) = child {
                stack.push(child_node);
            }
        }
    }
}

struct MappedOracleFrames {
    frame_to_doc: HashMap<String, DocId>,
    partial_frames: usize,
}

fn map_oracle_frames(
    tree: &SnapshotTree,
    oracle: &Oracle,
) -> Result<MappedOracleFrames, Box<dyn Error>> {
    let mut frame_to_doc: HashMap<String, DocId> = HashMap::new();
    let mut partial_frames = 0;

    let Some(main_frame) = oracle.frames.first() else {
        return Err(format!("{}: oracle has no main frame", oracle.fixture).into());
    };
    frame_to_doc.insert(main_frame.frame.clone(), DocId::MAIN);

    for frame in oracle.frames.iter().skip(1) {
        let Some(parent_frame) = frame.parent.as_deref() else {
            return Err(
                format!("{}: child frame {} lacks parent", oracle.fixture, frame.url).into(),
            );
        };
        let Some(owner) = frame.owner else {
            return Err(
                format!("{}: child frame {} lacks owner", oracle.fixture, frame.url).into(),
            );
        };
        let Some(&parent_doc_id) = frame_to_doc.get(parent_frame) else {
            partial_frames += 1;
            continue;
        };
        let parent_doc = tree
            .document(parent_doc_id)
            .ok_or("mapped parent doc id missing from SnapshotTree")?;
        let owner_id = DomNodeId::new(i32::try_from(owner)?);
        let key = NodeKey::new(parent_doc.token().clone(), owner_id);
        let Some(iframe_node) = tree.node_by_key(&key) else {
            return Err(format!(
                "{}: owner iframe {owner} for {} not found in parent doc",
                oracle.fixture, frame.url
            )
            .into());
        };
        if let Some(child_doc) = iframe_node.child_doc() {
            frame_to_doc.insert(frame.frame.clone(), child_doc);
        } else {
            partial_frames += 1;
        }
    }

    Ok(MappedOracleFrames {
        frame_to_doc,
        partial_frames,
    })
}

const fn to_oracle_tristate(value: TriState) -> Tristate {
    match value {
        TriState::False => Tristate::False,
        TriState::True => Tristate::True,
        TriState::Mixed => Tristate::Mixed,
    }
}

const fn to_oracle_invalid(value: InvalidState) -> Invalid {
    match value {
        InvalidState::False => Invalid::False,
        InvalidState::True => Invalid::True,
        InvalidState::Grammar => Invalid::Grammar,
        InvalidState::Spelling => Invalid::Spelling,
    }
}

fn states_equal(actual: State, expected: &OracleStates) -> bool {
    let checked_eq = actual.checked().map(to_oracle_tristate) == expected.checked;
    let pressed_eq = actual.pressed().map(to_oracle_tristate) == expected.pressed;
    let expanded_eq = actual.expanded() == expected.expanded;
    let selected_eq = actual.selected() == expected.selected;
    let disabled_eq = actual.disabled() == expected.disabled;
    let invalid_eq = actual.invalid().map(to_oracle_invalid) == expected.invalid;
    let level_eq = match (actual.level(), expected.level.as_ref()) {
        (None, None) => true,
        (Some(level), Some(number)) => number.as_u64() == Some(u64::from(level)),
        _ => false,
    };
    let active_eq = actual.active() == expected.active;
    checked_eq
        && pressed_eq
        && expanded_eq
        && selected_eq
        && disabled_eq
        && invalid_eq
        && level_eq
        && active_eq
}

fn collect_all_nodes(nodes: &[AriaNode]) -> Vec<&AriaNode> {
    let mut out = Vec::new();
    let mut stack: Vec<&AriaNode> = nodes.iter().rev().collect();
    while let Some(node) = stack.pop() {
        out.push(node);
        for child in node.children.iter().rev() {
            if let AriaChild::Node(child_node) = child {
                stack.push(child_node);
            }
        }
    }
    out
}

fn collect_printed_refs(nodes: &[AriaNode]) -> BTreeSet<String> {
    collect_all_nodes(nodes)
        .into_iter()
        .filter_map(|node| node.r#ref.clone())
        .collect()
}

type EmittedByKey<'a> = HashMap<NodeKey, (&'a AriaNode, RefTarget)>;

fn index_emitted_by_key<'a>(
    nodes: &'a [AriaNode],
    refs: &TabRefTable,
) -> Result<EmittedByKey<'a>, Box<dyn Error>> {
    let mut by_key = HashMap::new();
    for node in collect_all_nodes(nodes) {
        let Some(ref_str) = node.r#ref.as_deref() else {
            continue;
        };
        let RefLookup::Key(target) = refs.resolve_str(ref_str) else {
            return Err(format!("emitted ref {ref_str:?} failed to resolve to a live key").into());
        };
        by_key.insert(target.key.clone(), (node, target));
    }
    Ok(by_key)
}

fn index_oracle_ai_by_bid<'a>(
    nodes: &'a [AriaNode],
    frame_refs: &BTreeMap<String, u64>,
) -> HashMap<u64, &'a AriaNode> {
    let mut by_bid = HashMap::new();
    for node in collect_all_nodes(nodes) {
        if let Some(ref_str) = node.r#ref.as_deref()
            && let Some(&bid) = frame_refs.get(ref_str)
        {
            by_bid.insert(bid, node);
        }
    }
    by_bid
}

struct RenderedBundle {
    ai_playwright: String,
    ai_full: String,
    ai_page: String,
    default_playwright: String,
}

fn assert_no_canary(context: &str, label: &str, text: &str) {
    assert!(
        !text.contains(CANARY_PREFIX),
        "{context}: canary leaked into {label}"
    );
}

fn render_and_check_safety(
    context: &str,
    tree: &SnapshotTree,
    ai: &AriaTree,
    default: AriaTree,
    refs: &TabRefTable,
) -> Result<(RenderedBundle, Baseline), Box<dyn Error>> {
    assert!(
        json_depth(ai.nodes()) < MAX_JSON_DEPTH,
        "{context}: ai json_depth {} must stay below {MAX_JSON_DEPTH}",
        json_depth(ai.nodes())
    );
    assert!(
        json_depth(default.nodes()) < MAX_JSON_DEPTH,
        "{context}: default json_depth {} must stay below {MAX_JSON_DEPTH}",
        json_depth(default.nodes())
    );
    assert!(
        collect_printed_refs(default.nodes()).is_empty(),
        "{context}: default mode must emit zero refs"
    );

    let ai_json = serde_json::to_string(ai.nodes())?;
    let default_json = serde_json::to_string(default.nodes())?;
    assert_no_canary(context, "ai JSON", &ai_json);
    assert_no_canary(context, "default JSON", &default_json);
    assert_no_canary(context, "SnapshotTree Debug", &format!("{tree:?}"));
    assert_no_canary(context, "ai AriaTree Debug", &format!("{ai:?}"));
    assert_no_canary(context, "default AriaTree Debug", &format!("{default:?}"));
    assert_no_canary(context, "TabRefTable Debug", &format!("{refs:?}"));

    let passed = Passed::default();
    let ai_snap = ai.to_snapshot();
    let default_snap = default.into_snapshot();

    let ai_playwright = render(&ai_snap, Call::Playwright).into_text();
    let ai_full = render(&ai_snap, Call::Full { max_tokens: None }).into_text();
    let ai_page_answer = render(&ai_snap, Call::Page(&passed));
    let pending = ai_page_answer
        .pending()
        .ok_or_else(|| format!("{context}: Call::Page must return a pending baseline"))?;
    let baseline = Baseline {
        text: pending.text.to_owned(),
        form: pending.form,
    };
    let ai_page = ai_page_answer.into_text();

    let default_playwright = render(&default_snap, Call::Playwright).into_text();
    let default_full = render(&default_snap, Call::Full { max_tokens: None }).into_text();
    let default_page = render(&default_snap, Call::Page(&passed)).into_text();

    for (label, text) in [
        ("ai Call::Playwright", ai_playwright.as_str()),
        ("ai Call::Full", ai_full.as_str()),
        ("ai Call::Page", ai_page.as_str()),
        ("default Call::Playwright", default_playwright.as_str()),
        ("default Call::Full", default_full.as_str()),
        ("default Call::Page", default_page.as_str()),
    ] {
        assert_no_canary(context, label, text);
    }

    Ok((
        RenderedBundle {
            ai_playwright,
            ai_full,
            ai_page,
            default_playwright,
        },
        baseline,
    ))
}

fn assert_idempotent_delta(
    context: &str,
    tree: &SnapshotTree,
    first_ai: &AriaTree,
    baseline: &Baseline,
    refs: &mut TabRefTable,
) {
    let first_refs = collect_printed_refs(first_ai.nodes());
    let second_ai = tree.aria(AriaAsk::ai(refs));
    let second_refs = collect_printed_refs(second_ai.nodes());
    assert_eq!(
        second_refs, first_refs,
        "{context}: repeated ai emission with the same TabRefTable must preserve identical refs"
    );
    let passed = Passed::default();
    let second_snap = second_ai.into_snapshot();
    let delta = render(
        &second_snap,
        Call::Delta {
            passed: &passed,
            earlier: Earlier::Text(baseline),
        },
    )
    .into_text();
    assert_no_canary(context, "ai Call::Delta", &delta);
    assert_eq!(
        delta, NO_CHANGE_LINE,
        "{context}: repeated identical ai capture must render as unchanged delta"
    );
}

fn assert_all_refs_resolve_in_tree(
    context: &str,
    tree: &SnapshotTree,
    ai: &AriaTree,
    refs: &TabRefTable,
) -> Result<usize, Box<dyn Error>> {
    let mut resolved_count = 0;
    for node in collect_all_nodes(ai.nodes()) {
        let Some(ref_str) = node.r#ref.as_deref() else {
            continue;
        };
        let lookup = refs.resolve_str(ref_str);
        let RefLookup::Key(target) = lookup else {
            return Err(format!(
                "{context}: emitted ref {ref_str:?} resolved to {lookup:?} instead of RefLookup::Key"
            )
            .into());
        };
        assert!(
            tree.node_by_key(&target.key).is_some(),
            "{context}: emitted ref {ref_str:?} points to missing node {:?}",
            target.key
        );
        resolved_count += 1;
    }
    Ok(resolved_count)
}

fn assert_stock_roles_only(context: &str, mode_label: &str, nodes: &[AriaNode]) {
    for node in collect_all_nodes(nodes) {
        assert!(
            matches!(node.role.as_str(), "generic" | "iframe" | "text"),
            "{context}: stock Chromium {mode_label} tree emitted forbidden inferred role {:?}",
            node.role
        );
    }
}

fn is_ref_comparable_role(role: &str) -> bool {
    !matches!(role, "document" | "presentation" | "none")
}

fn assert_v4_emitted_node_parity(
    fixture: &str,
    frame_url: &str,
    element: &OracleElement,
    emitted: &AriaNode,
    oracle_node: &AriaNode,
) {
    let bid = element.backend_node_id;
    assert_eq!(
        emitted.role, oracle_node.role,
        "{fixture} [{frame_url}] node {bid}: role"
    );
    assert_eq!(
        emitted.name, oracle_node.name,
        "{fixture} [{frame_url}] node {bid}: name"
    );
    assert_eq!(
        emitted.checked, oracle_node.checked,
        "{fixture} [{frame_url}] node {bid}: checked"
    );
    assert_eq!(
        emitted.disabled, oracle_node.disabled,
        "{fixture} [{frame_url}] node {bid}: disabled"
    );
    assert_eq!(
        emitted.expanded, oracle_node.expanded,
        "{fixture} [{frame_url}] node {bid}: expanded"
    );
    assert_eq!(
        emitted.active, oracle_node.active,
        "{fixture} [{frame_url}] node {bid}: active"
    );
    assert_eq!(
        emitted.invalid, oracle_node.invalid,
        "{fixture} [{frame_url}] node {bid}: invalid"
    );
    assert_eq!(
        emitted.level, oracle_node.level,
        "{fixture} [{frame_url}] node {bid}: level"
    );
    assert_eq!(
        emitted.pressed, oracle_node.pressed,
        "{fixture} [{frame_url}] node {bid}: pressed"
    );
    assert_eq!(
        emitted.selected, oracle_node.selected,
        "{fixture} [{frame_url}] node {bid}: selected"
    );
}

fn compare_v4_recorded_frame(
    fixture: &str,
    tree: &SnapshotTree,
    cache: &NameCache<'_>,
    doc_id: DocId,
    frame: &OracleFrame,
    emitted_by_key: &HashMap<NodeKey, (&AriaNode, RefTarget)>,
) -> Result<usize, Box<dyn Error>> {
    let doc = tree
        .document(doc_id)
        .ok_or("mapped doc id missing from SnapshotTree")?;
    let mut oracle_ai = parse_oracle_aria(&frame.aria.ai)?;
    normalize_oracle_d20(&mut oracle_ai);
    let oracle_by_bid = index_oracle_ai_by_bid(&oracle_ai, &frame.refs);
    let oracle_ref_ids: HashSet<u64> = frame.refs.values().copied().collect();
    let mut joined = 0;

    for element in &frame.elements {
        let key = NodeKey::new(
            doc.token().clone(),
            DomNodeId::new(i32::try_from(element.backend_node_id)?),
        );
        let Some(node) = tree.node_by_key(&key) else {
            continue;
        };
        joined += 1;
        assert_eq!(
            node.role(),
            Role::parse(&element.role),
            "{fixture} [{}] node {}: role mismatch",
            frame.url,
            element.backend_node_id
        );
        assert_eq!(
            cache.name(node.id(), NameVisibility::VisibleOnly).text(),
            element.name,
            "{fixture} [{}] node {}: accessible name mismatch",
            frame.url,
            element.backend_node_id
        );
        assert!(
            states_equal(node.state(), &element.states),
            "{fixture} [{}] node {}: state mismatch tree={:?} oracle={:?}",
            frame.url,
            element.backend_node_id,
            node.state(),
            element.states
        );
        if is_ref_comparable_role(&element.role) {
            let expected_ref = oracle_ref_ids.contains(&element.backend_node_id)
                || tree.click_proxy(node.id()).is_some();
            assert_eq!(
                tree.is_ref_worthy(node.id()),
                expected_ref,
                "{fixture} [{}] node {}: ref-worthiness mismatch",
                frame.url,
                element.backend_node_id
            );
        }
        if let Some((emitted, _)) = emitted_by_key.get(&key)
            && let Some(oracle_node) = oracle_by_bid.get(&element.backend_node_id)
        {
            assert_v4_emitted_node_parity(fixture, &frame.url, element, emitted, oracle_node);
        }
    }
    Ok(joined)
}

fn check_stock_ref_worthiness(
    name: &str,
    tree: &SnapshotTree,
    oracle: &Oracle,
    frame_to_doc: &HashMap<String, DocId>,
) -> Result<usize, Box<dyn Error>> {
    let mut joined_elements = 0;
    let mut ref_mismatches = Vec::new();

    for frame in &oracle.frames {
        let Some(&doc_id) = frame_to_doc.get(&frame.frame) else {
            continue;
        };
        let doc = tree
            .document(doc_id)
            .ok_or("mapped doc id missing from SnapshotTree")?;
        let oracle_ref_ids: HashSet<u64> = frame.refs.values().copied().collect();

        for element in &frame.elements {
            let key = NodeKey::new(
                doc.token().clone(),
                DomNodeId::new(i32::try_from(element.backend_node_id)?),
            );
            let Some(node) = tree.node_by_key(&key) else {
                continue;
            };
            joined_elements += 1;
            if !is_ref_comparable_role(&element.role) {
                continue;
            }
            let actual = tree.is_ref_worthy(node.id());
            let expected = oracle_ref_ids.contains(&element.backend_node_id);
            if actual != expected {
                ref_mismatches.push((
                    element.backend_node_id,
                    element.role.clone(),
                    element.visible,
                    actual,
                    expected,
                ));
            }
        }
    }

    assert_expected_stock_ref_diffs(name, &ref_mismatches);
    Ok(joined_elements)
}

fn assert_expected_stock_ref_diffs(name: &str, diffs: &[(u64, String, bool, bool, bool)]) {
    match name {
        "zero-size-controls" => {
            // Blink repairs the two zero-size <input type="file"> outer boxes from their UA shadow
            // buttons (design §5.2).
            assert_eq!(
                diffs.len(),
                2,
                "{name}: expected only the two Blink-repaired file inputs to differ, got {diffs:?}"
            );
            assert!(
                diffs
                    .iter()
                    .all(|(_, role, visible, actual, expected)| role == "button"
                        && !*visible
                        && *actual
                        && !*expected),
                "{name}: unexpected ref-worthiness diff shape: {diffs:?}"
            );
        }
        "nosize" => {
            // Stock Chromium omits browseros.pointer_events_none, so the 11 visible
            // `pointer-events: none` elements on nosize stay ref-worthy on stock .pb.
            assert_eq!(
                diffs.len(),
                11,
                "{name}: expected only the 11 pointer-events:none elements to differ, got {diffs:?}"
            );
            assert!(
                diffs
                    .iter()
                    .all(|(_, _, visible, actual, expected)| *visible && *actual && !*expected),
                "{name}: unexpected nosize ref-worthiness diff shape: {diffs:?}"
            );
        }
        _ => {
            assert!(
                diffs.is_empty(),
                "{name}: unexpected ref-worthiness mismatches on stock .pb: {diffs:?}"
            );
        }
    }
}

struct RecordedFixtureOutcome {
    joined_elements: usize,
    resolved_refs: usize,
}

fn verify_v4_recorded_frames(
    name: &str,
    tree: &SnapshotTree,
    oracle: &Oracle,
    frame_to_doc: &HashMap<String, DocId>,
    ai: &AriaTree,
    refs: &TabRefTable,
) -> Result<usize, Box<dyn Error>> {
    let cache = tree.name_cache();
    let emitted_by_key = index_emitted_by_key(ai.nodes(), refs)?;
    let mut joined = 0;
    for frame in &oracle.frames {
        if let Some(&doc_id) = frame_to_doc.get(&frame.frame) {
            joined +=
                compare_v4_recorded_frame(name, tree, &cache, doc_id, frame, &emitted_by_key)?;
        }
    }
    Ok(joined)
}

fn load_and_build_recorded_fixture(
    name: &str,
) -> Result<(SnapshotTree, Oracle, Option<u32>), Box<dyn Error>> {
    let mut apc = load_pb(name)?;
    let oracle = load_oracle(name)?;
    let recorded_version = apc
        .browseros
        .as_ref()
        .and_then(|meta| meta.contract_version);

    if recorded_version.is_none() {
        let refused = SnapshotTree::build(apc.clone(), BuildOptions::default()).err();
        assert_eq!(
            refused,
            Some(BuildError::Unpatched {
                found: None,
                required: MIN_CONTRACT_VERSION,
            }),
            "{name}: unpatched stock .pb must be refused by SnapshotTree::build"
        );
        apc.browseros = Some(BrowserOsPageContentData {
            contract_version: Some(MIN_CONTRACT_VERSION),
            subframe_timeout_ms: Some(2_000),
        });
    }

    let tree = SnapshotTree::build(apc, BuildOptions::default())?;
    Ok((tree, oracle, recorded_version))
}

fn verify_recorded_fixture(name: &str) -> Result<RecordedFixtureOutcome, Box<dyn Error>> {
    let (tree, oracle, recorded_version) = load_and_build_recorded_fixture(name)?;
    let MappedOracleFrames {
        frame_to_doc,
        partial_frames,
    } = map_oracle_frames(&tree, &oracle)?;

    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs));
    let default = tree.aria(AriaAsk::default_mode());

    assert_eq!(
        ai.partial().len(),
        partial_frames,
        "{name}: ai partial count"
    );
    assert_eq!(
        default.partial().len(),
        partial_frames,
        "{name}: default partial count"
    );

    if recorded_version.is_none() {
        assert_stock_roles_only(name, "ai", ai.nodes());
        assert_stock_roles_only(name, "default", default.nodes());
    }

    let resolved_refs = assert_all_refs_resolve_in_tree(name, &tree, &ai, &refs)?;
    let (rendered, baseline) = render_and_check_safety(name, &tree, &ai, default, &refs)?;
    assert_idempotent_delta(name, &tree, &ai, &baseline, &mut refs);

    if name == "hung-subframe" {
        assert_eq!(partial_frames, 1, "hung-subframe must have 1 partial frame");
        assert!(
            rendered.ai_playwright.contains("# partial:")
                && rendered.ai_full.contains("# partial:")
                && rendered.ai_page.contains("# partial:")
                && rendered.default_playwright.contains("# partial:"),
            "hung-subframe partial frame must render `# partial:` across output formats"
        );
    }

    let joined_elements = if recorded_version >= Some(CONTRACT_V4) {
        verify_v4_recorded_frames(name, &tree, &oracle, &frame_to_doc, &ai, &refs)?
    } else {
        check_stock_ref_worthiness(name, &tree, &oracle, &frame_to_doc)?
    };

    assert!(
        !rendered.ai_playwright.is_empty() && !rendered.default_playwright.is_empty(),
        "{name}: rendered Playwright text must be non-empty"
    );

    Ok(RecordedFixtureOutcome {
        joined_elements,
        resolved_refs,
    })
}

#[test]
fn all_16_recorded_fixtures_emit_render_and_match_oracle_expressible_facts()
-> Result<(), Box<dyn Error>> {
    let mut total_joined = 0;
    let mut total_resolved_refs = 0;

    for fixture in FIXTURES {
        let outcome = verify_recorded_fixture(fixture.name)?;
        assert!(
            outcome.joined_elements > 0,
            "{}: expected joined elements between .pb and oracle",
            fixture.name
        );
        total_joined += outcome.joined_elements;
        total_resolved_refs += outcome.resolved_refs;
    }

    assert!(
        total_joined >= 640,
        "expected >= 640 joined oracle elements across all 16 fixtures, saw {total_joined}"
    );
    assert!(
        total_resolved_refs > 0,
        "expected emitted ai refs across recorded fixtures"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared builders & assertions for contract-v4 edge-page mirrors
// ---------------------------------------------------------------------------

fn visible_box(builder: NodeBuilder) -> NodeBuilder {
    builder.bounding_box(10, 10, 200, 24)
}

fn zero_box(builder: NodeBuilder) -> NodeBuilder {
    builder.bounding_box(0, 0, 0, 0)
}

fn label_for(dom_id: i32, target_dom_id: i32, text: &str) -> NodeBuilder {
    visible_box(
        NodeBuilder::element("label")
            .dom_id(dom_id)
            .label_for(target_dom_id)
            .child(NodeBuilder::text(text)),
    )
}

fn select_opt(text: &str, selected: bool) -> SelectOption {
    SelectOption {
        value: Some(text.to_owned()),
        text: Some(text.to_owned()),
        is_selected: Some(selected),
        is_disabled: Some(false),
    }
}

fn ref_node(dom_id: i32) -> BrowserOsRef {
    BrowserOsRef {
        dom_node_id: Some(dom_id),
        unrendered_text: None,
    }
}

fn assert_tree_element_matches_oracle(
    context: &str,
    tree: &SnapshotTree,
    cache: &NameCache<'_>,
    doc_token: &DocToken,
    element: &OracleElement,
) -> Result<(), Box<dyn Error>> {
    let key = NodeKey::new(
        doc_token.clone(),
        DomNodeId::new(i32::try_from(element.backend_node_id)?),
    );
    let node = tree.node_by_key(&key).ok_or_else(|| {
        format!(
            "{context}: missing oracle backendNodeId {} (role={:?}, name={:?})",
            element.backend_node_id, element.role, element.name
        )
    })?;
    let expected_role = Role::parse(&element.role)
        .ok_or_else(|| format!("{context}: oracle role {:?} failed to parse", element.role))?;
    assert_eq!(
        node.role(),
        Some(expected_role),
        "{context}: backendNodeId {} role mismatch",
        element.backend_node_id
    );
    assert_eq!(
        node.hidden().is_hidden_for_aria(),
        element.hidden,
        "{context}: backendNodeId {} hidden mismatch",
        element.backend_node_id
    );
    assert_eq!(
        cache.name(node.id(), NameVisibility::VisibleOnly).text(),
        element.name,
        "{context}: backendNodeId {} name mismatch",
        element.backend_node_id
    );
    assert!(
        states_equal(node.state(), &element.states),
        "{context}: backendNodeId {} state mismatch tree={:?} oracle={:?}",
        element.backend_node_id,
        node.state(),
        element.states
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 2: ua-shadow contract-v4 mirror vs tests/fixtures/ua-shadow.oracle.json
// ---------------------------------------------------------------------------

fn ua_shadow_text_and_file_rows() -> Vec<NodeBuilder> {
    vec![
        visible_box(NodeBuilder::element("div").dom_id(16))
            .child(label_for(116, 6, "textbox: text input with value"))
            .child(
                visible_box(
                    NodeBuilder::form_control(FormControlType::InputText)
                        .tag("input")
                        .dom_id(6)
                        .field_value("value: public text"),
                )
                .child(
                    visible_box(NodeBuilder::container().dom_id(901).ua_shadow(true))
                        .child(NodeBuilder::text("value: public text").ua_shadow(true)),
                ),
            ),
        visible_box(NodeBuilder::element("div").dom_id(17))
            .child(label_for(117, 7, "textbox: date input with value"))
            .child(
                visible_box(
                    NodeBuilder::form_control(FormControlType::InputDate)
                        .tag("input")
                        .dom_id(7)
                        .field_value("2020-02-03"),
                )
                .child(
                    visible_box(
                        NodeBuilder::element("span")
                            .dom_id(902)
                            .role("spinbutton")
                            .ua_shadow(true),
                    )
                    .child(NodeBuilder::text("02/03/2020").ua_shadow(true)),
                )
                .child(
                    visible_box(
                        NodeBuilder::form_control(FormControlType::ButtonButton)
                            .tag("button")
                            .dom_id(903)
                            .ua_shadow(true),
                    )
                    .child(NodeBuilder::text("Show date picker").ua_shadow(true)),
                ),
            ),
        visible_box(NodeBuilder::element("div").dom_id(18))
            .child(label_for(118, 11, "button: file input"))
            .child(
                visible_box(
                    NodeBuilder::form_control(FormControlType::InputFile)
                        .tag("input")
                        .dom_id(11),
                )
                .child(visible_box(
                    NodeBuilder::form_control(FormControlType::InputButton)
                        .tag("input")
                        .dom_id(904)
                        .field_value("Choose File")
                        .ua_shadow(true),
                )),
            ),
    ]
}

fn ua_shadow_select_and_textarea_rows() -> Vec<NodeBuilder> {
    vec![
        visible_box(NodeBuilder::element("div").dom_id(19))
            .child(label_for(119, 8, "combobox: collapsed select"))
            .child(
                visible_box(
                    NodeBuilder::form_control(FormControlType::SelectOne)
                        .tag("select")
                        .dom_id(8)
                        .select_option(select_opt("option: first", false))
                        .select_option(select_opt("option: second, selected", true))
                        .select_option(select_opt("option: third", false)),
                )
                .child(
                    visible_box(NodeBuilder::container().dom_id(905).ua_shadow(true))
                        .child(NodeBuilder::text("option: second, selected").ua_shadow(true)),
                ),
            ),
        visible_box(NodeBuilder::element("div").dom_id(20))
            .child(label_for(120, 9, "textbox: textarea with value"))
            .child(
                visible_box(
                    NodeBuilder::form_control(FormControlType::TextArea)
                        .tag("textarea")
                        .dom_id(9)
                        .field_value("value: public textarea text"),
                )
                .child(
                    visible_box(NodeBuilder::container().dom_id(906).ua_shadow(true))
                        .child(NodeBuilder::text("value: public textarea text").ua_shadow(true)),
                ),
            ),
        visible_box(NodeBuilder::element("div").dom_id(21)).child(
            visible_box(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(10)
                    .placeholder("textbox: input with only a placeholder"),
            )
            .child(
                visible_box(NodeBuilder::container().dom_id(907).ua_shadow(true)).child(
                    NodeBuilder::text("textbox: input with only a placeholder").ua_shadow(true),
                ),
            ),
        ),
    ]
}

fn ua_shadow_media_and_details() -> Vec<NodeBuilder> {
    vec![
        visible_box(NodeBuilder::element("figure").dom_id(22))
            .child(
                NodeBuilder::element("video")
                    .dom_id(73)
                    .bounding_box(10, 200, 320, 180)
                    .child(
                        visible_box(NodeBuilder::container().dom_id(908).ua_shadow(true))
                            .child(visible_box(
                                NodeBuilder::form_control(FormControlType::ButtonButton)
                                    .tag("button")
                                    .dom_id(909)
                                    .aria_label("Play")
                                    .ua_shadow(true),
                            ))
                            .child(visible_box(
                                NodeBuilder::element("input")
                                    .dom_id(910)
                                    .role("slider")
                                    .ua_shadow(true),
                            )),
                    ),
            )
            .child(
                visible_box(NodeBuilder::element("figcaption").dom_id(23))
                    .child(NodeBuilder::text("figure: video with controls, no src")),
            ),
        visible_box(NodeBuilder::element("details").dom_id(24))
            .child(
                visible_box(NodeBuilder::container().dom_id(911).ua_shadow(true)).child(
                    visible_box(NodeBuilder::element("summary").dom_id(25))
                        .child(NodeBuilder::text("summary: closed details")),
                ),
            )
            .child(
                NodeBuilder::container()
                    .dom_id(912)
                    .ua_shadow(true)
                    .annotated_role(AnnotatedRole::ContentHidden)
                    .child(
                        NodeBuilder::element("p")
                            .dom_id(153)
                            .child(NodeBuilder::text("paragraph: inside closed details")),
                    )
                    .child(
                        NodeBuilder::form_control(FormControlType::ButtonButton)
                            .tag("button")
                            .dom_id(156)
                            .child(NodeBuilder::text("button: inside closed details")),
                    ),
            ),
        visible_box(NodeBuilder::element("details").dom_id(26))
            .child(
                visible_box(NodeBuilder::container().dom_id(913).ua_shadow(true)).child(
                    visible_box(NodeBuilder::element("summary").dom_id(27))
                        .child(NodeBuilder::text("summary: open details")),
                ),
            )
            .child(
                visible_box(NodeBuilder::container().dom_id(914).ua_shadow(true))
                    .child(
                        visible_box(NodeBuilder::element("p").dom_id(29))
                            .child(NodeBuilder::text("paragraph: inside open details")),
                    )
                    .child(
                        visible_box(
                            NodeBuilder::form_control(FormControlType::ButtonButton)
                                .tag("button")
                                .dom_id(71),
                        )
                        .child(NodeBuilder::text("button: inside open details")),
                    ),
            ),
    ]
}

fn build_contract_v4_ua_shadow() -> AnnotatedPageContent {
    let mut body_children = vec![
        visible_box(NodeBuilder::element("h1").dom_id(15))
            .child(NodeBuilder::text("heading: browser-made shadow content")),
    ];
    body_children.extend(ua_shadow_text_and_file_rows());
    body_children.extend(ua_shadow_select_and_textarea_rows());
    body_children.extend(ua_shadow_media_and_details());

    ApcBuilder::new("doc-ua-shadow-v4")
        .contract_version(CONTRACT_V4)
        .main_url("http://127.0.0.1/edge/ua-shadow.html")
        .main_title("Edge: browser-made shadow content")
        .main_focused_node_id(14)
        .root(
            NodeBuilder::root()
                .tag("html")
                .dom_id(13)
                .bounding_box(0, 0, 800, 600)
                .child(
                    NodeBuilder::element("body")
                        .dom_id(14)
                        .bounding_box(0, 0, 800, 600)
                        .children(body_children),
                ),
        )
        .build()
}

fn assert_ua_shadow_ai_nodes(
    doc_token: &DocToken,
    emitted_by_key: &HashMap<NodeKey, (&AriaNode, RefTarget)>,
    oracle_by_bid: &HashMap<u64, &AriaNode>,
    ai_nodes: &[AriaNode],
) -> Result<(), Box<dyn Error>> {
    for bid in [15_u64, 6, 7, 11, 8, 9, 10, 22, 24, 25, 26, 27, 29, 71] {
        let key = NodeKey::new(doc_token.clone(), DomNodeId::new(i32::try_from(bid)?));
        let (emitted, target) = emitted_by_key
            .get(&key)
            .ok_or_else(|| format!("ua-shadow ai tree missing node {bid}"))?;
        let oracle_node = oracle_by_bid
            .get(&bid)
            .ok_or_else(|| format!("ua-shadow oracle.aria.ai missing node {bid}"))?;
        assert!(
            target.click_proxy.is_none(),
            "ua-shadow node {bid} is normal size and must not carry a click_proxy"
        );
        assert_eq!(emitted.role, oracle_node.role, "ua-shadow node {bid} role");
        assert_eq!(emitted.name, oracle_node.name, "ua-shadow node {bid} name");
        assert_eq!(emitted.text, oracle_node.text, "ua-shadow node {bid} text");
        assert_eq!(
            emitted.level, oracle_node.level,
            "ua-shadow node {bid} level"
        );
        assert_eq!(
            emitted.disabled, oracle_node.disabled,
            "ua-shadow node {bid} disabled"
        );
        if bid == 8 {
            assert_eq!(
                emitted.children, oracle_node.children,
                "ua-shadow collapsed <select> must synthesize identical ref-less option children"
            );
        }
    }

    for node in collect_all_nodes(ai_nodes) {
        assert!(
            !matches!(node.role.as_str(), "spinbutton" | "slider"),
            "ua-shadow leaked internal UA shadow role {:?}",
            node.role
        );
        assert_ne!(
            node.name.as_deref(),
            Some("Play"),
            "ua-shadow leaked <video> UA shadow Play button"
        );
        assert_ne!(
            node.text.as_deref(),
            Some("02/03/2020"),
            "ua-shadow leaked <input type=date> UA shadow text"
        );
    }
    Ok(())
}

#[test]
fn contract_v4_ua_shadow_matches_recorded_oracle() -> Result<(), Box<dyn Error>> {
    let oracle = load_oracle("ua-shadow")?;
    let frame = oracle
        .frames
        .first()
        .ok_or("ua-shadow.oracle.json missing main frame")?;
    let oracle_ref_ids: HashSet<u64> = frame.refs.values().copied().collect();
    let mut oracle_ai = parse_oracle_aria(&frame.aria.ai)?;
    normalize_oracle_d20(&mut oracle_ai);
    let oracle_default = parse_oracle_aria(&frame.aria.default)?;

    let tree = SnapshotTree::build(build_contract_v4_ua_shadow(), BuildOptions::default())?;
    let cache = tree.name_cache();
    let doc_token = tree
        .document(DocId::MAIN)
        .ok_or("missing DocId::MAIN")?
        .token()
        .clone();

    let collapsed_option_ids: HashSet<u64> = HashSet::from([132, 135, 138]);
    for element in &frame.elements {
        if collapsed_option_ids.contains(&element.backend_node_id) {
            assert!(!oracle_ref_ids.contains(&element.backend_node_id));
            continue;
        }
        assert_tree_element_matches_oracle("ua-shadow", &tree, &cache, &doc_token, element)?;
        if is_ref_comparable_role(&element.role) {
            let key = NodeKey::new(
                doc_token.clone(),
                DomNodeId::new(i32::try_from(element.backend_node_id)?),
            );
            let node = tree.node_by_key(&key).ok_or("verified node must exist")?;
            assert_eq!(
                tree.is_ref_worthy(node.id()),
                oracle_ref_ids.contains(&element.backend_node_id),
                "ua-shadow node {}: ref-worthiness must match oracle.refs",
                element.backend_node_id
            );
        }
    }

    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs));
    let default = tree.aria(AriaAsk::default_mode());

    assert_eq!(
        default.nodes(),
        oracle_default.as_slice(),
        "ua-shadow default mode must match ua-shadow.oracle.json aria.default exactly"
    );

    let emitted_by_key = index_emitted_by_key(ai.nodes(), &refs)?;
    let oracle_by_bid = index_oracle_ai_by_bid(&oracle_ai, &frame.refs);
    assert_ua_shadow_ai_nodes(&doc_token, &emitted_by_key, &oracle_by_bid, ai.nodes())?;

    for ua_id in 901..=914 {
        let key = NodeKey::new(doc_token.clone(), DomNodeId::new(ua_id));
        let node = tree
            .node_by_key(&key)
            .ok_or_else(|| format!("missing synthetic ua_shadow node {ua_id}"))?;
        assert!(node.hidden().is_ua_shadow());
        assert!(!tree.is_ref_worthy(node.id()));
        assert!(!emitted_by_key.contains_key(&key));
    }

    let (rendered, baseline) = render_and_check_safety("ua-shadow", &tree, &ai, default, &refs)?;
    assert_idempotent_delta("ua-shadow", &tree, &ai, &baseline, &mut refs);
    assert!(
        rendered
            .ai_playwright
            .contains("combobox: collapsed select")
    );
    assert!(rendered.ai_page.contains("button: inside open details"));
    assert!(!rendered.ai_playwright.contains("inside closed details"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3: zero-size-controls contract-v4 mirror vs zero-size-controls.oracle.json
// ---------------------------------------------------------------------------

fn zero_size_fieldset_rows() -> Vec<NodeBuilder> {
    vec![
        visible_box(NodeBuilder::element("legend").dom_id(9))
            .child(NodeBuilder::text("group: zero-size controls")),
        visible_box(NodeBuilder::element("div").dom_id(10))
            .child(zero_box(
                NodeBuilder::form_control(FormControlType::InputCheckbox)
                    .tag("input")
                    .dom_id(39)
                    .checked(true),
            ))
            .child(label_for(
                101,
                39,
                "checkbox: zero-size, label by for, checked",
            )),
        visible_box(NodeBuilder::element("div").dom_id(11)).child(
            visible_box(NodeBuilder::element("label").dom_id(63).label_for(40))
                .child(zero_box(
                    NodeBuilder::form_control(FormControlType::InputCheckbox)
                        .tag("input")
                        .dom_id(40)
                        .checked(false),
                ))
                .child(NodeBuilder::text(
                    "checkbox: zero-size, nested in label, unchecked",
                )),
        ),
        visible_box(NodeBuilder::element("div").dom_id(12))
            .child(zero_box(
                NodeBuilder::form_control(FormControlType::InputRadio)
                    .tag("input")
                    .dom_id(41)
                    .checked(false),
            ))
            .child(label_for(
                102,
                41,
                "radio: zero-size, label by for, unchecked",
            )),
        visible_box(NodeBuilder::element("div").dom_id(13)).child(
            visible_box(NodeBuilder::element("label").dom_id(67).label_for(42))
                .child(zero_box(
                    NodeBuilder::form_control(FormControlType::InputRadio)
                        .tag("input")
                        .dom_id(42)
                        .checked(true),
                ))
                .child(NodeBuilder::text(
                    "radio: zero-size, nested in label, checked",
                )),
        ),
        visible_box(NodeBuilder::element("div").dom_id(14))
            .child(zero_box(
                NodeBuilder::form_control(FormControlType::InputFile)
                    .tag("input")
                    .dom_id(43),
            ))
            .child(label_for(
                103,
                43,
                "button: zero-size file input, label by for",
            )),
        visible_box(NodeBuilder::element("div").dom_id(15)).child(
            visible_box(NodeBuilder::element("label").dom_id(71).label_for(44))
                .child(zero_box(
                    NodeBuilder::form_control(FormControlType::InputFile)
                        .tag("input")
                        .dom_id(44),
                ))
                .child(NodeBuilder::text(
                    "button: zero-size file input, nested in label",
                )),
        ),
    ]
}

fn normal_size_fieldset_rows() -> Vec<NodeBuilder> {
    vec![
        visible_box(NodeBuilder::element("legend").dom_id(17))
            .child(NodeBuilder::text("group: normal-size controls")),
        visible_box(NodeBuilder::element("div").dom_id(18))
            .child(visible_box(
                NodeBuilder::form_control(FormControlType::InputCheckbox)
                    .tag("input")
                    .dom_id(30)
                    .checked(false),
            ))
            .child(label_for(
                104,
                30,
                "checkbox: normal size, label by for, unchecked",
            )),
        visible_box(NodeBuilder::element("div").dom_id(19)).child(
            visible_box(NodeBuilder::element("label").dom_id(78).label_for(32))
                .child(visible_box(
                    NodeBuilder::form_control(FormControlType::InputRadio)
                        .tag("input")
                        .dom_id(32)
                        .checked(true),
                ))
                .child(NodeBuilder::text(
                    "radio: normal size, nested in label, checked",
                )),
        ),
        visible_box(NodeBuilder::element("div").dom_id(20))
            .child(visible_box(
                NodeBuilder::form_control(FormControlType::InputFile)
                    .tag("input")
                    .dom_id(34),
            ))
            .child(label_for(
                105,
                34,
                "button: normal-size file input, label by for",
            )),
    ]
}

fn build_contract_v4_zero_size_controls() -> AnnotatedPageContent {
    ApcBuilder::new("doc-zero-size-v4")
        .contract_version(CONTRACT_V4)
        .main_url("http://127.0.0.1/edge/zero-size-controls.html")
        .main_title("Edge: zero-size controls")
        .root(
            NodeBuilder::root()
                .tag("html")
                .dom_id(5)
                .bounding_box(0, 0, 800, 600)
                .child(
                    NodeBuilder::element("body")
                        .dom_id(6)
                        .bounding_box(0, 0, 800, 600)
                        .child(
                            visible_box(NodeBuilder::element("h1").dom_id(7))
                                .child(NodeBuilder::text("heading: zero-size controls")),
                        )
                        .child(
                            visible_box(NodeBuilder::element("fieldset").dom_id(8))
                                .children(zero_size_fieldset_rows()),
                        )
                        .child(
                            visible_box(NodeBuilder::element("fieldset").dom_id(16))
                                .children(normal_size_fieldset_rows()),
                        ),
                ),
        )
        .build()
}

fn assert_zero_size_click_proxies(
    tree: &SnapshotTree,
    doc_token: &DocToken,
    oracle_ref_ids: &HashSet<u64>,
    emitted_by_key: &HashMap<NodeKey, (&AriaNode, RefTarget)>,
) -> Result<(), Box<dyn Error>> {
    for control_id in [30_i32, 32, 34] {
        let bid = u64::try_from(control_id)?;
        assert!(oracle_ref_ids.contains(&bid));
        let key = NodeKey::new(doc_token.clone(), DomNodeId::new(control_id));
        let (_, target) = emitted_by_key
            .get(&key)
            .ok_or_else(|| format!("missing emitted ref for normal-size control {control_id}"))?;
        assert_eq!(
            target,
            &RefTarget {
                key,
                click_proxy: None,
            }
        );
    }

    for (control_id, label_id) in [
        (39_i32, 101_i32),
        (40, 63),
        (41, 102),
        (42, 67),
        (43, 103),
        (44, 71),
    ] {
        let bid = u64::try_from(control_id)?;
        assert!(!oracle_ref_ids.contains(&bid));
        let control_key = NodeKey::new(doc_token.clone(), DomNodeId::new(control_id));
        let label_key = NodeKey::new(doc_token.clone(), DomNodeId::new(label_id));
        let label_node = tree
            .node_by_key(&label_key)
            .ok_or_else(|| format!("missing label node {label_id}"))?;
        assert!(!tree.is_ref_worthy(label_node.id()));
        assert!(!emitted_by_key.contains_key(&label_key));
        let (_, target) = emitted_by_key
            .get(&control_key)
            .ok_or_else(|| format!("zero-size control {control_id} must gain a ClickProxy ref"))?;
        assert_eq!(
            target,
            &RefTarget {
                key: control_key,
                click_proxy: Some(label_key),
            }
        );
    }

    let zero_group_key = NodeKey::new(doc_token.clone(), DomNodeId::new(8));
    let (zero_group, _) = emitted_by_key
        .get(&zero_group_key)
        .ok_or("missing zero-size group node 8")?;
    assert_eq!(
        zero_group.children.len(),
        6,
        "zero-size group must hold only the 6 controls with all paired labels folded"
    );
    Ok(())
}

#[test]
fn contract_v4_zero_size_controls_verifies_click_proxy_against_recorded_oracle()
-> Result<(), Box<dyn Error>> {
    let oracle = load_oracle("zero-size-controls")?;
    let frame = oracle
        .frames
        .first()
        .ok_or("zero-size-controls.oracle.json missing main frame")?;
    let oracle_ref_ids: HashSet<u64> = frame.refs.values().copied().collect();
    let oracle_default = parse_oracle_aria(&frame.aria.default)?;

    let tree = SnapshotTree::build(
        build_contract_v4_zero_size_controls(),
        BuildOptions::default(),
    )?;
    let cache = tree.name_cache();
    let doc_token = tree
        .document(DocId::MAIN)
        .ok_or("missing DocId::MAIN")?
        .token()
        .clone();

    for element in &frame.elements {
        assert_tree_element_matches_oracle(
            "zero-size-controls",
            &tree,
            &cache,
            &doc_token,
            element,
        )?;
    }

    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs));
    let default = tree.aria(AriaAsk::default_mode());

    assert_eq!(
        default.nodes(),
        oracle_default.as_slice(),
        "zero-size-controls default mode must match oracle aria.default exactly with zero refs"
    );

    let emitted_by_key = index_emitted_by_key(ai.nodes(), &refs)?;
    assert_zero_size_click_proxies(&tree, &doc_token, &oracle_ref_ids, &emitted_by_key)?;

    let (rendered, baseline) =
        render_and_check_safety("zero-size-controls", &tree, &ai, default, &refs)?;
    assert_idempotent_delta("zero-size-controls", &tree, &ai, &baseline, &mut refs);
    assert!(
        rendered
            .ai_page
            .contains("checkbox: zero-size, label by for, checked")
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 4: private-forms contract-v4 mirror vs private-forms.oracle.json
// ---------------------------------------------------------------------------

struct RedactedFieldSpec {
    row_id: i32,
    label_id: i32,
    control_id: i32,
    label_text: &'static str,
    control_type: FormControlType,
    tag: &'static str,
    decision: RedactionDecision,
    canary_suffix: &'static str,
}

const PRIVATE_INPUT_SPECS: &[RedactedFieldSpec] = &[
    RedactedFieldSpec {
        row_id: 31,
        label_id: 54,
        control_id: 7,
        label_text: "textbox: input autocomplete=current-password",
        control_type: FormControlType::InputText,
        tag: "input",
        decision: RedactionDecision::RedactedHasBeenPassword,
        canary_suffix: "in-cur",
    },
    RedactedFieldSpec {
        row_id: 32,
        label_id: 57,
        control_id: 8,
        label_text: "textbox: input autocomplete=new-password",
        control_type: FormControlType::InputText,
        tag: "input",
        decision: RedactionDecision::RedactedHasBeenPassword,
        canary_suffix: "in-new",
    },
    RedactedFieldSpec {
        row_id: 33,
        label_id: 60,
        control_id: 9,
        label_text: "textbox: input autocomplete=one-time-code",
        control_type: FormControlType::InputText,
        tag: "input",
        decision: RedactionDecision::RedactedIsOtp,
        canary_suffix: "in-otp",
    },
    RedactedFieldSpec {
        row_id: 34,
        label_id: 63,
        control_id: 10,
        label_text: "textbox: input autocomplete=cc-number",
        control_type: FormControlType::InputText,
        tag: "input",
        decision: RedactionDecision::RedactedIsSensitivePaymentField,
        canary_suffix: "in-cc",
    },
    RedactedFieldSpec {
        row_id: 35,
        label_id: 66,
        control_id: 11,
        label_text: "textbox: input autocomplete=cc-csc",
        control_type: FormControlType::InputText,
        tag: "input",
        decision: RedactionDecision::RedactedIsSensitivePaymentField,
        canary_suffix: "in-csc",
    },
];

const PRIVATE_TEXTAREA_SPECS: &[RedactedFieldSpec] = &[
    RedactedFieldSpec {
        row_id: 37,
        label_id: 70,
        control_id: 12,
        label_text: "textbox: textarea autocomplete=current-password",
        control_type: FormControlType::TextArea,
        tag: "textarea",
        decision: RedactionDecision::RedactedHasBeenPassword,
        canary_suffix: "ta-cur",
    },
    RedactedFieldSpec {
        row_id: 38,
        label_id: 73,
        control_id: 13,
        label_text: "textbox: textarea autocomplete=new-password",
        control_type: FormControlType::TextArea,
        tag: "textarea",
        decision: RedactionDecision::RedactedHasBeenPassword,
        canary_suffix: "ta-new",
    },
    RedactedFieldSpec {
        row_id: 39,
        label_id: 76,
        control_id: 14,
        label_text: "textbox: textarea autocomplete=one-time-code",
        control_type: FormControlType::TextArea,
        tag: "textarea",
        decision: RedactionDecision::RedactedIsOtp,
        canary_suffix: "ta-otp",
    },
    RedactedFieldSpec {
        row_id: 40,
        label_id: 79,
        control_id: 15,
        label_text: "textbox: textarea autocomplete=cc-number",
        control_type: FormControlType::TextArea,
        tag: "textarea",
        decision: RedactionDecision::RedactedIsSensitivePaymentField,
        canary_suffix: "ta-cc",
    },
    RedactedFieldSpec {
        row_id: 41,
        label_id: 82,
        control_id: 16,
        label_text: "textbox: textarea autocomplete=cc-csc",
        control_type: FormControlType::TextArea,
        tag: "textarea",
        decision: RedactionDecision::RedactedIsSensitivePaymentField,
        canary_suffix: "ta-csc",
    },
];

fn build_redacted_row(spec: &RedactedFieldSpec) -> Result<NodeBuilder, Box<dyn Error>> {
    let secret = format!("{CANARY_PREFIX}-{}", spec.canary_suffix);
    let len = u32::try_from(secret.encode_utf16().count())?;
    let mut control = visible_box(
        NodeBuilder::form_control(spec.control_type)
            .tag(spec.tag)
            .dom_id(spec.control_id)
            .redaction(spec.decision)
            .redacted_value_length(len)
            .field_value(&secret),
    );
    if spec.tag == "textarea" {
        control = control.child(NodeBuilder::text(&secret).dom_text(&secret));
    }
    Ok(visible_box(NodeBuilder::element("div").dom_id(spec.row_id))
        .child(label_for(spec.label_id, spec.control_id, spec.label_text))
        .child(control))
}

fn private_password_fieldset() -> Result<NodeBuilder, Box<dyn Error>> {
    let pw_canary_0 = format!("{CANARY_PREFIX}-private-forms-0");
    let pw_canary_1 = format!("{CANARY_PREFIX}-private-forms-1");
    let pw_len = u32::try_from(pw_canary_0.encode_utf16().count())?;

    Ok(visible_box(NodeBuilder::element("fieldset").dom_id(18))
        .child(
            visible_box(NodeBuilder::element("legend").dom_id(27))
                .child(NodeBuilder::text("group: password fields")),
        )
        .child(
            visible_box(NodeBuilder::element("div").dom_id(28))
                .child(label_for(45, 5, "textbox: input type=password"))
                .child(visible_box(
                    NodeBuilder::form_control(FormControlType::InputPassword)
                        .tag("input")
                        .dom_id(5)
                        .redaction(RedactionDecision::RedactedHasBeenPassword)
                        .redacted_value_length(pw_len)
                        .field_value(&pw_canary_0),
                )),
        )
        .child(
            visible_box(NodeBuilder::element("div").dom_id(29))
                .child(label_for(
                    48,
                    6,
                    "textbox: show-password field, typed as password, then shown",
                ))
                .child(visible_box(
                    NodeBuilder::form_control(FormControlType::InputText)
                        .tag("input")
                        .dom_id(6)
                        .redaction(RedactionDecision::RedactedHasBeenPassword)
                        .redacted_value_length(pw_len)
                        .field_value(&pw_canary_1),
                ))
                .child(
                    visible_box(
                        NodeBuilder::form_control(FormControlType::ButtonButton)
                            .tag("button")
                            .dom_id(19)
                            .pressed_state(BrowserOsTriState::True),
                    )
                    .child(NodeBuilder::text("Show password")),
                ),
        ))
}

fn build_contract_v4_private_forms() -> Result<AnnotatedPageContent, Box<dyn Error>> {
    let mut input_group = vec![
        visible_box(NodeBuilder::element("legend").dom_id(30)).child(NodeBuilder::text(
            "group: autocomplete tokens on input type=text",
        )),
    ];
    for spec in PRIVATE_INPUT_SPECS {
        input_group.push(build_redacted_row(spec)?);
    }

    let mut textarea_group = vec![
        visible_box(NodeBuilder::element("legend").dom_id(36))
            .child(NodeBuilder::text("group: autocomplete tokens on textarea")),
    ];
    for spec in PRIVATE_TEXTAREA_SPECS {
        textarea_group.push(build_redacted_row(spec)?);
    }

    let form = visible_box(
        NodeBuilder::element("form")
            .dom_id(4)
            .aria_label("form: sign-in"),
    )
    .child(private_password_fieldset()?)
    .child(visible_box(NodeBuilder::element("fieldset").dom_id(20)).children(input_group))
    .child(visible_box(NodeBuilder::element("fieldset").dom_id(21)).children(textarea_group))
    .child(
        visible_box(NodeBuilder::element("div").dom_id(42))
            .child(label_for(
                85,
                17,
                "textbox: plain input with a public value",
            ))
            .child(visible_box(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(17)
                    .field_value("plain: public note"),
            )),
    )
    .child(
        visible_box(
            NodeBuilder::form_control(FormControlType::ButtonSubmit)
                .tag("button")
                .dom_id(22),
        )
        .child(NodeBuilder::text("button: submit sign-in")),
    );

    Ok(ApcBuilder::new("doc-private-forms-v4")
        .contract_version(CONTRACT_V4)
        .main_url("http://127.0.0.1/edge/private-forms.html")
        .main_title("Edge: private forms")
        .root(
            NodeBuilder::root()
                .tag("html")
                .dom_id(24)
                .bounding_box(0, 0, 800, 600)
                .child(
                    NodeBuilder::element("body")
                        .dom_id(25)
                        .bounding_box(0, 0, 800, 600)
                        .child(
                            visible_box(NodeBuilder::element("h1").dom_id(26))
                                .child(NodeBuilder::text("heading: private forms")),
                        )
                        .child(form),
                ),
        )
        .build())
}

fn assert_private_forms_masks(
    doc_token: &DocToken,
    emitted_by_key: &HashMap<NodeKey, (&AriaNode, RefTarget)>,
    oracle_by_bid: &HashMap<u64, &AriaNode>,
    default_nodes: &[AriaNode],
) -> Result<(), Box<dyn Error>> {
    let bullets_33 = "•".repeat(33);
    for pw_bid in [5_u64, 6] {
        let key = NodeKey::new(doc_token.clone(), DomNodeId::new(i32::try_from(pw_bid)?));
        let (emitted, _) = emitted_by_key
            .get(&key)
            .ok_or_else(|| format!("missing password node {pw_bid}"))?;
        let oracle_node = oracle_by_bid
            .get(&pw_bid)
            .ok_or_else(|| format!("oracle missing password node {pw_bid}"))?;
        assert_eq!(emitted.text.as_deref(), Some(bullets_33.as_str()));
        assert_eq!(emitted.text, oracle_node.text);
    }

    for spec in PRIVATE_INPUT_SPECS.iter().chain(PRIVATE_TEXTAREA_SPECS) {
        let expected_len = CANARY_PREFIX.len() + 1 + spec.canary_suffix.len();
        let key = NodeKey::new(doc_token.clone(), DomNodeId::new(spec.control_id));
        let (emitted, _) = emitted_by_key
            .get(&key)
            .ok_or_else(|| format!("missing redacted control {}", spec.control_id))?;
        assert_eq!(
            emitted.text.as_deref(),
            Some("•".repeat(expected_len).as_str())
        );
    }

    let (toggle, _) = emitted_by_key
        .get(&NodeKey::new(doc_token.clone(), DomNodeId::new(19)))
        .ok_or("missing Show password button 19")?;
    assert_eq!(toggle.pressed, Some(Tristate::True));

    let (heading, _) = emitted_by_key
        .get(&NodeKey::new(doc_token.clone(), DomNodeId::new(26)))
        .ok_or("missing heading 26")?;
    assert_eq!(heading.level, Some(Number::from(1)));

    let (public_input, _) = emitted_by_key
        .get(&NodeKey::new(doc_token.clone(), DomNodeId::new(17)))
        .ok_or("missing public input 17")?;
    assert_eq!(public_input.text.as_deref(), Some("plain: public note"));

    let default_textboxes: Vec<&AriaNode> = collect_all_nodes(default_nodes)
        .into_iter()
        .filter(|node| node.role == "textbox")
        .collect();
    assert_eq!(default_textboxes.len(), 13);
    assert_eq!(
        default_textboxes
            .iter()
            .filter(|node| node.text.as_deref() == Some(bullets_33.as_str()))
            .count(),
        2
    );
    Ok(())
}

#[test]
fn contract_v4_private_forms_masks_all_tokens_and_matches_recorded_oracle()
-> Result<(), Box<dyn Error>> {
    let oracle = load_oracle("private-forms")?;
    let frame = oracle
        .frames
        .first()
        .ok_or("private-forms.oracle.json missing main frame")?;
    let oracle_ref_ids: HashSet<u64> = frame.refs.values().copied().collect();
    let oracle_ai = parse_oracle_aria(&frame.aria.ai)?;
    let oracle_by_bid = index_oracle_ai_by_bid(&oracle_ai, &frame.refs);

    let tree = SnapshotTree::build(build_contract_v4_private_forms()?, BuildOptions::default())?;
    let cache = tree.name_cache();
    let doc_token = tree
        .document(DocId::MAIN)
        .ok_or("missing DocId::MAIN")?
        .token()
        .clone();

    for element in &frame.elements {
        assert_tree_element_matches_oracle("private-forms", &tree, &cache, &doc_token, element)?;
        if is_ref_comparable_role(&element.role) {
            let key = NodeKey::new(
                doc_token.clone(),
                DomNodeId::new(i32::try_from(element.backend_node_id)?),
            );
            let node = tree.node_by_key(&key).ok_or("verified node must exist")?;
            assert_eq!(
                tree.is_ref_worthy(node.id()),
                oracle_ref_ids.contains(&element.backend_node_id),
                "private-forms node {}: ref-worthiness mismatch",
                element.backend_node_id
            );
        }
    }

    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs));
    let default = tree.aria(AriaAsk::default_mode());
    let emitted_by_key = index_emitted_by_key(ai.nodes(), &refs)?;

    assert_private_forms_masks(&doc_token, &emitted_by_key, &oracle_by_bid, default.nodes())?;

    let (rendered, baseline) =
        render_and_check_safety("private-forms", &tree, &ai, default, &refs)?;
    assert_idempotent_delta("private-forms", &tree, &ai, &baseline, &mut refs);
    assert!(rendered.ai_playwright.contains("plain: public note"));
    assert!(rendered.ai_page.contains("[pressed]"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 5: modal-inert contract-v4 mirror vs modal-inert.oracle.json
// ---------------------------------------------------------------------------

fn modal_inert_dialog() -> NodeBuilder {
    let inert_inside_modal = visible_box(NodeBuilder::element("div").dom_id(21).inert(true))
        .child(
            visible_box(
                NodeBuilder::anchor("#inert-link")
                    .tag("a")
                    .dom_id(61)
                    .href("#inert-link")
                    .inert(true)
                    .clickability_reason(ClickabilityReason::CursorPointer),
            )
            .child(NodeBuilder::text("link: inert inside the modal")),
        )
        .child(
            visible_box(
                NodeBuilder::form_control(FormControlType::ButtonButton)
                    .tag("button")
                    .dom_id(31)
                    .inert(true),
            )
            .child(NodeBuilder::text("button: inert inside the modal")),
        )
        .child(label_for(121, 4, "textbox: inert inside the modal").inert(true))
        .child(visible_box(
            NodeBuilder::form_control(FormControlType::InputText)
                .tag("input")
                .dom_id(4)
                .inert(true),
        ));

    visible_box(
        NodeBuilder::element("dialog")
            .dom_id(18)
            .labelledby(vec![ref_node(19)]),
    )
    .child(
        visible_box(NodeBuilder::element("h2").dom_id(19)).child(NodeBuilder::text(
            "heading: modal title, also the dialog name",
        )),
    )
    .child(
        visible_box(NodeBuilder::element("p").dom_id(20))
            .child(NodeBuilder::text("paragraph: modal text")),
    )
    .child(
        visible_box(
            NodeBuilder::form_control(FormControlType::ButtonButton)
                .tag("button")
                .dom_id(24),
        )
        .child(NodeBuilder::text("button: modal confirm")),
    )
    .child(
        visible_box(
            NodeBuilder::form_control(FormControlType::ButtonButton)
                .tag("button")
                .dom_id(27),
        )
        .child(NodeBuilder::text("button: modal cancel")),
    )
    .child(inert_inside_modal)
}

fn build_contract_v4_modal_inert() -> AnnotatedPageContent {
    ApcBuilder::new("doc-modal-inert-v4")
        .contract_version(CONTRACT_V4)
        .main_url("http://127.0.0.1/edge/modal-inert.html")
        .main_title("Edge: modal and inert")
        .main_focused_node_id(24)
        .root(
            NodeBuilder::root()
                .tag("html")
                .dom_id(6)
                .inert(true)
                .bounding_box(0, 0, 800, 600)
                .child(
                    NodeBuilder::element("body")
                        .dom_id(7)
                        .inert(true)
                        .bounding_box(0, 0, 800, 600)
                        .child(
                            visible_box(NodeBuilder::element("h1").dom_id(8).inert(true))
                                .child(NodeBuilder::text("heading: page behind the modal")),
                        )
                        .child(
                            visible_box(
                                NodeBuilder::form_control(FormControlType::ButtonButton)
                                    .tag("button")
                                    .dom_id(12)
                                    .inert(true),
                            )
                            .child(NodeBuilder::text("button: behind modal")),
                        )
                        .child(
                            visible_box(NodeBuilder::element("p").dom_id(9).inert(true))
                                .child(NodeBuilder::text("paragraph: page text behind the modal")),
                        )
                        .child(
                            visible_box(NodeBuilder::element("div").dom_id(10).inert(true)).child(
                                visible_box(
                                    NodeBuilder::form_control(FormControlType::ButtonButton)
                                        .tag("button")
                                        .dom_id(15)
                                        .inert(true),
                                )
                                .child(NodeBuilder::text(
                                    "button: in an inert div outside the dialog",
                                )),
                            ),
                        )
                        .child(modal_inert_dialog()),
                ),
        )
        .build()
}

fn assert_modal_inert_marks(
    doc_token: &DocToken,
    emitted_by_key: &HashMap<NodeKey, (&AriaNode, RefTarget)>,
) -> Result<(), Box<dyn Error>> {
    for inert_root_id in [8_i32, 12, 9, 15, 21] {
        let key = NodeKey::new(doc_token.clone(), DomNodeId::new(inert_root_id));
        let (node, _) = emitted_by_key
            .get(&key)
            .ok_or_else(|| format!("missing inert root {inert_root_id} in ai tree"))?;
        assert_eq!(node.inert, Some(true));
    }

    for live_id in [18_i32, 19, 20, 24, 27] {
        let key = NodeKey::new(doc_token.clone(), DomNodeId::new(live_id));
        let (node, _) = emitted_by_key
            .get(&key)
            .ok_or_else(|| format!("missing live modal node {live_id} in ai tree"))?;
        assert_eq!(node.inert, None);
    }

    let (confirm, _) = emitted_by_key
        .get(&NodeKey::new(doc_token.clone(), DomNodeId::new(24)))
        .ok_or("missing confirm button 24")?;
    assert_eq!(confirm.active, Some(true));

    let (inert_link, _) = emitted_by_key
        .get(&NodeKey::new(doc_token.clone(), DomNodeId::new(61)))
        .ok_or("missing inert link 61")?;
    assert_eq!(inert_link.url.as_deref(), Some("#inert-link"));
    assert_eq!(inert_link.cursor.as_deref(), Some("pointer"));
    Ok(())
}

#[test]
fn contract_v4_modal_inert_marks_inert_and_matches_recorded_oracle() -> Result<(), Box<dyn Error>> {
    let oracle = load_oracle("modal-inert")?;
    let frame = oracle
        .frames
        .first()
        .ok_or("modal-inert.oracle.json missing main frame")?;
    let oracle_ref_ids: HashSet<u64> = frame.refs.values().copied().collect();
    let oracle_default = parse_oracle_aria(&frame.aria.default)?;

    let tree = SnapshotTree::build(build_contract_v4_modal_inert(), BuildOptions::default())?;
    let cache = tree.name_cache();
    let doc_token = tree
        .document(DocId::MAIN)
        .ok_or("missing DocId::MAIN")?
        .token()
        .clone();

    for element in &frame.elements {
        assert_tree_element_matches_oracle("modal-inert", &tree, &cache, &doc_token, element)?;
        if is_ref_comparable_role(&element.role) {
            let key = NodeKey::new(
                doc_token.clone(),
                DomNodeId::new(i32::try_from(element.backend_node_id)?),
            );
            let node = tree.node_by_key(&key).ok_or("verified node must exist")?;
            assert_eq!(
                tree.is_ref_worthy(node.id()),
                oracle_ref_ids.contains(&element.backend_node_id),
                "modal-inert node {}: inertness must never strip refs",
                element.backend_node_id
            );
        }
    }

    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs));
    let default = tree.aria(AriaAsk::default_mode());

    assert_eq!(
        default.nodes(),
        oracle_default.as_slice(),
        "modal-inert default mode must match oracle aria.default exactly"
    );

    let emitted_by_key = index_emitted_by_key(ai.nodes(), &refs)?;
    assert_modal_inert_marks(&doc_token, &emitted_by_key)?;

    let (rendered, baseline) = render_and_check_safety("modal-inert", &tree, &ai, default, &refs)?;
    assert_idempotent_delta("modal-inert", &tree, &ai, &baseline, &mut refs);
    assert!(rendered.ai_full.contains("# inert:"));
    assert!(rendered.ai_page.contains("# not listed:"));
    assert!(rendered.ai_page.contains("button: modal confirm"));
    assert!(rendered.ai_page.contains("button: modal cancel"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 6: nested-cross-site-frames contract-v4 mirror vs oracle (all 8 frames)
// ---------------------------------------------------------------------------

struct NestedFrameIds {
    doc_token: &'static str,
    url: &'static str,
    label: &'static str,
    html_id: i32,
    body_id: i32,
    h1_id: i32,
    button_id: i32,
    input_id: i32,
}

const NESTED_FRAMES: &[NestedFrameIds] = &[
    NestedFrameIds {
        doc_token: "doc-top",
        url: "http://127.0.0.1/nested/top.html",
        label: "Top",
        html_id: 8,
        body_id: 9,
        h1_id: 10,
        button_id: 14,
        input_id: 4,
    },
    NestedFrameIds {
        doc_token: "doc-a1",
        url: "http://127.0.0.1/nested/a1.html",
        label: "A1",
        html_id: 25,
        body_id: 26,
        h1_id: 27,
        button_id: 30,
        input_id: 22,
    },
    NestedFrameIds {
        doc_token: "doc-b2",
        url: "http://localhost/nested/b2.html",
        label: "B2",
        html_id: 7,
        body_id: 8,
        h1_id: 9,
        button_id: 12,
        input_id: 3,
    },
    NestedFrameIds {
        doc_token: "doc-b3",
        url: "http://localhost/nested/b3.html",
        label: "B3",
        html_id: 20,
        body_id: 21,
        h1_id: 22,
        button_id: 24,
        input_id: 18,
    },
    NestedFrameIds {
        doc_token: "doc-b1",
        url: "http://localhost/nested/b1.html",
        label: "B1",
        html_id: 61,
        body_id: 67,
        h1_id: 28,
        button_id: 30,
        input_id: 4,
    },
    NestedFrameIds {
        doc_token: "doc-c2",
        url: "http://[::1]/nested/c2.html",
        label: "C2",
        html_id: 18,
        body_id: 24,
        h1_id: 6,
        button_id: 8,
        input_id: 1,
    },
    NestedFrameIds {
        doc_token: "doc-a3",
        url: "http://127.0.0.1/nested/a3.html",
        label: "A3",
        html_id: 72,
        body_id: 78,
        h1_id: 39,
        button_id: 41,
        input_id: 36,
    },
    NestedFrameIds {
        doc_token: "doc-c4",
        url: "http://[::1]/nested/c4.html",
        label: "C4",
        html_id: 34,
        body_id: 40,
        h1_id: 12,
        button_id: 14,
        input_id: 4,
    },
];

fn nested_frame_body(spec: &NestedFrameIds, iframes: Vec<NodeBuilder>) -> NodeBuilder {
    let mut children = vec![
        visible_box(NodeBuilder::element("h1").dom_id(spec.h1_id))
            .child(NodeBuilder::text(spec.label)),
        visible_box(
            NodeBuilder::form_control(FormControlType::ButtonButton)
                .tag("button")
                .dom_id(spec.button_id),
        )
        .child(NodeBuilder::text(&format!("{} button", spec.label))),
        visible_box(
            NodeBuilder::form_control(FormControlType::InputText)
                .tag("input")
                .dom_id(spec.input_id)
                .aria_label(&format!("{} field", spec.label)),
        ),
    ];
    children.extend(iframes);
    NodeBuilder::root()
        .tag("html")
        .dom_id(spec.html_id)
        .bounding_box(0, 0, 800, 600)
        .child(
            NodeBuilder::element("body")
                .dom_id(spec.body_id)
                .bounding_box(0, 0, 800, 600)
                .children(children),
        )
}

fn nested_child_iframe(
    owner_id: i32,
    spec: &NestedFrameIds,
    child_iframes: Vec<NodeBuilder>,
) -> NodeBuilder {
    let frame = FrameBuilder::new(spec.doc_token)
        .url(spec.url)
        .root(nested_frame_body(spec, child_iframes));
    visible_box(NodeBuilder::iframe(frame).tag("iframe").dom_id(owner_id))
}

fn build_contract_v4_nested_frames() -> Result<AnnotatedPageContent, Box<dyn Error>> {
    let [top, a1, b2, b3, b1, c2, a3, c4] = NESTED_FRAMES else {
        return Err("expected exactly 8 nested frame specs".into());
    };
    let iframe_b3 = nested_child_iframe(10, b3, Vec::new());
    let iframe_b2 = nested_child_iframe(28, b2, vec![iframe_b3]);
    let iframe_a1 = nested_child_iframe(11, a1, vec![iframe_b2]);

    let iframe_c4 = nested_child_iframe(44, c4, Vec::new());
    let iframe_a3 = nested_child_iframe(11, a3, vec![iframe_c4]);
    let iframe_c2 = nested_child_iframe(33, c2, vec![iframe_a3]);
    let iframe_b1 = nested_child_iframe(12, b1, vec![iframe_c2]);

    Ok(ApcBuilder::new(top.doc_token)
        .contract_version(CONTRACT_V4)
        .main_url(top.url)
        .main_title(top.label)
        .main_focused_node_id(top.body_id)
        .root(nested_frame_body(top, vec![iframe_a1, iframe_b1]))
        .build())
}

fn stitch_oracle_trees(mut per_frame: Vec<Vec<AriaNode>>) -> Result<Vec<AriaNode>, Box<dyn Error>> {
    if per_frame.is_empty() {
        return Ok(Vec::new());
    }
    let mut cursor = 1_usize;
    let mut root_nodes = per_frame
        .first_mut()
        .map(mem::take)
        .ok_or("missing root frame nodes")?;

    let mut stack: Vec<&mut AriaNode> = root_nodes.iter_mut().rev().collect();
    while let Some(node) = stack.pop() {
        if node.role == "iframe" {
            let child_nodes = per_frame
                .get_mut(cursor)
                .map(mem::take)
                .ok_or_else(|| format!("missing child oracle frame at preorder index {cursor}"))?;
            cursor += 1;
            node.children = child_nodes
                .into_iter()
                .map(|child| AriaChild::Node(Box::new(child)))
                .collect();
        }
        for child in node.children.iter_mut().rev() {
            if let AriaChild::Node(child_node) = child {
                stack.push(child_node);
            }
        }
    }

    assert_eq!(
        cursor,
        per_frame.len(),
        "every oracle frame must be stitched into an iframe slot"
    );
    Ok(root_nodes)
}

fn assert_nested_frame_refs_and_elements(
    tree: &SnapshotTree,
    cache: &NameCache<'_>,
    oracle: &Oracle,
    frame_to_doc: &HashMap<String, DocId>,
    refs: &TabRefTable,
) -> Result<(), Box<dyn Error>> {
    for (frame_idx, frame) in oracle.frames.iter().enumerate() {
        let doc_id = *frame_to_doc
            .get(&frame.frame)
            .ok_or("all 8 frames must map to a DocId")?;
        let doc_token = tree
            .document(doc_id)
            .ok_or("mapped DocId missing")?
            .token()
            .clone();
        let expected_prefix = if frame_idx == 0 {
            "e".to_owned()
        } else {
            format!("f{frame_idx}e")
        };

        for element in &frame.elements {
            assert_tree_element_matches_oracle(
                "nested-cross-site-frames",
                tree,
                cache,
                &doc_token,
                element,
            )?;
        }

        for (ref_str, &bid) in &frame.refs {
            assert!(
                ref_str.starts_with(&expected_prefix),
                "frame {frame_idx} ref {ref_str:?} must start with {expected_prefix:?}"
            );
            let expected_key = NodeKey::new(doc_token.clone(), DomNodeId::new(i32::try_from(bid)?));
            assert_eq!(
                refs.resolve_str(ref_str),
                RefLookup::Key(RefTarget {
                    key: expected_key,
                    click_proxy: None,
                }),
                "frame {frame_idx} ref {ref_str:?} must resolve to exact (DocToken, DomNodeId)"
            );
        }
    }
    Ok(())
}

#[test]
fn contract_v4_nested_cross_site_frames_stitches_all_frames_matching_recorded_oracle()
-> Result<(), Box<dyn Error>> {
    let oracle = load_oracle("nested-cross-site-frames")?;
    assert_eq!(
        oracle.frames.len(),
        NESTED_FRAMES.len(),
        "nested-cross-site-frames.oracle.json must list all 8 frames"
    );

    let tree = SnapshotTree::build(build_contract_v4_nested_frames()?, BuildOptions::default())?;
    let cache = tree.name_cache();
    let MappedOracleFrames {
        frame_to_doc,
        partial_frames,
    } = map_oracle_frames(&tree, &oracle)?;
    assert_eq!(partial_frames, 0, "all 8 frames must stitch cleanly");
    assert_eq!(frame_to_doc.len(), 8);

    let mut refs = TabRefTable::new();
    let ai = tree.aria(AriaAsk::ai(&mut refs));
    let default = tree.aria(AriaAsk::default_mode());

    let oracle_ai_frames: Vec<Vec<AriaNode>> = oracle
        .frames
        .iter()
        .map(|frame| parse_oracle_aria(&frame.aria.ai))
        .collect::<Result<_, _>>()?;
    let oracle_default_frames: Vec<Vec<AriaNode>> = oracle
        .frames
        .iter()
        .map(|frame| parse_oracle_aria(&frame.aria.default))
        .collect::<Result<_, _>>()?;

    let expected_ai = stitch_oracle_trees(oracle_ai_frames)?;
    let expected_default = stitch_oracle_trees(oracle_default_frames)?;

    assert_eq!(
        ai.nodes(),
        expected_ai.as_slice(),
        "stitched 8-frame ai tree must match nested-cross-site-frames.oracle.json in exact preorder (e*, f1e*..f7e*)"
    );
    assert_eq!(
        default.nodes(),
        expected_default.as_slice(),
        "stitched 8-frame default tree must match nested-cross-site-frames.oracle.json exactly"
    );

    assert_nested_frame_refs_and_elements(&tree, &cache, &oracle, &frame_to_doc, &refs)?;

    let (rendered, baseline) =
        render_and_check_safety("nested-cross-site-frames", &tree, &ai, default, &refs)?;
    assert_idempotent_delta("nested-cross-site-frames", &tree, &ai, &baseline, &mut refs);
    for label in ["Top", "A1", "B2", "B3", "B1", "C2", "A3", "C4"] {
        assert!(
            rendered.ai_playwright.contains(&format!("{label} button"))
                && rendered.ai_page.contains(&format!("{label} field")),
            "rendered multi-frame snapshot must include {label} controls"
        );
    }
    Ok(())
}
