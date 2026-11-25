//! Offline parity checks comparing [`neo_snapshot_apc::tree`] roles, hidden flags, and states
//! against the 16 recorded Playwright oracles (`tests/fixtures/<name>.{pb,oracle.json}`).
//!
//! Frames join by `owner` `<iframe>` `DomNodeId` and elements join by `backendNodeId`. Stock
//! recordings assert contract section 9 (zero inferred roles) plus stock-expressible focus,
//! `ContentHidden`, and layout box facts; recordings at contract v3+ and v4+ assert full role,
//! hidden, and state equality.

#[path = "support/fixture_pages.rs"]
#[expect(
    dead_code,
    reason = "only FIXTURES is read here; page loaders serve the recorder"
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

use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use fixture_pages::FIXTURES;
use neo_snapshot::{Invalid, Tristate};
use neo_snapshot_apc::proto::{AnnotatedPageContent, BrowserOsPageContentData};
use neo_snapshot_apc::tree::{
    BuildError, BuildOptions, DocId, DomNodeId, InvalidState, MIN_CONTRACT_VERSION, NodeKey, Role,
    SnapshotTree, State, TreeNode, TriState,
};
use oracle::{Oracle, OracleElement, OracleFrame, OracleStates};
use serde::Deserialize;

const CONTRACT_ROLES_HIDDEN_VERSION: u32 = 3;
const CONTRACT_STATES_VERSION: u32 = 4;

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
    let mut reader = serde_json::Deserializer::from_str(&text);
    reader.disable_recursion_limit();
    let oracle =
        Oracle::deserialize(&mut reader).map_err(|err| format!("{}: {err}", path.display()))?;
    reader.end()?;
    Ok(oracle)
}

#[derive(Default, Debug)]
struct FixtureComparison {
    oracle_frames: usize,
    joined_frames: usize,
    partial_frames: usize,
    joined_elements: usize,
    active_matches: usize,
    active_true_oracle: usize,
    active_true_matches: usize,
    box_visible_matches: usize,
    content_hidden_total: usize,
    content_hidden_implies_oracle_hidden: usize,
    spurious_stock_roles: usize,
    active_mismatches: Vec<String>,
    box_visible_mismatches: Vec<String>,
    contract_mismatches: Vec<String>,
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

fn to_oracle_tristate(value: TriState) -> Tristate {
    match value {
        TriState::False => Tristate::False,
        TriState::True => Tristate::True,
        TriState::Mixed => Tristate::Mixed,
    }
}

fn to_oracle_invalid(value: InvalidState) -> Invalid {
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

fn compare_joined_element(
    fixture: &str,
    frame: &OracleFrame,
    element: &OracleElement,
    node: &TreeNode,
    contract_version: Option<u32>,
    stats: &mut FixtureComparison,
) {
    stats.joined_elements += 1;

    if element.states.active {
        stats.active_true_oracle += 1;
    }
    if node.state().active() == element.states.active {
        stats.active_matches += 1;
        if element.states.active {
            stats.active_true_matches += 1;
        }
    } else {
        stats.active_mismatches.push(format!(
            "{fixture} [{}] node {}: active tree={} oracle={}",
            frame.url,
            element.backend_node_id,
            node.state().active(),
            element.states.active,
        ));
    }

    if node.hidden().is_box_visible() == element.visible {
        stats.box_visible_matches += 1;
    } else {
        stats.box_visible_mismatches.push(format!(
            "{fixture} [{}] node {}: box_visible tree={} oracle={} (role={:?})",
            frame.url,
            element.backend_node_id,
            node.hidden().is_box_visible(),
            element.visible,
            element.role,
        ));
    }

    if node.hidden().is_content_hidden() {
        stats.content_hidden_total += 1;
        if element.hidden {
            stats.content_hidden_implies_oracle_hidden += 1;
        }
    }

    let version = contract_version.unwrap_or(0);
    if version >= CONTRACT_ROLES_HIDDEN_VERSION {
        let expected_role = Role::parse(&element.role);
        if node.role() != expected_role {
            stats.contract_mismatches.push(format!(
                "{fixture} [{}] node {}: role tree={:?} oracle={:?}",
                frame.url,
                element.backend_node_id,
                node.role(),
                expected_role
            ));
        }
        if node.hidden().is_hidden_for_aria() != element.hidden {
            stats.contract_mismatches.push(format!(
                "{fixture} [{}] node {}: hidden tree={} oracle={}",
                frame.url,
                element.backend_node_id,
                node.hidden().is_hidden_for_aria(),
                element.hidden
            ));
        }
    }
    if version >= CONTRACT_STATES_VERSION && !states_equal(node.state(), &element.states) {
        stats.contract_mismatches.push(format!(
            "{fixture} [{}] node {}: state tree={:?} oracle={:?}",
            frame.url,
            element.backend_node_id,
            node.state(),
            element.states
        ));
    }
}

fn compare_fixture(name: &str) -> Result<FixtureComparison, Box<dyn Error>> {
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
    let MappedOracleFrames {
        frame_to_doc,
        partial_frames,
    } = map_oracle_frames(&tree, &oracle)?;

    let mut stats = FixtureComparison {
        oracle_frames: oracle.frames.len(),
        joined_frames: frame_to_doc.len(),
        partial_frames,
        ..FixtureComparison::default()
    };

    if recorded_version.is_none() {
        stats.spurious_stock_roles = tree
            .nodes()
            .iter()
            .filter(|node| node.role().is_some())
            .count();
    }

    for frame in &oracle.frames {
        let Some(&doc_id) = frame_to_doc.get(&frame.frame) else {
            continue;
        };
        let doc = tree
            .document(doc_id)
            .ok_or("mapped doc id missing from SnapshotTree")?;
        for element in &frame.elements {
            let key = NodeKey::new(
                doc.token().clone(),
                DomNodeId::new(i32::try_from(element.backend_node_id)?),
            );
            if let Some(node) = tree.node_by_key(&key) {
                compare_joined_element(name, frame, element, node, recorded_version, &mut stats);
            }
        }
    }

    Ok(stats)
}

#[test]
fn every_role_recorded_across_all_16_oracles_parses_into_role_enum() -> Result<(), Box<dyn Error>> {
    let mut total_elements = 0;
    for fixture in FIXTURES {
        let oracle = load_oracle(fixture.name)?;
        for frame in &oracle.frames {
            for element in &frame.elements {
                total_elements += 1;
                let parsed = Role::parse(&element.role).ok_or_else(|| {
                    format!(
                        "{}: oracle element {} has role {:?} missing from Role",
                        fixture.name, element.backend_node_id, element.role
                    )
                })?;
                assert_eq!(parsed.as_str(), element.role);
            }
        }
    }
    assert!(
        total_elements > 600,
        "expected all 16 fixtures to cover >600 oracle elements, saw {total_elements}"
    );
    Ok(())
}

#[test]
fn recorded_fixtures_agree_with_playwright_oracles_on_all_expressible_facts()
-> Result<(), Box<dyn Error>> {
    let mut total_joined = 0;
    let mut total_focused_matched = 0;

    for fixture in FIXTURES {
        let stats = compare_fixture(fixture.name)?;
        total_joined += stats.joined_elements;
        total_focused_matched += stats.active_true_matches;

        assert_eq!(
            stats.spurious_stock_roles, 0,
            "{}: contract §9 forbids deriving any role from stock Chromium APC fields",
            fixture.name
        );
        assert_eq!(
            stats.joined_frames + stats.partial_frames,
            stats.oracle_frames,
            "{}: every oracle frame must join a TreeDoc or a partial iframe",
            fixture.name
        );
        assert!(
            stats.joined_elements > 0,
            "{}: expected joined elements between .pb and oracle",
            fixture.name
        );
        assert_eq!(
            stats.active_matches, stats.joined_elements,
            "{}: active focus mismatches: {:?}",
            fixture.name, stats.active_mismatches
        );
        assert_eq!(
            stats.active_true_matches, stats.active_true_oracle,
            "{}: every focused oracle element must be active in SnapshotTree",
            fixture.name
        );
        assert_eq!(
            stats.content_hidden_implies_oracle_hidden, stats.content_hidden_total,
            "{}: every ContentHidden node must be hidden in the Playwright oracle",
            fixture.name
        );
        // Blink repairs a zero-size ancestor's outer_bounding_box from its descendants
        // (design §5.2): the two zero-size <input type="file"> controls on `zero-size-controls`
        // inherit their UA shadow button's box in APC.
        if fixture.name == "zero-size-controls" {
            assert_eq!(
                stats.box_visible_mismatches.len(),
                2,
                "{}: expected only the two zero-size file inputs to differ on box_visible, got {:?}",
                fixture.name,
                stats.box_visible_mismatches
            );
        } else {
            assert_eq!(
                stats.box_visible_matches, stats.joined_elements,
                "{}: box_visible mismatches: {:?}",
                fixture.name, stats.box_visible_mismatches
            );
        }
        assert!(
            stats.contract_mismatches.is_empty(),
            "{}: contract role/hidden/state mismatches: {:?}",
            fixture.name,
            stats.contract_mismatches
        );
    }

    assert!(
        total_joined >= 640,
        "expected >= 640 joined oracle elements across all 16 fixtures, saw {total_joined}"
    );
    assert!(
        total_focused_matched >= 2,
        "expected focused elements on states and modal-inert to be verified, saw {total_focused_matched}"
    );

    Ok(())
}
