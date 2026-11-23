use crate::cap::MaxTokens;

/// How a text in Neo's default format was rendered (the TypeScript's `SnapshotForm`). A whole
/// page's text is kept as the tab's baseline beside its form, and a later delta renders the new
/// snapshot in that same form, because two texts rendered differently differ on lines that did
/// not change: the page's text, every link's url, the cut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Form {
    /// How the page's text was decided, and what that came to.
    pub(crate) text: TextForm,
    /// Each link's `/url` line is printed.
    pub(crate) urls: bool,
    /// The cap the text was cut at.
    pub(crate) max_tokens: MaxTokens,
    /// Inert subtrees are listed (`{ inert: true }`). False and absent are one form: the
    /// TypeScript writes the key only when it is true.
    pub(crate) inert: bool,
}

/// What `text` came to: as the script passed it, or what the length rule chose when it was left
/// out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextForm {
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
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Passed {
    pub(crate) text: Option<bool>,
    pub(crate) urls: Option<bool>,
    pub(crate) max_tokens: Option<MaxTokens>,
    pub(crate) inert: Option<bool>,
}
