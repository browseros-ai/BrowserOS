//! Spike only (branch `feat/chromium-tree-spike`, never merged): `page.desktopSnapshot`, desktop
//! Neo's own snapshot of a hosted tab, with each of its refs handed back as a Playwright element
//! handle, so a script can act on a desktop ref with Playwright's own actions.
//!
//! ```text
//!  BrowserView::observe(page).snapshot_with_options(mode)   browseros-core, called as it is:
//!    Chromium's accessibility tree and DOM snapshot per frame target, rendered in Rust
//!    → text, RefMap { ref → (frame id, backend node id, role, name) }
//!  with `handles`, per ref:
//!    core's frame registry: the frame's CDP session
//!    hosted's world for that frame (installed on first use, as a `frame.*` form's is)
//!    → { $neo: 'element', page, seq: that world's document, node: the backend node id }
//!  a later `handle.click()` / `handle.fill()` resolves the node into that isolated world with
//!  DOM.resolveNode (resolve/handle_site.rs) and runs Playwright's own action there
//! ```
//!
//! Nothing here runs in the page's own world: core's capture sends no `Runtime` call, and the
//! only script is the probe's own install in Neo's isolated world. Core's `resolve_ref` is not
//! used, because its liveness check resolves the node into the page's main world.

use std::collections::HashMap;
use std::time::Instant;

use browseros_core::snapshot::{SnapshotMode as CoreMode, SnapshotOptions};
use browseros_core::{FrameId, ProtocolSession};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::spike_cdp_meter::{self, MethodBytes};
use crate::target::{ElementTarget, PageRef, RootHandle};
use crate::world::{BackendNodeId, Endpoint, FrameSeq, Place};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// `page.desktopSnapshot`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "DesktopSnapshotInput")]
pub(super) struct DesktopInput {
    page: PageRef,
    /// Desktop's own modes: `full` (its default) or `interactive`.
    #[serde(default)]
    #[ts(optional)]
    mode: Option<String>,
    /// Hand back an element handle for every ref.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    handles: bool,
    /// Count the CDP bytes of the capture (`spike_cdp_meter`).
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    meter: bool,
    /// Leave the text and the refs out of the result, keeping their sizes: for the cost runs on
    /// large pages, so the script's heap does not decide what is measured.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    brief: bool,
}
crate::wire::options!(DesktopInput);

/// What desktop's snapshot printed, its refs, and what it cost.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "DesktopSnapshot")]
pub(super) struct DesktopSnapshot {
    text: String,
    /// The text's length in UTF-16 units, as `String.length` counts it in the script.
    text_units: usize,
    /// How many refs core's map holds.
    ref_count: usize,
    url: String,
    /// Wall time of core's capture and render, in milliseconds.
    capture_ms: f64,
    /// Wall time of turning the refs into handles (world installs and lookups), in ms.
    bind_ms: f64,
    /// Core's frame errors, each as its JSON.
    frame_errors: Vec<String>,
    /// CDP bytes by method, when `meter` was set.
    cdp: Vec<MethodBytes>,
    refs: Vec<DesktopRef>,
}

/// One ref of desktop's map, in the order the text prints them.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "DesktopRef")]
struct DesktopRef {
    #[serde(rename = "ref")]
    ref_id: String,
    role: String,
    name: String,
    /// Core's frame id, `None` for the main frame.
    frame: Option<String>,
    node: i64,
    /// Hosted's document seq for the ref's frame, when a handle was made.
    seq: Option<u32>,
    #[ts(type = "unknown")]
    handle: Option<RootHandle>,
    /// Why no handle was made, when `handles` was set.
    unbound: Option<String>,
}

/// Runs desktop's snapshot on the admitted tab.
pub(super) async fn desktop_snapshot(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: DesktopInput,
) -> Result<DesktopSnapshot, PlaywrightError> {
    let page = OpPage::admit(pw, scope, input.page.tab()).await?;
    let mode = match input.mode.as_deref() {
        None | Some("full") => CoreMode::Full,
        Some("interactive") => CoreMode::Interactive,
        Some(other) => {
            return Err(scope.fail(
                ErrorCode::InvalidInput,
                &format!("mode must be 'full' or 'interactive', not {other:?}"),
            ));
        }
    };
    let observer = scope
        .view()
        .observe(page.core_page().clone())
        .await
        .map_err(|error| scope.core_error(&error))?;
    if input.meter {
        spike_cdp_meter::start();
    }
    let started = Instant::now();
    let captured = observer
        .snapshot_with_options(SnapshotOptions { mode, depth: None })
        .await;
    let capture_ms = started.elapsed().as_secs_f64() * 1000.0;
    let cdp = if input.meter {
        spike_cdp_meter::take()
    } else {
        Vec::new()
    };
    let result = captured.map_err(|error| scope.core_error(&error))?;

    let bind_started = Instant::now();
    let mut seqs: HashMap<Option<FrameId>, Result<FrameSeq, String>> = HashMap::new();
    let mut refs = Vec::new();
    for entry in result.refs.entries_in_order() {
        let (seq, unbound) = if input.handles {
            if !seqs.contains_key(&entry.frame_id) {
                let seq = frame_seq(&page, entry.frame_id.clone()).await;
                seqs.insert(entry.frame_id.clone(), seq);
            }
            match seqs.get(&entry.frame_id) {
                Some(Ok(seq)) => (Some(*seq), None),
                Some(Err(why)) => (None, Some(why.clone())),
                None => (None, Some("no frame lookup".to_owned())),
            }
        } else {
            (None, None)
        };
        let handle = seq.map(|seq| {
            RootHandle::Element(ElementTarget {
                display: format!("{} \"{}\"", entry.role, entry.name),
                node: BackendNodeId(entry.backend_node_id),
                page: input.page.clone(),
                seq,
            })
        });
        refs.push(DesktopRef {
            ref_id: entry.ref_id.as_str().to_owned(),
            role: entry.role.clone(),
            name: entry.name.clone(),
            frame: entry.frame_id.as_ref().map(|frame| frame.0.clone()),
            node: entry.backend_node_id,
            seq: seq.map(|seq| seq.0),
            handle,
            unbound,
        });
    }
    let bind_ms = bind_started.elapsed().as_secs_f64() * 1000.0;
    let text_units = result.text.encode_utf16().count();
    let ref_count = refs.len();
    if input.brief {
        refs.clear();
    }
    Ok(DesktopSnapshot {
        text: if input.brief { String::new() } else { result.text },
        text_units,
        ref_count,
        url: result.url,
        capture_ms,
        bind_ms,
        frame_errors: result
            .coverage
            .frame_errors
            .iter()
            .map(|error| serde_json::to_string(error).unwrap_or_default())
            .collect(),
        cdp,
        refs,
    })
}

/// Hosted's document seq for desktop's frame `frame` (`None`: the main frame): core's frame
/// registry finds the frame's CDP session, and hosted installs (or finds) its isolated world
/// there, as it does for a frame a `frame.*` form names. The error is the reason, for the result.
async fn frame_seq(page: &OpPage<'_>, frame: Option<FrameId>) -> Result<FrameSeq, String> {
    let main = page.main_endpoint();
    let (endpoint, place) = match frame {
        None => (main, Place::Main),
        Some(frame) if frame == main.frame => (main, Place::Main),
        Some(frame) => {
            let target = page
                .browser()
                .frames()
                .resolve_frame_target(page.core_page().clone(), Some(frame.clone()), None, false)
                .await
                .map_err(|error| format!("frame session: {error}"))?;
            let session: ProtocolSession = target.session;
            (Endpoint { session, frame }, Place::Descendant)
        }
    };
    page.world(&endpoint, place)
        .await
        .map(|world| world.seq())
        .map_err(|failure| format!("world: {failure:?}"))
}
