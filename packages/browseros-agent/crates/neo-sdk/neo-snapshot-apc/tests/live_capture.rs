//! Live browser integration test proving `Page.getAnnotatedPageContent` decodes cleanly against
//! Chromium 158 through both the typed CDP wrapper (`browseros_cdp::page::get_annotated_page_content`
//! -> `neo_snapshot_apc::decode`) and the raw-reply path (`CdpClient::send_raw` ->
//! `neo_snapshot_apc::decode_reply_json`).
//!
//! Run from `packages/browseros-agent`:
//! ```sh
//! NEO_E2E_BROWSER="/Users/shadowfax/code/chromium-3/src/out/Default_browseros_arm64/BrowserOS Dev.app/Contents/MacOS/BrowserOS Dev" \
//!   bun run test:browser -- cargo test -p neo-snapshot-apc --test live_capture -- --ignored
//! ```

#[path = "support/fixture_pages.rs"]
#[expect(
    dead_code,
    reason = "ALL_FIXTURES serves record_fixtures while live_capture tests nested-cross-site-frames"
)]
mod fixture_pages;
#[path = "support/fixture_server.rs"]
mod fixture_server;

use std::error::Error;

use fixture_pages::{
    all_fixture_pages, capture_fixture_raw_json, capture_fixture_typed, connect_cdp,
    count_frames_and_nodes,
};
use fixture_server::FixtureServer;
use neo_snapshot_apc::AnnotatedPageContent;
use neo_snapshot_apc::proto::{ContentData, ContentNode};

fn collect_text_and_labels(apc: &AnnotatedPageContent) -> Vec<String> {
    fn walk(node: &ContentNode, out: &mut Vec<String>) {
        if let Some(attrs) = node.content_attributes.as_ref() {
            if let Some(label) = attrs.label.as_ref() {
                out.push(label.clone());
            }
            if let Some(ContentData::Text(text)) = attrs.content_data.as_ref()
                && let Some(content) = text.text_content.as_ref()
            {
                out.push(content.trim().to_owned());
            }
        }
        for child in &node.children_nodes {
            walk(child, out);
        }
    }

    let mut out = Vec::new();
    if let Some(root) = apc.root_node.as_ref() {
        walk(root, &mut out);
    }
    out
}

fn assert_nested_frames_tree(apc: &AnnotatedPageContent, path_name: &str) {
    let (frames, nodes) = count_frames_and_nodes(apc);
    assert_eq!(
        frames, 8,
        "{path_name}: expected 8 frames (Top + 7 nested cross-site iframes)"
    );
    assert!(
        nodes >= 40,
        "{path_name}: expected at least 40 content nodes across 8 frames, got {nodes}"
    );

    let texts = collect_text_and_labels(apc);
    for frame in ["Top", "A1", "B2", "B3", "B1", "C2", "A3", "C4"] {
        let button_text = format!("{frame} button");
        let field_label = format!("{frame} field");
        assert!(
            texts.iter().any(|item| item == frame),
            "{path_name}: missing heading text {frame:?} in {texts:?}"
        );
        assert!(
            texts.iter().any(|item| item == &button_text),
            "{path_name}: missing button text {button_text:?} in {texts:?}"
        );
        assert!(
            texts.iter().any(|item| item == &field_label),
            "{path_name}: missing input label {field_label:?} in {texts:?}"
        );
    }
}

#[tokio::test]
#[ignore = "needs Chromium 158 with CDP on BROWSEROS_CDP_PORT (default 9005)"]
async fn live_chromium_158_captures_nested_cross_site_frames_typed_and_raw_json()
-> Result<(), Box<dyn Error>> {
    let server = FixtureServer::start(all_fixture_pages()).await?;
    let client = connect_cdp().await?;

    let result = async {
        let (raw_bytes, typed_apc) =
            capture_fixture_typed(&client, &server, "nested-cross-site-frames").await?;
        assert!(
            !raw_bytes.is_empty(),
            "typed APC capture returned empty protobuf bytes"
        );
        assert_nested_frames_tree(&typed_apc, "typed get_annotated_page_content -> decode");

        let (reply_json, raw_json_apc) =
            capture_fixture_raw_json(&client, &server, "nested-cross-site-frames").await?;
        assert!(
            reply_json.contains("\"content\""),
            "raw CDP reply JSON should contain the content field"
        );
        assert_nested_frames_tree(&raw_json_apc, "CdpClient::send_raw -> decode_reply_json");

        let (typed_frames, typed_nodes) = count_frames_and_nodes(&typed_apc);
        let (raw_frames, raw_nodes) = count_frames_and_nodes(&raw_json_apc);
        assert_eq!(
            (typed_frames, typed_nodes),
            (raw_frames, raw_nodes),
            "typed and raw JSON capture paths should decode identical frame/node counts"
        );
        Ok::<(), Box<dyn Error>>(())
    }
    .await;

    client.disconnect().await;
    result
}
