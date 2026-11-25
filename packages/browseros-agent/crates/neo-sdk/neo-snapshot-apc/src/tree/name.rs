use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::OnceLock;

use crate::proto::{
    BrowserOsRef, ContentAttributes, ContentData, FormControlData, FormControlType,
    RedactionDecision,
};

use super::hidden;
use super::node_key::{DocId, DomNodeId, NodeId};
use super::role::{self, Role};
use super::snapshot_tree::SnapshotTree;
use super::state::InvalidState;

/// Upper bound on emitted redaction bullets so corrupt payloads cannot trigger OOM.
const MAX_REDACTED_BULLETS: usize = 65_536;

static EMPTY_NAME: Name = Name {
    text: String::new(),
    derived_from_content: false,
};

static EMPTY_DESCRIPTION: Description = Description {
    text: String::new(),
    derived_from_content: false,
};

/// Computed WAI-ARIA accessible name for a [`TreeNode`](super::TreeNode).
///
/// Resolved in ACCNAME order from BrowserOS APC wire facts and finalized with
/// [`normalize_whitespace`]. Its manual `Debug` implementation prints only byte length and
/// provenance, never page text.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Name {
    text: String,
    derived_from_content: bool,
}

impl Name {
    /// Normalized accessible name text, or `""` when the node has no accessible name.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the accessible name is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Whether the name came from the target element's own content (step 2f) or an
    /// `aria-labelledby` reference pointing at the target or one of its same-document descendants.
    #[must_use]
    pub const fn derived_from_content(&self) -> bool {
        self.derived_from_content
    }

    /// Consumes this value and returns the owned normalized name string.
    #[must_use]
    pub fn into_text(self) -> String {
        self.text
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Name")
            .field("text_len", &self.text.len())
            .field("derived_from_content", &self.derived_from_content)
            .finish()
    }
}

/// Computed WAI-ARIA accessible description for a [`TreeNode`](super::TreeNode).
///
/// Follows Playwright's `aria-describedby` -> `aria-description` -> `title` precedence and
/// finalizes with [`normalize_whitespace`]. Its manual `Debug` implementation prints only byte
/// length and provenance, never page text.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Description {
    text: String,
    derived_from_content: bool,
}

impl Description {
    /// Normalized accessible description text, or `""` when the node has no description.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the accessible description is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Whether an `aria-describedby` reference pointed at the target element or one of its
    /// same-document descendants.
    #[must_use]
    pub const fn derived_from_content(&self) -> bool {
        self.derived_from_content
    }

    /// Consumes this value and returns the owned normalized description string.
    #[must_use]
    pub fn into_text(self) -> String {
        self.text
    }
}

impl fmt::Debug for Description {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Description")
            .field("text_len", &self.text.len())
            .field("derived_from_content", &self.derived_from_content)
            .finish()
    }
}

/// Controls whether hidden nodes are skipped during ACCNAME name and description computation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NameVisibility {
    /// Exclude ARIA-hidden and ignored elements unless referenced by a hidden relation root.
    #[default]
    VisibleOnly,
    /// Include hidden and ignored elements in name and description computation.
    IncludeHidden,
}

impl NameVisibility {
    const fn include_hidden(self) -> bool {
        matches!(self, Self::IncludeHidden)
    }
}

/// Thread-safe, on-demand memoized resolver for accessible names, descriptions, and error messages
/// over a [`SnapshotTree`].
///
/// Document-scoped lookup indices (`(DocId, DomNodeId) -> NodeId` and `<label for>` reverse maps)
/// and per-node result tables initialize lazily via `OnceLock`. Evaluations read only the shared
/// lookup indices and never re-enter per-node cache slots, so concurrent lookups cannot deadlock.
/// Its manual `Debug` implementation prints only node counts and initialization flags, never page
/// text.
pub struct NameCache<'a> {
    index: LookupIndex<'a>,
    names_visible: OnceLock<Vec<OnceLock<Name>>>,
    names_hidden: OnceLock<Vec<OnceLock<Name>>>,
    descriptions_visible: OnceLock<Vec<OnceLock<Description>>>,
    descriptions_hidden: OnceLock<Vec<OnceLock<Description>>>,
    error_messages: OnceLock<Vec<OnceLock<String>>>,
}

impl<'a> NameCache<'a> {
    /// Creates an empty on-demand cache borrowing `tree`.
    #[must_use]
    pub fn new(tree: &'a SnapshotTree) -> Self {
        Self {
            index: LookupIndex::new(tree),
            names_visible: OnceLock::new(),
            names_hidden: OnceLock::new(),
            descriptions_visible: OnceLock::new(),
            descriptions_hidden: OnceLock::new(),
            error_messages: OnceLock::new(),
        }
    }

    /// Returns the memoized accessible [`Name`] for `id`, or an empty [`Name`] if `id` is out of
    /// bounds.
    #[must_use]
    pub fn name(&self, id: NodeId, visibility: NameVisibility) -> &Name {
        let len = self.index.tree.nodes().len();
        if id.index() >= len {
            return &EMPTY_NAME;
        }
        let table = match visibility {
            NameVisibility::VisibleOnly => &self.names_visible,
            NameVisibility::IncludeHidden => &self.names_hidden,
        };
        let slots = table.get_or_init(|| alloc_slots(len));
        let Some(slot) = slots.get(id.index()) else {
            return &EMPTY_NAME;
        };
        slot.get_or_init(|| compute_accessible_name(&self.index, id, visibility))
    }

    /// Returns the memoized accessible [`Description`] for `id`, or an empty [`Description`] if
    /// `id` is out of bounds.
    #[must_use]
    pub fn description(&self, id: NodeId, visibility: NameVisibility) -> &Description {
        let len = self.index.tree.nodes().len();
        if id.index() >= len {
            return &EMPTY_DESCRIPTION;
        }
        let table = match visibility {
            NameVisibility::VisibleOnly => &self.descriptions_visible,
            NameVisibility::IncludeHidden => &self.descriptions_hidden,
        };
        let slots = table.get_or_init(|| alloc_slots(len));
        let Some(slot) = slots.get(id.index()) else {
            return &EMPTY_DESCRIPTION;
        };
        slot.get_or_init(|| compute_accessible_description(&self.index, id, visibility))
    }

    /// Returns the memoized accessible error message for `id`, or `""` if `id` is out of bounds or
    /// valid.
    #[must_use]
    pub fn error_message(&self, id: NodeId) -> &str {
        let len = self.index.tree.nodes().len();
        if id.index() >= len {
            return "";
        }
        let slots = self.error_messages.get_or_init(|| alloc_slots(len));
        let Some(slot) = slots.get(id.index()) else {
            return "";
        };
        slot.get_or_init(|| compute_accessible_error_message(&self.index, id))
            .as_str()
    }
}

impl fmt::Debug for NameCache<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NameCache")
            .field("node_count", &self.index.tree.nodes().len())
            .field("has_doc_dom_index", &self.index.by_doc_dom.get().is_some())
            .field("has_labels_index", &self.index.labels_for.get().is_some())
            .field("has_visible_names", &self.names_visible.get().is_some())
            .field("has_hidden_names", &self.names_hidden.get().is_some())
            .field(
                "has_visible_descriptions",
                &self.descriptions_visible.get().is_some(),
            )
            .field(
                "has_hidden_descriptions",
                &self.descriptions_hidden.get().is_some(),
            )
            .field("has_error_messages", &self.error_messages.get().is_some())
            .finish()
    }
}

fn alloc_slots<T>(len: usize) -> Vec<OnceLock<T>> {
    let mut slots = Vec::with_capacity(len);
    slots.resize_with(len, OnceLock::new);
    slots
}

/// Converts `text` into an ACCNAME flat string while preserving interior non-breaking spaces
/// (`U+00A0`), matching Playwright's `asFlatString`.
///
/// Strips zero-width spaces (`U+200B`) and soft hyphens (`U+00AD`), collapses JavaScript
/// whitespace runs inside each `U+00A0`-delimited chunk to a single ASCII space, rejoins with
/// `U+00A0`, and trims leading/trailing JavaScript whitespace (including `U+00A0` and `U+FEFF`,
/// excluding `U+0085`).
#[must_use]
pub fn as_flat_string(text: &str) -> String {
    let mut joined = String::with_capacity(text.len());
    for (chunk_idx, chunk) in text.split('\u{00A0}').enumerate() {
        if chunk_idx > 0 {
            joined.push('\u{00A0}');
        }
        let mut in_whitespace = false;
        for ch in chunk.chars() {
            if ch == '\u{200B}' || ch == '\u{00AD}' {
                continue;
            }
            if role::is_js_whitespace(ch) {
                if !in_whitespace {
                    joined.push(' ');
                    in_whitespace = true;
                }
            } else {
                joined.push(ch);
                in_whitespace = false;
            }
        }
    }
    let trimmed = role::trim_js_whitespace(&joined);
    if trimmed.len() == joined.len() {
        joined
    } else {
        trimmed.to_owned()
    }
}

/// Normalizes whitespace for snapshot rendering and accessible name/description output, matching
/// Playwright's `normalizeWhiteSpace`.
///
/// Strips zero-width spaces (`U+200B`) and soft hyphens (`U+00AD`), trims leading and trailing
/// JavaScript whitespace, and collapses all interior JavaScript whitespace runs (including
/// `U+00A0` and `U+FEFF`, excluding `U+0085`) to a single ASCII space.
#[must_use]
pub fn normalize_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut started = false;
    let mut pending_space = false;

    for ch in text.chars() {
        if ch == '\u{200B}' || ch == '\u{00AD}' {
            continue;
        }
        if role::is_js_whitespace(ch) {
            if started {
                pending_space = true;
            }
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
            started = true;
        }
    }

    out
}

impl SnapshotTree {
    /// Creates an on-demand memoized [`NameCache`] for computing names, descriptions, and error
    /// messages across many nodes in this tree.
    ///
    /// Contract version gating happens once at [`SnapshotTree::build`] via
    /// [`BuildOptions::with_min_contract_version`](super::BuildOptions::with_min_contract_version);
    /// fields added in higher contract versions simply read as unset on lower-version trees.
    #[must_use]
    pub fn name_cache(&self) -> NameCache<'_> {
        NameCache::new(self)
    }

    /// Computes the accessible [`Name`] for `id` without allocating per-node cache tables.
    #[must_use]
    pub fn accessible_name(&self, id: NodeId, visibility: NameVisibility) -> Name {
        let index = LookupIndex::new(self);
        compute_accessible_name(&index, id, visibility)
    }

    /// Computes the accessible [`Description`] for `id` without allocating per-node cache tables.
    #[must_use]
    pub fn accessible_description(&self, id: NodeId, visibility: NameVisibility) -> Description {
        let index = LookupIndex::new(self);
        compute_accessible_description(&index, id, visibility)
    }

    /// Computes the accessible error message for `id` without allocating per-node cache tables.
    #[must_use]
    pub fn accessible_error_message(&self, id: NodeId) -> String {
        let index = LookupIndex::new(self);
        compute_accessible_error_message(&index, id)
    }
}

/// Shared lazy relation indices used by both [`NameCache`] and one-shot [`SnapshotTree`] queries
/// so one-shot callers avoid allocating per-node memoization arrays.
struct LookupIndex<'a> {
    tree: &'a SnapshotTree,
    by_doc_dom: OnceLock<HashMap<(DocId, DomNodeId), NodeId>>,
    labels_for: OnceLock<HashMap<(DocId, DomNodeId), Vec<NodeId>>>,
    in_svg_scope: OnceLock<Vec<bool>>,
}

impl<'a> LookupIndex<'a> {
    fn new(tree: &'a SnapshotTree) -> Self {
        Self {
            tree,
            by_doc_dom: OnceLock::new(),
            labels_for: OnceLock::new(),
            in_svg_scope: OnceLock::new(),
        }
    }

    /// Whether `node_id` is an `<svg>` element or an SVG descendant (`ownerSVGElement != null`)
    /// whose direct `<title>` children live in the SVG namespace.
    ///
    /// Computed once in a single O(N) preorder pass on first demand so deep trees never incur
    /// quadratic ancestor walks. Stops at `<foreignObject>` and document boundaries, and treats
    /// untagged HTML `div`/`span` nodes (`raw_tag == None`) as outside SVG.
    fn is_in_svg_scope(&self, node_id: NodeId) -> bool {
        let flags = self.in_svg_scope.get_or_init(|| {
            let nodes = self.tree.nodes();
            let mut in_svg = vec![false; nodes.len()];
            for node in nodes {
                let Some(raw) = node.raw_attributes() else {
                    continue;
                };
                let parent_in_svg = node
                    .parent()
                    .and_then(|pid| nodes.get(pid.index()).filter(|p| p.doc() == node.doc()))
                    .and_then(|p| in_svg.get(p.id().index()).copied())
                    .unwrap_or(false);
                let value = if hidden::is_ua_shadow(raw) || hidden::is_text_node(raw) {
                    parent_in_svg
                } else {
                    match hidden::raw_tag(raw) {
                        Some(tag) if tag.eq_ignore_ascii_case("svg") => true,
                        Some(tag) if tag.eq_ignore_ascii_case("foreignobject") => false,
                        Some(_) => parent_in_svg,
                        None => false,
                    }
                };
                if let Some(slot) = in_svg.get_mut(node.id().index()) {
                    *slot = value;
                }
            }
            in_svg
        });
        flags.get(node_id.index()).copied().unwrap_or(false)
    }

    fn node_by_doc_dom(&self, doc: DocId, dom_id: DomNodeId) -> Option<NodeId> {
        let map = self.by_doc_dom.get_or_init(|| {
            let mut by_doc_dom = HashMap::new();
            for node in self.tree.nodes() {
                let Some(dom) = node.dom_node_id() else {
                    continue;
                };
                let Some(raw) = node.raw_attributes() else {
                    continue;
                };
                // First non-ua_shadow element in preorder wins; never let a text node shadow an
                // element sharing the same common_ancestor_dom_node_id.
                if hidden::is_ua_shadow(raw) || hidden::is_text_node(raw) {
                    continue;
                }
                by_doc_dom.entry((node.doc(), dom)).or_insert(node.id());
            }
            by_doc_dom
        });
        map.get(&(doc, dom_id)).copied()
    }

    fn labels_for(&self, doc: DocId, dom_id: Option<DomNodeId>) -> &[NodeId] {
        let Some(target_dom_id) = dom_id else {
            return &[];
        };
        let map = self.labels_for.get_or_init(|| {
            let mut labels: HashMap<(DocId, DomNodeId), Vec<NodeId>> = HashMap::new();
            for node in self.tree.nodes() {
                let Some(raw) = node.raw_attributes() else {
                    continue;
                };
                if hidden::is_ua_shadow(raw) || hidden::is_text_node(raw) {
                    continue;
                }
                if let Some(target_raw) = raw.label_for_dom_node_id {
                    labels
                        .entry((node.doc(), DomNodeId::new(target_raw)))
                        .or_default()
                        .push(node.id());
                }
            }
            labels
        });
        map.get(&(doc, target_dom_id)).map_or(&[], Vec::as_slice)
    }

    fn resolve_refs<'b>(&self, doc: DocId, refs: &'b [BrowserOsRef]) -> Vec<ResolvedRef<'b>> {
        let mut resolved = Vec::with_capacity(refs.len());
        let mut seen_dom_ids: HashSet<DomNodeId> = HashSet::new();

        for item in refs {
            if let Some(raw_dom_id) = item.dom_node_id {
                let dom_id = DomNodeId::new(raw_dom_id);
                if !seen_dom_ids.insert(dom_id) {
                    continue;
                }
                if let Some(target_id) = self.node_by_doc_dom(doc, dom_id) {
                    resolved.push(ResolvedRef::Node(target_id));
                } else if let Some(text) = item.unrendered_text.as_deref() {
                    resolved.push(ResolvedRef::Unrendered(text));
                }
            } else if let Some(text) = item.unrendered_text.as_deref() {
                resolved.push(ResolvedRef::Unrendered(text));
            }
        }

        resolved
    }
}

#[derive(Clone, Copy)]
enum ResolvedRef<'a> {
    Node(NodeId),
    Unrendered(&'a str),
}

#[derive(Clone, Copy)]
struct NodeFacts<'a> {
    id: NodeId,
    doc: DocId,
    dom_id: Option<DomNodeId>,
    role: Option<Role>,
    tag: Option<&'a str>,
    raw: &'a ContentAttributes,
}

impl<'a> NodeFacts<'a> {
    fn from_tree(tree: &'a SnapshotTree, id: NodeId) -> Option<Self> {
        let node = tree.node(id)?;
        let raw = node.raw_attributes()?;
        Some(Self {
            id,
            doc: node.doc(),
            dom_id: node.dom_node_id(),
            role: node.role(),
            tag: hidden::raw_tag(raw),
            raw,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EmbeddedTarget {
    SelfNode,
    Descendant,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum RefTraversal {
    #[default]
    None,
    Visible,
    Hidden,
}

impl RefTraversal {
    const fn from_hidden(is_hidden: bool) -> Self {
        if is_hidden {
            Self::Hidden
        } else {
            Self::Visible
        }
    }

    const fn is_active(self) -> bool {
        !matches!(self, Self::None)
    }

    const fn is_hidden(self) -> bool {
        matches!(self, Self::Hidden)
    }
}

#[derive(Clone, Copy)]
struct EvalOptions {
    visibility: NameVisibility,
    embedded_in_target: Option<EmbeddedTarget>,
    embedded_in_labelledby: RefTraversal,
    embedded_in_describedby: RefTraversal,
    embedded_in_label: RefTraversal,
    embedded_in_native: RefTraversal,
}

impl EvalOptions {
    const fn for_root(visibility: NameVisibility) -> Self {
        Self {
            visibility,
            embedded_in_target: Some(EmbeddedTarget::SelfNode),
            embedded_in_labelledby: RefTraversal::None,
            embedded_in_describedby: RefTraversal::None,
            embedded_in_label: RefTraversal::None,
            embedded_in_native: RefTraversal::None,
        }
    }

    const fn for_labelledby(visibility: NameVisibility, target_hidden: bool) -> Self {
        Self {
            visibility,
            embedded_in_target: None,
            embedded_in_labelledby: RefTraversal::from_hidden(target_hidden),
            embedded_in_describedby: RefTraversal::None,
            embedded_in_label: RefTraversal::None,
            embedded_in_native: RefTraversal::None,
        }
    }

    const fn for_describedby(visibility: NameVisibility, target_hidden: bool) -> Self {
        Self {
            visibility,
            embedded_in_target: None,
            embedded_in_labelledby: RefTraversal::None,
            embedded_in_describedby: RefTraversal::from_hidden(target_hidden),
            embedded_in_label: RefTraversal::None,
            embedded_in_native: RefTraversal::None,
        }
    }

    const fn for_label(visibility: NameVisibility, label_hidden: bool) -> Self {
        Self {
            visibility,
            embedded_in_target: None,
            embedded_in_labelledby: RefTraversal::None,
            embedded_in_describedby: RefTraversal::None,
            embedded_in_label: RefTraversal::from_hidden(label_hidden),
            embedded_in_native: RefTraversal::None,
        }
    }

    const fn child_options(self) -> Self {
        Self {
            embedded_in_target: match self.embedded_in_target {
                Some(EmbeddedTarget::SelfNode) => Some(EmbeddedTarget::Descendant),
                other => other,
            },
            ..self
        }
    }

    const fn inside_target(self) -> bool {
        matches!(
            self.embedded_in_target,
            Some(EmbeddedTarget::SelfNode | EmbeddedTarget::Descendant)
        )
    }

    const fn is_embedded_in_hidden_ref(self) -> bool {
        self.embedded_in_labelledby.is_hidden()
            || self.embedded_in_describedby.is_hidden()
            || self.embedded_in_label.is_hidden()
            || self.embedded_in_native.is_hidden()
    }
}

fn is_name_prohibited(role: Option<Role>) -> bool {
    matches!(
        role,
        Some(
            Role::Caption
                | Role::Code
                | Role::Definition
                | Role::Deletion
                | Role::Emphasis
                | Role::Generic
                | Role::Insertion
                | Role::Mark
                | Role::Paragraph
                | Role::Presentation
                | Role::Strong
                | Role::Subscript
                | Role::Superscript
                | Role::Term
                | Role::Time
        )
    )
}

fn allows_name_from_content(role: Option<Role>, target_descendant: bool) -> bool {
    let always = matches!(
        role,
        Some(
            Role::Button
                | Role::Cell
                | Role::Checkbox
                | Role::Columnheader
                | Role::Gridcell
                | Role::Heading
                | Role::Link
                | Role::Menuitem
                | Role::Menuitemcheckbox
                | Role::Menuitemradio
                | Role::Option
                | Role::Radio
                | Role::Row
                | Role::Rowheader
                | Role::Switch
                | Role::Tab
                | Role::Tooltip
                | Role::Treeitem
        )
    );
    let descendant = target_descendant
        && matches!(
            role,
            None | Some(
                Role::Caption
                    | Role::Code
                    | Role::Contentinfo
                    | Role::Definition
                    | Role::Deletion
                    | Role::Emphasis
                    | Role::Insertion
                    | Role::List
                    | Role::Listitem
                    | Role::Mark
                    | Role::None
                    | Role::Paragraph
                    | Role::Presentation
                    | Role::Region
                    | Role::Row
                    | Role::Rowgroup
                    | Role::Strong
                    | Role::Subscript
                    | Role::Superscript
                    | Role::Table
                    | Role::Term
                    | Role::Time
            )
        );
    always || descendant
}

fn is_ignored_tag(tag: &str) -> bool {
    tag.eq_ignore_ascii_case("style")
        || tag.eq_ignore_ascii_case("script")
        || tag.eq_ignore_ascii_case("noscript")
        || tag.eq_ignore_ascii_case("template")
}

fn text_node_content(raw: &ContentAttributes) -> &str {
    if let Some(dom_text) = raw
        .browseros
        .as_deref()
        .and_then(|browseros| browseros.dom_text.as_deref())
    {
        return dom_text;
    }
    match raw.content_data.as_ref() {
        Some(ContentData::Text(info)) => info.text_content.as_deref().unwrap_or(""),
        _ => "",
    }
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

fn raw_title(raw: &ContentAttributes) -> &str {
    raw.browseros
        .as_deref()
        .and_then(|browseros| browseros.title.as_deref())
        .unwrap_or("")
}

fn raw_alt(raw: &ContentAttributes) -> &str {
    raw.browseros
        .as_deref()
        .and_then(|browseros| browseros.alt.as_deref())
        .unwrap_or("")
}

fn raw_placeholder(raw: &ContentAttributes) -> &str {
    form_control_data(raw)
        .and_then(|control| control.placeholder.as_deref())
        .unwrap_or("")
}

fn decode_redaction(code: Option<i32>) -> Option<RedactionDecision> {
    code.and_then(|raw_code| RedactionDecision::try_from(raw_code).ok())
}

fn is_redacted_decision(decision: RedactionDecision) -> bool {
    matches!(
        decision,
        RedactionDecision::RedactedHasBeenPassword
            | RedactionDecision::RedactedIsSensitivePaymentField
            | RedactionDecision::RedactedCustomPasswordCss
            | RedactionDecision::RedactedCustomPasswordJs
            | RedactionDecision::RedactedIsOtp
    )
}

fn is_unredacted_empty_decision(decision: RedactionDecision) -> bool {
    matches!(
        decision,
        RedactionDecision::UnredactedEmptyPassword
            | RedactionDecision::UnredactedEmptyPaymentField
            | RedactionDecision::UnredactedEmptyCustomPassword
            | RedactionDecision::UnredactedEmptyOtpField
    )
}

fn is_redacted(raw: &ContentAttributes) -> bool {
    let attr_redacted = decode_redaction(raw.redaction_decision).is_some_and(is_redacted_decision);
    let control_redacted = form_control_data(raw)
        .and_then(|control| decode_redaction(control.redaction_decision))
        .is_some_and(is_redacted_decision);
    attr_redacted || control_redacted
}

fn is_unredacted_empty(raw: &ContentAttributes) -> bool {
    let attr_empty =
        decode_redaction(raw.redaction_decision).is_some_and(is_unredacted_empty_decision);
    let control_empty = form_control_data(raw)
        .and_then(|control| decode_redaction(control.redaction_decision))
        .is_some_and(is_unredacted_empty_decision);
    attr_empty || control_empty
}

fn redacted_bullets(raw: &ContentAttributes) -> String {
    let len = raw
        .browseros
        .as_deref()
        .and_then(|browseros| browseros.redacted_value_length)
        .unwrap_or(0);
    let count = usize::try_from(len)
        .unwrap_or(MAX_REDACTED_BULLETS)
        .min(MAX_REDACTED_BULLETS);
    if count == 0 {
        String::new()
    } else {
        "•".repeat(count)
    }
}

fn masked_or_raw_field_value(raw: &ContentAttributes) -> String {
    // Privacy gate: redacted controls must never read `field_value`, even if a payload populates it.
    if is_redacted(raw) {
        return redacted_bullets(raw);
    }
    if is_unredacted_empty(raw) {
        return String::new();
    }
    form_control_data(raw)
        .and_then(|control| control.field_value.as_deref())
        .unwrap_or("")
        .to_owned()
}

fn contains_in_same_doc(tree: &SnapshotTree, ancestor_id: NodeId, target_id: NodeId) -> bool {
    let (Some(ancestor), Some(target)) = (tree.node(ancestor_id), tree.node(target_id)) else {
        return false;
    };
    ancestor.doc() == target.doc() && ancestor.subtree_range().contains(target_id)
}

fn push_children_reversed(
    tree: &SnapshotTree,
    parent_id: NodeId,
    doc: DocId,
    stack: &mut Vec<NodeId>,
) {
    let start = stack.len();
    for child in tree.children(parent_id) {
        if child.doc() == doc {
            stack.push(child.id());
        }
    }
    if let Some(added) = stack.get_mut(start..) {
        added.reverse();
    }
}

/// Iterative walker over direct logical children of a node in flat-tree preorder within the same
/// document, skipping `ua_shadow` text nodes and transparently descending through `ua_shadow`
/// containers.
struct LogicalChildren<'a> {
    tree: &'a SnapshotTree,
    doc: DocId,
    stack: Vec<NodeId>,
}

impl<'a> LogicalChildren<'a> {
    fn new(tree: &'a SnapshotTree, parent_id: NodeId) -> Self {
        let Some(parent) = tree.node(parent_id) else {
            return Self {
                tree,
                doc: DocId::MAIN,
                stack: Vec::new(),
            };
        };
        let doc = parent.doc();
        let mut stack = Vec::new();
        push_children_reversed(tree, parent_id, doc, &mut stack);
        Self { tree, doc, stack }
    }
}

impl Iterator for LogicalChildren<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(child_id) = self.stack.pop() {
            let Some(child) = self.tree.node(child_id) else {
                continue;
            };
            if child.doc() != self.doc {
                continue;
            }
            let Some(raw) = child.raw_attributes() else {
                continue;
            };
            if hidden::is_ua_shadow(raw) {
                if !hidden::is_text_node(raw) {
                    push_children_reversed(self.tree, child_id, self.doc, &mut self.stack);
                }
                continue;
            }
            return Some(child_id);
        }
        None
    }
}

/// Collects direct logical children of `parent_id` in flat-tree preorder within the same document.
fn collect_logical_children(tree: &SnapshotTree, parent_id: NodeId) -> Vec<NodeId> {
    LogicalChildren::new(tree, parent_id).collect()
}

fn find_first_direct_child_tag(
    tree: &SnapshotTree,
    parent_id: NodeId,
    expected_tag: &str,
) -> Option<NodeId> {
    LogicalChildren::new(tree, parent_id).find(|&child_id| {
        let Some(child) = tree.node(child_id) else {
            return false;
        };
        let Some(raw) = child.raw_attributes() else {
            return false;
        };
        !hidden::is_text_node(raw)
            && hidden::raw_tag(raw).is_some_and(|tag| tag.eq_ignore_ascii_case(expected_tag))
    })
}

fn collect_dom_text_in_subtree(tree: &SnapshotTree, root_id: NodeId) -> Option<String> {
    let root = tree.node(root_id)?;
    let doc = root.doc();
    let nodes = tree.nodes();
    let end = root.subtree_end().index().min(nodes.len());
    let mut cursor = root_id.index().saturating_add(1);
    let mut found_text = false;
    let mut text = String::new();

    while cursor < end {
        let Some(desc) = nodes.get(cursor) else {
            break;
        };
        if desc.doc() != doc || desc.child_doc().is_some() {
            cursor = desc.subtree_end().index().max(cursor.saturating_add(1));
            continue;
        }
        if desc.dom_node_id().is_some()
            && let Some(raw) = desc.raw_attributes()
            && !hidden::is_ua_shadow(raw)
            && hidden::is_text_node(raw)
        {
            found_text = true;
            text.push_str(text_node_content(raw));
        }
        cursor = cursor.saturating_add(1);
    }

    found_text.then_some(text)
}

fn find_select_option_nodes(tree: &SnapshotTree, select_id: NodeId) -> Option<Vec<NodeId>> {
    let select = tree.node(select_id)?;
    let doc = select.doc();
    let nodes = tree.nodes();
    let end = select.subtree_end().index().min(nodes.len());
    let mut cursor = select_id.index().saturating_add(1);
    let mut first_option: Option<NodeId> = None;
    let mut selected = Vec::new();

    while cursor < end {
        let Some(desc) = nodes.get(cursor) else {
            break;
        };
        if desc.doc() != doc || desc.child_doc().is_some() {
            cursor = desc.subtree_end().index().max(cursor.saturating_add(1));
            continue;
        }
        if let Some(raw) = desc.raw_attributes()
            && !hidden::is_ua_shadow(raw)
            && !hidden::is_text_node(raw)
            && hidden::raw_tag(raw).is_some_and(|tag| tag.eq_ignore_ascii_case("option"))
        {
            if first_option.is_none() {
                first_option = Some(desc.id());
            }
            if desc.state().selected() == Some(true) {
                selected.push(desc.id());
            }
        }
        cursor = cursor.saturating_add(1);
    }

    let first = first_option?;
    if selected.is_empty() {
        selected.push(first);
    }
    Some(selected)
}

fn select_options_fallback_text(raw: &ContentAttributes) -> String {
    let Some(control) = form_control_data(raw) else {
        return String::new();
    };
    if control.select_options.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    let mut count: usize = 0;
    for opt in &control.select_options {
        if opt.is_selected == Some(true) {
            if count > 0 {
                out.push(' ');
            }
            out.push_str(opt.text.as_deref().unwrap_or(""));
            count += 1;
        }
    }
    if count > 0 {
        return out;
    }
    control
        .select_options
        .first()
        .and_then(|opt| opt.text.as_deref())
        .unwrap_or("")
        .to_owned()
}

fn find_first_listbox_descendant(tree: &SnapshotTree, combobox_id: NodeId) -> Option<NodeId> {
    let combobox = tree.node(combobox_id)?;
    let doc = combobox.doc();
    let nodes = tree.nodes();
    let end = combobox.subtree_end().index().min(nodes.len());
    let mut cursor = combobox_id.index().saturating_add(1);

    while cursor < end {
        let Some(desc) = nodes.get(cursor) else {
            break;
        };
        if desc.doc() != doc || desc.child_doc().is_some() {
            cursor = desc.subtree_end().index().max(cursor.saturating_add(1));
            continue;
        }
        if let Some(raw) = desc.raw_attributes()
            && !hidden::is_ua_shadow(raw)
            && !hidden::is_text_node(raw)
            && desc.role() == Some(Role::Listbox)
        {
            return Some(desc.id());
        }
        cursor = cursor.saturating_add(1);
    }

    None
}

fn find_selected_options_in_listbox(tree: &SnapshotTree, listbox_id: NodeId) -> Vec<NodeId> {
    let Some(listbox) = tree.node(listbox_id) else {
        return Vec::new();
    };
    let doc = listbox.doc();
    let nodes = tree.nodes();
    let end = listbox.subtree_end().index().min(nodes.len());
    let mut cursor = listbox_id.index().saturating_add(1);
    let mut selected = Vec::new();

    while cursor < end {
        let Some(desc) = nodes.get(cursor) else {
            break;
        };
        if desc.doc() != doc || desc.child_doc().is_some() {
            cursor = desc.subtree_end().index().max(cursor.saturating_add(1));
            continue;
        }
        if let Some(raw) = desc.raw_attributes()
            && !hidden::is_ua_shadow(raw)
            && !hidden::is_text_node(raw)
            && desc.role() == Some(Role::Option)
            && desc.state().selected() == Some(true)
        {
            selected.push(desc.id());
        }
        cursor = cursor.saturating_add(1);
    }

    selected
}

/// Heap-allocated continuation frames driving [`eval_text_alternative`] iteratively without call
/// stack recursion.
enum Frame<'a> {
    LabelledBy {
        node_id: NodeId,
        opts: EvalOptions,
        refs: Vec<ResolvedRef<'a>>,
        next_index: usize,
        acc: String,
        part_count: usize,
        self_contained: bool,
    },
    JoinAll {
        opts: EvalOptions,
        nodes: Vec<NodeId>,
        next_index: usize,
        acc: String,
        part_count: usize,
    },
    JoinNonEmptyLabels {
        visibility: NameVisibility,
        labels: Vec<NodeId>,
        next_index: usize,
        acc: String,
    },
    ContentChildren {
        node_id: NodeId,
        opts: EvalOptions,
        children: Vec<NodeId>,
        next_index: usize,
        acc: String,
    },
}

enum StepOutcome<'a> {
    Done(String),
    TailCall { node_id: NodeId, opts: EvalOptions },
    Push(Frame<'a>),
}

/// Mutable execution state threaded through the iterative ACCNAME evaluator.
struct EvalState<'a> {
    stack: Vec<Frame<'a>>,
    active: Option<(NodeId, EvalOptions)>,
    returned: Option<String>,
    visited: HashSet<NodeId>,
    derived_from_content: bool,
}

impl<'a> EvalState<'a> {
    fn new(root_id: NodeId, root_opts: EvalOptions) -> Self {
        Self {
            stack: Vec::new(),
            active: Some((root_id, root_opts)),
            returned: None,
            visited: HashSet::new(),
            derived_from_content: false,
        }
    }

    fn apply_outcome(&mut self, outcome: StepOutcome<'a>) {
        match outcome {
            StepOutcome::Done(text) => self.returned = Some(text),
            StepOutcome::TailCall { node_id, opts } => self.active = Some((node_id, opts)),
            StepOutcome::Push(frame) => self.stack.push(frame),
        }
    }
}

/// Evaluates ACCNAME step 2 iteratively on an explicit heap stack so deeply nested DOM trees never
/// overflow the call stack.
fn eval_text_alternative(
    index: &LookupIndex<'_>,
    root_id: NodeId,
    root_opts: EvalOptions,
    derived_from_content: &mut bool,
) -> String {
    let mut state = EvalState::new(root_id, root_opts);

    loop {
        if let Some((node_id, opts)) = state.active.take() {
            let outcome = start_node_eval(index, node_id, opts, &mut state.visited);
            state.apply_outcome(outcome);
            continue;
        }

        if state.stack.is_empty() {
            *derived_from_content = state.derived_from_content;
            return state.returned.unwrap_or_default();
        }

        advance_top_frame(index, &mut state);
    }
}

fn advance_top_frame<'a>(index: &LookupIndex<'a>, state: &mut EvalState<'a>) {
    match state.stack.last() {
        Some(Frame::LabelledBy { .. }) => advance_labelledby_frame(index, state),
        Some(Frame::JoinAll { .. }) => advance_join_all_frame(state),
        Some(Frame::JoinNonEmptyLabels { .. }) => advance_labels_frame(index, state),
        Some(Frame::ContentChildren { .. }) => advance_content_frame(index, state),
        None => {}
    }
}

fn advance_labelledby_frame<'a>(index: &LookupIndex<'a>, state: &mut EvalState<'a>) {
    if let Some(Frame::LabelledBy {
        opts,
        refs,
        next_index,
        acc,
        part_count,
        ..
    }) = state.stack.last_mut()
    {
        if let Some(text) = state.returned.take() {
            if *part_count > 0 {
                acc.push(' ');
            }
            acc.push_str(&text);
            *part_count += 1;
        }
        while let Some(&item) = refs.get(*next_index) {
            *next_index += 1;
            match item {
                ResolvedRef::Unrendered(text) => {
                    if *part_count > 0 {
                        acc.push(' ');
                    }
                    acc.push_str(text);
                    *part_count += 1;
                }
                ResolvedRef::Node(target_id) => {
                    let target_hidden = index
                        .tree
                        .node(target_id)
                        .is_some_and(|node| node.hidden().is_hidden_for_aria());
                    state.active = Some((
                        target_id,
                        EvalOptions::for_labelledby(opts.visibility, target_hidden),
                    ));
                    return;
                }
            }
        }
    }

    let Some(Frame::LabelledBy {
        node_id,
        opts,
        refs,
        acc,
        self_contained,
        ..
    }) = state.stack.pop()
    else {
        return;
    };

    if !acc.is_empty() {
        if opts.inside_target() && self_contained {
            state.derived_from_content = true;
        }
        state.returned = Some(acc);
        return;
    }

    let presence = LabelledByPresence {
        has_any: !refs.is_empty(),
        references_self: refs
            .iter()
            .any(|item| matches!(item, ResolvedRef::Node(target_id) if *target_id == node_id)),
    };
    let outcome = eval_from_step_2c(index, node_id, opts, presence, &mut state.visited);
    state.apply_outcome(outcome);
}

fn advance_join_all_frame(state: &mut EvalState<'_>) {
    if let Some(Frame::JoinAll {
        opts,
        nodes,
        next_index,
        acc,
        part_count,
    }) = state.stack.last_mut()
    {
        if let Some(text) = state.returned.take() {
            if *part_count > 0 {
                acc.push(' ');
            }
            acc.push_str(&text);
            *part_count += 1;
        }
        if let Some(&target_id) = nodes.get(*next_index) {
            *next_index += 1;
            state.active = Some((target_id, *opts));
            return;
        }
    }

    if let Some(Frame::JoinAll { acc, .. }) = state.stack.pop() {
        state.returned = Some(acc);
    }
}

fn advance_labels_frame(index: &LookupIndex<'_>, state: &mut EvalState<'_>) {
    if let Some(Frame::JoinNonEmptyLabels {
        visibility,
        labels,
        next_index,
        acc,
    }) = state.stack.last_mut()
    {
        if let Some(text) = state.returned.take()
            && !text.is_empty()
        {
            if !acc.is_empty() {
                acc.push(' ');
            }
            acc.push_str(&text);
        }
        if let Some(&label_id) = labels.get(*next_index) {
            *next_index += 1;
            let label_hidden = index
                .tree
                .node(label_id)
                .is_some_and(|node| node.hidden().is_hidden_for_aria());
            state.active = Some((label_id, EvalOptions::for_label(*visibility, label_hidden)));
            return;
        }
    }

    if let Some(Frame::JoinNonEmptyLabels { acc, .. }) = state.stack.pop() {
        state.returned = Some(acc);
    }
}

fn advance_content_frame(index: &LookupIndex<'_>, state: &mut EvalState<'_>) {
    if let Some(Frame::ContentChildren {
        opts,
        children,
        next_index,
        acc,
        ..
    }) = state.stack.last_mut()
    {
        if let Some(text) = state.returned.take() {
            acc.push_str(&text);
        }
        while let Some(&child_id) = children.get(*next_index) {
            *next_index += 1;
            let Some(child) = index.tree.node(child_id) else {
                continue;
            };
            let Some(child_raw) = child.raw_attributes() else {
                continue;
            };
            // Emit any Blink block break before checking hidden-ness so breaks transferred onto
            // hidden siblings still separate surrounding inline text.
            if child_raw
                .browseros
                .as_deref()
                .is_some_and(|browseros| browseros.break_before == Some(true))
            {
                acc.push(' ');
            }
            if hidden::is_text_node(child_raw) {
                if opts.visibility.include_hidden()
                    || opts.is_embedded_in_hidden_ref()
                    || !child.hidden().is_hidden_for_aria()
                {
                    acc.push_str(text_node_content(child_raw));
                }
                continue;
            }
            state.active = Some((child_id, opts.child_options()));
            return;
        }
    }

    let Some(Frame::ContentChildren {
        node_id, opts, acc, ..
    }) = state.stack.pop()
    else {
        return;
    };

    let has_trimmed = !role::trim_js_whitespace(&acc).is_empty();
    let is_non_empty = if opts.embedded_in_target == Some(EmbeddedTarget::SelfNode) {
        has_trimmed
    } else {
        !acc.is_empty()
    };

    if is_non_empty {
        if opts.inside_target() && has_trimmed {
            state.derived_from_content = true;
        }
        state.returned = Some(acc);
        return;
    }

    state.returned = Some(eval_step_2i(index.tree, node_id, &mut state.visited));
}

#[derive(Clone, Copy)]
struct LabelledByPresence {
    has_any: bool,
    references_self: bool,
}

fn start_node_eval<'a>(
    index: &LookupIndex<'a>,
    node_id: NodeId,
    opts: EvalOptions,
    visited: &mut HashSet<NodeId>,
) -> StepOutcome<'a> {
    if visited.contains(&node_id) {
        return StepOutcome::Done(String::new());
    }
    let Some(node) = index.tree.node(node_id) else {
        return StepOutcome::Done(String::new());
    };
    let Some(raw) = node.raw_attributes() else {
        return StepOutcome::Done(String::new());
    };
    if hidden::is_text_node(raw) || hidden::is_ua_shadow(raw) {
        return StepOutcome::Done(String::new());
    }

    // Step 2a: Hidden Not Referenced.
    if !opts.visibility.include_hidden() {
        let ignored = hidden::raw_tag(raw).is_some_and(is_ignored_tag);
        if ignored || (!opts.is_embedded_in_hidden_ref() && node.hidden().is_hidden_for_aria()) {
            visited.insert(node_id);
            return StepOutcome::Done(String::new());
        }
    }

    let raw_labelledby = raw
        .browseros
        .as_deref()
        .map_or(&[][..], |browseros| browseros.labelledby.as_slice());
    let resolved_labelledby = if raw_labelledby.is_empty() {
        Vec::new()
    } else {
        index.resolve_refs(node.doc(), raw_labelledby)
    };

    // Step 2b: aria-labelledby (when not already inside an aria-labelledby traversal).
    if !opts.embedded_in_labelledby.is_active() && !resolved_labelledby.is_empty() {
        let self_contained = resolved_labelledby.iter().any(|item| match *item {
            ResolvedRef::Node(target_id) => contains_in_same_doc(index.tree, node_id, target_id),
            ResolvedRef::Unrendered(_) => false,
        });
        return StepOutcome::Push(Frame::LabelledBy {
            node_id,
            opts,
            refs: resolved_labelledby,
            next_index: 0,
            acc: String::new(),
            part_count: 0,
            self_contained,
        });
    }

    let presence = LabelledByPresence {
        has_any: !resolved_labelledby.is_empty(),
        references_self: resolved_labelledby
            .iter()
            .any(|item| matches!(item, ResolvedRef::Node(target_id) if *target_id == node_id)),
    };
    eval_from_step_2c(index, node_id, opts, presence, visited)
}

fn eval_from_step_2c<'a>(
    index: &LookupIndex<'a>,
    node_id: NodeId,
    opts: EvalOptions,
    labelledby: LabelledByPresence,
    visited: &mut HashSet<NodeId>,
) -> StepOutcome<'a> {
    let Some(facts) = NodeFacts::from_tree(index.tree, node_id) else {
        return StepOutcome::Done(String::new());
    };

    // Step 2c: Embedded control.
    if let Some(outcome) = eval_step_2c(index, facts, opts, labelledby, visited) {
        return outcome;
    }

    // Step 2d: aria-label.
    if let Some(aria_label) = facts
        .raw
        .browseros
        .as_deref()
        .and_then(|browseros| browseros.aria_label.as_deref())
        && !role::trim_js_whitespace(aria_label).is_empty()
    {
        visited.insert(node_id);
        return StepOutcome::Done(aria_label.to_owned());
    }

    // Step 2e: Host-language native naming (non-presentational roles only).
    if !facts.role.is_some_and(Role::is_presentational)
        && let Some(outcome) = eval_step_2e(index, facts, opts, labelledby.has_any, visited)
    {
        return outcome;
    }

    // Step 2f + 2h: Name from content.
    if let Some(outcome) = eval_step_2f(index.tree, facts, opts, visited) {
        return outcome;
    }

    // Step 2i + default.
    StepOutcome::Done(eval_step_2i(index.tree, node_id, visited))
}

fn eval_step_2c<'a>(
    index: &LookupIndex<'a>,
    facts: NodeFacts<'_>,
    opts: EvalOptions,
    labelledby: LabelledByPresence,
    visited: &mut HashSet<NodeId>,
) -> Option<StepOutcome<'a>> {
    let embedded = opts.embedded_in_label.is_active()
        || opts.embedded_in_labelledby.is_active()
        || opts.embedded_in_target == Some(EmbeddedTarget::Descendant);
    if !embedded || labelledby.references_self {
        return None;
    }

    let control_role = facts.role?;
    let is_embedded_control_role = matches!(
        control_role,
        Role::Textbox
            | Role::Searchbox
            | Role::Combobox
            | Role::Listbox
            | Role::Progressbar
            | Role::Scrollbar
            | Role::Slider
            | Role::Spinbutton
            | Role::Meter
            | Role::Menu
    );
    if !is_embedded_control_role {
        return None;
    }

    let is_own_label = index
        .labels_for(facts.doc, facts.dom_id)
        .contains(&facts.id);
    if is_own_label {
        return None;
    }

    match control_role {
        Role::Textbox | Role::Searchbox => {
            visited.insert(facts.id);
            if is_redacted(facts.raw) {
                return Some(StepOutcome::Done(redacted_bullets(facts.raw)));
            }
            if is_unredacted_empty(facts.raw) {
                return Some(StepOutcome::Done(String::new()));
            }
            if facts.tag.is_some_and(|t| {
                t.eq_ignore_ascii_case("input") || t.eq_ignore_ascii_case("textarea")
            }) {
                return Some(StepOutcome::Done(masked_or_raw_field_value(facts.raw)));
            }
            let text = collect_dom_text_in_subtree(index.tree, facts.id)
                .unwrap_or_else(|| masked_or_raw_field_value(facts.raw));
            Some(StepOutcome::Done(text))
        }
        Role::Combobox | Role::Listbox => {
            visited.insert(facts.id);
            Some(eval_step_2c_select_or_listbox(
                index.tree,
                facts,
                control_role,
                opts.child_options(),
            ))
        }
        Role::Progressbar | Role::Scrollbar | Role::Slider | Role::Spinbutton | Role::Meter => {
            visited.insert(facts.id);
            Some(StepOutcome::Done(masked_or_raw_field_value(facts.raw)))
        }
        Role::Menu => {
            visited.insert(facts.id);
            Some(StepOutcome::Done(String::new()))
        }
        _ => None,
    }
}

fn eval_step_2c_select_or_listbox<'a>(
    tree: &SnapshotTree,
    facts: NodeFacts<'_>,
    role: Role,
    child_opts: EvalOptions,
) -> StepOutcome<'a> {
    if facts.tag.is_some_and(|t| t.eq_ignore_ascii_case("select")) {
        if let Some(option_nodes) = find_select_option_nodes(tree, facts.id) {
            return StepOutcome::Push(Frame::JoinAll {
                opts: child_opts,
                nodes: option_nodes,
                next_index: 0,
                acc: String::new(),
                part_count: 0,
            });
        }
        return StepOutcome::Done(select_options_fallback_text(facts.raw));
    }

    let listbox_id = if role == Role::Listbox {
        Some(facts.id)
    } else {
        find_first_listbox_descendant(tree, facts.id)
    };
    let selected = listbox_id
        .map(|lb| find_selected_options_in_listbox(tree, lb))
        .unwrap_or_default();

    if selected.is_empty() && facts.tag.is_some_and(|t| t.eq_ignore_ascii_case("input")) {
        return StepOutcome::Done(masked_or_raw_field_value(facts.raw));
    }

    StepOutcome::Push(Frame::JoinAll {
        opts: child_opts,
        nodes: selected,
        next_index: 0,
        acc: String::new(),
        part_count: 0,
    })
}

fn labels_outcome<'a>(labels: &[NodeId], visibility: NameVisibility) -> StepOutcome<'a> {
    StepOutcome::Push(Frame::JoinNonEmptyLabels {
        visibility,
        labels: labels.to_vec(),
        next_index: 0,
        acc: String::new(),
    })
}

fn native_child_tail_call<'a>(
    tree: &SnapshotTree,
    child_id: NodeId,
    child_opts: EvalOptions,
) -> StepOutcome<'a> {
    let child_hidden = tree
        .node(child_id)
        .is_some_and(|child| child.hidden().is_hidden_for_aria());
    StepOutcome::TailCall {
        node_id: child_id,
        opts: EvalOptions {
            embedded_in_native: RefTraversal::from_hidden(child_hidden),
            ..child_opts
        },
    }
}

fn eval_step_2e<'a>(
    index: &LookupIndex<'a>,
    facts: NodeFacts<'_>,
    opts: EvalOptions,
    has_labelledby: bool,
    visited: &mut HashSet<NodeId>,
) -> Option<StepOutcome<'a>> {
    let tag_name = facts.tag?;
    let control_type = form_control_type(facts.raw);

    if tag_name.eq_ignore_ascii_case("input")
        && let Some(outcome) = eval_step_2e_special_input(index, facts, control_type, opts, visited)
    {
        return Some(outcome);
    }

    if !has_labelledby && tag_name.eq_ignore_ascii_case("button") {
        visited.insert(facts.id);
        let labels = index.labels_for(facts.doc, facts.dom_id);
        if !labels.is_empty() {
            return Some(labels_outcome(labels, opts.visibility));
        }
        return None;
    }

    if !has_labelledby && tag_name.eq_ignore_ascii_case("output") {
        visited.insert(facts.id);
        let labels = index.labels_for(facts.doc, facts.dom_id);
        if !labels.is_empty() {
            return Some(labels_outcome(labels, opts.visibility));
        }
        return Some(StepOutcome::Done(raw_title(facts.raw).to_owned()));
    }

    if !has_labelledby
        && (tag_name.eq_ignore_ascii_case("textarea")
            || tag_name.eq_ignore_ascii_case("select")
            || tag_name.eq_ignore_ascii_case("input")
            || tag_name.eq_ignore_ascii_case("meter")
            || tag_name.eq_ignore_ascii_case("progress"))
    {
        visited.insert(facts.id);
        let labels = index.labels_for(facts.doc, facts.dom_id);
        if !labels.is_empty() {
            return Some(labels_outcome(labels, opts.visibility));
        }
        let use_placeholder = tag_name.eq_ignore_ascii_case("textarea")
            || (tag_name.eq_ignore_ascii_case("input")
                && matches!(
                    control_type,
                    None | Some(
                        FormControlType::Unspecified
                            | FormControlType::InputText
                            | FormControlType::InputPassword
                            | FormControlType::InputNumber
                            | FormControlType::InputSearch
                            | FormControlType::InputTelephone
                            | FormControlType::InputEmail
                            | FormControlType::InputUrl
                    )
                ));
        let title = raw_title(facts.raw);
        let result = if !use_placeholder || !title.is_empty() {
            title
        } else {
            raw_placeholder(facts.raw)
        };
        return Some(StepOutcome::Done(result.to_owned()));
    }

    eval_step_2e_containers_and_media(index, facts, tag_name, opts, has_labelledby, visited)
}

fn eval_step_2e_special_input<'a>(
    index: &LookupIndex<'a>,
    facts: NodeFacts<'_>,
    control_type: Option<FormControlType>,
    opts: EvalOptions,
    visited: &mut HashSet<NodeId>,
) -> Option<StepOutcome<'a>> {
    match control_type? {
        FormControlType::InputButton
        | FormControlType::InputSubmit
        | FormControlType::InputReset => {
            visited.insert(facts.id);
            let value = masked_or_raw_field_value(facts.raw);
            if !role::trim_js_whitespace(&value).is_empty() {
                return Some(StepOutcome::Done(value));
            }
            let fallback = match control_type {
                Some(FormControlType::InputSubmit) => "Submit",
                Some(FormControlType::InputReset) => "Reset",
                _ => raw_title(facts.raw),
            };
            Some(StepOutcome::Done(fallback.to_owned()))
        }
        FormControlType::InputFile => {
            visited.insert(facts.id);
            let labels = index.labels_for(facts.doc, facts.dom_id);
            if !labels.is_empty() && !opts.embedded_in_labelledby.is_active() {
                return Some(labels_outcome(labels, opts.visibility));
            }
            Some(StepOutcome::Done("Choose File".to_owned()))
        }
        FormControlType::InputImage => {
            visited.insert(facts.id);
            let labels = index.labels_for(facts.doc, facts.dom_id);
            if !labels.is_empty() && !opts.embedded_in_labelledby.is_active() {
                return Some(labels_outcome(labels, opts.visibility));
            }
            let alt = raw_alt(facts.raw);
            if !role::trim_js_whitespace(alt).is_empty() {
                return Some(StepOutcome::Done(alt.to_owned()));
            }
            let title = raw_title(facts.raw);
            if !role::trim_js_whitespace(title).is_empty() {
                return Some(StepOutcome::Done(title.to_owned()));
            }
            Some(StepOutcome::Done("Submit".to_owned()))
        }
        _ => None,
    }
}

fn eval_step_2e_containers_and_media<'a>(
    index: &LookupIndex<'a>,
    facts: NodeFacts<'_>,
    tag_name: &str,
    opts: EvalOptions,
    has_labelledby: bool,
    visited: &mut HashSet<NodeId>,
) -> Option<StepOutcome<'a>> {
    let tree = index.tree;
    if !has_labelledby && tag_name.eq_ignore_ascii_case("fieldset") {
        visited.insert(facts.id);
        if let Some(legend_id) = find_first_direct_child_tag(tree, facts.id, "legend") {
            return Some(native_child_tail_call(
                tree,
                legend_id,
                opts.child_options(),
            ));
        }
        return Some(StepOutcome::Done(raw_title(facts.raw).to_owned()));
    }

    if !has_labelledby && tag_name.eq_ignore_ascii_case("figure") {
        visited.insert(facts.id);
        if let Some(caption_id) = find_first_direct_child_tag(tree, facts.id, "figcaption") {
            return Some(native_child_tail_call(
                tree,
                caption_id,
                opts.child_options(),
            ));
        }
        return Some(StepOutcome::Done(raw_title(facts.raw).to_owned()));
    }

    if tag_name.eq_ignore_ascii_case("img") || tag_name.eq_ignore_ascii_case("area") {
        visited.insert(facts.id);
        let alt = raw_alt(facts.raw);
        if !role::trim_js_whitespace(alt).is_empty() {
            return Some(StepOutcome::Done(alt.to_owned()));
        }
        return Some(StepOutcome::Done(raw_title(facts.raw).to_owned()));
    }

    if tag_name.eq_ignore_ascii_case("table") {
        visited.insert(facts.id);
        if let Some(caption_id) = find_first_direct_child_tag(tree, facts.id, "caption") {
            return Some(native_child_tail_call(
                tree,
                caption_id,
                opts.child_options(),
            ));
        }
        // Intentionally fall through to Step 2f so descendant tables collect content and top-level
        // tables fall through to Step 2i `title`.
        return None;
    }

    if !tag_name.eq_ignore_ascii_case("foreignobject")
        && let Some(title_id) = find_first_direct_child_tag(tree, facts.id, "title")
        && index.is_in_svg_scope(facts.id)
    {
        visited.insert(facts.id);
        let title_hidden = tree
            .node(title_id)
            .is_some_and(|child| child.hidden().is_hidden_for_aria());
        return Some(StepOutcome::TailCall {
            node_id: title_id,
            opts: EvalOptions {
                embedded_in_labelledby: RefTraversal::from_hidden(title_hidden),
                ..opts.child_options()
            },
        });
    }

    None
}

fn eval_step_2f<'a>(
    tree: &SnapshotTree,
    facts: NodeFacts<'_>,
    opts: EvalOptions,
    visited: &mut HashSet<NodeId>,
) -> Option<StepOutcome<'a>> {
    let summary_from_content = facts.tag.is_some_and(|t| t.eq_ignore_ascii_case("summary"))
        && !facts.role.is_some_and(Role::is_presentational);
    let allowed = allows_name_from_content(
        facts.role,
        opts.embedded_in_target == Some(EmbeddedTarget::Descendant),
    ) || summary_from_content
        || opts.embedded_in_labelledby.is_active()
        || opts.embedded_in_describedby.is_active()
        || opts.embedded_in_label.is_active()
        || opts.embedded_in_native.is_active();
    if !allowed {
        return None;
    }

    visited.insert(facts.id);
    // Privacy guard: never walk child nodes of a redacted control, which could echo sensitive
    // characters even when referenced through `aria-labelledby`.
    if is_redacted(facts.raw) {
        return None;
    }

    let children = collect_logical_children(tree, facts.id);
    Some(StepOutcome::Push(Frame::ContentChildren {
        node_id: facts.id,
        opts,
        children,
        next_index: 0,
        acc: String::new(),
    }))
}

fn eval_step_2i(tree: &SnapshotTree, node_id: NodeId, visited: &mut HashSet<NodeId>) -> String {
    visited.insert(node_id);
    let Some(node) = tree.node(node_id) else {
        return String::new();
    };
    let Some(raw) = node.raw_attributes() else {
        return String::new();
    };
    let tag = hidden::raw_tag(raw);
    let allow_title = !node.role().is_some_and(Role::is_presentational)
        || tag.is_some_and(|t| t.eq_ignore_ascii_case("iframe") || t.eq_ignore_ascii_case("frame"));
    if allow_title {
        let title = raw_title(raw);
        if !role::trim_js_whitespace(title).is_empty() {
            return title.to_owned();
        }
    }
    String::new()
}

fn compute_accessible_name(
    index: &LookupIndex<'_>,
    id: NodeId,
    visibility: NameVisibility,
) -> Name {
    let Some(node) = index.tree.node(id) else {
        return Name::default();
    };
    let Some(raw) = node.raw_attributes() else {
        return Name::default();
    };
    if hidden::is_text_node(raw) || hidden::is_ua_shadow(raw) {
        return Name::default();
    }

    // Step 1: Name-prohibited roles always yield an empty name.
    if is_name_prohibited(node.role()) {
        return Name::default();
    }

    let mut derived_from_content = false;
    let raw_text = eval_text_alternative(
        index,
        id,
        EvalOptions::for_root(visibility),
        &mut derived_from_content,
    );
    Name {
        text: normalize_whitespace(&raw_text),
        derived_from_content,
    }
}

fn compute_accessible_description(
    index: &LookupIndex<'_>,
    id: NodeId,
    visibility: NameVisibility,
) -> Description {
    let Some(node) = index.tree.node(id) else {
        return Description::default();
    };
    let Some(raw) = node.raw_attributes() else {
        return Description::default();
    };
    if hidden::is_text_node(raw) || hidden::is_ua_shadow(raw) {
        return Description::default();
    }

    let browseros = raw.browseros.as_deref();
    if let Some(refs) = browseros
        .map(|attrs| attrs.describedby.as_slice())
        .filter(|slice| !slice.is_empty())
    {
        let resolved = index.resolve_refs(node.doc(), refs);
        let mut joined = String::new();
        let mut derived_from_content = false;

        for (idx, item) in resolved.into_iter().enumerate() {
            if idx > 0 {
                joined.push(' ');
            }
            match item {
                ResolvedRef::Node(target_id) => {
                    if contains_in_same_doc(index.tree, id, target_id) {
                        derived_from_content = true;
                    }
                    let target_hidden = index
                        .tree
                        .node(target_id)
                        .is_some_and(|target| target.hidden().is_hidden_for_aria());
                    let mut ignored_derived = false;
                    let text = eval_text_alternative(
                        index,
                        target_id,
                        EvalOptions::for_describedby(visibility, target_hidden),
                        &mut ignored_derived,
                    );
                    joined.push_str(&text);
                }
                ResolvedRef::Unrendered(text) => {
                    joined.push_str(text);
                }
            }
        }

        return Description {
            text: normalize_whitespace(&joined),
            derived_from_content,
        };
    }

    if let Some(aria_description) = browseros.and_then(|attrs| attrs.aria_description.as_deref()) {
        return Description {
            text: normalize_whitespace(aria_description),
            derived_from_content: false,
        };
    }

    Description {
        text: normalize_whitespace(raw_title(raw)),
        derived_from_content: false,
    }
}

fn compute_accessible_error_message(index: &LookupIndex<'_>, id: NodeId) -> String {
    let Some(node) = index.tree.node(id) else {
        return String::new();
    };
    let Some(raw) = node.raw_attributes() else {
        return String::new();
    };
    if hidden::is_text_node(raw) || hidden::is_ua_shadow(raw) {
        return String::new();
    }
    let Some(browseros) = raw.browseros.as_deref() else {
        return String::new();
    };
    // Match Playwright's `getElementAccessibleErrorMessage`: evaluate `aria-invalid` for every
    // role, not only roles in `kAriaInvalidRoles`.
    if InvalidState::from_aria_attr(browseros.aria_invalid.as_deref()) == InvalidState::False {
        return String::new();
    }
    if browseros.errormessage.is_empty() {
        return String::new();
    }

    let resolved = index.resolve_refs(node.doc(), &browseros.errormessage);
    let mut joined = String::new();
    for (idx, item) in resolved.into_iter().enumerate() {
        if idx > 0 {
            joined.push(' ');
        }
        let part = match item {
            ResolvedRef::Node(target_id) => {
                let target_hidden = index
                    .tree
                    .node(target_id)
                    .is_some_and(|target| target.hidden().is_hidden_for_aria());
                let mut ignored_derived = false;
                let raw_part = eval_text_alternative(
                    index,
                    target_id,
                    EvalOptions::for_describedby(NameVisibility::VisibleOnly, target_hidden),
                    &mut ignored_derived,
                );
                as_flat_string(&raw_part)
            }
            ResolvedRef::Unrendered(text) => as_flat_string(text),
        };
        joined.push_str(&part);
    }

    role::trim_js_whitespace(&joined).to_owned()
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::thread;

    use crate::proto::{
        BrowserOsContentAttributes, BrowserOsRef, FormControlType, RedactionDecision, SelectOption,
    };

    use super::{Description, Name, NameCache, NameVisibility};
    use crate::tree::{
        ApcBuilder, BuildOptions, DocToken, DomNodeId, FrameBuilder, NodeBuilder, NodeId, NodeKey,
        Role, SnapshotTree,
    };

    fn assert_send_sync<T: Send + Sync>() {}

    fn build_tree(root: NodeBuilder) -> Result<SnapshotTree, Box<dyn Error>> {
        let apc = ApcBuilder::new("doc-main").root(root).build();
        Ok(SnapshotTree::build(apc, BuildOptions::default())?)
    }

    fn node_id(tree: &SnapshotTree, doc: &str, dom: i32) -> Result<NodeId, Box<dyn Error>> {
        let token = DocToken::new(doc).ok_or("doc token must be non-empty")?;
        let key = NodeKey::new(token, DomNodeId::new(dom));
        Ok(tree
            .node_by_key(&key)
            .ok_or("expected node key in tree")?
            .id())
    }

    #[test]
    fn name_cache_is_send_and_sync() {
        assert_send_sync::<Name>();
        assert_send_sync::<Description>();
        assert_send_sync::<NameVisibility>();
        assert_send_sync::<NameCache<'_>>();
    }

    #[test]
    fn as_flat_string_preserves_interior_nbsp_and_uses_js_whitespace() {
        let cases: &[(&str, &str, &str)] = &[
            ("", "", "empty string stays empty"),
            ("   \t\r\n  ", "", "ASCII whitespace trims to empty"),
            (
                "  hello \r\n\t world  ",
                "hello world",
                "runs of ASCII whitespace collapse to one space",
            ),
            (
                "a\u{00A0}b",
                "a\u{00A0}b",
                "interior NBSP is preserved by asFlatString",
            ),
            (
                "a  \u{00A0}  b",
                "a \u{00A0} b",
                "spaces adjacent to interior NBSP collapse independently per chunk",
            ),
            (
                " \u{00A0} hello \u{00A0} ",
                "hello",
                "leading and trailing NBSP are stripped by JS trim",
            ),
            (
                "a\u{200B}b\u{00AD}c",
                "abc",
                "zero-width space and soft hyphen are stripped",
            ),
            (
                "a \u{200B} b",
                "a b",
                "stripping zero-width space before collapsing merges surrounding spaces",
            ),
            (
                "\u{0085}hello\u{0085}",
                "\u{0085}hello\u{0085}",
                "U+0085 NEL is not JavaScript whitespace",
            ),
            (
                "\u{FEFF}hello\u{FEFF}world\u{FEFF}",
                "hello world",
                "U+FEFF BOM is JavaScript whitespace",
            ),
        ];

        for &(input, expected, reason) in cases {
            assert_eq!(super::as_flat_string(input), expected, "{reason}");
        }
    }

    #[test]
    fn normalize_whitespace_collapses_nbsp_and_strips_invisible_separators() {
        let cases: &[(&str, &str, &str)] = &[
            ("", "", "empty string stays empty"),
            (
                " \u{00A0}\u{200B}\u{00AD}\u{FEFF} ",
                "",
                "invisible and JS whitespace chars normalize to empty",
            ),
            (
                "  hello \u{00A0}\t\r\n  world  ",
                "hello world",
                "NBSP and ASCII whitespace collapse into a single ASCII space",
            ),
            (
                "a\u{200B}b\u{00AD}c",
                "abc",
                "zero-width space and soft hyphen are removed without inserting spaces",
            ),
            (
                "a \u{200B} b",
                "a b",
                "removing zero-width space joins adjacent spaces before collapsing",
            ),
            (
                "\u{0085}keep\u{0085}nel\u{0085}",
                "\u{0085}keep\u{0085}nel\u{0085}",
                "U+0085 NEL is not JavaScript whitespace and must not be trimmed or collapsed",
            ),
            (
                "\u{FEFF}one\u{FEFF}\u{00A0}two\u{FEFF}",
                "one two",
                "U+FEFF BOM and NBSP both act as JavaScript whitespace",
            ),
        ];

        for &(input, expected, reason) in cases {
            assert_eq!(super::normalize_whitespace(input), expected, "{reason}");
        }
    }

    #[test]
    fn name_prohibited_role_set_matches_playwright_exactly() {
        let expected_prohibited: &[Role] = &[
            Role::Caption,
            Role::Code,
            Role::Definition,
            Role::Deletion,
            Role::Emphasis,
            Role::Generic,
            Role::Insertion,
            Role::Mark,
            Role::Paragraph,
            Role::Presentation,
            Role::Strong,
            Role::Subscript,
            Role::Superscript,
            Role::Term,
            Role::Time,
        ];

        assert!(
            !super::is_name_prohibited(None),
            "elements without a concrete role must not prohibit naming"
        );
        assert!(
            !super::is_name_prohibited(Some(Role::None)),
            "Role::None is intentionally not in Playwright's name-prohibited list"
        );

        for &role in Role::ALL {
            let expected = expected_prohibited.contains(&role);
            assert_eq!(
                super::is_name_prohibited(Some(role)),
                expected,
                "name-prohibited classification mismatch for role {role}"
            );
        }
    }

    #[test]
    fn name_from_content_role_sets_match_playwright_exactly() {
        let expected_always: &[Role] = &[
            Role::Button,
            Role::Cell,
            Role::Checkbox,
            Role::Columnheader,
            Role::Gridcell,
            Role::Heading,
            Role::Link,
            Role::Menuitem,
            Role::Menuitemcheckbox,
            Role::Menuitemradio,
            Role::Option,
            Role::Radio,
            Role::Row,
            Role::Rowheader,
            Role::Switch,
            Role::Tab,
            Role::Tooltip,
            Role::Treeitem,
        ];
        let expected_descendant_only: &[Role] = &[
            Role::Caption,
            Role::Code,
            Role::Contentinfo,
            Role::Definition,
            Role::Deletion,
            Role::Emphasis,
            Role::Insertion,
            Role::List,
            Role::Listitem,
            Role::Mark,
            Role::None,
            Role::Paragraph,
            Role::Presentation,
            Role::Region,
            Role::Rowgroup,
            Role::Strong,
            Role::Subscript,
            Role::Superscript,
            Role::Table,
            Role::Term,
            Role::Time,
        ];

        assert!(
            !super::allows_name_from_content(None, false),
            "unroled elements do not allow name from content at self"
        );
        assert!(
            super::allows_name_from_content(None, true),
            "unroled elements allow name from content as target descendants"
        );
        assert!(
            !super::allows_name_from_content(Some(Role::Generic), true),
            "explicit Role::Generic does not allow name from content even as a descendant"
        );

        for &role in Role::ALL {
            let in_always = expected_always.contains(&role);
            let in_desc = in_always || expected_descendant_only.contains(&role);
            assert_eq!(
                super::allows_name_from_content(Some(role), false),
                in_always,
                "self name-from-content mismatch for role {role}"
            );
            assert_eq!(
                super::allows_name_from_content(Some(role), true),
                in_desc,
                "descendant name-from-content mismatch for role {role}"
            );
        }
    }

    #[test]
    fn debug_formats_redact_page_text_and_oob_ids_return_empty() -> Result<(), Box<dyn Error>> {
        let secret = "TOP_SECRET_VALUE_999";
        let tree = build_tree(
            NodeBuilder::root().child(
                NodeBuilder::element("button")
                    .dom_id(10)
                    .aria_label(secret)
                    .aria_description(secret)
                    .aria_invalid("true"),
            ),
        )?;
        let btn = node_id(&tree, "doc-main", 10)?;
        let cache = tree.name_cache();

        let name = cache.name(btn, NameVisibility::VisibleOnly);
        let desc = cache.description(btn, NameVisibility::VisibleOnly);
        assert_eq!(name.text(), secret, "button should resolve its aria-label");
        assert_eq!(
            desc.text(),
            secret,
            "button should resolve its aria-description"
        );

        let name_dbg = format!("{name:?}");
        let desc_dbg = format!("{desc:?}");
        let cache_dbg = format!("{cache:?}");
        assert!(
            !name_dbg.contains(secret),
            "Name Debug must never expose page text"
        );
        assert!(
            !desc_dbg.contains(secret),
            "Description Debug must never expose page text"
        );
        assert!(
            !cache_dbg.contains(secret),
            "NameCache Debug must never expose page text"
        );

        let oob = NodeId::new(9_999);
        assert!(
            cache.name(oob, NameVisibility::VisibleOnly).is_empty(),
            "out-of-bounds NodeId must return empty Name"
        );
        assert!(
            cache
                .description(oob, NameVisibility::VisibleOnly)
                .is_empty(),
            "out-of-bounds NodeId must return empty Description"
        );
        assert!(
            cache.error_message(oob).is_empty(),
            "out-of-bounds NodeId must return empty error message"
        );
        Ok(())
    }

    #[test]
    fn redacted_controls_emit_bullets_and_never_read_field_value_or_children()
    -> Result<(), Box<dyn Error>> {
        let tree = build_tree(
            NodeBuilder::root()
                .child(
                    NodeBuilder::element("button")
                        .dom_id(10)
                        .child(NodeBuilder::text("Pin:"))
                        .child(
                            NodeBuilder::form_control(FormControlType::InputPassword)
                                .tag("input")
                                .dom_id(11)
                                .field_value("LEAK_ME")
                                .redaction(RedactionDecision::RedactedHasBeenPassword)
                                .redacted_value_length(4),
                        ),
                )
                .child(
                    NodeBuilder::element("button")
                        .dom_id(20)
                        .child(NodeBuilder::text("Empty:"))
                        .child(
                            NodeBuilder::form_control(FormControlType::InputPassword)
                                .tag("input")
                                .dom_id(21)
                                .field_value("SHOULD_BE_EMPTY")
                                .redaction(RedactionDecision::UnredactedEmptyPassword),
                        ),
                )
                .child(
                    NodeBuilder::form_control(FormControlType::InputPassword)
                        .tag("input")
                        .role("button")
                        .dom_id(30)
                        .redaction(RedactionDecision::RedactedIsOtp)
                        .redacted_value_length(6)
                        .child(NodeBuilder::text("LEAK_CHILD_TEXT")),
                ),
        )?;

        let btn1 = node_id(&tree, "doc-main", 10)?;
        let btn2 = node_id(&tree, "doc-main", 20)?;
        let redacted_btn = node_id(&tree, "doc-main", 30)?;

        assert_eq!(
            tree.accessible_name(btn1, NameVisibility::VisibleOnly)
                .text(),
            "Pin:••••",
            "embedded redacted textbox must emit bullets instead of field_value"
        );
        assert_eq!(
            tree.accessible_name(btn2, NameVisibility::VisibleOnly)
                .text(),
            "Empty:",
            "UnredactedEmpty* must emit empty string without reading field_value"
        );
        assert_eq!(
            tree.accessible_name(redacted_btn, NameVisibility::VisibleOnly)
                .text(),
            "",
            "Step 2f must not walk children of a redacted input"
        );
        Ok(())
    }

    #[test]
    fn break_before_on_hidden_child_and_ua_shadow_descent_work() -> Result<(), Box<dyn Error>> {
        let tree = build_tree(
            NodeBuilder::root().child(
                NodeBuilder::element("button")
                    .dom_id(10)
                    .child(
                        NodeBuilder::container()
                            .ua_shadow(true)
                            .child(NodeBuilder::text("SHADOW_NOISE").ua_shadow(true))
                            .child(NodeBuilder::container().child(NodeBuilder::text("Hello"))),
                    )
                    .child(
                        NodeBuilder::container()
                            .aria_hidden(true)
                            .break_before(true)
                            .child(NodeBuilder::text("Secret")),
                    )
                    .child(NodeBuilder::container().child(NodeBuilder::text("World"))),
            ),
        )?;

        let btn = node_id(&tree, "doc-main", 10)?;
        let name = tree.accessible_name(btn, NameVisibility::VisibleOnly);
        assert_eq!(
            name.text(),
            "Hello World",
            "ua_shadow text must be skipped, slotted children read, and break_before on hidden sibling must separate words"
        );
        assert!(
            name.derived_from_content(),
            "button name gathered from descendants must set derived_from_content"
        );
        Ok(())
    }

    #[test]
    fn labelledby_describedby_errormessage_and_iframe_isolation() -> Result<(), Box<dyn Error>> {
        let iframe = FrameBuilder::new("doc-iframe").root(
            NodeBuilder::root().child(
                NodeBuilder::element("span")
                    .dom_id(50)
                    .child(NodeBuilder::text("WRONG_FRAME")),
            ),
        );

        let tree = build_tree(
            NodeBuilder::root()
                .child(NodeBuilder::iframe(iframe))
                .child(
                    NodeBuilder::element("span")
                        .dom_id(50)
                        .aria_hidden(true)
                        .child(NodeBuilder::text("Hidden\u{00A0}Error")),
                )
                .child(
                    NodeBuilder::form_control(FormControlType::InputText)
                        .tag("input")
                        .dom_id(60)
                        .labelledby(vec![
                            BrowserOsRef {
                                dom_node_id: Some(50),
                                unrendered_text: None,
                            },
                            BrowserOsRef {
                                dom_node_id: Some(50),
                                unrendered_text: None,
                            },
                            BrowserOsRef {
                                dom_node_id: None,
                                unrendered_text: Some("Suffix".to_owned()),
                            },
                        ])
                        .aria_description("")
                        .title("Ignored Title")
                        .aria_invalid("true")
                        .errormessage(vec![BrowserOsRef {
                            dom_node_id: Some(50),
                            unrendered_text: None,
                        }]),
                )
                .child(
                    NodeBuilder::element("button")
                        .dom_id(70)
                        .aria_invalid("grammar")
                        .errormessage(vec![BrowserOsRef {
                            dom_node_id: None,
                            unrendered_text: Some("Check\u{00A0}grammar".to_owned()),
                        }]),
                ),
        )?;

        let input = node_id(&tree, "doc-main", 60)?;
        let button = node_id(&tree, "doc-main", 70)?;
        let name = tree.accessible_name(input, NameVisibility::VisibleOnly);
        assert_eq!(
            name.text(),
            "Hidden Error Suffix",
            "labelledby must stay in doc-main, deduplicate repeated dom_node_id, include hidden direct target, and normalize NBSP"
        );
        assert!(
            !name.derived_from_content(),
            "external labelledby target must not set derived_from_content"
        );

        let desc = tree.accessible_description(input, NameVisibility::VisibleOnly);
        assert_eq!(
            desc.text(),
            "",
            "present empty aria-description must take precedence over title"
        );

        let err = tree.accessible_error_message(input);
        assert_eq!(
            err, "Hidden\u{00A0}Error",
            "error_message must apply as_flat_string (preserving interior NBSP) and read hidden direct target"
        );

        assert_eq!(
            tree.accessible_error_message(button),
            "Check\u{00A0}grammar",
            "error_message must evaluate aria-invalid even on roles outside kAriaInvalidRoles"
        );
        Ok(())
    }

    #[test]
    fn self_labelledby_and_embedded_controls_follow_accname() -> Result<(), Box<dyn Error>> {
        let tree = build_tree(
            NodeBuilder::root()
                .child(
                    NodeBuilder::form_control(FormControlType::InputText)
                        .tag("input")
                        .dom_id(10)
                        .aria_label("Username")
                        .field_value("alice")
                        .labelledby(vec![BrowserOsRef {
                            dom_node_id: Some(10),
                            unrendered_text: None,
                        }]),
                )
                .child(
                    NodeBuilder::element("button")
                        .dom_id(20)
                        .child(NodeBuilder::text("Pick "))
                        .child(
                            NodeBuilder::container().role("combobox").child(
                                NodeBuilder::container()
                                    .role("listbox")
                                    .child(
                                        NodeBuilder::container()
                                            .role("option")
                                            .selected(false)
                                            .child(NodeBuilder::text("One")),
                                    )
                                    .child(
                                        NodeBuilder::container()
                                            .role("option")
                                            .selected(true)
                                            .child(NodeBuilder::text("Two")),
                                    ),
                            ),
                        )
                        .child(
                            NodeBuilder::container()
                                .role("menu")
                                .child(NodeBuilder::text("HiddenMenu")),
                        )
                        .child(
                            NodeBuilder::form_control(FormControlType::InputRange)
                                .tag("input")
                                .field_value("42"),
                        ),
                ),
        )?;

        let self_input = node_id(&tree, "doc-main", 10)?;
        let btn = node_id(&tree, "doc-main", 20)?;

        let self_name = tree.accessible_name(self_input, NameVisibility::VisibleOnly);
        assert_eq!(
            self_name.text(),
            "Username",
            "self aria-labelledby must not enter embedded control and must use aria-label"
        );
        assert!(
            self_name.derived_from_content(),
            "self-referencing aria-labelledby marks derived_from_content"
        );

        assert_eq!(
            tree.accessible_name(btn, NameVisibility::VisibleOnly)
                .text(),
            "Pick Two42",
            "embedded combobox picks selected option, menu yields empty, and slider yields field_value"
        );
        Ok(())
    }

    #[test]
    fn select_and_native_naming_precedence() -> Result<(), Box<dyn Error>> {
        let tree =
            build_tree(
                NodeBuilder::root()
                    .child(
                        NodeBuilder::element("button")
                            .dom_id(10)
                            .child(NodeBuilder::text("Choose "))
                            .child(
                                NodeBuilder::form_control(FormControlType::SelectOne)
                                    .tag("select")
                                    .dom_id(11)
                                    .select_option(SelectOption {
                                        value: Some("a".to_owned()),
                                        text: Some("Alpha".to_owned()),
                                        is_selected: Some(false),
                                        is_disabled: None,
                                    })
                                    .select_option(SelectOption {
                                        value: Some("b".to_owned()),
                                        text: Some("Beta".to_owned()),
                                        is_selected: Some(true),
                                        is_disabled: None,
                                    }),
                            ),
                    )
                    .child(
                        NodeBuilder::form_control(FormControlType::InputText)
                            .tag("input")
                            .dom_id(20)
                            .placeholder("Email address"),
                    )
                    .child(
                        NodeBuilder::element("fieldset").dom_id(30).child(
                            NodeBuilder::element("legend").child(NodeBuilder::text("Billing")),
                        ),
                    )
                    .child(
                        NodeBuilder::element("table")
                            .dom_id(40)
                            .title("Summary Table")
                            .child(NodeBuilder::element("tr").child(
                                NodeBuilder::element("td").child(NodeBuilder::text("Cell")),
                            )),
                    )
                    .child(NodeBuilder::element("svg").dom_id(50).child(
                        NodeBuilder::element("title").child(NodeBuilder::text("Icon Title")),
                    )),
            )?;

        let btn = node_id(&tree, "doc-main", 10)?;
        let input = node_id(&tree, "doc-main", 20)?;
        let fieldset = node_id(&tree, "doc-main", 30)?;
        let table = node_id(&tree, "doc-main", 40)?;
        let svg = node_id(&tree, "doc-main", 50)?;

        assert_eq!(
            tree.accessible_name(btn, NameVisibility::VisibleOnly)
                .text(),
            "Choose Beta",
            "embedded <select> without child <option> elements must use selected FormControlData::select_options"
        );
        assert_eq!(
            tree.accessible_name(input, NameVisibility::VisibleOnly)
                .text(),
            "Email address",
            "text input without labels or title must fall back to placeholder"
        );
        assert_eq!(
            tree.accessible_name(fieldset, NameVisibility::VisibleOnly)
                .text(),
            "Billing",
            "fieldset must take its accessible name from its direct <legend> child"
        );
        assert_eq!(
            tree.accessible_name(table, NameVisibility::VisibleOnly)
                .text(),
            "Summary Table",
            "top-level table without caption must fall through to Step 2i title"
        );
        assert_eq!(
            tree.accessible_name(svg, NameVisibility::VisibleOnly)
                .text(),
            "Icon Title",
            "svg must take its accessible name from its direct <title> child"
        );
        Ok(())
    }

    #[test]
    fn deep_tree_computes_name_iteratively_without_stack_overflow() -> Result<(), Box<dyn Error>> {
        let handle = thread::Builder::new()
            .name("name-deep-stack-test".to_owned())
            .stack_size(2 * 1024 * 1024)
            .spawn(|| -> Result<String, String> {
                let mut apc = ApcBuilder::deep_chain(25_000);
                if let Some(root) = apc.root_node.as_mut() {
                    if let Some(attrs) = root.content_attributes.as_mut() {
                        attrs.browseros = Some(Box::new(BrowserOsContentAttributes {
                            tag: Some("button".to_owned()),
                            ..BrowserOsContentAttributes::default()
                        }));
                    }
                    let mut cursor = root;
                    while !cursor.children_nodes.is_empty() {
                        cursor = cursor
                            .children_nodes
                            .first_mut()
                            .ok_or_else(|| "missing chain child".to_owned())?;
                    }
                    cursor.children_nodes.push(
                        ApcBuilder::new("doc-tmp")
                            .root(NodeBuilder::text("DeepLeaf"))
                            .build()
                            .root_node
                            .ok_or_else(|| "missing text node".to_owned())?,
                    );
                }

                let tree = SnapshotTree::build(apc, BuildOptions::default())
                    .map_err(|err| err.to_string())?;
                Ok(tree
                    .accessible_name(NodeId::new(0), NameVisibility::VisibleOnly)
                    .into_text())
            })?;

        let result = handle.join().map_err(|panic_payload| {
            format!("deep stack test thread panicked: {panic_payload:?}")
        })??;
        assert_eq!(
            result, "DeepLeaf",
            "25,000-deep tree must compute name iteratively on a 2 MiB stack"
        );
        Ok(())
    }
}
