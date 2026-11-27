use std::{fmt, str};

use serde_json::Number;

use crate::aria_node::{AriaChild, AriaNode, Invalid, NodeBox, Tristate};
use crate::snapshot::{PartialFrame, Snapshot};

/// Maximum length of a canonical formatted ref (`f4294967295e4294967295` is 22 ASCII bytes).
pub const INLINE_REF_CAP: usize = 24;

/// A node's ref text: borrowed from an existing [`AriaNode`] or stored as compact integer parts
/// formatted lazily only when printed.
#[derive(Clone, Copy)]
pub enum NodeRef<'a> {
    /// Borrowed from an existing string (such as [`AriaNode::r#ref`]).
    Borrowed(&'a str),
    /// Compact canonical ref (`e<n>`, `n<n>`, `f<seq>e<n>`, `f<seq>n<n>`), avoiding formatting on
    /// nodes whose ref is never emitted.
    Compact {
        /// Document sequence (`0` omits the `f<seq>` prefix).
        seq: u32,
        /// ASCII channel byte (`b'e'` or `b'n'`).
        kind: u8,
        /// 1-based element counter.
        element: u32,
    },
}

impl<'a> NodeRef<'a> {
    /// Wraps a borrowed string slice.
    #[must_use]
    pub const fn borrowed(value: &'a str) -> Self {
        Self::Borrowed(value)
    }

    /// Stores a canonical compact ref without formatting until needed.
    #[must_use]
    pub const fn compact(seq: u32, kind: u8, element: u32) -> Self {
        Self::Compact { seq, kind, element }
    }

    /// Whether the ref string is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        match self {
            Self::Borrowed(value) => value.is_empty(),
            Self::Compact { .. } => false,
        }
    }

    /// Appends the formatted ref text directly into `out` without `std::fmt` dispatch.
    pub fn write_to(&self, out: &mut String) {
        match *self {
            Self::Borrowed(value) => out.push_str(value),
            Self::Compact { seq, kind, element } => {
                let mut buf = [0u8; INLINE_REF_CAP];
                let len = format_compact_bytes(seq, kind, element, &mut buf);
                if let Some(bytes) = buf.get(..len)
                    && let Ok(text) = str::from_utf8(bytes)
                {
                    out.push_str(text);
                }
            }
        }
    }

    /// Returns this ref as a string slice, formatting into `buf` only for [`Self::Compact`].
    #[must_use]
    pub fn format_into<'b>(&self, buf: &'b mut [u8; INLINE_REF_CAP]) -> &'b str
    where
        'a: 'b,
    {
        match *self {
            Self::Borrowed(value) => value,
            Self::Compact { seq, kind, element } => {
                let len = format_compact_bytes(seq, kind, element, buf);
                buf.get(..len)
                    .and_then(|bytes| str::from_utf8(bytes).ok())
                    .unwrap_or_default()
            }
        }
    }

    /// Compares this ref's text against `other` without heap allocation.
    #[must_use]
    pub fn eq_str(&self, other: &str) -> bool {
        let mut buf = [0u8; INLINE_REF_CAP];
        self.format_into(&mut buf) == other
    }

    /// Allocates an owned `String` holding this ref's text.
    #[must_use]
    pub fn to_owned_string(&self) -> String {
        match *self {
            Self::Borrowed(value) => value.to_owned(),
            Self::Compact { .. } => {
                let mut out = String::with_capacity(12);
                self.write_to(&mut out);
                out
            }
        }
    }
}

impl PartialEq for NodeRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        let mut lhs_buf = [0u8; INLINE_REF_CAP];
        let mut rhs_buf = [0u8; INLINE_REF_CAP];
        self.format_into(&mut lhs_buf) == other.format_into(&mut rhs_buf)
    }
}

impl Eq for NodeRef<'_> {}

impl fmt::Debug for NodeRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = [0u8; INLINE_REF_CAP];
        f.debug_tuple("NodeRef")
            .field(&self.format_into(&mut buf))
            .finish()
    }
}

impl fmt::Display for NodeRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = [0u8; INLINE_REF_CAP];
        f.write_str(self.format_into(&mut buf))
    }
}

fn push_u32_ascii(buf: &mut [u8; INLINE_REF_CAP], mut pos: usize, mut n: u32) -> usize {
    let mut digits = [0u8; 10];
    let mut count = 0usize;
    loop {
        if let Some(slot) = digits.get_mut(count) {
            *slot = b'0' + u8::try_from(n % 10).unwrap_or(0);
        }
        count += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    while count > 0 {
        count -= 1;
        if let (Some(&digit), Some(dst)) = (digits.get(count), buf.get_mut(pos)) {
            *dst = digit;
            pos += 1;
        }
    }
    pos
}

fn format_compact_bytes(seq: u32, kind: u8, element: u32, buf: &mut [u8; INLINE_REF_CAP]) -> usize {
    let mut pos = 0usize;
    if seq != 0 {
        if let Some(slot) = buf.get_mut(0) {
            *slot = b'f';
        }
        pos = push_u32_ascii(buf, 1, seq);
    }
    if let Some(slot) = buf.get_mut(pos) {
        *slot = kind;
        pos += 1;
    }
    push_u32_ascii(buf, pos, element)
}

/// Borrowed properties of one emitted ARIA node, handed to an [`AriaSink`] or read from an
/// [`AriaSource`](crate::AriaSource) without allocating an [`AriaNode`].
#[derive(Clone, Copy, Debug)]
pub struct EmittedNode<'a> {
    /// ARIA role name, `iframe`, or `text`.
    pub role: &'a str,
    /// Accessible name, if present.
    pub name: Option<&'a str>,
    /// Checkbox, radio, or switch checked state.
    pub checked: Option<Tristate>,
    /// Disabled state.
    pub disabled: Option<bool>,
    /// Expanded state.
    pub expanded: Option<bool>,
    /// Active (focused) state.
    pub active: Option<bool>,
    /// Top-most inert subtree mark.
    pub inert: Option<bool>,
    /// Whole-view covering layer ref mark.
    pub on_top: Option<&'a str>,
    /// Validation invalid state.
    pub invalid: Option<Invalid>,
    /// Heading or treeitem level.
    pub level: Option<&'a Number>,
    /// Toggle button pressed state.
    pub pressed: Option<Tristate>,
    /// Option, tab, or row selected state.
    pub selected: Option<bool>,
    /// Outermost emitted `aria-hidden="true"` boundary in this document (own or inherited from an
    /// unemitted ancestor; omitted beneath an already-marked emitted ancestor).
    pub aria_hidden: Option<bool>,
    /// Snapshot ref (`eN` / `fMeN`), borrowed or inline.
    pub r#ref: Option<NodeRef<'a>>,
    /// Pointer cursor marker (`"pointer"`).
    pub cursor: Option<&'a str>,
    /// Rounded viewport bounding box when `boxes` was requested.
    pub r#box: Option<&'a NodeBox>,
    /// Link raw `href`.
    pub url: Option<&'a str>,
    /// Textbox placeholder when distinct from the name.
    pub placeholder: Option<&'a str>,
    /// Sole text content folded onto the node.
    pub text: Option<&'a str>,
    /// Whether child events follow this node before its matching `end_node`.
    pub has_children: bool,
}

impl<'a> From<&'a AriaNode> for EmittedNode<'a> {
    fn from(node: &'a AriaNode) -> Self {
        Self {
            role: &node.role,
            name: node.name.as_deref(),
            checked: node.checked,
            disabled: node.disabled,
            expanded: node.expanded,
            active: node.active,
            inert: node.inert,
            on_top: node.on_top.as_deref(),
            invalid: node.invalid,
            level: node.level.as_ref(),
            pressed: node.pressed,
            selected: node.selected,
            aria_hidden: node.aria_hidden,
            r#ref: node.r#ref.as_deref().map(NodeRef::borrowed),
            cursor: node.cursor.as_deref(),
            r#box: node.r#box.as_ref(),
            url: node.url.as_deref(),
            placeholder: node.placeholder.as_deref(),
            text: node.text.as_deref(),
            has_children: !node.children.is_empty(),
        }
    }
}

/// Streaming consumer of emitted ARIA nodes, text runs, and partial-frame records in document
/// preorder.
///
/// Snapshot emitters walk their distilled drafts and push events to an `AriaSink` so text
/// printers and external consumers avoid allocating an intermediate `Vec<AriaNode>`. Callers that
/// need the tree (`toMatchAriaSnapshot`, `ariaSnapshotJSON`, test comparisons) pass
/// [`AriaNodeSink`].
pub trait AriaSink {
    /// Opens an emitted node; balanced by a later [`Self::end_node`].
    fn begin_node(&mut self, node: &EmittedNode<'_>);

    /// Emits a text run under the currently open node, or at the top level when none is open.
    fn text(&mut self, text: &str);

    /// Closes the most recently opened node.
    fn end_node(&mut self);

    /// Records an embedded iframe whose content was omitted, redacted, or refused.
    fn partial_frame(&mut self, frame: PartialFrame);
}

/// [`AriaSink`] adapter that materializes the streamed events into owned [`AriaNode`]s and
/// [`PartialFrame`] records, iteratively so arbitrarily deep trees cannot overflow the stack.
#[derive(Debug, Default)]
pub struct AriaNodeSink {
    roots: Vec<AriaNode>,
    stack: Vec<AriaNode>,
    partial: Vec<PartialFrame>,
}

impl AriaNodeSink {
    /// Creates an empty [`AriaNode`] builder sink.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Consumes this sink and returns its top-level nodes and partial-frame records.
    #[must_use]
    pub fn into_parts(mut self) -> (Vec<AriaNode>, Vec<PartialFrame>) {
        while let Some(node) = self.stack.pop() {
            if let Some(parent) = self.stack.last_mut() {
                parent.children.push(AriaChild::Node(Box::new(node)));
            } else {
                self.roots.push(node);
            }
        }
        (self.roots, self.partial)
    }

    /// Consumes this sink and wraps its nodes and partial frames into a [`Snapshot`].
    #[must_use]
    pub fn into_snapshot(self) -> Snapshot {
        let (tree, partial) = self.into_parts();
        Snapshot {
            tree,
            partial,
            ..Snapshot::default()
        }
    }
}

impl AriaSink for AriaNodeSink {
    fn begin_node(&mut self, node: &EmittedNode<'_>) {
        self.stack.push(AriaNode {
            role: node.role.to_owned(),
            name: node.name.map(str::to_owned),
            checked: node.checked,
            disabled: node.disabled,
            expanded: node.expanded,
            active: node.active,
            inert: node.inert,
            on_top: node.on_top.map(str::to_owned),
            invalid: node.invalid,
            level: node.level.cloned(),
            pressed: node.pressed,
            selected: node.selected,
            aria_hidden: node.aria_hidden,
            r#ref: node.r#ref.map(|r| r.to_owned_string()),
            cursor: node.cursor.map(str::to_owned),
            r#box: node.r#box.cloned(),
            url: node.url.map(str::to_owned),
            placeholder: node.placeholder.map(str::to_owned),
            text: node.text.map(str::to_owned),
            children: Vec::new(),
        });
    }

    fn text(&mut self, text: &str) {
        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(AriaChild::Text(text.to_owned()));
        } else {
            self.roots.push(AriaNode {
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
                text: Some(text.to_owned()),
                children: Vec::new(),
            });
        }
    }

    fn end_node(&mut self) {
        let Some(finished) = self.stack.pop() else {
            return;
        };
        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(AriaChild::Node(Box::new(finished)));
        } else {
            self.roots.push(finished);
        }
    }

    fn partial_frame(&mut self, frame: PartialFrame) {
        self.partial.push(frame);
    }
}
