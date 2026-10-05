//! The decision policy behind neo's goal-driven browsing mode.
//!
//! One observation becomes one request: which operation to run, which element
//! to run it on, whether the goal is already satisfied, and how far along the
//! run is. The model answers by choosing among elements the browser already
//! observed, so its output is never a selector, a coordinate or code.
//!
//! The crate deliberately has no browser and no transport of its own, so the
//! logic that decides what to ask and what an answer means is testable without
//! either.

pub mod action;
pub mod answer;
pub mod condense;
pub mod decide;
pub mod drive;
pub mod questions;

pub use action::{ActionSpace, Element, Head, Operation};
pub use answer::{AnswerError, Decision, interpret};
pub use condense::{Condensed, DEFAULT_TREE_BUDGET, condense};
pub use decide::{DecideError, Decider, Step};
pub use drive::{Budget, Driver, Oracle, Outcome, Status, TrailEntry, drive};
pub use questions::{Observation, PastAction, state, state_with_budget};
