//! Outbound failure reporting.
//!
//! Everything here exists to answer one question, why a `run` script failed, while
//! sending nothing about what the user was doing. Both modules fail closed: the
//! fingerprint keeps structure and discards every literal, the allowlist keeps a
//! recognised error class and discards anything it does not recognise.

pub mod drift_guards;
pub mod error_allowlist;
pub mod script_fingerprint;
