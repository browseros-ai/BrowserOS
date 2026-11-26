//! Offline parity checks comparing [`neo_snapshot_apc::tree`] roles, hidden flags, and states
//! against the 16 recorded Playwright oracles (`tests/fixtures/<name>.{pb,oracle.json}`).
//!
//! Frames join by `owner` `<iframe>` `DomNodeId` and elements join by `backendNodeId`. Every
//! recording carries contract v6, so the elements that fail to join or disagree on role, hidden,
//! box visibility or any state must equal the parity ledger (`support/parity_ledger.rs`), and every
//! `ContentHidden` node must be hidden for the oracle.

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
#[path = "support/parity_ledger.rs"]
#[expect(
    dead_code,
    reason = "this test measures join, role, hidden and state; the name fields belong to tree_names_oracle.rs"
)]
mod parity_ledger;

use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::{fmt, fs};

use fixture_pages::FIXTURES;
use neo_snapshot::{Invalid, Tristate};
use neo_snapshot_apc::proto::AnnotatedPageContent;
use neo_snapshot_apc::tree::{
    BuildOptions, DocId, DomNodeId, InvalidState, NodeKey, Role, SnapshotTree, State, TreeNode,
    TriState,
};
use oracle::{Oracle, OracleElement, OracleFrame, OracleStates};
use parity_ledger::{Field, Measured};
use serde::Deserialize;

/// The first contract version that carries roles, hidden-ness and states (contract PRs C3, C4).
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
    active_true_matches: usize,
    content_hidden_total: usize,
    content_hidden_implies_oracle_hidden: usize,
    disagreements: Vec<Measured>,
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

/// One element's view of a single fact, as `check` compares it.
struct Fact<'a> {
    field: Field,
    equal: bool,
    tree: &'a dyn fmt::Debug,
    oracle: &'a dyn fmt::Debug,
}

/// Records `fact` when the two sides differ, keyed for the parity ledger: a child frame by its
/// URL, the main frame (the only one with no `parent`) by `None`.
fn check(
    stats: &mut FixtureComparison,
    fixture: &'static str,
    frame: &OracleFrame,
    element: &OracleElement,
    fact: &Fact<'_>,
) {
    if fact.equal {
        return;
    }
    stats.disagreements.push(Measured {
        fixture,
        frame: frame.parent.as_ref().map(|_| frame.url.clone()),
        backend_node_id: element.backend_node_id,
        field: fact.field,
        tree: if fact.field == Field::Join {
            String::new()
        } else {
            format!("{:?}", fact.tree)
        },
        detail: format!(
            "{fixture} [{}] node {} ({:?}): {:?} tree={:?} oracle={:?}",
            frame.url, element.backend_node_id, element.role, fact.field, fact.tree, fact.oracle
        ),
    });
}

/// Each state `toAriaNode` offers, compared on its own so a ledger entry for one state cannot
/// hide a regression in another on the same node.
fn check_states(
    stats: &mut FixtureComparison,
    fixture: &'static str,
    frame: &OracleFrame,
    element: &OracleElement,
    actual: State,
) {
    let expected: &OracleStates = &element.states;
    let checked = actual.checked().map(to_oracle_tristate);
    let pressed = actual.pressed().map(to_oracle_tristate);
    let invalid = actual.invalid().map(to_oracle_invalid);
    let level = actual.level();
    let level_equal = match (level, expected.level.as_ref()) {
        (None, None) => true,
        (Some(level), Some(number)) => number.as_u64() == Some(u64::from(level)),
        _ => false,
    };
    let facts = [
        Fact {
            field: Field::Checked,
            equal: checked == expected.checked,
            tree: &checked,
            oracle: &expected.checked,
        },
        Fact {
            field: Field::Pressed,
            equal: pressed == expected.pressed,
            tree: &pressed,
            oracle: &expected.pressed,
        },
        Fact {
            field: Field::Expanded,
            equal: actual.expanded() == expected.expanded,
            tree: &actual.expanded(),
            oracle: &expected.expanded,
        },
        Fact {
            field: Field::Selected,
            equal: actual.selected() == expected.selected,
            tree: &actual.selected(),
            oracle: &expected.selected,
        },
        Fact {
            field: Field::Disabled,
            equal: actual.disabled() == expected.disabled,
            tree: &actual.disabled(),
            oracle: &expected.disabled,
        },
        Fact {
            field: Field::Invalid,
            equal: invalid == expected.invalid,
            tree: &invalid,
            oracle: &expected.invalid,
        },
        Fact {
            field: Field::Level,
            equal: level_equal,
            tree: &level,
            oracle: &expected.level,
        },
        Fact {
            field: Field::Active,
            equal: actual.active() == expected.active,
            tree: &actual.active(),
            oracle: &expected.active,
        },
    ];
    for fact in &facts {
        check(stats, fixture, frame, element, fact);
    }
}

fn compare_joined_element(
    fixture: &'static str,
    frame: &OracleFrame,
    element: &OracleElement,
    node: &TreeNode,
    stats: &mut FixtureComparison,
) {
    stats.joined_elements += 1;
    if element.states.active && node.state().active() {
        stats.active_true_matches += 1;
    }
    if node.hidden().is_content_hidden() {
        stats.content_hidden_total += 1;
        if element.hidden {
            stats.content_hidden_implies_oracle_hidden += 1;
        }
    }

    let role = node.role();
    let expected_role = Role::parse(&element.role);
    check(
        stats,
        fixture,
        frame,
        element,
        &Fact {
            field: Field::Role,
            equal: role == expected_role,
            tree: &role,
            oracle: &expected_role,
        },
    );
    let hidden = node.hidden().is_hidden_for_aria();
    check(
        stats,
        fixture,
        frame,
        element,
        &Fact {
            field: Field::Hidden,
            equal: hidden == element.hidden,
            tree: &hidden,
            oracle: &element.hidden,
        },
    );
    let box_visible = node.hidden().is_box_visible();
    check(
        stats,
        fixture,
        frame,
        element,
        &Fact {
            field: Field::BoxVisible,
            equal: box_visible == element.visible,
            tree: &box_visible,
            oracle: &element.visible,
        },
    );
    check_states(stats, fixture, frame, element, node.state());
}

fn compare_fixture(name: &'static str) -> Result<FixtureComparison, Box<dyn Error>> {
    let apc = load_pb(name)?;
    let oracle = load_oracle(name)?;
    let recorded_version = apc
        .browseros
        .as_ref()
        .and_then(|meta| meta.contract_version);
    assert!(
        recorded_version >= Some(CONTRACT_STATES_VERSION),
        "{name}: recorded at contract {recorded_version:?}; parity needs v{CONTRACT_STATES_VERSION}+ \
         roles, hidden-ness and states (tests/fixtures/README.md re-records with --contract)"
    );

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

    for frame in &oracle.frames {
        let doc = match frame_to_doc.get(&frame.frame) {
            Some(&doc_id) => Some(
                tree.document(doc_id)
                    .ok_or("mapped doc id missing from SnapshotTree")?,
            ),
            None => None,
        };
        for element in &frame.elements {
            let node = match doc {
                Some(doc) => tree.node_by_key(&NodeKey::new(
                    doc.token().clone(),
                    DomNodeId::new(i32::try_from(element.backend_node_id)?),
                )),
                None => None,
            };
            if let Some(node) = node {
                compare_joined_element(name, frame, element, node, &mut stats);
            } else {
                check(
                    &mut stats,
                    name,
                    frame,
                    element,
                    &Fact {
                        field: Field::Join,
                        equal: false,
                        tree: &"no SnapshotTree node",
                        oracle: &element.name,
                    },
                );
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
    let mut disagreements = Vec::new();

    for fixture in FIXTURES {
        let stats = compare_fixture(fixture.name)?;
        total_joined += stats.joined_elements;
        total_focused_matched += stats.active_true_matches;

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
            stats.content_hidden_implies_oracle_hidden, stats.content_hidden_total,
            "{}: every ContentHidden node must be hidden in the Playwright oracle",
            fixture.name
        );
        disagreements.extend(stats.disagreements);
    }
    parity_ledger::assert_measured_equals_ledger(
        &disagreements,
        &[
            Field::Join,
            Field::Role,
            Field::Hidden,
            Field::BoxVisible,
            Field::Checked,
            Field::Pressed,
            Field::Expanded,
            Field::Selected,
            Field::Disabled,
            Field::Invalid,
            Field::Level,
            Field::Active,
        ],
    );

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
