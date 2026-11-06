//! Playwright's reads of elements: resolved and read in one sealed-world call, so field values
//! come back masked at the source. The seal guards each field, not the text around it, so every
//! string a read returns, a preview or a strict violation's message included, is masked again
//! with the values of every frame of the read frame's site, swept alongside the read
//! (`page::site_sweep`), and those values go to the host. Reads never mark their op sent.
//!
//! ```text
//!  read_op ── decodes the op's input, then
//!    ├─ element_read    one element: waits for a locator's on the outer backoff, reads a
//!    │                  handle's node once; marks the op's target sensitive when the seal says so
//!    │    └─ viewport_box   a box in the element's own frame, plus each parent iframe's
//!    │                      content origin, is the box in the main frame's viewport
//!    ├─ selector_wait   waitFor and waitForSelector: polls until the first match is in a state
//!    ├─ element_wait    waitForElementState: retries a handle's node until it is in a state
//!    ├─ element_handle  a pinned match ─ DOM.describeNode ─▶ {"$neo":"element", seq, node}
//!    └─ every match     count, allTextContents, allInnerTexts: now, without waiting
//! ```

mod element_handle;
mod element_read;
mod element_wait;
pub(crate) mod read_op;
mod selector_wait;
mod viewport_box;

pub(crate) use element_handle::{backend_node, every_match, query_one};
pub(crate) use read_op::ReadOp;
pub(crate) use selector_wait::{Returning, SelectorState, wait_for_selector};
