use crate::tree::NodeKey;

/// Where a tree stage looks up the node that an `aria-ref` part names.
///
/// This is the seam to the tab's ref table (design 3.6, `TabRefTable` in engine chunk 5). A ref
/// that leads a chunk never reaches a stage, because neo-playwright resolves it without a capture
/// (design 6.4). A stage meets the refs nested in a chain, such as
/// `filter({ has: page.getByRef('e7') })`.
pub trait RefResolver {
    /// The node that `body` (`e5`, `f2e5`, `n1`, `f2n1`) names, or `None` when no live node
    /// answers to it: never issued, retired with its document, or evicted. The stage reports the
    /// first such ref as [`TreeMatches::stale_ref`](super::TreeMatches::stale_ref), as the page's
    /// ref engine reports an unresolved ref today (`aria-ref.ts`).
    fn resolve(&self, body: &str) -> Option<NodeKey>;
}
