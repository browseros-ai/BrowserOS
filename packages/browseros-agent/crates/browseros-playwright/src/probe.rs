//! The probe protocol: the one contract between this crate and Neo's probe in each frame's
//! sealed `browseros-sdk` world.
//!
//! ```text
//!  Runtime.callFunctionOn {
//!    objectId: <the world's global, held since install>,   // never executionContextId: see probe_call
//!    functionDeclaration: CALL_FUNCTION,
//!    arguments: [{ value: <ProbeRequest> }], returnByValue: true }
//!        └─▶ globalThis.__neoProbe.call(request) ─▶ <ProbeReply>, or { kind: "uninstalled" }
//! ```
//!
//! The TypeScript source of the shapes is `packages/browseros-playwright-js/src/probe/
//! protocol.ts`, and neither side generates the other. The JS build writes a typed example of
//! every request and reply to `tests/data/probe-protocol.json`, and the tests here decode each
//! one, so a shape that changes on one side fails the other's build. The probe owns three
//! things in its world: the pin table, the armed hit-target interceptor and the ref map.
//!
//! A request whose reply holds page text that Playwright's engine cuts or quotes (a preview,
//! a strict violation's hints) needs its frame's site's values first, as a [`ProbeMask`]: the
//! probe masks the text with them before the engine shortens it, since a value cut short no
//! longer matches Rust's mask of whole values. Without one it answers `needsMask`.

mod probe_call;
mod probe_reply;
mod probe_request;

pub(crate) use probe_call::{
    NAVIGATED_OR_CLOSED, ProbeFailure, ProbeMask, Ran, call, call_masked, call_on,
    call_with_gesture, pinned_object, reply_of,
};
pub(crate) use probe_reply::{
    ErrorClass, Hit, PROBE_PROTOCOL, PageText, Point, ProbeReply, Rect, SnapshotIframe,
};
pub(crate) use probe_request::{
    ArmPlan, DocumentPart, Edit, ElementState, ExpectParams, FilePayload, FillValue, HitAction,
    ParsedSelector, ProbeRequest, ProbeTarget, QueryState, Read, ReadAll, Retarget, ScrollPlan,
    SelectOption, SnapshotMode, UrlPattern, Want,
};
