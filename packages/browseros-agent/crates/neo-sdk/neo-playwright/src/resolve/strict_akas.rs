//! The tree's role locators in a strict-mode violation's `aka` lines (apc-engine-design 5.5).
//!
//! The page writes a violation it decides with upstream's `strictModeViolationError`, whose
//! selector generator runs with the stand-in role engine and so never offers
//! `getByRole(role, { name })`, the candidate upstream's generator prefers to every other but a
//! test id. The page lists what it wrote (`StrictListing`); this module reads each listed
//! element's role and name from a capture and rewrites the lines where the tree has a role
//! locator that selects the element alone, so the message names elements as upstream's does.
//!
//! ```text
//!  page   strict { message, listing: { locator, count, pins, items } }
//!  Rust   capture ─ pinned_nodes_in_document(pins) ─ keys in the world's document
//!           ─ select::role_aka per element ─ strict_message(locator, count, items)
//! ```
//!
//! A message the tree cannot improve stays the page's, byte for byte.

use neo_snapshot_apc::select::{self, AccessibleText, RefResolver, StrictPreview};
use neo_snapshot_apc::tree::{NameCache, NodeId, NodeKey, SnapshotTree, TreeNode};

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, PinPreview, StrictListing};
use crate::tree::{AttemptStart, backend_to_dom_node_id, frame_doc};
use crate::world::IsolatedWorld;

/// `message` with the tree's role locator in place of each listed `aka` it improves, for a
/// violation the page decided in `world`. Every failure along the way keeps `message`.
pub(crate) async fn worded(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    message: String,
    listing: StrictListing,
) -> String {
    let Ok(captured) = page
        .tree()
        .capture_seeded(scope, page, AttemptStart::now())
        .await
    else {
        return message;
    };
    // After the capture, as every hydration is: the world answering proves it held the
    // capture's document (design 6.2), and each pin is then a node of that document.
    let Ok(Some(ids)) =
        probe::pinned_nodes_in_document(scope, page, world, listing.pins.clone()).await
    else {
        return message;
    };
    let tree = captured.tree();
    let main_frame = page.main_endpoint().frame;
    let Some(document) =
        frame_doc(tree, &main_frame, world.frame()).and_then(|doc| tree.document(doc))
    else {
        return message;
    };
    let keys: Vec<Option<NodeKey>> = ids
        .into_iter()
        .map(|id| {
            backend_to_dom_node_id(id)
                .ok()
                .map(|dom| NodeKey::new(document.token().clone(), dom))
        })
        .collect();
    let names = NameCache::new(tree);
    let nodes = keys.iter().map(|key| {
        key.as_ref()
            .and_then(|key| tree.node_by_key(key))
            .map(TreeNode::id)
    });
    match with_roles(tree, &names, page.tree(), &listing.items, nodes) {
        Some(items) => {
            let count = usize::try_from(listing.count).unwrap_or(usize::MAX);
            select::strict_message(&listing.locator, count, &items)
        }
        None => message,
    }
}

/// The listed lines with the tree's role locator in place of each `aka` it improves, or `None`
/// when it improves none.
pub(crate) fn with_roles(
    tree: &SnapshotTree,
    text: &dyn AccessibleText,
    refs: &dyn RefResolver,
    items: &[PinPreview],
    nodes: impl Iterator<Item = Option<NodeId>>,
) -> Option<Vec<StrictPreview>> {
    let mut changed = false;
    let lines = items
        .iter()
        .zip(nodes)
        .map(|(item, node)| {
            let role = node
                .filter(|_| !is_test_id(&item.aka))
                .and_then(|node| select::role_aka(tree, text, refs, node));
            let aka = match role {
                Some(role) if role != item.aka => {
                    changed = true;
                    role
                }
                Some(_) | None => item.aka.clone(),
            };
            StrictPreview {
                preview: item.preview.clone(),
                aka,
            }
        })
        .collect();
    changed.then_some(lines)
}

/// Whether the page's locator is a test id, the one candidate upstream scores below a role with
/// a name (`kTestIdScore`, `kOtherTestIdScore`).
pub(crate) fn is_test_id(aka: &str) -> bool {
    aka.starts_with("getByTestId(") || aka.starts_with("locator('[data-test")
}
