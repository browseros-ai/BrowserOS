//! Playwright's retrying `expect` matchers: one poll at once, then the outer backoff on
//! Rust's clock until the matcher holds or the op's timeout passes.

pub(crate) mod expect_op;

pub(crate) use expect_op::ExpectOp;
