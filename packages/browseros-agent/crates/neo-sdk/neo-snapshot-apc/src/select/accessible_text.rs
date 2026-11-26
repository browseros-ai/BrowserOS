use std::borrow::Cow;

use crate::tree::{NameCache, NameVisibility, NodeId};

/// Where a tree stage reads the accessible names and descriptions that its `name` and
/// `description` filters match.
///
/// Production callers pass the tree's name pass, `&tree.name_cache()` (design 3.3, engine chunk
/// 4), through the [`NameCache`] impl below. The seam stays a parameter rather than a cache that
/// each `select` builds because a locator attempt (chunk 11) builds one [`NameCache`] and shares
/// it across every stage, strict-mode hint and expect matcher; a cache per `select` would
/// recompute names for each hint query. A stage asks only for nodes that already passed every
/// cheaper check, in `queryRole`'s order, so an implementation may compute on demand.
pub trait AccessibleText {
    /// `node`'s accessible name as `getElementAccessibleNameText(element, includeHidden)` returns
    /// it, or `""` when it has none. The stage normalizes whitespace itself before matching.
    fn name(&self, node: NodeId, visibility: NameVisibility) -> Cow<'_, str>;

    /// `node`'s accessible description as `getElementAccessibleDescription(element,
    /// includeHidden).text` returns it, or `""` when it has none.
    fn description(&self, node: NodeId, visibility: NameVisibility) -> Cow<'_, str>;
}

/// The name pass's memoized answers, borrowed from the cache.
impl AccessibleText for NameCache<'_> {
    fn name(&self, node: NodeId, visibility: NameVisibility) -> Cow<'_, str> {
        Cow::Borrowed(NameCache::name(self, node, visibility).text())
    }

    fn description(&self, node: NodeId, visibility: NameVisibility) -> Cow<'_, str> {
        Cow::Borrowed(NameCache::description(self, node, visibility).text())
    }
}
