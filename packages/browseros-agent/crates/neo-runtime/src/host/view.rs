//! Neo: What a live view may show of one session at one instant: the hand-off between the session
//! ledger (`sessions.rs`, which derives it) and capture (`capture.rs`, which follows it). Derived,
//! never stored; the ledger publishes a fresh one on a `watch` whenever any input to it changes,
//! so a follower never polls.

use crate::host::ids::TabId;
use crate::run::PageId;

/// One tab a session may show: a tab some run of the session touched, open in the browser, and
/// not tainted by a withheld session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShownTab {
    pub(crate) target: PageId,
    pub(crate) tab: TabId,
    /// From the tab directory (`Target.targetInfoChanged`): the `X-Url` header.
    pub(crate) url: String,
}

/// A session's live view. `tabs[0]` is the current tab: the one rule the screencast follower,
/// the `?tab=` check and `/v1`'s `currentTabId` all read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FollowView {
    /// The session can still be watched; false once it ended (idle, explicit end, restart).
    pub(crate) live: bool,
    /// Its visuals are withheld: every stream of it ends, and new ones are refused.
    pub(crate) withheld: bool,
    /// Most recently touched first.
    pub(crate) tabs: Vec<ShownTab>,
}
