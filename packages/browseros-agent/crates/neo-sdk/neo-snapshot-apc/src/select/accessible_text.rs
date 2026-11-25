use std::borrow::Cow;

use crate::tree::NodeId;

/// Whether content hidden for ARIA counts toward an accessible name or description: the
/// `includeHidden` argument of roleUtils.ts's `getElementAccessibleNameText` and
/// `getElementAccessibleDescription`, which a role part sets with `include-hidden`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncludeHidden {
    /// Hidden descendants and referents contribute nothing: every role part without
    /// `include-hidden`.
    No,
    /// Hidden content counts, as `[include-hidden]` asks.
    Yes,
}

/// Where a tree stage reads the accessible names and descriptions that its `name` and
/// `description` filters match.
///
/// This is the seam to the name pass (design 3.3, engine chunk 4), which computes names from the
/// tree's facts and memoizes them. Until that pass lands, tests implement it over names set
/// directly. A stage asks only for nodes that already passed every cheaper check, in
/// `queryRole`'s order, so an implementation may compute on demand.
pub trait AccessibleText {
    /// `node`'s accessible name as `getElementAccessibleNameText(element, includeHidden)` returns
    /// it, or `""` when it has none. The stage normalizes whitespace itself before matching.
    fn name(&self, node: NodeId, hidden: IncludeHidden) -> Cow<'_, str>;

    /// `node`'s accessible description as `getElementAccessibleDescription(element,
    /// includeHidden).text` returns it, or `""` when it has none.
    fn description(&self, node: NodeId, hidden: IncludeHidden) -> Cow<'_, str>;
}
