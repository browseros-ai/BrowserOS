//! Neo's default format: the text `ariaSnapshot()` returns by default, a short list of what an
//! agent can act on, in Playwright's own line syntax and with Playwright's own refs. Ported from
//! the TypeScript renderer's `snapshot-v1.ts`, `snapshot-on-top.ts` and `snapshot-unmarked.ts`,
//! whose output it must equal byte for byte.
//!
//! ```text
//!  Snapshot, Passed ─▶ neo_text: the options' defaults, the length rule, the head lines
//!                         │ one walk per form (text on, text off)
//!                         ▼
//!                      walk ─▶ OnTop: what a whole-view layer holds, printed first
//!                         │      Printer: an explicit step stack, one Rule per child, inert
//!                         │      subtrees counted on printers of their own
//!                         ▼
//!                      node_text (playwright/) prints every element line
//!                         │
//!                         ▼
//!                      cap: the cut, the kept blocks after it ─▶ the text and its Form
//! ```
//!
//! Each element line is upstream's printer's line for one node without its children (`NodeAlone`,
//! built by `node_line`), so keys, attribute order, quoting and escaping stay Playwright's. This
//! module only decides which nodes print, which refs and marks they keep, and where the page's
//! text goes:
//!
//! ```text
//!  kept block   dialog, alertdialog, alert, status   its line, then all of its text; a dialog's
//!                                                    ref, an alert's only when actionable
//!  section      main, navigation, form, iframe, …    its line with its ref, content indented
//!  heading                                           its line, no ref
//!  control      a ref, and an interactive role or a pointer cursor; a field without a ref
//!  text         `- text:` lines, in the text form only; none that holds only separators
//!  other        no line; its content moves up to the level it sits in
//! ```
//!
//! No walk recurses: each keeps its own stack, as the TypeScript does, so a tree of any depth
//! prints. Where the TypeScript relied on JavaScript's object identity (which nodes a layer
//! holds, which line an omission follows), the Rust compares node addresses in the borrowed tree
//! and a serial number per line.

mod neo_text;
mod node_line;
mod on_top;
mod own_text;
mod printer;
mod rule;
#[cfg(test)]
mod tests;
mod unmarked_head;
mod walk;

pub use neo_text::TEXT_LIMIT_TOKENS;
pub(crate) use neo_text::{NeoText, neo_text, neo_text_as};
pub(crate) use on_top::{is_on_top_line, on_top_delta, on_top_head};
pub use rule::KEPT_ROLES;
pub(crate) use rule::{SECTION, section_label, shown_text};
pub(crate) use unmarked_head::{is_unmarked_line, unmarked_head};
