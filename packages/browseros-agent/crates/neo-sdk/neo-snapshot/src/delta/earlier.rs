use crate::form::Form;

/// What a delta has to compare with, as only the snapshot operation can say: it reads the tab's
/// stored text under the call's clock. A delta is built from nothing else, so "compare with a text
/// pending in this run" cannot be written.
#[derive(Clone, Copy, Debug)]
pub enum Earlier<'a> {
    /// The call's time ran out before the tab's stored text could be read. The answer is the
    /// whole page under `# whole page: no time was left …`, and nothing is kept: the call that
    /// returns it records nothing.
    NotRead,
    /// The tab holds no text of this document.
    Nothing,
    /// The tab's stored text and the form it was rendered in.
    Text(&'a Baseline),
}

/// A whole page's text as the caller was given it in Neo's default format, after its heads and
/// its cut, and the form it was rendered in: what the tab's store keeps and what a later delta
/// compares with. neo-playwright's stores hold it across runs; this crate renders every one.
#[derive(Clone, Debug, PartialEq)]
pub struct Baseline {
    /// The text, as the caller was given it.
    pub text: String,
    /// How it was rendered, which a delta renders the new page in.
    pub form: Form,
}
