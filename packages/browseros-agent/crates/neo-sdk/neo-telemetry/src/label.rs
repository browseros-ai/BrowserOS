//! The closed vocabularies Neo's metrics are labelled with.
//!
//! Every metric label value comes from one of these enums or from the server's own tables
//! ([`crate::Vocabulary`]), so no caller can widen a series set: there is no `String` label
//! anywhere in the facade. Each type's `as_str` (an [`HttpStatus`]'s `code`) is its wire spelling,
//! the same in Prometheus text, OTLP attributes and span attributes. The server maps its own types (`neo_trace`'s
//! status, outcome, delivery and owner; its tab registry's origin; neo-captcha's solve outcome)
//! onto these with exhaustive matches, so a new server variant forces a decision about its label.

mod captcha_family;
mod captcha_provider;
mod captcha_reason;
mod captcha_verdict;
mod close_reason;
mod delivery;
mod error_code;
mod error_label;
mod error_owner;
mod http_status;
mod net_error;
mod op_outcome;
mod run_status;
mod tab_origin;

pub use captcha_family::CaptchaFamily;
pub use captcha_provider::CaptchaProvider;
pub use captcha_reason::CaptchaReason;
pub use captcha_verdict::CaptchaVerdict;
pub use close_reason::CloseReason;
pub use delivery::Delivery;
pub use error_code::ErrorCode;
pub use error_label::ErrorLabel;
pub use error_owner::ErrorOwner;
pub use http_status::HttpStatus;
pub use net_error::NetError;
pub use op_outcome::OpOutcome;
pub use run_status::RunStatus;
pub use tab_origin::TabOrigin;
