use std::collections::HashMap;
use std::fmt;
use std::iter::FusedIterator;

use crate::proto::{
    ClickabilityReason, ContentAttributeType, ContentAttributes, ContentData, FormControlData,
    FormControlType,
};

use super::node_key::{DocId, DomNodeId, NodeId, NodeKey};
use super::role::Role;
use super::tree_doc::TreeDoc;
use super::tree_node::TreeNode;
use super::{hidden, role};

const FLAG_CLICKABLE: u8 = 1 << 0;
const FLAG_PRUNABLE: u8 = 1 << 1;

/// Pairing between an ARIA-visible zero-size `<input>` control and the visible `<label>` that
/// receives pointer clicks on its behalf.
///
/// Used for visually collapsed checkbox, radio, and file inputs (`0x0` outer bounding box) so
/// snapshot emission folds the `<label>` into the control's line and click actuation can target
/// the pointer-receiving `<label>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClickProxy {
    /// Arena index of the zero-size `<input>` control.
    pub control: NodeId,
    /// Arena index of the visible pointer-receiving `<label>` paired with [`Self::control`].
    pub label_node: NodeId,
    /// Composite cross-frame key of [`Self::label_node`].
    pub label_key: NodeKey,
}

/// Iterative walker over direct logical children of a [`TreeNode`] in flat-tree preorder within
/// the same document.
///
/// Skips `ua_shadow` text nodes and transparently descends through `ua_shadow` containers using an
/// explicit heap stack so page-authored descendants hoist to the nearest non-`ua_shadow` ancestor
/// in document order without crossing document boundaries.
#[must_use]
#[derive(Clone)]
pub struct LogicalChildrenIter<'a> {
    nodes: &'a [TreeNode],
    doc: DocId,
    stack: Vec<NodeId>,
}

impl<'a> LogicalChildrenIter<'a> {
    fn new(nodes: &'a [TreeNode], parent: NodeId) -> Self {
        let Some(parent_node) = nodes.get(parent.index()) else {
            return Self::empty(nodes);
        };
        let doc = parent_node.doc();
        let mut stack = Vec::new();
        push_children_reversed(nodes, parent, doc, &mut stack);
        Self { nodes, doc, stack }
    }

    const fn empty(nodes: &'a [TreeNode]) -> Self {
        Self {
            nodes,
            doc: DocId::MAIN,
            stack: Vec::new(),
        }
    }
}

impl Iterator for LogicalChildrenIter<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(child_id) = self.stack.pop() {
            let Some(child) = self.nodes.get(child_id.index()) else {
                continue;
            };
            if child.doc() != self.doc {
                continue;
            }
            if let Some(raw) = child.raw_attributes()
                && hidden::is_ua_shadow(raw)
            {
                if !hidden::is_text_node(raw) {
                    push_children_reversed(self.nodes, child_id, self.doc, &mut self.stack);
                }
                continue;
            }
            return Some(child_id);
        }
        None
    }
}

impl FusedIterator for LogicalChildrenIter<'_> {}

impl fmt::Debug for LogicalChildrenIter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogicalChildrenIter")
            .field("remaining_stack", &self.stack.len())
            .finish_non_exhaustive()
    }
}

/// Precomputed logical folding and pruning overlay over a `SnapshotTree`'s flat-tree arena.
///
/// Keeps raw arena indices intact while exposing zero-size control [`ClickProxy`] pairings,
/// folded-label targets, clickable-container flags, plain-container pruning decisions, and
/// `ua_shadow`-transparent logical child iteration. Its manual `Debug` implementation prints
/// counts only, never page text.
pub struct FoldInfo {
    click_proxies: Vec<ClickProxy>,
    click_proxy_index: HashMap<NodeId, usize>,
    folded_label_for: HashMap<NodeId, NodeId>,
    node_flags: Vec<u8>,
    clickable_count: usize,
    prunable_count: usize,
}

impl FoldInfo {
    /// Returns the [`ClickProxy`] pairing for zero-size control `control`, if one was found.
    #[must_use]
    pub fn click_proxy(&self, control: NodeId) -> Option<&ClickProxy> {
        if self.click_proxy_index.is_empty() {
            return None;
        }
        self.click_proxy_index
            .get(&control)
            .and_then(|&index| self.click_proxies.get(index))
    }

    /// All zero-size control [`ClickProxy`] pairings in flat-tree document order.
    #[must_use]
    pub fn click_proxies(&self) -> &[ClickProxy] {
        &self.click_proxies
    }

    /// Returns the paired zero-size control [`NodeId`] when `label` is folded into that control's
    /// snapshot line.
    #[must_use]
    pub fn folded_label_target(&self, label: NodeId) -> Option<NodeId> {
        if self.folded_label_for.is_empty() {
            return None;
        }
        self.folded_label_for.get(&label).copied()
    }

    /// Whether `node` is a non-text, non-`ua_shadow` element whose `clickability_reasons` carry
    /// [`ClickabilityReason::CursorPointer`] or [`ClickabilityReason::ClickHandler`].
    #[must_use]
    pub fn is_clickable_container(&self, node: NodeId) -> bool {
        self.node_flags
            .get(node.index())
            .is_some_and(|&flags| (flags & FLAG_CLICKABLE) != 0)
    }

    /// Whether `node` is a plain `div`/`span` container carrying no ARIA role, clickability,
    /// focusability, active focus, own `aria-hidden` or `inert` boundary, or accessible label
    /// attributes, and can therefore be pruned during snapshot emission.
    #[must_use]
    pub fn is_plain_prunable_container(&self, node: NodeId) -> bool {
        self.node_flags
            .get(node.index())
            .is_some_and(|&flags| (flags & FLAG_PRUNABLE) != 0)
    }

    /// Iterates direct logical children of `parent` within its document, transparently folding
    /// `ua_shadow` containers while skipping `ua_shadow` text nodes.
    pub fn logical_children<'a>(
        &self,
        nodes: &'a [TreeNode],
        parent: NodeId,
    ) -> LogicalChildrenIter<'a> {
        if parent.index() >= self.node_flags.len() {
            return LogicalChildrenIter::empty(nodes);
        }
        logical_children_iter(nodes, parent)
    }
}

impl fmt::Debug for FoldInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FoldInfo")
            .field("click_proxy_count", &self.click_proxies.len())
            .field("folded_label_count", &self.folded_label_for.len())
            .field("clickable_count", &self.clickable_count)
            .field("prunable_count", &self.prunable_count)
            .finish_non_exhaustive()
    }
}

/// Returns a [`LogicalChildrenIter`] over direct logical children of `parent` within its document.
pub(crate) fn logical_children_iter(nodes: &[TreeNode], parent: NodeId) -> LogicalChildrenIter<'_> {
    LogicalChildrenIter::new(nodes, parent)
}

/// Visits direct logical children of `parent` within its document in preorder without allocating
/// on the common path where no child is a `ua_shadow` container.
pub(crate) fn for_each_logical_child(
    nodes: &[TreeNode],
    parent: NodeId,
    mut visit: impl FnMut(NodeId),
) {
    let Some(parent_node) = nodes.get(parent.index()) else {
        return;
    };
    let doc = parent_node.doc();
    let mut shadow_stack = Vec::new();
    let mut cursor = parent_node.first_child();
    while let Some(child_id) = cursor {
        let Some(child) = nodes.get(child_id.index()) else {
            break;
        };
        if child.doc() == doc {
            if let Some(raw) = child.raw_attributes()
                && hidden::is_ua_shadow(raw)
            {
                if !hidden::is_text_node(raw) {
                    push_children_reversed(nodes, child_id, doc, &mut shadow_stack);
                    while let Some(desc_id) = shadow_stack.pop() {
                        let Some(desc) = nodes.get(desc_id.index()) else {
                            continue;
                        };
                        if desc.doc() != doc {
                            continue;
                        }
                        if let Some(desc_raw) = desc.raw_attributes()
                            && hidden::is_ua_shadow(desc_raw)
                        {
                            if !hidden::is_text_node(desc_raw) {
                                push_children_reversed(nodes, desc_id, doc, &mut shadow_stack);
                            }
                            continue;
                        }
                        visit(desc_id);
                    }
                }
            } else {
                visit(child_id);
            }
        }
        cursor = child.next_sibling();
    }
}

/// Computes the immutable logical folding and pruning overlay for `nodes` and `docs` in O(N)
/// iterative time.
pub(crate) fn compute_fold(nodes: &[TreeNode], docs: &[TreeDoc]) -> FoldInfo {
    let bottom_up = compute_bottom_up(nodes);
    let mut top_down = compute_top_down(nodes, &bottom_up);
    let proxies = resolve_click_proxies(
        nodes,
        docs,
        &top_down.labels_for,
        &top_down.nearest_ancestor_unlinked_label,
        &bottom_up.first_labelable,
    );
    settle_label_targets(nodes, &proxies, &mut top_down);

    FoldInfo {
        click_proxies: proxies.click_proxies,
        click_proxy_index: proxies.click_proxy_index,
        folded_label_for: proxies.folded_label_for,
        node_flags: top_down.node_flags,
        clickable_count: top_down.clickable_count,
        prunable_count: top_down.prunable_count,
    }
}

struct DescendantSummary {
    first_labelable: Vec<Option<NodeId>>,
    has_live: Vec<bool>,
    /// Whether a same-document descendant is a target the page draws ([`is_drawn_target`]): what
    /// keeps a `tabindex` wrapper from taking clicks of its own.
    has_drawn_target_below: Vec<bool>,
}

struct TopDownSummary {
    labels_for: HashMap<(DocId, DomNodeId), Vec<NodeId>>,
    nearest_ancestor_unlinked_label: Vec<Option<NodeId>>,
    node_flags: Vec<u8>,
    clickable_count: usize,
    prunable_count: usize,
}

struct ClickProxyTables {
    click_proxies: Vec<ClickProxy>,
    click_proxy_index: HashMap<NodeId, usize>,
    folded_label_for: HashMap<NodeId, NodeId>,
}

fn push_children_reversed(
    nodes: &[TreeNode],
    parent_id: NodeId,
    doc: DocId,
    stack: &mut Vec<NodeId>,
) {
    let start = stack.len();
    let mut cursor = nodes.get(parent_id.index()).and_then(TreeNode::first_child);
    while let Some(child_id) = cursor {
        let Some(child) = nodes.get(child_id.index()) else {
            break;
        };
        if child.doc() == doc {
            stack.push(child_id);
        }
        cursor = child.next_sibling();
    }
    if let Some(added) = stack.get_mut(start..) {
        added.reverse();
    }
}

fn same_doc_parent(nodes: &[TreeNode], node: &TreeNode) -> Option<NodeId> {
    node.parent().filter(|&parent_id| {
        nodes
            .get(parent_id.index())
            .is_some_and(|parent| parent.doc() == node.doc())
    })
}

fn form_control_data(raw: &ContentAttributes) -> Option<&FormControlData> {
    match raw.content_data.as_ref()? {
        ContentData::FormControl(control) => Some(control),
        _ => None,
    }
}

fn form_control_type(raw: &ContentAttributes) -> Option<FormControlType> {
    let control = form_control_data(raw)?;
    FormControlType::try_from(control.form_control_type?).ok()
}

fn is_labelable_element(node: &TreeNode) -> bool {
    let Some(raw) = node.raw_attributes() else {
        return false;
    };
    if hidden::is_text_node(raw) || hidden::is_ua_shadow(raw) {
        return false;
    }
    let Some(tag) = hidden::raw_tag(raw) else {
        return false;
    };
    if tag.eq_ignore_ascii_case("input") {
        return form_control_type(raw) != Some(FormControlType::InputHidden);
    }
    tag.eq_ignore_ascii_case("button")
        || tag.eq_ignore_ascii_case("meter")
        || tag.eq_ignore_ascii_case("output")
        || tag.eq_ignore_ascii_case("progress")
        || tag.eq_ignore_ascii_case("select")
        || tag.eq_ignore_ascii_case("textarea")
}

fn is_unlinked_label(node: &TreeNode) -> bool {
    let Some(raw) = node.raw_attributes() else {
        return false;
    };
    if hidden::is_text_node(raw) || hidden::is_ua_shadow(raw) {
        return false;
    }
    raw.label_for_dom_node_id.is_none()
        && hidden::raw_tag(raw).is_some_and(|tag| tag.eq_ignore_ascii_case("label"))
}

fn compute_bottom_up(nodes: &[TreeNode]) -> DescendantSummary {
    let mut first_labelable: Vec<Option<NodeId>> = vec![None; nodes.len()];
    let mut has_live = vec![false; nodes.len()];
    let mut has_drawn_target_below = vec![false; nodes.len()];

    for index in (0..nodes.len()).rev() {
        let Some(node) = nodes.get(index) else {
            continue;
        };
        let Some(parent_id) = same_doc_parent(nodes, node) else {
            continue;
        };

        let is_live_element = node.raw_attributes().is_some_and(|raw| {
            !hidden::is_text_node(raw) && !hidden::is_ua_shadow(raw) && !node.state().inert()
        });
        let child_has_live = has_live.get(index).copied().unwrap_or(false);
        if (is_live_element || child_has_live)
            && let Some(parent_slot) = has_live.get_mut(parent_id.index())
        {
            *parent_slot = true;
        }

        let drawn_below = has_drawn_target_below.get(index).copied().unwrap_or(false);
        if (drawn_below || is_drawn_target(node))
            && let Some(parent_slot) = has_drawn_target_below.get_mut(parent_id.index())
        {
            *parent_slot = true;
        }

        let labelable_candidate = if is_labelable_element(node) {
            Some(node.id())
        } else {
            first_labelable.get(index).copied().flatten()
        };
        if let Some(labelable_id) = labelable_candidate
            && let Some(parent_slot) = first_labelable.get_mut(parent_id.index())
            && parent_slot.is_none_or(|existing| labelable_id < existing)
        {
            *parent_slot = Some(labelable_id);
        }
    }

    DescendantSummary {
        first_labelable,
        has_live,
        has_drawn_target_below,
    }
}

/// A node the page draws that holds a line of its own: a box Chromium reports clickable for any
/// reason (a control, an ARIA role, a pointer cursor, another target), a focusable one, or a
/// section the text prints with its ref. A `tabindex` wrapper around one is a container and not a
/// target itself, as the old marking's leaf rule had it (`pointer-targets.ts`'s `isOwnTarget`):
/// as a target it would fold that line into its own, and marking must never take a line away.
fn is_drawn_target(node: &TreeNode) -> bool {
    let Some(raw) = node.raw_attributes() else {
        return false;
    };
    if hidden::is_text_node(raw) || hidden::is_ua_shadow(raw) || !node.hidden().is_box_visible() {
        return false;
    }
    node.role().is_some_and(is_section_role)
        || raw.interaction_info.as_ref().is_some_and(|info| {
            !info.clickability_reasons.is_empty() || info.is_focusable == Some(true)
        })
}

/// The roles the v1 text prints as a section line with its ref (`neo_snapshot`'s section rule,
/// the old marking's `SECTION_ROLES`).
const fn is_section_role(role: Role) -> bool {
    matches!(
        role,
        Role::Banner
            | Role::Navigation
            | Role::Main
            | Role::Complementary
            | Role::Contentinfo
            | Role::Search
            | Role::Form
            | Role::Region
            | Role::Article
    )
}

fn is_clickable_element(raw: &ContentAttributes) -> bool {
    let Some(info) = raw.interaction_info.as_ref() else {
        return false;
    };
    let cursor_pointer = i32::from(ClickabilityReason::CursorPointer);
    let click_handler = i32::from(ClickabilityReason::ClickHandler);
    info.clickability_reasons
        .iter()
        .any(|&code| code == cursor_pointer || code == click_handler)
}

/// Whether Chromium reports `raw` editable (`contenteditable`) while its parent is not: the
/// editing host, which an agent fills or types into, and not every element inside it.
fn is_editing_host(raw: &ContentAttributes, parent: Option<&ContentAttributes>) -> bool {
    has_reason(raw, ClickabilityReason::Editable)
        && !parent.is_some_and(|parent| has_reason(parent, ClickabilityReason::Editable))
}

/// Whether `raw` takes focus by its `tabindex` in the tab order (`tabindex >= 0`). A
/// `tabindex="-1"` shell, focusable from script only, is not.
fn is_tabbable_wrapper(raw: &ContentAttributes) -> bool {
    has_reason(raw, ClickabilityReason::TabIndex)
        && raw
            .interaction_info
            .as_ref()
            .is_some_and(|info| info.is_tabbable == Some(true))
}

fn has_reason(raw: &ContentAttributes, reason: ClickabilityReason) -> bool {
    raw.interaction_info
        .as_ref()
        .is_some_and(|info| info.clickability_reasons.contains(&i32::from(reason)))
}

fn is_focusable_element(raw: &ContentAttributes) -> bool {
    let Some(info) = raw.interaction_info.as_ref() else {
        return false;
    };
    if info.is_focusable == Some(true) {
        return true;
    }
    let tab_index = i32::from(ClickabilityReason::TabIndex);
    info.clickability_reasons.contains(&tab_index)
}

fn has_accessible_label_attr(raw: &ContentAttributes) -> bool {
    let Some(browseros) = raw.browseros.as_deref() else {
        return false;
    };
    let has_aria_label = browseros
        .aria_label
        .as_deref()
        .is_some_and(|value| !role::trim_js_whitespace(value).is_empty());
    let has_labelledby = !browseros.labelledby.is_empty();
    let has_title = browseros
        .title
        .as_deref()
        .is_some_and(|value| !role::trim_js_whitespace(value).is_empty());
    has_aria_label || has_labelledby || has_title
}

fn is_plain_prunable_element(
    node: &TreeNode,
    raw: &ContentAttributes,
    parent_logical_inert: bool,
) -> bool {
    if raw.attribute_type == Some(i32::from(ContentAttributeType::Root))
        || node.is_iframe_boundary()
    {
        return false;
    }
    let is_div_or_span = hidden::raw_tag(raw)
        .is_none_or(|tag| tag.eq_ignore_ascii_case("div") || tag.eq_ignore_ascii_case("span"));
    if !is_div_or_span || node.role().is_some() {
        return false;
    }
    if is_focusable_element(raw) || node.state().active() || node.hidden().has_own_aria_hidden() {
        return false;
    }
    if node.state().inert() && !parent_logical_inert {
        return false;
    }
    !has_accessible_label_attr(raw)
}

fn compute_top_down(nodes: &[TreeNode], bottom_up: &DescendantSummary) -> TopDownSummary {
    let mut labels_for: HashMap<(DocId, DomNodeId), Vec<NodeId>> = HashMap::new();
    let mut nearest_ancestor_unlinked_label: Vec<Option<NodeId>> = Vec::with_capacity(nodes.len());
    let mut unlinked_label_for_descendants: Vec<Option<NodeId>> = Vec::with_capacity(nodes.len());
    let mut logical_inert_for_descendants: Vec<bool> = Vec::with_capacity(nodes.len());
    let mut node_flags: Vec<u8> = Vec::with_capacity(nodes.len());
    let mut clickable_count: usize = 0;
    let mut prunable_count: usize = 0;

    for (index, node) in nodes.iter().enumerate() {
        let parent_id = same_doc_parent(nodes, node);
        let parent_unlinked_label = parent_id
            .and_then(|pid| unlinked_label_for_descendants.get(pid.index()).copied())
            .flatten();
        let parent_logical_inert = parent_id
            .and_then(|pid| logical_inert_for_descendants.get(pid.index()).copied())
            .unwrap_or(false);
        let has_live = bottom_up.has_live.get(index).copied().unwrap_or(false);
        // Only an inert ancestor with zero live descendants can itself be coalesced as `inert`;
        // an inert ancestor protecting a live modal stays unmarked so its inert children must not
        // prune into it.
        let self_marks_inert = node.state().inert() && !has_live;

        nearest_ancestor_unlinked_label.push(parent_unlinked_label);
        unlinked_label_for_descendants.push(if is_unlinked_label(node) {
            Some(node.id())
        } else {
            parent_unlinked_label
        });

        let Some(raw) = node.raw_attributes() else {
            logical_inert_for_descendants.push(self_marks_inert);
            node_flags.push(0);
            continue;
        };

        let is_ua_shadow = hidden::is_ua_shadow(raw);
        logical_inert_for_descendants.push(if is_ua_shadow {
            parent_logical_inert
        } else {
            self_marks_inert
        });

        if is_ua_shadow || hidden::is_text_node(raw) {
            node_flags.push(0);
            continue;
        }

        if let Some(target_dom) = raw.label_for_dom_node_id
            && hidden::raw_tag(raw).is_none_or(|tag| tag.eq_ignore_ascii_case("label"))
        {
            labels_for
                .entry((node.doc(), DomNodeId::new(target_dom)))
                .or_default()
                .push(node.id());
        }

        let parent_raw = parent_id
            .and_then(|pid| nodes.get(pid.index()))
            .and_then(TreeNode::raw_attributes);
        let drawn_below = bottom_up
            .has_drawn_target_below
            .get(index)
            .copied()
            .unwrap_or(false);
        // An iframe is a frame host: what it holds is its own document's, which this document's
        // guard cannot see, and focus on it moves into that document. It never takes clicks by
        // its `tabindex`.
        let is_clickable = is_clickable_element(raw)
            || is_editing_host(raw, parent_raw)
            || (is_tabbable_wrapper(raw) && !drawn_below && !node.is_iframe_boundary());
        let is_prunable =
            !is_clickable && is_plain_prunable_element(node, raw, parent_logical_inert);
        let mut flags: u8 = 0;
        if is_clickable {
            flags |= FLAG_CLICKABLE;
            clickable_count += 1;
        }
        if is_prunable {
            flags |= FLAG_PRUNABLE;
            prunable_count += 1;
        }
        node_flags.push(flags);
    }

    TopDownSummary {
        labels_for,
        nearest_ancestor_unlinked_label,
        node_flags,
        clickable_count,
        prunable_count,
    }
}

/// A `<label>` shares its control's ref (a click on it acts on the control). A label whose
/// control the tree draws, or pairs with it as a click proxy, is no target of its own, whatever
/// cursor or listener it has. A label whose control the tree holds no drawn node for, as a
/// `display: none` checkbox behind a styled label, is the control's only target, so it takes
/// clicks and keeps a ref of its own.
fn settle_label_targets(
    nodes: &[TreeNode],
    proxies: &ClickProxyTables,
    top_down: &mut TopDownSummary,
) {
    if top_down.labels_for.is_empty() {
        return;
    }
    let by_dom: HashMap<(DocId, DomNodeId), NodeId> = nodes
        .iter()
        .filter_map(|node| Some(((node.doc(), node.dom_node_id()?), node.id())))
        .collect();
    for (key, labels) in &top_down.labels_for {
        let control_stands = by_dom.get(key).is_some_and(|&control| {
            proxies.click_proxy_index.contains_key(&control)
                || nodes
                    .get(control.index())
                    .is_some_and(|node| node.hidden().is_box_visible())
        });
        for &label in labels {
            if proxies.folded_label_for.contains_key(&label) {
                continue;
            }
            let Some(flags) = top_down.node_flags.get_mut(label.index()) else {
                continue;
            };
            let was_clickable = *flags & FLAG_CLICKABLE != 0;
            let takes_clicks = !control_stands
                && nodes.get(label.index()).is_some_and(|node| {
                    !node.hidden().is_hidden_for_aria() && node.hidden().is_box_visible()
                });
            if takes_clicks && !was_clickable {
                *flags |= FLAG_CLICKABLE;
                top_down.clickable_count += 1;
                if *flags & FLAG_PRUNABLE != 0 {
                    *flags &= !FLAG_PRUNABLE;
                    top_down.prunable_count = top_down.prunable_count.saturating_sub(1);
                }
            } else if !takes_clicks && was_clickable {
                *flags &= !FLAG_CLICKABLE;
                top_down.clickable_count = top_down.clickable_count.saturating_sub(1);
            }
        }
    }
}

fn zero_size_control_dom_id(node: &TreeNode) -> Option<DomNodeId> {
    let raw = node.raw_attributes()?;
    if hidden::is_text_node(raw) || hidden::is_ua_shadow(raw) {
        return None;
    }
    let tag = hidden::raw_tag(raw)?;
    if !tag.eq_ignore_ascii_case("input") {
        return None;
    }
    if !matches!(
        form_control_type(raw),
        Some(
            FormControlType::InputCheckbox
                | FormControlType::InputRadio
                | FormControlType::InputFile
        )
    ) {
        return None;
    }
    let dom_id = node.dom_node_id()?;
    let hidden = node.hidden();
    if hidden.is_hidden_for_aria() || hidden.is_display_contents() || hidden.is_box_visible() {
        return None;
    }
    Some(dom_id)
}

fn qualifies_as_click_proxy_label(nodes: &[TreeNode], docs: &[TreeDoc], id: NodeId) -> bool {
    let Some(label) = nodes.get(id.index()) else {
        return false;
    };
    if label.dom_node_id().is_none() || docs.get(label.doc().index()).is_none() {
        return false;
    }
    let hidden = label.hidden();
    !hidden.is_ua_shadow()
        && !hidden.is_hidden_for_aria()
        && hidden.is_box_visible()
        && hidden.receives_pointer_events()
}

fn node_key_for(nodes: &[TreeNode], docs: &[TreeDoc], id: NodeId) -> Option<NodeKey> {
    let node = nodes.get(id.index())?;
    let dom_id = node.dom_node_id()?;
    let doc = docs.get(node.doc().index())?;
    Some(NodeKey::new(doc.token().clone(), dom_id))
}

fn resolve_click_proxies(
    nodes: &[TreeNode],
    docs: &[TreeDoc],
    labels_for: &HashMap<(DocId, DomNodeId), Vec<NodeId>>,
    nearest_ancestor_unlinked_label: &[Option<NodeId>],
    first_labelable_descendant: &[Option<NodeId>],
) -> ClickProxyTables {
    let mut click_proxies = Vec::new();
    let mut click_proxy_index = HashMap::new();
    let mut folded_label_for = HashMap::new();

    for node in nodes {
        let Some(control_dom_id) = zero_size_control_dom_id(node) else {
            continue;
        };

        let explicit_label = labels_for
            .get(&(node.doc(), control_dom_id))
            .and_then(|candidates| {
                candidates
                    .iter()
                    .copied()
                    .find(|&label_id| qualifies_as_click_proxy_label(nodes, docs, label_id))
            });

        let wrapping_label = nearest_ancestor_unlinked_label
            .get(node.id().index())
            .copied()
            .flatten()
            .filter(|&label_id| {
                first_labelable_descendant
                    .get(label_id.index())
                    .copied()
                    .flatten()
                    == Some(node.id())
            })
            .filter(|&label_id| qualifies_as_click_proxy_label(nodes, docs, label_id));

        let chosen_label = match (explicit_label, wrapping_label) {
            (Some(explicit), Some(wrapping)) => Some(explicit.min(wrapping)),
            (Some(explicit), None) => Some(explicit),
            (None, Some(wrapping)) => Some(wrapping),
            (None, None) => None,
        };

        if let Some(label_node) = chosen_label
            && let Some(label_key) = node_key_for(nodes, docs, label_node)
        {
            let proxy_index = click_proxies.len();
            click_proxies.push(ClickProxy {
                control: node.id(),
                label_node,
                label_key,
            });
            click_proxy_index.insert(node.id(), proxy_index);
            folded_label_for.entry(label_node).or_insert(node.id());
        }
    }

    ClickProxyTables {
        click_proxies,
        click_proxy_index,
        folded_label_for,
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::thread;

    use crate::proto::{
        AnnotatedRole, BrowserOsContentAttributes, BrowserOsRef, ClickabilityReason,
        FormControlType,
    };
    use crate::tree::{
        ApcBuilder, BuildOptions, DocToken, DomNodeId, FrameBuilder, NodeBuilder, NodeId, NodeKey,
        PartialIframeSpec, SnapshotTree, TreeNode,
    };

    use super::{ClickProxy, compute_fold, logical_children_iter};

    fn build_tree(root: NodeBuilder) -> Result<SnapshotTree, Box<dyn Error>> {
        let apc = ApcBuilder::new("doc-main").root(root).build();
        Ok(SnapshotTree::build(apc, BuildOptions::default())?)
    }

    fn node_id(
        tree: &SnapshotTree,
        doc_token: &str,
        dom_id: i32,
    ) -> Result<NodeId, Box<dyn Error>> {
        let token = DocToken::new(doc_token).ok_or("empty doc token")?;
        let key = NodeKey::new(token, DomNodeId::new(dom_id));
        tree.node_by_key(&key)
            .map(TreeNode::id)
            .ok_or_else(|| format!("missing node ({doc_token}, {dom_id})").into())
    }

    #[test]
    fn ua_shadow_fold_hoists_descendants_and_stops_at_doc_boundaries() -> Result<(), Box<dyn Error>>
    {
        let iframe = FrameBuilder::new("doc-iframe").root(
            NodeBuilder::root()
                .dom_id(1)
                .child(NodeBuilder::element("p").dom_id(2)),
        );

        let tree = build_tree(
            NodeBuilder::root()
                .dom_id(1)
                .child(NodeBuilder::element("header").dom_id(10))
                .child(
                    NodeBuilder::element("details")
                        .dom_id(20)
                        .child(
                            NodeBuilder::container()
                                .ua_shadow(true)
                                .child(NodeBuilder::text("MARKER").ua_shadow(true))
                                .child(NodeBuilder::element("summary").dom_id(21))
                                .child(
                                    NodeBuilder::container()
                                        .ua_shadow(true)
                                        .annotated_role(AnnotatedRole::ContentHidden)
                                        .child(NodeBuilder::element("p").dom_id(22)),
                                ),
                        )
                        .child(NodeBuilder::element("footer").dom_id(23)),
                )
                .child(NodeBuilder::iframe(iframe).dom_id(30)),
        )?;

        let fold = compute_fold(tree.nodes(), tree.documents());
        let details = node_id(&tree, "doc-main", 20)?;
        let summary = node_id(&tree, "doc-main", 21)?;
        let hidden_body = node_id(&tree, "doc-main", 22)?;
        let footer = node_id(&tree, "doc-main", 23)?;
        let iframe_node = node_id(&tree, "doc-main", 30)?;

        let mut iter = fold.logical_children(tree.nodes(), details);
        let iter_dbg = format!("{iter:?}");
        assert!(
            iter_dbg.contains("remaining_stack"),
            "LogicalChildrenIter Debug must report remaining_stack"
        );
        let hoisted: Vec<NodeId> = iter.by_ref().collect();
        assert_eq!(
            hoisted,
            [summary, hidden_body, footer],
            "ua_shadow text must be skipped while slotted descendants hoist in document order"
        );
        assert_eq!(
            iter.next(),
            None,
            "LogicalChildrenIter must remain fused after exhaustion"
        );
        assert!(
            tree.node(hidden_body)
                .is_some_and(|node| node.hidden().is_content_hidden()),
            "hoisted child under ContentHidden ua_shadow slot must retain is_content_hidden"
        );

        let iframe_logical: Vec<NodeId> =
            logical_children_iter(tree.nodes(), iframe_node).collect();
        assert!(
            iframe_logical.is_empty(),
            "logical children must stop at document boundaries"
        );

        let oob: Vec<NodeId> = fold
            .logical_children(tree.nodes(), NodeId::new(9_999))
            .collect();
        assert!(oob.is_empty(), "out-of-bounds NodeId must yield empty");
        Ok(())
    }

    #[test]
    fn tests_that_a_label_takes_clicks_only_when_no_drawn_node_stands_for_its_control()
    -> Result<(), Box<dyn Error>> {
        let label = |dom: i32, control: i32| {
            NodeBuilder::element("label")
                .dom_id(dom)
                .label_for(control)
                .bounding_box(0, 0, 120, 20)
                .child(NodeBuilder::text("Label"))
        };
        let tree = build_tree(
            NodeBuilder::root()
                .dom_id(1)
                .child(
                    NodeBuilder::form_control(FormControlType::InputText)
                        .tag("input")
                        .dom_id(10)
                        .bounding_box(0, 30, 120, 20),
                )
                .child(label(11, 10).clickability_reason(ClickabilityReason::CursorPointer))
                .child(label(12, 99)),
        )?;
        let fold = tree.fold();
        assert!(
            !fold.is_clickable_container(node_id(&tree, "doc-main", 11)?),
            "a pointer label whose control is drawn shares the control's ref"
        );
        assert!(
            fold.is_clickable_container(node_id(&tree, "doc-main", 12)?),
            "a label whose control the tree does not hold (display: none) is its only target"
        );
        Ok(())
    }

    #[test]
    fn tests_that_editing_hosts_and_tabbable_wrappers_without_drawn_targets_take_clicks()
    -> Result<(), Box<dyn Error>> {
        let wrapper = |dom: i32| {
            NodeBuilder::container()
                .dom_id(dom)
                .clickability_reason(ClickabilityReason::TabIndex)
                .focusable(true)
                .tabbable(true)
                .bounding_box(0, 0, 200, 20)
        };
        let tree = build_tree(
            NodeBuilder::root()
                .dom_id(1)
                .child(
                    NodeBuilder::container()
                        .dom_id(20)
                        .clickability_reason(ClickabilityReason::Editable)
                        .bounding_box(0, 0, 200, 20)
                        .child(
                            NodeBuilder::container()
                                .dom_id(21)
                                .clickability_reason(ClickabilityReason::Editable)
                                .bounding_box(0, 0, 200, 20)
                                .child(NodeBuilder::text("Draft")),
                        ),
                )
                .child(wrapper(30).child(NodeBuilder::text("Tabindex target")))
                .child(
                    wrapper(40).child(
                        NodeBuilder::form_control(FormControlType::ButtonButton)
                            .tag("button")
                            .dom_id(41)
                            .clickability_reason(ClickabilityReason::ClickableControl)
                            .bounding_box(0, 0, 60, 20),
                    ),
                )
                .child(
                    wrapper(50).child(
                        NodeBuilder::form_control(FormControlType::InputCheckbox)
                            .tag("input")
                            .dom_id(51)
                            .clickability_reason(ClickabilityReason::ClickableControl)
                            .bounding_box(0, 0, 0, 0),
                    ),
                )
                .child(
                    NodeBuilder::container()
                        .dom_id(60)
                        .clickability_reason(ClickabilityReason::TabIndex)
                        .focusable(true)
                        .tabbable(false)
                        .bounding_box(0, 0, 400, 400)
                        .child(NodeBuilder::text("App shell")),
                ),
        )?;
        let fold = tree.fold();
        let clickable = |dom: i32| -> Result<bool, Box<dyn Error>> {
            Ok(fold.is_clickable_container(node_id(&tree, "doc-main", dom)?))
        };
        assert!(clickable(20)?, "the editing host");
        assert!(!clickable(21)?, "not an editable element inside it");
        assert!(clickable(30)?, "a tabbable wrapper holding only text");
        assert!(!clickable(40)?, "not one around a drawn button");
        assert!(clickable(50)?, "one whose only control is not drawn");
        assert!(
            !clickable(60)?,
            "not a tabindex=-1 shell, outside the tab order"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_tabbable_iframe_or_a_wrapper_around_a_section_takes_no_clicks()
    -> Result<(), Box<dyn Error>> {
        let wrapper = |dom: i32| {
            NodeBuilder::container()
                .dom_id(dom)
                .clickability_reason(ClickabilityReason::TabIndex)
                .focusable(true)
                .tabbable(true)
                .bounding_box(0, 0, 200, 20)
        };
        let tree = build_tree(
            NodeBuilder::root()
                .dom_id(1)
                .child(
                    NodeBuilder::iframe(
                        FrameBuilder::new("doc-frame").root(
                            NodeBuilder::root().dom_id(1).child(
                                NodeBuilder::form_control(FormControlType::InputText)
                                    .tag("input")
                                    .dom_id(2)
                                    .bounding_box(0, 0, 100, 20),
                            ),
                        ),
                    )
                    .dom_id(70)
                    .clickability_reason(ClickabilityReason::TabIndex)
                    .focusable(true)
                    .tabbable(true)
                    .bounding_box(0, 0, 300, 150),
                )
                .child(
                    wrapper(80).child(NodeBuilder::text("Note")).child(
                        NodeBuilder::container()
                            .dom_id(81)
                            .role("region")
                            .aria_label("Notes")
                            .bounding_box(0, 0, 200, 20)
                            .child(NodeBuilder::text("Region text")),
                    ),
                ),
        )?;
        let fold = tree.fold();
        let clickable = |dom: i32| -> Result<bool, Box<dyn Error>> {
            Ok(fold.is_clickable_container(node_id(&tree, "doc-main", dom)?))
        };
        assert!(
            !clickable(70)?,
            "not a tabindex iframe, whose targets are its document's"
        );
        assert!(
            !clickable(80)?,
            "not one around a drawn section, which keeps its own line"
        );
        Ok(())
    }

    #[test]
    fn click_proxy_pairs_explicit_labels_and_isolates_iframes() -> Result<(), Box<dyn Error>> {
        let iframe = FrameBuilder::new("doc-iframe").root(
            NodeBuilder::root()
                .dom_id(1)
                .child(
                    NodeBuilder::form_control(FormControlType::InputFile)
                        .tag("input")
                        .dom_id(10)
                        .bounding_box(0, 0, 0, 0),
                )
                .child(
                    NodeBuilder::container()
                        .dom_id(11)
                        .label_for(10)
                        .bounding_box(0, 0, 80, 24),
                ),
        );

        let tree = build_tree(
            NodeBuilder::root()
                .dom_id(1)
                .child(
                    NodeBuilder::element("label")
                        .dom_id(100)
                        .label_for(10)
                        .aria_hidden(true)
                        .bounding_box(0, 0, 80, 24),
                )
                .child(
                    NodeBuilder::element("label")
                        .dom_id(101)
                        .label_for(10)
                        .pointer_events_none(true)
                        .bounding_box(0, 0, 80, 24),
                )
                .child(
                    NodeBuilder::form_control(FormControlType::InputCheckbox)
                        .tag("input")
                        .dom_id(10)
                        .bounding_box(0, 0, 0, 0),
                )
                .child(
                    NodeBuilder::element("label")
                        .dom_id(102)
                        .label_for(10)
                        .bounding_box(0, 0, 80, 24),
                )
                .child(NodeBuilder::iframe(iframe).dom_id(400)),
        )?;

        let fold = compute_fold(tree.nodes(), tree.documents());
        let checkbox = node_id(&tree, "doc-main", 10)?;
        let good_label = node_id(&tree, "doc-main", 102)?;
        let iframe_file = node_id(&tree, "doc-iframe", 10)?;
        let iframe_label = node_id(&tree, "doc-iframe", 11)?;

        let main_token = DocToken::new("doc-main").ok_or("empty main token")?;
        let iframe_token = DocToken::new("doc-iframe").ok_or("empty iframe token")?;
        let expected = [
            ClickProxy {
                control: checkbox,
                label_node: good_label,
                label_key: NodeKey::new(main_token, DomNodeId::new(102)),
            },
            ClickProxy {
                control: iframe_file,
                label_node: iframe_label,
                label_key: NodeKey::new(iframe_token, DomNodeId::new(11)),
            },
        ];

        assert_eq!(
            fold.click_proxies(),
            expected,
            "click proxies must appear in document order with doc-scoped keys"
        );
        assert_eq!(fold.click_proxy(checkbox), expected.first());
        assert_eq!(fold.folded_label_target(good_label), Some(checkbox));
        assert_eq!(fold.folded_label_target(iframe_label), Some(iframe_file));
        Ok(())
    }

    #[test]
    fn click_proxy_pairs_wrapping_label_for_first_labelable_only() -> Result<(), Box<dyn Error>> {
        let tree = build_tree(
            NodeBuilder::root().dom_id(1).child(
                NodeBuilder::element("label")
                    .dom_id(200)
                    .bounding_box(0, 0, 100, 24)
                    .child(
                        NodeBuilder::form_control(FormControlType::InputHidden)
                            .tag("input")
                            .dom_id(201),
                    )
                    .child(
                        NodeBuilder::form_control(FormControlType::InputRadio)
                            .tag("input")
                            .dom_id(202)
                            .bounding_box(0, 0, 0, 0),
                    )
                    .child(
                        NodeBuilder::form_control(FormControlType::InputCheckbox)
                            .tag("input")
                            .dom_id(203)
                            .bounding_box(0, 0, 0, 0),
                    ),
            ),
        )?;

        let fold = compute_fold(tree.nodes(), tree.documents());
        let wrap_label = node_id(&tree, "doc-main", 200)?;
        let wrapped_radio = node_id(&tree, "doc-main", 202)?;
        let second_wrapped = node_id(&tree, "doc-main", 203)?;
        let main_token = DocToken::new("doc-main").ok_or("empty main token")?;

        assert_eq!(
            fold.click_proxy(wrapped_radio),
            Some(&ClickProxy {
                control: wrapped_radio,
                label_node: wrap_label,
                label_key: NodeKey::new(main_token, DomNodeId::new(200)),
            }),
            "unlinked wrapping <label> must skip InputHidden and pair with first labelable control"
        );
        assert_eq!(fold.folded_label_target(wrap_label), Some(wrapped_radio));
        assert_eq!(
            fold.click_proxy(second_wrapped),
            None,
            "second labelable descendant under wrapping <label> must not pair"
        );
        Ok(())
    }

    #[test]
    fn click_proxy_rejects_display_contents_visible_and_non_target_inputs()
    -> Result<(), Box<dyn Error>> {
        let tree = build_tree(
            NodeBuilder::root()
                .dom_id(1)
                .child(
                    NodeBuilder::form_control(FormControlType::InputCheckbox)
                        .tag("input")
                        .dom_id(300)
                        .display_contents(true)
                        .bounding_box(0, 0, 0, 0)
                        .child(NodeBuilder::element("span").bounding_box(0, 0, 0, 0)),
                )
                .child(
                    NodeBuilder::element("label")
                        .dom_id(301)
                        .label_for(300)
                        .bounding_box(0, 0, 80, 24),
                )
                .child(
                    NodeBuilder::form_control(FormControlType::InputCheckbox)
                        .tag("input")
                        .dom_id(310)
                        .bounding_box(0, 0, 16, 16),
                )
                .child(
                    NodeBuilder::element("label")
                        .dom_id(311)
                        .label_for(310)
                        .bounding_box(0, 0, 80, 24),
                )
                .child(
                    NodeBuilder::form_control(FormControlType::InputText)
                        .tag("input")
                        .dom_id(320)
                        .bounding_box(0, 0, 0, 0),
                )
                .child(
                    NodeBuilder::element("label")
                        .dom_id(321)
                        .label_for(320)
                        .bounding_box(0, 0, 80, 24),
                ),
        )?;

        let fold = compute_fold(tree.nodes(), tree.documents());
        let display_contents_cb = node_id(&tree, "doc-main", 300)?;
        let visible_cb = node_id(&tree, "doc-main", 310)?;
        let text_input = node_id(&tree, "doc-main", 320)?;

        assert!(
            tree.node(display_contents_cb)
                .is_some_and(|node| !node.hidden().is_hidden_for_aria()),
            "display: contents fixture must remain ARIA-visible so display: contents is the exclusion"
        );
        assert_eq!(
            fold.click_proxy(display_contents_cb),
            None,
            "display: contents input must never be taken as a zero-size control"
        );
        assert_eq!(
            fold.click_proxy(visible_cb),
            None,
            "box-visible checkbox must not get a click proxy"
        );
        assert_eq!(
            fold.click_proxy(text_input),
            None,
            "non-checkbox/radio/file input must not get a click proxy"
        );
        assert!(fold.click_proxies().is_empty());
        Ok(())
    }

    #[test]
    fn clickable_and_basic_prunable_container_rules() -> Result<(), Box<dyn Error>> {
        let apc = ApcBuilder::new("doc-main")
            .main_focused_node_id(40)
            .root(
                NodeBuilder::root()
                    .dom_id(1)
                    .child(NodeBuilder::container().dom_id(10))
                    .child(NodeBuilder::element("div").dom_id(11))
                    .child(NodeBuilder::element("SPAN").dom_id(12))
                    .child(
                        NodeBuilder::element("div")
                            .dom_id(20)
                            .clickability_reason(ClickabilityReason::CursorPointer),
                    )
                    .child(
                        NodeBuilder::element("span")
                            .dom_id(21)
                            .clickability_reason(ClickabilityReason::ClickHandler),
                    )
                    .child(
                        NodeBuilder::element("div")
                            .dom_id(22)
                            .clickability_reason(ClickabilityReason::HoverPseudoClass),
                    )
                    .child(NodeBuilder::element("div").dom_id(30).focusable(true))
                    .child(
                        NodeBuilder::element("div")
                            .dom_id(31)
                            .clickability_reason(ClickabilityReason::TabIndex),
                    )
                    .child(NodeBuilder::element("div").dom_id(40))
                    .child(NodeBuilder::element("div").dom_id(73).role("generic"))
                    .child(NodeBuilder::element("section").dom_id(74))
                    .child(NodeBuilder::partial_iframe(PartialIframeSpec::CrossSite).dom_id(75)),
            )
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::default())?;
        let fold = compute_fold(tree.nodes(), tree.documents());

        for dom in [10, 11, 12, 22] {
            let id = node_id(&tree, "doc-main", dom)?;
            assert!(
                fold.is_plain_prunable_container(id),
                "dom_id {dom} must be a plain prunable container"
            );
        }
        for dom in [1, 20, 21, 30, 31, 40, 73, 74, 75] {
            let id = node_id(&tree, "doc-main", dom)?;
            assert!(
                !fold.is_plain_prunable_container(id),
                "dom_id {dom} must NOT be pruned"
            );
        }

        assert!(fold.is_clickable_container(node_id(&tree, "doc-main", 20)?));
        assert!(fold.is_clickable_container(node_id(&tree, "doc-main", 21)?));
        assert!(!fold.is_clickable_container(node_id(&tree, "doc-main", 22)?));
        Ok(())
    }

    #[test]
    fn prunable_container_respects_aria_hidden_inert_and_label_boundaries()
    -> Result<(), Box<dyn Error>> {
        let tree = build_tree(
            NodeBuilder::root()
                .dom_id(1)
                .child(
                    NodeBuilder::element("div")
                        .dom_id(13)
                        .aria_label(" \t\u{00A0}\u{FEFF} ")
                        .title("  "),
                )
                .child(
                    NodeBuilder::element("div")
                        .dom_id(50)
                        .aria_hidden(true)
                        .child(NodeBuilder::element("div").dom_id(51)),
                )
                .child(
                    NodeBuilder::element("div")
                        .dom_id(60)
                        .inert(true)
                        .child(
                            NodeBuilder::container()
                                .ua_shadow(true)
                                .child(NodeBuilder::element("div").dom_id(61).inert(true)),
                        )
                        .child(
                            NodeBuilder::element("dialog")
                                .dom_id(62)
                                .child(NodeBuilder::element("div").dom_id(63).inert(true)),
                        ),
                )
                .child(
                    NodeBuilder::element("div").dom_id(64).inert(true).child(
                        NodeBuilder::container()
                            .ua_shadow(true)
                            .child(NodeBuilder::element("div").dom_id(65).inert(true)),
                    ),
                )
                .child(
                    NodeBuilder::element("div")
                        .dom_id(70)
                        .aria_label("\u{0085}"),
                )
                .child(
                    NodeBuilder::element("div")
                        .dom_id(71)
                        .labelledby(vec![BrowserOsRef {
                            dom_node_id: None,
                            unrendered_text: Some("Label".to_owned()),
                        }]),
                )
                .child(NodeBuilder::element("div").dom_id(72).title("Hint")),
        )?;
        let fold = compute_fold(tree.nodes(), tree.documents());

        for dom in [13, 51, 65] {
            let id = node_id(&tree, "doc-main", dom)?;
            assert!(
                fold.is_plain_prunable_container(id),
                "dom_id {dom} must be prunable (blank attrs or pure inherited boundary)"
            );
        }
        for dom in [50, 60, 61, 62, 63, 64, 70, 71, 72] {
            let id = node_id(&tree, "doc-main", dom)?;
            assert!(
                !fold.is_plain_prunable_container(id),
                "dom_id {dom} must NOT be pruned (own boundary, live-protecting inert parent, or non-blank label)"
            );
        }
        Ok(())
    }

    #[test]
    fn deep_ua_shadow_and_container_chain_folds_iteratively_without_stack_overflow()
    -> Result<(), Box<dyn Error>> {
        let handle = thread::Builder::new()
            .name("fold-deep-stack-test".to_owned())
            .stack_size(2 * 1024 * 1024)
            .spawn(|| -> Result<(usize, usize), String> {
                let mut apc = ApcBuilder::deep_chain(25_000);
                if let Some(root) = apc.root_node.as_mut() {
                    let mut cursor = root;
                    let mut depth: usize = 0;
                    while let Some(child) = cursor.children_nodes.first_mut() {
                        depth += 1;
                        if depth < 24_999
                            && let Some(attrs) = child.content_attributes.as_mut()
                        {
                            attrs.browseros = Some(Box::new(BrowserOsContentAttributes {
                                ua_shadow: Some(true),
                                ..BrowserOsContentAttributes::default()
                            }));
                        }
                        cursor = child;
                    }
                }

                let tree = SnapshotTree::build(apc, BuildOptions::default())
                    .map_err(|err| err.to_string())?;
                let fold = compute_fold(tree.nodes(), tree.documents());
                let hoisted: Vec<NodeId> = fold
                    .logical_children(tree.nodes(), NodeId::new(0))
                    .collect();
                let last_id =
                    NodeId::new(u32::try_from(tree.nodes().len().saturating_sub(1)).unwrap_or(0));
                if hoisted.as_slice() != [last_id] {
                    return Err(format!(
                        "expected hoisted leaf {last_id:?}, got {hoisted:?}"
                    ));
                }
                let prunable_leaf = usize::from(fold.is_plain_prunable_container(last_id));
                Ok((hoisted.len(), prunable_leaf))
            })?;

        let (hoisted_count, prunable_leaf) = handle.join().map_err(|panic_payload| {
            format!("deep stack test thread panicked: {panic_payload:?}")
        })??;
        assert_eq!(
            hoisted_count, 1,
            "25,000-deep ua_shadow chain must hoist the single page leaf iteratively"
        );
        assert_eq!(
            prunable_leaf, 1,
            "page leaf at bottom of 25,000-deep ua_shadow chain must be prunable"
        );
        Ok(())
    }
}
