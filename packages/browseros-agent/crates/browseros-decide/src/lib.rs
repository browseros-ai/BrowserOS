//! The decision layer: typed questions to a System One model, and the code that
//! owns everything around them.
//!
//! The division of labour is the one the model's own documentation prescribes:
//! code owns the control flow, the candidate set and every threshold, and the
//! model answers narrow closed-set questions about a small state.

pub mod client;
pub mod relevance;
pub mod space;
pub mod view;

pub use client::{Answer, Jev, JevError, Question, Response, Usage};
pub use view::{Control, ControlState, PageView};
