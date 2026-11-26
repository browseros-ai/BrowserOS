//! How an oracle element finds the tree node it is compared with: the one join every oracle
//! comparison makes, through [`SnapshotTree::node_at`] as Neo's own reverse join does
//! (`snapshot::element_in_tree`).
//!
//! An element APC sent joins by its backendNodeId in its frame's document. An `<option>` of a
//! collapsed `<select>` has no node APC sent, only an entry of its select's `select_options`, so
//! it joins the select's option slot at its recorded index (`OracleElement::option`). A select
//! with a different number of slots than the page listed options is a broken recording, not a
//! miss. An oracle recorded before that field joins such an option nowhere.

use std::error::Error;

use neo_snapshot_apc::tree::{
    DocToken, DomNodeId, ElementPlace, NodeKey, OptionPlace, SnapshotTree, TreeDoc, TreeNode,
};

use crate::oracle::OracleElement;

/// The tree node `element` stands for in `doc`, the tree document behind its oracle frame:
/// `None` when the tree has none for it, an error when the select's option count disagrees.
pub(crate) fn joined_node<'tree>(
    tree: &'tree SnapshotTree,
    doc: &TreeDoc,
    element: &OracleElement,
) -> Result<Option<&'tree TreeNode>, Box<dyn Error>> {
    // One page load records both the capture and the oracle, so a select's option count can
    // only disagree when the recording is broken.
    tree.node_at(&element_place(doc.token(), element)?)
        .map_err(|changed| {
            format!(
                "oracle element backendNodeId {} (role={:?}, name={:?}) in document {:?}: {changed}; \
                 re-record the fixture",
                element.backend_node_id,
                element.role,
                element.name,
                doc.token(),
            )
            .into()
        })
}

/// Where `element` sits in the document `token`, as the tree names places.
fn element_place(
    token: &DocToken,
    element: &OracleElement,
) -> Result<ElementPlace, Box<dyn Error>> {
    let key = |backend_node_id: u64| -> Result<NodeKey, Box<dyn Error>> {
        Ok(NodeKey::new(
            token.clone(),
            DomNodeId::new(i32::try_from(backend_node_id)?),
        ))
    };
    let option = match &element.option {
        Some(option) => Some(OptionPlace {
            select: key(option.select)?,
            index: option.index,
            count: option.count,
        }),
        None => None,
    };
    Ok(ElementPlace {
        key: key(element.backend_node_id)?,
        option,
    })
}
