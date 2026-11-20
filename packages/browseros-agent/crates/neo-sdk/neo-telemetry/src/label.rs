//! The closed vocabularies Neo's metrics are labelled with.
//!
//! Every metric label value comes from one of these enums or from the server's own tables
//! ([`crate::Vocabulary`]), so no caller can widen a series set: there is no `String` label
//! anywhere in the facade. Each type's `as_str` is its wire spelling, the same in Prometheus
//! text, OTLP attributes and span attributes. The server maps its own types (`neo_trace`'s
//! status, outcome, delivery and owner; its tab registry's origin) onto these with exhaustive
//! matches, so a new server variant forces a decision about its label.

mod close_reason;
mod delivery;
mod error_code;
mod error_label;
mod error_owner;
mod op_outcome;
mod run_status;
mod tab_origin;

pub use close_reason::CloseReason;
pub use delivery::Delivery;
pub use error_code::ErrorCode;
pub use error_label::ErrorLabel;
pub use error_owner::ErrorOwner;
pub use op_outcome::OpOutcome;
pub use run_status::RunStatus;
pub use tab_origin::TabOrigin;
