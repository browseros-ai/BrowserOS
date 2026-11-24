use crate::cap::MaxTokens;
use crate::delta::Earlier;
use crate::form::Passed;

/// Which text [`render`](crate::render) prints: one variant per row of today's
/// `Frame._ariaSnapshot` (neo-playwright-js `prelude/frame.ts`). The variant carries what only
/// the snapshot operation knows: what was snapshotted (a page or one element) and the options
/// the script passed, each left out as the script left it out.
#[derive(Clone, Copy, Debug)]
pub enum Call<'a> {
    /// Playwright's own text: `mode: 'default'`, and every mode under the conformance switch.
    /// Upstream's lines for every node, each `# partial:` comment under its iframe's line, and
    /// nothing else: no head line, no cap. An iframe keeps its fallback text, as upstream prints
    /// it.
    Playwright,
    /// `mode: 'full'`: Playwright's lines for every node of the marked tree under Neo's head
    /// lines, an iframe's fallback text dropped so its frame prints, cut at `max_tokens`. A cap
    /// the script left out stays `None`, and the crate applies
    /// [`DEFAULT_MAX_TOKENS`](crate::DEFAULT_MAX_TOKENS) itself.
    Full {
        /// The `maxTokens` the script passed, if any.
        max_tokens: Option<MaxTokens>,
    },
    /// Neo's default format for one element (a locator). Its text never becomes a baseline.
    Element(&'a Passed),
    /// Neo's default format for a whole page. Its text, in its own form, may become the tab's
    /// baseline: the answer's [`pending`](crate::Answer::pending) lends both.
    Page(&'a Passed),
    /// A whole page with `{ delta: true }`: what changed since the text the tab's store holds,
    /// as the snapshot operation read it under the call's clock. Only [`Earlier`] says what there
    /// is to compare with, so a text pending in this run can never be compared. Its answer keeps
    /// the page for the tab unless the stored text was not read.
    Delta {
        /// The options the script passed; what it left out is taken from the stored form.
        passed: &'a Passed,
        /// What the tab held when the call looked.
        earlier: Earlier<'a>,
    },
}
