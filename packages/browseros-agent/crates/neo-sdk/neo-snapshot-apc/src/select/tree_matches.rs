use std::fmt::{self, Write as _};

use crate::tree::{NodeId, NodeKey, SnapshotTree};

/// The elements one tree stage selected, and the first `aria-ref` it could not resolve.
///
/// The order is the order of Playwright's `querySelectorAll` over the tree: each root's matches
/// in flat-tree order, roots in the order the previous part produced them (design 5.3). A
/// locator's `count()`, strict mode and `nth()` read this set before anything is hydrated
/// (design 5.4), so its order and size are final answers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeMatches {
    nodes: Vec<NodeId>,
    stale_ref: Option<String>,
}

impl TreeMatches {
    pub(crate) fn new(nodes: Vec<NodeId>, stale_ref: Option<String>) -> Self {
        Self { nodes, stale_ref }
    }

    /// The matched nodes, in Playwright's order. They index the tree the stage ran over and mean
    /// nothing in another capture.
    #[must_use]
    pub fn nodes(&self) -> &[NodeId] {
        &self.nodes
    }

    /// How many elements matched: a locator's `count()`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether nothing matched.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Each match's cross-capture key, in order, for hydration. A node with no DOM node id has
    /// none: such a node is never an element today, but the option nodes that engine chunk 5
    /// synthesizes for a collapsed `<select>` will be, and they hydrate through their select.
    pub fn keys<'t>(
        &'t self,
        tree: &'t SnapshotTree,
    ) -> impl Iterator<Item = Option<NodeKey>> + 't {
        self.nodes.iter().map(move |&id| tree.node_key(id))
    }

    /// The first `aria-ref` body the stage met that names no live node. When nothing matched,
    /// the op fails at once (`element_detached`) instead of waiting for a ref that cannot come
    /// back, as the probe's `stale_ref` reply does today.
    #[must_use]
    pub fn stale_ref(&self) -> Option<&str> {
        self.stale_ref.as_deref()
    }

    /// The one element a strict locator acts on (`None` when nothing matched), or the violation
    /// when more than one did, as `querySelector(selector, root, strict)` decides in
    /// `injectedScript.ts`.
    pub fn strict(&self) -> Result<Option<NodeId>, StrictViolation> {
        match self.nodes.as_slice() {
            [] => Ok(None),
            [only] => Ok(Some(*only)),
            many => Err(StrictViolation {
                count: many.len(),
                preview_nodes: many
                    .iter()
                    .take(StrictViolation::PREVIEW_LIMIT)
                    .copied()
                    .collect(),
            }),
        }
    }
}

/// More than one element matched a strict locator.
///
/// Rust decides the violation from the tree, before any hydration. The page previews the first
/// [`Self::PREVIEW_LIMIT`] elements and builds their `aka` locators (design 5.5), and
/// [`Self::message`] joins the two into upstream's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrictViolation {
    count: usize,
    preview_nodes: Vec<NodeId>,
}

impl StrictViolation {
    /// How many elements upstream lists in the message: `_generateSelectors` takes ten in
    /// Chromium (`injectedScript.ts`).
    pub const PREVIEW_LIMIT: usize = 10;

    /// How many elements matched.
    #[must_use]
    pub fn count(&self) -> usize {
        self.count
    }

    /// The first [`Self::PREVIEW_LIMIT`] matches, in order: the elements the page previews.
    #[must_use]
    pub fn preview_nodes(&self) -> &[NodeId] {
        &self.preview_nodes
    }

    /// Upstream's `strictModeViolationError` text. `locator` is the chunk as `asLocator` prints
    /// it, and `previews` holds the page's lines for [`Self::preview_nodes`], in the same order.
    /// When there are fewer lines than matches, upstream ends the list with `...`.
    #[must_use]
    pub fn message(&self, locator: &str, previews: &[StrictPreview]) -> String {
        strict_message(locator, self.count, previews)
    }
}

/// Upstream's `strictModeViolationError` text for `count` matches of `locator`, the first of them
/// worded by `previews`: the one formatter of a violation Rust decided ([`StrictViolation`]) and
/// of one the page decided whose `aka` lines Rust rewrote with the tree's role locators.
#[must_use]
pub fn strict_message(locator: &str, count: usize, previews: &[StrictPreview]) -> String {
    let mut message = format!("strict mode violation: {locator} resolved to {count} elements:");
    let listed = previews.iter().take(StrictViolation::PREVIEW_LIMIT);
    let mut lines = 0;
    for (index, line) in listed.enumerate() {
        let _ = write!(
            message,
            "\n    {}) {} aka {}",
            index + 1,
            line.preview,
            line.aka
        );
        lines += 1;
    }
    if lines < count {
        message.push_str("\n    ...");
    }
    message.push('\n');
    message
}

/// The page's lines for one element of a strict-mode violation: `previewNode`'s markup and the
/// `aka` locator the selector generator built for it.
///
/// Both carry page text (masked by the page, `failure-output.ts`), so `Debug` prints only their
/// lengths.
#[derive(Clone, PartialEq, Eq)]
pub struct StrictPreview {
    /// The element as `InjectedScript.previewNode` prints it, such as `<button>Go</button>`.
    pub preview: String,
    /// The locator that selects only this element, as `asLocator` prints it, such as
    /// `getByRole('button', { name: 'Go' })`.
    pub aka: String,
}

impl fmt::Debug for StrictPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StrictPreview")
            .field("preview_len", &self.preview.len())
            .field("aka_len", &self.aka.len())
            .finish()
    }
}
