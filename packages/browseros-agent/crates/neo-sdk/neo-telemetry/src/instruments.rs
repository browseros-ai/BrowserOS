//! The 11 foundational instruments, as one table (wire-contract.md, "Metrics").
//!
//! Owned by the metrics-and-spans work: one `catalog!` list (name, kind, unit, help, typed label
//! tuple, buckets) that generates the instruments, their typed record functions and the
//! Prometheus HELP / TYPE lines. Labels come only from `crate::label` and `Vocabulary`'s
//! interned names, through a sealed trait, so no `&str` can reach a `KeyValue`: never a URL, host,
//! title, tab, run or task id. `neo_operations_total` carries `{op, status, error_owner}` and no
//! `error_code`; codes go on `neo_errors_total` alone. `neo_tabs_open` is an observable gauge
//! whose callback reads the open-tabs source, wrapped so a panicking source cannot kill the
//! reader thread.
