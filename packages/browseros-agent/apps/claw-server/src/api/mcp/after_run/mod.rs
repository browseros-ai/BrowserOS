//! Host projections run after the SDK reply is parsed and before the audit row.
//! Each module owns one projection so subsequent chunks can fill them independently.

pub(super) mod naming;
pub(super) mod tabs;
