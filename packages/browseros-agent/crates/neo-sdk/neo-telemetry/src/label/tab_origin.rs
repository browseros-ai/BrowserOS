//! How a tab came to be open, as a label.

/// Who opened a tab: the `neo.tab.origin` of the tab metrics. Mirrors the server's own tab
/// registry origin (`browser/tab_registry.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TabOrigin {
    /// An agent's script opened it.
    Agent,
    /// It was open before Neo saw it, or someone other than an agent opened it.
    Found,
    /// Neo opened it for itself (its blank tab).
    Neo,
}

impl TabOrigin {
    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Found => "found",
            Self::Neo => "neo",
        }
    }
}
