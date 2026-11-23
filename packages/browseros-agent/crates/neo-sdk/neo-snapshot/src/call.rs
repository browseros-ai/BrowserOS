use crate::cap::MaxTokens;
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
}
