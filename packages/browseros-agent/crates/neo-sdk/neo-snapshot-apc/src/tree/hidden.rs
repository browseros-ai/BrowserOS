use std::fmt;

use crate::proto::{AnnotatedRole, ContentAttributeType, ContentAttributes, ContentData};

use super::tree_node::TreeNode;

const HIDDEN_FOR_ARIA: u16 = 1 << 0;
const BOX_VISIBLE: u16 = 1 << 1;
const ARIA_HIDDEN: u16 = 1 << 2;
const OWN_ARIA_HIDDEN: u16 = 1 << 3;
const VISIBILITY_HIDDEN: u16 = 1 << 4;
const CONTENT_HIDDEN: u16 = 1 << 5;
const OWN_CONTENT_HIDDEN: u16 = 1 << 6;
const INERT: u16 = 1 << 7;
const UA_SHADOW: u16 = 1 << 8;
const DISPLAY_CONTENTS: u16 = 1 << 9;
const POINTER_EVENTS_NONE: u16 = 1 << 10;

/// Precomputed ARIA visibility, box visibility, and folding inputs for one [`TreeNode`].
///
/// Computed in two iterative passes over the flat-tree arena (top-down preorder for inherited
/// `aria-hidden` and `content-visibility` state, followed by bottom-up reverse preorder for
/// `display: contents` wrappers) so snapshot rendering and `getByRole` share identical visibility
/// decisions without recursion or page-text inspection.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Hidden(u16);

impl Hidden {
    /// Whether this node is excluded from the ARIA tree (`isElementHiddenForAria` parity).
    #[must_use]
    pub const fn is_hidden_for_aria(self) -> bool {
        (self.0 & HIDDEN_FOR_ARIA) != 0
    }

    /// Whether this node renders a non-empty layout box (`computeBox().visible` parity).
    #[must_use]
    pub const fn is_box_visible(self) -> bool {
        (self.0 & BOX_VISIBLE) != 0
    }

    /// Whether `aria-hidden="true"` applies to this node (inherited within the same document,
    /// including onto text nodes).
    #[must_use]
    pub const fn is_aria_hidden(self) -> bool {
        (self.0 & ARIA_HIDDEN) != 0
    }

    /// Whether this node starts an `aria-hidden="true"` boundary (raw `browseros.aria_hidden` is
    /// true here while the immediate same-document parent does not carry raw `aria_hidden`).
    #[must_use]
    pub const fn has_own_aria_hidden(self) -> bool {
        (self.0 & OWN_ARIA_HIDDEN) != 0
    }

    /// Whether this element's own computed CSS `visibility` is hidden (element nodes only, never
    /// inherited because descendants can override `visibility: visible`).
    #[must_use]
    pub const fn is_visibility_hidden(self) -> bool {
        (self.0 & VISIBILITY_HIDDEN) != 0
    }

    /// Whether `AnnotatedRole::ContentHidden` applies to this node or a same-document ancestor
    /// (including propagation through a user-agent shadow slot such as a closed `<details>`).
    #[must_use]
    pub const fn is_content_hidden(self) -> bool {
        (self.0 & CONTENT_HIDDEN) != 0
    }

    /// Whether this node itself carries `AnnotatedRole::ContentHidden` (folding input).
    #[must_use]
    pub const fn has_own_content_hidden(self) -> bool {
        (self.0 & OWN_CONTENT_HIDDEN) != 0
    }

    /// Whether this element's computed style is inert (element nodes only, never inherited because
    /// a modal `<dialog>` escapes an inert `<body>`).
    #[must_use]
    pub const fn is_inert(self) -> bool {
        (self.0 & INERT) != 0
    }

    /// Whether this node lives inside a user-agent shadow root (never inherited so page content
    /// slotted under `<details>` stays readable).
    #[must_use]
    pub const fn is_ua_shadow(self) -> bool {
        (self.0 & UA_SHADOW) != 0
    }

    /// Whether this element uses CSS `display: contents` and derives visibility from its direct
    /// same-document children.
    #[must_use]
    pub const fn is_display_contents(self) -> bool {
        (self.0 & DISPLAY_CONTENTS) != 0
    }

    /// Whether this element's computed CSS `pointer-events` is `none` (element nodes only, never
    /// set by `inert` alone).
    #[must_use]
    pub const fn is_pointer_events_none(self) -> bool {
        (self.0 & POINTER_EVENTS_NONE) != 0
    }

    /// Whether this node can receive pointer events (`!self.is_pointer_events_none()`).
    #[must_use]
    pub const fn receives_pointer_events(self) -> bool {
        !self.is_pointer_events_none()
    }

    const fn set(&mut self, bit: u16, enabled: bool) {
        if enabled {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }
}

impl fmt::Debug for Hidden {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hidden")
            .field("hidden_for_aria", &self.is_hidden_for_aria())
            .field("box_visible", &self.is_box_visible())
            .field("aria_hidden", &self.is_aria_hidden())
            .field("own_aria_hidden", &self.has_own_aria_hidden())
            .field("visibility_hidden", &self.is_visibility_hidden())
            .field("content_hidden", &self.is_content_hidden())
            .field("own_content_hidden", &self.has_own_content_hidden())
            .field("inert", &self.is_inert())
            .field("ua_shadow", &self.is_ua_shadow())
            .field("display_contents", &self.is_display_contents())
            .field("pointer_events_none", &self.is_pointer_events_none())
            .finish()
    }
}

pub(crate) fn is_text_node(raw: &ContentAttributes) -> bool {
    raw.attribute_type == Some(i32::from(ContentAttributeType::Text))
        || matches!(raw.content_data, Some(ContentData::Text(_)))
}

pub(crate) fn is_ua_shadow(raw: &ContentAttributes) -> bool {
    raw.browseros
        .as_deref()
        .is_some_and(|browseros| browseros.ua_shadow == Some(true))
}

pub(crate) fn raw_tag(raw: &ContentAttributes) -> Option<&str> {
    raw.browseros
        .as_deref()
        .and_then(|browseros| browseros.tag.as_deref())
}

fn has_non_empty_outer_box(raw: &ContentAttributes) -> bool {
    raw.geometry
        .as_ref()
        .and_then(|geometry| geometry.outer_bounding_box)
        .is_some_and(|rect| rect.width.unwrap_or(0) > 0 && rect.height.unwrap_or(0) > 0)
}

fn is_ignored_for_aria_tag(tag: &str) -> bool {
    tag.eq_ignore_ascii_case("style")
        || tag.eq_ignore_ascii_case("script")
        || tag.eq_ignore_ascii_case("noscript")
        || tag.eq_ignore_ascii_case("template")
}

fn has_own_content_hidden_attr(raw: &ContentAttributes) -> bool {
    let code = i32::from(AnnotatedRole::ContentHidden);
    raw.annotated_roles.contains(&code)
}

/// Computes [`Hidden`] for every node in `nodes` using two non-recursive passes.
///
/// Pass A runs top-down in preorder (`0..nodes.len()`) so inherited `aria-hidden`,
/// `ContentHidden`, and `<select>` ancestry propagate strictly within each document in O(N) time.
/// Pass B runs bottom-up in reverse preorder so arbitrarily deep chains of `display: contents`
/// wrappers resolve from their direct same-document children in O(N) time and O(1) stack depth.
pub(crate) fn compute_hidden(nodes: &[TreeNode]) -> Vec<Hidden> {
    let mut results = compute_hidden_top_down(nodes);
    resolve_display_contents_bottom_up(nodes, &mut results);
    results
}

fn compute_hidden_top_down(nodes: &[TreeNode]) -> Vec<Hidden> {
    let mut results = vec![Hidden::default(); nodes.len()];
    let mut raw_aria_hidden_flags = vec![false; nodes.len()];
    let mut inside_select = vec![false; nodes.len()];

    for (index, node) in nodes.iter().enumerate() {
        let same_doc_parent = node.parent().filter(|&parent_id| {
            nodes
                .get(parent_id.index())
                .is_some_and(|parent| parent.doc() == node.doc())
        });
        let parent_hidden = same_doc_parent
            .and_then(|parent_id| results.get(parent_id.index()).copied())
            .unwrap_or_default();
        let parent_had_raw_aria_hidden = same_doc_parent
            .and_then(|parent_id| raw_aria_hidden_flags.get(parent_id.index()).copied())
            .unwrap_or(false);
        let parent_inside_select = same_doc_parent
            .and_then(|parent_id| inside_select.get(parent_id.index()).copied())
            .unwrap_or(false);

        let raw = node.raw_attributes();
        let browseros = raw.and_then(|attrs| attrs.browseros.as_deref());
        let is_text = raw.is_some_and(is_text_node);
        let ua_shadow = browseros.is_some_and(|b| b.ua_shadow == Some(true));
        let raw_aria_hidden = browseros.is_some_and(|b| b.aria_hidden == Some(true));
        let own_content_hidden = raw.is_some_and(has_own_content_hidden_attr);

        if let Some(slot) = raw_aria_hidden_flags.get_mut(index) {
            *slot = raw_aria_hidden;
        }

        let tag = browseros.and_then(|b| b.tag.as_deref());
        let is_select =
            !is_text && !ua_shadow && tag.is_some_and(|name| name.eq_ignore_ascii_case("select"));
        if let Some(slot) = inside_select.get_mut(index) {
            *slot = parent_inside_select || is_select;
        }

        let aria_hidden = raw_aria_hidden || parent_hidden.is_aria_hidden();
        let has_own_aria_hidden = raw_aria_hidden && !parent_had_raw_aria_hidden;
        let content_hidden = own_content_hidden || parent_hidden.is_content_hidden();

        let visibility_hidden =
            !is_text && browseros.is_some_and(|b| b.visibility_hidden == Some(true));
        let pointer_events_none =
            !is_text && browseros.is_some_and(|b| b.pointer_events_none == Some(true));
        let inert = !is_text && browseros.is_some_and(|b| b.inert == Some(true));
        let display_contents =
            !is_text && browseros.is_some_and(|b| b.display_contents == Some(true));

        let is_slot = !is_text && tag.is_some_and(|name| name.eq_ignore_ascii_case("slot"));
        let is_ignored = !is_text && tag.is_some_and(is_ignored_for_aria_tag);
        let option_in_select = !is_text
            && parent_inside_select
            && tag.is_some_and(|name| name.eq_ignore_ascii_case("option"));

        let box_visible = !ua_shadow
            && !visibility_hidden
            && !content_hidden
            && raw.is_some_and(has_non_empty_outer_box);

        let hidden_for_aria = if ua_shadow {
            true
        } else if is_text {
            aria_hidden || content_hidden
        } else {
            is_ignored
                || aria_hidden
                || (!option_in_select && !is_slot && (visibility_hidden || content_hidden))
        };

        let mut flags = Hidden::default();
        flags.set(HIDDEN_FOR_ARIA, hidden_for_aria);
        flags.set(BOX_VISIBLE, box_visible);
        flags.set(ARIA_HIDDEN, aria_hidden);
        flags.set(OWN_ARIA_HIDDEN, has_own_aria_hidden);
        flags.set(VISIBILITY_HIDDEN, visibility_hidden);
        flags.set(CONTENT_HIDDEN, content_hidden);
        flags.set(OWN_CONTENT_HIDDEN, own_content_hidden);
        flags.set(INERT, inert);
        flags.set(UA_SHADOW, ua_shadow);
        flags.set(DISPLAY_CONTENTS, display_contents);
        flags.set(POINTER_EVENTS_NONE, pointer_events_none);

        if let Some(slot) = results.get_mut(index) {
            *slot = flags;
        }
    }

    results
}

fn resolve_display_contents_bottom_up(nodes: &[TreeNode], results: &mut [Hidden]) {
    for index in (0..nodes.len()).rev() {
        let Some(node) = nodes.get(index) else {
            continue;
        };
        let Some(current) = results.get(index).copied() else {
            continue;
        };
        if !current.is_display_contents() || current.is_ua_shadow() {
            continue;
        }

        let tag = node.raw_attributes().and_then(raw_tag);
        let is_slot = tag.is_some_and(|name| name.eq_ignore_ascii_case("slot"));
        let is_ignored = tag.is_some_and(is_ignored_for_aria_tag);

        let mut any_child_box_visible = false;
        let mut any_child_unhidden = false;
        let mut cursor = node.first_child();
        while let Some(child_id) = cursor {
            let Some(child) = nodes.get(child_id.index()) else {
                break;
            };
            if child.doc() == node.doc()
                && let Some(child_hidden) = results.get(child_id.index()).copied()
                && !child_hidden.is_ua_shadow()
            {
                if child_hidden.is_box_visible() {
                    any_child_box_visible = true;
                }
                let child_is_text = child.raw_attributes().is_some_and(is_text_node);
                let counts_for_aria = if child_is_text {
                    child_hidden.is_box_visible() && !child_hidden.is_aria_hidden()
                } else {
                    !child_hidden.is_hidden_for_aria()
                };
                if counts_for_aria {
                    any_child_unhidden = true;
                }
            }
            cursor = child.next_sibling();
        }

        let mut updated = current;
        updated.set(BOX_VISIBLE, any_child_box_visible);
        if !is_slot {
            let hidden_for_aria =
                if is_ignored || current.is_aria_hidden() || current.is_content_hidden() {
                    true
                } else {
                    !any_child_unhidden
                };
            updated.set(HIDDEN_FOR_ARIA, hidden_for_aria);
        }

        if let Some(slot) = results.get_mut(index) {
            *slot = updated;
        }
    }
}
