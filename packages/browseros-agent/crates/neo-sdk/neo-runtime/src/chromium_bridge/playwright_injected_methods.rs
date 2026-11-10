//! The utility world in a frame, where neo-playwright-injected's glue (Playwright's
//! injected script behind Neo's typed table) answers page calls, and the one way Rust calls the
//! page: [`World::call`], typed by the spec's `InjectedCall`.
//!
//! Lifecycle (no `Runtime.enable`, no execution-context events):
//!
//! ```text
//!  first call in a document      Page.createIsolatedWorld{frameId, WORLD_NAME} → contextId
//!    callFunctionOn{executionContextId, ATTACH, [key]}             → glue objectId | undefined
//!    undefined → callFunctionOn{executionContextId, INSTALL, [key, SPEC_SHA256, initializer]}
//!  every call                    callFunctionOn{objectId: glue, CALL, [name, params, ...handles]}
//!  frame document commit         → invalidate cached glue before publishing navigation
//!  a protocol error              → PageError::NotConnected and the cache is cleared; the next
//!    (context or object gone,       call in the new document attaches again
//!     navigating)
//! ```
//!
//! Measured on BrowserOS neo (grounding E-probes, probe P1): a named isolated world belongs to
//! the frame's document, every CDP session on the tab gets the same context for the same name,
//! and it outlives the session that created it, with neither `Page.enable` nor
//! `Runtime.enable`. So the `InjectedScript` is built once per document and later runs only
//! ATTACH, while each run still gets its own glue `objectId` (object ids are per session). The
//! glue is always called through that object id, never a cached context id, so a call can only
//! land in the world that returned it.
//!
//! A locator has two halves that must agree on names: neo-quickjs-injected builds the selector
//! inside the realm, and the page's injected script matches it. [`TEST_ID_ATTRIBUTE`] is the one
//! name both halves share.
//!
//! Every byte of JavaScript sent here is a built artifact of neo-playwright-injected (`dist/*.js`);
//! this file writes none.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use browseros_cdp::CdpError;
use browseros_cdp::runtime::{CallFunctionOnResult, PropertyDescriptor, RemoteObject};
use neo_playwright_protocol::injected::InjectedInitializer;
use neo_playwright_protocol::{
    Element, Handle, HandleId, InjectedCall, PageResult, Properties, SPEC_SHA256,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::chromium_bridge::cdp::{self, Command, dom, page, runtime};
use crate::playwright_methods::tab::PageError;

/// Neo's isolated world. Distinct from BrowserOS core's `browseros` and Playwright's
/// `__playwright_utility_world_<guid>`: worlds are shared by name across every client of the
/// tab, and shared by every Neo run on the tab, which is what makes the install reusable.
pub(crate) const WORLD_NAME: &str = "__neo_utility_world__";

/// The attribute `getByTestId` selects by. One constant for both halves: `quickjs.rs` hands it to
/// neo-quickjs-injected's `createRun`, which sets Playwright's module-level test id attribute, and
/// [`initializer`] gives it to the page's `InjectedScript`. Two copies could drift into a
/// `getByTestId` locator that never matches anything.
pub(crate) const TEST_ID_ATTRIBUTE: &str = "data-testid";

/// `(key) => glue | undefined`, about a kilobyte; it also hides BrowserOS's own page UI from
/// the world (`neo-playwright-injected/src/browseros-ui.ts`).
const ATTACH: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/attach.js"
));
/// `(key, specSha256, initializer) => glue`, the ~320 KB bundle.
const INSTALL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/install.js"
));
/// `function (name, params, ...handles) { return this[name](params, ...handles) }`.
const CALL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/call.js"
));

// A spec edit regenerates `neo_playwright_protocol::SPEC_SHA256`; until neo-playwright-injected is rebuilt
// and its dist committed, neo-runtime does not compile, so a page bundle built from another spec
// can never answer this build's decode.
const _: () = assert!(
    neo_playwright_protocol::is_current(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../neo-playwright-injected/dist/spec-sha256"
    ))),
    "neo-playwright-injected/dist was built from another spec: run `bun run build` in crates/neo-sdk/neo-playwright-injected",
);

/// The run-long object group the glue handle lives in. Never released by Neo: Chromium frees
/// it with the session, and a new document's glue replaces the old one's dead handle.
const GLUE_GROUP: &str = "neo-glue";

/// The initializer every document gets: local refs (`frameSeq: 0`), [`TEST_ID_ATTRIBUTE`],
/// `stableRafCount: 1` as Playwright's Chromium passes it (`crPage.ts`
/// `rafCountForStablePosition`), utility world.
pub(crate) fn initializer() -> InjectedInitializer {
    InjectedInitializer {
        frame_seq: 0,
        test_id_attribute_name: TEST_ID_ATTRIBUTE.to_string(),
        stable_raf_count: 1,
        is_utility_world: true,
    }
}

/// [`initializer`] with the run's test id attribute `test_id` (`selectors.setTestIdAttribute`).
fn initializer_named(test_id: &str) -> InjectedInitializer {
    InjectedInitializer {
        test_id_attribute_name: test_id.to_string(),
        ..initializer()
    }
}

/// The glue's key on the world's global: `<SPEC_SHA256>/<initializer JSON>`. A glue object an
/// older Neo build or another initializer installed in this document is never attached to, so
/// its answers are never decoded against the wrong spec.
#[cfg(test)]
fn glue_key() -> Result<String, PageError> {
    glue_key_named(TEST_ID_ATTRIBUTE)
}

/// [`glue_key`] for the initializer of `test_id`: a document holds one glue per test id
/// attribute a run named, each with its own `InjectedScript`.
fn glue_key_named(test_id: &str) -> Result<String, PageError> {
    let initializer = serde_json::to_string(&initializer_named(test_id))
        .map_err(|error| PageError::Contract(format!("encoding the initializer: {error}")))?;
    Ok(format!("{SPEC_SHA256}/{initializer}"))
}

/// The run's test id attribute, [`TEST_ID_ATTRIBUTE`] until the script's
/// `selectors.setTestIdAttribute` names another. Every world of the run's pages shares it, so a
/// document the run attaches to from then on gets an injected script that names the attribute
/// neo-quickjs-injected's `getByTestId` selects by (in the locators a strict mode violation
/// suggests), as Playwright's injected script for a new document does.
#[derive(Debug)]
pub(crate) struct TestIdAttribute(RwLock<String>);

impl Default for TestIdAttribute {
    fn default() -> Self {
        Self(RwLock::new(TEST_ID_ATTRIBUTE.to_string()))
    }
}

impl TestIdAttribute {
    pub(crate) fn get(&self) -> String {
        self.0
            .read()
            .map_or_else(|_| TEST_ID_ATTRIBUTE.to_string(), |name| name.clone())
    }

    /// Sets the attribute for the run's next page calls. Called in the call's native send, so
    /// every call the script makes after it sees it.
    pub(crate) fn set(&self, name: &str) {
        if let Ok(mut current) = self.0.write() {
            name.clone_into(&mut current);
        }
    }
}

/// A CDP object group of the run's session. Every handle a page call returns lives in the group
/// its caller passed, and releasing the group (`Runtime.releaseObjectGroup`) frees them all,
/// `getProperties` children included (probe P1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObjectGroup(pub(crate) String);

/// The run's handle on one frame's utility world.
pub(crate) struct World {
    /// The session of the frame's host: the tab's, or an out-of-process frame's.
    cdp: cdp::Session,
    /// A frame id; only the main frame also uses the tab target id.
    frame_id: String,
    child_host: bool,
    gone: CancellationToken,
    /// The glue this session found in the current document. A mutex, not a lock-free cache, so
    /// concurrent first calls attach once.
    attached: Mutex<Option<Arc<Attached>>>,
    document: AtomicU64,
    /// The run's test id attribute, which picks the glue this world attaches to.
    test_id: Arc<TestIdAttribute>,
}

/// The glue as this session sees it in one document.
struct Attached {
    /// In [`GLUE_GROUP`].
    glue: HandleId,
    context: i64,
    document: u64,
}

enum StaleReply {
    Discard,
    Retain,
}

impl World {
    /// The main frame's world on the tab's session, which `page.rs` attached.
    pub(crate) fn new(session: cdp::Session, frame_id: String) -> Self {
        Self::on_host(session, frame_id, false, CancellationToken::new())
    }

    pub(crate) fn on_host(
        cdp: cdp::Session,
        frame_id: String,
        child_host: bool,
        gone: CancellationToken,
    ) -> Self {
        Self {
            cdp,
            frame_id,
            child_host,
            gone,
            attached: Mutex::new(None),
            document: AtomicU64::new(0),
            test_id: Arc::default(),
        }
    }

    /// This world with the run's test id attribute instead of the default.
    pub(crate) fn with_test_id(mut self, test_id: Arc<TestIdAttribute>) -> Self {
        self.test_id = test_id;
        self
    }

    /// The run's test id attribute, for the worlds of this page's child frames.
    pub(crate) fn test_id(&self) -> Arc<TestIdAttribute> {
        Arc::clone(&self.test_id)
    }

    /// Advance before publishing a commit: reads and attachments already in flight must
    /// retry in the new document. Setup replies are retained separately for cleanup.
    pub(crate) fn document_committed(&self) {
        self.document.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut slot) = self.attached.try_lock() {
            *slot = None;
        }
    }

    /// How many document commits this world has seen. `main_world.rs` keys its main-world
    /// install on it, so a commit retires the main world's `UtilityScript` with the glue.
    pub(crate) fn generation(&self) -> u64 {
        self.document.load(Ordering::SeqCst)
    }

    /// Bind an iframe owner returned by DOM.getFrameOwner to this parent's utility world.
    /// A direct scope has no selector for that owner; the typed handle still lives in the
    /// caller's group and follows the same document checks as a selector result.
    pub(crate) async fn element(
        &self,
        group: &ObjectGroup,
        backend: i64,
    ) -> Result<Handle<Element>, PageError> {
        let attached = self.attached().await?;
        let result = self
            .send(dom::ResolveNode {
                backend_node_id: Some(backend),
                execution_context_id: Some(attached.context),
                object_group: Some(group.0.clone()),
                node_id: None,
            })
            .await;
        let reply = match result {
            Ok(reply) => reply,
            Err(error) => {
                if error == PageError::NotConnected {
                    self.invalidate(&attached).await;
                }
                return Err(error);
            }
        };
        if self.document.load(Ordering::SeqCst) != attached.document {
            return Err(PageError::NotConnected);
        }
        let object = reply.object;
        let id = object.object_id.ok_or_else(|| {
            PageError::Contract(format!("{} returned no objectId", dom::ResolveNode::METHOD))
        })?;
        Handle::new(HandleId::new(id), object.subtype.as_deref()).ok_or_else(|| {
            PageError::Contract(format!(
                "{} returned a non-element owner",
                dom::ResolveNode::METHOD
            ))
        })
    }

    /// Calls glue function `C::NAME` with `params` and its handles, and decodes the result per
    /// `C::Result`: by value, or with `returnByValue: false` into `group` and one
    /// `Runtime.getProperties {ownProperties: true}` of the returned object. `group` is required
    /// exactly when the result holds a handle. Cancel-safe: dropped mid-attach, the document
    /// still ends with one glue object, because install is idempotent.
    ///
    /// # Errors
    ///
    /// [`PageError`] by [`classify`] for CDP errors (a protocol error also clears the cached glue
    /// if it is still the one that failed); `Threw` for `exceptionDetails`; `Contract` for a
    /// result the spec does not allow.
    pub(crate) async fn call<C: InjectedCall>(
        &self,
        group: Option<&ObjectGroup>,
        params: &C,
    ) -> Result<C::Result, PageError> {
        self.call_impl(group, params, StaleReply::Discard)
            .await
            .map(|(result, _)| result)
    }

    /// Decode even an answer that spans a commit. A setup can already have armed a page
    /// resource, so its caller must own and stop it before discarding the stale answer.
    /// The boolean says whether the answer still belongs to the current document.
    pub(crate) async fn call_retaining_reply<C: InjectedCall>(
        &self,
        group: Option<&ObjectGroup>,
        params: &C,
    ) -> Result<(C::Result, bool), PageError> {
        self.call_impl(group, params, StaleReply::Retain).await
    }

    async fn call_impl<C: InjectedCall>(
        &self,
        group: Option<&ObjectGroup>,
        params: &C,
        stale: StaleReply,
    ) -> Result<(C::Result, bool), PageError> {
        let by_value = <C::Result as PageResult>::BY_VALUE;
        let group = match (by_value, group) {
            (true, _) => None,
            (false, Some(group)) => Some(group),
            (false, None) => {
                return Err(PageError::Contract(format!(
                    "page call {} returns a page object and needs an object group",
                    C::NAME
                )));
            }
        };
        let params_json = serde_json::to_value(params).map_err(|error| {
            PageError::Contract(format!("encoding page call {}: {error}", C::NAME))
        })?;
        let mut arguments = vec![argument(json!(C::NAME)), argument(params_json)];
        arguments.extend(
            params
                .handles()
                .into_iter()
                .map(|handle| runtime::CallArgument {
                    object_id: Some(handle.object_id().to_string()),
                    ..runtime::CallArgument::default()
                }),
        );

        let attached = self.attached().await?;
        let request = runtime::CallFunctionOn {
            function_declaration: CALL.to_string(),
            object_id: Some(attached.glue.object_id().to_string()),
            arguments: Some(arguments),
            await_promise: Some(true),
            return_by_value: Some(by_value),
            object_group: group.map(|ObjectGroup(group)| group.clone()),
            ..runtime::CallFunctionOn::default()
        };
        let reply = match self.send(request).await {
            Ok(reply) => reply,
            Err(error) => {
                if error == PageError::NotConnected {
                    self.invalidate(&attached).await;
                }
                return Err(error);
            }
        };
        if matches!(stale, StaleReply::Discard)
            && self.document.load(Ordering::SeqCst) != attached.document
        {
            return Err(PageError::NotConnected);
        }
        let result = evaluated(reply, C::NAME)?;
        let decoded = if by_value {
            <C::Result as PageResult>::from_value(result.value.unwrap_or(Value::Null))
        } else {
            let object_id = result.object_id.ok_or_else(|| {
                PageError::Contract(format!(
                    "page call {} returned {}, not an object",
                    C::NAME,
                    result.r#type
                ))
            })?;
            let properties = self
                .send(runtime::GetProperties {
                    object_id,
                    own_properties: Some(true),
                    ..runtime::GetProperties::default()
                })
                .await?;
            <C::Result as PageResult>::from_properties(split_properties(properties.result)?)
        };
        let current = self.document.load(Ordering::SeqCst) == attached.document;
        if !current && matches!(stale, StaleReply::Discard) {
            return Err(PageError::NotConnected);
        }
        decoded
            .map(|result| (result, current))
            .map_err(|error| PageError::Contract(error.to_string()))
    }

    /// The glue for the current document, attaching first when this session has none.
    /// `Page.createIsolatedWorld` and the bundle calls go untyped where browseros-cdp's surface
    /// has no type (`createIsolatedWorld`).
    async fn attached(&self) -> Result<Arc<Attached>, PageError> {
        let mut slot = self.attached.lock().await;
        let document = self.document.load(Ordering::SeqCst);
        // A document keeps the glue it was first attached to: as Playwright's injected script, it
        // names the attribute that was current then, and a new document takes the run's current one.
        let test_id = self.test_id.get();
        if slot
            .as_ref()
            .is_some_and(|attached| attached.document != document)
        {
            *slot = None;
        }
        if let Some(attached) = slot.as_ref() {
            return Ok(Arc::clone(attached));
        }
        let key = glue_key_named(&test_id)?;
        let world = self
            .send(page::CreateIsolatedWorld {
                frame_id: self.frame_id.clone(),
                world_name: Some(WORLD_NAME.to_string()),
                grant_univeral_access: Some(true),
                content_security_policy: None,
            })
            .await?;
        let context = world.execution_context_id;
        let found = self
            .run_bundle(context, ATTACH, vec![argument(json!(key))])
            .await?;
        let glue = if let Some(glue) = found {
            glue
        } else {
            let arguments = vec![
                argument(json!(key)),
                argument(json!(SPEC_SHA256)),
                argument(json!(initializer_named(&test_id))),
            ];
            self.run_bundle(context, INSTALL, arguments)
                .await?
                .ok_or_else(|| {
                    PageError::Contract("install.js returned no glue object".to_string())
                })?
        };
        if self.document.load(Ordering::SeqCst) != document {
            return Err(PageError::NotConnected);
        }
        let attached = Arc::new(Attached {
            glue,
            context,
            document,
        });
        *slot = Some(Arc::clone(&attached));
        Ok(attached)
    }

    /// Runs a bundle entry by execution context (measured to work on a bare session, probe
    /// P1) and returns the object id it returned, `None` for `undefined`. An exception means
    /// the bundle refused this build (`install.js` checks the spec hash): `Contract`.
    async fn run_bundle(
        &self,
        context: i64,
        function: &str,
        arguments: Vec<runtime::CallArgument>,
    ) -> Result<Option<HandleId>, PageError> {
        let reply = self
            .send(runtime::CallFunctionOn {
                function_declaration: function.to_string(),
                execution_context_id: Some(context),
                arguments: Some(arguments),
                return_by_value: Some(false),
                object_group: Some(GLUE_GROUP.to_string()),
                ..runtime::CallFunctionOn::default()
            })
            .await?;
        match evaluated(reply, "the glue bundle") {
            Ok(result) => Ok(result.object_id.map(HandleId::new)),
            Err(PageError::Threw(message)) => Err(PageError::Contract(message)),
            Err(error) => Err(error),
        }
    }

    /// Clears the cache if it still holds `stale`, so a concurrent call's newer attach survives.
    async fn invalidate(&self, stale: &Arc<Attached>) {
        let mut slot = self.attached.lock().await;
        if slot
            .as_ref()
            .is_some_and(|attached| Arc::ptr_eq(attached, stale))
        {
            *slot = None;
        }
    }

    /// Sends `command` on the world's session. A host whose session detached answers
    /// `NotConnected` at once, and a CDP error is classified as for this world's host. The
    /// command is serialized now, when the send is made ([`cdp::Serialized`]).
    fn send<C: cdp::Command<Via = cdp::OnSession>>(
        &self,
        command: C,
    ) -> impl Future<Output = Result<C::Reply, PageError>> {
        let command = cdp::Serialized::new(command);
        async move {
            tokio::select! {
                biased;
                () = self.gone.cancelled() => Err(PageError::NotConnected),
                result = self.cdp.send_serialized(command) => {
                    result.map_err(|error| classify_host(&error, self.child_host))
                }
            }
        }
    }
}

/// A `Runtime.callFunctionOn` argument by value. A JSON `null` stays a value: only an absent
/// `value` is `undefined` to CDP.
pub(crate) fn argument(value: Value) -> runtime::CallArgument {
    runtime::CallArgument {
        value: Some(value),
        ..runtime::CallArgument::default()
    }
}

/// Reads a `Runtime.callFunctionOn` answer: the result object, or `Threw` with the exception's
/// description (`Error: <message>` for Playwright's stackless errors).
fn evaluated(reply: Value, call: &str) -> Result<RemoteObject, PageError> {
    let answer: CallFunctionOnResult = serde_json::from_value(reply).map_err(|error| {
        PageError::Contract(format!(
            "{} for {call} answered: {error}",
            runtime::CallFunctionOn::METHOD
        ))
    })?;
    if let Some(details) = answer.exception_details {
        let message = details
            .exception
            .and_then(|exception| exception.description)
            .unwrap_or(details.text);
        return Err(PageError::Threw(message));
    }
    Ok(answer.result)
}

/// Playwright's classification of a page-call failure. Pure. A gone session or connection is
/// `Closed`; a protocol error or browseros-cdp's 60 s request timeout is `NotConnected`, which
/// loop A retries, as Playwright retries every error it does not name non-retriable; a reply
/// that does not parse is `Contract`.
pub(crate) fn classify(error: &CdpError) -> PageError {
    match error {
        CdpError::SessionGone
        | CdpError::ConnectionLost
        | CdpError::NotConnected
        | CdpError::ConnectionEpochChanged { .. }
        | CdpError::Discovery(_)
        | CdpError::Transport(_) => PageError::Closed,
        CdpError::Protocol { .. } | CdpError::Timeout { .. } => PageError::NotConnected,
        CdpError::Json(message) => PageError::Contract(message.clone()),
    }
}

/// A vanished OOPIF is a stale frame, while the tab and the call remain alive.
pub(crate) fn classify_host(error: &CdpError, child: bool) -> PageError {
    if child && matches!(error, CdpError::SessionGone) {
        PageError::NotConnected
    } else {
        classify(error)
    }
}

/// Splits `Runtime.getProperties` descriptors into the spec's [`Properties`]: primitives by
/// `value`, objects by `objectId` and `subtype`, `undefined` and `__proto__` dropped. Pure.
fn split_properties(descriptors: Vec<PropertyDescriptor>) -> Result<Properties, PageError> {
    let mut properties = Properties::default();
    for descriptor in descriptors {
        if descriptor.name == "__proto__" {
            continue;
        }
        let Some(value) = descriptor.value else {
            return Err(PageError::Contract(format!(
                "the page result's `{}` is an accessor, not a value",
                descriptor.name
            )));
        };
        if value.r#type == "undefined" {
            continue;
        }
        if let Some(object_id) = value.object_id {
            properties
                .handles
                .insert(descriptor.name, (HandleId::new(object_id), value.subtype));
            continue;
        }
        let primitive = match (value.value, value.unserializable_value.as_deref()) {
            (Some(primitive), _) => primitive,
            // An integral `-0` reads as 0, as CDP writes it for a by-value result.
            (None, Some("-0")) => json!(0),
            (None, Some(unserializable)) => {
                return Err(PageError::Contract(format!(
                    "the page result's `{}` is {unserializable}",
                    descriptor.name
                )));
            }
            // `null` arrives as an object without an id or a value.
            (None, None) => Value::Null,
        };
        properties.values.insert(descriptor.name, primitive);
    }
    Ok(properties)
}

#[cfg(test)]
mod tests {
    use neo_playwright_protocol::injected::{
        HitTargetSetup, InjectedResolveResult, InjectedSetupHitTargetInterceptorResult,
    };
    use neo_playwright_protocol::{Element, Handle};

    use super::*;

    fn descriptors(value: Value) -> Vec<PropertyDescriptor> {
        serde_json::from_value(value).expect("recorded descriptors parse")
    }

    #[test]
    fn classify_sorts_every_cdp_error() {
        let cases = [
            (CdpError::SessionGone, PageError::Closed),
            (CdpError::ConnectionLost, PageError::Closed),
            (CdpError::NotConnected, PageError::Closed),
            (
                CdpError::ConnectionEpochChanged {
                    expected: 1,
                    actual: 2,
                },
                PageError::Closed,
            ),
            (CdpError::Transport("reset".to_string()), PageError::Closed),
            (
                CdpError::Discovery("refused".to_string()),
                PageError::Closed,
            ),
            (
                CdpError::Protocol {
                    code: -32000,
                    message: "Cannot find context with specified id".to_string(),
                },
                PageError::NotConnected,
            ),
            (
                CdpError::Timeout {
                    method: "Runtime.callFunctionOn".to_string(),
                },
                PageError::NotConnected,
            ),
            (
                CdpError::Json("eof".to_string()),
                PageError::Contract("eof".to_string()),
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(classify(&error), expected, "{error:?}");
        }
    }

    #[test]
    fn child_session_loss_is_a_disconnected_frame() {
        assert_eq!(
            classify_host(&CdpError::SessionGone, true),
            PageError::NotConnected
        );
        assert_eq!(
            classify_host(&CdpError::SessionGone, false),
            PageError::Closed
        );
    }

    #[tokio::test]
    async fn a_detached_child_interrupts_a_pending_world_call() {
        use crate::fake_browser::{FakeBrowser, PAGE, Replies, Reply};
        use neo_playwright_protocol::injected::InjectedTitleParams;
        use tokio::task;

        let browser = FakeBrowser::start(Replies {
            world: Reply {
                answers: 0,
                ..Reply::default()
            },
            ..Replies::default()
        })
        .await;
        let attached = cdp::Browser::new(browser.cdp().await)
            .attach(PAGE)
            .await
            .unwrap();
        let gone = CancellationToken::new();
        let world = World::on_host(attached, PAGE.to_string(), true, gone.clone());
        let pending = world.call(None, &InjectedTitleParams {});
        tokio::pin!(pending);
        tokio::select! { biased; result = &mut pending => panic!("world should be pending: {result:?}"), () = task::yield_now() => {} }
        gone.cancel();
        assert_eq!(pending.await, Err(PageError::NotConnected));
    }

    /// `resolve`'s answer as BrowserOS neo gives it (probe P1's shapes), decoded end to end.
    #[test]
    fn a_resolve_answer_splits_into_its_element_count_and_preview() {
        let properties = split_properties(descriptors(json!([
            { "name": "element", "configurable": true, "enumerable": true, "isOwn": true,
              "value": { "type": "object", "subtype": "node", "className": "HTMLButtonElement",
                         "description": "button", "objectId": "-66.2.2" } },
            { "name": "count", "configurable": true, "enumerable": true, "isOwn": true,
              "value": { "type": "number", "value": 2, "description": "2" } },
            { "name": "preview", "configurable": true, "enumerable": true, "isOwn": true,
              "value": { "type": "string", "value": "<button>Submit</button>" } },
            { "name": "__proto__", "configurable": true, "enumerable": false, "isOwn": true,
              "value": { "type": "object", "className": "Object", "objectId": "-66.2.3" } },
        ])))
        .expect("splits");
        let resolved = InjectedResolveResult::from_properties(properties).expect("decodes");
        assert_eq!(
            resolved,
            InjectedResolveResult {
                element: Handle::<Element>::new(HandleId::new("-66.2.2".to_string()), Some("node")),
                count: 2,
                preview: Some("<button>Submit</button>".to_string()),
            }
        );
    }

    #[test]
    fn nothing_matched_drops_the_undefined_properties() {
        let properties = split_properties(descriptors(json!([
            { "name": "element", "configurable": true, "enumerable": true,
              "value": { "type": "undefined" } },
            { "name": "count", "configurable": true, "enumerable": true,
              "value": { "type": "number", "value": 0 } },
            { "name": "preview", "configurable": true, "enumerable": true,
              "value": { "type": "undefined" } },
        ])))
        .expect("splits");
        let resolved = InjectedResolveResult::from_properties(properties).expect("decodes");
        assert_eq!(
            (resolved.element, resolved.count, resolved.preview),
            (None, 0, None)
        );
    }

    #[test]
    fn an_armed_interceptor_is_a_plain_object_handle_and_a_refusal_a_string() {
        let armed = split_properties(descriptors(json!([
            { "name": "outcome", "configurable": true, "enumerable": true,
              "value": { "type": "object", "className": "Object", "objectId": "-66.4.9" } },
        ])))
        .expect("splits");
        let armed =
            InjectedSetupHitTargetInterceptorResult::from_properties(armed).expect("decodes");
        assert!(
            matches!(armed.outcome, HitTargetSetup::Armed(_)),
            "{armed:?}"
        );

        let refused = split_properties(descriptors(json!([
            { "name": "outcome", "configurable": true, "enumerable": true,
              "value": { "type": "string", "value": "<div id=\"overlay\"></div>" } },
        ])))
        .expect("splits");
        let refused =
            InjectedSetupHitTargetInterceptorResult::from_properties(refused).expect("decodes");
        assert_eq!(
            refused.outcome,
            HitTargetSetup::Description("<div id=\"overlay\"></div>".to_string())
        );
    }

    #[test]
    fn an_accessor_or_a_nan_is_outside_the_contract() {
        let accessor = descriptors(json!([
            { "name": "count", "configurable": true, "enumerable": true,
              "get": { "type": "function", "objectId": "-66.1.1" } },
        ]));
        assert!(matches!(
            split_properties(accessor),
            Err(PageError::Contract(_))
        ));
        let nan = descriptors(json!([
            { "name": "count", "configurable": true, "enumerable": true,
              "value": { "type": "number", "unserializableValue": "NaN" } },
        ]));
        assert!(matches!(split_properties(nan), Err(PageError::Contract(_))));
    }

    #[test]
    fn the_glue_key_is_the_spec_hash_and_the_initializer() {
        let key = glue_key().expect("encodes");
        assert_eq!(
            key,
            format!(
                "{SPEC_SHA256}/{{\"frameSeq\":0,\"testIdAttributeName\":\"data-testid\",\"stableRafCount\":1,\"isUtilityWorld\":true}}"
            ),
            "neo-playwright-injected's load test builds the same key"
        );
    }

    #[test]
    fn a_page_exception_reads_as_its_description() {
        let threw = evaluated(
            json!({
                "result": { "type": "object", "subtype": "error" },
                "exceptionDetails": {
                    "exceptionId": 1, "text": "Uncaught", "lineNumber": 0, "columnNumber": 0,
                    "exception": { "type": "object", "subtype": "error",
                                   "description": "Error: strict mode violation: …" },
                },
            }),
            "resolve",
        );
        assert_eq!(
            threw.map(|_| ()),
            Err(PageError::Threw(
                "Error: strict mode violation: …".to_string()
            ))
        );
    }
}
