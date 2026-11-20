//! The two explicit spans that leave Neo, `neo.run` and `neo.op`, and their attributes
//! (wire-contract.md, "Spans").
//!
//! Owned by the metrics-and-spans work. No automatic bridge from the `tracing` crate: only what
//! this module starts is exported. A run span's parent is the caller's `TraceContext` when one
//! was given; an op span is a child of its run's span, or a root for an op of an internal run.
