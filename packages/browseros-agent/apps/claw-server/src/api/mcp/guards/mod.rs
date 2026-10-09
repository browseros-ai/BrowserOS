//! Guards refuse a dispatch outright.
//!
//! Page ownership is deliberately NOT among them, and must never be added back.
//! Ownership is a label that tells an agent whose tab it is looking at; it is not a
//! permission. See `effects::page_ownership_notice`.

pub mod browser_connected;
pub mod navigate_scheme;
