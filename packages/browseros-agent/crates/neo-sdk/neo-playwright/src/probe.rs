//! The probe protocol: the one contract between this crate and Neo's probe in each frame's
//! isolated `browseros-engine` world.
//!
//! ```text
//!  Runtime.callFunctionOn {
//!    objectId: <the world's global, held since install>,   // never executionContextId: see probe_call
//!    functionDeclaration: CALL_FUNCTION,
//!    arguments: [{ value: <ProbeRequest> }], returnByValue: true }
//!        └─▶ globalThis.__neoProbe.call(request) ─▶ <ProbeReply>, or { kind: "uninstalled" }
//! ```
//!
//! The TypeScript source of the shapes is `crates/neo-sdk/neo-playwright-js/src/probe/
//! protocol.ts`, and neither side generates the other. The JS build writes a typed example of
//! every request and reply to `tests/data/probe-protocol.json`, and the tests here decode each
//! one, so a shape that changes on one side fails the other's build. The probe owns three
//! things in its world: the pin table, the armed hit-target interceptor and the ref map.

mod probe_call;
mod probe_reply;
mod probe_request;

pub(crate) use probe_call::{
    NAVIGATED_OR_CLOSED, ProbeFailure, Ran, call, call_on, call_with_gesture, pinned_object,
    reply_of,
};
pub(crate) use probe_reply::{
    ErrorClass, Hit, PROBE_PROTOCOL, PageText, Point, ProbeReply, Rect, Resolution, SnapshotIframe,
};
pub(crate) use probe_request::{
    ArmPlan, DocumentPart, DropEntry, Edit, ElementState, ExpectParams, FilePayload, FillValue,
    HitAction, ParsedSelector, ProbeRequest, ProbeTarget, QueryState, Read, ReadAll, Retarget,
    ScrollPlan, SelectOption, SnapshotMode, StorageAction, StorageKind, UrlPattern, Want,
};
