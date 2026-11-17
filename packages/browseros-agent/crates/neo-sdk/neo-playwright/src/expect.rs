//! Playwright's retrying `expect` matchers: one poll at once, then the outer backoff on
//! Rust's clock until the matcher holds or the op's timeout passes.
//!
//! ```text
//!  prelude: matcher, Expected line, FrameExpectParams ─▶ expect.locator / expect.page
//!    expect_op     decode, admit the page, then one poll per attempt:
//!                  Resolver (frames, strict mode) ─▶ probe `expect` in the last world
//!    expectation   upstream's rules per expression, the loop, the code of a failure
//!    matcher_message   Playwright Test's failure text and call-log lines
//! ```
//!
//! Every expect op is a read: no poll changes the page, so none calls `send_effect`.

pub(crate) mod expect_op;
mod expectation;
mod matcher_message;

pub(crate) use expect_op::ExpectOp;
