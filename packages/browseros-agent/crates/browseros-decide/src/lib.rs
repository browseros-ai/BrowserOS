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
