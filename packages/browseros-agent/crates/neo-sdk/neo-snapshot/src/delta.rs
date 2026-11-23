//! The delta: what changed on a whole page since the last snapshot of its tab that the caller was
//! given, as a line diff of the two texts in Neo's default format. Ported from the TypeScript
//! renderer's `snapshot-delta.ts`, whose output it must equal byte for byte.
//!
//! ```text
//!  Snapshot, Passed, Earlier ─▶ delta_text: which first line, and in which form
//!     │ NotRead: no time was left        the whole page by the options passed; nothing kept
//!     │ Nothing: no earlier snapshot     the whole page; it is kept in its own form
//!     │ Text: the stored text and form   the page rendered in that form (neo_text_as), when every
//!     ▼                                  option passed agrees with it, else the whole page
//!  list_changes: the read and size lines left out of both texts
//!     ▼
//!  line_diff: the common head and tail, a bound before the search, then Myers' search for the
//!     │       shortest edit script, at most 1,000 changed lines
//!     ▼
//!  hunks: three lines of context, each hunk named by the block it is in
//!     ▼
//!  the first line, the on-top lines, the `# refs missing:` line, then the hunks; or `# whole
//!  page: too much differs` and the page when the changes would take more than half of it
//! ```
//!
//! The store and the clock are the caller's: [`Earlier`] says what the tab held when the call
//! looked, and [`DeltaText`] what it may keep, which `render` lends through the answer's
//! `pending()`. The search's tie-break decides the shape of every
//! hunk, so it is kept exactly as the TypeScript has it, in the search and in the walk back.

mod delta_text;
mod earlier;
mod hunks;
mod line_diff;
mod list_changes;
#[cfg(test)]
mod tests;

pub(crate) use delta_text::{DeltaText, delta_text};
pub use earlier::{Baseline, Earlier};
