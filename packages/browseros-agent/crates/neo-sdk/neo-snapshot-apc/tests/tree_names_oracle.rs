//! Offline parity checks comparing [`neo_snapshot_apc::tree::NameCache`] accessible names,
//! descriptions, and error messages against the 16 recorded Playwright oracles
//! (`tests/fixtures/<name>.{pb,oracle.json}`) and a contract-v4 mirror of `tests/pages/names.html`.
//!
//! Frames join by `owner` `<iframe>` `DomNodeId` and elements join by `backendNodeId`. Every
//! recording carries contract v4, so the joined elements whose name, description or error message
//! differ from the oracle's must equal the parity ledger (`support/parity_ledger.rs`).

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
    reason = "this test measures name, description and error message; join, role, hidden and state belong to tree_roles_states_oracle.rs"
)]
mod parity_ledger;

use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use fixture_pages::FIXTURES;
use neo_snapshot_apc::proto::{
    AnnotatedPageContent, BrowserOsRef, FormControlType, RedactionDecision,
};
use neo_snapshot_apc::tree::{
    ApcBuilder, BuildOptions, DocId, DomNodeId, NameCache, NameVisibility, NodeBuilder, NodeKey,
    Role, SnapshotTree, TreeNode,
};
use oracle::{Oracle, OracleElement, OracleFrame};
use parity_ledger::{Field, Measured};

/// The first contract version that carries names, descriptions and error messages (contract C4).
const CONTRACT_NAMES_VERSION: u32 = 4;
const CONTRACT_GAP_G6_ERROR_MESSAGE: &str = "error message: shown because validity fails";
const ERR_REQUIRED_BACKEND_NODE_ID: u64 = 17;

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
    use serde::Deserialize;

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
    oracle_frames: usize,
    joined_frames: usize,
    partial_frames: usize,
    joined_elements: usize,
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

fn compare_joined_element(
    fixture: &'static str,
    frame: &OracleFrame,
    element: &OracleElement,
    node: &TreeNode,
    cache: &NameCache<'_>,
    stats: &mut FixtureNameComparison,
) {
    stats.joined_elements += 1;

    let actual_name = cache.name(node.id(), NameVisibility::VisibleOnly).text();
    let actual_description = cache
        .description(node.id(), NameVisibility::VisibleOnly)
        .text();
    let actual_error = cache.error_message(node.id());

    let mut check = |field: Field, actual: &str, expected: &str| {
        if actual != expected {
            stats.disagreements.push(Measured {
                fixture,
                // The parity ledger keys a child frame by its URL and the main frame, the only
                // one with no `parent`, by `None`.
                frame: frame.parent.as_ref().map(|_| frame.url.clone()),
                backend_node_id: element.backend_node_id,
                field,
                tree: actual.to_owned(),
                detail: format!(
                    "{fixture} [{}] node {}: {field:?} tree={actual:?} oracle={expected:?}",
                    frame.url, element.backend_node_id
                ),
            });
        }
    };
    check(Field::Name, actual_name, &element.name);
    check(Field::Description, actual_description, &element.description);
    check(Field::ErrorMessage, actual_error, &element.error_message);
}

fn compare_fixture(name: &'static str) -> Result<FixtureNameComparison, Box<dyn Error>> {
    let apc = load_pb(name)?;
    let oracle = load_oracle(name)?;
    let recorded_version = apc
        .browseros
        .as_ref()
        .and_then(|meta| meta.contract_version);
    assert!(
        recorded_version >= Some(CONTRACT_NAMES_VERSION),
        "{name}: recorded at contract {recorded_version:?}; parity needs v{CONTRACT_NAMES_VERSION}+ \
         names, descriptions and error messages (tests/fixtures/README.md re-records with --contract)"
    );

    let tree = SnapshotTree::build(apc, BuildOptions::default())?;
    let cache = tree.name_cache();
    let MappedOracleFrames {
        frame_to_doc,
        partial_frames,
    } = map_oracle_frames(&tree, &oracle)?;

    let mut stats = FixtureNameComparison {
        oracle_frames: oracle.frames.len(),
        joined_frames: frame_to_doc.len(),
        partial_frames,
        joined_elements: 0,
        disagreements: Vec::new(),
    };

    // An element with no SnapshotTree node is the roles test's `Field::Join` ledger entry.
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
                compare_joined_element(name, frame, element, node, &cache, &mut stats);
            }
        }
    }

    Ok(stats)
}

#[test]
fn recorded_fixtures_agree_with_playwright_oracles_on_all_expressible_name_facts()
-> Result<(), Box<dyn Error>> {
    let mut total_joined = 0;
    let mut disagreements = Vec::new();

    for fixture in FIXTURES {
        let stats = compare_fixture(fixture.name)?;
        total_joined += stats.joined_elements;

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
        disagreements.extend(stats.disagreements);
    }

    assert!(
        total_joined >= 640,
        "expected >= 640 joined oracle elements across all 16 fixtures, saw {total_joined}"
    );
    parity_ledger::assert_measured_equals_ledger(
        &disagreements,
        &[Field::Name, Field::Description, Field::ErrorMessage],
    );

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
