//! What hosted BrowserOS Neo (claw-sdk) remembers about agent work: bounded run and operation
//! records, screenshots and request-id replay history.
//!
//! claw-sdk's dispatcher hands every run and every SDK operation to a [`TraceSink`]. One writer
//! task bounds each event and batches it into a SQLite file. The [`TraceStore`] reads
//! it back for the service API. The store never re-derives delivery: it records what the
//! dispatcher decided, so the agent's error and the record cannot disagree.
//!
//! ```text
//!  dispatcher ─ op_started/op_ended ─▶ TraceSink ─(queue)─▶ writer task ──▶ trace.db (WAL)
//!      │                                                    │  Redactor      ▲
//!      │                                                    ├─▶ broadcast    │ TraceStore reads
//!      │                                                    └─ ShotJob ─▶ camera (service)
//!      │                                     ShotAnswer ◀── writes screenshots/<run>/<seq>.jpg
//!  run service, at a run's end ─▶ RunOutputs ─▶ outputs/<run>/ (kept downloads and reply
//!                                                blobs), beside trace.db
//! ```
//!
//! The writer decides which operations get a screenshot and when a run is `complete`; the
//! camera that takes the pictures lives in `neo-service`, so this crate makes no browser
//! calls.
//!
//! claw-sdk is stateless. Records hang off the browser's tab id
//! (`browseros_core::TabId`) and the host's optional `requestId` and `taskId`. The crate takes
//! only that id from `browseros-core` and has no dispatcher or HTTP types, so it builds and
//! tests on its own. claw-sdk maps its dispatcher's types onto the record types here
//! one-to-one.

#![deny(missing_docs)]
#![deny(
    clippy::wildcard_imports,
    clippy::missing_panics_doc,
    clippy::needless_for_each,
    clippy::fn_params_excessive_bools,
    clippy::string_add
)]

mod blob_id;
mod epoch_millis;
mod error_record;
mod ids;
mod json_pointer;
mod operation_record;
mod read_model;
mod redactor;
mod run_download;
mod run_outputs;
mod run_record;
mod salvage;
mod screenshot_files;
mod screenshot_queue;
mod screenshot_rule;
mod storage;
mod trace_error;
mod trace_store;
mod trace_writer;

pub use blob_id::{BlobId, NotABlobId};
pub use epoch_millis::EpochMillis;
pub use error_record::{ErrorOwner, ErrorRecord};
pub use ids::{IdError, OperationId, RequestId, RunId, TaskId};
pub use json_pointer::{JsonPointer, PointerError};
pub use operation_record::{
    Delivery, Effect, OperationEnded, OperationKind, OperationReached, OperationStarted, Outcome,
    PageState,
};
pub use read_model::{
    OperationRow, Page, RunDetail, RunQuery, RunSummary, ShotRead, ShotState, TailQuery, TaskAct,
    TraceTail,
};
pub use redactor::{Bounded, RedactionLimits, Redactor, Truncation};
pub use run_download::{DownloadStatus, RunDownload};
pub use run_outputs::{KeptFile, RunOutputs};
pub use run_record::{RunEffects, RunEnded, RunStarted, RunStatus};
pub use salvage::{Loss, Recovery, Salvage};
pub use screenshot_queue::{Shot, ShotAnswer, ShotJob, ShotJobs, ShotQueue, screenshot_queue};
pub use trace_error::TraceError;
pub use trace_store::{RetentionPolicy, Swept, TraceStore};
pub use trace_writer::{TraceRecordEvent, TraceSink, WriterConfig, spawn_writer};
