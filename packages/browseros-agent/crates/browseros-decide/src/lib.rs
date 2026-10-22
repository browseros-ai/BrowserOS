//! Driving a browser with a decision model.
//!
//! The model this crate talks to cannot generate text. It answers typed
//! questions: a probability, one option from a set we author, or a position on
//! a scale. Everything here follows from that one constraint.
//!
//! The layout mirrors one decision:
//!
//! ```text
//!   view        the page, as a decision needs to see it
//!   space       which controls are offered, decided in code
//!   questions   one request, many questions, evaluated in parallel
//!   client      the endpoint
//!   gate        whether the answer is sure enough to act on
//! ```

pub mod client;
pub mod drive;
pub mod gate;
pub mod kind;
pub mod offer;
pub mod pursue;
pub mod questions;
pub mod relevance;
pub mod report;
pub mod run;
pub mod space;
pub mod view;
pub mod vocabulary;
