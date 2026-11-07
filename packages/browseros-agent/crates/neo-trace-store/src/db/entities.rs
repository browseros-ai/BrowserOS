//! Neo: One SeaORM entity per table. Every child table's key leads with `run_id` and belongs to its
//! run, so a run's rows are primary-key ranges and reads use `find_related`. The schema itself is
//! the migrations' (`db/migration.rs`); a test holds the two to the same columns.

pub(crate) mod trace_call_artifacts;
pub(crate) mod trace_calls;
pub(crate) mod trace_getters;
pub(crate) mod trace_logs;
pub(crate) mod trace_pages;
pub(crate) mod trace_run_ids;
pub(crate) mod trace_runs;
pub(crate) mod trace_sessions;
