//! A collapsed `<select>`'s options as arena nodes, "option slots" (apc-engine-design 3.5).
//!
//! APC sends a collapsed select's options as data on the select (`FormControlData.select_options`,
//! which Chromium fills from `select.options`, in order), with no node, DOM id or box. Frame
//! stitching parses that data into the same shape a list box's real `<option>` nodes have: one
//! slot per entry, appended as the select's last children, carrying the facts the passes read on a
//! real `<option>` ([`option_attributes`]). So the role, hidden, state, name and emitter rules are
//! the `<option>` rules, and selection matches, counts and orders slots as nodes.
//!
//! ```text
//!  [s]    select          Apc(Some(41))            select_options: [a, b, c]
//!  [s+1]    ua_shadow     Apc(None)                (folded away)
//!  [s+2]    option slot   SelectOption{index: 0}  ┐ the select's last children, leaves,
//!  [s+3]    option slot   SelectOption{index: 1}  │ in select.options order
//!  [s+4]    option slot   SelectOption{index: 2}  ┘
//!
//!  forward  slot ─ option_slot ─▶ OptionSlot{select, index, count} ─▶ page: select.options[index]
//!  reverse  page: <option> ─▶ ElementPlace{key, option} ─ node_at ─▶ slot
//! ```
//!
//! The label stays in the entry, the one copy the mask bullets or clears ([`label`]). A slot whose
//! entry the mask cleared is withheld ([`withhold_cleared`]), so a cleared list still shows
//! nothing.

use crate::proto::{BrowserOsContentAttributes, ContentAttributes, ContentData, SelectOption};

use super::hidden;
use super::node_key::{NodeId, NodeKey};
use super::snapshot_tree::SnapshotTree;
use super::tree_node::TreeNode;

/// A tree node that stands for an option of a collapsed `<select>`: the page reaches it as
/// `select.options[index]`.
///
/// `count` is how many options the select listed in the capture. It is the witness the page checks
/// before it trusts `index`: once the list grew or shrank, `index` may name another option, and the
/// page answers that the member changed instead of acting on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptionSlot {
    /// The select whose option list holds the option.
    pub select: NodeId,
    /// The option's index in `select.options`.
    pub index: u32,
    /// How many options the select listed in the capture.
    pub count: u32,
}

/// Where the page says one of its elements sits: the reverse of [`OptionSlot`], read by
/// [`SnapshotTree::node_at`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementPlace {
    /// The element's own key, which joins every node APC sent.
    pub key: NodeKey,
    /// The element's place in its select's option list, when it is an `<option>` its select lists.
    pub option: Option<OptionPlace>,
}

/// An `<option>`'s place in its select's option list, as the page reads it now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionPlace {
    /// The key of the select that lists the option.
    pub select: NodeKey,
    /// The option's index in `select.options`.
    pub index: u32,
    /// How many options the select lists now.
    pub count: u32,
}

/// A select's entries when they become slots: a page `<select>` that is not a list box, with a
/// non-empty list. A list box sends its options as real nodes instead. The other half of the rule,
/// that the select holds no real `<option>` node in its own document, is frame stitching's walk
/// flag, since only the walk sees the whole subtree before the select's range closes.
pub(crate) fn listed_options(select: &ContentAttributes) -> Option<&[SelectOption]> {
    if hidden::is_text_node(select) || hidden::is_ua_shadow(select) {
        return None;
    }
    if !hidden::raw_tag(select).is_some_and(|tag| tag.eq_ignore_ascii_case("select")) {
        return None;
    }
    let is_list_box = select
        .browseros
        .as_deref()
        .is_some_and(|browseros| browseros.list_box == Some(true));
    if is_list_box {
        return None;
    }
    entries(select).filter(|entries| !entries.is_empty())
}

/// A page `<option>` element APC sent as a node, as a list box's options are.
pub(crate) fn is_option_element(raw: &ContentAttributes) -> bool {
    !hidden::is_text_node(raw)
        && !hidden::is_ua_shadow(raw)
        && hidden::raw_tag(raw).is_some_and(|tag| tag.eq_ignore_ascii_case("option"))
}

/// The facts a slot carries, as APC writes them on a list box's real `<option>`: the tag, its
/// `selected` and `disabled` (contract fields 20 and 22, which `is_disabled` fills from Blink's
/// `IsDisabledFormControl()`, so a disabled `<optgroup>` counts), and the select's own `inert`,
/// since an option's computed style is its select's. Inherited facts (`aria-hidden`,
/// `aria-disabled`) are left unset, so the passes inherit them from the select as for any child.
pub(crate) fn option_attributes(
    select: &ContentAttributes,
    entry: &SelectOption,
) -> ContentAttributes {
    ContentAttributes {
        browseros: Some(Box::new(BrowserOsContentAttributes {
            tag: Some("option".to_owned()),
            selected: entry.is_selected,
            disabled: entry.is_disabled,
            inert: select
                .browseros
                .as_deref()
                .and_then(|browseros| browseros.inert),
            ..BrowserOsContentAttributes::default()
        })),
        ..ContentAttributes::default()
    }
}

/// A slot's content text: its entry's `text`, as the mask left it (bullets when private). Never
/// the entry's `value`, which is a submission value and no part of Playwright's name.
pub(crate) fn label(tree: &SnapshotTree, id: NodeId) -> Option<&str> {
    let node = tree.node(id)?;
    let index = usize::try_from(node.option_index()?).ok()?;
    let select = tree.node(node.parent()?)?.raw_attributes()?;
    entries(select)?.get(index)?.text.as_deref()
}

/// Withholds every slot whose entry the mask removed. The mask clears a list whole (a select
/// inside a Chromium-redacted node), so this keeps "a cleared list synthesizes nothing" while the
/// slots keep their arena places: every range and link around them is already final.
pub(crate) fn withhold_cleared(nodes: &mut [TreeNode]) {
    let cleared: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| !entry_survives(nodes, node))
        .map(|(position, _)| position)
        .collect();
    for position in cleared {
        if let Some(node) = nodes.get_mut(position) {
            node.withhold();
        }
    }
}

/// Whether `node` is not a slot, or is one whose entry its select still lists.
fn entry_survives(nodes: &[TreeNode], node: &TreeNode) -> bool {
    let Some(index) = node.option_index() else {
        return true;
    };
    let entry_count = node
        .parent()
        .and_then(|select| nodes.get(select.index()))
        .and_then(TreeNode::raw_attributes)
        .and_then(entries)
        .map_or(0, <[SelectOption]>::len);
    usize::try_from(index).is_ok_and(|index| index < entry_count)
}

fn entries(raw: &ContentAttributes) -> Option<&[SelectOption]> {
    match raw.content_data.as_ref()? {
        ContentData::FormControl(control) => Some(&control.select_options),
        _ => None,
    }
}

impl SnapshotTree {
    /// `id`'s place in its select when it is an option slot, the only address the page has for
    /// it: hydration hands the select over with `index` and `count` (`Handover::member`). `None`
    /// for every other node, a withheld slot included.
    #[must_use]
    pub fn option_slot(&self, id: NodeId) -> Option<OptionSlot> {
        let node = self.node(id)?;
        let index = node.option_index()?;
        let select = node.parent()?;
        // The slots are the select's contiguous last children, so the last one's index gives the
        // list's length in O(1).
        let last = self
            .node(self.node(select)?.last_child()?)?
            .option_index()?;
        Some(OptionSlot {
            select,
            index,
            count: last.checked_add(1)?,
        })
    }

    /// The node at `place`, the reverse of [`Self::option_slot`]: the node APC sent under
    /// `place.key`, else, for an `<option>` its select lists, that select's slot at `index`. A
    /// select whose slot count differs from `count` answers `None`: the list changed since the
    /// capture, so `index` may name another option, and the caller captures again.
    #[must_use]
    pub fn node_at(&self, place: &ElementPlace) -> Option<&TreeNode> {
        if let Some(node) = self.node_by_key(&place.key) {
            return Some(node);
        }
        let option = place.option.as_ref()?;
        let last = self.node(self.node_by_key(&option.select)?.last_child()?)?;
        let count = last.option_index()?.checked_add(1)?;
        if count != option.count || option.index >= count {
            return None;
        }
        let first = last.id().raw().checked_sub(count - 1)?;
        self.node(NodeId::new(first.checked_add(option.index)?))
    }
}
