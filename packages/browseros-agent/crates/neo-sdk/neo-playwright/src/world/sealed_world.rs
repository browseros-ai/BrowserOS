//! One sealed world, and the install that makes it. One of the three files in core's
//! `tests/page_js_guard.rs` ALLOWED list: it creates the `browseros-sdk` world and runs the
//! engine there, apart from the page's main world.

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{CoreError, FrameId, ProtocolSession, SessionId};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::TryRecvError;

use crate::probe::{self, PROBE_PROTOCOL, ProbeFailure, ProbeReply, Ran};
use crate::world::engine_source::install_function;
use crate::world::{ContextId, Epoch, FrameSeq, SEALED_WORLD};

/// The object group of every world's handles, its global's and its document's. Never released:
/// they die with their context, and an op's own group (`OpScope`) holds only the objects that
/// op asked for.
const WORLD_GROUP: &str = "browseros-sdk-world";

/// How `Runtime.evaluate` refuses a `uniqueContextId` that names no live context (measured on
/// BrowserOS: an unknown id, and a world whose document moved to another renderer).
const UNIQUE_CONTEXT_GONE: &str = "uniqueContextId not found";

/// A live sealed world: where a probe call goes. Only [`SealedWorld::install`] makes one, so
/// holding one means the engine ran in that world, an isolated world the page's own scripts
/// cannot reach. Cheap to clone. It may go stale at any time (the document navigates); a call
/// on it then fails `Gone`.
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
}

/// What an install leaves: the world, and the seq its boot took over from, if it did.
#[derive(Debug)]
pub(crate) struct Installed {
    pub(crate) world: SealedWorld,
    /// The seq the world's probe held before this boot renumbered it. From another
    /// `Playwright` (a restart's re-key) it names a document that old refs may still print, so
    /// the page must never hand it out again (`FrameWorlds`).
    pub(crate) retired: Option<FrameSeq>,
    /// The document's URL when the world booted (`location.href`): the page a main frame's
    /// next commit leaves, when no commit of it was seen yet (`FrameWorlds`).
    pub(crate) url: String,
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

impl SealedWorld {
    /// Creates (or finds, by name) the frame's `browseros-sdk` world and installs the engine
    /// there, or re-keys the probe already there. `candidate` is the seq for a new
    /// document; the reply's seq wins, because a world this `Playwright` already booted keeps
    /// the one it has, and a re-key reports the seq it replaced. `Runtime` must already be
    /// enabled on the session (`SessionSetup`) so the watch observes context creation.
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
    ///  Runtime.callFunctionOn { handle, install_function,     engine if absent, boot
    ///                           [{ frameSeq, epoch }] }       → described { protocol, seq, url }
    /// ```
    ///
    /// The op's own timeout bounds the install; nothing here waits on a clock.
    ///
    /// N alone is not enough to take the handle. It is a per-renderer counter: measured on
    /// BrowserOS, when the document moves to another renderer between the create and the
    /// evaluate, N names a child frame's main world there, and the engine and the epoch would
    /// run in a world the page controls. A new world's `uniqueId` names it and
    /// nothing else, and fails `uniqueContextId not found` once it is gone. A world that
    /// already existed (another session or connection made it, an earlier install failed after
    /// the create) gets no event, so there is no `uniqueId` to take; its numeric evaluate counts
    /// only if the session reported nothing about N from before the create to after the
    /// evaluate's reply. The context that N names in a new
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
        let (seq, retired, url) = match probe::reply_of(called) {
            Ok(ProbeReply::Described {
                protocol,
                seq,
                retired,
                url,
            }) if protocol == PROBE_PROTOCOL => (seq, retired, url),
            Ok(ProbeReply::Described { protocol, .. }) => {
                return Err(ProbeFailure::Broken(format!(
                    "engine asset is stale: the world runs probe protocol {protocol}, this build \
                     speaks {PROBE_PROTOCOL}; rebuild crates/neo-sdk/neo-playwright-js"
                )));
            }
            Ok(_) => {
                return Err(ProbeFailure::Broken(
                    "boot answered something other than `described`".to_owned(),
                ));
            }
            Err(ProbeFailure::Broken(why)) => {
                return Err(ProbeFailure::Broken(format!(
                    "the engine failed to install: {why}"
                )));
            }
            Err(failure) => return Err(failure),
        };
        Ok(Installed {
            world: Self {
                session,
                frame,
                seq,
                global,
                context,
            },
            retired,
            url,
        })
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

    /// Runs an internal screenshot rendering or geometry script in this held isolated world.
    /// Screenshot code may read layout numbers here; agent-provided functions never enter this
    /// gateway. The held global prevents a recycled numeric context from changing the world.
    pub(crate) async fn call_for_screenshot(
        &self,
        script: &str,
        arguments: &Value,
    ) -> Result<Value, CoreError> {
        #[derive(Deserialize)]
        struct ResultValue {
            value: Option<Value>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Reply {
            result: Option<ResultValue>,
            exception_details: Option<Value>,
        }
        let arguments: Vec<Value> = arguments
            .as_array()
            .into_iter()
            .flatten()
            .map(|value| json!({ "value": value }))
            .collect();
        let reply: Reply = self.session.send("Runtime.callFunctionOn", json!({
            "objectId": self.global(), "functionDeclaration": script, "arguments": arguments,
            "returnByValue": true, "awaitPromise": true,
        })).await?;
        if reply.exception_details.is_some() {
            return Err(CoreError::PageScript("screenshot script failed".to_owned()));
        }
        Ok(reply
            .result
            .and_then(|result| result.value)
            .unwrap_or(Value::Null))
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
        }
    }
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

    use browseros_cdp::{CdpError, CdpEvent};
    use browseros_core::testing::{TestCall, TestConnection, cdp_error};
    use browseros_core::{CdpConnection, FrameId, ProtocolSession, SessionId};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use tokio::sync::{broadcast, mpsc};

    use super::{ContextWatch, Installed, SealedWorld};
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
    }

    impl Browser {
        /// Replies to the install's commands: `evaluated` to the evaluate, `booted` as the
        /// install function's return.
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
            ]);
            Arc::new(Self {
                replies,
                caused: Mutex::default(),
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
            self.replies.send(method, params, session)
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
