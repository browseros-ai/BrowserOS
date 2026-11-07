//! Neo: the only door from Rust into a dedicated worker's execution context, where Playwright
//! runs `Worker.evaluate` and `Worker.evaluateHandle` (`server/page.ts`
//! `Worker.evaluateExpression`). As `main_world.rs` does for a frame, it runs exactly two built
//! functions of neo-runtime-injected and nothing else: `dist/utility.js` builds Playwright's
//! `UtilityScript` in the worker, and `dist/evaluate.js` calls it. The agent's code only ever
//! travels as an argument value of `evaluate.js`.
//!
//! ```text
//!  attach (workers.rs)
//!    Runtime.enable                       the worker's session only: its one execution
//!                                         context (Runtime.executionContextCreated)
//!  first call
//!    Runtime.callFunctionOn{executionContextId, UTILITY}       → the UtilityScript's object id
//!  every call
//!    Runtime.callFunctionOn{objectId: UtilityScript, EVALUATE,
//!      arguments: [{objectId: UtilityScript}, …], awaitPromise, userGesture, returnByValue}
//!  the worker closes → every call in flight or later ends `Closed`
//! ```
//!
//! A worker has no document to anchor the install on, so it is anchored on the worker's own
//! execution context, the only one a dedicated worker has. `tests/page_js_guard.rs` and
//! `crates/browseros-core/tests/page_js_guard.rs` allow this file to send
//! `Runtime.callFunctionOn` and nothing else that runs script, and require every
//! `functionDeclaration` here to be one of the built constants below; `Runtime.evaluate` stays
//! forbidden. `Runtime.enable` goes to worker sessions only: a rule of this file, which neither
//! guard checks, as neither lists `Runtime.enable`.

use browseros_cdp::runtime::CallFunctionOnResult;
use browseros_cdp::{CdpClient, CdpError, SessionId};
use neo_runtime_spec::HandleId;
use serde_json::{Value, json};
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

use crate::click::PageError;
use crate::main_world::{self, Called};
use crate::world::{self, ObjectGroup};

/// `() => new UtilityScript(globalThis, false)`.
/// Playwright: `server/javascript.ts` `ExecutionContext._utilityScript`
const UTILITY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-runtime-injected/dist/utility.js"
));

/// `(utilityScript, ...args) => utilityScript.evaluate(...args)`.
/// Playwright: `server/javascript.ts` `evaluateExpression`
const EVALUATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-runtime-injected/dist/evaluate.js"
));

/// Neo: the object group the worker's `UtilityScript` lives in; it goes with the worker.
const WORKER_WORLD_GROUP: &str = "neo-worker-world";

/// Neo: what an install answer without an object id is, which no healthy worker gives.
const NO_UTILITY: &str = "the worker's UtilityScript came back without an object id";

/// Sends `Runtime.enable` on a dedicated worker's session, so it reports its execution context.
/// Playwright: `server/chromium/crPage.ts` `FrameSession._initializeWorker`: `session._sendMayFail('Runtime.enable');`
///
/// # Errors
///
/// The session's answer when the worker is already gone.
pub(crate) async fn enable(cdp: &CdpClient, session: &SessionId) -> Result<(), CdpError> {
    cdp.send("Runtime.enable", json!({}), Some(session))
        .await
        .map(|_| ())
}

/// One dedicated worker's execution context, and the `UtilityScript` installed in it on first
/// use, as Playwright's `ExecutionContext` keeps its utility script.
pub(crate) struct WorkerWorld {
    cdp: CdpClient,
    session: SessionId,
    /// `Runtime.executionContextCreated`'s `context.id` on the worker's session.
    context: i64,
    /// Cancelled when the worker closes: every call then ends `Closed`.
    closed: CancellationToken,
    utility: OnceCell<HandleId>,
}

impl WorkerWorld {
    pub(crate) fn new(
        cdp: CdpClient,
        session: SessionId,
        context: i64,
        closed: CancellationToken,
    ) -> Self {
        Self {
            cdp,
            session,
            context,
            closed,
            utility: OnceCell::new(),
        }
    }

    /// The worker's session, which a call's handle arguments must come from.
    pub(crate) fn session(&self) -> &SessionId {
        &self.session
    }

    /// `evaluate.js` with `arguments`, after the `{objectId: UtilityScript}` Playwright passes
    /// first, as `main_world.rs` `MainWorld::call` runs it in a frame: `awaitPromise` and
    /// `userGesture`, by value or into `group`.
    /// Playwright: `server/chromium/crExecutionContext.ts` `CRExecutionContext.evaluateWithArguments`
    ///
    /// # Errors
    ///
    /// `Closed` when the worker or its session is gone; every other outcome is a [`Called`].
    pub(crate) async fn call(
        &self,
        arguments: Vec<Value>,
        return_by_value: bool,
        group: Option<&ObjectGroup>,
    ) -> Result<Called, PageError> {
        let utility = self.utility().await?;
        let mut all = Vec::with_capacity(arguments.len() + 1);
        all.push(json!({ "objectId": utility.object_id() }));
        all.extend(arguments);
        let mut request = json!({ "functionDeclaration": EVALUATE });
        request["objectId"] = json!(utility.object_id());
        request["arguments"] = json!(all);
        request["returnByValue"] = json!(return_by_value);
        request["awaitPromise"] = json!(true);
        request["userGesture"] = json!(true);
        if let Some(ObjectGroup(group)) = group {
            request["objectGroup"] = json!(group);
        }
        let reply = match self.call_function_on(request).await? {
            Ok(reply) => reply,
            Err(CdpError::Protocol { message, .. }) => {
                return Ok(main_world::stale_or_protocol(message));
            }
            Err(error) => return Err(world::classify(&error)),
        };
        let answer: CallFunctionOnResult = serde_json::from_value(reply).map_err(|error| {
            PageError::Contract(format!("Runtime.callFunctionOn answered: {error}"))
        })?;
        if let Some(details) = answer.exception_details {
            return Ok(Called::Threw(main_world::exception_message(details)));
        }
        Ok(if return_by_value {
            Called::Value(answer.result.value)
        } else {
            Called::Object(Box::new(answer.result))
        })
    }

    /// The `UtilityScript` in the worker's context, built on first use.
    /// Playwright: `server/javascript.ts` `ExecutionContext._utilityScript`
    async fn utility(&self) -> Result<&HandleId, PageError> {
        self.utility
            .get_or_try_init(|| async {
                let request = json!({
                    "functionDeclaration": UTILITY,
                    "executionContextId": self.context,
                    "returnByValue": false,
                    "objectGroup": WORKER_WORLD_GROUP,
                });
                let reply = self
                    .call_function_on(request)
                    .await?
                    .map_err(|error| world::classify(&error))?;
                let answer: CallFunctionOnResult =
                    serde_json::from_value(reply).map_err(|error| {
                        PageError::Contract(format!("Runtime.callFunctionOn answered: {error}"))
                    })?;
                if let Some(details) = answer.exception_details {
                    return Err(PageError::Threw(main_world::exception_message(details)));
                }
                let id = answer
                    .result
                    .object_id
                    .ok_or_else(|| PageError::Contract(NO_UTILITY.to_string()))?;
                Ok(HandleId::new(id))
            })
            .await
    }

    /// One `Runtime.callFunctionOn` on the worker's session, ended `Closed` once the worker goes:
    /// a closed worker answers nothing more, and Playwright's call then rejects as closed.
    async fn call_function_on(&self, request: Value) -> Result<Result<Value, CdpError>, PageError> {
        let sent = self
            .cdp
            .send("Runtime.callFunctionOn", request, Some(&self.session));
        tokio::select! {
            biased;
            () = self.closed.cancelled() => Err(PageError::Closed),
            reply = sent => Ok(reply),
        }
    }
}
