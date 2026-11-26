//! Offline tests for [`neo_snapshot_apc::tree`]: document identity, iterative frame stitching,
//! containment semantics, redacting `Debug`, the contract-v6 Chromium 158 fixtures, and
//! stack-safety stress tests.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::{fs, thread};

use neo_snapshot_apc::proto::{
    AnnotatedPageContent, AnnotatedRole, AxRole, BrowserOsFrameData, BrowserOsPageContentData,
    BrowserOsRef, BrowserOsTriState, ClickabilityReason, FormControlType,
    InteractionDisabledReason, RedactionDecision, SelectOption,
};
use neo_snapshot_apc::tree::{
    ApcBuilder, BuildError, BuildOptions, DocId, DocOrigin, DocSeq, DocToken, DomNodeId,
    FrameBuilder, FrameToken, MIN_CONTRACT_VERSION, NodeBuilder, NodeId, NodeKey, NodeRange,
    PartialFrameReason, PartialIframeSpec, SnapshotTree, TreeDoc, TreeNode, Viewport,
};
use prost::Message as _;

fn decode_fixture(name: &str) -> Result<AnnotatedPageContent, Box<dyn Error>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(format!("{name}.pb"));
    let bytes = fs::read(path)?;
    Ok(neo_snapshot_apc::decode_protobuf(&bytes)?)
}

/// Builds a recorded fixture as captured: every `.pb` carries contract v6 (`tests/fixtures/README.md`).
fn load_fixture_tree(name: &str) -> Result<SnapshotTree, Box<dyn Error>> {
    Ok(SnapshotTree::build(
        decode_fixture(name)?,
        BuildOptions::default(),
    )?)
}

/// The CDP frame id of `hung-subframe`'s `[::1]` frame on its recorded load.
const HUNG_FRAME_TOKEN: &str = "A5748446FECE0320E1A027D7563F2CA7";

fn require_doc_token(raw: &str) -> Result<DocToken, Box<dyn Error>> {
    DocToken::new(raw).ok_or_else(|| format!("expected non-empty DocToken for {raw:?}").into())
}

#[test]
fn node_key_types_validate_tokens_ranges_and_serde() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        DocToken::new(""),
        None,
        "DocToken must reject empty strings"
    );
    assert_eq!(
        FrameToken::new(""),
        None,
        "FrameToken must reject empty strings"
    );

    let doc = DocToken::new("doc-alpha").ok_or("expected valid DocToken")?;
    assert_eq!(doc.as_str(), "doc-alpha");
    assert_eq!(doc.to_string(), "doc-alpha");

    let frame = FrameToken::new("frame-7").ok_or("expected valid FrameToken")?;
    assert_eq!(frame.as_str(), "frame-7");
    assert_eq!(frame.to_string(), "frame-7");

    let key = NodeKey::new(doc.clone(), DomNodeId::new(42));
    assert_eq!(key.node.raw(), 42);
    assert_eq!(key.node.as_backend_node_id(), 42_i64);
    assert_eq!(key.node.to_string(), "42");

    let encoded_key = serde_json::to_string(&key)?;
    let decoded_key: NodeKey = serde_json::from_str(&encoded_key)?;
    assert_eq!(
        decoded_key, key,
        "NodeKey must round-trip cleanly through JSON"
    );

    let empty_doc: Result<DocToken, _> = serde_json::from_str(r#""""#);
    assert!(
        empty_doc.is_err(),
        "DocToken deserialization must reject empty strings"
    );
    let empty_frame: Result<FrameToken, _> = serde_json::from_str(r#""""#);
    assert!(
        empty_frame.is_err(),
        "FrameToken deserialization must reject empty strings"
    );

    let seq = DocSeq::new(9);
    assert_eq!(seq.raw(), 9);
    assert_eq!(seq.to_string(), "9");

    assert_eq!(
        NodeRange::new(NodeId::new(5), NodeId::new(3)),
        None,
        "NodeRange must reject inverted start > end"
    );
    let empty = NodeRange::empty_at(NodeId::new(4));
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert!(!empty.contains(NodeId::new(4)));

    let range = NodeRange::new(NodeId::new(2), NodeId::new(5)).ok_or("expected valid NodeRange")?;
    assert_eq!(range.start(), NodeId::new(2));
    assert_eq!(range.end(), NodeId::new(5));
    assert_eq!(range.len(), 3);
    assert!(!range.is_empty());
    assert!(!range.contains(NodeId::new(1)));
    assert!(range.contains(NodeId::new(2)));
    assert!(range.contains(NodeId::new(4)));
    assert!(!range.contains(NodeId::new(5)));
    assert_eq!(range.as_usize_range(), 2..5);
    Ok(())
}

fn assert_main_doc_metadata(tree: &SnapshotTree) -> Result<(), Box<dyn Error>> {
    assert_eq!(tree.nodes().len(), 8);
    assert_eq!(tree.documents().len(), 1);
    assert!(tree.partial_frames().is_empty());
    assert_eq!(tree.contract_version(), MIN_CONTRACT_VERSION);
    assert_eq!(tree.subframe_timeout_ms(), Some(2_000));
    assert_eq!(
        tree.viewport(),
        Some(Viewport {
            x: 0,
            y: 10,
            width: 1280,
            height: 720,
        }),
        "viewport geometry must be normalized onto SnapshotTree"
    );

    let main_doc = tree.main_document().ok_or("missing main document")?;
    assert!(main_doc.is_main());
    assert_eq!(main_doc.id(), DocId::MAIN);
    assert_eq!(main_doc.token().as_str(), "doc-main");
    assert_eq!(main_doc.parent_doc(), None);
    assert_eq!(main_doc.iframe_node(), None);
    assert_eq!(main_doc.iframe_key(), None);
    assert_eq!(main_doc.root_node(), Some(NodeId::new(0)));
    assert_eq!(main_doc.node_count(), 8);
    assert_eq!(main_doc.node_range().as_usize_range(), 0..8);
    assert_eq!(main_doc.url(), Some("https://example.test/page"));
    assert_eq!(main_doc.title(), Some("Main Page"));
    assert_eq!(
        main_doc.origin(),
        Some(&DocOrigin {
            opaque: false,
            value: "https://example.test".to_owned(),
        })
    );
    assert_eq!(main_doc.focused_node_id(), Some(DomNodeId::new(5)));
    assert_eq!(
        tree.document_by_token(main_doc.token()).map(TreeDoc::id),
        Some(DocId::MAIN)
    );
    Ok(())
}

#[test]
fn single_doc_preorder_links_ancestors_subtree_and_containment() -> Result<(), Box<dyn Error>> {
    const ROOT: u32 = 0;
    const HEADING: u32 = 1;
    const CONTAINER: u32 = 3;
    const SAVE_BUTTON: u32 = 4;
    const SAVE_TEXT: u32 = 5;
    const PARAGRAPH: u32 = 6;
    const FOOTER_TEXT: u32 = 7;

    let apc = ApcBuilder::new("doc-main")
        .main_url("https://example.test/page")
        .main_title("Main Page")
        .main_origin("https://example.test", false)
        .main_focused_node_id(5)
        .viewport(0, 10, 1280, 720)
        .root(
            NodeBuilder::root()
                .child(NodeBuilder::heading().child(NodeBuilder::text("Title")))
                .child(
                    NodeBuilder::container()
                        .child(NodeBuilder::button("Save"))
                        .child(NodeBuilder::paragraph()),
                )
                .child(NodeBuilder::text("Footer")),
        )
        .build();

    let tree = SnapshotTree::build(apc, BuildOptions::default())?;
    assert_main_doc_metadata(&tree)?;

    let root_children: Vec<u32> = tree
        .children(NodeId::new(ROOT))
        .map(|n| n.id().raw())
        .collect();
    assert_eq!(
        root_children,
        vec![HEADING, CONTAINER, FOOTER_TEXT],
        "root children must match declaration order in flat-tree preorder"
    );
    let container_children: Vec<u32> = tree
        .children(NodeId::new(CONTAINER))
        .map(|n| n.id().raw())
        .collect();
    assert_eq!(container_children, vec![SAVE_BUTTON, PARAGRAPH]);

    let save_text_ancestors: Vec<u32> = tree
        .ancestors(NodeId::new(SAVE_TEXT))
        .map(|n| n.id().raw())
        .collect();
    assert_eq!(
        save_text_ancestors,
        vec![SAVE_BUTTON, CONTAINER, ROOT],
        "ancestors must walk parent links up to the root"
    );

    let container_subtree: Vec<u32> = tree
        .subtree(NodeId::new(CONTAINER))
        .iter()
        .map(|n| n.id().raw())
        .collect();
    assert_eq!(
        container_subtree,
        vec![CONTAINER, SAVE_BUTTON, SAVE_TEXT, PARAGRAPH],
        "subtree slice must cover node and all descendants in O(1)"
    );

    let doc_node_ids: Vec<u32> = tree.doc_nodes(DocId::MAIN).map(|n| n.id().raw()).collect();
    assert_eq!(doc_node_ids, vec![0, 1, 2, 3, 4, 5, 6, 7]);

    let token = require_doc_token("doc-main")?;
    let container_key = NodeKey::new(token.clone(), DomNodeId::new(4));
    let button_key = NodeKey::new(token.clone(), DomNodeId::new(5));
    let footer_key = NodeKey::new(token.clone(), DomNodeId::new(8));
    let unknown_key = NodeKey::new(token, DomNodeId::new(999));

    assert!(tree.contains(&container_key, &container_key));
    assert!(!tree.is_strict_descendant(&container_key, &container_key));
    assert!(tree.contains(&container_key, &button_key));
    assert!(tree.is_strict_descendant(&container_key, &button_key));
    assert!(!tree.contains(&container_key, &footer_key));
    assert!(!tree.is_strict_descendant(&container_key, &footer_key));
    assert!(!tree.contains(&button_key, &container_key));
    assert!(!tree.contains(&container_key, &unknown_key));
    assert!(!tree.contains(&unknown_key, &button_key));
    Ok(())
}

#[test]
fn nested_iframes_scope_doc_nodes_containment_seq_and_colliding_dom_ids()
-> Result<(), Box<dyn Error>> {
    let apc = ApcBuilder::new("doc-top")
        .main_title("Top")
        .root(
            NodeBuilder::root()
                .child(NodeBuilder::text("Before"))
                .child(
                    NodeBuilder::iframe(
                        FrameBuilder::new("doc-mid").title("Mid").root(
                            NodeBuilder::root()
                                .child(NodeBuilder::text("Mid text"))
                                .child(NodeBuilder::iframe(
                                    FrameBuilder::new("doc-leaf").title("Leaf").root(
                                        NodeBuilder::root().child(NodeBuilder::text("Leaf text")),
                                    ),
                                )),
                        ),
                    )
                    .label("Mid frame"),
                )
                .child(NodeBuilder::iframe(
                    FrameBuilder::new("doc-empty").title("Empty"),
                ))
                .child(NodeBuilder::text("After")),
        )
        .build();

    let counter = Cell::new(10_u32);
    let seq_assigner = |_token: &DocToken, _frame: Option<&FrameToken>| {
        let next = counter.get();
        counter.set(next + 1);
        DocSeq::new(next)
    };
    let tree = SnapshotTree::build(apc, BuildOptions::default().with_seq_of(&seq_assigner))?;

    assert_eq!(tree.documents().len(), 4);
    assert!(tree.partial_frames().is_empty());

    let top_doc = tree.document(DocId::new(0)).ok_or("missing top doc")?;
    let mid_doc = tree.document(DocId::new(1)).ok_or("missing mid doc")?;
    let leaf_doc = tree.document(DocId::new(2)).ok_or("missing leaf doc")?;
    let empty_doc = tree.document(DocId::new(3)).ok_or("missing empty doc")?;

    assert_eq!(top_doc.seq(), Some(DocSeq::new(10)));
    assert_eq!(mid_doc.seq(), Some(DocSeq::new(11)));
    assert_eq!(leaf_doc.seq(), Some(DocSeq::new(12)));
    assert_eq!(empty_doc.seq(), Some(DocSeq::new(13)));

    assert_eq!(mid_doc.parent_doc(), Some(DocId::MAIN));
    assert_eq!(leaf_doc.parent_doc(), Some(mid_doc.id()));
    assert_eq!(empty_doc.parent_doc(), Some(DocId::MAIN));
    assert_eq!(empty_doc.root_node(), None);
    assert_eq!(empty_doc.node_count(), 0);
    assert!(empty_doc.node_range().is_empty());
    assert_eq!(tree.doc_nodes(empty_doc.id()).count(), 0);

    let top_nodes: Vec<u32> = tree.doc_nodes(top_doc.id()).map(|n| n.id().raw()).collect();
    assert_eq!(
        top_nodes,
        vec![0, 1, 2, 8, 9],
        "doc_nodes(top) must include iframe elements and jump over child document subtrees"
    );
    assert_eq!(top_doc.node_count(), 5);

    let mid_nodes: Vec<u32> = tree.doc_nodes(mid_doc.id()).map(|n| n.id().raw()).collect();
    assert_eq!(mid_nodes, vec![3, 4, 5]);
    assert_eq!(mid_doc.node_count(), 3);

    let leaf_nodes: Vec<u32> = tree
        .doc_nodes(leaf_doc.id())
        .map(|n| n.id().raw())
        .collect();
    assert_eq!(leaf_nodes, vec![6, 7]);
    assert_eq!(leaf_doc.node_count(), 2);

    assert_nested_iframe_dom_ids_and_keys(&tree, top_doc, mid_doc, leaf_doc)?;
    Ok(())
}

fn assert_nested_iframe_dom_ids_and_keys(
    tree: &SnapshotTree,
    top_doc: &TreeDoc,
    mid_doc: &TreeDoc,
    leaf_doc: &TreeDoc,
) -> Result<(), Box<dyn Error>> {
    let top_dom_ids: Vec<Option<i32>> = tree
        .doc_nodes(top_doc.id())
        .map(|n| n.dom_node_id().map(DomNodeId::raw))
        .collect();
    let mid_dom_ids: Vec<Option<i32>> = tree
        .doc_nodes(mid_doc.id())
        .map(|n| n.dom_node_id().map(DomNodeId::raw))
        .collect();
    let leaf_dom_ids: Vec<Option<i32>> = tree
        .doc_nodes(leaf_doc.id())
        .map(|n| n.dom_node_id().map(DomNodeId::raw))
        .collect();
    assert_eq!(
        top_dom_ids,
        vec![Some(1), Some(2), Some(3), Some(4), Some(5)],
        "top document must keep its own sequential auto DOM IDs across nested iframes"
    );
    assert_eq!(
        mid_dom_ids,
        vec![Some(1), Some(2), Some(3)],
        "mid document must allocate sequential auto DOM IDs for all descendants"
    );
    assert_eq!(
        leaf_dom_ids,
        vec![Some(1), Some(2)],
        "leaf document must allocate sequential auto DOM IDs independently"
    );

    let top_root_key = NodeKey::new(require_doc_token("doc-top")?, DomNodeId::new(1));
    let mid_root_key = NodeKey::new(require_doc_token("doc-mid")?, DomNodeId::new(1));
    let leaf_root_key = NodeKey::new(require_doc_token("doc-leaf")?, DomNodeId::new(1));
    let top_text_key = NodeKey::new(require_doc_token("doc-top")?, DomNodeId::new(2));
    let mid_text_key = NodeKey::new(require_doc_token("doc-mid")?, DomNodeId::new(2));
    let leaf_text_key = NodeKey::new(require_doc_token("doc-leaf")?, DomNodeId::new(2));
    let mid_iframe_key = mid_doc.iframe_key().ok_or("mid_doc missing iframe_key")?;

    assert_eq!(
        tree.node_by_key(&top_root_key).map(TreeNode::id),
        Some(NodeId::new(0))
    );
    assert_eq!(
        tree.node_by_key(&mid_root_key).map(TreeNode::id),
        Some(NodeId::new(3))
    );
    assert_eq!(
        tree.node_by_key(&leaf_root_key).map(TreeNode::id),
        Some(NodeId::new(6))
    );
    assert_eq!(
        tree.node_by_key(&top_text_key).map(TreeNode::id),
        Some(NodeId::new(1))
    );
    assert_eq!(
        tree.node_by_key(&mid_text_key).map(TreeNode::id),
        Some(NodeId::new(4))
    );
    assert_eq!(
        tree.node_by_key(&leaf_text_key).map(TreeNode::id),
        Some(NodeId::new(7))
    );

    assert!(
        !tree.contains(&top_root_key, &mid_root_key),
        "containment must not cross from main document into child iframe document"
    );
    assert!(
        !tree.contains(mid_iframe_key, &mid_root_key),
        "iframe element in parent doc must not contain nodes belonging to child doc"
    );
    assert!(
        !tree.is_strict_descendant(&mid_root_key, &leaf_root_key),
        "strict descendant check must not cross nested iframe boundaries"
    );
    assert!(
        tree.contains(&top_root_key, mid_iframe_key),
        "main root must still contain the iframe element itself in the main document"
    );
    Ok(())
}

#[test]
fn missing_dom_ids_and_duplicate_dom_ids_follow_preorder_rules() -> Result<(), Box<dyn Error>> {
    let apc = ApcBuilder::new("doc-dup")
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .child(NodeBuilder::container().without_dom_id())
                .child(NodeBuilder::container().without_attributes())
                .child(NodeBuilder::text("first").dom_id(55))
                .child(NodeBuilder::text("second").dom_id(55)),
        )
        .build();

    let tree = SnapshotTree::build(apc, BuildOptions::default())?;
    assert_eq!(tree.nodes().len(), 5);
    assert_eq!(tree.node_key(NodeId::new(1)), None);
    assert_eq!(tree.node_key(NodeId::new(2)), None);

    let dup_key = NodeKey::new(require_doc_token("doc-dup")?, DomNodeId::new(55));
    let resolved = tree.node_by_key(&dup_key).ok_or("missing dup key")?;
    assert_eq!(
        resolved.id(),
        NodeId::new(3),
        "first node in flat-tree preorder must win when a document duplicates a DomNodeId"
    );
    assert_eq!(
        tree.node_key(NodeId::new(4)),
        Some(dup_key),
        "node_key(id) still reconstructs the key from the node's own doc and dom_node_id"
    );
    Ok(())
}

#[test]
fn apc_builder_reserves_explicit_dom_ids_and_preserves_multi_child_main_frames()
-> Result<(), Box<dyn Error>> {
    let apc = ApcBuilder::new("doc-explicit")
        .root(
            NodeBuilder::root()
                .dom_id(1)
                .child(NodeBuilder::button("First"))
                .child(NodeBuilder::button("Reserved").dom_id(2)),
        )
        .build();
    let tree = SnapshotTree::build(apc, BuildOptions::default())?;
    let dom_ids: Vec<Option<i32>> = tree
        .nodes()
        .iter()
        .map(|node| node.dom_node_id().map(DomNodeId::raw))
        .collect();
    assert_eq!(
        dom_ids,
        vec![Some(1), Some(3), Some(4), Some(2), Some(5)],
        "auto DOM IDs must skip explicit IDs declared earlier or later in the same document"
    );
    let token = require_doc_token("doc-explicit")?;
    let root_key = NodeKey::new(token.clone(), DomNodeId::new(1));
    let first_btn_key = NodeKey::new(token, DomNodeId::new(3));
    assert!(
        tree.is_strict_descendant(&root_key, &first_btn_key),
        "auto-numbered child under explicit dom_id(1) root must have a distinct key"
    );

    let multi_root_apc = ApcBuilder::with_main_frame(
        FrameBuilder::new("doc-multi")
            .child(NodeBuilder::text("One"))
            .child(NodeBuilder::text("Two")),
    )
    .build();
    let multi_tree = SnapshotTree::build(multi_root_apc, BuildOptions::default())?;
    assert_eq!(
        multi_tree.nodes().len(),
        3,
        "with_main_frame must preserve all top-level children beneath a synthetic root"
    );
    assert_eq!(multi_tree.children(NodeId::new(0)).count(), 2);
    Ok(())
}

#[test]
fn empty_root_and_error_cases_return_expected_results() -> Result<(), Box<dyn Error>> {
    let empty_root = ApcBuilder::new("doc-empty-root").without_root().build();
    let tree = SnapshotTree::build(empty_root, BuildOptions::default())?;
    assert!(tree.nodes().is_empty());
    let main = tree.main_document().ok_or("missing main doc")?;
    assert_eq!(main.root_node(), None);
    assert_eq!(main.node_count(), 0);
    assert!(main.node_range().is_empty());

    let missing_main = ApcBuilder::without_main_frame().build();
    assert_eq!(
        SnapshotTree::build(missing_main, BuildOptions::default()).err(),
        Some(BuildError::MissingMainFrame)
    );

    let missing_token = ApcBuilder::new("unused").without_main_doc_token().build();
    assert_eq!(
        SnapshotTree::build(missing_token, BuildOptions::default()).err(),
        Some(BuildError::MissingMainDocumentToken)
    );

    let empty_token = ApcBuilder::new("unused").empty_main_doc_token().build();
    assert_eq!(
        SnapshotTree::build(empty_token, BuildOptions::default()).err(),
        Some(BuildError::MissingMainDocumentToken)
    );

    let raised_gate = ApcBuilder::new("doc-gate").build();
    assert_eq!(
        SnapshotTree::build(
            raised_gate,
            BuildOptions::default().with_min_contract_version(2),
        )
        .err(),
        Some(BuildError::Unpatched {
            found: Some(MIN_CONTRACT_VERSION),
            required: 2,
        })
    );

    let unpatched = ApcBuilder::unpatched("doc-stock").build();
    assert_eq!(
        SnapshotTree::build(unpatched, BuildOptions::default()).err(),
        Some(BuildError::Unpatched {
            found: None,
            required: MIN_CONTRACT_VERSION,
        }),
        "captures without BrowserOS page metadata must be refused"
    );

    let missing_version = ApcBuilder::new("doc-no-ver")
        .without_contract_version()
        .build();
    assert_eq!(
        SnapshotTree::build(missing_version, BuildOptions::default()).err(),
        Some(BuildError::Unpatched {
            found: None,
            required: MIN_CONTRACT_VERSION,
        }),
        "captures with empty contract_version must be refused"
    );

    let zero_version = ApcBuilder::new("doc-zero").contract_version(0).build();
    assert_eq!(
        SnapshotTree::build(zero_version, BuildOptions::default()).err(),
        Some(BuildError::Unpatched {
            found: Some(0),
            required: MIN_CONTRACT_VERSION,
        }),
        "contract_version = 0 must be refused below MIN_CONTRACT_VERSION"
    );

    let patched_v2 = ApcBuilder::new("doc-v2")
        .contract_version(2)
        .subframe_timeout_ms(1_500)
        .build();
    let v2_tree = SnapshotTree::build(
        patched_v2,
        BuildOptions::default().with_min_contract_version(2),
    )?;
    assert_eq!(v2_tree.contract_version(), 2);
    assert_eq!(v2_tree.subframe_timeout_ms(), Some(1_500));
    Ok(())
}

#[test]
fn contract_gate_admits_v6_fixtures_and_refuses_them_with_the_contract_stripped()
-> Result<(), Box<dyn Error>> {
    for name in [
        "nested-cross-site-frames",
        "hung-subframe",
        "sensitive-form",
    ] {
        let tree = load_fixture_tree(name)?;
        assert_eq!(
            tree.contract_version(),
            6,
            "{name}.pb was recorded on a contract-v6 build"
        );
        let mut stock_apc = decode_fixture(name)?;
        stock_apc.browseros = None;
        assert_eq!(
            SnapshotTree::build(stock_apc, BuildOptions::default()).err(),
            Some(BuildError::Unpatched {
                found: None,
                required: MIN_CONTRACT_VERSION,
            }),
            "{name}.pb without its contract page message reads as a stock reply and must be \
             refused fail-closed"
        );
    }
    Ok(())
}

fn build_rich_browseros_node() -> NodeBuilder {
    NodeBuilder::element("button")
        .role("button")
        .aria_label("Pay")
        .href("")
        .alt("")
        .scope("col")
        .list_box(false)
        .has_datalist(false)
        .aria_hidden(false)
        .break_before(true)
        .dom_text("Pay")
        .labelledby(vec![BrowserOsRef {
            dom_node_id: Some(1),
            unrendered_text: None,
        }])
        .describedby(vec![BrowserOsRef {
            dom_node_id: None,
            unrendered_text: Some("Total due".to_owned()),
        }])
        .errormessage(Vec::new())
        .title("Submit payment")
        .aria_description("Charges card")
        .checked_state(BrowserOsTriState::Mixed)
        .pressed_state(BrowserOsTriState::True)
        .expanded_state(BrowserOsTriState::False)
        .selected(true)
        .level(2)
        .contract_disabled(false)
        .aria_disabled(false)
        .aria_invalid("false")
        .redacted_value_length(0)
        .visibility_hidden(false)
        .pointer_events_none(false)
        .inert(false)
        .closed_root_seed(false)
        .ua_shadow(false)
        .display_contents(false)
}

#[test]
fn frame_tokens_and_browseros_fields_round_trip_through_wire_decode() -> Result<(), Box<dyn Error>>
{
    let apc = ApcBuilder::new("doc-top")
        .contract_version(4)
        .subframe_timeout_ms(2_000)
        .main_frame_token("frame-top")
        .root(
            NodeBuilder::root()
                .child(build_rich_browseros_node())
                .child(NodeBuilder::iframe(
                    FrameBuilder::new("doc-child")
                        .frame_token("frame-child")
                        .root(NodeBuilder::root().child(NodeBuilder::text("Inside"))),
                ))
                .child(NodeBuilder::iframe(
                    FrameBuilder::new("doc-empty-ft").frame_token(""),
                ))
                .child(
                    NodeBuilder::partial_iframe(PartialIframeSpec::Timeout)
                        .redacted_frame_token("frame-hung")
                        .dom_id(90),
                )
                .child(
                    NodeBuilder::partial_iframe(PartialIframeSpec::CrossSite)
                        .redacted_frame_token("")
                        .dom_id(91),
                ),
        )
        .build();

    let encoded = apc.encode_to_vec();
    let decoded = neo_snapshot_apc::decode_protobuf(&encoded)?;

    let observed_seq_inputs: RefCell<Vec<(String, Option<String>)>> = RefCell::new(Vec::new());
    let seq_assigner = |doc: &DocToken, frame: Option<&FrameToken>| {
        let mut log = observed_seq_inputs.borrow_mut();
        let seq = u32::try_from(log.len()).unwrap_or(u32::MAX);
        log.push((
            doc.as_str().to_owned(),
            frame.map(|token| token.as_str().to_owned()),
        ));
        DocSeq::new(seq)
    };
    let tree = SnapshotTree::build(
        decoded,
        BuildOptions::default()
            .with_min_contract_version(4)
            .with_seq_of(&seq_assigner),
    )?;

    assert_eq!(tree.contract_version(), 4);
    assert_eq!(tree.subframe_timeout_ms(), Some(2_000));
    assert_eq!(
        *observed_seq_inputs.borrow(),
        vec![
            ("doc-top".to_owned(), Some("frame-top".to_owned())),
            ("doc-child".to_owned(), Some("frame-child".to_owned())),
            ("doc-empty-ft".to_owned(), None),
        ],
        "seq_of must receive validated non-empty FrameTokens and None for empty tokens"
    );

    assert_eq!(
        tree.document(DocId::MAIN)
            .and_then(TreeDoc::frame_token)
            .map(FrameToken::as_str),
        Some("frame-top")
    );
    assert_eq!(
        tree.document(DocId::new(1))
            .and_then(TreeDoc::frame_token)
            .map(FrameToken::as_str),
        Some("frame-child")
    );
    assert_eq!(
        tree.document(DocId::new(2)).and_then(TreeDoc::frame_token),
        None,
        "empty devtools_frame_token must normalize to None"
    );

    let hung = tree.partial_frame(0).ok_or("missing hung partial frame")?;
    assert_eq!(
        hung.frame_token.as_ref().map(FrameToken::as_str),
        Some("frame-hung")
    );
    assert_eq!(
        hung.reason,
        PartialFrameReason::Timeout {
            timeout_ms: Some(2_000),
        }
    );
    assert_eq!(hung.reason.to_string(), "timeout after 2000 ms");

    let cross_site = tree
        .partial_frame(1)
        .ok_or("missing cross-site partial frame")?;
    assert_eq!(
        cross_site.frame_token, None,
        "empty redacted devtools_frame_token must normalize to None"
    );
    assert_eq!(cross_site.reason, PartialFrameReason::CrossSite);
    Ok(())
}

#[test]
fn partial_and_redacted_iframes_record_reasons_without_descending_children()
-> Result<(), Box<dyn Error>> {
    let apc = ApcBuilder::new("doc-main")
        .without_subframe_timeout()
        .root(
            NodeBuilder::root()
                .child(
                    NodeBuilder::iframe(
                        FrameBuilder::without_doc_token()
                            .root(NodeBuilder::root().child(NodeBuilder::text("leaked"))),
                    )
                    .dom_id(10),
                )
                .child(
                    NodeBuilder::iframe(
                        FrameBuilder::new("")
                            .root(NodeBuilder::root().child(NodeBuilder::text("empty-tok"))),
                    )
                    .dom_id(11),
                )
                .child(
                    NodeBuilder::partial_iframe(PartialIframeSpec::CrossSite)
                        .dom_id(12)
                        .child(NodeBuilder::text("must-not-descend")),
                )
                .child(NodeBuilder::partial_iframe(PartialIframeSpec::CrossOrigin).dom_id(13))
                .child(NodeBuilder::partial_iframe(PartialIframeSpec::Timeout).dom_id(14))
                .child(NodeBuilder::partial_iframe(PartialIframeSpec::Redacted(42)).dom_id(15))
                .child(
                    NodeBuilder::partial_iframe(PartialIframeSpec::RedactedUnspecified).dom_id(16),
                )
                .child(NodeBuilder::partial_iframe(PartialIframeSpec::MissingContent).dom_id(17))
                .child(NodeBuilder::partial_iframe(PartialIframeSpec::MissingPayload).dom_id(18)),
        )
        .build();

    let tree = SnapshotTree::build(apc, BuildOptions::default())?;
    assert_eq!(
        tree.documents().len(),
        1,
        "partial and unidentifiable iframes must not register child TreeDocs"
    );
    assert_eq!(
        tree.nodes().len(),
        10,
        "children beneath partial iframes must never be stitched into the parent document"
    );

    let reasons: Vec<PartialFrameReason> =
        tree.partial_frames().iter().map(|pf| pf.reason).collect();
    assert_eq!(
        reasons,
        vec![
            PartialFrameReason::MissingDocumentToken,
            PartialFrameReason::MissingDocumentToken,
            PartialFrameReason::CrossSite,
            PartialFrameReason::CrossOrigin,
            PartialFrameReason::Timeout { timeout_ms: None },
            PartialFrameReason::Redacted(42),
            PartialFrameReason::Redacted(0),
            PartialFrameReason::MissingContent,
            PartialFrameReason::MissingContent,
        ]
    );

    let display_strings: Vec<String> = reasons.iter().map(ToString::to_string).collect();
    assert_eq!(
        display_strings,
        vec![
            "frame missing document token",
            "frame missing document token",
            "cross-site frame redacted",
            "cross-origin frame redacted",
            "timeout",
            "frame redacted (42)",
            "frame redacted (0)",
            "frame content unavailable",
            "frame content unavailable",
        ]
    );

    let expected_timeout_display = PartialFrameReason::Timeout {
        timeout_ms: Some(2_000),
    }
    .to_string();
    assert_eq!(expected_timeout_display, "timeout after 2000 ms");

    for (index, partial) in tree.partial_frames().iter().enumerate() {
        assert_eq!(partial.parent_doc, DocId::MAIN);
        let node = tree
            .node(partial.iframe_node)
            .ok_or("partial frame points to invalid iframe_node")?;
        assert!(node.is_iframe_boundary());
        assert_eq!(node.child_doc(), None);
        assert_eq!(node.partial_frame_index(), Some(index));
        assert_eq!(
            tree.partial_frame(index),
            Some(partial),
            "partial_frame(index) must match slice entry"
        );
        assert_eq!(
            tree.children(partial.iframe_node).count(),
            0,
            "partial iframe must have zero stitched children"
        );
    }
    Ok(())
}

#[test]
fn nested_partial_iframe_records_child_doc_as_parent_and_uses_child_doc_token()
-> Result<(), Box<dyn Error>> {
    let apc = ApcBuilder::new("doc-top")
        .subframe_timeout_ms(2_000)
        .root(
            NodeBuilder::root().child(NodeBuilder::iframe(
                FrameBuilder::new("doc-child")
                    .frame_token("frame-child")
                    .root(
                        NodeBuilder::root().child(
                            NodeBuilder::partial_iframe(PartialIframeSpec::Timeout)
                                .redacted_frame_token("frame-nested-hung")
                                .dom_id(77),
                        ),
                    ),
            )),
        )
        .build();

    let tree = SnapshotTree::build(apc, BuildOptions::default())?;
    assert_eq!(tree.documents().len(), 2);
    assert_eq!(tree.partial_frames().len(), 1);

    let child_doc = tree.document(DocId::new(1)).ok_or("missing child doc")?;
    assert_eq!(
        child_doc.frame_token().map(FrameToken::as_str),
        Some("frame-child")
    );
    let partial = tree
        .partial_frame(0)
        .ok_or("missing nested partial frame")?;
    assert_eq!(partial.parent_doc, child_doc.id());
    assert_eq!(
        partial.frame_token.as_ref().map(FrameToken::as_str),
        Some("frame-nested-hung")
    );
    assert_eq!(
        partial.iframe_key,
        Some(NodeKey::new(child_doc.token().clone(), DomNodeId::new(77))),
        "nested partial iframe's NodeKey must be scoped to its embedding child document"
    );
    assert_eq!(
        partial.reason,
        PartialFrameReason::Timeout {
            timeout_ms: Some(2_000),
        }
    );
    Ok(())
}

#[test]
fn debug_formatting_redacts_sensitive_page_content_and_metadata() -> Result<(), Box<dyn Error>> {
    const CANARIES: &[&str] = &[
        "CANARY_SECRET_FIELD_VALUE",
        "CANARY_DOM_TEXT_SECRET",
        "CANARY_BROWSEROS_DOM_TEXT",
        "CANARY_ARIA_LABEL_SECRET",
        "CANARY_BROWSEROS_ARIA_LABEL",
        "CANARY_BROWSEROS_TITLE",
        "CANARY_BROWSEROS_DESC",
        "CANARY_BROWSEROS_HREF",
        "CANARY_BROWSEROS_ALT",
        "CANARY_PLACEHOLDER_SECRET",
        "CANARY_DOC_TITLE_SECRET",
        "https://canary-url-secret.example/private?token=abc",
        "https://canary-origin-secret.example",
        "CANARY_ANCHOR_URL_SECRET",
        "CANARY_OPTION_TEXT_SECRET",
    ];

    let apc = ApcBuilder::new("doc-sensitive")
        .main_url("https://canary-url-secret.example/private?token=abc")
        .main_title("CANARY_DOC_TITLE_SECRET")
        .main_origin("https://canary-origin-secret.example", false)
        .root(
            NodeBuilder::root()
                .child(
                    NodeBuilder::text("CANARY_DOM_TEXT_SECRET")
                        .dom_text("CANARY_BROWSEROS_DOM_TEXT"),
                )
                .child(
                    NodeBuilder::form_control(FormControlType::InputText)
                        .aria_role(AxRole::TextField)
                        .label("CANARY_ARIA_LABEL_SECRET")
                        .aria_label("CANARY_BROWSEROS_ARIA_LABEL")
                        .title("CANARY_BROWSEROS_TITLE")
                        .aria_description("CANARY_BROWSEROS_DESC")
                        .href("CANARY_BROWSEROS_HREF")
                        .alt("CANARY_BROWSEROS_ALT")
                        .field_name("secret_input")
                        .field_value("CANARY_SECRET_FIELD_VALUE")
                        .placeholder("CANARY_PLACEHOLDER_SECRET")
                        .required(true)
                        .readonly(false)
                        .checked(false)
                        .redaction(RedactionDecision::NoRedactionNecessary)
                        .annotated_role(AnnotatedRole::Main)
                        .bounding_box(10, 20, 200, 32)
                        .z_order(5)
                        .focusable(true)
                        .tabbable(true)
                        .disabled(false)
                        .clickability_reason(ClickabilityReason::ClickableControl)
                        .disabled_reason(InteractionDisabledReason::Unspecified)
                        .select_option(SelectOption {
                            value: Some("v".to_owned()),
                            text: Some("CANARY_OPTION_TEXT_SECRET".to_owned()),
                            is_selected: Some(true),
                            is_disabled: Some(false),
                        }),
                )
                .child(
                    NodeBuilder::anchor("https://canary-anchor.example/CANARY_ANCHOR_URL_SECRET")
                        .label_for(2),
                ),
        )
        .build();

    let tree = SnapshotTree::build(apc, BuildOptions::default())?;
    let mut debug_outputs = vec![format!("{tree:?}")];
    for doc in tree.documents() {
        debug_outputs.push(format!("{doc:?}"));
    }
    for node in tree.nodes() {
        debug_outputs.push(format!("{node:?}"));
    }

    for output in &debug_outputs {
        for canary in CANARIES {
            assert!(
                !output.contains(canary),
                "redacting Debug output leaked sensitive string {canary:?}: {output}"
            );
        }
    }
    Ok(())
}

#[test]
fn nested_cross_site_frames_fixture_stitches_eight_docs_and_disambiguates_dom_ids()
-> Result<(), Box<dyn Error>> {
    let mut apc = decode_fixture("nested-cross-site-frames")?;
    apc.browseros = Some(BrowserOsPageContentData {
        contract_version: Some(2),
        subframe_timeout_ms: Some(2_000),
    });
    if let Some(main) = apc.main_frame_data.as_mut() {
        main.browseros = Some(BrowserOsFrameData {
            devtools_frame_token: Some("frame-top".to_owned()),
        });
    }
    let tree = SnapshotTree::build(apc, BuildOptions::default())?;
    assert_eq!(tree.documents().len(), 8);
    assert_eq!(tree.nodes().len(), 75);
    assert!(tree.partial_frames().is_empty());
    assert_eq!(
        tree.main_document()
            .and_then(TreeDoc::frame_token)
            .map(FrameToken::as_str),
        Some("frame-top")
    );

    let titles: Vec<Option<&str>> = tree.documents().iter().map(TreeDoc::title).collect();
    assert_eq!(
        titles,
        vec![
            Some("Top"),
            Some("A1"),
            Some("B2"),
            Some("B3"),
            Some("B1"),
            Some("C2"),
            Some("A3"),
            Some("C4"),
        ],
        "documents must appear in flat-tree preorder across the 4-level hierarchy"
    );

    let by_title: HashMap<&str, DocId> = tree
        .documents()
        .iter()
        .filter_map(|doc| Some((doc.title()?, doc.id())))
        .collect();

    let expected_parents = [
        ("Top", None),
        ("A1", Some("Top")),
        ("B2", Some("A1")),
        ("B3", Some("B2")),
        ("B1", Some("Top")),
        ("C2", Some("B1")),
        ("A3", Some("C2")),
        ("C4", Some("A3")),
    ];
    for (title, expected_parent) in expected_parents {
        let doc_id = *by_title.get(title).ok_or("missing expected frame title")?;
        let doc = tree.document(doc_id).ok_or("missing doc by id")?;
        let actual_parent = doc
            .parent_doc()
            .and_then(|pid| tree.document(pid))
            .and_then(|parent| parent.title());
        assert_eq!(
            actual_parent, expected_parent,
            "parent hierarchy mismatch for frame {title}"
        );
        if let Some(iframe_id) = doc.iframe_node() {
            let iframe_node = tree.node(iframe_id).ok_or("invalid iframe_node")?;
            assert_eq!(iframe_node.child_doc(), Some(doc_id));
            assert_eq!(
                iframe_node.doc(),
                doc.parent_doc().ok_or("missing parent_doc")?
            );
            assert!(iframe_node.is_iframe_boundary());
        }
    }

    let mut total_owned = 0_usize;
    for doc in tree.documents() {
        let walked: Vec<NodeId> = tree.doc_nodes(doc.id()).map(TreeNode::id).collect();
        assert_eq!(
            walked.len(),
            doc.node_count(),
            "doc_nodes count must equal node_count for {:?}",
            doc.title()
        );
        for id in walked {
            let node = tree.node(id).ok_or("walked invalid node id")?;
            assert_eq!(
                node.doc(),
                doc.id(),
                "doc_nodes must only yield nodes directly owned by the requested document"
            );
        }
        total_owned += doc.node_count();
    }
    assert_eq!(
        total_owned, 75,
        "sum of per-document node_count across all 8 documents must equal total arena size"
    );

    assert_colliding_dom_ids(&tree, &by_title, 12, &["Top", "B2", "C4"])?;
    assert_colliding_dom_ids(&tree, &by_title, 11, &["Top", "B2", "C2"])?;
    Ok(())
}

fn assert_colliding_dom_ids(
    tree: &SnapshotTree,
    by_title: &HashMap<&str, DocId>,
    collision_dom_id: i32,
    frame_titles: &[&str],
) -> Result<(), Box<dyn Error>> {
    let mut seen_arena_ids = Vec::new();
    for &title in frame_titles {
        let doc_id = *by_title.get(title).ok_or("missing colliding frame title")?;
        let doc = tree.document(doc_id).ok_or("missing doc")?;
        let key = NodeKey::new(doc.token().clone(), DomNodeId::new(collision_dom_id));
        let node = tree
            .node_by_key(&key)
            .ok_or("expected colliding DomNodeId in fixture document")?;
        assert_eq!(
            node.doc(),
            doc_id,
            "composite NodeKey must resolve dom_id={collision_dom_id} inside {title}"
        );
        assert!(
            !seen_arena_ids.contains(&node.id()),
            "colliding dom_id={collision_dom_id} must map to distinct arena nodes across frames"
        );
        seen_arena_ids.push(node.id());
    }
    Ok(())
}

#[test]
fn hung_subframe_fixture_records_partial_frame_and_preserves_healthy_sibling()
-> Result<(), Box<dyn Error>> {
    let tree = load_fixture_tree("hung-subframe")?;
    assert_eq!(tree.documents().len(), 2);
    assert_eq!(tree.nodes().len(), 16);
    assert_eq!(tree.partial_frames().len(), 1);

    let main_doc = tree.document(DocId::MAIN).ok_or("missing main doc")?;
    let healthy_doc = tree.document(DocId::new(1)).ok_or("missing healthy doc")?;
    assert_eq!(main_doc.title(), Some("Hung Subframe Fixture"));
    assert_eq!(healthy_doc.title(), Some("Healthy Frame"));
    assert_eq!(healthy_doc.parent_doc(), Some(DocId::MAIN));

    let partial = tree
        .partial_frame(0)
        .ok_or("expected one PartialFrame for hung iframe")?;
    assert_eq!(
        partial.reason,
        PartialFrameReason::Timeout {
            timeout_ms: Some(2_000),
        },
        "the contract stamps the hung iframe BrowserOsTimeout and the page its 2000 ms timeout"
    );
    assert_eq!(partial.reason.to_string(), "timeout after 2000 ms");
    assert_eq!(
        partial.frame_token.as_ref().map(FrameToken::as_str),
        Some(HUNG_FRAME_TOKEN),
        "the hung frame's token is the CDP frame id hung-subframe.oracle.json records for it"
    );
    assert_eq!(partial.parent_doc, DocId::MAIN);
    let partial_key = partial
        .iframe_key
        .as_ref()
        .ok_or("hung iframe must carry a composite NodeKey in main doc")?;
    assert_eq!(partial_key.doc, *main_doc.token());
    let partial_node = tree
        .node(partial.iframe_node)
        .ok_or("invalid partial iframe_node")?;
    assert_eq!(partial_node.doc(), DocId::MAIN);
    assert_eq!(partial_node.partial_frame_index(), Some(0));
    assert!(partial_node.is_iframe_boundary());

    assert_eq!(
        main_doc.node_count() + healthy_doc.node_count(),
        16,
        "populated docs must account for all 16 arena nodes"
    );

    for raw_id in [4_i32, 7_i32] {
        let main_key = NodeKey::new(main_doc.token().clone(), DomNodeId::new(raw_id));
        let healthy_key = NodeKey::new(healthy_doc.token().clone(), DomNodeId::new(raw_id));
        let main_node = tree
            .node_by_key(&main_key)
            .ok_or("missing colliding dom id in main doc")?;
        let healthy_node = tree
            .node_by_key(&healthy_key)
            .ok_or("missing colliding dom id in healthy doc")?;
        assert_eq!(main_node.doc(), DocId::MAIN);
        assert_eq!(healthy_node.doc(), healthy_doc.id());
        assert_ne!(main_node.id(), healthy_node.id());
    }

    Ok(())
}

#[test]
fn remaining_chromium_fixtures_build_and_redact_sensitive_debug() -> Result<(), Box<dyn Error>> {
    let cases = [
        ("deep-nesting", 1_usize, 127_usize),
        ("shadow-dom", 1, 23),
        ("zero-size-checkbox", 1, 13),
        ("covered-button", 1, 16),
        ("sensitive-form", 1, 20),
    ];

    for (name, expected_docs, expected_nodes) in cases {
        let tree = load_fixture_tree(name)?;
        assert_eq!(
            tree.documents().len(),
            expected_docs,
            "{name}: unexpected document count"
        );
        assert_eq!(
            tree.nodes().len(),
            expected_nodes,
            "{name}: unexpected node count"
        );
        assert!(
            tree.partial_frames().is_empty(),
            "{name}: expected zero partial frames"
        );
        let main = tree.main_document().ok_or("missing main document")?;
        assert_eq!(main.node_count(), expected_nodes);
        assert_eq!(tree.doc_nodes(DocId::MAIN).count(), expected_nodes);

        if name == "deep-nesting" {
            let max_ancestors = tree
                .nodes()
                .iter()
                .map(|node| tree.ancestors(node.id()).count())
                .max()
                .unwrap_or(0);
            assert!(
                max_ancestors > 100,
                "deep-nesting.pb must preserve >100-level ancestor chain (got {max_ancestors})"
            );
        }

        if name == "sensitive-form" {
            let mut debug_outputs = vec![format!("{tree:?}")];
            for doc in tree.documents() {
                debug_outputs.push(format!("{doc:?}"));
            }
            for node in tree.nodes() {
                debug_outputs.push(format!("{node:?}"));
            }
            for output in &debug_outputs {
                for forbidden in [
                    "Sensitive Form",
                    "Card number",
                    "One-time code",
                    "Password",
                    "Pay now",
                    "4242",
                    "123456",
                    "http://",
                ] {
                    assert!(
                        !output.contains(forbidden),
                        "sensitive-form Debug output leaked {forbidden:?}: {output}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn deep_chain_100k_nodes_builds_queries_formats_and_drops_without_stack_overflow()
-> Result<(), Box<dyn Error>> {
    const DEPTH: usize = 100_000;

    // Run on a constrained 2 MiB thread stack so any accidental recursion fails immediately.
    let handle = thread::Builder::new()
        .name("deep-chain-100k".to_owned())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| -> Result<(), String> {
            let apc = ApcBuilder::deep_chain(DEPTH);
            let tree = SnapshotTree::build(apc, BuildOptions::default())
                .map_err(|err| format!("build failed: {err}"))?;

            assert_eq!(tree.nodes().len(), DEPTH);
            assert_eq!(tree.subtree(NodeId::new(0)).len(), DEPTH);

            let doc_token = DocToken::new("doc-deep-chain").ok_or("valid doc token")?;
            let root_key = NodeKey::new(doc_token.clone(), DomNodeId::new(1));
            let deepest_dom = i32::try_from(DEPTH).map_err(|err| err.to_string())?;
            let deepest_key = NodeKey::new(doc_token, DomNodeId::new(deepest_dom));
            let deepest_id = NodeId::new(u32::try_from(DEPTH - 1).map_err(|err| err.to_string())?);

            assert!(tree.contains(&root_key, &deepest_key));
            assert!(tree.is_strict_descendant(&root_key, &deepest_key));
            assert_eq!(tree.ancestors(deepest_id).count(), DEPTH - 1);

            let summary = format!("{tree:?}");
            assert!(
                summary.contains("node_count: 100000"),
                "Debug summary must report node_count without recursing: {summary}"
            );
            drop(tree);
            Ok(())
        })?;

    match handle.join() {
        Ok(result) => result.map_err(Into::into),
        Err(_panic) => Err("deep-chain-100k thread panicked".into()),
    }
}
