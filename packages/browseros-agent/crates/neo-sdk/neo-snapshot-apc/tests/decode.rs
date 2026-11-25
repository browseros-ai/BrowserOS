//! End-to-end decoder and Edition 2023 wire-compatibility tests for `neo-snapshot-apc`.

use std::error::Error;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use neo_snapshot_apc::proto::{
    AnchorData, AnchorRel, AnnotatedPageContentMode, AnnotatedPageContentVersion, AnnotatedRole,
    AutofillInformation, AxRole, BROWSEROS_EXTENSION_TAG, BoundingRect, BoundingSize, CanvasData,
    ClickabilityReason, CoarseAutofillFieldType, ContentAttributeType, ContentAttributes,
    ContentData, ContentNode, CssPosition, DocumentIdentifier, FillableData, FormControlData,
    FormControlType, FormInfo, FrameData, FrameInteractionInfo, GeminiInChromePageMetadata,
    Geometry, IframeData, IframeDataContent, IframeInfo, ImageInfo, InteractionDisabledReason,
    InteractionInfo, MathData, MediaData, MediaDataType, MediaTranscript, PageInteractionInfo,
    PaidContentMetadata, Point, PopupWindow, ProfileInformation, REASON_BROWSEROS_TIMEOUT,
    RedactedFrameMetadata, RedactedFrameReason, RedactionDecision, ScreenshotInfo, ScriptTool,
    ScriptToolAnnotations, ScriptToolResult, ScrollerInfo, SecurityOrigin, SelectOption, Selection,
    SvgRootData, TableData, TableRowData, TableRowType, TextInfo, TextSize, TextStyle, VideoData,
};
use neo_snapshot_apc::{
    AnnotatedPageContent, DecodeError, MAX_CONTENT_NODE_DEPTH, decode, decode_protobuf,
    decode_reply_json,
};
use prost::Message as _;

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

fn push_varint_field(tag: u32, value: u64, out: &mut Vec<u8>) {
    encode_varint(u64::from(tag) << 3, out);
    encode_varint(value, out);
}

fn node_with_data(
    dom_id: i32,
    attribute_type: ContentAttributeType,
    data: ContentData,
    children: Vec<ContentNode>,
) -> ContentNode {
    ContentNode {
        children_nodes: children,
        content_attributes: Some(ContentAttributes {
            common_ancestor_dom_node_id: Some(dom_id),
            attribute_type: Some(i32::from(attribute_type)),
            content_data: Some(data),
            ..ContentAttributes::default()
        }),
    }
}

fn build_form_control_node() -> ContentNode {
    ContentNode {
        children_nodes: Vec::new(),
        content_attributes: Some(ContentAttributes {
            common_ancestor_dom_node_id: Some(16),
            attribute_type: Some(i32::from(ContentAttributeType::FormControl)),
            geometry: Some(Geometry {
                outer_bounding_box: Some(BoundingRect {
                    x: Some(10),
                    y: Some(-5),
                    width: Some(200),
                    height: Some(32),
                    is_screenshot_relative: Some(false),
                }),
                visible_bounding_box: Some(BoundingRect {
                    x: Some(10),
                    y: Some(0),
                    width: Some(200),
                    height: Some(27),
                    is_screenshot_relative: Some(false),
                }),
                is_fixed_or_sticky_position: Some(false),
                fragment_visible_bounding_boxes: vec![BoundingRect {
                    x: Some(10),
                    y: Some(0),
                    width: Some(200),
                    height: Some(27),
                    is_screenshot_relative: Some(false),
                }],
                css_position: Some(i32::from(CssPosition::Relative)),
            }),
            content_data: Some(ContentData::FormControl(Box::new(FormControlData {
                field_name: Some("bio".to_owned()),
                field_value: Some("hello".to_owned()),
                form_control_type: Some(i32::from(FormControlType::TextArea)),
                select_options: vec![SelectOption {
                    value: Some("a".to_owned()),
                    text: Some("Alpha".to_owned()),
                    is_selected: Some(true),
                    is_disabled: Some(false),
                }],
                placeholder: Some("Write bio".to_owned()),
                is_checked: Some(false),
                is_required: Some(true),
                redaction_decision: Some(i32::from(RedactionDecision::NoRedactionNecessary)),
                coarse_autofill_field_type: vec![
                    i32::from(CoarseAutofillFieldType::Address),
                    i32::from(CoarseAutofillFieldType::Otp),
                ],
                autofill_section_id: Some(3),
                is_readonly: Some(false),
            }))),
            annotated_roles: vec![i32::from(AnnotatedRole::Main)],
            label: Some("Biography".to_owned()),
            interaction_info: Some(InteractionInfo {
                is_selectable: Some(true),
                is_editable: Some(true),
                can_resize_horizontal: Some(false),
                can_resize_vertical: Some(true),
                is_focusable: Some(true),
                is_draggable: Some(false),
                is_clickable: Some(true),
                scroller_info: Some(ScrollerInfo {
                    scrolling_bounds: Some(BoundingSize {
                        width: Some(200),
                        height: Some(400),
                    }),
                    visible_area: Some(BoundingRect {
                        x: Some(0),
                        y: Some(20),
                        width: Some(200),
                        height: Some(32),
                        is_screenshot_relative: Some(false),
                    }),
                    user_scrollable_horizontal: Some(false),
                    user_scrollable_vertical: Some(true),
                }),
                document_scoped_z_order: Some(4),
                debug_clickability_reasons: vec![i32::from(ClickabilityReason::ClickableControl)],
                clickability_reasons: vec![
                    i32::from(ClickabilityReason::ClickableControl),
                    i32::from(ClickabilityReason::Editable),
                    i32::from(ClickabilityReason::MouseEnter),
                ],
                is_disabled: Some(false),
                interaction_disabled_reasons: vec![i32::from(
                    InteractionDisabledReason::AriaRolePresentational,
                )],
                is_tabbable: Some(true),
                has_aria_activedescendant: Some(false),
                aria_action_target_node_ids: vec![17, 18],
            }),
            label_for_dom_node_id: Some(16),
            aria_role: Some(i32::from(AxRole::TextField)),
            is_ad_related: Some(false),
            redaction_decision: Some(i32::from(RedactionDecision::NoRedactionNecessary)),
        }),
    }
}

fn build_media_and_text_children() -> Vec<ContentNode> {
    let text_node = node_with_data(
        10,
        ContentAttributeType::Text,
        ContentData::Text(TextInfo {
            text_content: Some("Hello world".to_owned()),
            text_style: Some(TextStyle {
                text_size: Some(i32::from(TextSize::L)),
                has_emphasis: Some(true),
                color: Some(0x1122_33ff),
            }),
        }),
        Vec::new(),
    );

    let image_node = node_with_data(
        11,
        ContentAttributeType::Image,
        ContentData::Image(ImageInfo {
            image_caption: Some("Hero banner".to_owned()),
            security_origin: Some(SecurityOrigin {
                opaque: Some(false),
                value: Some("https://cdn.example.com".to_owned()),
            }),
            url: Some("https://cdn.example.com/hero.png".to_owned()),
        }),
        Vec::new(),
    );

    let svg_node = node_with_data(
        12,
        ContentAttributeType::SvgRoot,
        ContentData::SvgRoot(SvgRootData {
            inner_text: Some("Chart title".to_owned()),
        }),
        Vec::new(),
    );

    let canvas_node = node_with_data(
        13,
        ContentAttributeType::Canvas,
        ContentData::Canvas(CanvasData {
            layout_width: Some(640),
            layout_height: Some(480),
        }),
        Vec::new(),
    );

    let video_node = node_with_data(
        14,
        ContentAttributeType::Video,
        ContentData::Video(VideoData {
            url: Some("https://cdn.example.com/clip.mp4".to_owned()),
            security_origin: Some(SecurityOrigin {
                opaque: Some(false),
                value: Some("https://cdn.example.com".to_owned()),
            }),
        }),
        Vec::new(),
    );

    let anchor_node = node_with_data(
        15,
        ContentAttributeType::Anchor,
        ContentData::Anchor(AnchorData {
            url: Some("https://example.com/privacy".to_owned()),
            rel: vec![
                i32::from(AnchorRel::NoOpener),
                i32::from(AnchorRel::PrivacyPolicy),
            ],
        }),
        vec![text_node],
    );

    vec![image_node, svg_node, canvas_node, video_node, anchor_node]
}

fn build_structured_and_iframe_children() -> Vec<ContentNode> {
    let form_node = node_with_data(
        19,
        ContentAttributeType::Form,
        ContentData::Form(FormInfo {
            form_name: Some("profile".to_owned()),
            action_url: Some("https://example.com/save".to_owned()),
        }),
        vec![build_form_control_node()],
    );

    let table_row_node = node_with_data(
        20,
        ContentAttributeType::TableRow,
        ContentData::TableRow(TableRowData {
            row_type: Some(i32::from(TableRowType::Header)),
        }),
        Vec::new(),
    );

    let table_node = node_with_data(
        21,
        ContentAttributeType::Table,
        ContentData::Table(TableData {
            table_name: Some("Metrics".to_owned()),
        }),
        vec![table_row_node],
    );

    let math_node = node_with_data(
        22,
        ContentAttributeType::Math,
        ContentData::Math(MathData {
            stripped_mathml: Some("<math><mi>x</mi></math>".to_owned()),
            is_block: Some(true),
        }),
        Vec::new(),
    );

    let included_iframe = node_with_data(
        23,
        ContentAttributeType::Iframe,
        ContentData::Iframe(Box::new(IframeData {
            data: Some(IframeDataContent::FrameData(Box::new(FrameData {
                url: Some("https://example.com/embed".to_owned()),
                title: Some("Embed".to_owned()),
                default_line_height_px: Some(18),
                ..FrameData::default()
            }))),
        })),
        Vec::new(),
    );

    let redacted_iframe = node_with_data(
        24,
        ContentAttributeType::Iframe,
        ContentData::Iframe(Box::new(IframeData {
            data: Some(IframeDataContent::RedactedFrameMetadata(
                RedactedFrameMetadata {
                    reason: Some(i32::from(RedactedFrameReason::CrossSite)),
                },
            )),
        })),
        Vec::new(),
    );

    vec![
        form_node,
        table_node,
        math_node,
        included_iframe,
        redacted_iframe,
    ]
}

fn build_root_children() -> Vec<ContentNode> {
    let mut children = build_media_and_text_children();
    children.extend(build_structured_and_iframe_children());
    children
}

fn build_main_frame_data() -> FrameData {
    let tool = ScriptTool {
        name: Some("search_docs".to_owned()),
        description: Some("Searches page docs".to_owned()),
        input_schema: Some(r#"{"type":"object"}"#.to_owned()),
        annotations: Some(ScriptToolAnnotations {
            read_only: Some(true),
        }),
    };

    FrameData {
        security_origin: Some(SecurityOrigin {
            opaque: Some(false),
            value: Some("https://example.com".to_owned()),
        }),
        frame_interaction_info: Some(FrameInteractionInfo {
            selection: Some(Selection {
                start_node_id: Some(10),
                start_offset: Some(0),
                end_node_id: Some(10),
                end_offset: Some(5),
                selected_text: Some("Hello".to_owned()),
            }),
            focused_node_id: Some(16),
            accessibility_focused_node_id: Some(16),
        }),
        document_identifier: Some(DocumentIdentifier {
            serialized_token: Some("doc-token-main".to_owned()),
        }),
        url: Some("https://example.com/".to_owned()),
        title: Some("Example".to_owned()),
        paid_content_metadata: Some(PaidContentMetadata {
            contains_paid_content: Some(false),
        }),
        media_data: Some(MediaData {
            media_data_type: Some(i32::from(MediaDataType::Video)),
            transcripts: vec![MediaTranscript {
                text: Some("Welcome".to_owned()),
                start_timestamp_milliseconds: Some(1_250),
            }],
            duration_milliseconds: Some(60_000),
            current_position_milliseconds: Some(1_250),
            is_playing: Some(true),
            title: Some("Demo".to_owned()),
            artist: Some("Team".to_owned()),
            album: Some("Launch".to_owned()),
        }),
        script_tools: vec![tool.clone()],
        script_tool_results: vec![ScriptToolResult {
            index_of_script_tool_action: Some(0),
            result: Some("ok".to_owned()),
            tool_name: Some("search_docs".to_owned()),
            input_arguments: Some(r#"{"q":"apc"}"#.to_owned()),
            tool: Some(tool),
        }],
        default_line_height_px: Some(20),
    }
}

fn build_full_apc() -> AnnotatedPageContent {
    AnnotatedPageContent {
        version: Some(i32::from(AnnotatedPageContentVersion::Version10)),
        mode: Some(i32::from(AnnotatedPageContentMode::ActionableElements)),
        root_node: Some(ContentNode {
            children_nodes: build_root_children(),
            content_attributes: Some(ContentAttributes {
                common_ancestor_dom_node_id: Some(1),
                attribute_type: Some(i32::from(ContentAttributeType::Root)),
                aria_role: Some(i32::from(AxRole::RootWebArea)),
                ..ContentAttributes::default()
            }),
        }),
        main_frame_data: Some(build_main_frame_data()),
        page_interaction_info: Some(PageInteractionInfo {
            focused_node_id: Some(16),
            accessibility_focused_node_id: Some(16),
            mouse_position: Some(Point {
                x: Some(120),
                y: Some(240),
            }),
            focused_frame: Some(DocumentIdentifier {
                serialized_token: Some("doc-token-main".to_owned()),
            }),
            accessibility_focused_frame: Some(DocumentIdentifier {
                serialized_token: Some(String::new()),
            }),
        }),
        viewport_geometry: Some(BoundingRect {
            x: Some(0),
            y: Some(0),
            width: Some(1280),
            height: Some(720),
            is_screenshot_relative: Some(false),
        }),
        tab_id: Some(42),
        popup_window: Some(PopupWindow {
            root_node: Some(ContentNode {
                children_nodes: Vec::new(),
                content_attributes: Some(ContentAttributes {
                    common_ancestor_dom_node_id: Some(99),
                    attribute_type: Some(i32::from(ContentAttributeType::Container)),
                    aria_role: Some(i32::from(AxRole::MenuItemSeparator)),
                    ..ContentAttributes::default()
                }),
            }),
            opener_document_id: Some(DocumentIdentifier {
                serialized_token: Some("doc-token-main".to_owned()),
            }),
            opener_common_ancestor_dom_node_id: Some(16),
            visible_bounding_box: Some(BoundingRect {
                x: Some(10),
                y: Some(32),
                width: Some(200),
                height: Some(120),
                is_screenshot_relative: Some(false),
            }),
        }),
        profile_information: Some(ProfileInformation {
            autofill_information: Some(AutofillInformation {
                fillable_data: vec![
                    i32::from(FillableData::Address),
                    i32::from(FillableData::CreditCard),
                ],
            }),
        }),
        gemini_in_chrome_page_metadata: Some(GeminiInChromePageMetadata {
            screenshot_info: Some(ScreenshotInfo {
                has_selection_region_in_screenshot: Some(true),
                iframe_info: vec![IframeInfo {
                    security_origin: Some(SecurityOrigin {
                        opaque: Some(true),
                        value: Some("opaque-nonce-1".to_owned()),
                    }),
                    url: Some("about:blank".to_owned()),
                    bounding_box: Some(BoundingRect {
                        x: Some(0),
                        y: Some(0),
                        width: Some(300),
                        height: Some(150),
                        is_screenshot_relative: Some(true),
                    }),
                }],
                screenshot_size: Some(BoundingSize {
                    width: Some(2560),
                    height: Some(1440),
                }),
            }),
        }),
    }
}

#[test]
fn empty_payload_decodes_to_all_absent_fields() -> Result<(), Box<dyn Error>> {
    let from_bytes = decode_protobuf(&[])?;
    let from_b64 = decode("")?;
    let from_json = decode_reply_json(r#"{"content":""}"#)?;
    assert_eq!(
        from_bytes,
        AnnotatedPageContent::default(),
        "empty protobuf bytes should leave every explicit-presence field unset"
    );
    assert_eq!(
        from_b64, from_bytes,
        "empty base64 string should decode identically to empty bytes"
    );
    assert_eq!(
        from_json, from_bytes,
        "empty JSON reply content should decode identically to empty bytes"
    );
    assert_eq!(
        from_bytes.version, None,
        "absent Edition 2023 enum field must stay None rather than defaulting to Some(0)"
    );
    Ok(())
}

#[test]
fn full_apc_round_trips_through_all_three_entry_points() -> Result<(), Box<dyn Error>> {
    let original = build_full_apc();
    let wire = original.encode_to_vec();
    let b64 = STANDARD.encode(&wire);
    let reply_json = format!(r#"{{"content":"{b64}"}}"#);

    assert_eq!(
        decode_protobuf(&wire)?,
        original,
        "raw protobuf decode must preserve every field across the full APC schema"
    );
    assert_eq!(
        decode(&b64)?,
        original,
        "base64 decode must match raw protobuf decode"
    );
    assert_eq!(
        decode_reply_json(&reply_json)?,
        original,
        "CDP reply JSON decode must match base64 decode"
    );
    Ok(())
}

#[test]
fn explicit_presence_distinguishes_zero_and_empty_from_absent() -> Result<(), Box<dyn Error>> {
    let apc = AnnotatedPageContent {
        version: Some(0),
        mode: Some(0),
        tab_id: Some(0),
        main_frame_data: Some(FrameData {
            url: Some(String::new()),
            title: None,
            default_line_height_px: Some(0),
            ..FrameData::default()
        }),
        ..AnnotatedPageContent::default()
    };
    let decoded = decode_protobuf(&apc.encode_to_vec())?;
    assert_eq!(
        decoded.tab_id,
        Some(0),
        "explicitly set zero scalar must round-trip as Some(0)"
    );
    assert_eq!(
        decoded.version,
        Some(0),
        "explicitly set zero enum must round-trip as Some(0)"
    );
    let frame = decoded
        .main_frame_data
        .as_ref()
        .ok_or("main_frame_data missing after round-trip")?;
    assert_eq!(
        frame.url.as_deref(),
        Some(""),
        "explicitly set empty string must stay Some(\"\")"
    );
    assert_eq!(
        frame.title, None,
        "unset string field must remain None under Edition 2023 explicit presence"
    );
    Ok(())
}

#[test]
fn open_enums_and_field_1000_extensions_survive_decode() -> Result<(), Box<dyn Error>> {
    let mut redacted_bytes = Vec::new();
    push_varint_field(
        1,
        u64::try_from(REASON_BROWSEROS_TIMEOUT)?,
        &mut redacted_bytes,
    );

    let mut iframe_bytes = Vec::new();
    push_length_delimited_field(4, &redacted_bytes, &mut iframe_bytes);

    let mut attrs_bytes = Vec::new();
    push_varint_field(
        3,
        u64::try_from(i32::from(ContentAttributeType::Iframe))?,
        &mut attrs_bytes,
    );
    push_length_delimited_field(9, &iframe_bytes, &mut attrs_bytes);
    // Unpacked repeated open enum entries including a future unknown value (777).
    push_varint_field(
        15,
        u64::try_from(i32::from(AnnotatedRole::Nav))?,
        &mut attrs_bytes,
    );
    push_varint_field(15, 777, &mut attrs_bytes);
    // Unknown BrowserOS field 1000 extension payload on ContentAttributes.
    push_length_delimited_field(BROWSEROS_EXTENSION_TAG, b"ext-node", &mut attrs_bytes);

    let mut root_bytes = Vec::new();
    push_length_delimited_field(2, &attrs_bytes, &mut root_bytes);

    let mut frame_bytes = Vec::new();
    push_length_delimited_field(4, b"https://example.com", &mut frame_bytes);
    // Unknown BrowserOS field 1000 extension payload on FrameData.
    push_length_delimited_field(BROWSEROS_EXTENSION_TAG, b"ext-frame", &mut frame_bytes);

    let mut apc_bytes = Vec::new();
    push_length_delimited_field(2, &root_bytes, &mut apc_bytes);
    push_length_delimited_field(3, &frame_bytes, &mut apc_bytes);
    // Unknown BrowserOS field 1000 extension payload on AnnotatedPageContent.
    push_length_delimited_field(BROWSEROS_EXTENSION_TAG, b"ext-page", &mut apc_bytes);

    let decoded = decode_protobuf(&apc_bytes)?;
    let attrs = decoded
        .root_node
        .as_ref()
        .and_then(|node| node.content_attributes.as_ref())
        .ok_or("root_node.content_attributes missing")?;
    assert_eq!(
        attrs.annotated_roles,
        vec![i32::from(AnnotatedRole::Nav), 777],
        "unpacked repeated open enum must preserve both known and unknown values"
    );
    match attrs.content_data.as_ref() {
        Some(ContentData::Iframe(iframe)) => match iframe.data.as_ref() {
            Some(IframeDataContent::RedactedFrameMetadata(meta)) => {
                assert_eq!(
                    meta.reason,
                    Some(REASON_BROWSEROS_TIMEOUT),
                    "open enum storage must preserve REASON_BROWSEROS_TIMEOUT = 1000"
                );
            }
            other => panic!("expected RedactedFrameMetadata, got {other:?}"),
        },
        other => panic!("expected ContentData::Iframe, got {other:?}"),
    }
    assert_eq!(
        decoded
            .main_frame_data
            .as_ref()
            .and_then(|frame| frame.url.as_deref()),
        Some("https://example.com"),
        "skipping unknown field 1000 on FrameData must preserve neighboring fields"
    );
    Ok(())
}

#[test]
fn ax_role_covers_all_214_variants_and_version_alias_matches() -> Result<(), Box<dyn Error>> {
    for value in 0..=213 {
        assert!(
            AxRole::try_from(value).is_ok(),
            "AxRole must define every discriminant in 0..=213 (missing {value})"
        );
    }
    assert!(
        AxRole::try_from(-1).is_err() && AxRole::try_from(214).is_err(),
        "out-of-range AxRole discriminants must fail TryFrom"
    );
    assert_eq!(
        AxRole::try_from(213)?,
        AxRole::MenuItemSeparator,
        "discriminant 213 must map to MenuItemSeparator"
    );
    assert_eq!(
        AnnotatedPageContentVersion::MAX_VALID,
        AnnotatedPageContentVersion::OnlyActionableElements10,
        "MAX_VALID alias must match OnlyActionableElements10 (value 2)"
    );
    assert_eq!(
        i32::from(AnnotatedPageContentVersion::MAX_VALID),
        2,
        "MAX_VALID wire value must equal 2"
    );
    Ok(())
}

#[test]
fn decode_reply_json_handles_escapes_and_extra_fields() -> Result<(), Box<dyn Error>> {
    let apc = AnnotatedPageContent {
        tab_id: Some(7),
        ..AnnotatedPageContent::default()
    };
    let b64 = STANDARD.encode(apc.encode_to_vec());
    let (first, rest) = b64
        .split_at_checked(1)
        .ok_or("encoded base64 should have at least one ASCII character")?;
    let first_byte = first.as_bytes()[0];
    let json = format!(r#"{{"content":"\u{first_byte:04x}{rest}","extraField":true}}"#);
    assert_eq!(
        decode_reply_json(&json)?,
        apc,
        "reply decoder must handle JSON escape sequences and forward-compatible extra fields"
    );
    Ok(())
}

#[test]
fn invalid_inputs_return_distinct_error_variants() {
    assert!(
        matches!(
            decode_reply_json("not json"),
            Err(DecodeError::ReplyJson(_))
        ),
        "malformed JSON must report DecodeError::ReplyJson"
    );
    assert!(
        matches!(decode_reply_json("{}"), Err(DecodeError::ReplyJson(_))),
        "missing content field must report DecodeError::ReplyJson"
    );
    assert!(
        matches!(
            decode_reply_json(r#"{"content":123}"#),
            Err(DecodeError::ReplyJson(_))
        ),
        "non-string content field must report DecodeError::ReplyJson"
    );
    assert!(
        matches!(decode("not!valid!base64"), Err(DecodeError::Base64(_))),
        "invalid base64 must report DecodeError::Base64"
    );
    assert!(
        matches!(
            decode_reply_json(r#"{"content":"not!valid!base64"}"#),
            Err(DecodeError::Base64(_))
        ),
        "invalid base64 inside reply JSON must report DecodeError::Base64"
    );
    // Tag 1 with wire type 7 (invalid wire type) triggers prost::DecodeError.
    let corrupt_proto = [0x0f_u8];
    assert!(
        matches!(
            decode_protobuf(&corrupt_proto),
            Err(DecodeError::Protobuf(_))
        ),
        "corrupt protobuf wire bytes must report DecodeError::Protobuf"
    );
    // Tag 2 (root_node, length-delimited) declaring 50 bytes when only 2 bytes follow.
    let truncated_root = [0x12_u8, 50, 0x0a, 0x00];
    assert!(
        matches!(
            decode_protobuf(&truncated_root),
            Err(DecodeError::Protobuf(_))
        ),
        "truncated length-delimited root_node must report DecodeError::Protobuf"
    );
    let corrupt_b64 = STANDARD.encode(corrupt_proto);
    assert!(
        matches!(decode(&corrupt_b64), Err(DecodeError::Protobuf(_))),
        "corrupt protobuf inside valid base64 must report DecodeError::Protobuf"
    );
}

/// Encodes a linear `ContentNode` chain of `depth` nodes (depth 1 is a single leaf node) without
/// recursion so tests can construct trees deeper than `prost`'s 100-level recursion limit.
fn encode_nested_content_node_chain(depth: usize, leaf_text: &str) -> Vec<u8> {
    let leaf_attrs = ContentAttributes {
        common_ancestor_dom_node_id: Some(999),
        attribute_type: Some(i32::from(ContentAttributeType::Text)),
        content_data: Some(ContentData::Text(TextInfo {
            text_content: Some(leaf_text.to_owned()),
            text_style: None,
        })),
        ..ContentAttributes::default()
    };
    let mut current = Vec::new();
    push_length_delimited_field(2, &leaf_attrs.encode_to_vec(), &mut current);

    let wrapper_attrs = ContentAttributes {
        attribute_type: Some(i32::from(ContentAttributeType::Container)),
        aria_role: Some(i32::from(AxRole::Group)),
        ..ContentAttributes::default()
    }
    .encode_to_vec();

    for _ in 1..depth {
        let mut parent = Vec::with_capacity(current.len() + wrapper_attrs.len() + 16);
        push_length_delimited_field(2, &wrapper_attrs, &mut parent);
        push_length_delimited_field(1, &current, &mut parent);
        current = parent;
    }
    current
}

#[test]
fn deep_content_node_trees_beyond_prost_recursion_limit_decode_in_root_and_popup()
-> Result<(), Box<dyn Error>> {
    const DEPTH: usize = 250;
    let chain = encode_nested_content_node_chain(DEPTH, "Deep leaf text");

    let mut popup_wire = Vec::new();
    push_length_delimited_field(1, &chain, &mut popup_wire);
    push_varint_field(4, 77, &mut popup_wire);

    let mut apc_wire = Vec::new();
    push_varint_field(1, 1, &mut apc_wire);
    push_length_delimited_field(2, &chain, &mut apc_wire);
    push_length_delimited_field(8, &popup_wire, &mut apc_wire);

    assert!(
        AnnotatedPageContent::decode(apc_wire.as_slice()).is_err(),
        "default prost::Message::decode rejects 250-level trees at its 100-level recursion limit"
    );

    let decoded = decode_protobuf(&apc_wire)?;
    let mut walk_depth = 0_usize;
    let mut cursor = decoded.root_node.as_ref();
    let mut leaf_found = None;
    while let Some(node) = cursor {
        walk_depth += 1;
        if let Some(ContentData::Text(text)) = node
            .content_attributes
            .as_ref()
            .and_then(|attrs| attrs.content_data.as_ref())
        {
            leaf_found = text.text_content.clone();
        }
        cursor = node.children_nodes.first();
    }
    assert_eq!(
        walk_depth, DEPTH,
        "root_node must decode all {DEPTH} levels"
    );
    assert_eq!(
        leaf_found.as_deref(),
        Some("Deep leaf text"),
        "deepest leaf text must survive iterative decode"
    );

    let popup = decoded
        .popup_window
        .as_ref()
        .ok_or("popup_window missing")?;
    assert_eq!(popup.opener_common_ancestor_dom_node_id, Some(77));
    let mut popup_depth = 0_usize;
    let mut popup_cursor = popup.root_node.as_ref();
    while let Some(node) = popup_cursor {
        popup_depth += 1;
        popup_cursor = node.children_nodes.first();
    }
    assert_eq!(
        popup_depth, DEPTH,
        "popup_window.root_node must decode all {DEPTH} levels"
    );
    Ok(())
}

#[test]
fn duplicate_root_node_fields_merge_in_wire_order() -> Result<(), Box<dyn Error>> {
    let first_root = ContentNode {
        children_nodes: vec![node_with_data(
            10,
            ContentAttributeType::Text,
            ContentData::Text(TextInfo {
                text_content: Some("first".to_owned()),
                text_style: None,
            }),
            Vec::new(),
        )],
        content_attributes: Some(ContentAttributes {
            common_ancestor_dom_node_id: Some(1),
            attribute_type: Some(i32::from(ContentAttributeType::Root)),
            ..ContentAttributes::default()
        }),
    };
    let second_root = ContentNode {
        children_nodes: vec![node_with_data(
            11,
            ContentAttributeType::Text,
            ContentData::Text(TextInfo {
                text_content: Some("second".to_owned()),
                text_style: None,
            }),
            Vec::new(),
        )],
        content_attributes: Some(ContentAttributes {
            label: Some("merged-label".to_owned()),
            ..ContentAttributes::default()
        }),
    };

    let mut wire = Vec::new();
    push_length_delimited_field(2, &first_root.encode_to_vec(), &mut wire);
    push_length_delimited_field(2, &second_root.encode_to_vec(), &mut wire);

    let decoded = decode_protobuf(&wire)?;
    let root = decoded.root_node.as_ref().ok_or("root_node missing")?;
    assert_eq!(
        root.children_nodes.len(),
        2,
        "merged root_node must keep children from both wire occurrences in order"
    );
    let attrs = root
        .content_attributes
        .as_ref()
        .ok_or("root_node.content_attributes missing")?;
    assert_eq!(attrs.common_ancestor_dom_node_id, Some(1));
    assert_eq!(attrs.label.as_deref(), Some("merged-label"));
    Ok(())
}

#[test]
fn content_node_nesting_limit_allows_max_and_rejects_excess() -> Result<(), Box<dyn Error>> {
    let at_limit = encode_nested_content_node_chain(MAX_CONTENT_NODE_DEPTH, "at limit");
    let mut wire_ok = Vec::new();
    push_length_delimited_field(2, &at_limit, &mut wire_ok);
    let decoded = decode_protobuf(&wire_ok)?;
    assert!(
        decoded.root_node.is_some(),
        "tree at exact MAX_CONTENT_NODE_DEPTH must decode"
    );

    let over_limit = encode_nested_content_node_chain(MAX_CONTENT_NODE_DEPTH + 1, "over limit");
    let mut wire_err = Vec::new();
    push_length_delimited_field(2, &over_limit, &mut wire_err);
    assert!(
        matches!(
            decode_protobuf(&wire_err),
            Err(DecodeError::NestingLimitExceeded { limit }) if limit == MAX_CONTENT_NODE_DEPTH
        ),
        "tree deeper than MAX_CONTENT_NODE_DEPTH must return DecodeError::NestingLimitExceeded"
    );
    Ok(())
}
