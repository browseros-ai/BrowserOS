//! Playwright's reads of elements: resolved and read in one isolated-world call. Reads never mark
//! their op sent.
//!
//! ```text
//!  read_op ── decodes the op's input, then
//!    ├─ element_read    one element: waits for a locator's on the outer backoff, reads a
//!    │                  handle's node once
//!    │    └─ viewport_box   a box in the element's own frame, plus each parent iframe's
//!    │                      content origin, is the box in the main frame's viewport
//!    ├─ selector_wait   waitFor and waitForSelector: polls until the first match is in a state
//!    ├─ element_wait    waitForElementState: retries a handle's node until it is in a state
//!    ├─ element_handle  a pinned match ─ DOM.describeNode ─▶ {"$neo":"element", seq, node}
//!    ├─ every match     count, allTextContents, allInnerTexts: now, without waiting
//!    └─ normalize       the generator's selector for the first match, frames included
//! ```

#[cfg(test)]
mod alignment_tests;
mod element_handle;
mod element_read;
mod element_wait;
mod normalize;
pub(crate) mod read_op;
mod selector_wait;
mod viewport_box;

// Read runners own these value payloads; the probe generator shares their declarations.
pub(crate) use element_handle::{Located, PinnedMatch, every_match, find_one};
pub(crate) use element_read::read_attached;
pub(crate) use read_op::ReadOp;
pub(crate) use selector_wait::{Returning, SelectorState, wait_for_match, wait_hidden};
pub(crate) use viewport_box::{FrameOrigin, for_screenshot};
