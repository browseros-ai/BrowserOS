use crate::cap::MaxTokens;

/// How a text in Neo's default format was rendered (the TypeScript's `SnapshotForm`). A whole
/// page's text is kept as the tab's baseline beside its form, and a later delta renders the new
/// snapshot in that same form, because two texts rendered differently differ on lines that did
/// not change: the page's text, every link's url, the cut.
///
/// Part of the interface: [`Pending`](crate::Pending) lends it beside the text to keep, and only
/// [`render`](crate::render) decides it, so a kept text is always in the form it was printed in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Form {
    /// How the page's text was decided, and what that came to.
    pub text: TextForm,
    /// Each link's `/url` line is printed.
    pub urls: bool,
    /// The cap the text was cut at.
    pub max_tokens: MaxTokens,
    /// Inert subtrees are listed (`{ inert: true }`). False and absent are one form: the
    /// TypeScript writes the key only when it is true.
    pub inert: bool,
}

/// What `text` came to: as the script passed it, or what the length rule chose when it was left
/// out. Part of a [`Form`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextForm {
    /// Left out by the script, and the page's text was short enough to keep.
    Kept,
    /// Left out by the script, and the page's text was too long: printed without it, under the
    /// `# text left out:` notice.
    LeftOut,
    /// `{ text: true }`.
    Always,
    /// `{ text: false }`.
    Never,
}

/// The default format's options as the script passed them, each `None` when left out. The
/// prelude has checked their types with its own messages. "Left out" survives to the renderer
/// because a delta takes what was left out from its baseline's form, not from the defaults.
///
/// Part of the interface: [`Call::Element`](crate::Call::Element) and
/// [`Call::Page`](crate::Call::Page) borrow one, and the crate applies every default itself.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Passed {
    /// `text`: print the page's text (`true`), never (`false`), or by the length rule (`None`).
    pub text: Option<bool>,
    /// `urls`: print each link's `/url` line; left out, they are not printed.
    pub urls: Option<bool>,
    /// `maxTokens`: the cap; left out, [`DEFAULT_MAX_TOKENS`](crate::DEFAULT_MAX_TOKENS).
    pub max_tokens: Option<MaxTokens>,
    /// `inert`: list inert subtrees (`true`); left out or `false`, they are counted on one line.
    pub inert: Option<bool>,
}
