//! Offline fixture tests decoding the seven committed Chromium 158 `AnnotatedPageContent` `.pb`
//! captures plus synthetic forward-compatibility and large-page payloads.

#[path = "support/tree_counts.rs"]
mod tree_counts;

use std::error::Error;
use std::fs;
use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use neo_snapshot_apc::proto::{
    AnnotatedPageContentMode, AnnotatedPageContentVersion, AxRole, BROWSEROS_EXTENSION_TAG,
    BoundingRect, CoarseAutofillFieldType, ContentAttributeType, ContentAttributes, ContentData,
    ContentNode, FormControlType, FrameData, Geometry, IframeData, IframeDataContent,
    RedactionDecision, TextInfo,
};
use neo_snapshot_apc::{AnnotatedPageContent, decode, decode_protobuf, decode_reply_json};
use prost::Message as _;
use tree_counts::{TreeCounts, count_frames_and_nodes};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(format!("{name}.pb"))
}

/// Loads `name.pb` and verifies that `decode_protobuf`, `decode` (base64), and `decode_reply_json`
/// all decode to the exact same [`AnnotatedPageContent`] tree.
fn decode_fixture_all_paths(name: &str) -> Result<(Vec<u8>, AnnotatedPageContent), Box<dyn Error>> {
    let raw_bytes = fs::read(fixture_path(name))?;
    let from_proto = decode_protobuf(&raw_bytes)?;

    let b64 = STANDARD.encode(&raw_bytes);
    let from_b64 = decode(&b64)?;
    assert_eq!(
        from_b64, from_proto,
        "{name}: base64 decode must match raw protobuf decode"
    );

    let reply_json = format!(r#"{{"content":"{b64}"}}"#);
    let from_json = decode_reply_json(&reply_json)?;
    assert_eq!(
        from_json, from_proto,
        "{name}: decode_reply_json must match raw protobuf decode"
    );

    Ok((raw_bytes, from_proto))
}

fn walk_nodes(apc: &AnnotatedPageContent) -> Vec<&ContentNode> {
    let mut out = Vec::new();
    let mut stack: Vec<&ContentNode> = apc.root_node.iter().collect();
    while let Some(node) = stack.pop() {
        out.push(node);
        stack.extend(node.children_nodes.iter().rev());
    }
    out
}

fn max_tree_depth(apc: &AnnotatedPageContent) -> usize {
    let mut deepest = 0;
    let mut stack: Vec<(&ContentNode, usize)> =
        apc.root_node.iter().map(|node| (node, 1)).collect();
    while let Some((node, depth)) = stack.pop() {
        deepest = deepest.max(depth);
        for child in node.children_nodes.iter().rev() {
            stack.push((child, depth + 1));
        }
    }
    deepest
}

fn text_inside(node: &ContentNode) -> String {
    let mut parts = Vec::new();
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if let Some(attrs) = current.content_attributes.as_ref()
            && let Some(ContentData::Text(text)) = attrs.content_data.as_ref()
            && let Some(content) = text.text_content.as_ref()
        {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_owned());
            }
        }
        stack.extend(current.children_nodes.iter().rev());
    }
    parts.join(" ")
}

fn collect_all_texts(apc: &AnnotatedPageContent) -> Vec<String> {
    walk_nodes(apc)
        .into_iter()
        .filter_map(|node| node.content_attributes.as_ref())
        .filter_map(|attrs| match attrs.content_data.as_ref() {
            Some(ContentData::Text(text)) => text.text_content.as_deref(),
            _ => None,
        })
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .collect()
}

fn collect_all_labels(apc: &AnnotatedPageContent) -> Vec<String> {
    walk_nodes(apc)
        .into_iter()
        .filter_map(|node| node.content_attributes.as_ref())
        .filter_map(|attrs| attrs.label.as_deref())
        .filter(|label| !label.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let low = u8::try_from(value & 0x7f).unwrap_or(0);
        value >>= 7;
        if value == 0 {
            out.push(low);
            break;
        }
        out.push(low | 0x80);
    }
}

fn push_length_delimited_field(tag: u32, payload: &[u8], out: &mut Vec<u8>) {
    encode_varint((u64::from(tag) << 3) | 2, out);
    encode_varint(u64::try_from(payload.len()).unwrap_or(0), out);
    out.extend_from_slice(payload);
}

#[test]
fn nested_cross_site_frames_fixture_decodes_all_eight_frames() -> Result<(), Box<dyn Error>> {
    let (_, apc) = decode_fixture_all_paths("nested-cross-site-frames")?;
    assert_eq!(
        apc.version,
        Some(i32::from(AnnotatedPageContentVersion::Version10)),
        "expected APC version 1.0"
    );
    assert_eq!(
        apc.mode,
        Some(i32::from(AnnotatedPageContentMode::ActionableElements)),
        "expected ActionableElements extraction mode"
    );
    assert_eq!(
        count_frames_and_nodes(&apc),
        TreeCounts {
            frames: 8,
            nodes: 71,
        },
        "nested-cross-site-frames.pb must decode 8 frames and 71 nodes"
    );

    let texts = collect_all_texts(&apc);
    let labels = collect_all_labels(&apc);
    for frame in ["Top", "A1", "B2", "B3", "B1", "C2", "A3", "C4"] {
        assert!(
            texts.iter().any(|text| text == frame),
            "missing heading text {frame:?}"
        );
        let button_text = format!("{frame} button");
        assert!(
            texts.iter().any(|text| text == &button_text),
            "missing button text {button_text:?}"
        );
        let field_label = format!("{frame} field");
        assert!(
            labels.iter().any(|label| label == &field_label),
            "missing input label {field_label:?}"
        );
    }

    let iframe_titles: Vec<&str> = walk_nodes(&apc)
        .into_iter()
        .filter_map(|node| node.content_attributes.as_ref())
        .filter_map(|attrs| match attrs.content_data.as_ref() {
            Some(ContentData::Iframe(iframe)) => match iframe.data.as_ref() {
                Some(IframeDataContent::FrameData(frame)) => frame.title.as_deref(),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(
        iframe_titles,
        vec!["A1", "B2", "B3", "B1", "C2", "A3", "C4"],
        "all 7 nested iframes must carry populated FrameData in tree order"
    );
    Ok(())
}

#[test]
fn shadow_dom_fixture_decodes_light_open_closed_and_slotted_nodes() -> Result<(), Box<dyn Error>> {
    let (_, apc) = decode_fixture_all_paths("shadow-dom")?;
    assert_eq!(
        count_frames_and_nodes(&apc),
        TreeCounts {
            frames: 1,
            nodes: 23,
        },
        "shadow-dom.pb must decode 1 frame and 23 nodes"
    );

    let texts = collect_all_texts(&apc);
    let labels = collect_all_labels(&apc);
    for expected_text in [
        "Shadow DOM Fixture",
        "Light button",
        "Open shadow button",
        "Open slotted text",
        "Closed shadow button",
        "Closed slotted text",
    ] {
        assert!(
            texts.iter().any(|text| text == expected_text),
            "missing shadow-dom text {expected_text:?} in {texts:?}"
        );
    }
    for expected_label in ["Open shadow input", "Closed shadow input"] {
        assert!(
            labels.iter().any(|label| label == expected_label),
            "missing shadow-dom input label {expected_label:?} in {labels:?}"
        );
    }
    Ok(())
}

#[test]
fn zero_size_checkbox_fixture_preserves_zero_boxes_and_label_for_links()
-> Result<(), Box<dyn Error>> {
    let (_, apc) = decode_fixture_all_paths("zero-size-checkbox")?;
    assert_eq!(
        count_frames_and_nodes(&apc),
        TreeCounts {
            frames: 1,
            nodes: 13,
        },
        "zero-size-checkbox.pb must decode 1 frame and 13 nodes"
    );

    let nodes = walk_nodes(&apc);
    let checkboxes: Vec<&ContentAttributes> = nodes
        .iter()
        .filter_map(|node| node.content_attributes.as_ref())
        .filter(|attrs| {
            matches!(
                attrs.content_data.as_ref(),
                Some(ContentData::FormControl(control))
                    if control.form_control_type == Some(i32::from(FormControlType::InputCheckbox))
            )
        })
        .collect();
    assert_eq!(
        checkboxes.len(),
        2,
        "expected both 0x0 checkboxes in zero-size-checkbox.pb"
    );

    let mut checked_states = Vec::new();
    for checkbox in &checkboxes {
        let dom_id = checkbox
            .common_ancestor_dom_node_id
            .ok_or("checkbox missing common_ancestor_dom_node_id")?;
        let geometry = checkbox
            .geometry
            .as_ref()
            .ok_or("checkbox missing geometry")?;
        let outer = geometry
            .outer_bounding_box
            .ok_or("checkbox missing outer_bounding_box")?;
        let visible = geometry
            .visible_bounding_box
            .ok_or("checkbox missing visible_bounding_box")?;
        assert_eq!(
            (outer.width, outer.height),
            (Some(0), Some(0)),
            "0x0 checkbox outer_bounding_box must be 0x0"
        );
        assert_eq!(
            (visible.width, visible.height),
            (Some(0), Some(0)),
            "0x0 checkbox visible_bounding_box must be 0x0"
        );

        let paired_label = nodes
            .iter()
            .find(|node| {
                node.content_attributes
                    .as_ref()
                    .and_then(|attrs| attrs.label_for_dom_node_id)
                    == Some(dom_id)
            })
            .ok_or("expected a <label> node whose label_for_dom_node_id matches checkbox")?;
        let label_text = text_inside(paired_label);
        assert!(
            label_text == "Accept terms" || label_text == "Subscribe newsletter",
            "unexpected paired label text {label_text:?}"
        );

        if let Some(ContentData::FormControl(control)) = checkbox.content_data.as_ref() {
            checked_states.push((label_text, control.is_checked));
        }
    }

    assert_eq!(
        checked_states,
        vec![
            ("Accept terms".to_owned(), Some(false)),
            ("Subscribe newsletter".to_owned(), Some(true)),
        ],
        "checkbox checked states must match fixture markup"
    );
    Ok(())
}

#[test]
fn covered_button_fixture_decodes_dialog_and_higher_overlay_z_order() -> Result<(), Box<dyn Error>>
{
    let (_, apc) = decode_fixture_all_paths("covered-button")?;
    assert_eq!(
        count_frames_and_nodes(&apc),
        TreeCounts {
            frames: 1,
            nodes: 16,
        },
        "covered-button.pb must decode 1 frame and 16 nodes"
    );

    let nodes = walk_nodes(&apc);
    let buy_button = nodes
        .iter()
        .find(|node| {
            node.content_attributes.as_ref().is_some_and(|attrs| {
                attrs.attribute_type == Some(i32::from(ContentAttributeType::FormControl))
            }) && text_inside(node) == "Buy"
        })
        .ok_or("missing Buy button node")?;
    let buy_z = buy_button
        .content_attributes
        .as_ref()
        .and_then(|attrs| attrs.interaction_info.as_ref())
        .and_then(|info| info.document_scoped_z_order)
        .ok_or("Buy button missing document_scoped_z_order")?;

    let dialog = nodes
        .iter()
        .find(|node| {
            node.content_attributes.as_ref().is_some_and(|attrs| {
                attrs.aria_role == Some(i32::from(AxRole::Dialog))
                    && attrs.label.as_deref() == Some("What You Need To Know")
            })
        })
        .ok_or("missing modal dialog node")?;
    let dialog_attrs = dialog
        .content_attributes
        .as_ref()
        .ok_or("dialog missing content_attributes")?;
    assert_eq!(
        dialog_attrs.attribute_type,
        Some(i32::from(ContentAttributeType::DialogModeless)),
        "div[role=dialog] without aria-modal=true decodes as DialogModeless in Chromium 158"
    );
    let dialog_z = dialog_attrs
        .interaction_info
        .as_ref()
        .and_then(|info| info.document_scoped_z_order)
        .ok_or("dialog missing document_scoped_z_order")?;
    assert!(
        dialog_z > buy_z,
        "covering dialog z-order ({dialog_z}) must exceed occluded Buy button z-order ({buy_z})"
    );

    let texts = collect_all_texts(&apc);
    for expected in [
        "Buy",
        "Resale prices often exceed face value.",
        "Accept & Continue",
        "Dismiss",
    ] {
        assert!(
            texts.iter().any(|text| text == expected),
            "missing covered-button text {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn sensitive_form_fixture_redacts_password_and_classifies_card_and_otp()
-> Result<(), Box<dyn Error>> {
    let (raw_bytes, apc) = decode_fixture_all_paths("sensitive-form")?;
    assert_eq!(
        count_frames_and_nodes(&apc),
        TreeCounts {
            frames: 1,
            nodes: 24,
        },
        "sensitive-form.pb must decode 1 frame and 24 nodes"
    );

    assert!(
        !raw_bytes
            .windows(b"example-pass-value".len())
            .any(|window| window == b"example-pass-value"),
        "password dummy value must be redacted out of wire protobuf bytes"
    );

    let nodes = walk_nodes(&apc);
    let label_for = |target_dom_id: i32| -> Option<String> {
        nodes.iter().find_map(|node| {
            let attrs = node.content_attributes.as_ref()?;
            (attrs.label_for_dom_node_id == Some(target_dom_id)).then(|| text_inside(node))
        })
    };

    let mut found_password = false;
    let mut found_card = false;
    let mut found_otp = false;
    let mut found_submit = false;

    for node in &nodes {
        let Some(attrs) = node.content_attributes.as_ref() else {
            continue;
        };
        let Some(ContentData::FormControl(control)) = attrs.content_data.as_ref() else {
            continue;
        };
        let dom_id = attrs.common_ancestor_dom_node_id.unwrap_or_default();
        match label_for(dom_id).as_deref() {
            Some("Password") => {
                found_password = true;
                assert_eq!(
                    control.form_control_type,
                    Some(i32::from(FormControlType::InputPassword))
                );
                assert_eq!(
                    attrs.redaction_decision,
                    Some(i32::from(RedactionDecision::RedactedHasBeenPassword)),
                    "password input must carry RedactedHasBeenPassword on ContentAttributes"
                );
                assert_eq!(
                    control.redaction_decision,
                    Some(i32::from(RedactionDecision::RedactedHasBeenPassword)),
                    "password input must carry RedactedHasBeenPassword on FormControlData"
                );
                assert_eq!(
                    control.field_value, None,
                    "redacted password input must omit field_value"
                );
            }
            Some("Card number") => {
                found_card = true;
                assert_eq!(
                    control.coarse_autofill_field_type,
                    vec![i32::from(CoarseAutofillFieldType::CreditCard)],
                    "card number field must carry CoarseAutofillFieldType::CreditCard"
                );
            }
            Some("One-time code") => {
                found_otp = true;
                assert_eq!(
                    control.coarse_autofill_field_type,
                    vec![i32::from(CoarseAutofillFieldType::Otp)],
                    "OTP field must carry CoarseAutofillFieldType::Otp"
                );
            }
            None if text_inside(node) == "Pay now" => {
                found_submit = true;
                assert_eq!(
                    control.form_control_type,
                    Some(i32::from(FormControlType::ButtonSubmit))
                );
            }
            _ => {}
        }
    }

    assert!(
        found_password && found_card && found_otp && found_submit,
        "expected Password, Card number, One-time code, and Pay now controls"
    );
    Ok(())
}

#[test]
fn hung_subframe_fixture_preserves_healthy_sibling_and_empty_hung_iframe()
-> Result<(), Box<dyn Error>> {
    let (_, apc) = decode_fixture_all_paths("hung-subframe")?;
    assert_eq!(
        count_frames_and_nodes(&apc),
        TreeCounts {
            frames: 3,
            nodes: 16,
        },
        "hung-subframe.pb must decode 3 frames (top + healthy + hung) and 16 nodes"
    );

    let texts = collect_all_texts(&apc);
    assert!(
        texts.iter().any(|text| text == "Top button"),
        "main frame Top button must survive hung sibling"
    );
    assert!(
        texts.iter().any(|text| text == "Healthy frame button"),
        "healthy cross-site iframe content must survive hung sibling"
    );
    assert!(
        !texts.iter().any(|text| text == "Hung frame button"),
        "spinning hung subframe must not have populated child text"
    );

    let iframes: Vec<&ContentNode> = walk_nodes(&apc)
        .into_iter()
        .filter(|node| {
            node.content_attributes
                .as_ref()
                .is_some_and(|attrs| matches!(attrs.content_data, Some(ContentData::Iframe(_))))
        })
        .collect();
    assert_eq!(iframes.len(), 2, "expected healthy and hung iframe nodes");

    let healthy = iframes
        .iter()
        .find(|node| {
            node.content_attributes
                .as_ref()
                .and_then(|attrs| attrs.label.as_deref())
                == Some("Healthy frame")
        })
        .ok_or("missing Healthy frame iframe node")?;
    match healthy
        .content_attributes
        .as_ref()
        .and_then(|attrs| attrs.content_data.as_ref())
    {
        Some(ContentData::Iframe(iframe)) => match iframe.data.as_ref() {
            Some(IframeDataContent::FrameData(frame)) => {
                assert_eq!(frame.title.as_deref(), Some("Healthy Frame"));
                assert!(
                    frame
                        .url
                        .as_deref()
                        .is_some_and(|url| url.ends_with("/hung-subframe/healthy.html")),
                    "unexpected healthy iframe URL: {:?}",
                    frame.url
                );
            }
            other => panic!("expected FrameData on healthy iframe, got {other:?}"),
        },
        other => panic!("expected ContentData::Iframe on healthy iframe, got {other:?}"),
    }
    assert!(
        !healthy.children_nodes.is_empty(),
        "healthy iframe must contain its decoded subtree"
    );

    let hung = iframes
        .iter()
        .find(|node| {
            node.content_attributes
                .as_ref()
                .and_then(|attrs| attrs.label.as_deref())
                == Some("Hung frame")
        })
        .ok_or("missing Hung frame iframe node")?;
    match hung
        .content_attributes
        .as_ref()
        .and_then(|attrs| attrs.content_data.as_ref())
    {
        Some(ContentData::Iframe(iframe)) => {
            assert_eq!(
                iframe.data, None,
                "stock Chromium 158 emits empty IframeData.data when a subframe times out"
            );
        }
        other => panic!("expected ContentData::Iframe on hung iframe, got {other:?}"),
    }
    assert!(
        hung.children_nodes.is_empty(),
        "timed-out hung iframe must have zero child content nodes"
    );
    Ok(())
}

#[test]
fn deep_nesting_fixture_exceeds_prost_default_limit_and_decodes_cleanly()
-> Result<(), Box<dyn Error>> {
    let (raw_bytes, apc) = decode_fixture_all_paths("deep-nesting")?;
    assert!(
        AnnotatedPageContent::decode(raw_bytes.as_slice()).is_err(),
        "real Chromium 158 deep-nesting.pb exceeds prost's default 100-level recursion limit"
    );
    assert_eq!(
        count_frames_and_nodes(&apc),
        TreeCounts {
            frames: 1,
            nodes: 127,
        },
        "deep-nesting.pb must decode 1 frame and 127 nodes"
    );
    let depth = max_tree_depth(&apc);
    assert!(
        depth > 100,
        "deep-nesting.pb tree depth ({depth}) must exceed prost's 100-level limit"
    );

    let texts = collect_all_texts(&apc);
    for expected in ["Deep Nesting Fixture", "Deep nested button"] {
        assert!(
            texts.iter().any(|text| text == expected),
            "missing deep-nesting text {expected:?} in {texts:?}"
        );
    }
    Ok(())
}

#[test]
fn browseros_contract_on_real_fixture_decodes_beside_stock_content() -> Result<(), Box<dyn Error>> {
    let raw_bytes = fs::read(fixture_path("nested-cross-site-frames"))?;
    let baseline = decode_protobuf(&raw_bytes)?;
    assert_eq!(
        baseline.browseros, None,
        "stock Chromium 158 sends no BrowserOS contract, so contract_version is absent"
    );

    let mut extended = raw_bytes;
    // BrowserOSPageContentData { contract_version: 1 }: field 1, varint 1.
    push_length_delimited_field(BROWSEROS_EXTENSION_TAG, &[0x08, 0x01], &mut extended);

    let mut decoded = decode_protobuf(&extended)?;
    assert_eq!(
        decoded
            .browseros
            .take()
            .and_then(|page| page.contract_version),
        Some(1),
        "a contract submessage appended to a real Chromium 158 reply must decode"
    );
    assert_eq!(
        decoded, baseline,
        "the contract submessage must leave every stock field unchanged"
    );
    Ok(())
}

fn make_synthetic_item(next_id: &mut i32, idx: usize) -> ContentNode {
    let container_id = *next_id;
    let text_id = *next_id + 1;
    *next_id += 2;
    ContentNode {
        children_nodes: vec![ContentNode {
            children_nodes: Vec::new(),
            content_attributes: Some(ContentAttributes {
                common_ancestor_dom_node_id: Some(text_id),
                attribute_type: Some(i32::from(ContentAttributeType::Text)),
                content_data: Some(ContentData::Text(TextInfo {
                    text_content: Some(format!("Synthetic item #{idx}")),
                    text_style: None,
                })),
                ..ContentAttributes::default()
            }),
        }],
        content_attributes: Some(ContentAttributes {
            common_ancestor_dom_node_id: Some(container_id),
            attribute_type: Some(i32::from(ContentAttributeType::Paragraph)),
            geometry: Some(Geometry {
                outer_bounding_box: Some(BoundingRect {
                    x: Some(0),
                    y: Some(i32::try_from(idx % 1_000).unwrap_or(0) * 24),
                    width: Some(320),
                    height: Some(20),
                    is_screenshot_relative: Some(false),
                }),
                ..Geometry::default()
            }),
            ..ContentAttributes::default()
        }),
    }
}

#[test]
fn generated_large_page_fixture_decodes_cleanly() -> Result<(), Box<dyn Error>> {
    const IFRAMES: usize = 4;
    const ITEMS_PER_FRAME: usize = 400;
    const MAIN_ITEMS: usize = 3_400;

    let mut next_id = 2_i32;
    let mut root_children = Vec::with_capacity(IFRAMES + MAIN_ITEMS);
    for frame_idx in 0..IFRAMES {
        let iframe_id = next_id;
        let frame_root_id = next_id + 1;
        next_id += 2;
        let mut frame_items = Vec::with_capacity(ITEMS_PER_FRAME);
        for item_idx in 0..ITEMS_PER_FRAME {
            frame_items.push(make_synthetic_item(
                &mut next_id,
                frame_idx * ITEMS_PER_FRAME + item_idx,
            ));
        }
        root_children.push(ContentNode {
            children_nodes: vec![ContentNode {
                children_nodes: frame_items,
                content_attributes: Some(ContentAttributes {
                    common_ancestor_dom_node_id: Some(frame_root_id),
                    attribute_type: Some(i32::from(ContentAttributeType::Root)),
                    ..ContentAttributes::default()
                }),
            }],
            content_attributes: Some(ContentAttributes {
                common_ancestor_dom_node_id: Some(iframe_id),
                attribute_type: Some(i32::from(ContentAttributeType::Iframe)),
                content_data: Some(ContentData::Iframe(Box::new(IframeData {
                    data: Some(IframeDataContent::FrameData(Box::new(FrameData {
                        url: Some(format!("https://frame-{frame_idx}.example.test/")),
                        title: Some(format!("Frame {frame_idx}")),
                        ..FrameData::default()
                    }))),
                }))),
                ..ContentAttributes::default()
            }),
        });
    }
    for item_idx in 0..MAIN_ITEMS {
        root_children.push(make_synthetic_item(
            &mut next_id,
            IFRAMES * ITEMS_PER_FRAME + item_idx,
        ));
    }

    let apc = AnnotatedPageContent {
        version: Some(i32::from(AnnotatedPageContentVersion::Version10)),
        mode: Some(i32::from(AnnotatedPageContentMode::ActionableElements)),
        root_node: Some(ContentNode {
            children_nodes: root_children,
            content_attributes: Some(ContentAttributes {
                common_ancestor_dom_node_id: Some(1),
                attribute_type: Some(i32::from(ContentAttributeType::Root)),
                ..ContentAttributes::default()
            }),
        }),
        main_frame_data: Some(FrameData {
            url: Some("https://large.example.test/".to_owned()),
            title: Some("Synthetic Large Page".to_owned()),
            ..FrameData::default()
        }),
        ..AnnotatedPageContent::default()
    };

    // 1 root + 4 * (1 iframe + 1 frame_root + 400 * 2 item nodes) + 3400 * 2 main item nodes = 10,009 nodes.
    let expected_frames = 1 + IFRAMES;
    let expected_nodes = 1 + IFRAMES * (2 + ITEMS_PER_FRAME * 2) + MAIN_ITEMS * 2;
    assert_eq!(expected_frames, 5);
    assert_eq!(expected_nodes, 10_009);

    let b64 = STANDARD.encode(apc.encode_to_vec());
    let reply_json = format!(r#"{{"content":"{b64}"}}"#);
    let decoded = decode_reply_json(&reply_json)?;
    assert_eq!(
        count_frames_and_nodes(&decoded),
        TreeCounts {
            frames: expected_frames,
            nodes: expected_nodes,
        },
        "generated ~10k-node multi-frame payload must decode with exact frame and node counts"
    );
    Ok(())
}
