use crate::aria_node::AriaNode;
use crate::playwright::NodeAlone;

use super::rule::interactive;

/// What one line keeps of its node beyond the key's role, name and marks.
#[derive(Clone, Copy, Debug)]
pub(super) struct Keep<'a> {
    /// The ref, where the agent can act on the node or scope to it.
    pub(super) r#ref: bool,
    /// The `/url` line, when the script asked for urls.
    pub(super) urls: bool,
    /// The text after the key: a value, a label, a kept block's only text. `None` prints none,
    /// whatever the node's own `text` is.
    pub(super) text: Option<&'a str>,
}

/// `copy`: Playwright's node for one line of Neo's format, for upstream's printer. No children;
/// the ref only where `keep` says; `[cursor=pointer]` only beside a kept ref on a non-interactive
/// role, where it is what says the node is clickable; `/url` only when `keep` asks for it; the
/// text `keep` names. `/placeholder` stays, as Playwright prints it.
pub(super) fn node_line<'a>(node: &'a AriaNode, keep: Keep<'a>) -> NodeAlone<'a> {
    let r#ref = node.r#ref.as_deref().filter(|_| keep.r#ref);
    NodeAlone {
        r#ref,
        cursor: node
            .cursor
            .as_deref()
            .filter(|_| r#ref.is_some() && !interactive(&node.role)),
        url: node.url.as_deref().filter(|_| keep.urls),
        text: keep.text,
        ..NodeAlone::from(node)
    }
}
