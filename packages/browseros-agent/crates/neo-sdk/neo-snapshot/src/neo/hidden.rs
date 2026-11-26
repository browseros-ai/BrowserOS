use crate::aria_sink::EmittedNode;

/// What a line inherits of an `aria-hidden` node above it in its document. Upstream marks only
/// the node that carries the attribute, and its nesting shows what the mark hides. `getByRole`
/// reaches those elements only with `includeHidden`, which an agent asks for only where a line, or
/// a line it is nested under, says `[aria-hidden]`. This format moves lines out of that nesting
/// three ways, and the mark must go with each line:
///
/// ```text
///  flatten   a wrapper prints no line; a control's or a heading's content prints beside it
///  cut       the cap prints a kept block after its cut at the left edge, outside its sections
///  on top    a whole-view layer's content prints first, outside every line around it
/// ```
///
/// A child document is never hidden by its iframe's ancestors, as `aria-hidden` stops at the
/// document (`isElementHiddenForAria` walks one), so an iframe's content starts from [`Hidden::No`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hidden {
    /// No `aria-hidden` node is above the line in its document.
    No,
    /// A line around this one says `[aria-hidden]`, so the nesting shows it while the line stays
    /// inside.
    Enclosed,
    /// An `aria-hidden` node above printed no line around this one: the line says it itself.
    Carried,
}

impl Hidden {
    /// What `node`'s content inherits when it prints with no line of `node` around it, beside
    /// `node`'s line or lifted out of it: `node`'s own mark carries on, as does the one `node`
    /// carries. An iframe's content is a child document, which no mark above reaches.
    pub(super) fn beside(self, node: &EmittedNode<'_>) -> Self {
        if node.role == "iframe" {
            Self::No
        } else if self == Self::Carried || node.aria_hidden == Some(true) {
            Self::Carried
        } else {
            self
        }
    }

    /// What `node`'s content inherits nested under `node`'s line, which says any mark `node` has
    /// or carries. An iframe's content is a child document, which no mark above reaches.
    pub(super) fn under(self, node: &EmittedNode<'_>) -> Self {
        if node.role == "iframe" {
            Self::No
        } else if self != Self::No || node.aria_hidden == Some(true) {
            Self::Enclosed
        } else {
            Self::No
        }
    }

    /// What a line that may print outside every line around it inherits: a kept block's, which
    /// the cap prints at the left edge after its cut, so it says a mark only its sections show.
    pub(super) fn movable(self) -> Self {
        match self {
            Self::No => Self::No,
            Self::Enclosed | Self::Carried => Self::Carried,
        }
    }
}
