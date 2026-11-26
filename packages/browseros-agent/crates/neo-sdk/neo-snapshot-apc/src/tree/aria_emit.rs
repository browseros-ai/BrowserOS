use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::{fmt, mem};

use neo_snapshot::{
    AriaChild, AriaNode, Invalid, MAX_TREE_DEPTH, NodeBox, PartialFrame as SnapshotPartialFrame,
    Snapshot, Tristate,
};
use serde_json::Number;

use crate::proto::{
    BoundingRect, ContentAttributeType, ContentAttributes, ContentData, FormControlData,
    FormControlType,
};
use crate::refs::TabRefTable;

use super::fold::SyntheticOption;
use super::hidden;
use super::mask::{self, MaskSource, NodeMask};
use super::name::{self, NameCache, NameVisibility};
use super::node_key::{DocId, DocSeq, FrameToken, NodeId};
use super::role::Role;
use super::snapshot_tree::SnapshotTree;
use super::state::{InvalidState, State, TriState};
use super::tree_doc::TreeDoc;
use super::tree_node::TreeNode;

/// Maximum JSON nesting depth permitted for an emitted ARIA snapshot across stitched frames.
///
/// Matches `neo-playwright`'s 297-level envelope cap so every emitted [`AriaTree`] stays within
/// the depth that recursive serializers, deserializers, and [`MAX_TREE_DEPTH`] assume.
pub const MAX_JSON_DEPTH: usize = 297;

const MAX_SINGLE_FRAME_DEPTH: usize = MAX_TREE_DEPTH - 1;

const SNAPSHOT_DEPTH_ERROR: &str = "This tree is nested about {levels} levels deep, more than a snapshot carries (about 147). Pass { depth: n } with n of 140 or less to cut it there, or take a locator's snapshot of a smaller section.";

const NAME_PROHIBITED_ROLES: &[&str] = &[
    "caption",
    "code",
    "definition",
    "deletion",
    "emphasis",
    "generic",
    "insertion",
    "mark",
    "paragraph",
    "presentation",
    "strong",
    "subscript",
    "suggestion",
    "superscript",
    "term",
    "time",
];

/// Starting boundary within a [`SnapshotTree`] for [`SnapshotTree::aria`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AriaRoot {
    /// Emit the entire page starting from [`DocId::MAIN`].
    Page,
    /// Emit one stitched document starting from the given [`DocId`] and its nested iframes.
    Document(DocId),
    /// Emit a single subtree rooted at [`NodeId`] without retiring unseen sibling identities in
    /// its owning document.
    Subtree(NodeId),
}

/// Emission mode governing reference assignment, visibility, folding, inert marking, and
/// distillation in [`SnapshotTree::aria`].
pub enum AriaMode<'a> {
    /// AI mode: assigns stable snapshot `e`-refs in `refs`, folds `ClickProxy` labels, marks inert
    /// subtrees, includes box-visible `aria-hidden` elements, and runs full AI distillation.
    Ai {
        /// Per-tab reference table updated in document preorder.
        refs: &'a mut TabRefTable,
        /// Whether to emit `active: Some(true)` on the focused element of each document.
        render_active: bool,
    },
    /// Playwright default mode: omits refs, `active`, `inert`, `ClickProxy` folding, and AI-only
    /// distillation passes.
    Default,
    /// Default mode as Playwright's template matcher reads it (`toMatchAriaSnapshot`, whose tree
    /// is `toAriaNode`'s before `renderAriaTreeAsJSON` drops defaults): every state a node's role
    /// owns is written explicitly, `false` and `0` included, so `[checked=false]` and
    /// `[expanded=false]` can match. Everything else is [`Self::Default`]'s, and so is its YAML,
    /// since the printer drops `false` and `0`.
    Template,
}

impl AriaMode<'_> {
    const fn is_ai(&self) -> bool {
        matches!(self, Self::Ai { .. })
    }
}

impl fmt::Debug for AriaMode<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ai {
                refs,
                render_active,
            } => f
                .debug_struct("Ai")
                .field("refs", refs)
                .field("render_active", render_active)
                .finish(),
            Self::Default => f.write_str("Default"),
            Self::Template => f.write_str("Template"),
        }
    }
}

/// How a node's ARIA states are written. `Wire` is `AriaNodeJSON`'s: only the values Playwright's
/// renderer prints (true, mixed, a level above 0), so AI and default JSON keep their oracles'
/// bytes. `Explicit` is `toAriaNode`'s: every state the role owns, which a template's
/// `[selected=false]` compares against.
#[derive(Clone, Copy)]
enum StateForm {
    Wire,
    Explicit,
}

impl StateForm {
    const fn of(mode: &AriaMode<'_>) -> Self {
        match mode {
            AriaMode::Ai { .. } | AriaMode::Default => Self::Wire,
            AriaMode::Template => Self::Explicit,
        }
    }

    /// Writes `state` onto `draft`. `State` already gates each field by upstream's `kAria*Roles`,
    /// so `None` is `toAriaNode`'s `undefined`, and `Explicit` writes it as it stands; `expanded`
    /// stays `None` when `aria-expanded` is unset, which `[expanded=false]` must not match.
    fn write(self, state: State, draft: &mut DraftNode) {
        let checked = state.checked().map(tristate);
        let pressed = state.pressed().map(tristate);
        let invalid = state.invalid().map(invalid);
        let level = state.level();
        match self {
            Self::Explicit => {
                draft.checked = checked;
                draft.disabled = state.disabled();
                draft.expanded = state.expanded();
                draft.invalid = invalid;
                draft.level = level.map(Number::from);
                draft.pressed = pressed;
                draft.selected = state.selected();
            }
            Self::Wire => {
                draft.checked = checked.filter(|&checked| checked != Tristate::False);
                draft.disabled = state.disabled().filter(|&disabled| disabled);
                draft.expanded = state.expanded().filter(|&expanded| expanded);
                draft.invalid = invalid.filter(|&invalid| invalid != Invalid::False);
                draft.level = level.filter(|&level| level > 0).map(Number::from);
                draft.pressed = pressed.filter(|&pressed| pressed != Tristate::False);
                draft.selected = state.selected().filter(|&selected| selected);
            }
        }
    }

    /// Writes a `<select>`'s synthesized option's states onto `draft`. Role `option` owns
    /// `checked`, `disabled` and `selected`; the option carries no `aria-checked`, so upstream's
    /// `getAriaChecked` reads it `false`.
    fn write_option(self, option: &SyntheticOption, draft: &mut DraftNode) {
        match self {
            Self::Explicit => {
                draft.checked = Some(Tristate::False);
                draft.disabled = Some(option.disabled());
                draft.selected = Some(option.selected());
            }
            Self::Wire => {
                draft.disabled = option.disabled().then_some(true);
                draft.selected = option.selected().then_some(true);
            }
        }
    }
}

/// Request parameters controlling [`SnapshotTree::aria`].
#[must_use]
pub struct AriaAsk<'a> {
    root: AriaRoot,
    mode: AriaMode<'a>,
    depth: Option<usize>,
    boxes: bool,
}

impl<'a> AriaAsk<'a> {
    /// Creates an AI-mode request rooted at [`AriaRoot::Page`] with `render_active: true`, no
    /// explicit depth cutoff, and `boxes: false`.
    pub fn ai(refs: &'a mut TabRefTable) -> Self {
        Self {
            root: AriaRoot::Page,
            mode: AriaMode::Ai {
                refs,
                render_active: true,
            },
            depth: None,
            boxes: false,
        }
    }

    /// Creates a Playwright default-mode request rooted at [`AriaRoot::Page`] with no explicit
    /// depth cutoff and `boxes: false`.
    pub const fn default_mode() -> AriaAsk<'static> {
        AriaAsk {
            root: AriaRoot::Page,
            mode: AriaMode::Default,
            depth: None,
            boxes: false,
        }
    }

    /// Creates a template-mode request ([`AriaMode::Template`]) rooted at [`AriaRoot::Page`] with
    /// no explicit depth cutoff and `boxes: false`: the tree `toMatchAriaSnapshot` matches.
    pub const fn template_mode() -> AriaAsk<'static> {
        AriaAsk {
            root: AriaRoot::Page,
            mode: AriaMode::Template,
            depth: None,
            boxes: false,
        }
    }

    /// Creates a request for an explicit `root` and `mode` with no depth cutoff and `boxes: false`.
    pub const fn new(root: AriaRoot, mode: AriaMode<'a>) -> Self {
        Self {
            root,
            mode,
            depth: None,
            boxes: false,
        }
    }

    /// Overrides the starting [`AriaRoot`] for this request.
    pub const fn with_root(mut self, root: AriaRoot) -> Self {
        self.root = root;
        self
    }

    /// Sets whether `active` focus flags are emitted in [`AriaMode::Ai`]; no-op in
    /// [`AriaMode::Default`] and [`AriaMode::Template`].
    pub const fn with_render_active(mut self, render_active: bool) -> Self {
        if let AriaMode::Ai {
            render_active: slot,
            ..
        } = &mut self.mode
        {
            *slot = render_active;
        }
        self
    }

    /// Sets the depth option as Playwright's `ariaSnapshot({ depth })` takes it: a top-level node
    /// is at depth 0, nodes at `depth` print without children, and `Some(0)` is upstream's falsy
    /// zero, no cut at all. A child frame gets the arithmetic upstream gives it ([`FrameDepth`]).
    pub const fn with_depth(mut self, depth: Option<usize>) -> Self {
        self.depth = depth;
        self
    }

    /// Sets whether rounded CSS-pixel [`NodeBox`] rectangles are attached to emitted nodes.
    pub const fn with_boxes(mut self, boxes: bool) -> Self {
        self.boxes = boxes;
        self
    }

    /// Configured starting [`AriaRoot`].
    #[must_use]
    pub const fn root(&self) -> AriaRoot {
        self.root
    }

    /// Configured optional tree depth limit.
    #[must_use]
    pub const fn depth(&self) -> Option<usize> {
        self.depth
    }

    /// Whether bounding boxes are requested on emitted nodes.
    #[must_use]
    pub const fn boxes(&self) -> bool {
        self.boxes
    }
}

impl fmt::Debug for AriaAsk<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AriaAsk")
            .field("root", &self.root)
            .field("mode", &self.mode)
            .field("depth", &self.depth)
            .field("boxes", &self.boxes)
            .finish()
    }
}

/// Record of an embedded `<iframe>` whose content was omitted, redacted, or refused for depth.
///
/// Its manual `Debug` implementation prints only the reference, frame label byte length, and
/// reason string so URLs never leak into logs.
#[derive(Clone, PartialEq, Eq)]
pub struct AriaPartialFrame {
    /// Snapshot reference of the `<iframe>` node in its parent frame, or `""` when unreferenced.
    pub r#ref: String,
    /// Frame URL or `DevTools` frame token identifying the omitted iframe.
    pub frame: String,
    /// Human-readable explanation for why the iframe content was not attached.
    pub reason: String,
}

impl fmt::Debug for AriaPartialFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AriaPartialFrame")
            .field("ref", &self.r#ref)
            .field("frame_len", &self.frame.len())
            .field("reason", &self.reason)
            .finish()
    }
}

/// Emitted multi-frame ARIA tree and partial-frame diagnostics returned by [`SnapshotTree::aria`].
///
/// Its manual `Debug` implementation prints only node and partial-frame counts so page text and
/// URLs never leak into logs.
#[derive(Clone, PartialEq)]
pub struct AriaTree {
    nodes: Vec<AriaNode>,
    partial: Vec<AriaPartialFrame>,
}

impl AriaTree {
    /// Top-level emitted [`AriaNode`] list in document order with child frames stitched in.
    #[must_use]
    pub fn nodes(&self) -> &[AriaNode] {
        &self.nodes
    }

    /// Embedded iframes that were omitted, redacted, or refused by the [`MAX_JSON_DEPTH`] cap.
    #[must_use]
    pub fn partial(&self) -> &[AriaPartialFrame] {
        &self.partial
    }

    /// Consumes this tree and converts it into a [`Snapshot`] ready for [`neo_snapshot::render`].
    #[must_use]
    pub fn into_snapshot(self) -> Snapshot {
        Snapshot {
            tree: self.nodes,
            partial: self
                .partial
                .into_iter()
                .map(|entry| SnapshotPartialFrame {
                    r#ref: entry.r#ref,
                    frame: entry.frame,
                    reason: entry.reason,
                })
                .collect(),
            layers: Vec::new(),
            closed_roots: None,
            unmarked: None,
        }
    }

    /// Clones this tree into a [`Snapshot`] ready for [`neo_snapshot::render`].
    #[must_use]
    pub fn to_snapshot(&self) -> Snapshot {
        self.clone().into_snapshot()
    }
}

impl fmt::Debug for AriaTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AriaTree")
            .field("node_count", &self.nodes.len())
            .field("partial_count", &self.partial.len())
            .finish()
    }
}

impl SnapshotTree {
    /// Emits a distilled, depth-bounded, multi-frame [`AriaTree`] according to `ask`.
    ///
    /// # Errors
    ///
    /// [`AriaTooDeep`] when the captured document is nested deeper than an [`AriaNode`] tree may
    /// be and the requested depth does not cut it shorter. A child document that deep becomes a
    /// partial frame instead.
    pub fn aria(&self, ask: AriaAsk<'_>) -> Result<AriaTree, AriaTooDeep> {
        emit_aria(self, ask)
    }
}

/// The refusal of [`SnapshotTree::aria`]: the captured tree is nested deeper than
/// [`MAX_TREE_DEPTH`] levels, which every [`AriaNode`] consumer relies on, and no requested depth
/// cuts it shorter. Today's snapshot refuses such a page the same way, before any tree reaches the
/// agent, so the message is the one it prints and tells the agent how to retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AriaTooDeep {
    levels: usize,
}

impl AriaTooDeep {
    /// How many levels deep the refused tree is, a top-level node being level 1.
    #[must_use]
    pub const fn levels(self) -> usize {
        self.levels
    }
}

impl fmt::Display for AriaTooDeep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&depth_error(self.levels))
    }
}

impl Error for AriaTooDeep {}

/// One document's depth option, as upstream's numbers behave (`ariaSnapshot.ts`
/// `renderAriaTreeAsJSON`, `ariaSnapshotDistiller.ts` `beyondDepth`, and neo-playwright's
/// `snapshot_op.rs` `child_depth`). A child frame's option is the parent's minus the iframe's
/// depth minus one, so it can reach zero, which upstream reads as none, or go negative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameDepth {
    /// No cut: no depth asked, or a zero one.
    Unlimited,
    /// Nodes at this depth print without children, and deeper ones do not print. Never zero.
    Cut(usize),
    /// Below zero: the frame prints whole, yet its distiller counts every node as past the cut,
    /// so no name is dropped as redundant. An iframe at its parent's cut reaches this.
    PastCut,
}

impl FrameDepth {
    const fn requested(depth: Option<usize>) -> Self {
        match depth {
            None | Some(0) => Self::Unlimited,
            Some(depth) => Self::Cut(depth),
        }
    }

    /// The option of the document under an iframe printed at `iframe_depth`.
    const fn for_child(self, iframe_depth: usize) -> Self {
        match self {
            Self::Unlimited => Self::Unlimited,
            Self::PastCut => Self::PastCut,
            Self::Cut(depth) => match depth.checked_sub(iframe_depth + 1) {
                None => Self::PastCut,
                Some(0) => Self::Unlimited,
                Some(child) => Self::Cut(child),
            },
        }
    }

    /// The depth whose nodes print without children, when this frame has one.
    const fn print_cut(self) -> Option<usize> {
        match self {
            Self::Cut(depth) => Some(depth),
            Self::Unlimited | Self::PastCut => None,
        }
    }

    /// Whether the distiller treats a node at `depth` as unprinted.
    const fn is_beyond(self, depth: usize) -> bool {
        match self {
            Self::Unlimited => false,
            Self::Cut(cut) => depth > cut,
            Self::PastCut => true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DistillMode {
    Ai,
    Default,
}

impl DistillMode {
    const fn is_ai(self) -> bool {
        matches!(self, Self::Ai)
    }
}

#[derive(Clone, Copy)]
enum CaptureScope {
    FullDoc,
    Subtree(NodeId),
}

impl CaptureScope {
    /// Whether the capture holds `node`. A `ClickProxy` label folds into its control's line only
    /// when the capture holds the control too; otherwise the label's text would vanish with it.
    fn holds(self, tree: &SnapshotTree, node: NodeId) -> bool {
        match self {
            Self::FullDoc => true,
            Self::Subtree(root) => tree
                .node(root)
                .is_some_and(|root| root.subtree_range().contains(node)),
        }
    }
}

struct DocJob {
    job_idx: usize,
    doc_id: DocId,
    scope: CaptureScope,
    depth: FrameDepth,
    /// Levels of the stitched tree above this document's top nodes, for a refusal's message.
    levels_above: usize,
}

/// A document's materialized top nodes and the iframes among them, in document order.
struct EmittedFrames {
    nodes: Vec<AriaNode>,
    iframe_slots: Vec<EmittedIframe>,
}

/// The iframe slots of one document resolved for stitching, and the child documents to emit.
struct ScheduledFrames {
    slots: Vec<ResolvedIframeSlot>,
    jobs: Vec<DocJob>,
}

enum EmittedDoc {
    Frames {
        nodes: Vec<AriaNode>,
        iframe_slots: Vec<ResolvedIframeSlot>,
    },
    TooDeep(AriaTooDeep),
}

enum ResolvedIframeSlot {
    ChildJob {
        job_idx: usize,
        iframe_ref: String,
        frame_label: String,
    },
    Partial {
        iframe_ref: String,
        frame_label: String,
        reason: String,
    },
    Empty,
}

struct StitchedDoc {
    nodes: Vec<AriaNode>,
    partial: Vec<AriaPartialFrame>,
}

struct EmittedIframe {
    iframe_ref: String,
    kind: EmittedIframeKind,
    tree_depth: usize,
}

enum EmittedIframeKind {
    ChildDoc { doc_id: DocId, frame_label: String },
    Partial { frame_label: String, reason: String },
    Empty,
}

#[derive(Clone, Copy, Default)]
struct WalkFlags(u8);

impl WalkFlags {
    const PARENT_VISIBLE: u8 = 1 << 0;
    const IN_FOLDED_LABEL: u8 = 1 << 1;
    const SUPPRESS_TEXT: u8 = 1 << 2;

    const fn visible_root() -> Self {
        Self(Self::PARENT_VISIBLE)
    }

    const fn parent_visible(self) -> bool {
        (self.0 & Self::PARENT_VISIBLE) != 0
    }

    const fn in_folded_label(self) -> bool {
        (self.0 & Self::IN_FOLDED_LABEL) != 0
    }

    const fn suppress_text(self) -> bool {
        (self.0 & Self::SUPPRESS_TEXT) != 0
    }

    const fn with_parent_visible(self, enabled: bool) -> Self {
        self.with_bit(Self::PARENT_VISIBLE, enabled)
    }

    const fn with_folded_label(self, enabled: bool) -> Self {
        self.with_bit(Self::IN_FOLDED_LABEL, enabled)
    }

    const fn with_suppress_text(self, enabled: bool) -> Self {
        self.with_bit(Self::SUPPRESS_TEXT, enabled)
    }

    const fn with_bit(self, bit: u8, enabled: bool) -> Self {
        if enabled {
            Self(self.0 | bit)
        } else {
            Self(self.0 & !bit)
        }
    }
}

#[derive(Clone, Copy, Default)]
struct DraftFlags(u8);

impl DraftFlags {
    const POINTER_CURSOR: u8 = 1 << 0;
    const ELEMENT: u8 = 1 << 1;
    const INERT: u8 = 1 << 2;
    const MARK_INERT: u8 = 1 << 3;

    const fn pointer_cursor(self) -> bool {
        (self.0 & Self::POINTER_CURSOR) != 0
    }

    const fn is_element(self) -> bool {
        (self.0 & Self::ELEMENT) != 0
    }

    const fn is_inert(self) -> bool {
        (self.0 & Self::INERT) != 0
    }

    const fn mark_inert(self) -> bool {
        (self.0 & Self::MARK_INERT) != 0
    }

    fn set_mark_inert(&mut self) {
        self.0 |= Self::MARK_INERT;
    }

    const fn with_pointer_cursor(self, enabled: bool) -> Self {
        self.with_bit(Self::POINTER_CURSOR, enabled)
    }

    const fn with_element(self, enabled: bool) -> Self {
        self.with_bit(Self::ELEMENT, enabled)
    }

    const fn with_inert(self, enabled: bool) -> Self {
        self.with_bit(Self::INERT, enabled)
    }

    const fn with_bit(self, bit: u8, enabled: bool) -> Self {
        if enabled {
            Self(self.0 | bit)
        } else {
            Self(self.0 & !bit)
        }
    }
}

enum DraftChild {
    Text(String),
    Node(usize),
}

enum DraftIframe {
    None,
    ChildDoc { doc_id: DocId, frame_label: String },
    Partial { frame_label: String, reason: String },
}

struct DraftNode {
    role: &'static str,
    name: String,
    checked: Option<Tristate>,
    disabled: Option<bool>,
    expanded: Option<bool>,
    active: Option<bool>,
    invalid: Option<Invalid>,
    level: Option<Number>,
    pressed: Option<Tristate>,
    selected: Option<bool>,
    aria_hidden: Option<bool>,
    r#ref: Option<String>,
    url: Option<String>,
    placeholder: Option<String>,
    source_node: Option<NodeId>,
    name_from_content_drafts: Vec<usize>,
    flags: DraftFlags,
    iframe: DraftIframe,
    children: Vec<DraftChild>,
}

impl DraftNode {
    fn fragment() -> Self {
        Self::new("fragment", String::new(), DraftFlags::default())
    }

    fn new(role: &'static str, name: String, flags: DraftFlags) -> Self {
        Self {
            role,
            name,
            checked: None,
            disabled: None,
            expanded: None,
            active: None,
            invalid: None,
            level: None,
            pressed: None,
            selected: None,
            aria_hidden: None,
            r#ref: None,
            url: None,
            placeholder: None,
            source_node: None,
            name_from_content_drafts: Vec::new(),
            flags,
            iframe: DraftIframe::None,
            children: Vec::new(),
        }
    }

    const fn has_props(&self) -> bool {
        self.url.is_some() || self.placeholder.is_some() || self.aria_hidden.is_some()
    }
}

struct DraftArena {
    nodes: Vec<DraftNode>,
}

impl DraftArena {
    fn new() -> Self {
        Self {
            nodes: vec![DraftNode::fragment()],
        }
    }

    fn push(&mut self, node: DraftNode) -> usize {
        let index = self.nodes.len();
        self.nodes.push(node);
        index
    }

    fn push_child(&mut self, parent_idx: usize, child: DraftChild) {
        if let Some(parent) = self.nodes.get_mut(parent_idx) {
            parent.children.push(child);
        }
    }
}

fn emit_aria(tree: &SnapshotTree, ask: AriaAsk<'_>) -> Result<AriaTree, AriaTooDeep> {
    let AriaAsk {
        root,
        mut mode,
        depth,
        boxes,
    } = ask;
    let Some(first_job) = resolve_initial_job(tree, root, FrameDepth::requested(depth)) else {
        return Ok(AriaTree {
            nodes: Vec::new(),
            partial: Vec::new(),
        });
    };

    let scale = if boxes { compute_box_scale(tree) } else { 1.0 };
    let name_cache = NameCache::new(tree);

    let mut emitted_docs: Vec<Option<EmittedDoc>> = vec![None];
    let mut emit_order: Vec<usize> = Vec::new();
    let mut job_stack: Vec<DocJob> = vec![first_job];

    while let Some(job) = job_stack.pop() {
        emit_order.push(job.job_idx);
        let emitted = match emit_single_doc(tree, &name_cache, &mut mode, &job, boxes, scale) {
            Ok(frames) => {
                // Only ai mode reads past an iframe: a child frame's snapshot hangs from its
                // iframe's ref, and default mode mints none, so it reads one document as
                // upstream's `ariaSnapshot` and template matcher read one frame (`- iframe`).
                let slots = if mode.is_ai() {
                    frames.iframe_slots
                } else {
                    frames
                        .iframe_slots
                        .into_iter()
                        .map(|slot| EmittedIframe {
                            kind: EmittedIframeKind::Empty,
                            ..slot
                        })
                        .collect()
                };
                let scheduled = schedule_child_jobs(slots, &job, &mut emitted_docs);
                job_stack.extend(scheduled.jobs.into_iter().rev());
                EmittedDoc::Frames {
                    nodes: frames.nodes,
                    iframe_slots: scheduled.slots,
                }
            }
            // The captured document itself: nothing to stand in for it, so the whole ask fails.
            Err(too_deep) if job.job_idx == 0 => return Err(too_deep),
            Err(too_deep) => EmittedDoc::TooDeep(too_deep),
        };
        if let Some(slot) = emitted_docs.get_mut(job.job_idx) {
            *slot = Some(emitted);
        }
    }

    let root_stitched = stitch_documents(&emit_order, &mut emitted_docs);
    Ok(AriaTree {
        nodes: root_stitched.nodes,
        partial: root_stitched.partial,
    })
}

fn resolve_initial_job(tree: &SnapshotTree, root: AriaRoot, depth: FrameDepth) -> Option<DocJob> {
    match root {
        AriaRoot::Page => {
            if tree.nodes().is_empty() || tree.document(DocId::MAIN).is_none() {
                return None;
            }
            Some(DocJob {
                job_idx: 0,
                doc_id: DocId::MAIN,
                scope: CaptureScope::FullDoc,
                depth,
                levels_above: 0,
            })
        }
        AriaRoot::Document(doc_id) => {
            let doc = tree.document(doc_id)?;
            if doc.node_range().is_empty() {
                return None;
            }
            Some(DocJob {
                job_idx: 0,
                doc_id,
                scope: CaptureScope::FullDoc,
                depth,
                levels_above: 0,
            })
        }
        AriaRoot::Subtree(node_id) => {
            let node = tree.node(node_id)?;
            tree.document(node.doc())?;
            Some(DocJob {
                job_idx: 0,
                doc_id: node.doc(),
                scope: CaptureScope::Subtree(node_id),
                depth,
                levels_above: 0,
            })
        }
    }
}

/// Gives every printed iframe's document its own job. An iframe at its parent's cut prints, so its
/// document is emitted too, whole, as upstream fetches it with a negative depth.
fn schedule_child_jobs(
    raw_slots: Vec<EmittedIframe>,
    parent: &DocJob,
    emitted_docs: &mut Vec<Option<EmittedDoc>>,
) -> ScheduledFrames {
    let mut resolved = Vec::with_capacity(raw_slots.len());
    let mut child_jobs = Vec::new();
    for slot in raw_slots {
        match slot.kind {
            EmittedIframeKind::ChildDoc {
                doc_id,
                frame_label,
            } => {
                let child_job_idx = emitted_docs.len();
                emitted_docs.push(None);
                resolved.push(ResolvedIframeSlot::ChildJob {
                    job_idx: child_job_idx,
                    iframe_ref: slot.iframe_ref,
                    frame_label,
                });
                child_jobs.push(DocJob {
                    job_idx: child_job_idx,
                    doc_id,
                    scope: CaptureScope::FullDoc,
                    depth: parent.depth.for_child(slot.tree_depth),
                    levels_above: parent.levels_above + slot.tree_depth + 1,
                });
            }
            EmittedIframeKind::Partial {
                frame_label,
                reason,
            } => {
                resolved.push(ResolvedIframeSlot::Partial {
                    iframe_ref: slot.iframe_ref,
                    frame_label,
                    reason,
                });
            }
            EmittedIframeKind::Empty => resolved.push(ResolvedIframeSlot::Empty),
        }
    }
    ScheduledFrames {
        slots: resolved,
        jobs: child_jobs,
    }
}

fn emit_single_doc(
    tree: &SnapshotTree,
    name_cache: &NameCache<'_>,
    mode: &mut AriaMode<'_>,
    job: &DocJob,
    boxes: bool,
    scale: f64,
) -> Result<EmittedFrames, AriaTooDeep> {
    let empty = || EmittedFrames {
        nodes: Vec::new(),
        iframe_slots: Vec::new(),
    };
    let Some(doc) = tree.document(job.doc_id) else {
        return Ok(empty());
    };
    if doc.node_range().is_empty() {
        return Ok(empty());
    }

    let is_full_doc = matches!(job.scope, CaptureScope::FullDoc);
    let distill_mode = if mode.is_ai() {
        DistillMode::Ai
    } else {
        DistillMode::Default
    };
    let doc_seq = begin_doc_refs(mode, doc, is_full_doc);
    let mut arena = run_doc_build(tree, name_cache, mode, doc, job.scope, doc_seq);
    if is_full_doc && let AriaMode::Ai { refs, .. } = mode {
        refs.finish_doc_capture(doc.token(), |key| tree.node_by_key(key).is_some());
    }

    if distill_mode.is_ai() {
        resolve_name_from_content_drafts(&mut arena, name_cache);
    }
    distill_draft(&mut arena, distill_mode, job.depth);
    if distill_mode.is_ai() {
        mark_inert_draft(&mut arena);
    }

    // The cap keeps every AriaNode tree within MAX_TREE_DEPTH levels. When it, and not the asked
    // depth, would cut content, the document is refused rather than shown incomplete.
    let mut effective_max_depth = job.depth.print_cut().map_or(MAX_SINGLE_FRAME_DEPTH, |cut| {
        cut.min(MAX_SINGLE_FRAME_DEPTH)
    });
    if effective_max_depth == MAX_SINGLE_FRAME_DEPTH {
        let levels = draft_levels(&arena);
        // The reader's 297 levels of JSON hold one level more than MAX_TREE_DEPTH when the
        // deepest nodes are bare leaves (no box, no child array: `neo_snapshot::MAX_TREE_DEPTH`),
        // so a document that ends in such leaves one level past it is no deeper than one the
        // page's engine returned.
        let bare_extra_level = levels == MAX_TREE_DEPTH + 1
            && job.levels_above == 0
            && !boxes
            && deepest_drafts_are_bare(&arena, levels);
        if levels > MAX_TREE_DEPTH && !bare_extra_level {
            return Err(AriaTooDeep {
                levels: job.levels_above + levels,
            });
        }
        // Those leaves are materialized, not cut at the usual last level.
        if bare_extra_level {
            effective_max_depth += 1;
        }
    }
    let frame_origin = if boxes {
        doc_frame_origin(tree, doc)
    } else {
        FrameOrigin::default()
    };
    let mat_cfg = MaterializeConfig {
        tree,
        mode: distill_mode,
        boxes,
        frame_origin,
        scale,
        effective_max_depth,
    };
    Ok(materialize_draft(&mut arena, &mat_cfg))
}

/// How many levels the distilled tree has, a top-level node being level 1, walked without
/// recursion.
fn draft_levels(arena: &DraftArena) -> usize {
    let mut deepest = 0;
    let mut stack: Vec<(usize, usize)> = arena.nodes.first().map_or_else(Vec::new, |root| {
        root.children
            .iter()
            .filter_map(|child| match child {
                DraftChild::Node(idx) => Some((*idx, 1)),
                DraftChild::Text(_) => None,
            })
            .collect()
    });
    while let Some((idx, level)) = stack.pop() {
        deepest = deepest.max(level);
        if let Some(node) = arena.nodes.get(idx) {
            stack.extend(node.children.iter().filter_map(|child| match child {
                DraftChild::Node(child_idx) => Some((*child_idx, level + 1)),
                DraftChild::Text(_) => None,
            }));
        }
    }
    deepest
}

/// Whether every draft at `level` (top-level drafts are level 1) is a leaf that writes no child
/// array: no node child and at most one string, which the node then carries as its `text`.
fn deepest_drafts_are_bare(arena: &DraftArena, level: usize) -> bool {
    let mut stack: Vec<(usize, usize)> = arena.nodes.first().map_or_else(Vec::new, |root| {
        root.children
            .iter()
            .filter_map(|child| match child {
                DraftChild::Node(idx) => Some((*idx, 1)),
                DraftChild::Text(_) => None,
            })
            .collect()
    });
    while let Some((idx, at)) = stack.pop() {
        let Some(node) = arena.nodes.get(idx) else {
            continue;
        };
        if at == level {
            let nodes = node
                .children
                .iter()
                .filter(|child| matches!(child, DraftChild::Node(_)))
                .count();
            if nodes > 0 || node.children.len() > 1 {
                return false;
            }
            continue;
        }
        stack.extend(node.children.iter().filter_map(|child| match child {
            DraftChild::Node(child_idx) => Some((*child_idx, at + 1)),
            DraftChild::Text(_) => None,
        }));
    }
    true
}

fn begin_doc_refs(mode: &mut AriaMode<'_>, doc: &TreeDoc, is_full_doc: bool) -> Option<DocSeq> {
    let AriaMode::Ai { refs, .. } = mode else {
        return None;
    };
    let seq = if is_full_doc {
        refs.begin_doc_capture(doc.token(), doc.seq())
    } else {
        refs.seq_for_doc(doc.token(), doc.seq())
    };
    Some(seq)
}

fn run_doc_build(
    tree: &SnapshotTree,
    name_cache: &NameCache<'_>,
    mode: &mut AriaMode<'_>,
    doc: &TreeDoc,
    scope: CaptureScope,
    doc_seq: Option<DocSeq>,
) -> DraftArena {
    let state_form = StateForm::of(mode);
    let (refs, render_active, distill_mode) = match mode {
        AriaMode::Ai {
            refs,
            render_active,
        } => (Some(&mut **refs), *render_active, DistillMode::Ai),
        AriaMode::Default | AriaMode::Template => (None, false, DistillMode::Default),
    };
    let mut ctx = DocBuildCtx {
        tree,
        name_cache,
        refs,
        doc_seq,
        mode: distill_mode,
        state_form,
        render_active,
        scope,
        arena: DraftArena::new(),
    };
    build_doc_draft(&mut ctx, doc, scope);
    ctx.arena
}

struct DocBuildCtx<'t, 'c, 'r> {
    tree: &'t SnapshotTree,
    name_cache: &'c NameCache<'t>,
    refs: Option<&'r mut TabRefTable>,
    doc_seq: Option<DocSeq>,
    mode: DistillMode,
    state_form: StateForm,
    render_active: bool,
    scope: CaptureScope,
    arena: DraftArena,
}

impl DocBuildCtx<'_, '_, '_> {
    fn assign_ref(&mut self, node_id: NodeId, role: &str, name: &str) -> Option<String> {
        let doc_seq = self.doc_seq?;
        let refs = self.refs.as_deref_mut()?;
        // A label this capture folds never gets here: it passes through before its element is
        // visited. One that reaches here stands on its own line, so the unfolded rule applies.
        if !self.tree.is_ref_worthy_unfolded(node_id) {
            return None;
        }
        let key = self.tree.node_key(node_id)?;
        let click_proxy = self
            .tree
            .fold()
            .click_proxy(node_id)
            .map(|proxy| proxy.label_key.clone());
        let assigned = refs.assign_snapshot_ref(doc_seq, &key, role, name, click_proxy);
        Some(assigned.to_string())
    }
}

#[derive(Clone, Copy)]
struct WalkFrame {
    node_id: NodeId,
    parent_draft: usize,
    flags: WalkFlags,
}

fn build_doc_draft(ctx: &mut DocBuildCtx<'_, '_, '_>, doc: &TreeDoc, scope: CaptureScope) {
    let seeds = collect_doc_seeds(ctx.tree, doc, scope);
    let mut stack: Vec<WalkFrame> = seeds
        .into_iter()
        .rev()
        .map(|node_id| WalkFrame {
            node_id,
            parent_draft: 0,
            flags: WalkFlags::visible_root(),
        })
        .collect();

    while let Some(frame) = stack.pop() {
        process_walk_frame(ctx, frame, &mut stack);
    }
}

fn collect_doc_seeds(tree: &SnapshotTree, doc: &TreeDoc, scope: CaptureScope) -> Vec<NodeId> {
    if let CaptureScope::Subtree(node_id) = scope {
        let is_root = tree
            .node(node_id)
            .and_then(TreeNode::raw_attributes)
            .is_some_and(is_root_attr);
        return if is_root {
            tree.fold()
                .logical_children(tree.nodes(), node_id)
                .collect()
        } else {
            vec![node_id]
        };
    }

    let top_nodes: Vec<NodeId> = if let Some(iframe_id) = doc.iframe_node() {
        tree.children(iframe_id)
            .filter(|child| child.doc() == doc.id())
            .map(TreeNode::id)
            .collect()
    } else {
        vec![doc.node_range().start()]
    };

    let mut seeds = Vec::new();
    for root_id in top_nodes {
        let raw = tree.node(root_id).and_then(TreeNode::raw_attributes);
        if raw.is_some_and(is_root_attr) {
            for child_id in tree.fold().logical_children(tree.nodes(), root_id) {
                let child_raw = tree.node(child_id).and_then(TreeNode::raw_attributes);
                if child_raw.is_some_and(is_unroled_html) {
                    seeds.extend(tree.fold().logical_children(tree.nodes(), child_id));
                } else {
                    seeds.push(child_id);
                }
            }
        } else if raw.is_some_and(is_unroled_html) {
            seeds.extend(tree.fold().logical_children(tree.nodes(), root_id));
        } else {
            seeds.push(root_id);
        }
    }
    seeds
}

fn process_walk_frame(
    ctx: &mut DocBuildCtx<'_, '_, '_>,
    frame: WalkFrame,
    stack: &mut Vec<WalkFrame>,
) {
    let Some(node) = ctx.tree.node(frame.node_id) else {
        return;
    };
    let Some(raw) = node.raw_attributes() else {
        if !node.is_iframe_boundary() {
            push_logical_children_reversed(
                ctx.tree,
                frame.node_id,
                frame.parent_draft,
                frame.flags,
                stack,
            );
        }
        return;
    };

    if !frame.flags.suppress_text()
        && !frame.flags.in_folded_label()
        && raw
            .browseros
            .as_deref()
            .is_some_and(|b| b.break_before == Some(true))
    {
        ctx.arena
            .push_child(frame.parent_draft, DraftChild::Text(" ".to_owned()));
    }

    if hidden::is_text_node(raw) {
        visit_text_node(ctx, node, raw, frame);
        return;
    }

    if hidden::raw_tag(raw).is_some_and(is_ignored_tag) {
        return;
    }

    let is_ai = ctx.mode.is_ai();
    let is_iframe = is_iframe_element(node, raw);
    if !is_element_visible(ctx.tree, node, is_ai) {
        if is_ai && !is_iframe {
            let child_flags = frame.flags.with_parent_visible(false);
            push_logical_children_reversed(
                ctx.tree,
                frame.node_id,
                frame.parent_draft,
                child_flags,
                stack,
            );
        }
        return;
    }

    if let Some(passthrough_flags) =
        folded_label_passthrough(ctx.tree, node, frame.flags, ctx.mode, is_iframe, ctx.scope)
    {
        push_logical_children_reversed(
            ctx.tree,
            frame.node_id,
            frame.parent_draft,
            passthrough_flags,
            stack,
        );
        return;
    }

    if is_iframe {
        visit_iframe_node(ctx, node, raw, frame.parent_draft);
    } else {
        visit_element_node(ctx, node, raw, frame, stack);
    }
}

fn is_element_visible(tree: &SnapshotTree, node: &TreeNode, is_ai: bool) -> bool {
    if is_ai {
        !node.hidden().is_hidden_for_aria()
            || node.hidden().is_box_visible()
            || tree.fold().click_proxy(node.id()).is_some()
    } else {
        !node.hidden().is_hidden_for_aria()
    }
}

fn folded_label_passthrough(
    tree: &SnapshotTree,
    node: &TreeNode,
    flags: WalkFlags,
    mode: DistillMode,
    is_iframe: bool,
    scope: CaptureScope,
) -> Option<WalkFlags> {
    if mode.is_ai()
        && tree
            .fold()
            .folded_label_target(node.id())
            .is_some_and(|control| scope.holds(tree, control))
    {
        return Some(
            flags
                .with_parent_visible(true)
                .with_folded_label(true)
                .with_suppress_text(false),
        );
    }
    if flags.in_folded_label() && !should_preserve_in_folded_label(tree, node, is_iframe) {
        return Some(flags.with_parent_visible(true));
    }
    None
}

fn should_preserve_in_folded_label(tree: &SnapshotTree, node: &TreeNode, is_iframe: bool) -> bool {
    if is_iframe {
        return true;
    }
    let has_real_role = node
        .role()
        .is_some_and(|role| !matches!(role, Role::Presentation | Role::None | Role::Generic));
    has_real_role
        || tree.fold().click_proxy(node.id()).is_some()
        || tree.fold().is_clickable_container(node.id())
}

fn visit_text_node(
    ctx: &mut DocBuildCtx<'_, '_, '_>,
    node: &TreeNode,
    raw: &ContentAttributes,
    frame: WalkFrame,
) {
    if !frame.flags.parent_visible() || frame.flags.in_folded_label() || frame.flags.suppress_text()
    {
        return;
    }
    if !ctx.mode.is_ai() && node.hidden().is_hidden_for_aria() {
        return;
    }
    if let Some(text) = extract_text_content(raw)
        && !text.is_empty()
    {
        ctx.arena
            .push_child(frame.parent_draft, DraftChild::Text(text.to_owned()));
    }
}

fn visit_iframe_node(
    ctx: &mut DocBuildCtx<'_, '_, '_>,
    node: &TreeNode,
    raw: &ContentAttributes,
    parent_draft: usize,
) {
    let is_ai = ctx.mode.is_ai();
    let render_active = ctx.render_active;
    let assigned_ref = ctx.assign_ref(node.id(), "iframe", "");
    let flags = DraftFlags::default()
        .with_element(true)
        .with_inert(node.state().inert())
        .with_pointer_cursor(is_ai && ctx.tree.fold().is_clickable_container(node.id()));

    let mut draft = DraftNode::new("iframe", String::new(), flags);
    draft.r#ref = assigned_ref;
    draft.active = (is_ai && render_active && node.state().active_element()).then_some(true);
    draft.aria_hidden = (is_ai && node.hidden().has_own_aria_hidden()).then_some(true);
    draft.source_node = Some(node.id());
    draft.iframe = resolve_draft_iframe(ctx.tree, node, raw);

    let draft_idx = ctx.arena.push(draft);
    ctx.arena
        .push_child(parent_draft, DraftChild::Node(draft_idx));
}

fn resolve_draft_iframe(
    tree: &SnapshotTree,
    node: &TreeNode,
    raw: &ContentAttributes,
) -> DraftIframe {
    let href = raw
        .browseros
        .as_deref()
        .and_then(|b| b.href.as_deref())
        .filter(|value| !value.is_empty());

    if let Some(child_doc_id) = node.child_doc() {
        let child_doc = tree.document(child_doc_id);
        let label = child_doc
            .and_then(TreeDoc::url)
            .filter(|value| !value.is_empty())
            .or(href)
            .or_else(|| {
                child_doc
                    .and_then(TreeDoc::frame_token)
                    .map(FrameToken::as_str)
            })
            .unwrap_or_default()
            .to_owned();
        return DraftIframe::ChildDoc {
            doc_id: child_doc_id,
            frame_label: label,
        };
    }

    if let Some(pf_idx) = node.partial_frame_index()
        && let Some(pf) = tree.partial_frame(pf_idx)
    {
        let label = href
            .or_else(|| pf.frame_token.as_ref().map(FrameToken::as_str))
            .unwrap_or_default()
            .to_owned();
        return DraftIframe::Partial {
            frame_label: label,
            reason: pf.reason.to_string(),
        };
    }

    DraftIframe::None
}

fn visit_element_node(
    ctx: &mut DocBuildCtx<'_, '_, '_>,
    node: &TreeNode,
    raw: &ContentAttributes,
    frame: WalkFrame,
    stack: &mut Vec<WalkFrame>,
) {
    let is_ai = ctx.mode.is_ai();
    let effective_role = match node.role() {
        Some(Role::Presentation | Role::None) => None,
        Some(role) => Some(role.as_str()),
        None => {
            if is_ai && !ctx.tree.fold().is_plain_prunable_container(node.id()) {
                Some("generic")
            } else {
                None
            }
        }
    };

    let Some(role_str) = effective_role else {
        let mut child_flags = frame
            .flags
            .with_parent_visible(true)
            .with_folded_label(false);
        if let NodeMask::Redacted {
            source: MaskSource::Chromium,
            bullets,
        } = node.mask()
        {
            if bullets > 0 {
                ctx.arena
                    .push_child(frame.parent_draft, DraftChild::Text("•".repeat(bullets)));
            }
            child_flags = child_flags.with_suppress_text(true);
        }
        push_logical_children_reversed(ctx.tree, node.id(), frame.parent_draft, child_flags, stack);
        return;
    };

    let raw_name = ctx
        .name_cache
        .name(node.id(), NameVisibility::VisibleOnly)
        .text()
        .to_owned();
    let assigned_ref = ctx.assign_ref(node.id(), role_str, &raw_name);
    let mut draft = build_element_draft(ctx, node, raw, role_str, raw_name, assigned_ref);

    let child_suppress = populate_field_value_and_mask(node, raw, &mut draft, frame.flags);
    let draft_idx = ctx.arena.push(draft);
    push_synthetic_options(ctx.tree, node, draft_idx, ctx.state_form, &mut ctx.arena);
    ctx.arena
        .push_child(frame.parent_draft, DraftChild::Node(draft_idx));

    let child_flags = WalkFlags::visible_root().with_suppress_text(child_suppress);
    push_logical_children_reversed(ctx.tree, node.id(), draft_idx, child_flags, stack);
}

fn build_element_draft(
    ctx: &DocBuildCtx<'_, '_, '_>,
    node: &TreeNode,
    raw: &ContentAttributes,
    role_str: &'static str,
    raw_name: String,
    assigned_ref: Option<String>,
) -> DraftNode {
    let is_ai = ctx.mode.is_ai();
    let render_active = ctx.render_active;
    let state = node.state();
    let flags = DraftFlags::default()
        .with_element(true)
        .with_inert(state.inert())
        .with_pointer_cursor(is_ai && ctx.tree.fold().is_clickable_container(node.id()));

    let url = if role_str == "link" {
        raw.browseros
            .as_deref()
            .and_then(|b| b.href.as_deref())
            .map(truncate_data_url)
    } else {
        None
    };

    let placeholder = if role_str == "textbox" {
        form_control_data(raw)
            .and_then(|control| control.placeholder.as_deref())
            .filter(|&ph| !ph.is_empty() && ph != raw_name)
            .map(str::to_owned)
    } else {
        None
    };

    let mut draft = DraftNode::new(role_str, raw_name, flags);
    ctx.state_form.write(state, &mut draft);
    draft.active = (is_ai && render_active && state.active_element()).then_some(true);
    draft.aria_hidden = (is_ai && node.hidden().has_own_aria_hidden()).then_some(true);
    draft.r#ref = assigned_ref;
    draft.url = url;
    draft.placeholder = placeholder;
    draft.source_node = Some(node.id());
    draft
}

fn populate_field_value_and_mask(
    node: &TreeNode,
    raw: &ContentAttributes,
    draft: &mut DraftNode,
    flags: WalkFlags,
) -> bool {
    let tag = hidden::raw_tag(raw);
    let is_input = tag.is_some_and(|t| t.eq_ignore_ascii_case("input"));
    let is_textarea = tag.is_some_and(|t| t.eq_ignore_ascii_case("textarea"));
    if is_input || is_textarea {
        let emits_value = is_textarea
            || !matches!(
                form_control_type(raw),
                Some(
                    FormControlType::InputCheckbox
                        | FormControlType::InputRadio
                        | FormControlType::InputFile
                        | FormControlType::InputHidden
                        | FormControlType::InputImage
                )
            );
        if emits_value
            && let Some(val) = mask::effective_field_value(raw, node.mask())
            && !val.is_empty()
        {
            draft.children.push(DraftChild::Text(val));
        }
        return true;
    }

    if let NodeMask::Redacted {
        source: MaskSource::Chromium,
        bullets,
    } = node.mask()
    {
        if bullets > 0 {
            draft.children.push(DraftChild::Text("•".repeat(bullets)));
        }
        return true;
    }

    flags.suppress_text()
}

fn push_synthetic_options(
    tree: &SnapshotTree,
    select_node: &TreeNode,
    select_draft_idx: usize,
    state_form: StateForm,
    arena: &mut DraftArena,
) {
    let synthetic = tree.fold().synthetic_options(select_node.id());
    if synthetic.is_empty() {
        return;
    }
    let flags = DraftFlags::default()
        .with_element(true)
        .with_inert(select_node.state().inert());
    for opt in synthetic {
        let mut draft = DraftNode::new("option", opt.name().to_owned(), flags);
        state_form.write_option(opt, &mut draft);
        let opt_idx = arena.push(draft);
        arena.push_child(select_draft_idx, DraftChild::Node(opt_idx));
    }
}

fn push_logical_children_reversed(
    tree: &SnapshotTree,
    parent_id: NodeId,
    parent_draft: usize,
    flags: WalkFlags,
    stack: &mut Vec<WalkFrame>,
) {
    let start = stack.len();
    for child_id in tree.fold().logical_children(tree.nodes(), parent_id) {
        stack.push(WalkFrame {
            node_id: child_id,
            parent_draft,
            flags,
        });
    }
    if let Some(added) = stack.get_mut(start..) {
        added.reverse();
    }
}

/// Records, on every ref'd draft, the other ref'd drafts whose text its accessible name holds:
/// the half of Playwright's `removeRedundantNames` that runs while the tree is built
/// (`ariaSnapshot.ts` `visit`, `nameFromContentRefs`). [`distill_draft`] then drops a name whose
/// every contributor is printed anyway.
///
/// A draft index stands in for Playwright's ref string: within one document's arena each names
/// exactly one element. Playwright turns contributors into refs as soon as the node's own subtree
/// is visited, so a contributor visited after that, such as an `aria-labelledby` target further
/// down the page, has no ref yet and never counts. Drafts are allocated in the same preorder, so
/// the contributors that count are the ones allocated before the node's subtree ends.
fn resolve_name_from_content_drafts(arena: &mut DraftArena, name_cache: &NameCache<'_>) {
    let subtree_ends = draft_subtree_ends(&arena.nodes);
    let ref_draft_of: HashMap<NodeId, usize> = arena
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, draft)| draft.r#ref.is_some())
        .filter_map(|(draft_idx, draft)| Some((draft.source_node?, draft_idx)))
        .collect();

    for (draft_idx, draft) in arena.nodes.iter_mut().enumerate() {
        if draft.r#ref.is_none() {
            continue;
        }
        let Some(source) = draft.source_node else {
            continue;
        };
        let visited_by = subtree_ends
            .get(draft_idx)
            .copied()
            .unwrap_or(draft_idx.saturating_add(1));
        draft.name_from_content_drafts = name_cache
            .name(source, NameVisibility::VisibleOnly)
            .contributors()
            .iter()
            .filter_map(|contributor| ref_draft_of.get(contributor).copied())
            .filter(|&contrib_draft| contrib_draft != draft_idx && contrib_draft < visited_by)
            .collect();
    }
}

/// One past the last draft allocated while each draft's subtree was built. The builder allocates
/// drafts in preorder, so a child's index always exceeds its parent's and one reverse sweep is
/// enough.
fn draft_subtree_ends(nodes: &[DraftNode]) -> Vec<usize> {
    let mut ends: Vec<usize> = (1..=nodes.len()).collect();
    for (draft_idx, draft) in nodes.iter().enumerate().rev() {
        let end = draft
            .children
            .iter()
            .filter_map(|child| match child {
                DraftChild::Node(child_idx) => ends.get(*child_idx).copied(),
                DraftChild::Text(_) => None,
            })
            .fold(draft_idx.saturating_add(1), usize::max);
        if let Some(slot) = ends.get_mut(draft_idx) {
            *slot = end;
        }
    }
    ends
}

struct DistillFrame {
    node_idx: usize,
    depth: usize,
    pending_children: Vec<DraftChild>,
    kept_children: Vec<DraftChild>,
    ancestor_has_pointer: bool,
    self_sets_pointer: bool,
}

/// Ports Playwright's distiller (`ariaSnapshotDistiller.ts`) as one iterative traversal.
///
/// Default mode runs the normalize plugins (merge text, unwrap single-child generics). Ai mode runs
/// `aiPlugins` in upstream order: the enter hook of `removeRedundantNames` as a child is entered,
/// then on exit `mergeStringChildren`, `removeNamelessImages`, `removeRedundantNames`,
/// `inlineTextIntoGeneric`, `removeNameRepeatingChild` and `unwrapSingleChildGenerics`. The first
/// hook that removes or unwraps a node ends its chain. `pending_content` is upstream's
/// `pendingContentRefs` keyed by draft index. Depth counts the root's children as 0, as upstream
/// does, and `frame_depth` says which nodes lie past the cut.
fn distill_draft(arena: &mut DraftArena, mode: DistillMode, frame_depth: FrameDepth) {
    let mut root_pending = arena
        .nodes
        .first_mut()
        .map_or_else(Vec::new, |root| mem::take(&mut root.children));
    root_pending.reverse();

    let mut pending_content: HashSet<usize> = HashSet::new();
    let mut stack = vec![DistillFrame {
        node_idx: 0,
        depth: 0,
        pending_children: root_pending,
        kept_children: Vec::new(),
        ancestor_has_pointer: false,
        self_sets_pointer: false,
    }];

    while let Some(top) = stack.last_mut() {
        if let Some(child) = top.pending_children.pop() {
            match child {
                DraftChild::Text(text) => top.kept_children.push(DraftChild::Text(text)),
                DraftChild::Node(child_idx) => {
                    let child_ancestor_pointer = top.ancestor_has_pointer || top.self_sets_pointer;
                    let child_depth = if top.node_idx == 0 {
                        0
                    } else {
                        top.depth.saturating_add(1)
                    };
                    let (child_sets_pointer, mut child_pending) = arena
                        .nodes
                        .get_mut(child_idx)
                        .map_or((false, Vec::new()), |node| {
                            if mode.is_ai() {
                                enter_redundant_name_tracking(
                                    child_idx,
                                    node,
                                    frame_depth.is_beyond(child_depth),
                                    &mut pending_content,
                                );
                            }
                            let sets = node.r#ref.is_some() && node.flags.pointer_cursor();
                            (sets, mem::take(&mut node.children))
                        });
                    child_pending.reverse();
                    stack.push(DistillFrame {
                        node_idx: child_idx,
                        depth: child_depth,
                        pending_children: child_pending,
                        kept_children: Vec::new(),
                        ancestor_has_pointer: child_ancestor_pointer,
                        self_sets_pointer: child_sets_pointer,
                    });
                }
            }
            continue;
        }

        let Some(finished) = stack.pop() else {
            break;
        };
        if let Some(node) = arena.nodes.get_mut(finished.node_idx) {
            node.children = finished.kept_children;
            merge_string_children(node);
        }

        let Some(parent_frame) = stack.last_mut() else {
            continue;
        };
        let is_click_target_root = finished.self_sets_pointer && !finished.ancestor_has_pointer;
        apply_distill_exit(
            &mut arena.nodes,
            finished.node_idx,
            parent_frame,
            mode,
            is_click_target_root,
            &mut pending_content,
        );
    }
}

/// `removeRedundantNames.enter`: queue the node's name contributors as not yet printed, then mark
/// the node itself printed. A leaf generic only supplies text that a kept name may absorb, and a
/// node beyond the depth cut is never printed, so neither counts as printed.
fn enter_redundant_name_tracking(
    draft_idx: usize,
    node: &DraftNode,
    beyond_depth: bool,
    pending_content: &mut HashSet<usize>,
) {
    if node.r#ref.is_none() {
        return;
    }
    for &contrib_idx in &node.name_from_content_drafts {
        pending_content.insert(contrib_idx);
    }
    let is_leaf_generic = node.role == "generic"
        && node
            .children
            .iter()
            .all(|child| matches!(child, DraftChild::Text(_)));
    if !beyond_depth && !is_leaf_generic {
        pending_content.remove(&draft_idx);
    }
}

/// `removeRedundantNames.exit`: when every contributor is already shown, printed itself or carried
/// by a name kept below, the node's name only repeats them and is cleared. Otherwise the kept name
/// now carries their text, so they stop counting as unshown for the ancestors' names.
fn exit_redundant_name_tracking(node: &mut DraftNode, pending_content: &mut HashSet<usize>) {
    if node.r#ref.is_none() || node.name_from_content_drafts.is_empty() {
        return;
    }
    if node
        .name_from_content_drafts
        .iter()
        .all(|contrib_idx| !pending_content.contains(contrib_idx))
    {
        node.name.clear();
    } else {
        for &contrib_idx in &node.name_from_content_drafts {
            pending_content.remove(&contrib_idx);
        }
    }
}

fn apply_distill_exit(
    nodes: &mut [DraftNode],
    node_idx: usize,
    parent_frame: &mut DistillFrame,
    mode: DistillMode,
    is_click_target_root: bool,
    pending_content: &mut HashSet<usize>,
) {
    if mode.is_ai() {
        let remove_img = nodes.get(node_idx).is_some_and(|node| {
            node.role == "img"
                && node.name.is_empty()
                && node.children.is_empty()
                && !is_click_target_root
        });
        if remove_img {
            return;
        }
        if let Some(node) = nodes.get_mut(node_idx) {
            exit_redundant_name_tracking(node, pending_content);
        }
        inline_text_into_generic(nodes, node_idx);
        let (parent_name, parent_is_inert) = nodes
            .get(parent_frame.node_idx)
            .map_or(("", false), |parent| {
                (parent.name.as_str(), parent.flags.is_inert())
            });
        let remove_repeating = nodes.get(node_idx).is_some_and(|node| {
            should_remove_name_repeating_child(node, parent_name, parent_is_inert)
        });
        if remove_repeating {
            // The removed text now survives only in the names derived from it, so those names
            // must stay: the node counts as unprinted again, as upstream re-adds its ref.
            if nodes.get(node_idx).is_some_and(|node| node.r#ref.is_some()) {
                pending_content.insert(node_idx);
            }
            return;
        }
    }

    let unwrap = nodes
        .get(node_idx)
        .is_some_and(|node| should_unwrap_single_child_generic(nodes, node, is_click_target_root));
    if unwrap {
        if let Some(node) = nodes.get_mut(node_idx) {
            parent_frame
                .kept_children
                .extend(mem::take(&mut node.children));
        }
        return;
    }

    parent_frame.kept_children.push(DraftChild::Node(node_idx));
}

fn merge_string_children(node: &mut DraftNode) {
    let old = mem::take(&mut node.children);
    let mut merged = Vec::with_capacity(old.len());
    let mut buf = String::new();

    for child in old {
        match child {
            DraftChild::Text(text) => buf.push_str(&text),
            DraftChild::Node(id) => {
                flush_text_buffer(&mut buf, &mut merged);
                merged.push(DraftChild::Node(id));
            }
        }
    }
    flush_text_buffer(&mut buf, &mut merged);

    if let [DraftChild::Text(only)] = merged.as_slice()
        && !node.name.is_empty()
        && only == &node.name
    {
        merged.clear();
    }
    node.children = merged;
}

fn flush_text_buffer(buf: &mut String, out: &mut Vec<DraftChild>) {
    if buf.is_empty() {
        return;
    }
    let normalized = name::normalize_whitespace(buf);
    buf.clear();
    if !normalized.is_empty() {
        out.push(DraftChild::Text(normalized));
    }
}

fn inline_text_into_generic(nodes: &mut [DraftNode], idx: usize) {
    let Some((sole_child_idx, node_is_inert)) = nodes.get(idx).and_then(|node| {
        if node.role != "generic" || node.has_props() || node.children.len() != 1 {
            return None;
        }
        match node.children.first() {
            Some(DraftChild::Node(child_idx)) => Some((*child_idx, node.flags.is_inert())),
            _ => None,
        }
    }) else {
        return;
    };

    let inlined_text = nodes.get(sole_child_idx).and_then(|child| {
        // A click target keeps its own line, as the old marking's leaf rule had it: inlined into
        // its parent, its ref and pointer mark would go, and the v1 text would print its text
        // with nothing to click. Upstream's plugin has no such case, since it reads no cursor.
        if child.role != "generic"
            || !child.name.is_empty()
            || child.active == Some(true)
            || child.has_props()
            || child.flags.is_inert() != node_is_inert
            || (child.r#ref.is_some() && child.flags.pointer_cursor())
        {
            return None;
        }
        match child.children.as_slice() {
            [DraftChild::Text(text)] => Some(text.clone()),
            _ => None,
        }
    });

    if let Some(text) = inlined_text
        && let Some(node) = nodes.get_mut(idx)
    {
        node.children = vec![DraftChild::Text(text)];
    }
}

fn should_remove_name_repeating_child(
    node: &DraftNode,
    parent_name: &str,
    parent_is_inert: bool,
) -> bool {
    if parent_name.is_empty()
        || node.role != "generic"
        || node.active == Some(true)
        || node.has_props()
        || node.flags.is_inert() != parent_is_inert
    {
        return false;
    }
    let text = if !node.name.is_empty() {
        node.children.is_empty().then_some(node.name.as_str())
    } else if let [DraftChild::Text(only)] = node.children.as_slice() {
        Some(only.as_str())
    } else {
        None
    };
    text == Some(parent_name)
}

/// Upstream's `unwrapSingleChildGenerics` (ariaSnapshotDistiller.ts at v1.64.1). Unlike the
/// two rules above it does not spare an active generic: a focused document's body that holds a
/// single control unwraps, and its `[active]` goes with it, as upstream prints.
fn should_unwrap_single_child_generic(
    nodes: &[DraftNode],
    node: &DraftNode,
    is_click_target_root: bool,
) -> bool {
    if node.role != "generic"
        || !node.name.is_empty()
        || node.has_props()
        || node.children.len() > 1
    {
        return false;
    }
    let all_ref_nodes = node.children.iter().all(|child| {
        matches!(
            child,
            DraftChild::Node(cid) if nodes.get(*cid).is_some_and(|c| c.r#ref.is_some())
        )
    });
    if !all_ref_nodes {
        return false;
    }
    !(node.children.is_empty() && is_click_target_root)
}

fn mark_inert_draft(arena: &mut DraftArena) {
    let mut order = Vec::with_capacity(arena.nodes.len());
    let mut scan = vec![0usize];
    while let Some(idx) = scan.pop() {
        order.push(idx);
        if let Some(node) = arena.nodes.get(idx) {
            for child in &node.children {
                if let DraftChild::Node(child_idx) = *child {
                    scan.push(child_idx);
                }
            }
        }
    }

    let mut protected = vec![false; arena.nodes.len()];
    for &idx in order.iter().rev() {
        let is_protected = arena.nodes.get(idx).is_some_and(|node| {
            let own_live = !node.flags.is_inert() && node.flags.is_element();
            own_live
                || node.children.iter().any(|child| {
                    matches!(
                        child,
                        DraftChild::Node(cid) if protected.get(*cid).copied().unwrap_or(false)
                    )
                })
        });
        if let Some(slot) = protected.get_mut(idx) {
            *slot = is_protected;
        }
    }

    let mut walk = vec![0usize];
    while let Some(idx) = walk.pop() {
        let is_protected = protected.get(idx).copied().unwrap_or(false);
        let Some(node) = arena.nodes.get_mut(idx) else {
            continue;
        };
        if node.flags.is_inert() && !is_protected {
            node.flags.set_mark_inert();
            continue;
        }
        for child in &node.children {
            if let DraftChild::Node(child_idx) = *child {
                walk.push(child_idx);
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct FrameOrigin {
    x: i32,
    y: i32,
}

struct MaterializeConfig<'a> {
    tree: &'a SnapshotTree,
    mode: DistillMode,
    boxes: bool,
    frame_origin: FrameOrigin,
    scale: f64,
    effective_max_depth: usize,
}

struct MatFrame {
    node: AriaNode,
    pending_children: Vec<DraftChild>,
    tree_depth: usize,
    child_ancestor_has_pointer: bool,
}

fn materialize_draft(arena: &mut DraftArena, cfg: &MaterializeConfig<'_>) -> EmittedFrames {
    let root_children = arena
        .nodes
        .first_mut()
        .map_or_else(Vec::new, |root| mem::take(&mut root.children));

    let mut top_nodes = Vec::with_capacity(root_children.len());
    let mut iframe_slots = Vec::new();
    let mut stack: Vec<MatFrame> = Vec::new();

    for top_child in root_children {
        match top_child {
            DraftChild::Text(text) => top_nodes.push(text_aria_node(text)),
            DraftChild::Node(idx) => {
                let Some(frame) = start_mat_frame(arena, cfg, idx, 0, false, &mut iframe_slots)
                else {
                    continue;
                };
                stack.push(frame);
                drain_mat_stack(arena, cfg, &mut stack, &mut iframe_slots, &mut top_nodes);
            }
        }
    }

    EmittedFrames {
        nodes: top_nodes,
        iframe_slots,
    }
}

fn drain_mat_stack(
    arena: &mut DraftArena,
    cfg: &MaterializeConfig<'_>,
    stack: &mut Vec<MatFrame>,
    iframe_slots: &mut Vec<EmittedIframe>,
    top_nodes: &mut Vec<AriaNode>,
) {
    while let Some(top) = stack.last_mut() {
        if let Some(child) = top.pending_children.pop() {
            match child {
                DraftChild::Text(text) => top.node.children.push(AriaChild::Text(text)),
                DraftChild::Node(child_idx) => {
                    let next_depth = top.tree_depth.saturating_add(1);
                    let anc_pointer = top.child_ancestor_has_pointer;
                    if let Some(child_frame) = start_mat_frame(
                        arena,
                        cfg,
                        child_idx,
                        next_depth,
                        anc_pointer,
                        iframe_slots,
                    ) {
                        stack.push(child_frame);
                    }
                }
            }
            continue;
        }

        let Some(mut finished) = stack.pop() else {
            break;
        };
        apply_d20_prohibited_name(&mut finished.node);
        if let Some(parent) = stack.last_mut() {
            parent
                .node
                .children
                .push(AriaChild::Node(Box::new(finished.node)));
        } else {
            top_nodes.push(finished.node);
        }
    }
}

fn start_mat_frame(
    arena: &mut DraftArena,
    cfg: &MaterializeConfig<'_>,
    idx: usize,
    tree_depth: usize,
    ancestor_has_pointer: bool,
    iframe_slots: &mut Vec<EmittedIframe>,
) -> Option<MatFrame> {
    let draft = arena.nodes.get_mut(idx)?;
    let is_at_depth_limit = tree_depth >= cfg.effective_max_depth;
    if draft.role == "iframe" {
        record_iframe_slot(draft, tree_depth, iframe_slots);
    }

    let sets_pointer = cfg.mode.is_ai()
        && draft.r#ref.is_some()
        && draft.flags.pointer_cursor()
        && !ancestor_has_pointer;
    let node_box = if cfg.boxes {
        draft
            .source_node
            .and_then(|node_id| compute_node_box(cfg.tree, node_id, cfg.frame_origin, cfg.scale))
    } else {
        None
    };

    let draft_name = mem::take(&mut draft.name);
    let mut node = AriaNode {
        role: draft.role.to_owned(),
        name: (!draft_name.is_empty()).then_some(draft_name),
        checked: draft.checked,
        disabled: draft.disabled,
        expanded: draft.expanded,
        active: draft.active,
        inert: draft.flags.mark_inert().then_some(true),
        on_top: None,
        invalid: draft.invalid,
        level: draft.level.take(),
        pressed: draft.pressed,
        selected: draft.selected,
        aria_hidden: draft.aria_hidden,
        r#ref: draft.r#ref.take(),
        cursor: sets_pointer.then(|| "pointer".to_owned()),
        r#box: node_box,
        url: draft.url.take(),
        placeholder: draft.placeholder.take(),
        text: None,
        children: Vec::new(),
    };

    let mut draft_children = mem::take(&mut draft.children);
    let mut pending_children = Vec::new();
    if draft_children.len() == 1
        && matches!(draft_children.first(), Some(DraftChild::Text(_)))
        && let Some(DraftChild::Text(only)) = draft_children.pop()
    {
        node.text = Some(only);
    } else if !is_at_depth_limit && !draft_children.is_empty() {
        draft_children.reverse();
        pending_children = draft_children;
    }

    Some(MatFrame {
        node,
        pending_children,
        tree_depth,
        child_ancestor_has_pointer: ancestor_has_pointer || sets_pointer,
    })
}

fn record_iframe_slot(
    draft: &mut DraftNode,
    tree_depth: usize,
    iframe_slots: &mut Vec<EmittedIframe>,
) {
    let kind = match mem::replace(&mut draft.iframe, DraftIframe::None) {
        DraftIframe::ChildDoc {
            doc_id,
            frame_label,
        } => EmittedIframeKind::ChildDoc {
            doc_id,
            frame_label,
        },
        DraftIframe::Partial {
            frame_label,
            reason,
        } => EmittedIframeKind::Partial {
            frame_label,
            reason,
        },
        DraftIframe::None => EmittedIframeKind::Empty,
    };
    iframe_slots.push(EmittedIframe {
        iframe_ref: draft.r#ref.clone().unwrap_or_default(),
        kind,
        tree_depth,
    });
}

fn apply_d20_prohibited_name(node: &mut AriaNode) {
    if NAME_PROHIBITED_ROLES.contains(&node.role.as_str()) {
        let taken = node.name.take();
        if node.text.is_none() && node.children.is_empty() {
            node.text = taken;
        }
    }
}

/// A document stitched with its children, or the refusal its iframe slot reports instead.
type StitchedSlot = Option<Result<StitchedDoc, AriaTooDeep>>;

fn stitch_documents(emit_order: &[usize], emitted_docs: &mut [Option<EmittedDoc>]) -> StitchedDoc {
    let mut stitched: Vec<StitchedSlot> = Vec::with_capacity(emitted_docs.len());
    stitched.resize_with(emitted_docs.len(), || None);

    for &job_idx in emit_order.iter().rev() {
        let done = match emitted_docs.get_mut(job_idx).and_then(Option::take) {
            Some(EmittedDoc::Frames {
                nodes,
                iframe_slots,
            }) => Ok(stitch_single_doc(nodes, iframe_slots, &mut stitched)),
            Some(EmittedDoc::TooDeep(too_deep)) => Err(too_deep),
            None => continue,
        };
        if let Some(slot) = stitched.get_mut(job_idx) {
            *slot = Some(done);
        }
    }

    match stitched.first_mut().and_then(Option::take) {
        Some(Ok(root)) => root,
        _ => StitchedDoc {
            nodes: Vec::new(),
            partial: Vec::new(),
        },
    }
}

fn stitch_single_doc(
    mut nodes: Vec<AriaNode>,
    mut iframe_slots: Vec<ResolvedIframeSlot>,
    stitched: &mut [StitchedSlot],
) -> StitchedDoc {
    let mut doc_partial = Vec::new();

    if !iframe_slots.is_empty() {
        let mut walk: Vec<(&mut AriaNode, usize)> =
            nodes.iter_mut().rev().map(|node| (node, 3)).collect();
        let mut slot_cursor = 0usize;

        while let Some((node, obj_depth)) = walk.pop() {
            if node.role == "iframe" {
                if let Some(slot) = iframe_slots.get_mut(slot_cursor) {
                    slot_cursor += 1;
                    attach_iframe_slot(
                        node,
                        mem::replace(slot, ResolvedIframeSlot::Empty),
                        obj_depth,
                        stitched,
                        &mut doc_partial,
                    );
                }
                continue;
            }
            let next_depth = obj_depth.saturating_add(2);
            for child in node.children.iter_mut().rev() {
                if let AriaChild::Node(child_node) = child {
                    walk.push((child_node, next_depth));
                }
            }
        }
    }

    StitchedDoc {
        nodes,
        partial: doc_partial,
    }
}

fn attach_iframe_slot(
    node: &mut AriaNode,
    slot: ResolvedIframeSlot,
    obj_depth: usize,
    stitched: &mut [StitchedSlot],
    doc_partial: &mut Vec<AriaPartialFrame>,
) {
    match slot {
        ResolvedIframeSlot::ChildJob {
            job_idx,
            iframe_ref,
            frame_label,
        } => {
            let child = match stitched.get_mut(job_idx).and_then(Option::take) {
                Some(Ok(child)) => child,
                Some(Err(too_deep)) => {
                    doc_partial.push(AriaPartialFrame {
                        r#ref: iframe_ref,
                        frame: frame_label,
                        reason: too_deep.to_string(),
                    });
                    return;
                }
                None => return,
            };
            if !child.nodes.is_empty() {
                let deepest = neo_snapshot::json_depth(&child.nodes);
                let total_depth = obj_depth.saturating_add(deepest);
                if total_depth > MAX_JSON_DEPTH {
                    doc_partial.push(AriaPartialFrame {
                        r#ref: iframe_ref,
                        frame: frame_label,
                        reason: depth_error(total_depth.saturating_sub(1) / 2),
                    });
                } else {
                    node.children = child
                        .nodes
                        .into_iter()
                        .map(|child_node| AriaChild::Node(Box::new(child_node)))
                        .collect();
                    doc_partial.extend(child.partial);
                }
            }
        }
        ResolvedIframeSlot::Partial {
            iframe_ref,
            frame_label,
            reason,
        } => {
            doc_partial.push(AriaPartialFrame {
                r#ref: iframe_ref,
                frame: frame_label,
                reason,
            });
        }
        ResolvedIframeSlot::Empty => {}
    }
}

fn compute_box_scale(tree: &SnapshotTree) -> f64 {
    let Some(viewport) = tree.viewport() else {
        return 1.0;
    };
    if viewport.width <= 0 {
        return 1.0;
    }
    let root_width = tree
        .nodes()
        .first()
        .and_then(TreeNode::raw_attributes)
        .and_then(|raw| raw.geometry.as_ref())
        .and_then(|geo| {
            geo.visible_bounding_box
                .and_then(|rect| rect.width.filter(|&w| w > 0))
                .or_else(|| {
                    geo.outer_bounding_box
                        .and_then(|rect| rect.width.filter(|&w| w > 0))
                })
        });
    let Some(root_w) = root_width else {
        return 1.0;
    };
    let scale = f64::from(root_w) / f64::from(viewport.width);
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

fn doc_frame_origin(tree: &SnapshotTree, doc: &TreeDoc) -> FrameOrigin {
    if doc.id() == DocId::MAIN {
        return FrameOrigin::default();
    }
    if let Some(root_node) = doc.root_node().and_then(|id| tree.node(id))
        && root_node.raw_attributes().is_some_and(is_root_attr)
        && let Some(origin) = node_box_origin(root_node)
    {
        return origin;
    }
    if let Some(iframe_node) = doc.iframe_node().and_then(|id| tree.node(id))
        && let Some(origin) = node_box_origin(iframe_node)
    {
        return origin;
    }
    FrameOrigin::default()
}

fn node_box_origin(node: &TreeNode) -> Option<FrameOrigin> {
    let geo = node.raw_attributes()?.geometry.as_ref()?;
    let rect = geo.outer_bounding_box.or(geo.visible_bounding_box)?;
    Some(FrameOrigin {
        x: rect.x.unwrap_or(0),
        y: rect.y.unwrap_or(0),
    })
}

fn compute_node_box(
    tree: &SnapshotTree,
    node_id: NodeId,
    frame_origin: FrameOrigin,
    scale: f64,
) -> Option<NodeBox> {
    let geo = tree.node(node_id)?.raw_attributes()?.geometry.as_ref()?;
    let rect: BoundingRect = geo.outer_bounding_box.or(geo.visible_bounding_box)?;
    Some(NodeBox {
        x: scaled_round(
            f64::from(rect.x.unwrap_or(0)) - f64::from(frame_origin.x),
            scale,
        ),
        y: scaled_round(
            f64::from(rect.y.unwrap_or(0)) - f64::from(frame_origin.y),
            scale,
        ),
        width: scaled_round(f64::from(rect.width.unwrap_or(0)), scale),
        height: scaled_round(f64::from(rect.height.unwrap_or(0)), scale),
    })
}

fn scaled_round(value: f64, scale: f64) -> Number {
    let rounded = (value / scale)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX));
    #[expect(
        clippy::cast_possible_truncation,
        reason = "value is rounded and clamped to i32::MIN..=i32::MAX before converting to i64"
    )]
    let int_val = rounded as i64;
    Number::from(int_val)
}

fn depth_error(levels: usize) -> String {
    SNAPSHOT_DEPTH_ERROR.replace("{levels}", &levels.to_string())
}

fn truncate_data_url(href: &str) -> String {
    if href.starts_with("data:")
        && let Some((prefix, _)) = href.split_once(',')
    {
        return format!("{prefix},\u{2026}");
    }
    href.to_owned()
}

fn is_root_attr(raw: &ContentAttributes) -> bool {
    raw.attribute_type == Some(i32::from(ContentAttributeType::Root))
}

fn is_unroled_html(raw: &ContentAttributes) -> bool {
    hidden::raw_tag(raw).is_some_and(|tag| tag.eq_ignore_ascii_case("html"))
        && raw
            .browseros
            .as_deref()
            .and_then(|browseros| browseros.role.as_deref())
            .is_none()
}

fn is_ignored_tag(tag: &str) -> bool {
    tag.eq_ignore_ascii_case("style")
        || tag.eq_ignore_ascii_case("script")
        || tag.eq_ignore_ascii_case("noscript")
        || tag.eq_ignore_ascii_case("template")
}

fn is_iframe_element(node: &TreeNode, raw: &ContentAttributes) -> bool {
    node.is_iframe_boundary()
        || hidden::raw_tag(raw).is_some_and(|tag| {
            tag.eq_ignore_ascii_case("iframe") || tag.eq_ignore_ascii_case("frame")
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

fn extract_text_content(raw: &ContentAttributes) -> Option<&str> {
    if let Some(dom_text) = raw
        .browseros
        .as_deref()
        .and_then(|browseros| browseros.dom_text.as_deref())
    {
        return Some(dom_text);
    }
    match raw.content_data.as_ref() {
        Some(ContentData::Text(info)) => info.text_content.as_deref(),
        _ => None,
    }
}

const fn tristate(state: TriState) -> Tristate {
    match state {
        TriState::False => Tristate::False,
        TriState::True => Tristate::True,
        TriState::Mixed => Tristate::Mixed,
    }
}

const fn invalid(state: InvalidState) -> Invalid {
    match state {
        InvalidState::False => Invalid::False,
        InvalidState::True => Invalid::True,
        InvalidState::Grammar => Invalid::Grammar,
        InvalidState::Spelling => Invalid::Spelling,
    }
}

fn text_aria_node(text: String) -> AriaNode {
    AriaNode {
        role: "text".to_owned(),
        name: None,
        checked: None,
        disabled: None,
        expanded: None,
        active: None,
        inert: None,
        on_top: None,
        invalid: None,
        level: None,
        pressed: None,
        selected: None,
        aria_hidden: None,
        r#ref: None,
        cursor: None,
        r#box: None,
        url: None,
        placeholder: None,
        text: Some(text),
        children: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::thread::Builder as ThreadBuilder;

    use neo_snapshot::{AriaChild, AriaNode, Call, Invalid, NodeBox, Passed, Tristate};
    use serde_json::Number;

    use crate::proto::{
        AnnotatedPageContent, AnnotatedPageContentMode, AnnotatedPageContentVersion, BoundingRect,
        BrowserOsContentAttributes, BrowserOsPageContentData, BrowserOsRef, BrowserOsTriState,
        ClickabilityReason, ContentAttributeType, ContentAttributes, ContentNode,
        DocumentIdentifier, FormControlType, FrameData, Geometry, RedactionDecision, SelectOption,
    };
    use crate::refs::{RefLookup, RefTarget, TabRefTable};
    use crate::tree::{
        ApcBuilder, BuildOptions, DocToken, DomNodeId, FrameBuilder, MIN_CONTRACT_VERSION,
        NodeBuilder, NodeKey, PartialIframeSpec, SnapshotTree, TreeNode,
    };

    use super::{
        AriaAsk, AriaMode, AriaPartialFrame, AriaRoot, MAX_JSON_DEPTH, MAX_TREE_DEPTH, depth_error,
    };

    fn find_by_role_name<'a>(
        nodes: &'a [AriaNode],
        role: &str,
        name: Option<&str>,
    ) -> Option<&'a AriaNode> {
        let mut stack: Vec<&'a AriaNode> = nodes.iter().rev().collect();
        while let Some(node) = stack.pop() {
            if node.role == role && node.name.as_deref() == name {
                return Some(node);
            }
            for child in node.children.iter().rev() {
                if let AriaChild::Node(child_node) = child {
                    stack.push(child_node);
                }
            }
        }
        None
    }

    fn all_by_role<'a>(nodes: &'a [AriaNode], role: &str) -> Vec<&'a AriaNode> {
        let mut found = Vec::new();
        let mut stack: Vec<&'a AriaNode> = nodes.iter().rev().collect();
        while let Some(node) = stack.pop() {
            if node.role == role {
                found.push(node);
            }
            for child in node.children.iter().rev() {
                if let AriaChild::Node(child_node) = child {
                    stack.push(child_node);
                }
            }
        }
        found
    }

    fn find_by_role_text<'a>(
        nodes: &'a [AriaNode],
        role: &str,
        text: &str,
    ) -> Option<&'a AriaNode> {
        let mut stack: Vec<&'a AriaNode> = nodes.iter().rev().collect();
        while let Some(node) = stack.pop() {
            if node.role == role && node.text.as_deref() == Some(text) {
                return Some(node);
            }
            for child in node.children.iter().rev() {
                if let AriaChild::Node(child_node) = child {
                    stack.push(child_node);
                }
            }
        }
        None
    }

    fn all_nodes(nodes: &[AriaNode]) -> Vec<&AriaNode> {
        let mut out = Vec::new();
        let mut stack: Vec<&AriaNode> = nodes.iter().rev().collect();
        while let Some(node) = stack.pop() {
            out.push(node);
            for child in node.children.iter().rev() {
                if let AriaChild::Node(child_node) = child {
                    stack.push(child_node);
                }
            }
        }
        out
    }

    fn build_rich_form_nodes() -> Vec<NodeBuilder> {
        let select = NodeBuilder::form_control(FormControlType::SelectOne)
            .tag("select")
            .aria_label("Country")
            .bounding_box(0, 120, 140, 24)
            .select_option(SelectOption {
                value: Some("us".to_owned()),
                text: Some("United States".to_owned()),
                is_selected: Some(true),
                is_disabled: Some(false),
            })
            .select_option(SelectOption {
                value: Some("ca".to_owned()),
                text: Some("Canada".to_owned()),
                is_selected: Some(false),
                is_disabled: Some(true),
            });

        let clickable_outer = NodeBuilder::container()
            .bounding_box(0, 160, 200, 40)
            .clickability_reason(ClickabilityReason::CursorPointer)
            .child(
                NodeBuilder::form_control(FormControlType::ButtonButton)
                    .tag("button")
                    .bounding_box(10, 165, 80, 24)
                    .clickability_reason(ClickabilityReason::CursorPointer)
                    .child(NodeBuilder::text("Inner")),
            )
            .child(NodeBuilder::text("Card"));

        vec![
            NodeBuilder::form_control(FormControlType::InputText)
                .tag("input")
                .aria_label("Email")
                .placeholder("you@example.com")
                .field_value("ada@example.com")
                .aria_invalid("spelling")
                .bounding_box(0, 80, 200, 24),
            NodeBuilder::form_control(FormControlType::InputPassword)
                .tag("input")
                .aria_label("Secret")
                .field_value("s3cr3t")
                .redaction(RedactionDecision::RedactedHasBeenPassword)
                .redacted_value_length(6)
                .bounding_box(0, 100, 200, 24),
            NodeBuilder::element("a")
                .href("data:text/plain;base64,SGVsbG8=")
                .bounding_box(0, 110, 80, 20)
                .child(NodeBuilder::text("Download")),
            NodeBuilder::element("span")
                .aria_hidden(true)
                .bounding_box(100, 110, 40, 20)
                .child(NodeBuilder::text("Badge")),
            NodeBuilder::element("img").bounding_box(150, 110, 16, 16),
            select,
            clickable_outer,
        ]
    }

    fn build_rich_tree() -> Result<SnapshotTree, Box<dyn Error>> {
        let body = NodeBuilder::element("body")
            .bounding_box(0, 0, 800, 600)
            .child(
                NodeBuilder::element("h2")
                    .bounding_box(0, 0, 200, 30)
                    .child(NodeBuilder::text("Settings")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::ButtonButton)
                    .dom_id(10)
                    .tag("button")
                    .bounding_box(0, 30, 80, 24)
                    .contract_disabled(true)
                    .pressed_state(BrowserOsTriState::True)
                    .expanded_state(BrowserOsTriState::True)
                    .child(NodeBuilder::text("Toggle")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::InputCheckbox)
                    .tag("input")
                    .aria_label("Partial")
                    .bounding_box(0, 60, 16, 16)
                    .checked_state(BrowserOsTriState::Mixed),
            )
            .children(build_rich_form_nodes());

        let apc = ApcBuilder::new("doc-rich")
            .main_focused_node_id(10)
            .root(NodeBuilder::root().child(NodeBuilder::element("html").child(body)))
            .build();
        Ok(SnapshotTree::build(apc, BuildOptions::new())?)
    }

    /// A `<body>` holding `buttons` buttons, in the document that holds the page's focus with
    /// nothing focused in it, so the body is its active element.
    fn focused_body(buttons: i32) -> Result<SnapshotTree, Box<dyn Error>> {
        let mut body = NodeBuilder::element("body")
            .dom_id(3)
            .bounding_box(0, 0, 400, 200);
        for n in 0..buttons {
            body = body.child(
                NodeBuilder::form_control(FormControlType::ButtonButton)
                    .dom_id(10 + n)
                    .tag("button")
                    .bounding_box(0, 30 * n, 80, 24)
                    .child(NodeBuilder::text("Go")),
            );
        }
        let apc = ApcBuilder::new("doc-focus")
            .focused_frame("doc-focus")
            .root(NodeBuilder::root().child(NodeBuilder::element("html").child(body)))
            .build();
        Ok(SnapshotTree::build(apc, BuildOptions::new())?)
    }

    #[test]
    fn tests_that_an_active_body_holding_one_control_unwraps_as_upstream()
    -> Result<(), Box<dyn Error>> {
        let tree = focused_body(1)?;
        let mut refs = TabRefTable::new();
        let ai = tree.aria(AriaAsk::ai(&mut refs))?;
        let [button] = ai.nodes() else {
            return Err("the button alone at the top".into());
        };
        assert_eq!(button.role, "button", "the body's generic unwraps");
        assert_eq!(button.active, None, "and its [active] goes with it");
        Ok(())
    }

    #[test]
    fn tests_that_an_active_body_holding_two_controls_prints_active() -> Result<(), Box<dyn Error>>
    {
        let tree = focused_body(2)?;
        let mut refs = TabRefTable::new();
        let ai = tree.aria(AriaAsk::ai(&mut refs))?;
        let [body] = ai.nodes() else {
            return Err("the body's generic alone at the top".into());
        };
        assert_eq!(body.role, "generic");
        assert_eq!(body.active, Some(true));
        assert_eq!(body.children.len(), 2);
        Ok(())
    }

    #[test]
    fn ai_mode_emits_states_refs_cursor_and_synthesized_options() -> Result<(), Box<dyn Error>> {
        let tree = build_rich_tree()?;
        let mut refs = TabRefTable::new();
        let ai = tree.aria(AriaAsk::ai(&mut refs))?;

        assert!(
            find_by_role_name(ai.nodes(), "document", None).is_none(),
            "unroled <html> must unwrap without emitting a document wrapper"
        );
        let heading =
            find_by_role_name(ai.nodes(), "heading", Some("Settings")).ok_or("missing heading")?;
        assert_eq!(heading.level, Some(Number::from(2)));
        assert!(heading.r#ref.is_some());

        let toggle =
            find_by_role_name(ai.nodes(), "button", Some("Toggle")).ok_or("missing toggle")?;
        assert_eq!(toggle.active, Some(true));
        assert_eq!(toggle.disabled, Some(true));
        assert_eq!(toggle.pressed, Some(Tristate::True));
        assert_eq!(toggle.expanded, Some(true));

        let checkbox =
            find_by_role_name(ai.nodes(), "checkbox", Some("Partial")).ok_or("missing checkbox")?;
        assert_eq!(checkbox.checked, Some(Tristate::Mixed));

        let email =
            find_by_role_name(ai.nodes(), "textbox", Some("Email")).ok_or("missing email")?;
        assert_eq!(email.invalid, Some(Invalid::Spelling));
        assert_eq!(email.placeholder.as_deref(), Some("you@example.com"));
        assert_eq!(email.text.as_deref(), Some("ada@example.com"));

        let secret =
            find_by_role_name(ai.nodes(), "textbox", Some("Secret")).ok_or("missing secret")?;
        assert_eq!(secret.text.as_deref(), Some("••••••"));

        let link = find_by_role_name(ai.nodes(), "link", Some("Download")).ok_or("missing link")?;
        assert_eq!(link.url.as_deref(), Some("data:text/plain;base64,\u{2026}"));

        let hidden_badge =
            find_by_role_text(ai.nodes(), "generic", "Badge").ok_or("missing aria-hidden badge")?;
        assert_eq!(hidden_badge.aria_hidden, Some(true));
        assert!(find_by_role_name(ai.nodes(), "img", None).is_none());

        let select =
            find_by_role_name(ai.nodes(), "combobox", Some("Country")).ok_or("missing select")?;
        assert_eq!(select.children.len(), 2);
        let us = find_by_role_name(ai.nodes(), "option", Some("United States"))
            .ok_or("missing us option")?;
        assert_eq!(us.selected, Some(true));
        assert_eq!(us.r#ref, None);
        let ca =
            find_by_role_name(ai.nodes(), "option", Some("Canada")).ok_or("missing ca option")?;
        assert_eq!(ca.disabled, Some(true));

        let inner_btn =
            find_by_role_name(ai.nodes(), "button", Some("Inner")).ok_or("missing inner btn")?;
        assert_eq!(inner_btn.cursor, None);
        let pointer_count = all_nodes(ai.nodes())
            .into_iter()
            .filter(|node| node.cursor.as_deref() == Some("pointer"))
            .count();
        assert_eq!(pointer_count, 1);

        let ai_no_active = tree.aria(AriaAsk::ai(&mut refs).with_render_active(false))?;
        let toggle_no_active = find_by_role_name(ai_no_active.nodes(), "button", Some("Toggle"))
            .ok_or("missing toggle without active")?;
        assert_eq!(toggle_no_active.active, None);
        assert_eq!(toggle_no_active.r#ref, toggle.r#ref);
        Ok(())
    }

    /// Upstream's default `aria` visibility stops at an element hidden for aria, so a
    /// `visibility: visible` child of a `visibility: hidden` parent never prints; ai mode shows it.
    /// The cut needs the capture to carry the hidden parent, which v7 keeps for some markup only.
    /// A custom element's shadow card: the host prints as a generic holding one `tabindex` card,
    /// a click target whose only content is text, which stays on its own line with its ref.
    #[test]
    fn tests_that_a_click_target_is_not_inlined_into_its_parent_generic()
    -> Result<(), Box<dyn Error>> {
        let body = NodeBuilder::element("body")
            .dom_id(3)
            .bounding_box(0, 0, 400, 200)
            .child(
                NodeBuilder::element("text-card")
                    .dom_id(10)
                    .bounding_box(0, 0, 400, 20)
                    .child(
                        NodeBuilder::container()
                            .dom_id(11)
                            .clickability_reason(ClickabilityReason::TabIndex)
                            .focusable(true)
                            .tabbable(true)
                            .bounding_box(0, 0, 400, 20)
                            .child(NodeBuilder::text("Card text")),
                    ),
            )
            .child(
                NodeBuilder::form_control(FormControlType::ButtonButton)
                    .dom_id(20)
                    .tag("button")
                    .bounding_box(0, 40, 80, 24)
                    .child(NodeBuilder::text("Other")),
            );
        let apc = ApcBuilder::new("doc-card")
            .root(NodeBuilder::root().child(NodeBuilder::element("html").child(body)))
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::new())?;
        let mut refs = TabRefTable::new();
        let ai = tree.aria(AriaAsk::ai(&mut refs))?;
        let card = all_nodes(ai.nodes())
            .into_iter()
            .find(|node| node.cursor.is_some() && node.text.as_deref() == Some("Card text"))
            .ok_or("the card keeps its own pointer line")?;
        assert!(card.r#ref.is_some(), "and its ref");
        Ok(())
    }

    #[test]
    fn tests_that_default_mode_skips_the_visible_children_of_a_hidden_parent()
    -> Result<(), Box<dyn Error>> {
        let body = NodeBuilder::element("body")
            .dom_id(3)
            .bounding_box(0, 0, 400, 200)
            .child(
                NodeBuilder::container()
                    .dom_id(5)
                    .visibility_hidden(true)
                    .bounding_box(0, 0, 400, 100)
                    .child(
                        NodeBuilder::container()
                            .dom_id(6)
                            .bounding_box(0, 0, 400, 100)
                            .child(
                                NodeBuilder::form_control(FormControlType::ButtonButton)
                                    .dom_id(7)
                                    .tag("button")
                                    .bounding_box(0, 0, 80, 24)
                                    .child(NodeBuilder::text("Button")),
                            ),
                    ),
            );
        let apc = ApcBuilder::new("doc-hidden")
            .root(NodeBuilder::root().child(NodeBuilder::element("html").child(body)))
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::new())?;
        let body_id = tree
            .node_by_key(&NodeKey::new(
                DocToken::new("doc-hidden").ok_or("a token")?,
                DomNodeId::new(3),
            ))
            .map(TreeNode::id)
            .ok_or("the body")?;
        for root in [AriaRoot::Page, AriaRoot::Subtree(body_id)] {
            let default = tree.aria(AriaAsk::default_mode().with_root(root))?;
            assert!(
                find_by_role_name(default.nodes(), "button", Some("Button")).is_none(),
                "default mode stops at the hidden parent, as upstream's 'aria' visibility does"
            );
        }
        let mut refs = TabRefTable::new();
        let ai = tree.aria(AriaAsk::ai(&mut refs))?;
        assert!(
            find_by_role_name(ai.nodes(), "button", Some("Button")).is_some(),
            "ai mode shows it"
        );
        Ok(())
    }

    #[test]
    fn default_mode_omits_refs_active_cursor_and_ai_distillation() -> Result<(), Box<dyn Error>> {
        let tree = build_rich_tree()?;
        let default = tree.aria(AriaAsk::default_mode())?;
        for node in all_nodes(default.nodes()) {
            assert_eq!(node.r#ref, None);
            assert_eq!(node.active, None);
            assert_eq!(node.cursor, None);
            assert_eq!(node.aria_hidden, None);
        }
        assert!(find_by_role_text(default.nodes(), "generic", "Badge").is_none());
        assert!(find_by_role_name(default.nodes(), "img", None).is_some());
        Ok(())
    }

    /// The seven states a template compares, as one emitted node carries them.
    #[derive(Debug, PartialEq)]
    struct States {
        checked: Option<Tristate>,
        disabled: Option<bool>,
        expanded: Option<bool>,
        invalid: Option<Invalid>,
        level: Option<Number>,
        pressed: Option<Tristate>,
        selected: Option<bool>,
    }

    impl States {
        fn of(node: &AriaNode) -> Self {
            Self {
                checked: node.checked,
                disabled: node.disabled,
                expanded: node.expanded,
                invalid: node.invalid,
                level: node.level.clone(),
                pressed: node.pressed,
                selected: node.selected,
            }
        }
    }

    /// One node of each state shape `toAriaNode` writes: a role owning a state at its default, an
    /// `aria-expanded="false"` beside an unset one, levels 0 and 2, a role owning none (a list:
    /// default mode unwraps an unnamed generic, so none is printed), and a `<select>`'s
    /// synthesized options.
    fn build_state_tree() -> Result<SnapshotTree, Box<dyn Error>> {
        let body = NodeBuilder::element("body")
            .child(
                NodeBuilder::form_control(FormControlType::InputCheckbox)
                    .tag("input")
                    .aria_label("Terms"),
            )
            .child(NodeBuilder::element("button").child(NodeBuilder::text("Save")))
            .child(
                NodeBuilder::element("button")
                    .expanded_state(BrowserOsTriState::False)
                    .child(NodeBuilder::text("Menu")),
            )
            .child(
                NodeBuilder::element("ul")
                    .child(NodeBuilder::element("li").child(NodeBuilder::text("One"))),
            )
            .child(NodeBuilder::element("h2").child(NodeBuilder::text("Settings")))
            .child(
                NodeBuilder::form_control(FormControlType::InputText)
                    .tag("input")
                    .aria_label("Email"),
            )
            .child(
                NodeBuilder::element("div")
                    .role("row")
                    .child(NodeBuilder::text("Row")),
            )
            .child(
                NodeBuilder::element("a")
                    .href("/next")
                    .child(NodeBuilder::text("Next")),
            )
            .child(
                NodeBuilder::form_control(FormControlType::SelectOne)
                    .tag("select")
                    .aria_label("Country")
                    .select_option(SelectOption {
                        value: Some("us".to_owned()),
                        text: Some("United States".to_owned()),
                        is_selected: Some(true),
                        is_disabled: Some(false),
                    })
                    .select_option(SelectOption {
                        value: Some("ca".to_owned()),
                        text: Some("Canada".to_owned()),
                        is_selected: Some(false),
                        is_disabled: Some(true),
                    }),
            );
        let apc = ApcBuilder::new("doc-states")
            .root(NodeBuilder::root().child(NodeBuilder::element("html").child(body)))
            .build();
        Ok(SnapshotTree::build(apc, BuildOptions::new())?)
    }

    #[test]
    fn template_mode_writes_every_state_the_role_owns() -> Result<(), Box<dyn Error>> {
        let tree = build_state_tree()?;
        let template = tree.aria(AriaAsk::template_mode())?;
        let states =
            |checked, disabled, expanded, invalid, level: Option<u32>, pressed, selected| States {
                checked,
                disabled,
                expanded,
                invalid,
                level: level.map(Number::from),
                pressed,
                selected,
            };
        let unchecked = Some(Tristate::False);
        let enabled = Some(false);
        let valid = Some(Invalid::False);
        let rows = [
            (
                "checkbox",
                Some("Terms"),
                states(unchecked, enabled, None, valid, None, None, None),
                "kAriaChecked, Disabled, Expanded and Invalid roles; aria-expanded is unset",
            ),
            (
                "button",
                Some("Save"),
                states(None, enabled, None, None, None, unchecked, None),
                "a plain button: pressed false, expanded unset",
            ),
            (
                "button",
                Some("Menu"),
                states(None, enabled, Some(false), None, None, unchecked, None),
                "aria-expanded=\"false\" is false, which an unset one is not",
            ),
            (
                "listitem",
                None,
                states(None, None, None, None, Some(0), None, None),
                "a listitem with no aria-level is level 0",
            ),
            (
                "heading",
                Some("Settings"),
                states(None, None, None, None, Some(2), None, None),
                "h2's native level",
            ),
            (
                "textbox",
                Some("Email"),
                states(None, enabled, None, valid, None, None, None),
                "a valid textbox is invalid false",
            ),
            (
                "row",
                Some("Row"),
                states(None, enabled, None, None, Some(0), None, Some(false)),
                "an unselected row, level 0",
            ),
            (
                "link",
                Some("Next"),
                states(None, enabled, None, None, None, None, None),
                "a link owns disabled and expanded, and aria-expanded is unset",
            ),
            (
                "list",
                None,
                states(None, None, None, None, None, None, None),
                "a list owns no state, so it writes none",
            ),
            (
                "combobox",
                Some("Country"),
                states(None, enabled, None, valid, None, None, None),
                "a select is a combobox with aria-expanded unset",
            ),
            (
                "option",
                Some("United States"),
                states(unchecked, enabled, None, None, None, None, Some(true)),
                "a synthesized option owns checked, disabled and selected",
            ),
            (
                "option",
                Some("Canada"),
                states(unchecked, Some(true), None, None, None, None, Some(false)),
                "a disabled, unselected synthesized option",
            ),
        ];
        for (role, name, expected, why) in rows {
            let node = find_by_role_name(template.nodes(), role, name)
                .ok_or_else(|| format!("template mode emits {role} {name:?}"))?;
            assert_eq!(States::of(node), expected, "{role} {name:?}: {why}");
        }
        Ok(())
    }

    #[test]
    fn template_mode_prints_default_modes_yaml_and_default_keeps_the_wire_form()
    -> Result<(), Box<dyn Error>> {
        let tree = build_state_tree()?;
        let template = tree.aria(AriaAsk::template_mode())?;
        let default = tree.aria(AriaAsk::default_mode())?;
        assert_eq!(
            neo_snapshot::render(&template.to_snapshot(), Call::Playwright).into_text(),
            neo_snapshot::render(&default.to_snapshot(), Call::Playwright).into_text(),
            "the printer drops false and 0, so the two modes print the same YAML"
        );
        for node in all_nodes(default.nodes()) {
            assert!(
                node.checked != Some(Tristate::False)
                    && node.pressed != Some(Tristate::False)
                    && node.invalid != Some(Invalid::False)
                    && node.disabled != Some(false)
                    && node.expanded != Some(false)
                    && node.selected != Some(false)
                    && node.level != Some(Number::from(0)),
                "default mode writes only what AriaNodeJSON carries: {} {:?}",
                node.role,
                node.name
            );
        }
        Ok(())
    }

    fn build_click_proxy_tree() -> Result<SnapshotTree, Box<dyn Error>> {
        let apc = ApcBuilder::new("doc-proxy")
            .root(
                NodeBuilder::root()
                    .child(
                        NodeBuilder::element("label")
                            .dom_id(10)
                            .label_for(11)
                            .bounding_box(0, 0, 140, 24)
                            .child(NodeBuilder::text("Accept "))
                            .child(
                                NodeBuilder::element("a")
                                    .dom_id(12)
                                    .href("/terms")
                                    .bounding_box(60, 0, 50, 24)
                                    .child(NodeBuilder::text("Terms")),
                            ),
                    )
                    .child(
                        NodeBuilder::form_control(FormControlType::InputCheckbox)
                            .dom_id(11)
                            .tag("input")
                            .bounding_box(0, 0, 0, 0),
                    )
                    .child(
                        NodeBuilder::element("label")
                            .dom_id(20)
                            .label_for(21)
                            .bounding_box(0, 30, 120, 24)
                            .child(
                                NodeBuilder::form_control(FormControlType::InputRadio)
                                    .dom_id(21)
                                    .tag("input")
                                    .bounding_box(0, 30, 0, 0),
                            )
                            .child(NodeBuilder::container().child(NodeBuilder::text("Choice A"))),
                    ),
            )
            .build();
        Ok(SnapshotTree::build(apc, BuildOptions::new())?)
    }

    #[test]
    fn click_proxy_label_folding_ai_vs_default() -> Result<(), Box<dyn Error>> {
        let tree = build_click_proxy_tree()?;

        let mut refs = TabRefTable::new();
        let ai = tree.aria(AriaAsk::ai(&mut refs))?;

        let terms = find_by_role_name(ai.nodes(), "link", Some("Terms"))
            .ok_or("interactive link inside folded label must stay visible")?;
        assert_eq!(terms.url.as_deref(), Some("/terms"));

        let checkbox = find_by_role_name(ai.nodes(), "checkbox", Some("Accept Terms"))
            .ok_or("zero-size checkbox must be emitted")?;
        let cb_ref = checkbox
            .r#ref
            .as_deref()
            .ok_or("checkbox must receive a ref")?;

        let radio = find_by_role_name(ai.nodes(), "radio", Some("Choice A"))
            .ok_or("wrapped zero-size radio must be emitted")?;
        let radio_ref = radio.r#ref.as_deref().ok_or("radio must receive a ref")?;

        let doc_token = DocToken::new("doc-proxy").ok_or("valid token")?;
        assert_eq!(
            refs.resolve_str(cb_ref),
            RefLookup::Key(RefTarget {
                key: NodeKey::new(doc_token.clone(), DomNodeId::new(11)),
                click_proxy: Some(NodeKey::new(doc_token.clone(), DomNodeId::new(10))),
            })
        );
        assert_eq!(
            refs.resolve_str(radio_ref),
            RefLookup::Key(RefTarget {
                key: NodeKey::new(doc_token.clone(), DomNodeId::new(21)),
                click_proxy: Some(NodeKey::new(doc_token, DomNodeId::new(20))),
            })
        );

        let stray_text = all_nodes(ai.nodes()).into_iter().any(|node| {
            node.text
                .as_deref()
                .is_some_and(|t| t.contains("Accept") || t.contains("Choice A"))
        });
        assert!(
            !stray_text,
            "folded label text must not duplicate outside controls"
        );

        let default = tree.aria(AriaAsk::default_mode())?;
        let has_accept_text = all_nodes(default.nodes())
            .into_iter()
            .any(|node| node.role == "text" && node.text.as_deref() == Some("Accept"));
        assert!(
            has_accept_text,
            "default mode must not fold ClickProxy labels"
        );
        for node in all_nodes(default.nodes()) {
            assert_eq!(node.r#ref, None);
        }
        Ok(())
    }

    #[test]
    fn d20_prohibited_names_drops_or_promotes_to_text() -> Result<(), Box<dyn Error>> {
        let apc = ApcBuilder::new("doc-d20")
            .root(
                NodeBuilder::root()
                    .child(
                        NodeBuilder::container()
                            .aria_label("Chrome 1")
                            .bounding_box(0, 0, 100, 20)
                            .child(NodeBuilder::text("Chrome 1")),
                    )
                    .child(
                        NodeBuilder::container()
                            .aria_label("Status")
                            .bounding_box(0, 30, 120, 40)
                            .child(
                                NodeBuilder::form_control(FormControlType::ButtonButton)
                                    .tag("button")
                                    .bounding_box(5, 35, 60, 24)
                                    .child(NodeBuilder::text("Save")),
                            ),
                    )
                    .child(
                        NodeBuilder::element("code")
                            .aria_label("Run")
                            .bounding_box(0, 80, 100, 20)
                            .child(NodeBuilder::text("npm test")),
                    ),
            )
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::new())?;

        let mut refs = TabRefTable::new();
        let first = tree.aria(AriaAsk::ai(&mut refs))?;

        let chrome = find_by_role_text(first.nodes(), "generic", "Chrome 1")
            .ok_or("generic whose sole content matched aria-label must promote name to text")?;
        assert_eq!(chrome.name, None);
        let chrome_ref = chrome.r#ref.clone().ok_or("chrome generic must keep ref")?;

        let status = first.nodes().get(1).ok_or("missing status generic")?;
        assert_eq!(status.role, "generic");
        assert_eq!(status.name, None);
        assert_eq!(status.text, None);
        assert_eq!(status.children.len(), 1);
        assert!(find_by_role_name(first.nodes(), "button", Some("Save")).is_some());

        let code = find_by_role_text(first.nodes(), "code", "npm test").ok_or("missing code")?;
        assert_eq!(code.name, None);

        let second = tree.aria(AriaAsk::ai(&mut refs))?;
        let chrome_again =
            find_by_role_text(second.nodes(), "generic", "Chrome 1").ok_or("missing chrome 2")?;
        assert_eq!(
            chrome_again.r#ref.as_deref(),
            Some(chrome_ref.as_str()),
            "ref assignment before D20 must preserve ref stability across snapshots"
        );
        Ok(())
    }

    #[test]
    fn a_subtree_capture_folds_a_label_only_with_its_control() -> Result<(), Box<dyn Error>> {
        let tree = build_click_proxy_tree()?;
        let doc_token = DocToken::new("doc-proxy").ok_or("valid token")?;
        let node_of = |dom_id: i32| {
            tree.node_by_key(&NodeKey::new(doc_token.clone(), DomNodeId::new(dom_id)))
                .map(TreeNode::id)
                .ok_or("node in the tree")
        };

        let mut refs = TabRefTable::new();
        let label_alone =
            tree.aria(AriaAsk::ai(&mut refs).with_root(AriaRoot::Subtree(node_of(10)?)))?;
        let label = label_alone
            .nodes()
            .first()
            .ok_or("the label prints on its own line")?;
        let label_ref = label.r#ref.as_deref().ok_or("the label gets a ref")?;
        assert_eq!(
            refs.resolve_str(label_ref),
            RefLookup::Key(RefTarget {
                key: NodeKey::new(doc_token.clone(), DomNodeId::new(10)),
                click_proxy: None,
            })
        );
        assert!(
            serde_json::to_string(label_alone.nodes())?.contains("Accept"),
            "its checkbox lies outside the subtree, so the label keeps its text"
        );

        let wrapping =
            tree.aria(AriaAsk::ai(&mut refs).with_root(AriaRoot::Subtree(node_of(20)?)))?;
        assert!(
            find_by_role_name(wrapping.nodes(), "radio", Some("Choice A"))
                .is_some_and(|radio| radio.r#ref.is_some()),
            "the label holds its radio, so it still folds into the radio's line"
        );
        Ok(())
    }

    fn labelled_by(dom_id: i32) -> Vec<BrowserOsRef> {
        vec![BrowserOsRef {
            dom_node_id: Some(dom_id),
            unrendered_text: None,
        }]
    }

    #[test]
    fn ai_mode_drops_names_that_printed_contributors_repeat() -> Result<(), Box<dyn Error>> {
        let apc = ApcBuilder::new("doc-redundant")
            .root(
                NodeBuilder::root().child(
                    NodeBuilder::element("dialog")
                        .dom_id(10)
                        .labelledby(labelled_by(11))
                        .bounding_box(0, 0, 300, 200)
                        .child(
                            NodeBuilder::element("h2")
                                .dom_id(11)
                                .bounding_box(0, 0, 300, 30)
                                .child(NodeBuilder::text("Confirm delete")),
                        )
                        .child(
                            NodeBuilder::element("a")
                                .dom_id(12)
                                .href("/docs")
                                .bounding_box(0, 40, 300, 30)
                                .child(
                                    NodeBuilder::element("h3")
                                        .dom_id(13)
                                        .bounding_box(0, 40, 300, 30)
                                        .child(NodeBuilder::text("Read the docs")),
                                ),
                        ),
                ),
            )
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::new())?;

        let mut refs = TabRefTable::new();
        let ai = tree.aria(AriaAsk::ai(&mut refs))?;
        let dialog = find_by_role_name(ai.nodes(), "dialog", None)
            .ok_or("ai mode prints the dialog without the name its heading repeats")?;
        assert!(dialog.r#ref.is_some(), "the nameless dialog keeps its ref");
        assert!(find_by_role_name(ai.nodes(), "heading", Some("Confirm delete")).is_some());
        let link = find_by_role_name(ai.nodes(), "link", None)
            .ok_or("ai mode prints the link without the name its heading repeats")?;
        assert_eq!(link.url.as_deref(), Some("/docs"));
        assert!(find_by_role_name(ai.nodes(), "heading", Some("Read the docs")).is_some());

        let default = tree.aria(AriaAsk::default_mode())?;
        assert!(
            find_by_role_name(default.nodes(), "dialog", Some("Confirm delete")).is_some(),
            "default mode does not run removeRedundantNames"
        );
        assert!(find_by_role_name(default.nodes(), "link", Some("Read the docs")).is_some());
        Ok(())
    }

    #[test]
    fn a_link_keeps_the_name_its_svg_icon_title_gives_it() -> Result<(), Box<dyn Error>> {
        // Upstream's SVG step returns the unrendered `<title>`'s text, so no printed element
        // contributed the link's name, and removeRedundantNames keeps it.
        let apc = ApcBuilder::new("doc-svg-link")
            .root(
                NodeBuilder::root().child(
                    NodeBuilder::element("a")
                        .dom_id(10)
                        .href("/settings")
                        .bounding_box(0, 0, 24, 24)
                        .child(
                            NodeBuilder::element("svg")
                                .dom_id(11)
                                .svg_title("Settings")
                                .bounding_box(0, 0, 24, 24),
                        ),
                ),
            )
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::new())?;

        let mut refs = TabRefTable::new();
        let ai = tree.aria(AriaAsk::ai(&mut refs))?;
        let link = find_by_role_name(ai.nodes(), "link", Some("Settings"))
            .ok_or("the link keeps the name its icon's title gives it")?;
        assert_eq!(link.url.as_deref(), Some("/settings"));
        assert!(find_by_role_name(ai.nodes(), "img", Some("Settings")).is_some());
        Ok(())
    }

    #[test]
    fn ai_mode_keeps_a_name_whose_contributor_comes_later() -> Result<(), Box<dyn Error>> {
        let apc = ApcBuilder::new("doc-later")
            .root(
                NodeBuilder::root()
                    .child(
                        NodeBuilder::form_control(FormControlType::ButtonButton)
                            .dom_id(10)
                            .tag("button")
                            .bounding_box(0, 0, 120, 30)
                            .child(
                                NodeBuilder::element("span")
                                    .dom_id(11)
                                    .role("img")
                                    .labelledby(labelled_by(12))
                                    .bounding_box(0, 0, 20, 20),
                            ),
                    )
                    .child(
                        NodeBuilder::element("h2")
                            .dom_id(12)
                            .bounding_box(0, 40, 120, 30)
                            .child(NodeBuilder::text("Save draft")),
                    ),
            )
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::new())?;

        let mut refs = TabRefTable::new();
        let ai = tree.aria(AriaAsk::ai(&mut refs))?;
        let button = find_by_role_name(ai.nodes(), "button", Some("Save draft")).ok_or(
            "the button keeps its name: the later element had no ref when the button's subtree \
             ended, so upstream never counts it",
        )?;
        assert!(button.r#ref.is_some());
        assert!(find_by_role_name(ai.nodes(), "img", Some("Save draft")).is_some());
        assert!(find_by_role_name(ai.nodes(), "heading", Some("Save draft")).is_some());
        Ok(())
    }

    #[test]
    fn inert_marking_protects_live_modal_dialog() -> Result<(), Box<dyn Error>> {
        let apc = ApcBuilder::new("doc-inert")
            .root(
                NodeBuilder::root()
                    .child(
                        NodeBuilder::container()
                            .inert(true)
                            .bounding_box(0, 0, 400, 200)
                            .child(
                                NodeBuilder::element("dialog")
                                    .inert(false)
                                    .bounding_box(50, 50, 200, 100)
                                    .child(NodeBuilder::text("Live")),
                            )
                            .child(
                                NodeBuilder::form_control(FormControlType::ButtonButton)
                                    .tag("button")
                                    .inert(true)
                                    .bounding_box(10, 10, 80, 24)
                                    .child(NodeBuilder::text("Inert")),
                            ),
                    )
                    .child(
                        NodeBuilder::container()
                            .inert(true)
                            .bounding_box(0, 210, 400, 60)
                            .child(
                                NodeBuilder::form_control(FormControlType::ButtonButton)
                                    .tag("button")
                                    .inert(true)
                                    .bounding_box(10, 220, 60, 24)
                                    .child(NodeBuilder::text("One")),
                            )
                            .child(
                                NodeBuilder::form_control(FormControlType::ButtonButton)
                                    .tag("button")
                                    .inert(true)
                                    .bounding_box(80, 220, 60, 24)
                                    .child(NodeBuilder::text("Two")),
                            ),
                    ),
            )
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::new())?;

        let mut refs = TabRefTable::new();
        let ai = tree.aria(AriaAsk::ai(&mut refs))?;

        let mixed_wrapper = ai.nodes().first().ok_or("missing mixed wrapper")?;
        assert_eq!(
            mixed_wrapper.inert, None,
            "wrapper containing live dialog must stay un-marked"
        );
        let dialog = find_by_role_text(ai.nodes(), "dialog", "Live").ok_or("missing dialog")?;
        assert_eq!(dialog.inert, None);
        let inert_btn =
            find_by_role_name(ai.nodes(), "button", Some("Inert")).ok_or("missing inert button")?;
        assert_eq!(inert_btn.inert, Some(true));

        let pure_inert_wrapper = ai.nodes().get(1).ok_or("missing pure inert wrapper")?;
        assert_eq!(pure_inert_wrapper.inert, Some(true));
        let btn_one = find_by_role_name(ai.nodes(), "button", Some("One")).ok_or("missing One")?;
        assert_eq!(
            btn_one.inert, None,
            "inert mark must coalesce onto the topmost inert container"
        );

        let default = tree.aria(AriaAsk::default_mode())?;
        for node in all_nodes(default.nodes()) {
            assert_eq!(node.inert, None);
        }
        Ok(())
    }

    #[test]
    fn boxes_scales_by_dpr_and_subtracts_iframe_origin() -> Result<(), Box<dyn Error>> {
        let child_frame = FrameBuilder::new("doc-child")
            .url("https://child.example/frame")
            .root(
                NodeBuilder::root().bounding_box(200, 100, 600, 400).child(
                    NodeBuilder::form_control(FormControlType::ButtonButton)
                        .tag("button")
                        .bounding_box(230, 150, 120, 40)
                        .child(NodeBuilder::text("Inside")),
                ),
            );

        let apc = ApcBuilder::new("doc-dpr")
            .viewport(0, 0, 1000, 500)
            .root(
                NodeBuilder::root()
                    .bounding_box(0, 0, 2000, 1000)
                    .child(
                        NodeBuilder::form_control(FormControlType::ButtonButton)
                            .tag("button")
                            .bounding_box(40, 80, 200, 60)
                            .child(NodeBuilder::text("Main")),
                    )
                    .child(NodeBuilder::iframe(child_frame).bounding_box(200, 100, 600, 400)),
            )
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::new())?;

        let mut refs = TabRefTable::new();
        let out = tree.aria(AriaAsk::ai(&mut refs).with_boxes(true))?;

        let main_btn =
            find_by_role_name(out.nodes(), "button", Some("Main")).ok_or("missing main btn")?;
        assert_eq!(
            main_btn.r#box,
            Some(NodeBox {
                x: Number::from(20),
                y: Number::from(40),
                width: Number::from(100),
                height: Number::from(30),
            })
        );

        let child_btn =
            find_by_role_name(out.nodes(), "button", Some("Inside")).ok_or("missing child btn")?;
        assert_eq!(
            child_btn.r#box,
            Some(NodeBox {
                x: Number::from(15),
                y: Number::from(25),
                width: Number::from(60),
                height: Number::from(20),
            })
        );

        let no_boxes = tree.aria(AriaAsk::ai(&mut refs))?;
        for node in all_nodes(no_boxes.nodes()) {
            assert_eq!(node.r#box, None);
        }
        Ok(())
    }

    fn build_nested_group_chain(depth: usize, leaf: NodeBuilder) -> NodeBuilder {
        let mut current = leaf;
        for _ in 0..depth {
            current = NodeBuilder::container()
                .role("group")
                .bounding_box(0, 0, 100, 100)
                .child(current);
        }
        current
    }

    #[test]
    fn multiframe_stitching_and_partial_frames_in_ai_and_default() -> Result<(), Box<dyn Error>> {
        let child_frame = FrameBuilder::new("doc-child")
            .url("https://child.example/view")
            .root(
                NodeBuilder::root().child(
                    NodeBuilder::form_control(FormControlType::ButtonButton)
                        .tag("button")
                        .bounding_box(0, 0, 80, 24)
                        .child(NodeBuilder::text("Child Go")),
                ),
            );

        let apc = ApcBuilder::new("doc-parent")
            .root(
                NodeBuilder::root()
                    .child(NodeBuilder::iframe(child_frame).bounding_box(0, 0, 300, 200))
                    .child(
                        NodeBuilder::partial_iframe(PartialIframeSpec::Timeout)
                            .redacted_frame_token("frame-tok-2")
                            .href("https://slow.example/widget")
                            .bounding_box(0, 210, 300, 100),
                    ),
            )
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::new())?;

        let mut refs = TabRefTable::new();
        let ai = tree.aria(AriaAsk::ai(&mut refs))?;
        let child_btn = find_by_role_name(ai.nodes(), "button", Some("Child Go"))
            .ok_or("child iframe button must stitch in ai mode")?;
        assert!(
            child_btn
                .r#ref
                .as_deref()
                .is_some_and(|r| r.starts_with("f1e")),
            "child document must use f1e* refs"
        );
        assert_eq!(ai.partial().len(), 1);
        let partial = ai.partial().first().ok_or("missing partial frame")?;
        assert_eq!(partial.frame, "https://slow.example/widget");
        assert_eq!(partial.reason, "timeout after 2000 ms");
        assert!(!partial.r#ref.is_empty());

        // Default mode reads one document, as upstream's does: each iframe is a leaf, and a
        // frame it never reads cannot be partial.
        let default = tree.aria(AriaAsk::default_mode())?;
        assert!(
            find_by_role_name(default.nodes(), "button", Some("Child Go")).is_none(),
            "a child frame's button is not read in default mode"
        );
        let iframes = all_by_role(default.nodes(), "iframe");
        assert!(!iframes.is_empty(), "the iframes still print");
        assert!(iframes.iter().all(|iframe| iframe.children.is_empty()));
        assert!(default.partial().is_empty());

        let zero = tree.aria(AriaAsk::default_mode().with_depth(Some(0)))?;
        assert_eq!(
            zero,
            tree.aria(AriaAsk::default_mode())?,
            "depth 0 is upstream's falsy zero: no cut"
        );
        Ok(())
    }

    #[test]
    fn depth_cap_refuses_overdeep_child_frame_and_keeps_parent() -> Result<(), Box<dyn Error>> {
        let deep_child = FrameBuilder::new("doc-deep-child")
            .url("https://deep.example/inner")
            .root(
                NodeBuilder::root().child(build_nested_group_chain(
                    80,
                    NodeBuilder::form_control(FormControlType::ButtonButton)
                        .tag("button")
                        .bounding_box(0, 0, 40, 20)
                        .child(NodeBuilder::text("Deep")),
                )),
            );
        let deep_parent = build_nested_group_chain(
            80,
            NodeBuilder::iframe(deep_child).bounding_box(0, 0, 100, 100),
        );
        let deep_apc = ApcBuilder::new("doc-deep-parent")
            .root(NodeBuilder::root().child(deep_parent))
            .build();
        let deep_tree = SnapshotTree::build(deep_apc, BuildOptions::new())?;

        let mut deep_refs = TabRefTable::new();
        let deep_out = deep_tree.aria(AriaAsk::ai(&mut deep_refs))?;
        assert!(neo_snapshot::json_depth(deep_out.nodes()) < MAX_JSON_DEPTH);
        assert!(find_by_role_name(deep_out.nodes(), "button", Some("Deep")).is_none());
        let iframe_node = find_by_role_name(deep_out.nodes(), "iframe", None)
            .ok_or("parent frame must stay readable with iframe leaf")?;
        assert!(iframe_node.children.is_empty());
        let refusal = deep_out
            .partial()
            .first()
            .ok_or("over-deep child frame must emit a partial note")?;
        assert_eq!(refusal.frame, "https://deep.example/inner");
        let expected_reason = depth_error((163 + 162 - 1) / 2);
        assert_eq!(refusal.reason, expected_reason);
        Ok(())
    }

    /// A document that ends in bare leaves one level past `MAX_TREE_DEPTH` is no deeper than one
    /// the page's engine returned (`neo_snapshot::MAX_TREE_DEPTH`): it is emitted whole, leaves
    /// included, in both modes; one level more is refused.
    #[test]
    fn a_document_ending_in_bare_leaves_one_level_past_the_cap_is_emitted_whole()
    -> Result<(), Box<dyn Error>> {
        let chain = |groups| {
            ApcBuilder::new("doc-deep")
                .root(
                    NodeBuilder::root()
                        .child(build_nested_group_chain(groups, deep_button("Deep target"))),
                )
                .build()
        };
        let fits = SnapshotTree::build(chain(MAX_TREE_DEPTH), BuildOptions::new())?;
        let mut refs = TabRefTable::new();
        let ai = fits.aria(AriaAsk::ai(&mut refs))?;
        assert!(find_by_role_name(ai.nodes(), "button", Some("Deep target")).is_some());
        assert!(neo_snapshot::json_depth(ai.nodes()) < MAX_JSON_DEPTH);
        let default = fits.aria(AriaAsk::default_mode())?;
        assert!(find_by_role_name(default.nodes(), "button", Some("Deep target")).is_some());

        let deeper = SnapshotTree::build(chain(MAX_TREE_DEPTH + 1), BuildOptions::new())?;
        assert!(deeper.aria(AriaAsk::default_mode()).is_err());
        Ok(())
    }

    fn deep_button(text: &str) -> NodeBuilder {
        NodeBuilder::form_control(FormControlType::ButtonButton)
            .tag("button")
            .bounding_box(0, 0, 40, 20)
            .child(NodeBuilder::text(text))
    }

    #[test]
    fn child_frame_depth_follows_upstream_arithmetic() -> Result<(), Box<dyn Error>> {
        let frame_a = FrameBuilder::new("doc-depth-a")
            .url("https://a.example/")
            .root(
                NodeBuilder::root().child(
                    NodeBuilder::element("a")
                        .dom_id(5)
                        .href("/docs")
                        .bounding_box(0, 0, 100, 20)
                        .child(
                            NodeBuilder::element("h3")
                                .dom_id(6)
                                .bounding_box(0, 0, 100, 20)
                                .child(NodeBuilder::text("Read the docs")),
                        ),
                ),
            );
        let frame_b = FrameBuilder::new("doc-depth-b")
            .url("https://b.example/")
            .root(NodeBuilder::root().child(deep_button("B button")));
        let apc = ApcBuilder::new("doc-depth")
            .root(
                NodeBuilder::root()
                    .child(build_nested_group_chain(
                        1,
                        NodeBuilder::iframe(frame_a).bounding_box(0, 0, 100, 100),
                    ))
                    .child(build_nested_group_chain(
                        2,
                        NodeBuilder::iframe(frame_b).bounding_box(0, 100, 100, 100),
                    )),
            )
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::new())?;
        let mut refs = TabRefTable::new();

        // Depth 1: A's iframe prints at the cut, so its frame gets 1 - 1 - 1 = -1 and prints
        // whole, keeping every name. B's iframe lies past the cut, so its frame is never read.
        let cut = tree.aria(AriaAsk::ai(&mut refs).with_depth(Some(1)))?;
        assert!(find_by_role_name(cut.nodes(), "link", Some("Read the docs")).is_some());
        assert!(find_by_role_name(cut.nodes(), "heading", Some("Read the docs")).is_some());
        assert!(find_by_role_name(cut.nodes(), "button", Some("B button")).is_none());
        assert!(cut.partial().is_empty());

        // Depth 3: A's frame gets 3 - 1 - 1 = 1, a cut again, so the link's name repeats its
        // printed heading and goes. B's gets 3 - 2 - 1 = 0, which upstream reads as no cut.
        let deeper = tree.aria(AriaAsk::ai(&mut refs).with_depth(Some(3)))?;
        assert!(find_by_role_name(deeper.nodes(), "link", None).is_some());
        assert!(find_by_role_name(deeper.nodes(), "button", Some("B button")).is_some());
        Ok(())
    }

    #[test]
    fn a_page_deeper_than_the_cap_is_refused_unless_a_depth_cuts_it() -> Result<(), Box<dyn Error>>
    {
        let apc = ApcBuilder::new("doc-too-deep")
            .root(NodeBuilder::root().child(build_nested_group_chain(150, deep_button("Deep"))))
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::new())?;
        let mut refs = TabRefTable::new();

        let refused = tree
            .aria(AriaAsk::ai(&mut refs))
            .err()
            .ok_or("150 groups and a button pass the cap, so the page is refused, not cut")?;
        assert_eq!(refused.levels(), 151);
        assert_eq!(refused.to_string(), depth_error(151));
        assert!(tree.aria(AriaAsk::default_mode()).is_err());

        let asked = tree.aria(AriaAsk::ai(&mut refs).with_depth(Some(140)))?;
        assert!(
            find_by_role_name(asked.nodes(), "button", Some("Deep")).is_none(),
            "a depth the caller asked for cuts the page without a refusal"
        );
        Ok(())
    }

    #[test]
    fn a_child_frame_deeper_than_the_cap_becomes_a_partial_frame() -> Result<(), Box<dyn Error>> {
        let deep_frame = FrameBuilder::new("doc-deep-frame")
            .url("https://deep.example/frame")
            .root(NodeBuilder::root().child(build_nested_group_chain(150, deep_button("Deep"))));
        let apc = ApcBuilder::new("doc-shallow-parent")
            .root(
                NodeBuilder::root()
                    .child(deep_button("Top"))
                    .child(NodeBuilder::iframe(deep_frame).bounding_box(0, 40, 100, 100)),
            )
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::new())?;
        let mut refs = TabRefTable::new();

        let out = tree.aria(AriaAsk::ai(&mut refs))?;
        assert!(find_by_role_name(out.nodes(), "button", Some("Top")).is_some());
        assert!(find_by_role_name(out.nodes(), "button", Some("Deep")).is_none());
        let partial = out
            .partial()
            .first()
            .ok_or("the deep frame says why it is missing")?;
        assert_eq!(partial.frame, "https://deep.example/frame");
        assert_eq!(
            partial.reason,
            depth_error(152),
            "its iframe's level plus the frame's 151"
        );
        Ok(())
    }

    #[test]
    fn subtree_capture_preserves_sibling_ref_identities() -> Result<(), Box<dyn Error>> {
        let apc = ApcBuilder::new("doc-sub")
            .root(
                NodeBuilder::root()
                    .child(
                        NodeBuilder::form_control(FormControlType::ButtonButton)
                            .dom_id(10)
                            .tag("button")
                            .bounding_box(0, 0, 60, 24)
                            .child(NodeBuilder::text("Alpha")),
                    )
                    .child(
                        NodeBuilder::form_control(FormControlType::ButtonButton)
                            .dom_id(20)
                            .tag("button")
                            .bounding_box(70, 0, 60, 24)
                            .child(NodeBuilder::text("Beta")),
                    ),
            )
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::new())?;
        let token = DocToken::new("doc-sub").ok_or("valid doc token")?;
        let alpha_id = tree
            .node_by_key(&NodeKey::new(token, DomNodeId::new(10)))
            .ok_or("missing alpha node")?
            .id();

        let mut refs = TabRefTable::new();
        let full1 = tree.aria(AriaAsk::ai(&mut refs))?;
        let beta_ref_1 = find_by_role_name(full1.nodes(), "button", Some("Beta"))
            .and_then(|n| n.r#ref.clone())
            .ok_or("missing beta ref 1")?;

        let sub_ask = AriaAsk::ai(&mut refs).with_root(AriaRoot::Subtree(alpha_id));
        assert_eq!(sub_ask.root(), AriaRoot::Subtree(alpha_id));
        assert_eq!(sub_ask.depth(), None);
        assert!(!sub_ask.boxes());
        let sub = tree.aria(sub_ask)?;
        assert_eq!(sub.nodes().len(), 1);
        assert!(find_by_role_name(sub.nodes(), "button", Some("Alpha")).is_some());

        let full2 = tree.aria(AriaAsk::ai(&mut refs))?;
        let beta_ref_2 = find_by_role_name(full2.nodes(), "button", Some("Beta"))
            .and_then(|n| n.r#ref.clone())
            .ok_or("missing beta ref 2")?;
        assert_eq!(
            beta_ref_1, beta_ref_2,
            "Subtree capture must not clear last_identity on unseen sibling nodes"
        );
        Ok(())
    }

    #[test]
    fn into_snapshot_renders_and_debug_redacts() -> Result<(), Box<dyn Error>> {
        let apc = ApcBuilder::new("doc-render")
            .root(
                NodeBuilder::root()
                    .child(
                        NodeBuilder::form_control(FormControlType::ButtonButton)
                            .tag("button")
                            .bounding_box(0, 0, 100, 24)
                            .child(NodeBuilder::text("TOP_SECRET_TEXT")),
                    )
                    .child(
                        NodeBuilder::partial_iframe(PartialIframeSpec::CrossSite)
                            .href("https://secret.example/token=xyz")
                            .bounding_box(0, 30, 100, 50),
                    ),
            )
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::new())?;

        let mut refs = TabRefTable::new();
        let ask = AriaAsk::new(
            AriaRoot::Page,
            AriaMode::Ai {
                refs: &mut refs,
                render_active: true,
            },
        );
        let ask_dbg = format!("{ask:?}");
        assert!(ask_dbg.contains("AriaAsk"));

        let aria_tree = tree.aria(ask)?;
        let tree_dbg = format!("{aria_tree:?}");
        assert!(!tree_dbg.contains("TOP_SECRET_TEXT"));
        assert!(!tree_dbg.contains("secret.example"));

        let partial_entry: &AriaPartialFrame =
            aria_tree.partial().first().ok_or("missing partial entry")?;
        let partial_dbg = format!("{partial_entry:?}");
        assert!(!partial_dbg.contains("secret.example"));
        assert!(partial_dbg.contains("frame_len"));

        let cloned_snap = aria_tree.to_snapshot();
        let owned_snap = aria_tree.into_snapshot();
        assert_eq!(cloned_snap.tree, owned_snap.tree);

        let pw_text = neo_snapshot::render(&owned_snap, Call::Playwright).into_text();
        let full_text =
            neo_snapshot::render(&owned_snap, Call::Full { max_tokens: None }).into_text();
        let page_text =
            neo_snapshot::render(&owned_snap, Call::Page(&Passed::default())).into_text();
        assert!(pw_text.contains("button \"TOP_SECRET_TEXT\""));
        assert!(pw_text.contains("# partial:"));
        assert!(full_text.contains("TOP_SECRET_TEXT"));
        assert!(page_text.contains("TOP_SECRET_TEXT"));
        Ok(())
    }

    fn build_non_prunable_deep_apc(depth: usize) -> AnnotatedPageContent {
        let mut current: Option<ContentNode> = None;
        for index in (1..=depth).rev() {
            let is_root = index == 1;
            let attribute_type = if is_root {
                ContentAttributeType::Root
            } else {
                ContentAttributeType::Container
            };
            let mut children_nodes = Vec::new();
            if let Some(child) = current.take() {
                children_nodes.push(child);
            }
            let rect = BoundingRect {
                x: Some(0),
                y: Some(0),
                width: Some(100),
                height: Some(100),
                is_screenshot_relative: Some(false),
            };
            let browseros = (!is_root).then(|| {
                Box::new(BrowserOsContentAttributes {
                    role: Some("group".to_owned()),
                    ..BrowserOsContentAttributes::default()
                })
            });
            current = Some(ContentNode {
                children_nodes,
                content_attributes: Some(ContentAttributes {
                    common_ancestor_dom_node_id: i32::try_from(index).ok(),
                    attribute_type: Some(i32::from(attribute_type)),
                    geometry: Some(Geometry {
                        outer_bounding_box: Some(rect),
                        visible_bounding_box: Some(rect),
                        ..Geometry::default()
                    }),
                    browseros,
                    ..ContentAttributes::default()
                }),
            });
        }

        AnnotatedPageContent {
            version: Some(i32::from(AnnotatedPageContentVersion::Version10)),
            mode: Some(i32::from(AnnotatedPageContentMode::ActionableElements)),
            root_node: current,
            main_frame_data: Some(FrameData {
                document_identifier: Some(DocumentIdentifier {
                    serialized_token: Some("doc-deep-25k".to_owned()),
                }),
                ..FrameData::default()
            }),
            browseros: Some(BrowserOsPageContentData {
                contract_version: Some(MIN_CONTRACT_VERSION),
                subframe_timeout_ms: Some(2_000),
            }),
            ..AnnotatedPageContent::default()
        }
    }

    #[test]
    fn deep_25k_chain_does_not_overflow_2mib_stack() -> Result<(), Box<dyn Error>> {
        let handle = ThreadBuilder::new()
            .name("aria-emit-25k".to_owned())
            .stack_size(2 * 1024 * 1024)
            .spawn(|| -> Result<(), String> {
                let apc = build_non_prunable_deep_apc(25_000);
                let tree =
                    SnapshotTree::build(apc, BuildOptions::new()).map_err(|err| err.to_string())?;

                let mut refs = TabRefTable::new();
                for refused in [
                    tree.aria(AriaAsk::ai(&mut refs).with_boxes(true)),
                    tree.aria(AriaAsk::default_mode()),
                ] {
                    let Err(too_deep) = refused else {
                        return Err("a 25k-level page must be refused, not cut short".to_owned());
                    };
                    if too_deep.levels() <= MAX_TREE_DEPTH
                        || !too_deep
                            .to_string()
                            .starts_with("This tree is nested about")
                    {
                        return Err(format!("unexpected refusal: {too_deep}"));
                    }
                }

                let cut = tree
                    .aria(
                        AriaAsk::ai(&mut refs)
                            .with_boxes(true)
                            .with_depth(Some(100)),
                    )
                    .map_err(|err| err.to_string())?;
                if cut.nodes().len() != 1 {
                    return Err(format!(
                        "expected 1 top-level node, got {}",
                        cut.nodes().len()
                    ));
                }
                let cut_depth = 1 + neo_snapshot::json_depth(cut.nodes());
                if cut_depth > MAX_JSON_DEPTH {
                    return Err(format!(
                        "ai json depth {cut_depth} exceeded MAX_JSON_DEPTH ({MAX_JSON_DEPTH})"
                    ));
                }
                Ok(())
            })?;

        let outcome = match handle.join() {
            Ok(result) => result,
            Err(payload) => {
                return Err(format!("25k stack-safety thread panicked: {payload:?}").into());
            }
        };
        outcome.map_err(Into::into)
    }
}
