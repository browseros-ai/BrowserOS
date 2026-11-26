//! Offline tests for the BrowserOS half of the APC wire contract (`apc-contract-design` draft 2,
//! sections 4.1 to 4.4).
//!
//! The bytes are encoded by hand from the contract's field table, not by the structs under test,
//! so a wrong tag or wire type in `src/proto/browseros.rs` decodes into the wrong field or fails.
//! Re-encoding the decoded tree must give the same bytes back.

use std::error::Error;
use std::fs;
use std::path::PathBuf;

use neo_snapshot_apc::proto::{
    BrowserOsContentAttributes, BrowserOsFrameData, BrowserOsPageContentData,
    BrowserOsRedactedFrame, BrowserOsRef, BrowserOsTriState, ContentAttributeType,
    ContentAttributes, ContentData, ContentNode, FrameData, IframeData, IframeDataContent,
    REASON_BROWSEROS_TIMEOUT, RedactedFrameMetadata, RedactedFrameReason,
};
use neo_snapshot_apc::{AnnotatedPageContent, decode_protobuf};
use prost::Message as _;

/// Protobuf wire builder for one message, written field by field in the order given.
#[derive(Default)]
struct Wire(Vec<u8>);

impl Wire {
    fn varint(mut self, tag: u32, value: u64) -> Self {
        push_varint(u64::from(tag) << 3, &mut self.0);
        push_varint(value, &mut self.0);
        self
    }

    fn bytes(mut self, tag: u32, payload: &[u8]) -> Self {
        push_varint((u64::from(tag) << 3) | 2, &mut self.0);
        push_varint(
            u64::try_from(payload.len()).unwrap_or_default(),
            &mut self.0,
        );
        self.0.extend_from_slice(payload);
        self
    }

    fn string(self, tag: u32, value: &str) -> Self {
        self.bytes(tag, value.as_bytes())
    }

    fn message(self, tag: u32, message: Self) -> Self {
        let Self(payload) = message;
        self.bytes(tag, &payload)
    }
}

fn push_varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let low = u8::try_from(value & 0x7f).unwrap_or_default();
        value >>= 7;
        if value == 0 {
            out.push(low);
            return;
        }
        out.push(low | 0x80);
    }
}

/// `BrowserOSContentAttributes` with every field of section 4.1 set, in field-number order.
fn every_node_fact_wire() -> Wire {
    Wire::default()
        .string(1, "th")
        .string(2, "columnheader")
        .string(3, "/cart")
        .string(4, "")
        .string(5, "row")
        .varint(6, 1)
        .varint(7, 1)
        .varint(8, 1)
        .varint(9, 1)
        .string(10, "Buy")
        .string(11, "Close")
        .message(12, Wire::default().varint(1, 41))
        .message(12, Wire::default().string(2, "Hidden label"))
        .message(13, Wire::default().varint(1, 42))
        .message(14, Wire::default().varint(1, 43))
        .string(15, "Tip")
        .string(16, "More detail")
        .varint(17, 3)
        .varint(18, 2)
        .varint(19, 1)
        .varint(20, 1)
        .varint(21, 3)
        .varint(22, 1)
        .varint(23, 1)
        .string(24, "spelling")
        .varint(27, 6)
        .varint(28, 1)
        .varint(29, 1)
        .varint(30, 1)
        .varint(31, 1)
        .varint(32, 1)
        .varint(33, 1)
        .string(34, "Close")
}

fn every_node_fact() -> BrowserOsContentAttributes {
    BrowserOsContentAttributes {
        tag: Some("th".to_owned()),
        role: Some("columnheader".to_owned()),
        href: Some("/cart".to_owned()),
        alt: Some(String::new()),
        scope: Some("row".to_owned()),
        list_box: Some(true),
        has_datalist: Some(true),
        aria_hidden: Some(true),
        break_before: Some(true),
        dom_text: Some("Buy".to_owned()),
        aria_label: Some("Close".to_owned()),
        labelledby: vec![
            BrowserOsRef {
                dom_node_id: Some(41),
                unrendered_text: None,
            },
            BrowserOsRef {
                dom_node_id: None,
                unrendered_text: Some("Hidden label".to_owned()),
            },
        ],
        describedby: vec![BrowserOsRef {
            dom_node_id: Some(42),
            unrendered_text: None,
        }],
        errormessage: vec![BrowserOsRef {
            dom_node_id: Some(43),
            unrendered_text: None,
        }],
        title: Some("Tip".to_owned()),
        aria_description: Some("More detail".to_owned()),
        checked: Some(i32::from(BrowserOsTriState::Mixed)),
        pressed: Some(i32::from(BrowserOsTriState::True)),
        expanded: Some(i32::from(BrowserOsTriState::False)),
        selected: Some(true),
        level: Some(3),
        disabled: Some(true),
        aria_disabled: Some(true),
        aria_invalid: Some("spelling".to_owned()),
        redacted_value_length: Some(6),
        visibility_hidden: Some(true),
        pointer_events_none: Some(true),
        inert: Some(true),
        closed_root_seed: Some(true),
        ua_shadow: Some(true),
        display_contents: Some(true),
        svg_title: Some("Close".to_owned()),
    }
}

#[test]
fn every_browseros_message_round_trips_from_hand_encoded_bytes() -> Result<(), Box<dyn Error>> {
    // A main frame with one fully annotated node and one iframe that missed its deadline.
    let annotated_node = Wire::default().message(
        2,
        Wire::default()
            .varint(2, 7)
            .message(1000, every_node_fact_wire()),
    );
    let timed_out_iframe = Wire::default().message(
        2,
        Wire::default().varint(2, 8).varint(3, 3).message(
            9,
            Wire::default().message(
                4,
                Wire::default()
                    .varint(1, 1000)
                    .message(1000, Wire::default().string(1, "FRAME-TOKEN-HUNG")),
            ),
        ),
    );
    let root = Wire::default()
        .message(1, annotated_node)
        .message(1, timed_out_iframe)
        .message(2, Wire::default().varint(2, 1));
    let bytes = Wire::default()
        .message(2, root)
        .message(
            3,
            Wire::default().message(1000, Wire::default().string(1, "FRAME-TOKEN-MAIN")),
        )
        .message(1000, Wire::default().varint(1, 1).varint(2, 10_000))
        .0;

    let expected = AnnotatedPageContent {
        root_node: Some(ContentNode {
            children_nodes: vec![
                ContentNode {
                    children_nodes: Vec::new(),
                    content_attributes: Some(ContentAttributes {
                        common_ancestor_dom_node_id: Some(7),
                        browseros: Some(Box::new(every_node_fact())),
                        ..ContentAttributes::default()
                    }),
                },
                ContentNode {
                    children_nodes: Vec::new(),
                    content_attributes: Some(ContentAttributes {
                        common_ancestor_dom_node_id: Some(8),
                        attribute_type: Some(i32::from(ContentAttributeType::Iframe)),
                        content_data: Some(ContentData::Iframe(Box::new(IframeData {
                            data: Some(IframeDataContent::RedactedFrameMetadata(
                                RedactedFrameMetadata {
                                    reason: Some(i32::from(RedactedFrameReason::BrowserOsTimeout)),
                                    browseros: Some(BrowserOsRedactedFrame {
                                        devtools_frame_token: Some("FRAME-TOKEN-HUNG".to_owned()),
                                    }),
                                },
                            )),
                        }))),
                        ..ContentAttributes::default()
                    }),
                },
            ],
            content_attributes: Some(ContentAttributes {
                common_ancestor_dom_node_id: Some(1),
                ..ContentAttributes::default()
            }),
        }),
        main_frame_data: Some(FrameData {
            browseros: Some(BrowserOsFrameData {
                devtools_frame_token: Some("FRAME-TOKEN-MAIN".to_owned()),
            }),
            ..FrameData::default()
        }),
        browseros: Some(BrowserOsPageContentData {
            contract_version: Some(1),
            subframe_timeout_ms: Some(10_000),
        }),
        ..AnnotatedPageContent::default()
    };

    let decoded = decode_protobuf(&bytes)?;
    assert_eq!(
        decoded, expected,
        "hand-encoded contract bytes decode field by field"
    );
    assert_eq!(
        decoded.encode_to_vec(),
        bytes,
        "re-encoding the decoded tree gives the wire bytes back"
    );
    assert_eq!(
        REASON_BROWSEROS_TIMEOUT, 1000,
        "the timeout reason's raw value is the contract's 1000"
    );
    Ok(())
}

/// Borrows one boolean fact's field.
type FlagField = fn(&mut BrowserOsContentAttributes) -> &mut Option<bool>;

/// Each boolean fact of section 4.1, with the struct field its tag must decode into. Set together,
/// two flags of equal value could trade tags unseen, so each is decoded alone.
const FLAGS: &[(u32, FlagField)] = &[
    (6, |facts| &mut facts.list_box),
    (7, |facts| &mut facts.has_datalist),
    (8, |facts| &mut facts.aria_hidden),
    (9, |facts| &mut facts.break_before),
    (20, |facts| &mut facts.selected),
    (22, |facts| &mut facts.disabled),
    (23, |facts| &mut facts.aria_disabled),
    (28, |facts| &mut facts.visibility_hidden),
    (29, |facts| &mut facts.pointer_events_none),
    (30, |facts| &mut facts.inert),
    (31, |facts| &mut facts.closed_root_seed),
    (32, |facts| &mut facts.ua_shadow),
    (33, |facts| &mut facts.display_contents),
];

#[test]
fn each_boolean_fact_decodes_into_its_own_field() -> Result<(), Box<dyn Error>> {
    for &(tag, field) in FLAGS {
        let bytes = Wire::default().varint(tag, 1).0;
        let mut expected = BrowserOsContentAttributes::default();
        *field(&mut expected) = Some(true);
        assert_eq!(
            BrowserOsContentAttributes::decode(bytes.as_slice())?,
            expected,
            "tag {tag} decodes into its contract field and no other"
        );
    }
    Ok(())
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// What one recorded reply carries of the contract's frame and node messages (section 4.2 to 4.4).
#[derive(Debug, Default)]
struct ContractCoverage {
    documents: usize,
    documents_with_token: usize,
    timed_out_frames: usize,
    timed_out_frames_stamped: usize,
    nodes_with_facts: usize,
}

fn has_token(token: Option<&String>) -> bool {
    token.is_some_and(|token| !token.is_empty())
}

fn contract_coverage(apc: &AnnotatedPageContent) -> ContractCoverage {
    let frame_token = |frame: &FrameData| {
        has_token(
            frame
                .browseros
                .as_ref()
                .and_then(|page| page.devtools_frame_token.as_ref()),
        )
    };
    let mut coverage = ContractCoverage::default();
    if let Some(main) = apc.main_frame_data.as_ref() {
        coverage.documents += 1;
        coverage.documents_with_token += usize::from(frame_token(main));
    }
    let mut stack: Vec<&ContentNode> = apc.root_node.iter().collect();
    while let Some(node) = stack.pop() {
        stack.extend(&node.children_nodes);
        let Some(attrs) = node.content_attributes.as_ref() else {
            continue;
        };
        coverage.nodes_with_facts += usize::from(attrs.browseros.is_some());
        let Some(ContentData::Iframe(iframe)) = attrs.content_data.as_ref() else {
            continue;
        };
        match iframe.data.as_ref() {
            Some(IframeDataContent::FrameData(frame)) => {
                coverage.documents += 1;
                coverage.documents_with_token += usize::from(frame_token(frame));
            }
            Some(IframeDataContent::RedactedFrameMetadata(redacted)) => {
                coverage.timed_out_frames += 1;
                let stamped = redacted.reason
                    == Some(i32::from(RedactedFrameReason::BrowserOsTimeout))
                    && has_token(
                        redacted
                            .browseros
                            .as_ref()
                            .and_then(|frame| frame.devtools_frame_token.as_ref()),
                    );
                coverage.timed_out_frames_stamped += usize::from(stamped);
            }
            None => {}
        }
    }
    coverage
}

#[test]
fn patched_recordings_carry_the_contract_on_every_page_and_frame() -> Result<(), Box<dyn Error>> {
    let mut checked = 0;
    let mut timed_out = 0;
    for entry in fs::read_dir(fixtures_dir())? {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "pb") {
            continue;
        }
        let apc = decode_protobuf(&fs::read(&path)?)?;
        let name = path.display();
        assert_eq!(
            apc.browseros,
            Some(BrowserOsPageContentData {
                contract_version: Some(6),
                subframe_timeout_ms: Some(2_000),
            }),
            "{name}: recorded on a contract-v6 build with the default 2000 ms subframe timeout \
             (tests/fixtures/README.md)"
        );
        let coverage = contract_coverage(&apc);
        assert_eq!(
            coverage.documents_with_token, coverage.documents,
            "{name}: every document names its CDP frame through devtools_frame_token: {coverage:?}"
        );
        assert_eq!(
            coverage.timed_out_frames_stamped, coverage.timed_out_frames,
            "{name}: a timed-out frame carries BrowserOsTimeout and its frame token: {coverage:?}"
        );
        assert!(
            coverage.nodes_with_facts > 0,
            "{name}: the page's elements carry BrowserOS node facts: {coverage:?}"
        );
        timed_out += coverage.timed_out_frames;
        checked += 1;
    }
    assert_eq!(
        checked, 16,
        "all sixteen recorded replies are checked (tests/fixtures/README.md lists them)"
    );
    assert_eq!(
        timed_out, 1,
        "only hung-subframe's [::1] frame times out, so the redacted-frame check is not vacuous"
    );
    Ok(())
}
