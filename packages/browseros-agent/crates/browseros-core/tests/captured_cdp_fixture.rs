//! Structural regression guard over REAL `Accessibility.getFullAXTree`
//! payloads captured from a live BrowserOS by the claw-mcp contract
//! suite's capture mode:
//!
//!   CLAW_MCP_CAPTURE_DIR=crates/browseros-core/tests/data/captured \
//!     BROWSEROS_BINARY=… bun contracts/claw-mcp/tests/run.ts --smoke
//!
//! The hand-authored `snapshot_cdp_fixture.rs` pins exact rendered text
//! against a synthetic tree; this pins the *shape* of real captures so a
//! serde regression like the `backendDOMNodeId` casing break (which
//! silently zeroed ref minting) cannot pass unit tests again. Assertions
//! are content-independent so a capture refresh never breaks them.

use std::fs;
use std::path::{Path, PathBuf};

use browseros_core::snapshot::{AxNode, RefMap, RenderOptions, render_snapshot};
use serde::Deserialize;

#[derive(Deserialize)]
struct AxTreeResult {
    nodes: Vec<AxNode>,
}

fn captured_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/captured")
}

/// Every captured page directory that holds an AX-tree dump.
fn captured_pages() -> Vec<(String, PathBuf)> {
    let mut pages = Vec::new();
    let dir = captured_dir();
    let entries = fs::read_dir(&dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("read dir entry: {err}"));
        let ax = entry.path().join("get-full-ax-tree.json");
        if ax.is_file() {
            let name = entry.file_name().to_string_lossy().into_owned();
            pages.push((name, ax));
        }
    }
    pages.sort_by(|a, b| a.0.cmp(&b.0));
    pages
}

fn load_nodes(path: &Path) -> Vec<AxNode> {
    let raw =
        fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    let result: AxTreeResult = serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("deserialize {}: {err}", path.display()));
    result.nodes
}

fn mint_refs(nodes: &[AxNode]) -> usize {
    let mut refs = RefMap::new();
    let mut options = RenderOptions {
        refs: &mut refs,
        frame_id: None,
        document_id: None,
        cursor_hits: None,
        base_depth: 0,
    };
    let rendered = render_snapshot(nodes, &mut options).text;
    rendered.matches("[ref=e").count()
}

/// Pages whose fixtures carry ARIA-interactive nodes (buttons, textboxes,
/// links, file inputs) that mint refs from the AX tree alone — exactly
/// what the backendDOMNodeId serde break silently prevented. (scroll's
/// targets are cursor-hit-only and need hit-test data the AX dump lacks.)
const INTERACTIVE_PAGES: &[&str] = &["form", "links", "upload"];

#[test]
fn every_captured_ax_tree_deserializes_with_backend_node_ids() {
    let pages = captured_pages();
    assert!(
        !pages.is_empty(),
        "no captured AX trees under {} — run the claw-mcp suite with CLAW_MCP_CAPTURE_DIR",
        captured_dir().display(),
    );
    for (name, path) in &pages {
        let nodes = load_nodes(path);
        assert!(!nodes.is_empty(), "{name}: captured AX tree has no nodes");
        let with_backend = nodes
            .iter()
            .filter(|node| node.backend_dom_node_id.is_some())
            .count();
        assert!(
            with_backend > 0,
            "{name}: no node deserialized a backendDOMNodeId (the serde regression class)",
        );
    }
}

#[test]
fn interactive_captured_pages_mint_refs() {
    let pages = captured_pages();
    for (name, path) in &pages {
        if !INTERACTIVE_PAGES.contains(&name.as_str()) {
            continue;
        }
        let nodes = load_nodes(path);
        let ref_count = mint_refs(&nodes);
        assert!(
            ref_count > 0,
            "{name}: rendered snapshot minted zero refs from a real CDP capture",
        );
    }
}

#[test]
fn captured_frame_trees_and_describe_nodes_are_valid_json() {
    for (name, ax_path) in captured_pages() {
        let dir = ax_path
            .parent()
            .unwrap_or_else(|| panic!("missing parent for {}", ax_path.display()));
        for companion in ["get-frame-tree.json", "describe-node.json"] {
            let path = dir.join(companion);
            let raw = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
            let value: serde_json::Value = serde_json::from_str(&raw)
                .unwrap_or_else(|err| panic!("parse {}: {err}", path.display()));
            assert!(
                value.is_object(),
                "{name}/{companion}: expected a CDP result object",
            );
        }
    }
}
