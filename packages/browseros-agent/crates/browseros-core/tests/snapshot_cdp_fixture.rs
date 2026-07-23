use browseros_core::snapshot::{
    AxNode, RefMap, RenderOptions, SnapshotMode, SnapshotOptions, apply_snapshot_options,
    render_snapshot,
};
use serde::Deserialize;

const GET_FULL_AX_TREE: &str = include_str!("data/get-full-ax-tree.json");

#[derive(Deserialize)]
struct AxTreeResult {
    nodes: Vec<AxNode>,
}

fn fixture() -> Result<AxTreeResult, serde_json::Error> {
    serde_json::from_str(GET_FULL_AX_TREE)
}

#[test]
fn cdp_ax_tree_deserializes_backend_dom_node_ids() -> Result<(), serde_json::Error> {
    let result = fixture()?;
    let button = result.nodes.iter().find(|node| node.node_id == "4");
    let textbox = result.nodes.iter().find(|node| node.node_id == "5");

    assert_eq!(button.and_then(|node| node.backend_dom_node_id), Some(101));
    assert_eq!(textbox.and_then(|node| node.backend_dom_node_id), Some(102));
    Ok(())
}

#[test]
fn cdp_ax_tree_mints_refs_retained_by_interactive_mode() -> Result<(), serde_json::Error> {
    let result = fixture()?;
    let mut refs = RefMap::new();
    let mut render_options = RenderOptions {
        refs: &mut refs,
        frame_id: None,
        document_id: None,
        cursor_hits: None,
        base_depth: 0,
    };

    let full = render_snapshot(&result.nodes, &mut render_options).text;
    assert_eq!(
        full,
        [
            "- main \"Checkout\"",
            "  - paragraph \"Review your order\"",
            "  - button \"Place order\" [ref=e1]",
            "  - textbox \"Promo code\" [ref=e2]: \"SAVE20\"",
        ]
        .join("\n")
    );

    let interactive = apply_snapshot_options(
        &full,
        SnapshotOptions {
            mode: SnapshotMode::Interactive,
            depth: None,
        },
    );
    assert_eq!(
        interactive,
        [
            "- main \"Checkout\"",
            "  - button \"Place order\" [ref=e1]",
            "  - textbox \"Promo code\" [ref=e2]: \"SAVE20\"",
        ]
        .join("\n")
    );
    Ok(())
}
