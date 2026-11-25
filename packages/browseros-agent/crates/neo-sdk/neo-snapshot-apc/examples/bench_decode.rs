//! Deterministic release-mode benchmark for `neo-snapshot-apc` decode throughput.
//!
//! Benchmarks both the largest recorded Chromium 158 fixture (`nested-cross-site-frames.pb`) and
//! a deterministically generated large-page APC payload targeting ~11 MB base64 (~8 MB protobuf).
//!
//! Run from `packages/browseros-agent`:
//! ```sh
//! cargo run --release -p neo-snapshot-apc --example bench_decode
//! ```

#[path = "../tests/support/tree_counts.rs"]
mod tree_counts;

use std::error::Error;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::{fs, hint};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use neo_snapshot_apc::proto::{
    AnchorData, AnnotatedPageContentMode, AnnotatedPageContentVersion, AnnotatedRole, AxRole,
    BoundingRect, ClickabilityReason, ContentAttributeType, ContentAttributes, ContentData,
    ContentNode, CssPosition, DocumentIdentifier, FormControlData, FormControlType, FrameData,
    Geometry, IframeData, IframeDataContent, InteractionInfo, RedactionDecision, SecurityOrigin,
    TextInfo, TextSize, TextStyle,
};
use neo_snapshot_apc::{AnnotatedPageContent, decode, decode_protobuf, decode_reply_json};
use prost::Message as _;
use tree_counts::{TreeCounts, count_frames_and_nodes};

/// Number of unmeasured warmup runs before timing starts.
const WARMUP_RUNS: usize = 3;

/// Number of timed runs used to compute median, min, and max latency.
const TIMED_RUNS: usize = 11;

/// Target raw protobuf size (~8.25 MB -> ~11.0 MB standard base64).
const LARGE_PAGE_TARGET_PROTO_BYTES: usize = 8_250_000;

/// Latency summary statistics (in milliseconds) over one benchmarked operation.
struct LatencyMs {
    median: f64,
    min: f64,
    max: f64,
}

fn main() -> Result<(), Box<dyn Error>> {
    let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");

    let deep_bytes = fs::read(fixtures_dir.join("deep-nesting.pb"))?;
    bench_payload("recorded (largest): deep-nesting.pb", &deep_bytes)?;

    let nested_bytes = fs::read(fixtures_dir.join("nested-cross-site-frames.pb"))?;
    bench_payload("recorded: nested-cross-site-frames.pb", &nested_bytes)?;

    let large_apc = generate_large_page_apc(LARGE_PAGE_TARGET_PROTO_BYTES);
    let large_bytes = large_apc.encode_to_vec();
    bench_payload("synthetic: large-page (~11 MB base64)", &large_bytes)?;

    Ok(())
}

#[expect(clippy::print_stdout, reason = "benchmark CLI prints timing results")]
fn bench_payload(label: &str, proto_bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let b64 = STANDARD.encode(proto_bytes);
    let reply_json = format!(r#"{{"content":"{b64}"}}"#);

    let baseline = decode_protobuf(proto_bytes)?;
    let expected = count_frames_and_nodes(&baseline);

    let reply_stats = measure(
        || Ok(decode_reply_json(hint::black_box(reply_json.as_str()))?),
        |apc| verify_counts(apc, expected),
    )?;

    let decode_b64_stats = measure(
        || Ok(decode(hint::black_box(b64.as_str()))?),
        |apc| verify_counts(apc, expected),
    )?;

    let b64_stats = measure(
        || Ok(STANDARD.decode(hint::black_box(b64.as_str()))?),
        |raw| {
            if raw.len() != proto_bytes.len() {
                return Err(format!(
                    "base64 decode length mismatch: got {}, expected {}",
                    raw.len(),
                    proto_bytes.len()
                )
                .into());
            }
            Ok(())
        },
    )?;

    let proto_stats = measure(
        || Ok(decode_protobuf(hint::black_box(proto_bytes))?),
        |apc| verify_counts(apc, expected),
    )?;

    println!("=== {label} ===");
    println!(
        "  proto_bytes={}  base64_bytes={}  frames={}  nodes={}",
        proto_bytes.len(),
        b64.len(),
        expected.frames,
        expected.nodes
    );
    print_stats("decode_reply_json (end-to-end)", &reply_stats);
    print_stats("decode (base64 + prost)       ", &decode_b64_stats);
    print_stats("base64::STANDARD.decode       ", &b64_stats);
    print_stats("decode_protobuf (prost)       ", &proto_stats);

    Ok(())
}

#[expect(clippy::print_stdout, reason = "benchmark CLI prints timing results")]
fn print_stats(stage: &str, stats: &LatencyMs) {
    println!(
        "  {stage}: median={:.3} ms  min={:.3} ms  max={:.3} ms",
        stats.median, stats.min, stats.max
    );
}

fn measure<T>(
    mut op: impl FnMut() -> Result<T, Box<dyn Error>>,
    mut verify: impl FnMut(&T) -> Result<(), Box<dyn Error>>,
) -> Result<LatencyMs, Box<dyn Error>> {
    for _ in 0..WARMUP_RUNS {
        let warm = hint::black_box(op()?);
        verify(&warm)?;
    }

    let mut samples: Vec<Duration> = Vec::with_capacity(TIMED_RUNS);
    for _ in 0..TIMED_RUNS {
        let started = Instant::now();
        let output = hint::black_box(op()?);
        let elapsed = started.elapsed();
        verify(&output)?;
        drop(output);
        samples.push(elapsed);
    }
    samples.sort_unstable();

    let min = samples.first().copied().ok_or("no benchmark samples")?;
    let max = samples.last().copied().ok_or("no benchmark samples")?;
    let median = samples
        .get(samples.len() / 2)
        .copied()
        .ok_or("missing median benchmark sample")?;

    Ok(LatencyMs {
        median: duration_to_ms(median),
        min: duration_to_ms(min),
        max: duration_to_ms(max),
    })
}

fn duration_to_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn verify_counts(apc: &AnnotatedPageContent, expected: TreeCounts) -> Result<(), Box<dyn Error>> {
    let actual = count_frames_and_nodes(apc);
    if actual != expected {
        return Err(format!(
            "decoded count mismatch: got ({} frames, {} nodes), expected ({}, {})",
            actual.frames, actual.nodes, expected.frames, expected.nodes
        )
        .into());
    }
    Ok(())
}

/// Deterministically builds a large multi-frame [`AnnotatedPageContent`] tree whose encoded
/// protobuf byte size reaches at least `target_proto_bytes`.
fn generate_large_page_apc(target_proto_bytes: usize) -> AnnotatedPageContent {
    let mut sections = Vec::new();
    let mut approx_bytes = 0_usize;
    let mut next_dom_id = 10_i32;
    let mut section_index = 0_i32;

    // Add 3 embedded iframes first so multi-frame traversal is exercised alongside the main body.
    for frame_idx in 0_i32..3 {
        let mut frame_children = Vec::with_capacity(40);
        for item_idx in 0_i32..40 {
            let item = build_card_node(&mut next_dom_id, frame_idx * 1_000 + item_idx);
            approx_bytes += item.encoded_len();
            frame_children.push(item);
        }
        let iframe = build_iframe_node(&mut next_dom_id, frame_idx, frame_children);
        approx_bytes += iframe.encoded_len();
        sections.push(iframe);
    }

    while approx_bytes < target_proto_bytes {
        let card = build_card_node(&mut next_dom_id, section_index);
        approx_bytes += card.encoded_len();
        sections.push(card);
        section_index += 1;
    }

    AnnotatedPageContent {
        version: Some(i32::from(AnnotatedPageContentVersion::Version10)),
        mode: Some(i32::from(AnnotatedPageContentMode::ActionableElements)),
        root_node: Some(ContentNode {
            children_nodes: sections,
            content_attributes: Some(ContentAttributes {
                common_ancestor_dom_node_id: Some(1),
                attribute_type: Some(i32::from(ContentAttributeType::Root)),
                aria_role: Some(i32::from(AxRole::RootWebArea)),
                ..ContentAttributes::default()
            }),
        }),
        main_frame_data: Some(FrameData {
            security_origin: Some(SecurityOrigin {
                opaque: Some(false),
                value: Some("https://shop.example.test".to_owned()),
            }),
            document_identifier: Some(DocumentIdentifier {
                serialized_token: Some("BENCH-MAIN-DOC-0001".to_owned()),
            }),
            url: Some("https://shop.example.test/catalog".to_owned()),
            title: Some("Synthetic Large Catalog Page".to_owned()),
            default_line_height_px: Some(20),
            ..FrameData::default()
        }),
        viewport_geometry: Some(BoundingRect {
            x: Some(0),
            y: Some(0),
            width: Some(1440),
            height: Some(900),
            is_screenshot_relative: Some(false),
        }),
        ..AnnotatedPageContent::default()
    }
}

fn build_iframe_node(
    next_dom_id: &mut i32,
    frame_idx: i32,
    children: Vec<ContentNode>,
) -> ContentNode {
    let dom_id = take_id(next_dom_id);
    let doc_root_id = take_id(next_dom_id);
    ContentNode {
        children_nodes: vec![ContentNode {
            children_nodes: children,
            content_attributes: Some(ContentAttributes {
                common_ancestor_dom_node_id: Some(doc_root_id),
                attribute_type: Some(i32::from(ContentAttributeType::Root)),
                aria_role: Some(i32::from(AxRole::RootWebArea)),
                ..ContentAttributes::default()
            }),
        }],
        content_attributes: Some(ContentAttributes {
            common_ancestor_dom_node_id: Some(dom_id),
            attribute_type: Some(i32::from(ContentAttributeType::Iframe)),
            geometry: Some(sample_geometry(0, frame_idx * 320, 1200, 300)),
            content_data: Some(ContentData::Iframe(Box::new(IframeData {
                data: Some(IframeDataContent::FrameData(Box::new(FrameData {
                    security_origin: Some(SecurityOrigin {
                        opaque: Some(false),
                        value: Some(format!("https://widget-{frame_idx}.example.test")),
                    }),
                    document_identifier: Some(DocumentIdentifier {
                        serialized_token: Some(format!("BENCH-IFRAME-DOC-{frame_idx:04}")),
                    }),
                    url: Some(format!("https://widget-{frame_idx}.example.test/embed")),
                    title: Some(format!("Embedded Widget {frame_idx}")),
                    default_line_height_px: Some(18),
                    ..FrameData::default()
                }))),
            }))),
            aria_role: Some(i32::from(AxRole::Iframe)),
            ..ContentAttributes::default()
        }),
    }
}

fn build_card_node(next_dom_id: &mut i32, index: i32) -> ContentNode {
    let container_id = take_id(next_dom_id);
    let y = (index % 200) * 96;
    let heading = build_card_heading(next_dom_id, index, y);
    let anchor = build_card_anchor(next_dom_id, index, y);
    let button = build_card_button(next_dom_id, index, y);

    ContentNode {
        children_nodes: vec![heading, anchor, button],
        content_attributes: Some(ContentAttributes {
            common_ancestor_dom_node_id: Some(container_id),
            attribute_type: Some(i32::from(ContentAttributeType::Container)),
            geometry: Some(sample_geometry(16, y, 400, 88)),
            annotated_roles: vec![i32::from(AnnotatedRole::Article)],
            aria_role: Some(i32::from(AxRole::Article)),
            ..ContentAttributes::default()
        }),
    }
}

fn build_card_heading(next_dom_id: &mut i32, index: i32, y: i32) -> ContentNode {
    let heading_id = take_id(next_dom_id);
    let heading_text_id = take_id(next_dom_id);
    ContentNode {
        children_nodes: vec![ContentNode {
            children_nodes: Vec::new(),
            content_attributes: Some(ContentAttributes {
                common_ancestor_dom_node_id: Some(heading_text_id),
                attribute_type: Some(i32::from(ContentAttributeType::Text)),
                geometry: Some(sample_geometry(24, y + 8, 360, 24)),
                content_data: Some(ContentData::Text(TextInfo {
                    text_content: Some(format!("Catalog Listing Item #{index:05} — Edition 2023")),
                    text_style: Some(TextStyle {
                        text_size: Some(i32::from(TextSize::L)),
                        has_emphasis: Some(true),
                        color: Some(0x1f29_37ff),
                    }),
                })),
                ..ContentAttributes::default()
            }),
        }],
        content_attributes: Some(ContentAttributes {
            common_ancestor_dom_node_id: Some(heading_id),
            attribute_type: Some(i32::from(ContentAttributeType::Heading)),
            geometry: Some(sample_geometry(24, y + 8, 360, 24)),
            aria_role: Some(i32::from(AxRole::Heading)),
            ..ContentAttributes::default()
        }),
    }
}

fn build_card_anchor(next_dom_id: &mut i32, index: i32, y: i32) -> ContentNode {
    let link_id = take_id(next_dom_id);
    let link_text_id = take_id(next_dom_id);
    ContentNode {
        children_nodes: vec![ContentNode {
            children_nodes: Vec::new(),
            content_attributes: Some(ContentAttributes {
                common_ancestor_dom_node_id: Some(link_text_id),
                attribute_type: Some(i32::from(ContentAttributeType::Text)),
                content_data: Some(ContentData::Text(TextInfo {
                    text_content: Some(format!("Inspect details for item {index}")),
                    text_style: Some(TextStyle {
                        text_size: Some(i32::from(TextSize::MDefault)),
                        has_emphasis: Some(false),
                        color: Some(0x2563_ebff),
                    }),
                })),
                ..ContentAttributes::default()
            }),
        }],
        content_attributes: Some(ContentAttributes {
            common_ancestor_dom_node_id: Some(link_id),
            attribute_type: Some(i32::from(ContentAttributeType::Anchor)),
            geometry: Some(sample_geometry(24, y + 40, 240, 20)),
            content_data: Some(ContentData::Anchor(AnchorData {
                url: Some(format!("https://shop.example.test/items/{index}")),
                rel: Vec::new(),
            })),
            interaction_info: Some(InteractionInfo {
                is_focusable: Some(true),
                is_tabbable: Some(true),
                document_scoped_z_order: Some(1),
                clickability_reasons: vec![i32::from(ClickabilityReason::ClickableControl)],
                ..InteractionInfo::default()
            }),
            aria_role: Some(i32::from(AxRole::Link)),
            ..ContentAttributes::default()
        }),
    }
}

fn build_card_button(next_dom_id: &mut i32, index: i32, y: i32) -> ContentNode {
    let button_id = take_id(next_dom_id);
    ContentNode {
        children_nodes: Vec::new(),
        content_attributes: Some(ContentAttributes {
            common_ancestor_dom_node_id: Some(button_id),
            attribute_type: Some(i32::from(ContentAttributeType::FormControl)),
            geometry: Some(sample_geometry(280, y + 36, 120, 32)),
            content_data: Some(ContentData::FormControl(Box::new(FormControlData {
                field_name: Some(format!("add_{index}")),
                field_value: Some(String::new()),
                form_control_type: Some(i32::from(FormControlType::ButtonButton)),
                redaction_decision: Some(i32::from(RedactionDecision::NoRedactionNecessary)),
                ..FormControlData::default()
            }))),
            label: Some(format!("Add item {index} to cart")),
            interaction_info: Some(InteractionInfo {
                is_focusable: Some(true),
                is_tabbable: Some(true),
                document_scoped_z_order: Some(1),
                clickability_reasons: vec![
                    i32::from(ClickabilityReason::ClickableControl),
                    i32::from(ClickabilityReason::CursorPointer),
                ],
                ..InteractionInfo::default()
            }),
            aria_role: Some(i32::from(AxRole::Button)),
            redaction_decision: Some(i32::from(RedactionDecision::NoRedactionNecessary)),
            ..ContentAttributes::default()
        }),
    }
}

fn sample_geometry(x: i32, y: i32, width: i32, height: i32) -> Geometry {
    let rect = BoundingRect {
        x: Some(x),
        y: Some(y),
        width: Some(width),
        height: Some(height),
        is_screenshot_relative: Some(false),
    };
    Geometry {
        outer_bounding_box: Some(rect),
        visible_bounding_box: Some(rect),
        is_fixed_or_sticky_position: Some(false),
        fragment_visible_bounding_boxes: vec![rect],
        css_position: Some(i32::from(CssPosition::StaticDefault)),
    }
}

fn take_id(counter: &mut i32) -> i32 {
    let id = *counter;
    *counter += 1;
    id
}
