//! Frame identity and document hosts for a tab. Selectors start in the call's frame scope.

/// The document from which a call starts resolving selectors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FrameScope {
    Main,
}
