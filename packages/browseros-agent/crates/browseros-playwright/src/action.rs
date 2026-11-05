//! Playwright's actions on one element: its two retry levels run on Rust's clock, its
//! actionability checks run in the element's sealed world, and the input is trusted CDP input
//! on the page's root session, sent only after `OpScope::send_effect`.

pub(crate) mod action_op;

pub(crate) use action_op::ActionOp;
