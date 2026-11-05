//! What hosted BrowserOS Neo (claw-sdk) remembers about agent work, and what it refuses to
//! remember: run and operation records, the one process-wide secret set, and pixel privacy.
//!
//! claw-sdk's dispatcher hands every run and every SDK operation to a [`TraceSink`]. One writer
//! task masks and bounds each event and batches it into a SQLite file. The [`TraceStore`] reads
//! it back for the service API. The store never re-derives delivery: it records what the
//! dispatcher decided, so the agent's error and the record cannot disagree.
//!
//! ```text
//!  dispatcher ─ op_started/op_ended ─▶ TraceSink ─(queue)─▶ writer task ──▶ trace.db (WAL)
//!      │                                                    │  Redactor      ▲
//!      │                                                    └─▶ broadcast    │ TraceStore reads
//!      ├─ Masker ──────────────▶ SecretSet ◀── protected fill, /host/v1/secrets, takeover
//!      └─ SecretInput, takeover ▶ Privacy ── make_private ─▶ writer (durable) ─┘
//!                                    └── CaptureGuard ◀── every screenshot and frame
//! ```
//!
//! The crate has no dispatcher, browser or HTTP types, so it builds and tests on its own. claw-sdk
//! maps its dispatcher's types onto the record types here one-to-one.

#![deny(missing_docs)]
#![deny(
    clippy::wildcard_imports,
    clippy::missing_panics_doc,
    clippy::needless_for_each,
    clippy::fn_params_excessive_bools,
    clippy::string_add
)]

mod epoch_millis;
mod error_record;
mod ids;
mod json_pointer;
mod operation_record;
mod privacy;
mod read_model;
mod redactor;
mod run_record;
mod secret_set;
mod trace_error;
mod trace_store;
mod trace_writer;

pub use epoch_millis::EpochMillis;
pub use error_record::{ErrorOwner, ErrorRecord};
pub use ids::{IdError, OperationId, RunId, SessionId};
pub use json_pointer::{JsonPointer, PointerError};
pub use operation_record::{
    Delivery, Effect, OperationEnded, OperationKind, OperationStarted, Outcome, PageState,
    RedactionPolicy, TargetSensitivity,
};
pub use privacy::{
    CaptureGuard, ControlDisplay, DocumentKey, PageKey, Privacy, PrivacyError, PrivacyRecord,
    PrivacyStore, PrivateReason,
};
pub use read_model::{
    OperationRow, Page, RunDetail, RunQuery, RunSummary, SessionDetail, SessionQuery, SessionState,
    SessionSummary, SessionTrace,
};
pub use redactor::{Bounded, RedactionLimits, Redactor, Truncation};
pub use run_record::{RunEffects, RunEnded, RunStarted, RunStatus, SessionEnded, SessionStarted};
pub use secret_set::{Registration, SecretScope, SecretSet, SecretSetConfig, SecretValue};
pub use trace_error::TraceError;
pub use trace_store::{RetentionPolicy, Swept, TraceStore};
pub use trace_writer::{TraceRecordEvent, TraceSink, WriterConfig, spawn_writer};
