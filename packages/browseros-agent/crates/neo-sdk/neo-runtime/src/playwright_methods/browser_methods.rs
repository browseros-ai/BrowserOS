//! Neo: the `Browser` a run's script reaches (`context.browser()`, `playwright.chromium`'s
//! connected browser): the BrowserOS the run is attached to, `browser@default`. Neo never
//! launches or closes a browser. What a run can create is contexts: `browser.newContext` makes
//! an isolated browser context (`Target.createBrowserContext`), its pages are ordinary tabs of
//! the run's registry (`tabs.rs`), and the run disposes every context it created when it ends.
//!
//! ```text
//!  browser.newContext(o)  validate o → Target.createBrowserContext   (a tracked task: a late
//!                         reply still registers, and disposes after the run's end)
//!  context@<id>.newPage   Target.createTarget{browserContextId} → register → attach →
//!                         emulation::prepare (the context's options) → URL committed
//!  context@<id>.pages     Target.getTargets, the context's pages in first-seen order
//!  context@<id>.close     Target.disposeBrowserContext; its pages read closed
//!  browser.close(),       disconnect: no more creations, subscriptions end, the run's other
//!  context@default.close  calls stop, every context the run created is disposed; every tab of
//!                         the run's own context and BrowserOS stay open
//!  run end                Dispatcher::finish → dispose_all; a dropped run → Drop disposes
//! ```
//!
//! Deviation: `server/chromium/crBrowser.ts` `CRBrowserContext.doClose`: `await this._browser._session.send('Target.disposeBrowserContext', { browserContextId: this._browserContextId });`
//! Playwright keeps a context until the script closes it; a run's realm is gone when it ends, so
//! Neo disposes every context the run created then, with its pages.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use browseros_cdp::CdpError;
use futures_util::future;
use neo_playwright_protocol::serve::{
    BrowserCall, BrowserContextCall, BrowserContextChannel, BrowserContextNewPageResult,
    BrowserNewContextParams, BrowserNewContextParamsColorScheme, BrowserNewContextParamsContrast,
    BrowserNewContextParamsForcedColors, BrowserNewContextParamsReducedMotion,
    BrowserNewContextParamsServiceWorkers, BrowserNewContextResult, PageChannel,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::{OnceCell, oneshot};
use tokio_util::task::TaskTracker;
use tracing::debug;

use crate::browser_representations::emulation::{ContextEmulation, EmulatedMedia, Size};
use crate::browser_representations::page::PageSession;
use crate::browser_representations::tabs::{self, CreatedPage, PageEntry, Tabs};
use crate::chromium_bridge::cdp;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::quickjs_bridge;
use crate::run::PageId;

/// Neo: the guid of the run's `Browser`, the BrowserOS it is attached to.
pub(crate) const BROWSER_GUID: &str = "browser@default";

/// Neo: Playwright's device descriptors at the pin, vendored unchanged as
/// `neo-quickjs-injected/vendor/playwright/isomorphic/deviceDescriptorsSource.json` (its VENDOR.md
/// hashes them) and handed to the realm when a script first reads `playwright.devices`.
pub(crate) const DEVICE_DESCRIPTORS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-quickjs-injected/vendor/playwright/isomorphic/deviceDescriptorsSource.json"
));

/// Neo: the guid prefix of a context the run created, `context@<browserContextId>`.
const CONTEXT_PREFIX: &str = "context@";

/// Playwright: `server/browserContext.ts` `validateBrowserContextOptions`: `options.viewport = { width: 1280, height: 720 };`
const DEFAULT_VIEWPORT: Size = Size {
    width: 1280,
    height: 720,
};

/// Neo: the first backoff between URL reads while a new page's tab reports no URL yet.
const URL_BACKOFF_START: Duration = Duration::from_millis(1);
/// Neo: the longest backoff between those reads.
const URL_BACKOFF_MAX: Duration = Duration::from_millis(16);

/// The contexts a run created, and what the run read of the browser once. Lives in the run's
/// [`Tabs`], so calls and getters reach the same contexts.
pub(crate) struct CreatedContexts {
    cdp: cdp::Browser,
    tokio: Handle,
    registry: Mutex<Registry>,
    version: OnceCell<Version>,
    /// The creations still waiting for the browser's answer, which [`disconnect`] settles.
    creating: TaskTracker,
}

#[derive(Default)]
struct Registry {
    contexts: Vec<Arc<CreatedContext>>,
    /// Set when the run disposes its contexts: a creation answered later disposes its own.
    finished: bool,
    /// Set once the run disconnected ([`disconnect`]): it creates no context after that.
    disconnected: bool,
}

/// One context `browser.newContext` created.
pub(crate) struct CreatedContext {
    /// The browser's `browserContextId`.
    pub(crate) id: String,
    /// `context@<id>`.
    pub(crate) guid: String,
    pub(crate) emulation: Arc<ContextEmulation>,
    disposed: AtomicBool,
}

/// What `Browser.getVersion` says, read once per run.
#[derive(Clone)]
struct Version {
    version: String,
    headful: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetsReply {
    target_infos: Vec<TargetInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetInfo {
    target_id: String,
    #[serde(rename = "type")]
    kind: String,
    url: String,
    #[serde(default)]
    browser_context_id: Option<String>,
}

impl CreatedContexts {
    pub(crate) fn new(cdp: cdp::Browser, tokio: Handle) -> Self {
        Self {
            cdp,
            tokio,
            registry: Mutex::default(),
            version: OnceCell::new(),
            creating: TaskTracker::new(),
        }
    }

    /// The context `guid` names, if the run created it (disposed ones included).
    pub(crate) fn find(&self, guid: &str) -> Option<Arc<CreatedContext>> {
        let id = guid.strip_prefix(CONTEXT_PREFIX)?;
        self.registry
            .lock()
            .ok()?
            .contexts
            .iter()
            .find(|context| context.id == id)
            .cloned()
    }

    fn live(&self) -> Vec<Arc<CreatedContext>> {
        self.registry.lock().map_or_else(
            |_| Vec::new(),
            |registry| {
                registry
                    .contexts
                    .iter()
                    .filter(|context| !context.is_disposed())
                    .cloned()
                    .collect()
            },
        )
    }

    /// Playwright's `CRBrowser.connect` reads of `Browser.getVersion`.
    async fn version(&self) -> Result<Version, PlaywrightError> {
        self.version
            .get_or_try_init(|| async {
                let reply = self
                    .cdp
                    .send(cdp::browser::GetVersion {})
                    .await
                    .map_err(|error| tabs::cdp_error(&error))?;
                Ok(Version {
                    // Playwright: `server/chromium/crBrowser.ts` `CRBrowser.connect`: `browser._version = version.product.substring(version.product.indexOf('/') + 1);`
                    version: reply
                        .product
                        .split_once('/')
                        .map_or_else(|| reply.product.clone(), |(_, version)| version.to_string()),
                    // Playwright: `server/chromium/crBrowser.ts` `CRBrowser.connect`: `browser.options.headful = !version.userAgent.includes('Headless');`
                    headful: !reply.user_agent.contains("Headless"),
                })
            })
            .await
            .cloned()
    }
}

impl Drop for CreatedContexts {
    /// A run whose future was dropped never reaches `Dispatcher::finish`: its contexts are
    /// disposed here, in the background, so they do not outlive it in the user's browser.
    fn drop(&mut self) {
        let Ok(registry) = self.registry.get_mut() else {
            return;
        };
        for context in &registry.contexts {
            if context.disposed.swap(true, Ordering::AcqRel) {
                continue;
            }
            let cdp = self.cdp.clone();
            let id = context.id.clone();
            self.tokio.spawn(async move {
                if let Err(error) = dispose(&cdp, &id).await {
                    debug!(error = %error.message, "disposing a context after its run was dropped failed");
                }
            });
        }
    }
}

impl CreatedContext {
    fn is_disposed(&self) -> bool {
        self.disposed.load(Ordering::Acquire)
    }
}

/// A `Browser` call.
pub(crate) async fn browser(
    tabs: &Tabs,
    call: BrowserCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    match call {
        BrowserCall::NewContext(params) => new_context(tabs, &params, progress).await,
        BrowserCall::Close(_) => {
            disconnect(tabs, progress).await?;
            Ok(None)
        }
    }
}

/// A `BrowserContext` call on a context the run created.
pub(crate) async fn context(
    tabs: &Tabs,
    context: &Arc<CreatedContext>,
    call: BrowserContextCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    match call {
        BrowserContextCall::NewPage(_) => new_page(tabs, context, progress).await,
        BrowserContextCall::Close(_) => {
            close_context(tabs, context, progress).await?;
            Ok(None)
        }
        // Playwright: `server/dispatchers/browserContextDispatcher.ts` `BrowserContextDispatcher.pause`: `// Debugger will take care of this.`
        // `setTestIdAttributeName`: applied for the run before the call started (`Dispatcher::call`).
        BrowserContextCall::Pause(_) | BrowserContextCall::SetTestIdAttributeName(_) => Ok(None),
        // Session state (`cookies_and_storage.rs`) is the run's own context's.
        call @ (BrowserContextCall::AddCookies(_)
        | BrowserContextCall::Cookies(_)
        | BrowserContextCall::StorageState(_)
        | BrowserContextCall::SetStorageState(_)
        | BrowserContextCall::SetExtraHTTPHeaders(_)
        | BrowserContextCall::SetGeolocation(_)
        | BrowserContextCall::SetOffline(_)
        | BrowserContextCall::SetHTTPCredentials(_)
        | BrowserContextCall::GrantPermissions(_)
        | BrowserContextCall::ClearPermissions(_)) => Err(created_state(call.meta().method).into()),
        // Deviation: `client/browserContext.ts` `BrowserContext.route` Neo serves context routes only on the run's own context; a created context's page routes remain available.
        call @ BrowserContextCall::SetNetworkInterceptionPatterns(_) => {
            Err(created_state(call.meta().method).into())
        }
    }
}

/// Neo: session state, bindings and context routes on a context the run created, which Neo does not serve yet:
/// the run's own context's cookies, storage, headers, offline mode, geolocation, permissions and
/// bindings never reach a created context's pages, and the call fails instead of answering
/// with them.
pub(crate) fn created_state(method: &str) -> PlaywrightError {
    PlaywrightError::error(format!(
        "neo-runtime: {method} is not served yet for a context browser.newContext created, \
         only for the run's own context"
    ))
}

/// Playwright's `Browser.newContext`: validates the options, creates the context and answers
/// its guid. The context has no page yet.
async fn new_context(
    tabs: &Tabs,
    params: &BrowserNewContextParams,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let created = tabs.created();
    if created
        .registry
        .lock()
        .map_or(true, |registry| registry.disconnected)
    {
        return Err(PlaywrightError::target_closed().into());
    }
    let mut emulation = context_emulation(params)?;
    emulation.headful = progress.race(created.version()).await??.headful;
    let emulation = Arc::new(emulation);
    let (reply, answer) = oneshot::channel();
    let cdp = created.cdp.clone();
    let owner = tabs.clone();
    // The browser owns the context once it receives the command. This task reads the answer and
    // registers the context even after the waiting call is cancelled, so the run still disposes
    // it; one answered after the run disposed its contexts, or disconnected, disposes itself.
    tabs.spawn(created.creating.track_future(async move {
        // Playwright: `server/chromium/crBrowser.ts` `CRBrowser.doCreateNewContext`: `disposeOnDetach: true,`
        let sent = cdp
            .send(cdp::target::CreateBrowserContext {
                dispose_on_detach: Some(true),
                ..cdp::target::CreateBrowserContext::default()
            })
            .await;
        let result = match sent {
            Ok(created) => register(&owner, created.browser_context_id, emulation).await,
            Err(error) => Err(tabs::cdp_error(&error)),
        };
        let _waiting = reply.send(result);
    }));
    let context = progress
        .race(answer)
        .await?
        .map_err(|_| PlaywrightError::error("creating a context: the run ended first"))??;
    encode(&BrowserNewContextResult {
        context: BrowserContextChannel {
            guid: context.guid.clone(),
        },
    })
}

/// Records a created context in the run, or disposes it when the run already disposed its own or
/// disconnected: Playwright's connected browser rejects a creation still pending at its close,
/// and no context outlives it.
async fn register(
    tabs: &Tabs,
    id: String,
    emulation: Arc<ContextEmulation>,
) -> Result<Arc<CreatedContext>, PlaywrightError> {
    let context = Arc::new(CreatedContext {
        guid: format!("{CONTEXT_PREFIX}{id}"),
        id,
        emulation,
        disposed: AtomicBool::new(false),
    });
    let late = match tabs.created().registry.lock() {
        Ok(mut registry) => {
            let late = registry.finished || registry.disconnected;
            registry.contexts.push(Arc::clone(&context));
            late
        }
        Err(_) => true,
    };
    if late {
        context.disposed.store(true, Ordering::Release);
        if let Err(error) = dispose(&tabs.created().cdp, &context.id).await {
            debug!(error = %error.message, "disposing a context created after its run ended failed");
        }
        return Err(PlaywrightError::target_closed());
    }
    Ok(context)
}

/// Playwright's `BrowserContext.newPage` in a created context (`CRBrowserContext.doCreateNewPage`):
/// a tab of the context on `about:blank`, its options emulated before the page is answered.
async fn new_page(
    tabs: &Tabs,
    context: &Arc<CreatedContext>,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    if context.is_disposed() {
        return Err(PlaywrightError::target_closed().into());
    }
    // Playwright: `server/chromium/crBrowser.ts` `CRBrowserContext.doCreateNewPage`: `const { targetId } = await this._browser._session.send('Target.createTarget', { url: 'about:blank', browserContextId: this._browserContextId });`
    let created = progress
        .race(tabs.cdp().send(cdp::target::CreateTarget {
            url: "about:blank".to_string(),
            browser_context_id: Some(context.id.clone()),
            ..cdp::target::CreateTarget::default()
        }))
        .await?
        .map_err(|error| tabs::cdp_error(&error))?;
    // A tab created under a cancelled call stays registered, so the run's outcome lists it and
    // the context's disposal closes it.
    let entry = tabs.register_in_context(PageId::new(created.target_id), true, context)?;
    let session = progress.race(tabs.session(&entry)).await??;
    let mut backoff = URL_BACKOFF_START;
    loop {
        let url = progress
            .race(tabs.url(&entry))
            .await??
            .ok_or_else(PlaywrightError::target_closed)?;
        if !url.is_empty() {
            break;
        }
        progress.wait(backoff).await?;
        backoff = (backoff * 2).min(URL_BACKOFF_MAX);
    }
    // The context's init scripts reach the page's first document before the page is handed
    // out, as a new page's `FrameSession` runs them in Playwright.
    let scripts = progress.init_scripts();
    if scripts.has_scripts() {
        progress
            .race(scripts.show_new(tabs.cdp(), &session, &context.id))
            .await?;
    }
    encode(&BrowserContextNewPageResult {
        page: PageChannel {
            guid: quickjs_bridge::page_guid(&entry.id),
        },
    })
}

/// Playwright's `BrowserContext.close` of a created context (`CRBrowserContext.doClose`): the
/// context and its pages go. Closing a closed context does nothing.
async fn close_context(
    tabs: &Tabs,
    context: &Arc<CreatedContext>,
    progress: &Progress,
) -> Result<(), CallError> {
    if context.disposed.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let pages = tabs.context_entries(context);
    for page in &pages {
        page.request_close();
    }
    let disposed = progress.race(dispose(tabs.cdp(), &context.id)).await?;
    for page in &pages {
        page.closed.cancel();
        page.closed_by_run();
    }
    disposed.map_err(CallError::from)
}

/// Playwright's close of its connected browser, which closing the run's own context is too: a
/// connectOverCDP default context closes by closing its browser (`CRBrowserContext.doClose`), and
/// that close only disconnects, every tab left open (measured on BrowserOS neo,
/// a2-mapping-evidence.md §2). The run lets go of what it holds in the browser, in this order:
/// it creates no context from now on (a creation still pending disposes its own, [`register`]); a
/// `beforeunload` dialog a listener holds is dismissed, so its tab stays; every subscription ends
/// with target discovery; every other call of the run stops, as Playwright's calls stop with its
/// connection; a creation still pending settles, disposing its own context; and every context the
/// run created closes with its pages. The SDK closes the run's objects as the script sees them;
/// the reason stays there.
///
/// Never half-closed: a context the browser fails to dispose is left to the run's end, which
/// disposes it again ([`dispose_all`]).
pub(crate) async fn disconnect(tabs: &Tabs, progress: &Progress) -> Result<(), CallError> {
    if let Ok(mut registry) = tabs.created().registry.lock() {
        registry.disconnected = true;
    }
    for (_, session) in tabs.attached() {
        if session.events.dialogs.take_open_beforeunload() {
            let dismissed = progress
                .race(session.send_command(cdp::page::HandleJavaScriptDialog {
                    accept: false,
                    prompt_text: None,
                }))
                .await?;
            if let Err(error) = dismissed {
                debug!(%error, "dismissing a beforeunload dialog as the run disconnected failed");
            }
        }
    }
    tabs.events().end_subscriptions();
    // The script's promises for them have rejected (the SDK); this stops the calls themselves
    // before the close resolves, so none writes input to a tab the run let go of.
    progress.stop_the_runs_calls();
    // Then what the run set on the tabs it leaves open goes, before the close resolves, as
    // Playwright's overrides go with its connection (`Tabs::release_run_scoped`, #3223's D1).
    tabs.release_run_scoped(progress.run_scope()).await;
    let created = tabs.created();
    created.creating.close();
    progress.race(created.creating.wait()).await?;
    let contexts = created.live();
    let closes = contexts
        .iter()
        .map(|context| close_context(tabs, context, progress));
    for (context, closed) in contexts.iter().zip(future::join_all(closes).await) {
        match closed {
            Ok(()) => {}
            Err(CallError::Playwright(error)) => {
                context.disposed.store(false, Ordering::Release);
                debug!(error = %error.message, "disposing a context as the run disconnected failed");
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Disposes every context the run created and not yet closed, once its realm is gone
/// (`Dispatcher::finish`, within its budget).
pub(crate) async fn dispose_all(tabs: &Tabs) {
    let contexts = match tabs.created().registry.lock() {
        Ok(mut registry) => {
            registry.finished = true;
            registry.contexts.clone()
        }
        Err(_) => return,
    };
    let disposals = contexts.into_iter().filter_map(|context| {
        if context.disposed.swap(true, Ordering::AcqRel) {
            return None;
        }
        let pages = tabs.context_entries(&context);
        Some(async move {
            for page in &pages {
                page.request_close();
            }
            match dispose(tabs.cdp(), &context.id).await {
                Ok(()) => {
                    for page in &pages {
                        page.closed.cancel();
                        page.closed_by_run();
                    }
                }
                Err(error) => {
                    debug!(error = %error.message, "disposing a context at run end failed");
                }
            }
        })
    });
    future::join_all(disposals).await;
}

/// `Target.disposeBrowserContext`. A context the browser no longer has is disposed already.
async fn dispose(cdp: &cdp::Browser, id: &str) -> Result<(), PlaywrightError> {
    match cdp
        .send(cdp::target::DisposeBrowserContext {
            browser_context_id: id.to_string(),
        })
        .await
    {
        Ok(_) => Ok(()),
        Err(CdpError::Protocol { message, .. }) if message.contains("Failed to find context") => {
            Ok(())
        }
        Err(error) => Err(tabs::cdp_error(&error)),
    }
}

/// The emulation a created page of `entry`'s context runs under, applied to its fresh session
/// before any call on the page (`Tabs::session`). A page of the run's own context gets nothing.
pub(crate) async fn prepare(
    entry: &PageEntry,
    session: &PageSession,
) -> Result<(), PlaywrightError> {
    let Some(context) = entry.context.get() else {
        return Ok(());
    };
    session
        .emulation
        .prepare(session, Arc::clone(&context.emulation))
        .await
        .map_err(|error| tabs::cdp_error(&error))
}

/// `version()`: the browser's version as Playwright reports it.
pub(crate) async fn version(tabs: &Tabs) -> Result<Value, PlaywrightError> {
    Ok(json!(tabs.created().version().await?.version))
}

/// `pages()` of a created context: its open tabs, in the order the run first saw them.
pub(crate) async fn context_pages(
    tabs: &Tabs,
    context: &Arc<CreatedContext>,
) -> Result<Vec<CreatedPage>, PlaywrightError> {
    if context.is_disposed() {
        return Ok(Vec::new());
    }
    let reply = tabs
        .cdp()
        .send(cdp::target::GetTargets::default())
        .await
        .map_err(|error| tabs::cdp_error(&error))?;
    let reply: TargetsReply =
        serde_json::from_value(reply).map_err(|error| tabs::cdp_error(&CdpError::from(error)))?;
    let live: HashMap<String, String> = reply
        .target_infos
        .into_iter()
        .filter(|target| {
            target.kind == "page" && target.browser_context_id.as_deref() == Some(&context.id)
        })
        .map(|target| (target.target_id, target.url))
        .collect();
    tabs.live_context_pages(context, live)
}

/// Playwright's `validateBrowserContextOptions` and the options Neo serves, as the emulation a
/// created context's pages run under. An option Neo does not serve is refused by name.
fn context_emulation(
    params: &BrowserNewContextParams,
) -> Result<ContextEmulation, PlaywrightError> {
    refuse_unserved(params)?;
    let no_default_viewport = params.no_default_viewport == Some(true);
    if no_default_viewport && params.device_scale_factor.is_some() {
        // Playwright: `server/browserContext.ts` `validateBrowserContextOptions`: `"deviceScaleFactor" option is not supported with null "viewport"`
        return Err(PlaywrightError::error(
            "\"deviceScaleFactor\" option is not supported with null \"viewport\"",
        ));
    }
    if no_default_viewport && params.is_mobile == Some(true) {
        // Playwright: `server/browserContext.ts` `validateBrowserContextOptions`: `"isMobile" option is not supported with null "viewport"`
        return Err(PlaywrightError::error(
            "\"isMobile\" option is not supported with null \"viewport\"",
        ));
    }
    let viewport = (!no_default_viewport).then(|| {
        params
            .viewport
            .as_ref()
            .map_or(DEFAULT_VIEWPORT, |viewport| Size {
                width: viewport.width,
                height: viewport.height,
            })
    });
    Ok(ContextEmulation {
        viewport,
        screen: params.screen.as_ref().map(|screen| Size {
            width: screen.width,
            height: screen.height,
        }),
        device_scale_factor: params.device_scale_factor,
        is_mobile: params.is_mobile == Some(true),
        has_touch: params.has_touch == Some(true),
        user_agent: params.user_agent.clone(),
        locale: params.locale.clone(),
        timezone_id: params.timezone_id.clone(),
        media: EmulatedMedia {
            media: None,
            color_scheme: params.color_scheme.map(color_scheme_literal),
            reduced_motion: params.reduced_motion.map(reduced_motion_literal),
            forced_colors: params.forced_colors.map(forced_colors_literal),
            contrast: params.contrast.map(contrast_literal),
        },
        java_script_enabled: params.java_script_enabled,
        bypass_csp: params.bypass_csp == Some(true),
        ignore_https_errors: params.ignore_https_errors == Some(true),
        headful: false,
    })
}

/// Deviation: `server/browser.ts` `Browser.newContext`: `await context.setStorageState(progress, options.storageState, 'initial');`
/// Neo serves a created context's emulation options. Its network and session state (proxy,
/// storage state, cookies' origins, headers, permissions, geolocation, offline mode, HTTP and
/// client credentials, downloads, service worker blocking, HAR and video recording) and the
/// client-side `baseURL`, `strictSelectors` and custom selector engines are not served yet, and
/// each one given is refused by name instead of being ignored.
fn refuse_unserved(params: &BrowserNewContextParams) -> Result<(), PlaywrightError> {
    let given = [
        ("proxy", params.proxy.is_some()),
        ("storageState", params.storage_state.is_some()),
        ("geolocation", params.geolocation.is_some()),
        ("permissions", params.permissions.is_some()),
        ("extraHTTPHeaders", params.extra_http_headers.is_some()),
        ("offline", params.offline.is_some()),
        ("httpCredentials", params.http_credentials.is_some()),
        ("clientCertificates", params.client_certificates.is_some()),
        ("acceptDownloads", params.accept_downloads.is_some()),
        (
            "serviceWorkers",
            params.service_workers == Some(BrowserNewContextParamsServiceWorkers::Block),
        ),
        ("recordVideo", params.record_video.is_some()),
        ("baseURL", params.base_url.is_some()),
        ("strictSelectors", params.strict_selectors == Some(true)),
        (
            "selectorEngines",
            params
                .selector_engines
                .as_ref()
                .is_some_and(|engines| !engines.is_empty()),
        ),
        (
            "testIdAttributeName",
            params.test_id_attribute_name.is_some(),
        ),
    ];
    match given.into_iter().find(|(_, given)| *given) {
        Some((name, _)) => Err(PlaywrightError::error(format!(
            "Neo does not support the \"{name}\" option of a new context yet"
        ))),
        None => Ok(()),
    }
}

fn color_scheme_literal(value: BrowserNewContextParamsColorScheme) -> &'static str {
    match value {
        BrowserNewContextParamsColorScheme::Dark => "dark",
        BrowserNewContextParamsColorScheme::Light => "light",
        BrowserNewContextParamsColorScheme::NoPreference => "no-preference",
        BrowserNewContextParamsColorScheme::NoOverride => "no-override",
    }
}

fn reduced_motion_literal(value: BrowserNewContextParamsReducedMotion) -> &'static str {
    match value {
        BrowserNewContextParamsReducedMotion::Reduce => "reduce",
        BrowserNewContextParamsReducedMotion::NoPreference => "no-preference",
        BrowserNewContextParamsReducedMotion::NoOverride => "no-override",
    }
}

fn forced_colors_literal(value: BrowserNewContextParamsForcedColors) -> &'static str {
    match value {
        BrowserNewContextParamsForcedColors::Active => "active",
        BrowserNewContextParamsForcedColors::None => "none",
        BrowserNewContextParamsForcedColors::NoOverride => "no-override",
    }
}

fn contrast_literal(value: BrowserNewContextParamsContrast) -> &'static str {
    match value {
        BrowserNewContextParamsContrast::NoPreference => "no-preference",
        BrowserNewContextParamsContrast::More => "more",
        BrowserNewContextParamsContrast::NoOverride => "no-override",
    }
}

fn encode<T: serde::Serialize>(result: &T) -> Result<Option<Value>, CallError> {
    serde_json::to_value(result)
        .map(Some)
        .map_err(|error| PlaywrightError::error(error.to_string()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(value: Value) -> BrowserNewContextParams {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn options_map_to_the_emulation_playwright_applies() {
        let emulation = context_emulation(&params(json!({}))).unwrap();
        assert_eq!(emulation.viewport, Some(DEFAULT_VIEWPORT));
        assert_eq!(emulation.media, EmulatedMedia::default());
        let emulation = context_emulation(&params(json!({
            "viewport": {"width": 390, "height": 664},
            "screen": {"width": 390, "height": 844},
            "deviceScaleFactor": 3,
            "isMobile": true,
            "hasTouch": true,
            "userAgent": "UA",
            "locale": "de-DE",
            "timezoneId": "Europe/Berlin",
            "colorScheme": "dark",
            "reducedMotion": "no-override",
            "javaScriptEnabled": false,
            "bypassCSP": true,
            "ignoreHTTPSErrors": true,
            "serviceWorkers": "allow",
            "strictSelectors": false,
            "defaultBrowserType": "webkit",
        })))
        .unwrap();
        assert_eq!(
            emulation,
            ContextEmulation {
                viewport: Some(Size {
                    width: 390,
                    height: 664
                }),
                screen: Some(Size {
                    width: 390,
                    height: 844
                }),
                device_scale_factor: Some(3.0),
                is_mobile: true,
                has_touch: true,
                user_agent: Some("UA".into()),
                locale: Some("de-DE".into()),
                timezone_id: Some("Europe/Berlin".into()),
                media: EmulatedMedia {
                    color_scheme: Some("dark"),
                    reduced_motion: Some("no-override"),
                    ..EmulatedMedia::default()
                },
                java_script_enabled: Some(false),
                bypass_csp: true,
                ignore_https_errors: true,
                headful: false,
            }
        );
        let no_viewport = context_emulation(&params(json!({"noDefaultViewport": true}))).unwrap();
        assert_eq!(no_viewport.viewport, None);
    }

    #[test]
    fn invalid_and_unserved_options_are_refused_by_name() {
        for (options, message) in [
            (
                json!({"noDefaultViewport": true, "deviceScaleFactor": 2}),
                "\"deviceScaleFactor\" option is not supported with null \"viewport\"",
            ),
            (
                json!({"noDefaultViewport": true, "isMobile": true}),
                "\"isMobile\" option is not supported with null \"viewport\"",
            ),
            (
                json!({"proxy": {"server": "http://p:1"}}),
                "Neo does not support the \"proxy\" option of a new context yet",
            ),
            (
                json!({"serviceWorkers": "block"}),
                "Neo does not support the \"serviceWorkers\" option of a new context yet",
            ),
            (
                json!({"extraHTTPHeaders": []}),
                "Neo does not support the \"extraHTTPHeaders\" option of a new context yet",
            ),
            (
                json!({"strictSelectors": true}),
                "Neo does not support the \"strictSelectors\" option of a new context yet",
            ),
        ] {
            assert_eq!(
                context_emulation(&params(options.clone())),
                Err(PlaywrightError::error(message)),
                "{options}"
            );
        }
    }
}
