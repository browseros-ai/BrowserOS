//! One sealed world, and the install that makes it. One of the three files in core's
//! `tests/page_js_guard.rs` ALLOWED list: it creates the `browseros-sdk` world and runs the
//! seal and the engine there, and hands the seal the hidden shadow roots it must watch.

use std::collections::HashSet;
use std::mem;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::sensitive_fields::SEALED_WORLD;
use browseros_core::{CoreError, FrameId, ProtocolSession, SessionId};
use futures_util::future;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::time::{self, Instant};

use crate::page::FRAME_BUDGET;
use crate::probe::{self, PROBE_PROTOCOL, ProbeFailure, ProbeReply, Ran};
use crate::world::engine_source::install_function;
use crate::world::hidden_roots::{self, HiddenRoots, WatchedRoots};
use crate::world::{BackendNodeId, ContextId, Epoch, FrameSeq};

/// The object group of every world's handles, its global's and its document's. Never released:
/// they die with their context, and an op's own group (`OpScope`) holds only the objects that
/// op asked for.
const WORLD_GROUP: &str = "browseros-sdk-world";

/// Names the world's `document` by handle, for the hidden-root scan's describe.
const DOCUMENT: &str = "function () { return document; }";

/// Hands hidden shadow roots to the seal, which journals and sweeps each from then on. `false`
/// when the world holds no seal: it is not the world this handle was for.
const WATCH_ROOTS: &str = "function (...roots) { const seal = globalThis.__clawSeal; \
                           if (!seal) return false; for (const root of roots) seal.watch(root); \
                           return true; }";

/// Takes the oldest shadow host the seal's journal reported (`HIDDEN_ROOT_REPORT`), or null. A
/// seal that predates reports has no `takeHost` and has reported nothing.
const TAKE_HOST: &str = "function () { const seal = globalThis.__clawSeal; \
                         return seal && seal.takeHost ? seal.takeHost() : null; }";

/// How many reported hosts one watch takes at most: the seal holds no more.
const MAX_TAKEN: usize = 16;

/// The tops of the trees the seal watches out of the document (`detachedTops`), by reference.
const DETACHED_TOPS: &str = "function () { const seal = globalThis.__clawSeal; \
                             return seal && seal.detachedTops ? seal.detachedTops() : []; }";

/// The prefix of a watch's own object group ([`WatchHold`]); each watch takes a new number, so
/// two at once never release each other's handles.
const WATCH_GROUP: &str = "browseros-sdk-watch";

/// The next watch's number.
static NEXT_WATCH: AtomicU64 = AtomicU64::new(0);

/// How `DOM.resolveNode` refuses a backend id whose node is gone (measured).
const NODE_GONE: &str = "No node with given id found";

/// How `Runtime.evaluate` refuses a `uniqueContextId` that names no live context (measured on
/// BrowserOS: an unknown id, and a world whose document moved to another renderer).
const UNIQUE_CONTEXT_GONE: &str = "uniqueContextId not found";

/// A live sealed world: where a probe call goes. Only [`SealedWorld::install`] makes one, so
/// holding one means the seal and the engine ran in that world, and the seal watched every
/// hidden shadow root the document had then. Cheap to clone; clones share the record of the
/// roots watched. It may go stale at any time (the document navigates); a call on it then
/// fails `Gone`.
#[derive(Clone, Debug)]
pub(crate) struct SealedWorld {
    /// A raw session, without per-message admission: the page's own, an OOPIF's, or for a
    /// same-process child frame its nearest target ancestor's.
    session: ProtocolSession,
    frame: FrameId,
    seq: FrameSeq,
    /// The remote object id of the world's global object, in [`WORLD_GROUP`]. Calls go
    /// through it rather than a context id; `probe_call` says why.
    global: String,
    /// The world's execution context id, kept for one CDP method that takes nothing else:
    /// `DOM.resolveNode`, which wraps an element handle's node in this world. Its result is
    /// only ever passed as an argument to a call on [`SealedWorld::global`], which V8 refuses
    /// across worlds, so an id another context has since taken carries nothing anywhere.
    context: ContextId,
    /// The world's `document` (by handle in [`WORLD_GROUP`]), and the hidden roots its seal
    /// already watches.
    roots: Arc<WatchedRoots>,
}

/// One watch's object group: the hosts it took from the seal and the roots it resolved. They
/// stay alive while it lives, so a root whose host the page removed is still there for the sweep
/// that follows the watch. Dropping it releases the group on a spawned task, as `OpScope`'s
/// object group does, so a watch that the frame budget or the host's cancel drops halfway
/// releases its group too; each retry takes a new one. A release that fails leaves the handles
/// to die with the document.
#[must_use = "the watch's handles are released when this drops"]
#[derive(Debug)]
pub(crate) struct WatchHold {
    session: ProtocolSession,
    /// `browseros-sdk-watch-<n>`: two watches at once never release each other's handles.
    group: String,
}

impl WatchHold {
    fn new(session: ProtocolSession) -> Self {
        let group = format!(
            "{WATCH_GROUP}-{}",
            NEXT_WATCH.fetch_add(1, Ordering::Relaxed)
        );
        Self { session, group }
    }
}

impl Drop for WatchHold {
    fn drop(&mut self) {
        let Ok(spawner) = runtime::Handle::try_current() else {
            return;
        };
        let session = self.session.clone();
        let group = mem::take(&mut self.group);
        drop(spawner.spawn(async move {
            let _released = session
                .send::<_, Value>(
                    "Runtime.releaseObjectGroup",
                    json!({ "objectGroup": group }),
                )
                .await;
        }));
    }
}

/// How much of its document one watch scans for hidden shadow roots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scan {
    /// The whole document, and every host the seal reported: at install and before a sweep.
    Document,
    /// Only the hosts the seal reported, for the sweep a journal report starts.
    Reported,
}

/// What an install leaves: the world, and the seq its boot took over from, if it did.
#[derive(Debug)]
pub(crate) struct Installed {
    pub(crate) world: SealedWorld,
    /// The seq the world's probe held before this boot renumbered it. From another
    /// `Playwright` (a restart's re-key) it names a document that old refs may still print, so
    /// the page must never hand it out again (`FrameWorlds`).
    pub(crate) retired: Option<FrameSeq>,
}

/// The `Runtime` events one install reads about the world it creates. It must be subscribed
/// before `Page.createIsolatedWorld` is sent: the browser reports a new world's
/// `executionContextCreated` before the create's reply (measured on the main frame, a
/// same-process child and an OOPIF), and a broadcast receiver sees only what is sent after it
/// subscribed. Taking one by value is what makes [`SealedWorld::install`] unable to start late.
#[derive(Debug)]
pub(crate) struct ContextWatch {
    events: broadcast::Receiver<CdpEvent>,
}

/// What a session reported about one numeric context id while a watch ran.
#[derive(Debug, PartialEq, Eq)]
enum Reported {
    /// `executionContextCreated` for this frame's `browseros-sdk` world under the id: the
    /// world the create made, with its system-unique id.
    Created(String),
    /// Nothing about the id: the world already existed, so the create made no context and the
    /// browser sent no event (measured: a second create on the same session, and the first
    /// create from a second connection, which is how a restarted claw-sdk meets a world).
    Nothing,
    /// The id may name another context by now: the session cleared its contexts, destroyed
    /// the id, or created a context other than this world under it, which is how a renderer
    /// swap shows; or the watch lagged or closed and may have missed exactly that.
    Churned,
}

/// `Runtime.executionContextCreated`, as far as the install reads it.
#[derive(Deserialize)]
struct ContextCreated {
    context: ContextDescription,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextDescription {
    id: i64,
    #[serde(default)]
    unique_id: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    aux_data: AuxData,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuxData {
    #[serde(default)]
    frame_id: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

/// `Runtime.executionContextDestroyed`, as far as the install reads it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextDestroyed {
    execution_context_id: i64,
}

/// `Page.createIsolatedWorld`'s result.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatedWorld {
    execution_context_id: i64,
}

/// How the install names the world when it takes the handle to its global.
#[derive(Debug)]
enum Named {
    /// The world's `uniqueId`, which no other context in the browser ever carries.
    Unique(String),
    /// The numeric id the create returned, for a world that already existed. Only ever sent
    /// between two watch reads that saw nothing happen to the id (see `install`).
    Number(ContextId),
}

/// `Runtime.evaluate`'s result, by handle.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Evaluated {
    result: Handle,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Handle {
    #[serde(default)]
    object_id: Option<String>,
}

/// A list the seal answered by reference, with CDP's description of it (`Array(n)`).
#[derive(Deserialize)]
struct Listed {
    result: ListHandle,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListHandle {
    #[serde(default)]
    object_id: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// `Runtime.getProperties`' answer: a list's indices and their objects.
#[derive(Deserialize)]
struct Items {
    result: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    name: String,
    #[serde(default)]
    value: Option<Handle>,
}

impl SealedWorld {
    /// Creates (or finds, by name) the frame's `browseros-sdk` world and installs the seal and
    /// the engine there, or re-keys the probe already there. `candidate` is the seq for a new
    /// document; the reply's seq wins, because a world this `Playwright` already booted keeps
    /// the one it has, and a re-key reports the seq it replaced. The session's journal binding
    /// must already exist (`SessionSetup`).
    ///
    /// ```text
    ///  watch (subscribed by the caller)                       Runtime events of the session
    ///  Page.createIsolatedWorld { frameId, worldName }        → a numeric id N
    ///  read the watch about N:
    ///    created  { id N, frameId, isolated, browseros-sdk, uniqueId U }   a new world
    ///      Runtime.evaluate { uniqueContextId: U, "globalThis", group }
    ///    nothing  (the world already existed: no event)
    ///      Runtime.evaluate { contextId: N, "globalThis", group }, then read again: nothing
    ///    churned  → Gone
    ///  Runtime.callFunctionOn { handle, install_function,     seal, engine if absent, boot
    ///                           [{ frameSeq, epoch }] }       → described { protocol, seq, url }
    ///  Runtime.callFunctionOn { handle, document }            → the document's handle
    ///  watch_hidden_roots                                     the seal watches each closed root
    /// ```
    ///
    /// The last step is what makes the journal hear input inside a closed root from the world's
    /// first moment, a value the page sets with its own events included. A scan that fails
    /// fails the install: a world whose closed roots the seal cannot see is not sealed. So does
    /// one that is not done within the frame's budget from the install's start (`FRAME_BUDGET`):
    /// it fails `Late`, which refuses the op, as a sweep past the budget refuses it. A first read
    /// installs its world outside any sweep, so this deadline is the read's; inside a sweep,
    /// the sweep's own deadline started first and ends it sooner.
    ///
    /// N alone is not enough to take the handle. It is a per-renderer counter: measured on
    /// BrowserOS, when the document moves to another renderer between the create and the
    /// evaluate, N names a child frame's main world there, and the seal, the engine and the
    /// epoch would run in a world the page controls. A new world's `uniqueId` names it and
    /// nothing else, and fails `uniqueContextId not found` once it is gone. A world that
    /// already existed (another session or connection made it, `setContent` unbooted it, an
    /// earlier install failed after the create) gets no event, so there is no `uniqueId` to
    /// take; its numeric evaluate counts only if the session reported nothing about N from
    /// before the create to after the evaluate's reply. The context that N names in a new
    /// renderer has to be reported to the session before a command can run in it (measured:
    /// `executionContextsCleared`, then `executionContextCreated` for N, both ahead of the
    /// evaluate's reply), so a swap in the window reads as churn. That needs `Runtime` enabled
    /// on the session: core enables it on page sessions, `SessionSetup` on OOPIF sessions.
    /// Any churn fails `Gone`, and the caller retries the install as for any gone world.
    pub(crate) async fn install(
        session: ProtocolSession,
        frame: FrameId,
        candidate: FrameSeq,
        epoch: &Epoch,
        mut watch: ContextWatch,
    ) -> Result<Installed, ProbeFailure> {
        let deadline = Instant::now() + FRAME_BUDGET;
        let Some(session_id) = session.session_id().cloned() else {
            return Err(ProbeFailure::Broken(
                "a sealed world needs a target session".to_owned(),
            ));
        };
        let created: CreatedWorld = session
            .send(
                "Page.createIsolatedWorld",
                json!({ "frameId": frame.as_str(), "worldName": SEALED_WORLD }),
            )
            .await
            .map_err(ProbeFailure::from_core)?;
        let context = ContextId(created.execution_context_id);
        let named = match watch.read(&session_id, &frame, context)? {
            Reported::Created(unique) => Named::Unique(unique),
            Reported::Nothing => Named::Number(context),
            Reported::Churned => return Err(ProbeFailure::Gone(Ran::No)),
        };
        let global = global_of(&session, &named).await?;
        if matches!(named, Named::Number(_))
            && watch.read(&session_id, &frame, context)? != Reported::Nothing
        {
            return Err(ProbeFailure::Gone(Ran::No));
        }
        let called = session
            .send::<_, Value>(
                "Runtime.callFunctionOn",
                json!({
                    "objectId": global,
                    "functionDeclaration": install_function(),
                    "arguments": [{ "value": { "frameSeq": candidate, "epoch": epoch.as_str() } }],
                    "returnByValue": true,
                }),
            )
            .await
            .map_err(ProbeFailure::from_core)?;
        let (seq, retired) = match probe::reply_of(called) {
            Ok(ProbeReply::Described {
                protocol,
                seq,
                retired,
                ..
            }) if protocol == PROBE_PROTOCOL => (seq, retired),
            Ok(ProbeReply::Described { protocol, .. }) => {
                return Err(ProbeFailure::Broken(format!(
                    "engine asset is stale: the world runs probe protocol {protocol}, this build \
                     speaks {PROBE_PROTOCOL}; rebuild packages/browseros-playwright-js"
                )));
            }
            Ok(_) => {
                return Err(ProbeFailure::Broken(
                    "boot answered something other than `described`".to_owned(),
                ));
            }
            Err(ProbeFailure::Broken(why)) => {
                return Err(ProbeFailure::Broken(format!(
                    "the seal or the engine failed to install: {why}"
                )));
            }
            Err(failure) => return Err(failure),
        };
        // Boxed: every op's future holds an install's, and this step holds a whole world.
        let watching = Box::pin(Self::watching(session, frame, seq, global, context));
        let world = match time::timeout_at(deadline, watching).await {
            Ok(watched) => watched?,
            Err(_late) => return Err(ProbeFailure::Late),
        };
        Ok(Installed { world, retired })
    }

    /// The booted world, once its seal watches every hidden root its document holds now.
    async fn watching(
        session: ProtocolSession,
        frame: FrameId,
        seq: FrameSeq,
        global: String,
        context: ContextId,
    ) -> Result<Self, ProbeFailure> {
        let document = document_of(&session, &global).await?;
        let world = Self {
            session,
            frame,
            seq,
            global,
            context,
            roots: Arc::new(WatchedRoots::new(document)),
        };
        drop(world.watch_hidden_roots(Scan::Document).await?);
        Ok(world)
    }

    /// Has the seal watch every hidden shadow root that `scan` finds and it does not watch yet
    /// (`hidden_roots`): those in the document now and those of every host the seal reported.
    /// Run at install and right before every sweep reads, so a sweep reads every root that was
    /// in the document when it began, and every reported one, removed from it or not. A sweep
    /// keeps the returned hold until it has read: the hold keeps a removed root alive.
    ///
    /// # Errors
    /// `Gone` when the world's document is gone. Any other failure means a root may be unwatched,
    /// so the caller's sweep must not count.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "a lease on this document's watches: the seal hands each reported host out \
                  once, so a sweep must wait while another watch holds one it has not watched yet"
    )]
    pub(crate) async fn watch_hidden_roots(&self, scan: Scan) -> Result<WatchHold, ProbeFailure> {
        let mut watched = self.roots.lock().await;
        self.roots.unqueue_report();
        let hold = WatchHold::new(self.session.clone());
        // Boxed: every op's future holds an install's, inside `Playwright::invoke`'s.
        Box::pin(self.watch_found(&mut watched, &hold.group, scan)).await?;
        Ok(hold)
    }

    /// The body of [`SealedWorld::watch_hidden_roots`], with its handles in `group`.
    async fn watch_found(
        &self,
        watched: &mut HashSet<BackendNodeId>,
        group: &str,
        scan: Scan,
    ) -> Result<(), ProbeFailure> {
        let (in_document, hosts) = match scan {
            // The trees out of the document that the seal watches (a node a main-world script
            // was handed, then removed) are scanned as a reported host is, at every sweep: the
            // page can attach a closed root under one at any time.
            Scan::Document => {
                let (in_document, hosts, tops) = future::join3(
                    HiddenRoots::scan(&self.session, self.roots.document()),
                    self.take_hosts(group),
                    self.detached_tops(group),
                )
                .await;
                (in_document?, [hosts?, tops?].concat())
            }
            Scan::Reported => (HiddenRoots::default(), self.take_hosts(group).await?),
        };
        let mut found = in_document;
        for host in hosts {
            found.extend(HiddenRoots::scan(&self.session, &host).await?);
        }
        if scan == Scan::Document {
            hidden_roots::keep_found(watched, &found);
        }
        let fresh = hidden_roots::unwatched(watched, &found);
        if fresh.is_empty() {
            return Ok(());
        }
        self.watch_roots(group, &fresh).await?;
        watched.extend(fresh);
        Ok(())
    }

    /// Every host the seal's journal reported that no watch took yet, by handle in `group`.
    /// The seal hands each out once, and holds it until then, so a host the page removed since
    /// still has its root and fields.
    async fn take_hosts(&self, group: &str) -> Result<Vec<String>, ProbeFailure> {
        let mut hosts = Vec::new();
        while hosts.len() < MAX_TAKEN {
            let taken: Evaluated = self
                .session
                .send(
                    "Runtime.callFunctionOn",
                    json!({ "objectId": self.global, "functionDeclaration": TAKE_HOST,
                            "objectGroup": group }),
                )
                .await
                .map_err(ProbeFailure::from_core)?;
            let Some(host) = taken.result.object_id else {
                break;
            };
            hosts.push(host);
        }
        Ok(hosts)
    }

    /// The top of every tree the seal watches out of the document, by handle in `group`: one
    /// call for the list, and one more for its items when there are any.
    async fn detached_tops(&self, group: &str) -> Result<Vec<String>, ProbeFailure> {
        let listed: Listed = self
            .session
            .send(
                "Runtime.callFunctionOn",
                json!({ "objectId": self.global, "functionDeclaration": DETACHED_TOPS,
                        "objectGroup": group }),
            )
            .await
            .map_err(ProbeFailure::from_core)?;
        let Some(list) = listed.result.object_id else {
            return Err(ProbeFailure::Broken(
                "the seal listed no detached trees".to_owned(),
            ));
        };
        if listed.result.description.as_deref() == Some("Array(0)") {
            return Ok(Vec::new());
        }
        let items: Items = self
            .session
            .send(
                "Runtime.getProperties",
                json!({ "objectId": list, "ownProperties": true }),
            )
            .await
            .map_err(ProbeFailure::from_core)?;
        Ok(items
            .result
            .into_iter()
            .filter(|item| item.name.parse::<usize>().is_ok())
            .filter_map(|item| item.value.and_then(|value| value.object_id))
            .collect())
    }

    /// Hands the nodes `nodes` names (handles in this world) to the seal's `watch`, in one call:
    /// the elements a main-world script receives, which the seal then sweeps with their tree at
    /// every sweep they are out of the document (core's `seal.js`). `Gone` when the seal or the
    /// world is.
    pub(crate) async fn watch_trees(&self, nodes: &[String]) -> Result<(), ProbeFailure> {
        let arguments: Vec<Value> = nodes
            .iter()
            .map(|node| json!({ "objectId": node }))
            .collect();
        let called = self
            .session
            .send::<_, Value>(
                "Runtime.callFunctionOn",
                json!({ "objectId": self.global, "functionDeclaration": WATCH_ROOTS,
                        "arguments": arguments, "returnByValue": true }),
            )
            .await
            .map_err(ProbeFailure::from_core)?;
        match called.pointer("/result/value") {
            Some(Value::Bool(true)) => Ok(()),
            Some(Value::Bool(false)) => Err(ProbeFailure::Gone(Ran::No)),
            _ => Err(ProbeFailure::Broken(
                "the seal did not take the elements to watch".to_owned(),
            )),
        }
    }

    /// Resolves each of `roots` into this world and hands them all to the seal's `watch` in one
    /// call. A root whose node is gone since the scan is skipped: nothing in it can be read now.
    /// One the page only removed from the document still resolves (measured), and is watched.
    async fn watch_roots(&self, group: &str, roots: &[BackendNodeId]) -> Result<(), ProbeFailure> {
        let resolves = roots.iter().map(|root| {
            self.session.send::<_, Resolved>(
                "DOM.resolveNode",
                json!({ "backendNodeId": root.0, "executionContextId": self.context.0,
                        "objectGroup": group }),
            )
        });
        let mut arguments = Vec::new();
        for resolved in future::join_all(resolves).await {
            match resolved {
                Ok(Resolved {
                    object:
                        Handle {
                            object_id: Some(object),
                        },
                }) => arguments.push(json!({ "objectId": object })),
                Ok(_) => {
                    return Err(ProbeFailure::Broken(
                        "DOM.resolveNode returned no handle to a shadow root".to_owned(),
                    ));
                }
                Err(CoreError::Cdp(CdpError::Protocol { message, .. }))
                    if message.contains(NODE_GONE) => {}
                Err(error) => return Err(ProbeFailure::from_core(error)),
            }
        }
        if arguments.is_empty() {
            return Ok(());
        }
        // On the global, never on a root: a context id another world has since taken resolves
        // the roots there, and V8 refuses them as arguments to this world's call.
        let called = self
            .session
            .send::<_, Value>(
                "Runtime.callFunctionOn",
                json!({ "objectId": self.global, "functionDeclaration": WATCH_ROOTS,
                        "arguments": arguments, "returnByValue": true }),
            )
            .await
            .map_err(|error| match ProbeFailure::from_core(error) {
                ProbeFailure::Cdp(CoreError::Cdp(CdpError::Protocol { message, .. }))
                    if message.contains("same JavaScript world") =>
                {
                    ProbeFailure::Gone(Ran::No)
                }
                failure => failure,
            })?;
        match called.pointer("/result/value") {
            Some(Value::Bool(true)) => Ok(()),
            Some(Value::Bool(false)) => Err(ProbeFailure::Gone(Ran::No)),
            _ => Err(ProbeFailure::Broken(
                "the seal did not take the hidden shadow roots to watch".to_owned(),
            )),
        }
    }

    /// Queues the sweep a journal report of this world asks for; `false` when one is queued
    /// already, which takes this report's host too.
    pub(crate) fn queue_report(&self) -> bool {
        self.roots.queue_report()
    }

    /// The queued report's sweep gave up before its watch took the reported hosts.
    pub(crate) fn unqueue_report(&self) {
        self.roots.unqueue_report();
    }

    /// The raw session calls into this world go to.
    pub(crate) fn session(&self) -> &ProtocolSession {
        &self.session
    }

    /// The session's id; worlds live only on target sessions, which always have one.
    pub(crate) fn session_id(&self) -> Option<&SessionId> {
        self.session.session_id()
    }

    /// The frame whose document this world belongs to.
    pub(crate) fn frame(&self) -> &FrameId {
        &self.frame
    }

    /// The document's seq: refs minted here print `f<seq>e<n>`, or `e<n>` for seq 0.
    pub(crate) fn seq(&self) -> FrameSeq {
        self.seq
    }

    /// The handle every probe call names.
    pub(crate) fn global(&self) -> &str {
        &self.global
    }

    /// The context id `DOM.resolveNode` needs; see the field.
    pub(crate) fn context(&self) -> ContextId {
        self.context
    }

    /// Whether `other` is this very install, not just the same frame.
    pub(crate) fn same_install(&self, other: &Self) -> bool {
        self.global == other.global && self.session.same_session(&other.session)
    }

    /// A world as an install would leave it, for tests of the bookkeeping around worlds.
    #[cfg(test)]
    pub(crate) fn for_tests(session: ProtocolSession, frame: &str, seq: u32, global: &str) -> Self {
        Self {
            session,
            frame: FrameId(frame.to_owned()),
            seq: FrameSeq(seq),
            global: global.to_owned(),
            context: ContextId(0),
            roots: Arc::new(WatchedRoots::new(String::new())),
        }
    }
}

/// `DOM.resolveNode`'s result.
#[derive(Deserialize)]
struct Resolved {
    object: Handle,
}

/// The handle to the `document` of the world whose global is `global`, in [`WORLD_GROUP`].
async fn document_of(session: &ProtocolSession, global: &str) -> Result<String, ProbeFailure> {
    let called: Evaluated = session
        .send(
            "Runtime.callFunctionOn",
            json!({ "objectId": global, "functionDeclaration": DOCUMENT,
                    "objectGroup": WORLD_GROUP }),
        )
        .await
        .map_err(ProbeFailure::from_core)?;
    called.result.object_id.ok_or_else(|| {
        ProbeFailure::Broken("the sealed world returned no handle to its document".to_owned())
    })
}

/// The handle to the global of the world `named` names, in [`WORLD_GROUP`].
async fn global_of(session: &ProtocolSession, named: &Named) -> Result<String, ProbeFailure> {
    let params = match named {
        Named::Unique(unique) => json!({
            "expression": "globalThis",
            "uniqueContextId": unique,
            "objectGroup": WORLD_GROUP,
        }),
        Named::Number(context) => json!({
            "expression": "globalThis",
            "contextId": context.0,
            "objectGroup": WORLD_GROUP,
        }),
    };
    let evaluated: Evaluated = session
        .send("Runtime.evaluate", params)
        .await
        .map_err(|error| match &error {
            CoreError::Cdp(CdpError::Protocol { message, .. })
                if message.contains(UNIQUE_CONTEXT_GONE) =>
            {
                ProbeFailure::Gone(Ran::No)
            }
            _ => ProbeFailure::from_core(error),
        })?;
    evaluated.result.object_id.ok_or_else(|| {
        ProbeFailure::Broken("the sealed world returned no handle to its global".to_owned())
    })
}

impl ContextWatch {
    /// A watch over `events`, a subscription to the connection's events taken before the
    /// create.
    pub(crate) fn new(events: broadcast::Receiver<CdpEvent>) -> Self {
        Self { events }
    }

    /// What `session` reported about context `id` since the last read. After a reply has been
    /// read, every event the browser sent before it is already here: the connection's reader
    /// hands an event to subscribers before it completes the reply that came after it.
    ///
    fn read(
        &mut self,
        session: &SessionId,
        frame: &FrameId,
        id: ContextId,
    ) -> Result<Reported, ProbeFailure> {
        let mut reported = Reported::Nothing;
        loop {
            let seen = match self.events.try_recv() {
                Ok(event) if event.session_id.as_ref() == Some(session) => {
                    Reported::of(&event, frame, id)?
                }
                Ok(_) => continue,
                Err(TryRecvError::Lagged(_)) => Reported::Churned,
                Err(TryRecvError::Empty) => return Ok(reported),
                Err(TryRecvError::Closed) => return Ok(reported.and(Reported::Churned)),
            };
            reported = reported.and(seen);
        }
    }
}

impl Reported {
    /// Two reports about one id, in either order. A created world wins over churn: its unique
    /// id can only ever name that world, and an evaluate by it fails once the world is gone.
    fn and(self, later: Self) -> Self {
        match (self, later) {
            (Self::Created(unique), _) | (_, Self::Created(unique)) => Self::Created(unique),
            (Self::Churned, _) | (_, Self::Churned) => Self::Churned,
            (Self::Nothing, Self::Nothing) => Self::Nothing,
        }
    }

    /// What one event of the watched session says about context `id` of `frame`. An event this
    /// build cannot read is churn, since it might be the one that moved the id.
    fn of(event: &CdpEvent, frame: &FrameId, id: ContextId) -> Result<Self, ProbeFailure> {
        match event.method.as_str() {
            "Runtime.executionContextCreated" => {
                let Ok(created) = ContextCreated::deserialize(&event.params) else {
                    return Ok(Self::Churned);
                };
                let context = created.context;
                if context.id != id.0 {
                    return Ok(Self::Nothing);
                }
                let ours = context.name == SEALED_WORLD
                    && context.aux_data.kind.as_deref() == Some("isolated")
                    && context.aux_data.frame_id.as_deref() == Some(frame.as_str());
                match (ours, context.unique_id) {
                    (true, Some(unique)) => Ok(Self::Created(unique)),
                    (true, None) => Err(ProbeFailure::Broken(
                        "the browser reported the sealed world without a uniqueId, so its handle \
                         cannot be taken safely"
                            .to_owned(),
                    )),
                    (false, _) => Ok(Self::Churned),
                }
            }
            "Runtime.executionContextDestroyed" => {
                match ContextDestroyed::deserialize(&event.params) {
                    Ok(destroyed) if destroyed.execution_context_id != id.0 => Ok(Self::Nothing),
                    _ => Ok(Self::Churned),
                }
            }
            "Runtime.executionContextsCleared" => Ok(Self::Churned),
            _ => Ok(Self::Nothing),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
    use std::time::Duration;

    use browseros_cdp::{CdpError, CdpEvent};
    use browseros_core::testing::{TestCall, TestConnection, cdp_error, cdp_pending};
    use browseros_core::{CdpConnection, FrameId, ProtocolSession, SessionId};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use tokio::sync::{broadcast, mpsc};
    use tokio::{task, time};

    use super::{ContextWatch, Installed, Scan, SealedWorld};
    use crate::page::FRAME_BUDGET;
    use crate::probe::{PROBE_PROTOCOL, ProbeFailure};
    use crate::world::{Epoch, FrameSeq};

    type TestResult = Result<(), Box<dyn Error>>;

    const SESSION: &str = "S";
    const FRAME: &str = "F";
    const CONTEXT: i64 = 7;
    const UNIQUE: &str = "-4711290926028789826.-850444488107631177";

    /// An event the browser sends just before the reply to the command that causes it.
    type Caused = (&'static str, &'static str, Value);

    /// A scripted browser that sends the events a command causes ahead of its reply, in the
    /// order measured on BrowserOS.
    struct Browser {
        replies: Arc<TestConnection>,
        caused: Mutex<Vec<Caused>>,
        /// How long the browser takes to answer `DOM.describeNode`, a page too large or deep to
        /// scan quickly.
        describe_takes: Mutex<Duration>,
    }

    impl Browser {
        /// Replies to the install's commands: `evaluated` to the evaluate, `booted` as the
        /// install function's return, then the document's handle, no host the seal reported, no
        /// tree it watches out of the document, and a scan that finds no shadow root.
        fn answering(evaluated: Value, booted: Value) -> Arc<Self> {
            let mut called = json!({ "result": { "type": "object" } });
            called["result"]["value"] = booted;
            let replies = TestConnection::new([
                (
                    "Page.createIsolatedWorld",
                    json!({ "executionContextId": CONTEXT }),
                ),
                ("Runtime.evaluate", evaluated),
                ("Runtime.callFunctionOn", called),
                (
                    "Runtime.callFunctionOn",
                    json!({ "result": { "type": "object", "objectId": "1.7.2" } }),
                ),
                (
                    "Runtime.callFunctionOn",
                    json!({ "result": { "type": "object", "subtype": "null", "value": null } }),
                ),
                (
                    "Runtime.callFunctionOn",
                    json!({ "result": { "type": "object", "subtype": "array",
                                        "objectId": "1.7.3", "description": "Array(0)" } }),
                ),
                (
                    "DOM.describeNode",
                    json!({ "node": { "backendNodeId": 1, "nodeName": "#document",
                                      "childNodeCount": 0, "children": [] } }),
                ),
            ]);
            Arc::new(Self {
                replies,
                caused: Mutex::default(),
                describe_takes: Mutex::default(),
            })
        }

        /// A handle, and a world that boots with seq 3.
        fn new() -> Arc<Self> {
            Self::answering(handle(), described(None))
        }

        /// The session sends `event` just before the reply to the next `command`.
        fn on(&self, command: &'static str, event: &'static str, params: Value) {
            self.lock().push((command, event, params));
        }

        fn calls(&self, method: &str) -> Result<Vec<TestCall>, CdpError> {
            Ok(self
                .replies
                .calls()?
                .into_iter()
                .filter(|call| call.method == method)
                .collect())
        }

        fn lock(&self) -> MutexGuard<'_, Vec<Caused>> {
            self.caused.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }

    impl CdpConnection for Browser {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            let (now, later): (Vec<Caused>, Vec<Caused>) = self
                .lock()
                .drain(..)
                .partition(|(command, ..)| *command == method);
            *self.lock() = later;
            for (_, event, event_params) in now {
                self.replies.emit(event, event_params, session.cloned());
            }
            let takes = if method == "DOM.describeNode" {
                *self
                    .describe_takes
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
            } else {
                Duration::ZERO
            };
            Box::pin(async move {
                time::sleep(takes).await;
                self.replies.send(method, params, session).await
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            method: &'a str,
            params_json: &'a str,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            self.replies.send_raw_json(method, params_json, session)
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            self.replies.events()
        }

        fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
            self.replies.events_targeted(method)
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    /// `Runtime.executionContextCreated` as BrowserOS sends it.
    fn created(id: i64, name: &str, kind: &str, frame: &str) -> Value {
        json!({ "context": {
            "id": id, "origin": "https://a.test", "name": name, "uniqueId": UNIQUE,
            "auxData": { "isDefault": kind == "default", "type": kind, "frameId": frame },
        } })
    }

    fn our_world() -> Value {
        created(CONTEXT, "browseros-sdk", "isolated", FRAME)
    }

    /// `Runtime.evaluate`'s reply: the global's handle.
    fn handle() -> Value {
        json!({ "result": { "type": "object", "objectId": "1.7.1" } })
    }

    /// The install function's return: the world kept seq 3, after a re-key of `retired`.
    fn described(retired: Option<u32>) -> Value {
        let mut described =
            json!({ "kind": "described", "protocol": PROBE_PROTOCOL, "seq": 3, "url": "" });
        if let Some(retired) = retired {
            described["retired"] = json!(retired);
        }
        described
    }

    /// An install as `FrameWorlds` runs it: the watch subscribed first.
    async fn install(browser: &Arc<Browser>) -> Result<SealedWorld, ProbeFailure> {
        install_reporting(browser)
            .await
            .map(|installed| installed.world)
    }

    async fn install_reporting(browser: &Arc<Browser>) -> Result<Installed, ProbeFailure> {
        let connection = Arc::clone(browser) as Arc<dyn CdpConnection>;
        let watch = ContextWatch::new(connection.events());
        let session = ProtocolSession::for_session(connection, SessionId::from(SESSION));
        SealedWorld::install(
            session,
            FrameId(FRAME.to_owned()),
            FrameSeq(3),
            &Epoch::random(),
            watch,
        )
        .await
    }

    #[tokio::test]
    async fn tests_that_a_new_world_is_taken_by_its_unique_id_never_its_number() -> TestResult {
        let browser = Browser::new();
        browser.on(
            "Page.createIsolatedWorld",
            "Runtime.executionContextCreated",
            our_world(),
        );
        let world = install(&browser).await?;
        let evaluated = browser.calls("Runtime.evaluate")?;
        assert_eq!(evaluated.len(), 1, "one handle, taken once");
        assert_eq!(evaluated[0].params["uniqueContextId"], UNIQUE);
        assert!(
            evaluated[0].params.get("contextId").is_none(),
            "the numeric id never takes a new world's handle: {}",
            evaluated[0].params
        );
        let called = browser.calls("Runtime.callFunctionOn")?;
        assert_eq!(
            called[0].params["objectId"], "1.7.1",
            "the seal and the engine go to the handle the unique id gave"
        );
        assert_eq!(world.seq(), FrameSeq(3));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_another_context_under_the_number_fails_gone_before_any_evaluate()
    -> TestResult {
        for (name, kind, frame) in [
            ("", "default", FRAME),
            ("browseros-sdk", "isolated", "another frame"),
            ("browseros-sdk", "default", FRAME),
            ("another world", "isolated", FRAME),
        ] {
            let browser = Browser::new();
            browser.on(
                "Page.createIsolatedWorld",
                "Runtime.executionContextCreated",
                created(CONTEXT, name, kind, frame),
            );
            let installed = install(&browser).await;
            assert!(
                matches!(installed, Err(ProbeFailure::Gone(_))),
                "{name}/{kind}/{frame}: {installed:?}"
            );
            assert!(
                browser.calls("Runtime.evaluate")?.is_empty(),
                "{name}/{kind}/{frame}: no handle is taken under an id that names another context"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_world_that_already_existed_is_taken_by_number_only_in_a_quiet_window()
    -> TestResult {
        let quiet = Browser::new();
        quiet.on(
            "Page.createIsolatedWorld",
            "Runtime.executionContextCreated",
            created(CONTEXT + 1, "", "default", "another frame"),
        );
        install(&quiet).await?;
        assert_eq!(
            quiet.calls("Runtime.evaluate")?[0].params["contextId"],
            CONTEXT,
            "no event for the world: it existed, and nothing touched its number"
        );

        // The document moved to another renderer between the create and the evaluate's reply.
        for (event, params) in [
            ("Runtime.executionContextsCleared", json!({})),
            (
                "Runtime.executionContextCreated",
                created(CONTEXT, "", "default", "a child frame"),
            ),
            (
                "Runtime.executionContextDestroyed",
                json!({ "executionContextId": CONTEXT, "executionContextUniqueId": UNIQUE }),
            ),
        ] {
            let swapped = Browser::new();
            swapped.on("Runtime.evaluate", event, params);
            let installed = install(&swapped).await;
            assert!(
                matches!(installed, Err(ProbeFailure::Gone(_))),
                "{event}: {installed:?}"
            );
            assert!(
                swapped.calls("Runtime.callFunctionOn")?.is_empty(),
                "{event}: the seal and the engine never reach a handle taken across churn"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_gone_unique_id_and_a_lagged_watch_fail_gone() -> TestResult {
        let gone = Browser::answering(cdp_error("uniqueContextId not found"), described(None));
        gone.on(
            "Page.createIsolatedWorld",
            "Runtime.executionContextCreated",
            our_world(),
        );
        let installed = install(&gone).await;
        assert!(
            matches!(installed, Err(ProbeFailure::Gone(_))),
            "{installed:?}"
        );

        // More events than the connection's buffer holds: the watch may have lost the one that
        // moved the number.
        let flooded = Browser::new();
        for _ in 0..100 {
            flooded.on(
                "Page.createIsolatedWorld",
                "Page.lifecycleEvent",
                json!({ "name": "networkIdle" }),
            );
        }
        let installed = install(&flooded).await;
        assert!(
            matches!(installed, Err(ProbeFailure::Gone(_))),
            "{installed:?}"
        );
        assert!(flooded.calls("Runtime.evaluate")?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_sealed_world_reported_without_a_unique_id_is_broken() -> TestResult {
        let browser = Browser::new();
        let mut world = our_world();
        if let Some(context) = world["context"].as_object_mut() {
            context.remove("uniqueId");
        }
        browser.on(
            "Page.createIsolatedWorld",
            "Runtime.executionContextCreated",
            world,
        );
        let installed = install(&browser).await;
        assert!(
            matches!(installed, Err(ProbeFailure::Broken(_))),
            "{installed:?}"
        );
        assert!(browser.calls("Runtime.evaluate")?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_the_install_reports_the_seq_a_rekey_replaced() -> TestResult {
        for (booted, retired) in [
            (described(Some(0)), Some(FrameSeq(0))),
            (described(None), None),
        ] {
            let browser = Browser::answering(handle(), booted);
            browser.on(
                "Page.createIsolatedWorld",
                "Runtime.executionContextCreated",
                our_world(),
            );
            let installed = install_reporting(&browser).await?;
            assert_eq!(
                (installed.world.seq(), installed.retired),
                (FrameSeq(3), retired)
            );
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_an_install_whose_scan_misses_the_frame_budget_is_refused() -> TestResult {
        for (takes, late) in [
            (FRAME_BUDGET.saturating_sub(Duration::from_millis(1)), false),
            (FRAME_BUDGET + Duration::from_millis(1), true),
        ] {
            let browser = Browser::new();
            browser.on(
                "Page.createIsolatedWorld",
                "Runtime.executionContextCreated",
                our_world(),
            );
            *browser
                .describe_takes
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = takes;
            let installed = install(&browser).await;
            assert_eq!(
                matches!(installed, Err(ProbeFailure::Late)),
                late,
                "a scan of {takes:?}: {installed:?}"
            );
            assert_eq!(
                installed.is_ok(),
                !late,
                "a first read installs its world, so a scan past the budget refuses it"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_watch_dropped_mid_way_still_releases_its_object_group() -> TestResult {
        let connection = TestConnection::new([("Runtime.callFunctionOn", cdp_pending())]);
        let session = ProtocolSession::for_session(
            Arc::clone(&connection) as Arc<dyn CdpConnection>,
            SessionId::from(SESSION),
        );
        let world = SealedWorld::for_tests(session, FRAME, 3, "1.7.1");
        let watch = world.watch_hidden_roots(Scan::Reported);
        assert!(
            time::timeout(Duration::from_millis(50), watch)
                .await
                .is_err(),
            "the take of a reported host never answers, so the budget drops the watch"
        );
        for _ in 0..10 {
            task::yield_now().await;
        }
        let calls = connection.calls()?;
        let group = calls
            .iter()
            .find(|call| call.method == "Runtime.callFunctionOn")
            .map(|call| call.params["objectGroup"].clone())
            .ok_or("the watch took no host")?;
        assert!(
            calls
                .iter()
                .any(|call| call.method == "Runtime.releaseObjectGroup"
                    && call.params["objectGroup"] == group),
            "a retry makes a new group, so a dropped watch must release its own: {calls:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_event_a_command_causes_reaches_a_targeted_subscriber() -> TestResult {
        let browser = Browser::new();
        let mut broadcast = browser.events();
        let mut targeted = browser.events_targeted("Runtime.executionContextCreated");
        browser.on(
            "Page.createIsolatedWorld",
            "Runtime.executionContextCreated",
            our_world(),
        );
        browser
            .send(
                "Page.createIsolatedWorld",
                json!({}),
                Some(&SessionId::from(SESSION)),
            )
            .await?;
        assert_eq!(
            targeted.try_recv()?.method,
            "Runtime.executionContextCreated"
        );
        assert!(
            broadcast.try_recv().is_err(),
            "a method with a live targeted subscriber must bypass the broadcast"
        );
        Ok(())
    }
}
