//! Offline parity checks comparing [`neo_snapshot_apc::tree::NameCache`] accessible names,
//! descriptions, and error messages against the 16 recorded Playwright oracles
//! (`tests/fixtures/<name>.{pb,oracle.json}`) and a contract-v4 mirror of `tests/pages/names.html`.
//!
//! Frames join by `owner` `<iframe>` `DomNodeId` and elements join by `backendNodeId`. Stock
//! recordings assert contract section 9 (zero names, descriptions, or error messages derived from
//! stock `ContentAttributes::label` or `ImageInfo::image_caption`) and pin per-page counts;
//! recordings at contract v4+ assert full name, description, and error-message parity (allowing
//! only documented contract gap G6 on `names.html`'s `err-required` input).

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
use neo_snapshot_apc::proto::{
    AnnotatedPageContent, BrowserOsPageContentData, BrowserOsRef, ContentData, ContentNode,
    FormControlType, RedactionDecision,
};
use neo_snapshot_apc::tree::{
    ApcBuilder, BuildError, BuildOptions, DocId, DomNodeId, MIN_CONTRACT_VERSION, NameCache,
    NameVisibility, NodeBuilder, NodeKey, Role, SnapshotTree, TreeNode,
};
use oracle::{Oracle, OracleElement, OracleFrame};
use serde::Deserialize;

const CONTRACT_NAMES_VERSION: u32 = 4;
const CONTRACT_GAP_G6_ERROR_MESSAGE: &str = "error message: shown because validity fails";
const ERR_REQUIRED_BACKEND_NODE_ID: u64 = 17;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StockFixtureCounts {
    fixture: &'static str,
    joined_elements: usize,
    stock_label_nodes: usize,
    stock_image_caption_nodes: usize,
    name_matches_on_stock: usize,
    oracle_non_empty_names: usize,
    description_matches_on_stock: usize,
    oracle_non_empty_descriptions: usize,
    error_message_matches_on_stock: usize,
    oracle_non_empty_errors: usize,
}

/// Pinned per-page measurements on the 16 stock Chromium 158 `.pb` recordings.
const EXPECTED_STOCK_COUNTS: &[StockFixtureCounts] = &[
    StockFixtureCounts {
        fixture: "nested-cross-site-frames",
        joined_elements: 28,
        stock_label_nodes: 15,
        stock_image_caption_nodes: 0,
        name_matches_on_stock: 4,
        oracle_non_empty_names: 24,
        description_matches_on_stock: 28,
        oracle_non_empty_descriptions: 0,
        error_message_matches_on_stock: 28,
        oracle_non_empty_errors: 0,
    },
    StockFixtureCounts {
        fixture: "shadow-dom",
        joined_elements: 9,
        stock_label_nodes: 2,
        stock_image_caption_nodes: 0,
        name_matches_on_stock: 3,
        oracle_non_empty_names: 6,
        description_matches_on_stock: 9,
        oracle_non_empty_descriptions: 0,
        error_message_matches_on_stock: 9,
        oracle_non_empty_errors: 0,
    },
    StockFixtureCounts {
        fixture: "zero-size-checkbox",
        joined_elements: 4,
        stock_label_nodes: 0,
        stock_image_caption_nodes: 0,
        name_matches_on_stock: 1,
        oracle_non_empty_names: 3,
        description_matches_on_stock: 4,
        oracle_non_empty_descriptions: 0,
        error_message_matches_on_stock: 4,
        oracle_non_empty_errors: 0,
    },
    StockFixtureCounts {
        fixture: "covered-button",
        joined_elements: 8,
        stock_label_nodes: 1,
        stock_image_caption_nodes: 0,
        name_matches_on_stock: 3,
        oracle_non_empty_names: 5,
        description_matches_on_stock: 8,
        oracle_non_empty_descriptions: 0,
        error_message_matches_on_stock: 8,
        oracle_non_empty_errors: 0,
    },
    StockFixtureCounts {
        fixture: "sensitive-form",
        joined_elements: 6,
        stock_label_nodes: 0,
        stock_image_caption_nodes: 0,
        name_matches_on_stock: 1,
        oracle_non_empty_names: 5,
        description_matches_on_stock: 6,
        oracle_non_empty_descriptions: 0,
        error_message_matches_on_stock: 6,
        oracle_non_empty_errors: 0,
    },
    StockFixtureCounts {
        fixture: "hung-subframe",
        joined_elements: 6,
        stock_label_nodes: 2,
        stock_image_caption_nodes: 0,
        name_matches_on_stock: 2,
        oracle_non_empty_names: 4,
        description_matches_on_stock: 6,
        oracle_non_empty_descriptions: 0,
        error_message_matches_on_stock: 6,
        oracle_non_empty_errors: 0,
    },
    StockFixtureCounts {
        fixture: "deep-nesting",
        joined_elements: 123,
        stock_label_nodes: 0,
        stock_image_caption_nodes: 0,
        name_matches_on_stock: 121,
        oracle_non_empty_names: 2,
        description_matches_on_stock: 123,
        oracle_non_empty_descriptions: 0,
        error_message_matches_on_stock: 123,
        oracle_non_empty_errors: 0,
    },
    StockFixtureCounts {
        fixture: "shadow-roots",
        joined_elements: 22,
        stock_label_nodes: 1,
        stock_image_caption_nodes: 0,
        name_matches_on_stock: 1,
        oracle_non_empty_names: 21,
        description_matches_on_stock: 22,
        oracle_non_empty_descriptions: 0,
        error_message_matches_on_stock: 22,
        oracle_non_empty_errors: 0,
    },
    StockFixtureCounts {
        fixture: "zero-size-controls",
        joined_elements: 13,
        stock_label_nodes: 0,
        stock_image_caption_nodes: 0,
        name_matches_on_stock: 1,
        oracle_non_empty_names: 12,
        description_matches_on_stock: 13,
        oracle_non_empty_descriptions: 0,
        error_message_matches_on_stock: 13,
        oracle_non_empty_errors: 0,
    },
    StockFixtureCounts {
        fixture: "nosize",
        joined_elements: 27,
        stock_label_nodes: 14,
        stock_image_caption_nodes: 0,
        name_matches_on_stock: 4,
        oracle_non_empty_names: 23,
        description_matches_on_stock: 27,
        oracle_non_empty_descriptions: 0,
        error_message_matches_on_stock: 27,
        oracle_non_empty_errors: 0,
    },
    StockFixtureCounts {
        fixture: "ua-shadow",
        joined_elements: 15,
        stock_label_nodes: 8,
        stock_image_caption_nodes: 0,
        name_matches_on_stock: 6,
        oracle_non_empty_names: 9,
        description_matches_on_stock: 15,
        oracle_non_empty_descriptions: 0,
        error_message_matches_on_stock: 15,
        oracle_non_empty_errors: 0,
    },
    StockFixtureCounts {
        fixture: "roles",
        joined_elements: 141,
        stock_label_nodes: 16,
        stock_image_caption_nodes: 2,
        name_matches_on_stock: 56,
        oracle_non_empty_names: 85,
        description_matches_on_stock: 140,
        oracle_non_empty_descriptions: 1,
        error_message_matches_on_stock: 141,
        oracle_non_empty_errors: 0,
    },
    StockFixtureCounts {
        fixture: "names",
        joined_elements: 69,
        stock_label_nodes: 27,
        stock_image_caption_nodes: 4,
        name_matches_on_stock: 7,
        oracle_non_empty_names: 62,
        description_matches_on_stock: 54,
        oracle_non_empty_descriptions: 15,
        error_message_matches_on_stock: 67,
        oracle_non_empty_errors: 2,
    },
    StockFixtureCounts {
        fixture: "states",
        joined_elements: 140,
        stock_label_nodes: 24,
        stock_image_caption_nodes: 0,
        name_matches_on_stock: 9,
        oracle_non_empty_names: 131,
        description_matches_on_stock: 140,
        oracle_non_empty_descriptions: 0,
        error_message_matches_on_stock: 140,
        oracle_non_empty_errors: 0,
    },
    StockFixtureCounts {
        fixture: "private-forms",
        joined_elements: 21,
        stock_label_nodes: 1,
        stock_image_caption_nodes: 0,
        name_matches_on_stock: 1,
        oracle_non_empty_names: 20,
        description_matches_on_stock: 21,
        oracle_non_empty_descriptions: 0,
        error_message_matches_on_stock: 21,
        oracle_non_empty_errors: 0,
    },
    StockFixtureCounts {
        fixture: "modal-inert",
        joined_elements: 12,
        stock_label_nodes: 1,
        stock_image_caption_nodes: 0,
        name_matches_on_stock: 2,
        oracle_non_empty_names: 10,
        description_matches_on_stock: 12,
        oracle_non_empty_descriptions: 0,
        error_message_matches_on_stock: 12,
        oracle_non_empty_errors: 0,
    },
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
    let mut reader = serde_json::Deserializer::from_str(&text);
    reader.disable_recursion_limit();
    let oracle =
        Oracle::deserialize(&mut reader).map_err(|err| format!("{}: {err}", path.display()))?;
    reader.end()?;
    Ok(oracle)
}

#[derive(Debug)]
struct FixtureNameComparison {
    recorded_version: Option<u32>,
    oracle_frames: usize,
    joined_frames: usize,
    partial_frames: usize,
    counts: StockFixtureCounts,
    spurious_stock_names: usize,
    spurious_stock_descriptions: usize,
    spurious_stock_errors: usize,
    empty_oracle_false_positives: Vec<String>,
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

fn is_contract_gap_g6(fixture: &str, element: &OracleElement) -> bool {
    fixture == "names" && element.error_message == CONTRACT_GAP_G6_ERROR_MESSAGE
}

fn count_stock_label_and_caption_nodes(apc: &AnnotatedPageContent) -> (usize, usize) {
    let mut stock_label_nodes = 0;
    let mut stock_image_caption_nodes = 0;
    let mut stack: Vec<&ContentNode> = apc.root_node.iter().collect();

    while let Some(node) = stack.pop() {
        if let Some(raw) = node.content_attributes.as_ref() {
            if raw.label.as_deref().is_some_and(|label| !label.is_empty()) {
                stock_label_nodes += 1;
            }
            if matches!(
                raw.content_data.as_ref(),
                Some(ContentData::Image(info))
                    if info
                        .image_caption
                        .as_deref()
                        .is_some_and(|caption| !caption.is_empty())
            ) {
                stock_image_caption_nodes += 1;
            }
        }
        stack.extend(node.children_nodes.iter());
    }

    (stock_label_nodes, stock_image_caption_nodes)
}

fn compare_joined_element(
    fixture: &str,
    frame: &OracleFrame,
    element: &OracleElement,
    node: &TreeNode,
    cache: &NameCache<'_>,
    recorded_version: Option<u32>,
    stats: &mut FixtureNameComparison,
) {
    stats.counts.joined_elements += 1;

    let actual_name = cache.name(node.id(), NameVisibility::VisibleOnly).text();
    let actual_description = cache
        .description(node.id(), NameVisibility::VisibleOnly)
        .text();
    let actual_error = cache.error_message(node.id());

    if !element.name.is_empty() {
        stats.counts.oracle_non_empty_names += 1;
    }
    if !element.description.is_empty() {
        stats.counts.oracle_non_empty_descriptions += 1;
    }
    if !element.error_message.is_empty() {
        stats.counts.oracle_non_empty_errors += 1;
    }

    if recorded_version.is_none() {
        if actual_name == element.name {
            stats.counts.name_matches_on_stock += 1;
        } else if element.name.is_empty() {
            stats.empty_oracle_false_positives.push(format!(
                "{fixture} [{}] node {}: stock name expected empty, got {:?}",
                frame.url, element.backend_node_id, actual_name
            ));
        }

        if actual_description == element.description {
            stats.counts.description_matches_on_stock += 1;
        } else if element.description.is_empty() {
            stats.empty_oracle_false_positives.push(format!(
                "{fixture} [{}] node {}: stock description expected empty, got {:?}",
                frame.url, element.backend_node_id, actual_description
            ));
        }

        if actual_error == element.error_message {
            stats.counts.error_message_matches_on_stock += 1;
        } else if element.error_message.is_empty() {
            stats.empty_oracle_false_positives.push(format!(
                "{fixture} [{}] node {}: stock error_message expected empty, got {:?}",
                frame.url, element.backend_node_id, actual_error
            ));
        }
    }

    if recorded_version.unwrap_or(0) >= CONTRACT_NAMES_VERSION {
        if actual_name != element.name {
            stats.contract_mismatches.push(format!(
                "{fixture} [{}] node {}: name tree={:?} oracle={:?}",
                frame.url, element.backend_node_id, actual_name, element.name
            ));
        }
        if actual_description != element.description {
            stats.contract_mismatches.push(format!(
                "{fixture} [{}] node {}: description tree={:?} oracle={:?}",
                frame.url, element.backend_node_id, actual_description, element.description
            ));
        }
        if actual_error != element.error_message
            && !(is_contract_gap_g6(fixture, element) && actual_error.is_empty())
        {
            stats.contract_mismatches.push(format!(
                "{fixture} [{}] node {}: error_message tree={:?} oracle={:?}",
                frame.url, element.backend_node_id, actual_error, element.error_message
            ));
        }
    }
}

fn compare_fixture(name: &'static str) -> Result<FixtureNameComparison, Box<dyn Error>> {
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

    let (stock_label_nodes, stock_image_caption_nodes) = count_stock_label_and_caption_nodes(&apc);
    let tree = SnapshotTree::build(apc, BuildOptions::default())?;
    let cache = tree.name_cache();
    let MappedOracleFrames {
        frame_to_doc,
        partial_frames,
    } = map_oracle_frames(&tree, &oracle)?;

    let mut stats = FixtureNameComparison {
        recorded_version,
        oracle_frames: oracle.frames.len(),
        joined_frames: frame_to_doc.len(),
        partial_frames,
        counts: StockFixtureCounts {
            fixture: name,
            stock_label_nodes,
            stock_image_caption_nodes,
            ..StockFixtureCounts::default()
        },
        spurious_stock_names: 0,
        spurious_stock_descriptions: 0,
        spurious_stock_errors: 0,
        empty_oracle_false_positives: Vec::new(),
        contract_mismatches: Vec::new(),
    };

    if recorded_version.is_none() {
        for node in tree.nodes() {
            if !cache
                .name(node.id(), NameVisibility::VisibleOnly)
                .is_empty()
                || !cache
                    .name(node.id(), NameVisibility::IncludeHidden)
                    .is_empty()
            {
                stats.spurious_stock_names += 1;
            }
            if !cache
                .description(node.id(), NameVisibility::VisibleOnly)
                .is_empty()
                || !cache
                    .description(node.id(), NameVisibility::IncludeHidden)
                    .is_empty()
            {
                stats.spurious_stock_descriptions += 1;
            }
            if !cache.error_message(node.id()).is_empty() {
                stats.spurious_stock_errors += 1;
            }
        }
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
                compare_joined_element(
                    name,
                    frame,
                    element,
                    node,
                    &cache,
                    recorded_version,
                    &mut stats,
                );
            }
        }
    }

    Ok(stats)
}

#[test]
fn recorded_fixtures_agree_with_playwright_oracles_on_all_expressible_name_facts()
-> Result<(), Box<dyn Error>> {
    let mut total_joined = 0;
    let mut total_stock_label_nodes = 0;
    let mut total_stock_image_caption_nodes = 0;
    let mut measured_stock_counts = Vec::new();

    for fixture in FIXTURES {
        let stats = compare_fixture(fixture.name)?;
        total_joined += stats.counts.joined_elements;

        assert_eq!(
            stats.joined_frames + stats.partial_frames,
            stats.oracle_frames,
            "{}: every oracle frame must join a TreeDoc or a partial iframe",
            fixture.name
        );
        assert!(
            stats.counts.joined_elements > 0,
            "{}: expected joined elements between .pb and oracle",
            fixture.name
        );

        if stats.recorded_version.is_none() {
            total_stock_label_nodes += stats.counts.stock_label_nodes;
            total_stock_image_caption_nodes += stats.counts.stock_image_caption_nodes;
            assert_eq!(
                stats.spurious_stock_names, 0,
                "{}: contract §9 forbids deriving any accessible name from stock Chromium APC fields",
                fixture.name
            );
            assert_eq!(
                stats.spurious_stock_descriptions, 0,
                "{}: contract §9 forbids deriving any accessible description from stock Chromium APC fields",
                fixture.name
            );
            assert_eq!(
                stats.spurious_stock_errors, 0,
                "{}: contract §9 forbids deriving any error message from stock Chromium APC fields",
                fixture.name
            );
            assert!(
                stats.empty_oracle_false_positives.is_empty(),
                "{}: stock recording produced non-empty name/description/error for empty oracle entries: {:?}",
                fixture.name,
                stats.empty_oracle_false_positives
            );
            assert_eq!(
                stats.counts.name_matches_on_stock + stats.counts.oracle_non_empty_names,
                stats.counts.joined_elements,
                "{}: stock name matches must cover every empty oracle name with zero false positives",
                fixture.name
            );
            assert_eq!(
                stats.counts.description_matches_on_stock
                    + stats.counts.oracle_non_empty_descriptions,
                stats.counts.joined_elements,
                "{}: stock description matches must cover every empty oracle description with zero false positives",
                fixture.name
            );
            assert_eq!(
                stats.counts.error_message_matches_on_stock + stats.counts.oracle_non_empty_errors,
                stats.counts.joined_elements,
                "{}: stock error_message matches must cover every empty oracle error_message with zero false positives",
                fixture.name
            );
            measured_stock_counts.push(stats.counts);
        }

        assert!(
            stats.contract_mismatches.is_empty(),
            "{}: contract v4+ name/description/error mismatches: {:?}",
            fixture.name,
            stats.contract_mismatches
        );
    }

    assert!(
        total_joined >= 640,
        "expected >= 640 joined oracle elements across all 16 fixtures, saw {total_joined}"
    );
    if !measured_stock_counts.is_empty() {
        assert!(
            total_stock_label_nodes > 0,
            "stock Chromium .pb recordings must carry non-empty ContentAttributes::label values so the contract §9 refusal check is non-vacuous"
        );
        assert!(
            total_stock_image_caption_nodes > 0,
            "stock Chromium .pb recordings must carry non-empty ImageInfo::image_caption values so the contract §9 refusal check is non-vacuous"
        );
        let expected_for_stock: Vec<StockFixtureCounts> = measured_stock_counts
            .iter()
            .filter_map(|measured| {
                EXPECTED_STOCK_COUNTS
                    .iter()
                    .find(|expected| expected.fixture == measured.fixture)
                    .copied()
            })
            .collect();
        assert_eq!(
            measured_stock_counts, expected_for_stock,
            "pinned per-page stock counts across recorded stock fixtures must stay exact and auditable"
        );
    }

    Ok(())
}

fn ref_node(dom_id: i32) -> BrowserOsRef {
    BrowserOsRef {
        dom_node_id: Some(dom_id),
        unrendered_text: None,
    }
}

fn ref_unrendered(text: &str) -> BrowserOsRef {
    BrowserOsRef {
        dom_node_id: None,
        unrendered_text: Some(text.to_owned()),
    }
}

fn button(dom_id: i32, text: &str) -> NodeBuilder {
    NodeBuilder::form_control(FormControlType::ButtonButton)
        .tag("button")
        .dom_id(dom_id)
        .child(NodeBuilder::text(text))
}

fn label_for(target_dom_id: i32, text: &str) -> NodeBuilder {
    NodeBuilder::element("label")
        .label_for(target_dom_id)
        .child(NodeBuilder::text(text))
}

fn name_prohibited_nodes() -> Vec<NodeBuilder> {
    vec![
        NodeBuilder::element("p")
            .dom_id(24)
            .aria_label("leak: p aria-label")
            .child(NodeBuilder::text(
                "paragraph: p with aria-label, name prohibited",
            )),
        NodeBuilder::container().child(
            NodeBuilder::container()
                .aria_label("leak: span aria-label")
                .child(NodeBuilder::text(
                    "generic: span with aria-label, name prohibited",
                )),
        ),
        NodeBuilder::container().child(
            NodeBuilder::element("code")
                .dom_id(265)
                .aria_label("leak: code aria-label")
                .child(NodeBuilder::text(
                    "code: code with aria-label, name prohibited",
                )),
        ),
        NodeBuilder::container().child(
            NodeBuilder::element("strong")
                .dom_id(267)
                .aria_label("leak: strong aria-label")
                .child(NodeBuilder::text(
                    "strong: strong with aria-label, name prohibited",
                )),
        ),
        NodeBuilder::container().child(
            NodeBuilder::element("em")
                .dom_id(269)
                .aria_label("leak: em aria-label")
                .child(NodeBuilder::text(
                    "emphasis: em with aria-label, name prohibited",
                )),
        ),
        NodeBuilder::container()
            .aria_label("leak: div aria-label")
            .child(NodeBuilder::text(
                "generic: div with aria-label, name prohibited",
            )),
    ]
}

fn labelledby_nodes() -> Vec<NodeBuilder> {
    vec![
        NodeBuilder::container()
            .child(
                NodeBuilder::container()
                    .dom_id(1001)
                    .child(NodeBuilder::text("labelledby: first id")),
            )
            .child(
                NodeBuilder::container()
                    .dom_id(1002)
                    .child(NodeBuilder::text("second id")),
            )
            .child(
                button(76, "content loses to labelledby with several ids")
                    .labelledby(vec![ref_node(1001), ref_node(1002)]),
            ),
        NodeBuilder::container()
            .child(
                button(78, "labelledby: self reference")
                    .labelledby(vec![ref_node(78), ref_node(1003)]),
            )
            .child(
                NodeBuilder::container()
                    .dom_id(1003)
                    .child(NodeBuilder::text("plus a second id")),
            ),
        NodeBuilder::container().child(
            button(82, "content loses to a hidden referent").labelledby(vec![ref_unrendered(
                "labelledby: display-none referent still counts",
            )]),
        ),
        NodeBuilder::container()
            .child(
                NodeBuilder::container()
                    .dom_id(1005)
                    .aria_label("labelledby: referent's own aria-label wins")
                    .child(NodeBuilder::text("referent content loses")),
            )
            .child(
                button(86, "content loses to a referent with aria-label")
                    .labelledby(vec![ref_node(1005)]),
            ),
        NodeBuilder::container()
            .child(
                NodeBuilder::container()
                    .dom_id(1006)
                    .labelledby(vec![ref_node(1007)])
                    .child(NodeBuilder::text(
                        "labelledby: referent content, its own labelledby not followed",
                    )),
            )
            .child(
                NodeBuilder::container()
                    .dom_id(1007)
                    .child(NodeBuilder::text("leak: second hop of a labelledby chain")),
            )
            .child(
                button(92, "content loses to a referent with labelledby")
                    .labelledby(vec![ref_node(1006)]),
            ),
        NodeBuilder::container().child(
            button(94, "labelledby: every id missing, falls to content")
                .labelledby(vec![ref_node(9_999)]),
        ),
        NodeBuilder::container()
            .child(
                NodeBuilder::container()
                    .dom_id(1008)
                    .child(NodeBuilder::text("labelledby: private box")),
            )
            .child(label_for(
                4,
                "textbox: password box inside a labelledby chain",
            ))
            .child(
                NodeBuilder::form_control(FormControlType::InputPassword)
                    .tag("input")
                    .dom_id(4)
                    .redaction(RedactionDecision::RedactedHasBeenPassword)
                    .redacted_value_length(25),
            )
            .child(
                button(101, "content loses to labelledby with a private box")
                    .labelledby(vec![ref_node(1008), ref_node(4)]),
            ),
    ]
}

fn embedded_and_label_nodes() -> Vec<NodeBuilder> {
    vec![
        NodeBuilder::container()
            .child(
                NodeBuilder::form_control(FormControlType::InputCheckbox)
                    .tag("input")
                    .dom_id(18),
            )
            .child(
                NodeBuilder::element("label")
                    .label_for(18)
                    .child(NodeBuilder::text("embedded: send "))
                    .child(
                        NodeBuilder::form_control(FormControlType::InputText)
                            .tag("input")
                            .dom_id(5)
                            .field_value("three")
                            .aria_label("textbox: embedded in a label, value three"),
                    )
                    .child(NodeBuilder::text(" copies in "))
                    .child(
                        NodeBuilder::form_control(FormControlType::SelectOne)
                            .tag("select")
                            .dom_id(6)
                            .aria_label("combobox: embedded in a label")
                            .child(
                                NodeBuilder::element("option")
                                    .dom_id(307)
                                    .selected(false)
                                    .child(NodeBuilder::text("red")),
                            )
                            .child(
                                NodeBuilder::element("option")
                                    .dom_id(309)
                                    .selected(true)
                                    .child(NodeBuilder::text("blue")),
                            ),
                    )
                    .child(NodeBuilder::text(" ink")),
            ),
        NodeBuilder::container()
            .child(button(111, "aria-label blank: falls to content").aria_label("   "))
            .child(label_for(7, "aria-label blank: falls to label"))
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(7)
                    .aria_label("   "),
            ),
        NodeBuilder::container()
            .child(label_for(8, "native: label by for"))
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(8),
            ),
        NodeBuilder::container().child(
            NodeBuilder::element("label")
                .label_for(9)
                .child(NodeBuilder::text("native: wrapping label "))
                .child(
                    NodeBuilder::form_control(FormControlType::InputText)
                        .tag("input")
                        .dom_id(9),
                ),
        ),
        NodeBuilder::container()
            .child(label_for(10, "native: first of two labels"))
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(10),
            )
            .child(label_for(10, "second of two labels")),
    ]
}

fn native_button_and_image_input_nodes() -> Vec<NodeBuilder> {
    vec![
        NodeBuilder::container()
            .child(
                NodeBuilder::form_control(FormControlType::InputButton)
                    .tag("input")
                    .dom_id(126)
                    .field_value("native: input type=button by value"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputButton)
                    .tag("input")
                    .dom_id(129)
                    .title("native: input type=button without value falls to title"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputSubmit)
                    .tag("input")
                    .dom_id(131)
                    .field_value("native: input type=submit by value"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputSubmit)
                    .tag("input")
                    .dom_id(133),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputReset)
                    .tag("input")
                    .dom_id(136)
                    .field_value("native: input type=reset by value"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputReset)
                    .tag("input")
                    .dom_id(139),
            ),
        NodeBuilder::container()
            .child(label_for(19, "native: file input by label"))
            .child(
                NodeBuilder::form_control(FormControlType::InputFile)
                    .tag("input")
                    .dom_id(19),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputFile)
                    .tag("input")
                    .dom_id(146),
            ),
        NodeBuilder::container()
            .child(label_for(20, "native: image input by label"))
            .child(
                NodeBuilder::form_control(FormControlType::InputImage)
                    .tag("input")
                    .dom_id(20)
                    .alt("unused: alt loses to label"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputImage)
                    .tag("input")
                    .dom_id(153)
                    .alt("native: image input by alt")
                    .title("image input title becomes the description, alt wins the name"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputImage)
                    .tag("input")
                    .dom_id(155)
                    .title("native: image input by title"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputImage)
                    .tag("input")
                    .dom_id(157),
            ),
    ]
}

fn native_text_and_select_input_nodes() -> Vec<NodeBuilder> {
    vec![
        NodeBuilder::container()
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(11)
                    .title("native: text input by title")
                    .placeholder("placeholder: loses the name to title"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(12)
                    .placeholder("native: text input by placeholder"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::TextArea)
                    .tag("textarea")
                    .dom_id(13)
                    .placeholder("native: textarea by placeholder"),
            )
            .child(
                NodeBuilder::form_control(FormControlType::SelectOne)
                    .tag("select")
                    .dom_id(14)
                    .title("native: select by title")
                    .child(
                        NodeBuilder::element("option")
                            .dom_id(340)
                            .selected(true)
                            .child(NodeBuilder::text("option: in a select named by title")),
                    ),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputCheckbox)
                    .tag("input")
                    .dom_id(172)
                    .title("native: checkbox by title"),
            ),
    ]
}

fn native_media_and_container_nodes() -> Vec<NodeBuilder> {
    vec![
        NodeBuilder::container()
            .child(
                NodeBuilder::element("img")
                    .dom_id(173)
                    .alt("native: img by alt")
                    .title("img title becomes the description, alt wins the name"),
            )
            .child(
                NodeBuilder::element("img")
                    .dom_id(175)
                    .title("native: img by title"),
            )
            .child(
                NodeBuilder::element("img")
                    .dom_id(177)
                    .alt("native: img carrying an image map"),
            )
            .child(
                NodeBuilder::element("map")
                    .child(
                        NodeBuilder::element("area")
                            .dom_id(348)
                            .href("#area-alt")
                            .alt("native: area by alt")
                            .title("area title becomes the description, alt wins the name")
                            .visibility_hidden(true),
                    )
                    .child(
                        NodeBuilder::element("area")
                            .dom_id(350)
                            .href("#area-title")
                            .title("native: area by title")
                            .visibility_hidden(true),
                    ),
            ),
        NodeBuilder::element("fieldset")
            .dom_id(47)
            .child(
                NodeBuilder::element("legend").child(NodeBuilder::text("native: fieldset legend")),
            )
            .child(
                NodeBuilder::element("p")
                    .dom_id(49)
                    .child(NodeBuilder::text(
                        "paragraph: inside a fieldset named by legend",
                    )),
            ),
        NodeBuilder::element("figure")
            .dom_id(50)
            .child(
                NodeBuilder::element("figcaption")
                    .child(NodeBuilder::text("native: figure figcaption")),
            )
            .child(
                NodeBuilder::element("p")
                    .dom_id(52)
                    .child(NodeBuilder::text(
                        "paragraph: inside a figure named by figcaption",
                    )),
            ),
        NodeBuilder::element("table")
            .dom_id(53)
            .child(
                NodeBuilder::element("caption")
                    .dom_id(54)
                    .child(NodeBuilder::text("native: table caption")),
            )
            .child(
                NodeBuilder::element("tbody").dom_id(356).child(
                    NodeBuilder::element("tr").dom_id(357).child(
                        NodeBuilder::element("td")
                            .dom_id(55)
                            .child(NodeBuilder::text("cell: in a table named by caption")),
                    ),
                ),
            ),
    ]
}

fn content_button_nodes() -> Vec<NodeBuilder> {
    vec![
        NodeBuilder::container()
            .child(
                NodeBuilder::form_control(FormControlType::ButtonButton)
                    .tag("button")
                    .dom_id(184)
                    .child(NodeBuilder::text("content: block"))
                    .child(
                        NodeBuilder::container()
                            .break_before(true)
                            .child(NodeBuilder::text("div")),
                    )
                    .child(NodeBuilder::text("boundary").break_before(true)),
            )
            .child(
                NodeBuilder::form_control(FormControlType::ButtonButton)
                    .tag("button")
                    .dom_id(190)
                    .child(NodeBuilder::text("content: inline"))
                    .child(NodeBuilder::container().child(NodeBuilder::text("span")))
                    .child(NodeBuilder::text("join")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::ButtonButton)
                    .tag("button")
                    .dom_id(194)
                    .child(NodeBuilder::text("[before] ").without_dom_id())
                    .child(NodeBuilder::text("content: css pseudo"))
                    .child(NodeBuilder::text(" [after attr]").without_dom_id()),
            )
            .child(
                NodeBuilder::form_control(FormControlType::ButtonButton)
                    .tag("button")
                    .dom_id(197)
                    .child(NodeBuilder::text("content: block pseudo"))
                    .child(
                        NodeBuilder::text("block-after")
                            .without_dom_id()
                            .break_before(true),
                    ),
            )
            .child(
                NodeBuilder::form_control(FormControlType::ButtonButton)
                    .tag("button")
                    .dom_id(200)
                    .child(NodeBuilder::text("content: icon "))
                    .child(
                        NodeBuilder::element("img")
                            .dom_id(202)
                            .alt("nested img alt"),
                    ),
            )
            .child(
                NodeBuilder::form_control(FormControlType::ButtonButton)
                    .tag("button")
                    .dom_id(204)
                    .child(NodeBuilder::text("content: visible"))
                    .child(
                        NodeBuilder::container()
                            .aria_hidden(true)
                            .child(NodeBuilder::text(" aria-hidden")),
                    )
                    .child(
                        NodeBuilder::container()
                            .visibility_hidden(true)
                            .child(NodeBuilder::text(" visibility-hidden")),
                    )
                    .child(NodeBuilder::text(" end")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::ButtonButton)
                    .tag("button")
                    .dom_id(208)
                    .child(
                        NodeBuilder::text("CONTENT: TEXT-TRANSFORM UPPERCASE SOURCE")
                            .dom_text("content: text-transform uppercase source"),
                    ),
            )
            .child(button(
                210,
                "content: whitespace\u{00A0}nbsp zero\u{200B}width soft\u{00AD}hyphen\n    newline\ttab",
            )),
    ]
}

fn link_group_and_title_nodes() -> Vec<NodeBuilder> {
    vec![
        NodeBuilder::anchor("#heading-link")
            .tag("a")
            .href("#heading-link")
            .dom_id(374)
            .child(
                NodeBuilder::element("h3")
                    .dom_id(57)
                    .child(NodeBuilder::text("content: heading inside link")),
            ),
        NodeBuilder::container()
            .role("group")
            .dom_id(58)
            .child(NodeBuilder::text(
                "unused: group does not take its name from content",
            )),
        NodeBuilder::container()
            .child(
                NodeBuilder::form_control(FormControlType::ButtonButton)
                    .tag("button")
                    .dom_id(214)
                    .title("title: empty button with only a title"),
            )
            .child(
                NodeBuilder::container()
                    .role("button")
                    .dom_id(216)
                    .title("title: div role=button with only a title"),
            ),
    ]
}

fn description_and_error_nodes() -> Vec<NodeBuilder> {
    vec![
        NodeBuilder::container()
            .child(
                button(217, "description: describedby two ids")
                    .describedby(vec![ref_node(1009), ref_node(1010)]),
            )
            .child(
                NodeBuilder::container()
                    .dom_id(1009)
                    .child(NodeBuilder::text("first description part")),
            )
            .child(
                NodeBuilder::container()
                    .dom_id(1010)
                    .child(NodeBuilder::text("second description part")),
            ),
        NodeBuilder::container().child(
            button(223, "description: describedby a hidden referent")
                .describedby(vec![ref_unrendered("hidden description text still counts")]),
        ),
        NodeBuilder::container()
            .child(
                button(225, "description: aria-description")
                    .aria_description("aria-description text"),
            )
            .child(
                button(228, "description: describedby beats aria-description")
                    .describedby(vec![ref_node(1009)])
                    .aria_description("unused: aria-description loses to describedby"),
            )
            .child(
                button(230, "description: title on a button named by content")
                    .title("title text becomes the description"),
            ),
        NodeBuilder::container()
            .child(label_for(15, "error: aria-invalid=true with errormessage"))
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(15)
                    .aria_invalid("true")
                    .errormessage(vec![ref_node(1012)]),
            )
            .child(
                NodeBuilder::container()
                    .dom_id(1012)
                    .child(NodeBuilder::text(
                        "error message: shown because aria-invalid",
                    )),
            ),
        NodeBuilder::container()
            .child(label_for(16, "error: errormessage without aria-invalid"))
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(16)
                    .errormessage(vec![ref_node(1013)]),
            )
            .child(
                NodeBuilder::container()
                    .dom_id(1013)
                    .child(NodeBuilder::text("error message: unused, field is valid")),
            ),
        NodeBuilder::container()
            .child(label_for(
                17,
                "error: empty required field with errormessage, no aria-invalid",
            ))
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .dom_id(17)
                    .required(true)
                    .errormessage(vec![ref_node(1014)]),
            )
            .child(
                NodeBuilder::container()
                    .dom_id(1014)
                    .child(NodeBuilder::text(CONTRACT_GAP_G6_ERROR_MESSAGE)),
            ),
    ]
}

fn build_contract_v4_names_page() -> AnnotatedPageContent {
    let mut body_children = Vec::new();
    body_children.extend(name_prohibited_nodes());
    body_children.extend(labelledby_nodes());
    body_children.extend(embedded_and_label_nodes());
    body_children.extend(native_button_and_image_input_nodes());
    body_children.extend(native_text_and_select_input_nodes());
    body_children.extend(native_media_and_container_nodes());
    body_children.extend(content_button_nodes());
    body_children.extend(link_group_and_title_nodes());
    body_children.extend(description_and_error_nodes());

    ApcBuilder::new("doc-names-v4")
        .contract_version(CONTRACT_NAMES_VERSION)
        .main_url("http://127.0.0.1/edge/names.html")
        .main_title("Edge: names")
        .root(
            NodeBuilder::root()
                .tag("html")
                .dom_id(22)
                .children(body_children),
        )
        .build()
}

fn assert_names_oracle_element(
    node: &TreeNode,
    cache: &NameCache<'_>,
    element: &OracleElement,
) -> Result<bool, Box<dyn Error>> {
    let expected_role = Role::parse(&element.role).ok_or_else(|| {
        format!(
            "oracle role {:?} on backendNodeId {} must parse into Role",
            element.role, element.backend_node_id
        )
    })?;
    assert_eq!(
        node.role(),
        Some(expected_role),
        "backendNodeId {}: computed role must match names.oracle.json",
        element.backend_node_id
    );
    assert_eq!(
        node.hidden().is_hidden_for_aria(),
        element.hidden,
        "backendNodeId {}: computed hidden flag must match names.oracle.json",
        element.backend_node_id
    );
    assert_eq!(
        cache.name(node.id(), NameVisibility::VisibleOnly).text(),
        element.name,
        "backendNodeId {}: accessible name must match names.oracle.json",
        element.backend_node_id
    );
    assert_eq!(
        cache
            .description(node.id(), NameVisibility::VisibleOnly)
            .text(),
        element.description,
        "backendNodeId {}: accessible description must match names.oracle.json",
        element.backend_node_id
    );

    let actual_error = cache.error_message(node.id());
    if element.backend_node_id == ERR_REQUIRED_BACKEND_NODE_ID {
        assert_eq!(
            element.error_message, CONTRACT_GAP_G6_ERROR_MESSAGE,
            "backendNodeId {ERR_REQUIRED_BACKEND_NODE_ID} must remain the err-required fixture entry in names.oracle.json"
        );
        assert_eq!(
            actual_error, "",
            "contract gap G6: wire contract omits validity.valid, so required without aria-invalid must return empty error_message"
        );
        return Ok(true);
    }

    assert_eq!(
        actual_error, element.error_message,
        "backendNodeId {}: error_message must match names.oracle.json",
        element.backend_node_id
    );
    Ok(false)
}

#[test]
fn contract_v4_names_page_matches_recorded_playwright_oracle() -> Result<(), Box<dyn Error>> {
    let oracle = load_oracle("names")?;
    let main_frame = oracle
        .frames
        .first()
        .ok_or("names.oracle.json must contain a main frame")?;
    assert_eq!(
        oracle.frames.len(),
        1,
        "names.html is a single-frame fixture"
    );

    let tree = SnapshotTree::build(build_contract_v4_names_page(), BuildOptions::default())?;
    let cache = tree.name_cache();
    let main_doc = tree
        .document(DocId::MAIN)
        .ok_or("SnapshotTree must contain DocId::MAIN")?;

    let mut verified_elements: usize = 0;
    let mut gap_g6_elements: usize = 0;

    for element in &main_frame.elements {
        let dom_id = DomNodeId::new(i32::try_from(element.backend_node_id)?);
        let key = NodeKey::new(main_doc.token().clone(), dom_id);
        let node = tree.node_by_key(&key).ok_or_else(|| {
            format!(
                "names.html contract-v4 mirror missing oracle backendNodeId {} (role={:?}, name={:?})",
                element.backend_node_id, element.role, element.name
            )
        })?;
        if assert_names_oracle_element(node, &cache, element)? {
            gap_g6_elements += 1;
        }
        verified_elements += 1;
    }

    assert_eq!(
        verified_elements, 77,
        "contract-v4 mirror must verify every single one of the 77 elements recorded in names.oracle.json"
    );
    assert_eq!(
        gap_g6_elements, 1,
        "contract gap G6 must be hit and asserted once on backendNodeId {ERR_REQUIRED_BACKEND_NODE_ID}"
    );

    let area_alt = tree
        .node_by_key(&NodeKey::new(main_doc.token().clone(), DomNodeId::new(348)))
        .ok_or("area-alt node 348 must exist in contract-v4 mirror")?;
    let area_title = tree
        .node_by_key(&NodeKey::new(main_doc.token().clone(), DomNodeId::new(350)))
        .ok_or("area-title node 350 must exist in contract-v4 mirror")?;
    assert_eq!(
        cache
            .name(area_alt.id(), NameVisibility::IncludeHidden)
            .text(),
        "native: area by alt",
        "hidden <area> with alt must expose its alt text under NameVisibility::IncludeHidden"
    );
    assert_eq!(
        cache
            .name(area_title.id(), NameVisibility::IncludeHidden)
            .text(),
        "native: area by title",
        "hidden <area> without alt must fall back to title under NameVisibility::IncludeHidden"
    );

    Ok(())
}
