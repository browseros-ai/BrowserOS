//! Which non-semantic elements of a frame a user could click, read from Chromium's DOM snapshot.
//!
//! The accessibility tree names real controls, but sites also make plain `div`s clickable with a
//! pointer cursor, a `tabindex` or a script listener. This module finds those without running any
//! JavaScript in the page: one `DOMSnapshot.captureSnapshot` per session reports every node's
//! attributes, layout box, computed `cursor` and `isClickable` (any click, mousedown or mouseup
//! listener in any world, inline handlers and editable content). Page code cannot observe or
//! forge it, unlike a page-world scan that calls hookable built-ins and marks the DOM.
//!
//! ```text
//! acquire_frame_data ─▶ find_clickables(snapshots, session, document)
//!                         ├─ DomSnapshots: one capture per session per snapshot attempt
//!                         ├─ select the frame's document in that capture
//!                         └─ clickables(document): the pure rules below
//! ```

use super::acquisition::SnapshotBudget;
use crate::{ProtocolSession, snapshot::FrameReadReason};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::OnceCell;

/// Tags the accessibility tree already exposes as controls; a hit would only duplicate them.
const INTERACTIVE_TAGS: &[&str] = &[
    "a", "button", "input", "select", "textarea", "details", "summary",
];
const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "link",
    "textbox",
    "checkbox",
    "radio",
    "combobox",
    "listbox",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "searchbox",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "treeitem",
];
const ELEMENT_NODE: i64 = 1;
/// The tags DOM lets host a shadow root besides custom elements ("valid shadow host name").
const SHADOW_HOST_TAGS: &[&str] = &[
    "article",
    "aside",
    "blockquote",
    "body",
    "div",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "main",
    "nav",
    "p",
    "section",
    "span",
];

/// Which document of a session's capture belongs to the frame being acquired.
#[derive(Clone, Copy, Debug)]
pub enum ClickableDocument {
    /// The session's own root document: the top frame or an out-of-process iframe.
    SessionDefault,
    /// A same-process child frame, named by its document node's backend id.
    BackendNode(i64),
    /// The child frame's document could not be resolved.
    Unavailable,
}

/// One DOM capture per session for a whole snapshot attempt.
///
/// Same-process child frames share their parent's session and appear as extra documents in its
/// capture, so every frame of a session reads the same capture instead of re-serializing the
/// whole process's DOM once per frame. A failed capture is remembered too: retrying per frame
/// would multiply the cost of a page that cannot be captured, and each of its frames reports it.
#[derive(Clone, Default)]
pub struct DomSnapshots {
    captures: Arc<Mutex<HashMap<Option<String>, CaptureCell>>>,
}

/// A session's capture, filled once by whichever frame asks first.
type CaptureCell = Arc<OnceCell<Result<Arc<Capture>, FrameReadReason>>>;

impl DomSnapshots {
    async fn capture(
        &self,
        session: &ProtocolSession,
        budget: &SnapshotBudget,
    ) -> Result<Arc<Capture>, FrameReadReason> {
        let key = session.session_id().map(|id| id.0.clone());
        let cell = {
            let Ok(mut captures) = self.captures.lock() else {
                return Err(FrameReadReason::BrowserError);
            };
            captures.entry(key).or_default().clone()
        };
        cell.get_or_init(|| async {
            budget
                .send::<Capture>(
                    session,
                    "DOMSnapshot.captureSnapshot",
                    json!({ "computedStyles": ["cursor"] }),
                )
                .await
                .map(Arc::new)
                .map_err(|error| {
                    tracing::debug!(%error, "DOM snapshot unavailable");
                    FrameReadReason::of(&error)
                })
        })
        .await
        .clone()
    }
}

/// Clickable non-semantic elements of one frame's document, keyed by backend node id, each with
/// the signals that made it one (`cursor:pointer`, `clickable`, `tabindex`).
pub async fn find_clickables(
    snapshots: &DomSnapshots,
    session: &ProtocolSession,
    document: ClickableDocument,
    budget: &SnapshotBudget,
) -> Result<HashMap<i64, Vec<String>>, FrameReadReason> {
    if let ClickableDocument::Unavailable = document {
        return Err(FrameReadReason::NotFound);
    }
    let capture = snapshots.capture(session, budget).await?;
    let selected = select_document(&capture, document).ok_or(FrameReadReason::NotFound)?;
    Ok(clickables(selected, &capture.strings))
}

/// Elements of one frame's document that sit inside a closed shadow root, by backend node id: at
/// least one for every closed root that holds an element, the outermost roots' first. Read from
/// the same per-session capture as [`find_clickables`], so a caller that asks for both makes one
/// `DOMSnapshot.captureSnapshot`. A closed root answers `null` to every script from its host and
/// is reachable only from inside, so hosted Neo resolves these into its own isolated world to
/// read the roots there; desktop Neo never calls it.
pub async fn find_closed_shadow_seeds(
    snapshots: &DomSnapshots,
    session: &ProtocolSession,
    document: ClickableDocument,
    budget: &SnapshotBudget,
) -> Result<Vec<i64>, FrameReadReason> {
    if let ClickableDocument::Unavailable = document {
        return Err(FrameReadReason::NotFound);
    }
    let capture = snapshots.capture(session, budget).await?;
    let selected = select_document(&capture, document).ok_or(FrameReadReason::NotFound)?;
    Ok(closed_shadow_seeds(selected, &capture.strings))
}

/// The frame's document within its session's capture.
fn select_document(capture: &Capture, document: ClickableDocument) -> Option<&Document> {
    match document {
        // Blink serializes the session's root frame first.
        ClickableDocument::SessionDefault => capture.documents.first(),
        ClickableDocument::BackendNode(id) => capture
            .documents
            .iter()
            .find(|document| document.nodes.backend_node_id.first() == Some(&id)),
        ClickableDocument::Unavailable => None,
    }
}

/// The rules over one captured document. A hit is an element that
/// - sits under `body`, outside user-agent shadow roots, and is not a pseudo element;
/// - is not an interactive tag or role (the accessibility tree already exposes those);
/// - has a rendered box of non-zero size; and
/// - has `cursor: pointer`, `isClickable`, or a `tabindex` other than `-1`.
///
/// Two prunes keep a single click target to a single hit:
/// - same target: an element whose nearest rendered ancestor has `cursor: pointer` is part of
///   that ancestor's target, unless it is a target of its own: a `tabindex`, an inline `onclick`,
///   or a listener on an element that names itself (`aria-label`, `aria-labelledby`). Script
///   listeners alone do not count, since sites attach tracking listeners all over a card;
/// - delegation container: an element whose only signal is a listener and that contains a
///   control or another hit is a container handling its children's clicks, not a control.
fn clickables(document: &Document, strings: &[String]) -> HashMap<i64, Vec<String>> {
    let view = View::new(document, strings);
    let count = view.len();
    let body = (0..count).find(|&i| view.is_element(i) && view.name(i) == "body");
    let Some(body) = body else {
        return HashMap::new();
    };

    // DOMSnapshot lists nodes in pre-order, so a parent's facts are known before its children's.
    let mut under_body = vec![false; count];
    let mut in_user_agent_shadow = vec![false; count];
    let mut rendered_ancestor: Vec<Option<usize>> = vec![None; count];
    for i in 0..count {
        let user_agent_root = view.shadow_root_type(i) == Some("user-agent");
        let Some(parent) = view.parent(i).filter(|&parent| parent < i) else {
            in_user_agent_shadow[i] = user_agent_root;
            continue;
        };
        under_body[i] = parent == body || under_body[parent];
        in_user_agent_shadow[i] = user_agent_root || in_user_agent_shadow[parent];
        rendered_ancestor[i] = if view.is_element(parent) && view.layout(parent).is_some() {
            Some(parent)
        } else {
            rendered_ancestor[parent]
        };
    }

    let mut candidates: Vec<Option<Signals>> = vec![None; count];
    for i in 0..count {
        if !view.is_element(i)
            || !under_body[i]
            || in_user_agent_shadow[i]
            || view.is_pseudo(i)
            || view.is_interactive(i)
        {
            continue;
        }
        let Some(layout) = view.layout(i) else {
            continue;
        };
        if !view.has_area(layout) {
            continue;
        }
        let signals = Signals {
            pointer: view.cursor(layout) == Some("pointer"),
            listener: view.is_clickable(i),
            tabindex: view
                .attribute(i, "tabindex")
                .is_some_and(|value| value != "-1"),
            inline_onclick: view.attribute(i, "onclick").is_some(),
        };
        if !signals.pointer && !signals.listener && !signals.tabindex {
            continue;
        }
        let named = ["aria-label", "aria-labelledby"].iter().any(|name| {
            view.attribute(i, name)
                .is_some_and(|value| !value.trim().is_empty())
        });
        let own_target = signals.tabindex || signals.inline_onclick || (signals.listener && named);
        let inside_pointer = rendered_ancestor[i]
            .and_then(|ancestor| view.layout(ancestor))
            .is_some_and(|layout| view.cursor(layout) == Some("pointer"));
        if !own_target && inside_pointer {
            continue;
        }
        candidates[i] = Some(signals);
    }

    let mut contains_control = vec![false; count];
    for i in (0..count).rev() {
        let control = candidates[i].is_some() || (view.is_element(i) && view.is_interactive(i));
        if (control || contains_control[i])
            && let Some(parent) = view.parent(i).filter(|&parent| parent < i)
        {
            contains_control[parent] = true;
        }
    }

    candidates
        .iter()
        .enumerate()
        .filter_map(|(i, signals)| {
            let signals = signals.as_ref()?;
            if signals.listener_only() && contains_control[i] {
                return None;
            }
            Some((view.backend_id(i)?, signals.reasons()))
        })
        .collect()
}

/// The rule over one captured document. Chromium walks the flat tree, lists no node for a shadow
/// root and marks every node inside one with that root's type, so a host's flat children are its
/// root's top level, and a closed root directly inside a closed root reads as a parent and child
/// that are both `closed`, like any two elements of one root. So: for every flat parent that is
/// not a `slot` and has an element child marked `closed`, its first such child is a seed. Every
/// closed root that holds an element is the root of its host's flat children, so each gets a seed
/// at least; many parents inside one root give one each, and the caller reads each seed's root
/// once. A slot's flat children are its assigned nodes, from the tree its host sits in, which
/// that host's own children already reach. A parent marked `closed` whose tag cannot host a shadow
/// root (a list, a row, a button: DOM allows only custom elements and [`SHADOW_HOST_TAGS`]) holds
/// only children of its own root, so it gives none. Pseudo-elements and text nodes are never
/// seeds.
///
/// The seeds whose parent is not itself marked `closed` come first, in document order: each is
/// the top of a closed root in the document or in an open root, the outermost roots, so a caller
/// that keeps only the first few keeps those. A document with no node marked `closed` costs one
/// pass over the shadow column.
fn closed_shadow_seeds(document: &Document, strings: &[String]) -> Vec<i64> {
    let shadow = &document.nodes.shadow_root_type;
    let closed_value = |value: &i64| {
        usize::try_from(*value)
            .ok()
            .and_then(|index| strings.get(index))
            .is_some_and(|kind| kind == "closed")
    };
    if !shadow.value.iter().any(closed_value) {
        return Vec::new();
    }
    let view = View::new(document, strings);
    let closed = |i: usize| view.shadow_root_type(i) == Some("closed");
    let mut seeded = vec![false; view.len()];
    let (mut outermost, mut nested) = (Vec::new(), Vec::new());
    for i in 0..view.len() {
        if !view.is_element(i) || view.is_pseudo(i) || !closed(i) {
            continue;
        }
        let Some(parent) = view.parent(i).filter(|&parent| parent < i) else {
            continue;
        };
        let name = view.name(parent);
        let can_host = || name.contains('-') || SHADOW_HOST_TAGS.contains(&name.as_str());
        if seeded[parent] || name == "slot" || (closed(parent) && !can_host()) {
            continue;
        }
        let Some(seed) = view.backend_id(i) else {
            continue;
        };
        seeded[parent] = true;
        if closed(parent) {
            nested.push(seed);
        } else {
            outermost.push(seed);
        }
    }
    outermost.extend(nested);
    outermost
}

#[derive(Clone, Copy, Debug)]
struct Signals {
    pointer: bool,
    listener: bool,
    tabindex: bool,
    inline_onclick: bool,
}

impl Signals {
    fn listener_only(self) -> bool {
        self.listener && !self.pointer && !self.tabindex && !self.inline_onclick
    }

    fn reasons(self) -> Vec<String> {
        [
            (self.pointer, "cursor:pointer"),
            (self.listener, "clickable"),
            (self.tabindex, "tabindex"),
        ]
        .into_iter()
        .filter(|(present, _)| *present)
        .map(|(_, reason)| reason.to_string())
        .collect()
    }
}

/// Bounds-checked reads over the capture's column arrays and shared string table. A malformed
/// or truncated capture yields fewer hits, never a panic.
struct View<'a> {
    document: &'a Document,
    strings: &'a [String],
    layout_of: HashMap<usize, usize>,
    clickable: std::collections::HashSet<usize>,
    pseudo: std::collections::HashSet<usize>,
    shadow_root_type: HashMap<usize, i64>,
}

impl<'a> View<'a> {
    fn new(document: &'a Document, strings: &'a [String]) -> Self {
        let mut layout_of = HashMap::new();
        for (layout, &node) in document.layout.node_index.iter().enumerate() {
            if let Ok(node) = usize::try_from(node) {
                layout_of.entry(node).or_insert(layout);
            }
        }
        let indexes = |values: &[i64]| {
            values
                .iter()
                .filter_map(|&value| usize::try_from(value).ok())
                .collect()
        };
        let shadow = &document.nodes.shadow_root_type;
        Self {
            document,
            strings,
            layout_of,
            clickable: indexes(&document.nodes.is_clickable.index),
            pseudo: indexes(&document.nodes.pseudo_type.index),
            shadow_root_type: shadow
                .index
                .iter()
                .zip(&shadow.value)
                .filter_map(|(&node, &value)| Some((usize::try_from(node).ok()?, value)))
                .collect(),
        }
    }

    fn len(&self) -> usize {
        self.document.nodes.node_name.len()
    }

    fn string(&self, index: i64) -> Option<&'a str> {
        let strings: &'a [String] = self.strings;
        strings
            .get(usize::try_from(index).ok()?)
            .map(String::as_str)
    }

    fn is_element(&self, i: usize) -> bool {
        self.document.nodes.node_type.get(i) == Some(&ELEMENT_NODE)
    }

    fn name(&self, i: usize) -> String {
        self.document
            .nodes
            .node_name
            .get(i)
            .and_then(|&index| self.string(index))
            .unwrap_or_default()
            .to_ascii_lowercase()
    }

    fn parent(&self, i: usize) -> Option<usize> {
        usize::try_from(*self.document.nodes.parent_index.get(i)?).ok()
    }

    fn backend_id(&self, i: usize) -> Option<i64> {
        self.document.nodes.backend_node_id.get(i).copied()
    }

    fn attribute(&self, i: usize, name: &str) -> Option<&'a str> {
        let (pairs, _) = self.document.nodes.attributes.get(i)?.as_chunks::<2>();
        pairs.iter().find_map(|&[key, value]| {
            self.string(key)
                .filter(|key| key.eq_ignore_ascii_case(name))
                .and_then(|_| self.string(value))
        })
    }

    fn is_interactive(&self, i: usize) -> bool {
        INTERACTIVE_TAGS.contains(&self.name(i).as_str())
            || self
                .attribute(i, "role")
                .is_some_and(|role| INTERACTIVE_ROLES.contains(&role.to_ascii_lowercase().as_str()))
    }

    fn is_clickable(&self, i: usize) -> bool {
        self.clickable.contains(&i)
    }

    fn is_pseudo(&self, i: usize) -> bool {
        self.pseudo.contains(&i)
    }

    fn shadow_root_type(&self, i: usize) -> Option<&'a str> {
        self.string(*self.shadow_root_type.get(&i)?)
    }

    fn layout(&self, i: usize) -> Option<usize> {
        self.layout_of.get(&i).copied()
    }

    fn cursor(&self, layout: usize) -> Option<&'a str> {
        let styles = self.document.layout.styles.get(layout)?;
        self.string(*styles.first()?)
    }

    fn has_area(&self, layout: usize) -> bool {
        self.document
            .layout
            .bounds
            .get(layout)
            .is_some_and(|bounds| bounds.len() >= 4 && bounds[2] > 0.0 && bounds[3] > 0.0)
    }
}

/// The subset of `DOMSnapshot.captureSnapshot`'s columnar result this module reads.
#[derive(Debug, Default, Deserialize)]
struct Capture {
    #[serde(default)]
    documents: Vec<Document>,
    #[serde(default)]
    strings: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Document {
    #[serde(default)]
    nodes: Nodes,
    #[serde(default)]
    layout: Layout,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Nodes {
    #[serde(default)]
    parent_index: Vec<i64>,
    #[serde(default)]
    node_type: Vec<i64>,
    #[serde(default)]
    shadow_root_type: RareStrings,
    #[serde(default)]
    node_name: Vec<i64>,
    #[serde(default)]
    backend_node_id: Vec<i64>,
    #[serde(default)]
    attributes: Vec<Vec<i64>>,
    #[serde(default)]
    pseudo_type: RareStrings,
    #[serde(default)]
    is_clickable: RareBooleans,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Layout {
    #[serde(default)]
    node_index: Vec<i64>,
    #[serde(default)]
    styles: Vec<Vec<i64>>,
    #[serde(default)]
    bounds: Vec<Vec<f64>>,
}

#[derive(Debug, Default, Deserialize)]
struct RareStrings {
    #[serde(default)]
    index: Vec<i64>,
    #[serde(default)]
    value: Vec<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct RareBooleans {
    #[serde(default)]
    index: Vec<i64>,
}

#[cfg(test)]
mod tests;
