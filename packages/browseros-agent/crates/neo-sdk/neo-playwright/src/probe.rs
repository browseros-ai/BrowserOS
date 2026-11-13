//! The probe protocol: the one contract between this crate and Neo's probe in each frame's
//! isolated `browseros-engine` world.
//!
//! ```text
//!  Runtime.callFunctionOn {
//!    objectId: <the world's global, held since install>,   // never executionContextId: see probe_call
//!    functionDeclaration: CALL_FUNCTION,
//!    arguments: [{ value: <ProbeCall> }, root?, ...nodes?], returnByValue: true }
//!        └─▶ globalThis.__neoProbe.call(call, root, nodes) ─▶ <ProbeReply>,
//!            or { kind: "uninstalled" }
//! ```
//!
//! Rust's request/reply types generate `neo-playwright-js/src/probe/generated.ts` via
//! `bun run gen:probe`; a compare-only test refuses stale declarations. Typed examples still
//! build `tests/data/probe-protocol.json` and both sides decode it, guarding runtime behavior
//! independently of generation. The probe owns the pin table, armed interceptor and ref map.

mod probe_call;
mod probe_reply;
mod probe_request;
pub(crate) mod protocol;

#[cfg(test)]
pub(crate) use probe_call::call_for_tests;

pub(crate) use probe_call::{
    Control, NAVIGATED_OR_CLOSED, ProbeFailure, Ran, call, call_on, call_with_gesture,
    call_with_nodes, cleanup, picture_question, pinned_nodes, pinned_object, reply_of,
};
pub(crate) use probe_reply::{
    ErrorClass, Hit, PROBE_PROTOCOL, PageText, Point, ProbeReply, Rect, Resolution, SnapshotIframe,
};
pub(crate) use probe_request::{
    ArmPlan, BootOptions, DocumentPart, DropEntry, Edit, ElementState, ExpectParams, FilePayload,
    FillValue, HitAction, ParsedSelector, ProbePolicy, ProbeRequest, ProbeTarget, QueryState, Read,
    ReadAll, Retarget, ScrollPlan, SelectOption, SnapshotMode, StorageAction, StorageKind,
    UrlPattern, Want,
};
