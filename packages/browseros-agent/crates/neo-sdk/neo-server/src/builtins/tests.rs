//! The built-ins through the real door, on the fake browser, with a real trace store read back
//! from a temp dir.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_cdp::CdpError;
use browseros_core::{NativeCapabilities, TabId, TargetId};
use futures_util::future::{self, Either};
use neo_captcha::CaptchaSolver;
use neo_captcha::config::{CAPSOLVER_KEY_FILE_ENV, CAPSOLVER_URL_ENV, CaptchaConfig};
use neo_http::{Settings, SettingsDefaults};
use neo_playwright::{
    ErrorCode as PlaywrightCode, FileAccess, Op, OpEnv, PlaywrightError, Refusal, RunState,
};
use neo_takeover::{Lock, LockBinding, LockTtl, Locks, Reads, TabHold};
use neo_trace::{
    Delivery, Effect, EpochMillis, ErrorOwner, OperationId, OperationKind, OperationRow,
    OperationStarted, Outcome, RedactionLimits, Redactor, RunEnded, RunId, RunStarted, RunStatus,
    TaskId, TraceSink, TraceStore, WriterConfig, spawn_writer,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::Notify;
use tokio::task;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

use super::event_release::EventRelease;
use super::op_marks::{Hold, OpMarks};
use super::playwright_ops::{self, FILE_PAGES, OpCtxEnv, leaf_error};
use super::{BuiltinDeps, CaptchaClaims, PlaywrightFacade, TabJanitor};
use crate::browser::{Adopt, BrowserLink, NeoTab, PageRef, TabRegistry, TabsScope};
use crate::config::Defaults;
use crate::dispatch::{
    Call, CallSite, Dispatcher, ErrorCode, EventLog, FrameVersion, LeafError, Lifetime, Listing,
    Method, MethodSpec, Observer, OpCtx, OpFailure, OpReply, OperationError, PageChange, PageFact,
    PageScript, PageTarget, PendingReply, Policy, PolicyCheck, RegistryBuilder, RegistryError,
    RunCtx, RunEnd, RunGuard, RunStart, StopReason, Visibility,
};
use crate::host::{FilledTabs, HostHolds, TakeoverLock};
use crate::runtime::{Script, ScriptOutcome, run_script};
use crate::test_rig::{TestError, on_neo_runtime};
use crate::test_support::FakeCdp;

/// The built-ins' door on the fake browser, with the app's own rules in the app's order
/// (`door_rules`): the takeover lock over neo-takeover's real `Locks`, then the CAPTCHA claims.
/// A plain data bag.
struct Harness {
    fake: Arc<FakeCdp>,
    link: Arc<BrowserLink>,
    tabs: Arc<TabRegistry>,
    store: TraceStore,
    sink: TraceSink,
    /// What the host API's lock routes write; the door reads it through `takeover`, the janitor
    /// through `holds`.
    locks: Arc<Locks>,
    takeover: Arc<TakeoverLock>,
    /// The same claims the dispatcher checks, also available without a paid provider.
    claims: Arc<CaptchaClaims>,
    /// The host's claims as the janitor reads them: `locks`, and the fits a test puts in
    /// `fits` (a fit needs Neo's screen, which the fake browser has not).
    holds: Arc<HostHolds>,
    fits: Arc<Mutex<HashMap<TabId, TabHold>>>,
    /// The host's settings, at their defaults until a test patches them.
    settings: Settings,
    /// The facade every Playwright op of `dispatcher` runs on, as app.rs builds it.
    playwright: Arc<PlaywrightFacade>,
    dispatcher: Arc<Dispatcher>,
    /// Keeps the store's directory for the test's life.
    _dir: TempDir,
}

const DESKTOP_MODE: bool = false;
const HOSTED_MODE: bool = true;

impl Harness {
    /// A Neo with no CAPTCHA provider key.
    async fn new() -> Result<Self, Box<dyn Error>> {
        Self::with_solver(None).await
    }

    /// A Neo with the chosen CAPTCHA solver.
    async fn with_solver(solver: Option<Arc<CaptchaSolver>>) -> Result<Self, Box<dyn Error>> {
        Self::with_mode::<false>(solver, DESKTOP_MODE).await
    }

    /// A browser with explicit event capabilities: the fake can emit native console events.
    async fn with_mode<const SCRIPT_CDP: bool>(
        solver: Option<Arc<CaptchaSolver>>,
        hosted: bool,
    ) -> Result<Self, Box<dyn Error>> {
        let fake = FakeCdp::new();
        let tabs = Arc::new(TabRegistry::new());
        let connect = Arc::clone(&fake);
        let link = BrowserLink::start_with(
            NativeCapabilities::default(),
            Arc::clone(&tabs),
            move || future::ready(connect.connect()),
        );
        // Linked, with target discovery on, so a tab closed by the fake reaches the registry.
        within(async {
            while !link.is_ready() || fake.count("Target.setDiscoverTargets") == 0 {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?;
        // `page.opener` is the page read these tests use: neo-playwright's other reads run
        // in an isolated world the fake browser does not have, and this one is a single CDP call.
        fake.answer("Target.getTargetInfo", json!({ "targetInfo": {} }));
        let dir = tempfile::tempdir()?;
        let store = TraceStore::open(&dir.path().join("trace.db")).await?;
        let redactor = Redactor::new(RedactionLimits::default());
        let sink = spawn_writer(&store, redactor, WriterConfig::default(), None);
        // Armed the way the host API arms it once the service starts.
        let locks = Locks::new(store.clone(), sink.clone());
        let takeover = Arc::new(TakeoverLock::default());
        takeover.arm(Arc::clone(&locks));
        let fits: Arc<Mutex<HashMap<TabId, TabHold>>> = Arc::default();
        let holds = Arc::new(HostHolds::default());
        holds.arm({
            let locks = Arc::clone(&locks);
            let fits = Arc::clone(&fits);
            move |tab| {
                let fit = fits
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&tab)
                    .copied();
                TabHold::latest_of(locks.hold(tab), fit)
            }
        });
        let claims = Arc::new(CaptchaClaims::default());
        let settings = Settings::defaults(SettingsDefaults {
            read_dirs: vec!["/".into()],
        });
        let live = settings.clone();
        let file_access = Arc::new(FileAccess::new(move || live.read_dirs(), Vec::new()));
        let playwright =
            PlaywrightFacade::start(Arc::clone(&link), hosted, Arc::default(), file_access, None);
        let mut registry = RegistryBuilder::default();
        let deps = BuiltinDeps {
            script_cdp: SCRIPT_CDP,
            solver,
            claims: Arc::clone(&claims),
            playwright: Arc::clone(&playwright),
        };
        super::builtin_set(&mut registry, deps)?;
        let dispatcher = Arc::new(Dispatcher::new(
            registry.build(),
            super::door_rules(&takeover, &claims),
            sink.clone(),
            Arc::clone(&link),
            Arc::clone(&tabs),
            Observer::new(Arc::clone(&playwright) as _),
        ));
        Ok(Self {
            fake,
            link,
            tabs,
            store,
            sink,
            locks,
            takeover,
            claims,
            holds,
            fits,
            settings,
            playwright,
            dispatcher,
            _dir: dir,
        })
    }

    /// The host's fit on `tab`, as `Service::tab_hold` would report it: `Now` while active,
    /// `Until` once restored.
    fn fit(&self, tab: TabId, hold: TabHold) {
        self.fits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(tab, hold);
    }

    /// Ends `guard`'s run as the run service does once its script settled.
    fn finish(guard: RunGuard) {
        guard.finish(RunEnd {
            status: RunStatus::Ok,
            error: None,
            result: None,
            replay: None,
        });
    }

    /// The owner takes `tab`, as `POST /v1/locks` does. Returns once the tab's in-flight writes
    /// settled, at most 2 s.
    async fn lock(&self, tab: TabId, reads: Reads) -> Result<Lock, Box<dyn Error>> {
        let ttl = LockTtl::new(Duration::from_secs(60)).ok_or("a TTL within 1 s to 20 min")?;
        Ok(self.locks.lock(tab, reads, ttl, LockBinding::Host).await)
    }

    /// A recorded script run with no task and no default page yet.
    fn run(&self) -> Result<(Arc<RunCtx>, RunGuard), Box<dyn Error>> {
        let view = self.link.view().ok_or("the link is up")?;
        let start = RunStart {
            id: RunId::new(),
            code: "await page.title()".to_owned(),
            request_id: None,
            task_id: None,
            host_data: None,
            view,
            deadline: Instant::now() + Defaults::RUN_TIMEOUT,
            default_page: None,
            tabs_scope: TabsScope::Browser,
        };
        Ok(RunCtx::start(&self.dispatcher, start))
    }

    /// One script call, answered.
    async fn submit(&self, run: &Arc<RunCtx>, name: &str, input: Value) -> OpReply {
        self.submit_op(run, name, input).await.1
    }

    /// One call the script itself made, from a line of its code, answered: the kind that acts
    /// on a tab for the run's task. [`Harness::submit`]'s calls carry no site, as the prelude's
    /// own do.
    async fn submit_scripted(&self, run: &Arc<RunCtx>, name: &str, input: Value) -> OpReply {
        let pending = self.dispatcher.submit(
            run,
            Call::script(name.to_owned(), input, CallSite::parse("run:1:1")),
        );
        pending.wait().await
    }

    /// One script call, answered, with the op it became.
    async fn submit_op(
        &self,
        run: &Arc<RunCtx>,
        name: &str,
        input: Value,
    ) -> (OperationId, OpReply) {
        let pending = self
            .dispatcher
            .submit(run, Call::script(name.to_owned(), input, None));
        (pending.operation(), pending.wait().await)
    }

    /// The params of the first `method` call the fake browser received.
    fn first_params(&self, method: &str) -> Result<Value, Box<dyn Error>> {
        let call = self
            .fake
            .calls()
            .into_iter()
            .find(|call| call.method == method);
        Ok(call.ok_or_else(|| format!("{method} was called"))?.params)
    }

    /// A tab the owner opened, as core names it.
    async fn owner_tab(&self, url: &str) -> Result<PageRef, Box<dyn Error>> {
        let tab = self.fake.open_tab(url, "Owner's");
        self.listed(tab).await
    }

    /// The fake's `tab`, as core lists it.
    async fn listed(&self, tab: TabId) -> Result<PageRef, Box<dyn Error>> {
        let run = self.link.view().ok_or("the link is up")?;
        let listing = run.view.list_pages().await?;
        let info = listing
            .pages
            .iter()
            .find(|page| page.tab_id == tab)
            .ok_or("core lists the fake's tab")?;
        Ok(PageRef::from(info))
    }

    /// Whether the fake browser still has `tab` open.
    async fn is_open(&self, tab: TabId) -> Result<bool, Box<dyn Error>> {
        let run = self.link.view().ok_or("the link is up")?;
        let listing = run.view.list_pages().await?;
        Ok(listing.pages.iter().any(|page| page.tab_id == tab))
    }

    /// The op as the trace stored it, once everything sent so far is committed.
    async fn row(&self, operation: OperationId) -> Result<OperationRow, Box<dyn Error>> {
        self.sink.flush().await?;
        let row = self.store.operation(operation).await?;
        row.ok_or_else(|| format!("{operation} is recorded").into())
    }

    /// A door over this harness's browser and trace with only `rules` and the methods
    /// `methods` registers.
    fn dispatcher_with(
        &self,
        rules: Vec<Arc<dyn Policy>>,
        methods: impl FnOnce(&mut RegistryBuilder) -> Result<(), RegistryError>,
    ) -> Result<Arc<Dispatcher>, Box<dyn Error>> {
        let mut registry = RegistryBuilder::default();
        methods(&mut registry)?;
        Ok(Arc::new(Dispatcher::new(
            registry.build(),
            rules,
            self.sink.clone(),
            Arc::clone(&self.link),
            Arc::clone(&self.tabs),
            Observer::new(Arc::clone(&self.playwright) as _),
        )))
    }

    fn janitor(&self) -> TabJanitor {
        TabJanitor::new(
            Arc::clone(&self.dispatcher),
            Arc::clone(&self.link),
            Arc::clone(&self.tabs),
            Arc::clone(&self.holds),
            self.settings.clone(),
        )
    }
}

/// A solver over a placeholder key file that never reaches a provider: the fake browser's
/// pages hold no CAPTCHA, so every solve ends at detection, and the provider URL is a closed
/// local port in case one ever did not.
fn placeholder_solver() -> Result<Arc<CaptchaSolver>, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let key = dir.path().join("capsolver-key");
    fs::write(&key, "placeholder")?;
    let key = key.display().to_string();
    let config = CaptchaConfig::from_lookup(|name| match name {
        CAPSOLVER_KEY_FILE_ENV => Some(key.clone()),
        CAPSOLVER_URL_ENV => Some("http://127.0.0.1:9".to_owned()),
        _ => None,
    });
    Ok(Arc::new(
        CaptchaSolver::new(config).ok_or("a readable key file makes a solver")?,
    ))
}

/// `neo.solveCaptcha` on `page`, as a script calls it.
fn solve(page: &PageRef) -> Call {
    Call::script(
        "neo.solveCaptcha".to_owned(),
        json!({ "page": handle(page) }),
        None,
    )
}

/// The page brand a script sends for `page`.
fn handle(page: &PageRef) -> Value {
    json!({ "$neo": "page", "tab": page.tab.0 })
}

fn value(reply: OpReply) -> Result<Value, Box<dyn Error>> {
    reply
        .outcome
        .map_err(|failure| format!("expected a value, got {}", failure.error).into())
}

fn failure(reply: OpReply) -> Result<OpFailure, Box<dyn Error>> {
    match reply.outcome {
        Ok(value) => Err(format!("expected a failure, got {value}").into()),
        Err(failure) => Ok(failure),
    }
}

/// The page a `context.newPage` entry `{ page, url }` names.
fn page_of(entry: &Value) -> Result<PageRef, Box<dyn Error>> {
    let tab = entry["page"]["tab"]
        .as_i64()
        .ok_or("a page brand has a tab")?;
    Ok(PageRef { tab: TabId(tab) })
}

/// `work`, or an error after 10 s: a deadlock fails the test instead of hanging it.
async fn within<T>(work: impl Future<Output = T>) -> Result<T, Box<dyn Error>> {
    let limit = Duration::from_secs(10);
    time::timeout(limit, work)
        .await
        .map_err(|error| format!("not within {limit:?}: {error}").into())
}

#[tokio::test]
async fn a_script_opens_and_closes_a_page_with_the_doors_bookkeeping() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;

    let (opened_op, reply) = harness.submit_op(&run, "context.newPage", json!({})).await;
    let entry = value(reply)?;
    let page = page_of(&entry)?;
    assert_eq!(
        entry,
        json!({ "page": handle(&page), "url": "about:blank" }),
        "Playwright's page entry, with the brand the bootstrap revives"
    );
    assert_eq!(
        harness.first_params("Browser.createTab")?["background"],
        json!(true),
        "a new tab never takes the owner's focus in the live view"
    );
    assert!(
        harness.tabs.agent_tabs().contains(&page.tab),
        "agent-opened, so the janitor may close it once idle"
    );

    let on_page = json!({ "page": handle(&page) });
    let (read_op, reply) = harness
        .submit_op(&run, "page.opener", on_page.clone())
        .await;
    assert_eq!(
        value(reply)?,
        json!({ "opener": null }),
        "Neo opened it, no page did"
    );

    let (close_op, reply) = harness.submit_op(&run, "page.close", on_page).await;
    let fact = reply
        .pages
        .first()
        .cloned()
        .ok_or("close reports its page")?;
    assert!(fact.closed, "the reply says the page is gone");
    assert_eq!(value(reply)?, Value::Null);
    assert!(
        !harness.is_open(page.tab).await?,
        "the browser closed the tab"
    );

    for (operation, effect, delivery) in [
        (opened_op, Effect::Write, Delivery::Done),
        (read_op, Effect::Read, Delivery::NotSent),
        (close_op, Effect::Write, Delivery::Done),
    ] {
        let row = harness.row(operation).await?;
        assert_eq!(
            (row.effect, row.outcome, row.delivery),
            (effect, Some(Outcome::Ok), Some(delivery)),
            "{}: writes are done once sent, a read sends nothing",
            row.name
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_close_that_beforeunload_can_veto_leaves_the_page_open_until_the_browser_says()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let page = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    for name in ["page.close", "page.runBeforeUnload"] {
        let refused = failure(
            harness
                .submit(
                    &run,
                    name,
                    json!({ "page": handle(&page), "runBeforeUnload": true }),
                )
                .await,
        )?;
        assert_eq!(refused.error.code(), ErrorCode::INVALID_INPUT);
        assert_eq!(refused.delivery, Delivery::NotSent);
    }
    let close = json!({ "page": handle(&page) });
    let (close_op, reply) = harness.submit_op(&run, "page.runBeforeUnload", close).await;
    let fact = reply
        .pages
        .first()
        .cloned()
        .ok_or("close reports its page")?;
    assert_eq!(value(reply)?, Value::Null);
    assert_eq!(
        harness.fake.count("Page.close"),
        1,
        "the close went to the page, to run its beforeunload handlers"
    );
    // The fake answers `Page.close` and keeps the tab, as a page whose handler kept it does.
    assert!(harness.is_open(page.tab).await?, "the tab is still open");
    assert!(
        !fact.closed,
        "the close was only asked for, so the reply does not say the page is gone: {fact:?}"
    );
    assert_eq!(
        harness.row(close_op).await?.page.url.as_deref(),
        Some("about:blank"),
        "and neither does the trace"
    );
    let reply = harness
        .submit(&run, "page.opener", json!({ "page": handle(&page) }))
        .await;
    assert_eq!(
        value(reply)?,
        json!({ "opener": null }),
        "the page is still the script's to use"
    );
    Ok(())
}

#[tokio::test]
async fn a_load_the_browser_could_not_make_is_the_websites_failure() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let page = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    harness.fake.answer(
        "Page.navigate",
        json!({ "frameId": "frame", "errorText": "net::ERR_NAME_NOT_RESOLVED" }),
    );

    let input = json!({
        "page": handle(&page),
        "url": "https://nowhere.invalid/",
        "waitUntil": "load",
    });
    let failure = failure(within(harness.submit(&run, "page.goto", input)).await?)?;
    assert_eq!(
        (failure.error.owner(), failure.error.code()),
        (ErrorOwner::Website, ErrorCode::NAVIGATION_FAILED),
        "the page did not load: {}",
        failure.error
    );
    assert_eq!(
        failure.delivery,
        Delivery::Done,
        "sent, and the browser said how it ended"
    );
    assert!(!failure.retryable, "a sent write is not repeated blindly");
    assert!(!failure.error.is_timeout(), "an answer, not a timeout");

    let row = harness.row(failure.operation).await?;
    assert_eq!(row.delivery, Some(Delivery::Done));
    assert_eq!(
        row.error.map(|error| (error.owner, error.code)),
        Some((ErrorOwner::Website, "navigation_failed".to_owned()))
    );
    Ok(())
}

#[tokio::test]
async fn a_closed_page_names_who_closed_it() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let page = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    value(
        harness
            .submit(&run, "page.close", json!({ "page": handle(&page) }))
            .await,
    )?;

    let mine = failure(
        harness
            .submit(&run, "page.title", json!({ "page": handle(&page) }))
            .await,
    )?;
    assert_eq!(
        (mine.error.owner(), mine.error.code()),
        (ErrorOwner::Script, ErrorCode::PAGE_CLOSED),
        "this script closed it, so using it again is the script's mistake"
    );
    assert_eq!(mine.delivery, Delivery::NotSent);

    let owners = harness.owner_tab("https://example.com/").await?;
    assert!(
        harness.fake.close_tab(owners.tab),
        "the owner closes their tab"
    );
    let theirs = failure(
        harness
            .submit(&run, "page.title", json!({ "page": handle(&owners) }))
            .await,
    )?;
    assert_eq!(
        (theirs.error.owner(), theirs.error.code()),
        (ErrorOwner::Neo, ErrorCode::PAGE_CLOSED),
        "someone else closed it: {}",
        theirs.error
    );
    Ok(())
}

#[tokio::test]
async fn a_takeover_while_a_viewport_change_waits_prevents_its_first_send()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let page = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    let info = run
        .view()
        .list_pages()
        .await?
        .pages
        .into_iter()
        .find(|info| info.tab_id == page.tab)
        .ok_or("the opened tab is listed")?;
    let held = run.view().hold_viewport(info.page_id).await;
    let input = json!({ "page": handle(&page), "viewportSize": { "width": 640, "height": 480 } });
    let pending = harness.submit(&run, "page.setViewportSize", input);
    tokio::pin!(pending);
    tokio::select! {
        _ = &mut pending => return Err("the viewport change must queue behind the held capture".into()),
        () = time::sleep(Duration::from_millis(25)) => {}
    }
    let lock = harness.lock(page.tab, Reads::Allow).await?;
    let before = harness.fake.calls().len();
    drop(held);
    let refused = failure(pending.await)?;
    assert_eq!(
        refused.delivery,
        Delivery::NotSent,
        "the final recheck sees the takeover acquired during preparation"
    );
    assert!(
        harness
            .fake
            .calls()
            .iter()
            .skip(before)
            .all(|call| call.method != "Emulation.setDeviceMetricsOverride"),
        "no queued resize may overwrite an owner fit"
    );
    harness.locks.unlock_host(lock.id)?;
    Ok(())
}

#[tokio::test]
async fn a_viewport_write_is_refused_before_cdp_while_the_owner_locks_the_tab()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let page = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    let lock = harness.lock(page.tab, Reads::Allow).await?;
    let before = harness.fake.calls().len();
    let refused = failure(
        harness
            .submit(
                &run,
                "page.setViewportSize",
                json!({
                    "page": handle(&page), "viewportSize": { "width": 640, "height": 480 }
                }),
            )
            .await,
    )?;
    assert_eq!(
        refused.delivery,
        Delivery::NotSent,
        "a viewport change is an agent action on the locked tab"
    );
    assert!(
        harness
            .fake
            .calls()
            .iter()
            .skip(before)
            .all(|call| call.method != "Emulation.setDeviceMetricsOverride"),
        "the owner's fit must keep its metrics override"
    );
    harness.locks.unlock_host(lock.id)?;
    Ok(())
}

#[tokio::test]
async fn concurrent_calls_on_the_lazy_page_open_one_tab() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;

    let pending: Vec<PendingReply> = (0..5)
        .map(|_| {
            let call = Call::script(
                "page.opener".to_owned(),
                json!({ "page": { "$neo": "page", "tab": null } }),
                None,
            );
            harness.dispatcher.submit(&run, call)
        })
        .collect();
    let replies = within(future::join_all(
        pending.into_iter().map(PendingReply::wait),
    ))
    .await?;
    assert_eq!(
        harness.fake.count("Browser.createTab"),
        1,
        "context.newPage opened the default page once for all five"
    );
    let opened = replies
        .first()
        .and_then(|reply| reply.default_page.clone())
        .ok_or("the script learns its page")?;
    for reply in replies {
        assert_eq!(reply.default_page.as_ref(), Some(&opened));
        assert_eq!(
            value(reply)?,
            json!({ "opener": null }),
            "each read ran on the tab the door opened"
        );
    }
    assert!(
        harness.tabs.agent_tabs().contains(&opened.tab),
        "Neo opened it for the agent"
    );
    Ok(())
}

#[tokio::test]
async fn a_goto_that_opens_the_lazy_page_is_recorded_on_its_tab_and_holds_a_lock_unsettled()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    // A slow page: the navigation is still running when the owner takes the tab.
    harness.fake.hang("Page.navigate");
    let before = harness.fake.tabs();
    let goto = json!({
        "call": { "api": "page.goto", "timeoutMs": 30000 },
        "page": { "$neo": "page", "tab": null },
        "url": "https://bank.example/",
        "waitUntil": "load",
    });
    let pending = harness
        .dispatcher
        .submit(&run, Call::script("page.goto".to_owned(), goto, None));
    within(async {
        while harness.fake.count("Page.navigate") == 0 {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let tab = harness
        .fake
        .tabs()
        .into_iter()
        .find(|tab| !before.contains(tab))
        .ok_or("the door opened the run's page for the goto")?;
    assert_eq!(
        harness.row(pending.operation()).await?.tab,
        Some(tab),
        "the goto is on the record of the tab it navigates, from its start"
    );
    let lock = harness.lock(tab, Reads::Allow).await?;
    assert!(
        !lock.settled,
        "the agent's goto still navigates the tab, so the owner's lock is not settled"
    );
    Ok(())
}

#[tokio::test]
async fn the_owners_lock_keeps_the_agent_off_their_tab() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let owners = harness.owner_tab("https://example.com/").await?;
    let agents = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    let lock = harness.lock(owners.tab, Reads::Allow).await?;
    assert!(lock.settled, "nothing was in flight on the tab");

    let goto = json!({
        "page": handle(&owners),
        "url": "https://example.com/pay",
        "waitUntil": "load",
    });
    let (refused_op, reply) = harness.submit_op(&run, "page.goto", goto).await;
    let refused = failure(reply)?;
    assert_eq!(
        (refused.error.owner(), refused.error.code()),
        (ErrorOwner::Host, ErrorCode::OWNER_HAS_CONTROL),
        "the owner holds the tab: {}",
        refused.error
    );
    assert!(
        refused.error.message().contains(&lock.id.to_string()),
        "the refusal names the lock, so a host can match it: {}",
        refused.error
    );
    assert_eq!(
        refused.delivery,
        Delivery::NotSent,
        "refused before anything reached the page"
    );
    assert!(
        refused.retryable,
        "nothing was sent, so it is safe to repeat"
    );
    assert_eq!(
        harness.fake.count("Page.navigate"),
        0,
        "the browser never saw the navigation"
    );
    let row = harness.row(refused_op).await?;
    assert_eq!(
        (
            row.delivery,
            row.error.map(|error| (error.owner, error.code))
        ),
        (
            Some(Delivery::NotSent),
            Some((ErrorOwner::Host, "owner_has_control".to_owned()))
        ),
        "the trace says what the lock refused"
    );

    value(
        harness
            .submit(&run, "page.opener", json!({ "page": handle(&owners) }))
            .await,
    )
    .map_err(|error| format!("reads: allow lets a read through: {error}"))?;
    value(
        harness
            .submit(&run, "page.close", json!({ "page": handle(&agents) }))
            .await,
    )
    .map_err(|error| format!("a lock on one tab leaves the others alone: {error}"))?;

    harness.locks.unlock_host(lock.id)?;
    harness.lock(owners.tab, Reads::Block).await?;
    let read = failure(
        harness
            .submit(&run, "page.opener", json!({ "page": handle(&owners) }))
            .await,
    )?;
    assert_eq!(
        (read.error.code(), read.delivery),
        (ErrorCode::OWNER_HAS_CONTROL, Delivery::NotSent),
        "reads: block refuses reads too: the owner may be typing a password"
    );
    Ok(())
}

/// What the page shows once the owner took it, which a script that ran before the lock still
/// read from the page while the owner was acting on it.
const OWNERS_TEXT: &str = "the owner typed 4 Elm St";

/// `test.write`: a write on a page that marks itself sent, then answers only once the test says
/// so, so that the owner can take a tab while it runs. `page_script` says what it stands for:
/// a main-world script, as `OpMarks` marks `page.evaluate`, or an isolated write such as a click.
/// Either way it answers as its input's [`Answers`] says.
struct PausedWrite {
    page_script: PageScript,
    /// Notified once the op is sent and waits.
    running: Arc<Notify>,
    /// Lets the op answer.
    answer: Arc<Notify>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PausedWriteInput {
    page: PageTarget,
    answers: Answers,
}

/// What a [`PausedWrite`] answers once the test lets it.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Answers {
    /// What the page shows, [`OWNERS_TEXT`], as its value.
    Read,
    /// The same text, as the error the page threw.
    Throws,
    /// `{}`, as `page.setContent` answers once its HTML is written.
    Nothing,
}

impl Method for PausedWrite {
    type Input = PausedWriteInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            page_script: self.page_script,
            ..MethodSpec::new("test.write", Effect::Write)
        }
    }

    fn target(input: &PausedWriteInput) -> Option<&PageTarget> {
        Some(&input.page)
    }

    async fn call(&self, op: &OpCtx<Self>, input: PausedWriteInput) -> Result<Value, LeafError> {
        op.recheck()?;
        op.mark_sent();
        self.running.notify_one();
        self.answer.notified().await;
        match input.answers {
            Answers::Read => Ok(json!({ "read": OWNERS_TEXT })),
            Answers::Throws => Err(LeafError::definite(OperationError::website(
                ErrorCode::PAGE_ERROR,
                format_args!("page.evaluate: Error: {OWNERS_TEXT}"),
            ))),
            Answers::Nothing => Ok(json!({})),
        }
    }
}

/// Where a [`PausedWrite`] runs and what the owner takes while it does. A plain data bag.
#[derive(Clone, Copy)]
struct Takeover<'a> {
    /// The op's page.
    page: &'a PageRef,
    /// The tab the owner takes once the op is sent.
    taken: TabId,
    reads: Reads,
}

impl<'a> Takeover<'a> {
    /// The owner takes the op's own tab.
    fn of(page: &'a PageRef, reads: Reads) -> Self {
        Self {
            page,
            taken: page.tab,
            reads,
        }
    }
}

/// A [`PausedWrite`] standing for `page_script`, run through the app's rules in the app's order.
/// The owner takes a tab as `takeover` says once it is sent, and it answers once the lock
/// refuses. Returns its op and reply.
async fn answer_after_a_takeover(
    harness: &Harness,
    takeover: Takeover<'_>,
    page_script: PageScript,
    answers: Answers,
) -> Result<(OperationId, OpReply), Box<dyn Error>> {
    let running = Arc::new(Notify::new());
    let answer = Arc::new(Notify::new());
    let write = PausedWrite {
        page_script,
        running: Arc::clone(&running),
        answer: Arc::clone(&answer),
    };
    let claims = Arc::new(CaptchaClaims::default());
    let door = harness
        .dispatcher_with(super::door_rules(&harness.takeover, &claims), |registry| {
            registry.define(write)
        })?;
    let (run, _guard) = harness.run()?;
    let input = json!({
        "page": handle(takeover.page),
        "answers": answers,
    });
    let pending = door.submit(&run, Call::script("test.write".to_owned(), input, None));
    let operation = pending.operation();
    within(running.notified()).await?;
    // The lock refuses from the moment it is in the table; it settles once the write it found in
    // flight ends, so the op answers before the lock returns.
    let (lock, reply) = tokio::join!(harness.lock(takeover.taken, takeover.reads), async {
        while !harness.takeover.is_locked(takeover.taken) {
            time::sleep(Duration::from_millis(5)).await;
        }
        answer.notify_one();
        pending.wait().await
    });
    harness.locks.unlock_host(lock?.id)?;
    Ok((operation, reply))
}

/// Asserts that `reply`, the reply of `operation`, a [`PausedWrite`] that answered as `answers`
/// says, is withheld by the owner's lock: the script ran, so the op is `done`, and neither the
/// realm nor the record gets what the owner's tab showed.
async fn assert_withheld_by_the_lock(
    harness: &Harness,
    operation: OperationId,
    reply: OpReply,
    answers: Answers,
) -> Result<(), Box<dyn Error>> {
    let withheld = failure(reply).map_err(|error| format!("{answers:?}: {error}"))?;
    assert_eq!(
        (
            withheld.error.owner(),
            withheld.error.code(),
            withheld.delivery,
            withheld.retryable,
        ),
        (
            ErrorOwner::Host,
            ErrorCode::OWNER_HAS_CONTROL,
            Delivery::Done,
            false
        ),
        "{answers:?}: the script ran, and the owner's tab answered it: {}",
        withheld.error
    );
    assert!(
        !withheld.error.message().contains(OWNERS_TEXT),
        "{answers:?}: the realm gets none of what the script read: {}",
        withheld.error
    );
    let row = harness.row(operation).await?;
    assert_eq!(
        (
            row.delivery,
            row.error.as_ref().map(|error| error.code.as_str())
        ),
        (Some(Delivery::Done), Some("owner_has_control")),
        "{answers:?}: the record says the lock withheld it"
    );
    let recorded = serde_json::to_string(&(&row.result, &row.error))?;
    assert!(
        !recorded.contains(OWNERS_TEXT),
        "{answers:?}: nor does the record: {recorded}"
    );
    Ok(())
}

/// G2 (decision 35c). A main-world script is a write, but what it answers is what it read of the
/// page. A takeover that blocks reads, taken while the script ran, withholds its answer as it
/// withholds a read's: the script ran, so the op is `done`, and neither the realm nor the record
/// gets what the owner's tab showed. `page.setContent`'s `{}` reads the same (tradeoff T2): the
/// rule keys on the op's mark, never on what it answered, so a write that ran reads `done` with
/// the lock's error.
#[tokio::test]
async fn a_reads_blocking_takeover_taken_while_a_main_world_script_runs_withholds_its_result()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://bank.example/").await?;
    for answers in [Answers::Read, Answers::Throws, Answers::Nothing] {
        let (operation, reply) = answer_after_a_takeover(
            &harness,
            Takeover::of(&owners, Reads::Block),
            PageScript::MainWorld,
            answers,
        )
        .await?;
        assert_withheld_by_the_lock(&harness, operation, reply, answers).await?;
    }
    Ok(())
}

/// A lock that lets reads through releases what a main-world script read before it was taken,
/// as it releases a read's value: only a lock that blocks reads withholds it.
#[tokio::test]
async fn a_takeover_that_allows_reads_still_releases_a_main_world_result()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://bank.example/").await?;
    let (_, reply) = answer_after_a_takeover(
        &harness,
        Takeover::of(&owners, Reads::Allow),
        PageScript::MainWorld,
        Answers::Read,
    )
    .await?;
    assert_eq!(value(reply)?, json!({ "read": OWNERS_TEXT }));
    Ok(())
}

/// An isolated write's end goes out as it ended, a reads-blocking lock or not: the door's release
/// check is for what a script read, and a click's own result is not that.
#[tokio::test]
async fn a_isolated_write_is_released_as_it_ended_under_a_reads_blocking_takeover()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://bank.example/").await?;
    let (_, reply) = answer_after_a_takeover(
        &harness,
        Takeover::of(&owners, Reads::Block),
        PageScript::Isolated,
        Answers::Read,
    )
    .await?;
    assert_eq!(value(reply)?, json!({ "read": OWNERS_TEXT }));
    let (_, thrown) = answer_after_a_takeover(
        &harness,
        Takeover::of(&owners, Reads::Block),
        PageScript::Isolated,
        Answers::Throws,
    )
    .await?;
    let failed = failure(thrown)?;
    assert_eq!(
        (failed.error.code(), failed.delivery),
        (ErrorCode::PAGE_ERROR, Delivery::Done),
        "and so does its error: {}",
        failed.error
    );
    Ok(())
}

/// What a run's event pulls brought back. A plain data bag.
#[derive(Debug, Default)]
struct Pulled {
    /// The events released, oldest first.
    events: Vec<Value>,
    /// How many events the door withheld.
    withheld: u64,
}

impl Pulled {
    /// Each released event as `kind: text`, with a console message's text or a dialog's message.
    fn said(&self) -> Vec<String> {
        self.events
            .iter()
            .map(|event| {
                let text = event["message"]["text"]
                    .as_str()
                    .or_else(|| event["dialog"]["message"].as_str())
                    .unwrap_or_default();
                format!("{}: {text}", event["event"].as_str().unwrap_or_default())
            })
            .collect()
    }
}

/// Pulls as the prelude's event pump does, until `count` events were released or withheld.
async fn pull(harness: &Harness, run: &Arc<RunCtx>, count: u64) -> Result<Pulled, Box<dyn Error>> {
    let mut pulled = Pulled::default();
    while u64::try_from(pulled.events.len())? + pulled.withheld < count {
        let next = json!({ "call": { "api": "events.next", "timeoutMs": 0 } });
        let batch = value(within(harness.submit(run, "events.next", next)).await?)?;
        let events = batch["events"]
            .as_array()
            .ok_or("a pull returns its events")?;
        pulled.events.extend(events.iter().cloned());
        pulled.withheld += batch["withheld"].as_u64().unwrap_or_default();
    }
    Ok(pulled)
}

/// `Runtime.consoleAPICalled` for `console.log(text)`.
fn console_log(text: &str) -> Value {
    json!({
        "type": "log",
        "args": [{ "type": "string", "value": text }],
        "executionContextId": 1,
        "timestamp": 1.0,
    })
}

#[tokio::test]
async fn a_listener_gets_no_events_from_a_tab_whose_lock_blocks_reads() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://example.com/").await?;
    let (run, _guard) = harness.run()?;
    for event in ["console", "dialog"] {
        let listen = json!({
            "call": { "api": "page.on", "timeoutMs": 0 },
            "page": handle(&owners),
            "event": event,
            "enabled": true,
            "since": 0,
        });
        value(harness.submit(&run, "events.listen", listen).await)?;
    }
    harness.fake.page_event(
        owners.tab,
        "Runtime.consoleAPICalled",
        console_log("before"),
    );
    assert_eq!(pull(&harness, &run, 1).await?.said(), ["console: before"]);

    let lock = harness.lock(owners.tab, Reads::Block).await?;
    harness.fake.page_event(
        owners.tab,
        "Runtime.consoleAPICalled",
        console_log("the owner typed hunter2"),
    );
    harness.fake.page_event(
        owners.tab,
        "Page.javascriptDialogOpening",
        json!({ "type": "confirm", "message": "Send hunter2 to the bank?" }),
    );
    let during = pull(&harness, &run, 2).await?;
    assert_eq!(
        during.said(),
        Vec::<String>::new(),
        "nothing from the tab reaches the script while the owner blocks reads, and the pump \
         still answers"
    );
    assert_eq!(during.withheld, 2, "the pull says what it withheld");

    harness.locks.unlock_host(lock.id)?;
    harness
        .fake
        .page_event(owners.tab, "Runtime.consoleAPICalled", console_log("after"));
    assert_eq!(
        pull(&harness, &run, 1).await?.said(),
        ["console: after"],
        "the listener hears the tab again once the owner lets go"
    );
    Ok(())
}

#[tokio::test]
async fn a_wait_whose_event_names_a_tab_the_owner_holds_is_refused() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    // `context.waitForEvent('page')`: it names no page, so nothing but its event says which tab
    // it reads.
    let wait = json!({
        "call": { "api": "browserContext.waitForEvent", "timeoutMs": 5000 },
        "page": null,
        "event": "page",
        "since": 0,
    });
    let (operation, pending) = {
        let pending = harness
            .dispatcher
            .submit(&run, Call::script("events.wait".to_owned(), wait, None));
        (pending.operation(), pending)
    };
    // neo-playwright's target watcher subscribes to the browser's events before its first
    // `Target.getTargets`, so from then on it sees the tab open.
    within(async {
        while harness.fake.count("Target.getTargets") == 0 {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    // The tab announces no URL, so its open is recorded about a second later: after the lock.
    let tab = harness.fake.open_tab("https://bank.example/", "Bank");
    harness.lock(tab, Reads::Block).await?;
    let refused = failure(within(pending.wait()).await?)?;
    assert_eq!(
        (
            refused.error.owner(),
            refused.error.code(),
            refused.delivery
        ),
        (
            ErrorOwner::Host,
            ErrorCode::OWNER_HAS_CONTROL,
            Delivery::NotSent
        ),
        "the new tab is the owner's, reads blocked: {}",
        refused.error
    );
    assert_eq!(
        harness.row(operation).await?.result,
        None,
        "the event is not on the record either"
    );
    Ok(())
}

#[tokio::test]
async fn the_janitor_closes_idle_agent_tabs_through_the_door() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, guard) = harness.run()?;
    let idle = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    let held = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    let owners = harness.owner_tab("https://example.com/").await?;
    // A script op touched the owner's tab too, so it is known to the registry, and still Found.
    value(
        harness
            .submit(&run, "page.opener", json!({ "page": handle(&owners) }))
            .await,
    )?;
    // The run that used them ended; until then they are in use.
    Harness::finish(guard);
    let lock = harness.lock(held.tab, Reads::Block).await?;
    let janitor = harness.janitor();
    let idle_time = harness.settings.tab_idle_close();

    assert_eq!(
        janitor.sweep(Instant::now()).await,
        None,
        "nothing has been idle for 30 minutes yet"
    );
    let later = Instant::now() + idle_time;
    let sweep = within(janitor.sweep(later)).await?.ok_or("the sweep ran")?;

    assert!(
        !harness.is_open(idle.tab).await?,
        "the idle agent tab closed"
    );
    assert!(
        harness.is_open(held.tab).await?,
        "the owner holds this one, so the janitor left it open"
    );
    assert!(
        harness.is_open(owners.tab).await?,
        "never the owner's own tab"
    );

    harness.sink.flush().await?;
    let detail = harness
        .store
        .run(sweep)
        .await?
        .ok_or("the sweep is recorded")?;
    assert_eq!(
        (detail.summary.request_id, detail.summary.task_id),
        (None, None),
        "Neo's own run, for no host request or task"
    );
    assert_eq!(detail.summary.status, RunStatus::Ok);
    let closes: Vec<(&str, Option<TabId>, Option<Delivery>)> = detail
        .operations
        .iter()
        .map(|row| (row.name.as_str(), row.tab, row.delivery))
        .collect();
    assert_eq!(
        closes,
        vec![("page.close", Some(idle.tab), Some(Delivery::Done))],
        "the trace says what Neo closed; the held tab was skipped, not refused every minute"
    );

    assert_eq!(
        within(janitor.sweep(later + idle_time)).await?,
        None,
        "with only a held tab idle there is nothing to close, and no run is recorded"
    );
    harness.locks.unlock_host(lock.id)?;
    // The pass at `later + idle_time` saw the lock: the tab's idle clock runs from there.
    assert_eq!(
        within(janitor.sweep(later + idle_time + Duration::from_secs(1))).await?,
        None,
        "not right after the owner lets go"
    );
    let again = later + idle_time + idle_time;
    assert!(
        within(janitor.sweep(again)).await?.is_some(),
        "the held tab stayed a candidate"
    );
    assert!(
        !harness.is_open(held.tab).await?,
        "a full idle time after the owner lets go, a sweep closes it"
    );
    Ok(())
}

/// A host's five-minute idle setting
/// applies at the next pass, and a tab in use stays open however long the use lasts. A tab is
/// in use while a run that used it is in flight, a lock holds it, or a fit is on it, and its
/// clock restarts when the last of them ends. A popup an agent's tab opened is the agent's.
#[tokio::test]
async fn the_janitor_follows_the_idle_time_and_leaves_tabs_in_use() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let new_page = |run: Arc<RunCtx>| {
        let harness = &harness;
        async move {
            page_of(&value(
                harness.submit(&run, "context.newPage", json!({})).await,
            )?)
        }
    };
    let (busy_run, busy_guard) = harness.run()?;
    let busy = new_page(Arc::clone(&busy_run)).await?;
    let (run, guard) = harness.run()?;
    let free = new_page(Arc::clone(&run)).await?;
    let locked = new_page(Arc::clone(&run)).await?;
    let fitted = new_page(Arc::clone(&run)).await?;
    let opener = new_page(Arc::clone(&run)).await?;
    Harness::finish(guard);
    // The owner's tab keeps the browser open: the janitor never closes its last tab.
    let owners = harness.owner_tab("https://example.com/").await?;
    let popup = harness
        .fake
        .open_popup(opener.tab, "https://example.com/popup");
    within(async {
        while !harness.tabs.agent_tabs().contains(&popup) {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let lock = harness.lock(locked.tab, Reads::Block).await?;
    harness.fit(fitted.tab, TabHold::Now);
    let janitor = harness.janitor();
    let five_minutes = Duration::from_mins(5);

    assert_eq!(
        within(janitor.sweep(Instant::now() + five_minutes)).await?,
        None,
        "at the default 30 minutes nothing is idle after 5"
    );
    let unknown = harness
        .settings
        .patch(br#"{"tabs":{"idleCloseSeconds":300}}"#)
        .await?;
    assert!(unknown.is_empty(), "{unknown:?}");
    let first = Instant::now() + five_minutes;
    within(janitor.sweep(first)).await?.ok_or("the sweep ran")?;
    assert!(
        !harness.is_open(free.tab).await?,
        "the new idle time applies at the next pass, with no restart"
    );
    assert!(
        !harness.is_open(opener.tab).await? && !harness.is_open(popup).await?,
        "a popup an agent's tab opened goes with the agent's tabs"
    );
    for (page, why) in [
        (
            &busy,
            "a run in flight used it, so it stays past the idle time",
        ),
        (&locked, "a lock holds it"),
        (
            &fitted,
            "a fit is on it: the owner may be using it, with no lock",
        ),
    ] {
        assert!(harness.is_open(page.tab).await?, "{why}");
    }

    harness.locks.unlock_host(lock.id)?;
    harness.fit(fitted.tab, TabHold::Until(Instant::now()));
    Harness::finish(busy_guard);
    let ended = Instant::now();
    // The run's end, and the pass at `first` that saw the lock and the fit, restarted the
    // clocks: none of the three is idle a moment later.
    assert_eq!(
        within(janitor.sweep(ended + Duration::from_secs(1))).await?,
        None,
        "not right after a run ends, a lock is released or a fit restored"
    );
    within(janitor.sweep(first.max(ended) + five_minutes))
        .await?
        .ok_or("the sweep ran")?;
    for page in [&busy, &locked, &fitted] {
        assert!(
            !harness.is_open(page.tab).await?,
            "a full idle time after its last use ended, the tab is closed"
        );
    }
    assert!(harness.is_open(owners.tab).await?, "never the owner's tab");
    Ok(())
}

/// Closing the browser's last tab ends the browser, and Neo's server with it; both come back a
/// few seconds later with a new browser id. A host that keeps
/// one tab of its own would see that every idle time. "Last" counts every tab the browser
/// lists, the owner's included.
#[tokio::test]
async fn the_janitor_never_closes_the_browsers_last_tab() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let janitor = harness.janitor();
    let (run, guard) = harness.run()?;
    let keeper = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    Harness::finish(guard);
    let later = Instant::now() + harness.settings.tab_idle_close();

    assert_eq!(
        within(janitor.sweep(later)).await?,
        None,
        "nothing it may close, so no run in the trace every minute"
    );
    assert!(
        harness.is_open(keeper.tab).await?,
        "one idle agent tab alone is still there after the idle time"
    );

    let owners = harness.owner_tab("https://example.com/").await?;
    within(janitor.sweep(later)).await?.ok_or("the sweep ran")?;
    assert!(
        !harness.is_open(keeper.tab).await?,
        "with a second tab open, the idle agent tab is closed"
    );
    assert!(
        harness.is_open(owners.tab).await?,
        "and the owner's tab stays"
    );
    Ok(())
}

/// When the idle agent tabs are all the browser has, every one closes but the one used last.
/// Chromium answers a close before the tab leaves its listing, so listings right after it still
/// show the tab it just closed; the close waits them out (it answers closed only once the
/// browser's list drops the tab), and that tab must not count as the one keeping the browser
/// open, or the sweep would close the last tab too.
#[tokio::test]
async fn the_janitor_keeps_the_tab_used_last_while_a_closed_one_is_still_listed()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    harness.fake.linger_closed_tabs_for(2);
    let janitor = harness.janitor();
    let (run, guard) = harness.run()?;
    let older = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    let newer = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    Harness::finish(guard);
    // `newer` is used once more, so it is the one used last.
    time::sleep(Duration::from_millis(5)).await;
    let (run, guard) = harness.run()?;
    value(
        harness
            .submit(&run, "page.opener", json!({ "page": handle(&newer) }))
            .await,
    )?;
    Harness::finish(guard);

    let later = Instant::now() + harness.settings.tab_idle_close();
    let sweep = within(janitor.sweep(later)).await?.ok_or("the sweep ran")?;
    harness.sink.flush().await?;
    let detail = harness
        .store
        .run(sweep)
        .await?
        .ok_or("the sweep is recorded")?;
    assert_eq!(
        detail.result,
        Some(json!({ "closed": [older.tab.0], "kept": [newer.tab.0] })),
        "the older tab closed; the one used last stayed, though the browser still listed the closed one"
    );
    let closes: Vec<&str> = detail
        .operations
        .iter()
        .map(|row| row.name.as_str())
        .collect();
    assert_eq!(
        closes,
        vec!["page.close"],
        "one close, never the last tab's"
    );
    Ok(())
}

/// A tab whose navigation never commits answers no page command, and core attaches a tab afresh
/// on each browser connection. The janitor's close needs the browser alone, so a new connection
/// and a page that answers nothing still leave the idle tab closable within one sweep.
#[tokio::test]
async fn the_janitor_closes_an_idle_tab_that_answers_no_page_command() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let stuck = {
        let (run, _guard) = harness.run()?;
        page_of(&value(
            harness.submit(&run, "context.newPage", json!({})).await,
        )?)?
    };
    // Not the browser's last tab, which the janitor never closes.
    let owners = harness.owner_tab("https://example.com/").await?;
    harness.fake.drop_connection();
    harness.fake.reconnect();
    within(async {
        while !harness.link.is_ready() || harness.fake.count("Target.setDiscoverTargets") < 2 {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    for method in [
        "Page.enable",
        "DOM.enable",
        "Accessibility.enable",
        "Runtime.evaluate",
    ] {
        harness.fake.hang(method);
    }
    let attached = harness.fake.count("Target.attachToTarget");

    let later = Instant::now() + harness.settings.tab_idle_close();
    let sweep = within(harness.janitor().sweep(later)).await?;

    assert!(sweep.is_some(), "the sweep ran");
    assert!(
        !harness.is_open(stuck.tab).await?,
        "the idle agent tab closed"
    );
    assert!(harness.is_open(owners.tab).await?, "never the owner's tab");
    assert_eq!(
        harness.fake.count("Target.attachToTarget"),
        attached,
        "closing attached nothing"
    );
    Ok(())
}

/// A close is answered closed only once the browser's own tab list drops the tab. A browser that
/// acknowledges the close and keeps the tab (as `Browser.closeTab` does for a page that asks to
/// stay) fails the janitor's close as a timeout, and the tab stays a candidate: the next sweep
/// closes it once the browser does.
#[tokio::test]
async fn the_janitor_keeps_a_tab_the_browser_keeps_and_closes_it_later()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let kept = {
        let (run, _guard) = harness.run()?;
        page_of(&value(
            harness.submit(&run, "context.newPage", json!({})).await,
        )?)?
    };
    // Not the browser's last tab, which the janitor never closes.
    let owners = harness.owner_tab("https://example.com/").await?;
    harness
        .fake
        .answer("Target.closeTarget", json!({ "success": true }));
    let janitor = harness.janitor();
    let idle = harness.settings.tab_idle_close();
    // The close waits its 10 s for the listing, inside the sweep's 30 s budget.
    let first = time::timeout(
        Duration::from_secs(25),
        janitor.sweep(Instant::now() + idle),
    )
    .await?
    .ok_or("the sweep ran")?;
    harness.sink.flush().await?;
    let detail = harness
        .store
        .run(first)
        .await?
        .ok_or("the sweep is recorded")?;
    let close = detail
        .operations
        .iter()
        .find(|row| row.name == "page.close")
        .ok_or("the sweep tried the close")?;
    assert_eq!(
        close.error.as_ref().map(|error| error.code.as_str()),
        Some("wait_timeout"),
        "a tab the browser keeps is not answered closed: {close:?}"
    );
    assert!(harness.is_open(kept.tab).await?, "the browser kept it");
    assert!(
        harness
            .tabs
            .idle_agent_tabs(Duration::ZERO, Instant::now() + idle, &HashMap::new())
            .contains(&kept),
        "nothing reported it closed, so it stays the janitor's"
    );

    harness.fake.unanswer("Target.closeTarget");
    within(janitor.sweep(Instant::now() + idle))
        .await?
        .ok_or("the second sweep ran")?;
    assert!(
        !harness.is_open(kept.tab).await?,
        "the next sweep closed it"
    );
    assert!(harness.is_open(owners.tab).await?, "never the owner's tab");
    Ok(())
}

/// A tab whose target was replaced right before its close: whether or not core heard of the swap
/// first (a listing can land between them), the close ends on the tab's current target and the
/// tab is gone. The stale-target path itself is `page_op.rs`'s unit test.
#[tokio::test]
async fn a_close_closes_the_tabs_current_target_after_a_swap() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let page = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    harness.fake.hold_events();
    assert!(harness.fake.replace_target(page.tab));
    value(within(harness.submit(&run, "page.close", json!({ "page": handle(&page) }))).await?)?;
    harness.fake.release_events();
    let closes: Vec<Value> = harness
        .fake
        .calls()
        .into_iter()
        .filter(|call| call.method == "Target.closeTarget")
        .map(|call| call.params["targetId"].clone())
        .collect();
    assert_eq!(
        closes.last(),
        Some(&json!(format!("target-{}.1", page.tab.0))),
        "the last close named the tab's current target: {closes:?}"
    );
    assert!(harness.fake.tabs().iter().all(|tab| *tab != page.tab));
    assert!(!harness.is_open(page.tab).await?, "the tab is gone");
    Ok(())
}

#[tokio::test]
async fn script_cdp_off_does_not_resolve_even_a_lazy_page() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owner = harness.owner_tab("https://owner.example/").await?;
    let _lock = harness.lock(owner.tab, Reads::Block).await?;
    assert!(
        !harness
            .dispatcher
            .registry()
            .listed()
            .iter()
            .any(|spec| spec.name == "neo.cdp"),
        "the host did not enable raw CDP, so the description must not offer it"
    );
    let (run, _guard) = harness.run()?;
    let creates = harness.fake.count("Browser.createTab");
    let _claim = harness
        .claims
        .try_claim(owner.tab, OperationId::new(RunId::new(), 1))
        .map_err(|holder| format!("tab already claimed by {holder}"))?;
    let (op, reply) = harness
        .submit_op(
            &run,
            "neo.cdp",
            json!({
                "method": "Browser.getWindows",
                "page": { "$neo": "page", "tab": null },
            }),
        )
        .await;
    let refused = failure(reply)?;
    assert_eq!(
        (
            refused.error.owner(),
            refused.error.code(),
            refused.delivery
        ),
        (
            ErrorOwner::Host,
            ErrorCode::CDP_UNAVAILABLE,
            Delivery::NotSent
        )
    );
    assert_eq!(
        harness.fake.count("Browser.getWindows"),
        0,
        "off sends no raw command"
    );
    assert_eq!(
        harness.fake.count("Browser.createTab"),
        creates,
        "off must not open the lazy page"
    );
    let row = harness.row(op).await?;
    assert_eq!(
        (row.tab, row.sent_at),
        (None, None),
        "the disabled method never targets or sends"
    );
    Ok(())
}

#[tokio::test]
async fn script_cdp_refuses_link_commands_before_resolving_a_lazy_page()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::with_mode::<true>(None, DESKTOP_MODE).await?;
    let (run, _guard) = harness.run()?;
    let creates = harness.fake.count("Browser.createTab");
    for method in [
        "Browser.close",
        "Browser.crash",
        "Browser.crashGpuProcess",
        "Page.crash",
        "Browser.setDownloadBehavior",
        "Page.setDownloadBehavior",
        "Target.setAutoAttach",
        "Target.attachToTarget",
        "Target.detachFromTarget",
        "Target.setDiscoverTargets",
        "Target.exposeDevToolsProtocol",
        "Target.autoAttachRelated",
        "Target.attachToBrowserTarget",
        "Target.sendMessageToTarget",
    ] {
        let before = harness.fake.count(method);
        let (op, reply) = harness
            .submit_op(
                &run,
                "neo.cdp",
                json!({
                    "method": method,
                    "page": { "$neo": "page", "tab": null },
                }),
            )
            .await;
        let refused = failure(reply)?;
        assert_eq!(
            (
                refused.error.owner(),
                refused.error.code(),
                refused.delivery
            ),
            (
                ErrorOwner::Script,
                ErrorCode::UNSUPPORTED,
                Delivery::NotSent
            ),
            "{method}"
        );
        assert_eq!(
            harness.fake.count(method),
            before,
            "{method} never reaches CDP"
        );
        assert_eq!(
            harness.row(op).await?.sent_at,
            None,
            "refused before the send boundary"
        );
    }
    assert_eq!(
        harness.fake.count("Browser.createTab"),
        creates,
        "a refused call must not open the lazy page"
    );
    Ok(())
}

#[tokio::test]
async fn script_cdp_cannot_close_neos_reserved_tab_or_its_window() -> Result<(), Box<dyn Error>> {
    let harness = Harness::with_mode::<true>(None, DESKTOP_MODE).await?;
    let blank = blank_tab(&harness);
    let tab = within(blank.ensure(Adopt::Allowed))
        .await?
        .ok_or("Neo's tab opened")?;
    let (run, _guard) = harness.run()?;
    let listing = run.view().list_pages().await?;
    let neo = listing
        .pages
        .iter()
        .find(|page| page.tab_id == tab)
        .ok_or("core lists Neo's tab")?;
    let window = neo.window_id.ok_or("Neo's tab belongs to a window")?;
    let native_tab = run
        .view()
        .cdp("Browser.getTabInfo", json!({ "tabId": tab.0 }), None)
        .await?;
    // Chromium's page and tab target IDs both resolve to one native tab. The fake's normal
    // lookup accepts tab IDs only, so supply that same tab record for both target aliases.
    harness.fake.answer("Browser.getTabInfo", native_tab);
    let tab_target = format!("tab-target-{}", tab.0);
    for (method, params) in [
        (
            "Target.closeTarget",
            json!({ "targetId": neo.target_id.as_str() }),
        ),
        ("Target.closeTarget", json!({ "targetId": tab_target })),
        ("Browser.closeTab", json!({ "tabId": tab.0 })),
        (
            "Browser.closeTab",
            json!({ "targetId": neo.target_id.as_str() }),
        ),
        ("Browser.closeTab", json!({ "targetId": tab_target })),
        ("Browser.closeWindow", json!({ "windowId": window.0 })),
    ] {
        let before = harness.fake.count(method);
        let (op, reply) = harness
            .submit_op(
                &run,
                "neo.cdp",
                json!({ "method": method, "params": params }),
            )
            .await;
        let refused = failure(reply)?;
        assert_eq!(
            (
                refused.error.owner(),
                refused.error.code(),
                refused.delivery
            ),
            (
                ErrorOwner::Script,
                ErrorCode::UNSUPPORTED,
                Delivery::NotSent
            ),
            "Neo's reserved blank tab must be protected before {method} is sent"
        );
        assert!(
            refused.error.message().contains("blank tab"),
            "the refusal explains Neo's reserved tab"
        );
        assert_eq!(
            harness.fake.count(method),
            before,
            "{method} never reaches the browser"
        );
        assert_eq!(harness.row(op).await?.sent_at, None);
    }
    assert!(harness.is_open(tab).await?, "Neo's keeper tab remains open");
    Ok(())
}

#[tokio::test]
async fn script_cdp_calls_on_any_page_are_refused_under_a_lock_then_work_after_unlock()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::with_mode::<true>(None, DESKTOP_MODE).await?;
    let locked = harness.owner_tab("https://owner.example/").await?;
    let mine = harness.owner_tab("https://agent.example/").await?;
    let (run, _guard) = harness.run()?;
    let inputs = [
        json!({ "method": "Browser.getWindows" }),
        json!({ "method": "Browser.getWindows", "page": handle(&mine) }),
    ];
    for reads in [Reads::Allow, Reads::Block] {
        let lock = harness.lock(locked.tab, reads).await?;
        let before = harness.fake.count("Browser.getWindows");
        for input in &inputs {
            let refused = failure(harness.submit(&run, "neo.cdp", input.clone()).await)?;
            assert_eq!(
                (
                    refused.error.owner(),
                    refused.error.code(),
                    refused.delivery
                ),
                (
                    ErrorOwner::Host,
                    ErrorCode::OWNER_HAS_CONTROL,
                    Delivery::NotSent
                ),
                "every enabled raw call must be refused even through another unlocked page"
            );
        }
        assert_eq!(
            harness.fake.count("Browser.getWindows"),
            before,
            "a page session does not narrow a raw command to that page"
        );
        harness.locks.unlock_host(lock.id)?;
        for input in &inputs {
            value(harness.submit(&run, "neo.cdp", input.clone()).await)?;
        }
        assert_eq!(
            harness.fake.count("Browser.getWindows"),
            before + inputs.len(),
            "unlock restores both browser-level and page-session raw calls"
        );
    }
    Ok(())
}

#[tokio::test]
async fn script_cdp_respects_a_captcha_claim_on_any_page_until_released()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::with_mode::<true>(None, DESKTOP_MODE).await?;
    let solving = harness.owner_tab("https://captcha.example/").await?;
    let mine = harness.owner_tab("https://agent.example/").await?;
    let (run, _guard) = harness.run()?;
    let claim = harness
        .claims
        .try_claim(solving.tab, OperationId::new(RunId::new(), 1))
        .map_err(|holder| format!("tab already claimed by {holder}"))?;
    let inputs = [
        json!({ "method": "Browser.getWindows" }),
        json!({ "method": "Browser.getWindows", "page": handle(&mine) }),
    ];
    let before = harness.fake.count("Browser.getWindows");
    for input in &inputs {
        let refused = failure(harness.submit(&run, "neo.cdp", input.clone()).await)?;
        assert_eq!(
            (
                refused.error.owner(),
                refused.error.code(),
                refused.delivery
            ),
            (ErrorOwner::Script, ErrorCode::PAGE_BUSY, Delivery::NotSent),
            "raw CDP may reach the solving tab even through an unrelated page"
        );
    }
    assert_eq!(
        harness.fake.count("Browser.getWindows"),
        before,
        "no raw command is sent while another operation holds a claim"
    );
    drop(claim);
    for input in &inputs {
        value(harness.submit(&run, "neo.cdp", input.clone()).await)?;
    }
    assert_eq!(
        harness.fake.count("Browser.getWindows"),
        before + inputs.len(),
        "claim release restores browser-level and page-session raw calls"
    );
    Ok(())
}

#[tokio::test]
async fn script_cdp_releases_a_sent_calls_answer_when_a_captcha_claim_begins()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::with_mode::<true>(None, DESKTOP_MODE).await?;
    let solving = harness.owner_tab("https://captcha.example/").await?;
    let mine = harness.owner_tab("https://agent.example/").await?;
    harness.fake.serve("https://after.example/", "After");
    let (run, _guard) = harness.run()?;
    for (method, input) in [
        (
            "Browser.getWindows",
            json!({ "method": "Browser.getWindows" }),
        ),
        (
            "Page.navigate",
            json!({
                "method": "Page.navigate", "params": { "url": "https://after.example/" },
                "page": handle(&mine),
            }),
        ),
    ] {
        harness.fake.hold_next(method);
        let before = harness.fake.count(method);
        let pending = harness
            .dispatcher
            .submit(&run, Call::script("neo.cdp".to_owned(), input, None));
        let operation = pending.operation();
        within(async {
            while harness.fake.count(method) == before {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?;
        // The browser already received the command; the claim cannot prevent that write.
        let claim = harness
            .claims
            .try_claim(solving.tab, OperationId::new(RunId::new(), 1))
            .map_err(|holder| format!("tab already claimed by {holder}"))?;
        harness.fake.release_held();
        let reply = within(pending.wait()).await?;
        let raw: String = serde_json::from_value(value(reply)?)?;
        assert_eq!(
            raw, "{}",
            "a later claim must not withhold the browser's answer"
        );
        let row = harness.row(operation).await?;
        assert_eq!(row.delivery, Some(Delivery::Done));
        assert!(
            row.sent_at.is_some(),
            "the command passed the send boundary"
        );
        assert_eq!(harness.fake.count(method), before + 1);
        drop(claim);
    }
    let listing = run.view().list_pages().await?;
    assert!(
        listing
            .pages
            .iter()
            .any(|page| { page.tab_id == mine.tab && page.url == "https://after.example/" }),
        "the page-scoped command acted before the claim began"
    );
    Ok(())
}

#[tokio::test]
async fn script_cdp_rechecks_a_captcha_claim_taken_after_admission_before_send()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::with_mode::<true>(None, DESKTOP_MODE).await?;
    let solving = harness.owner_tab("https://captcha.example/").await?;
    let victim = harness.owner_tab("https://victim.example/").await?;
    let (run, _guard) = harness.run()?;
    harness.fake.hold_next("Browser.getTabInfo");
    let lookups = harness.fake.count("Browser.getTabInfo");
    let closes = harness.fake.count("Browser.closeTab");
    let pending = harness.dispatcher.submit(
        &run,
        Call::script(
            "neo.cdp".to_owned(),
            json!({
                "method": "Browser.closeTab",
                "params": { "targetId": format!("target-{}", victim.tab.0) },
            }),
            None,
        ),
    );
    let operation = pending.operation();
    // Reserved-tab lookup waits after admission, before the raw command's send recheck.
    within(async {
        while harness.fake.count("Browser.getTabInfo") == lookups {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let _claim = harness
        .claims
        .try_claim(solving.tab, OperationId::new(RunId::new(), 1))
        .map_err(|holder| format!("tab already claimed by {holder}"))?;
    harness.fake.release_held();
    let refused = failure(within(pending.wait()).await?)?;
    assert_eq!(
        (
            refused.error.owner(),
            refused.error.code(),
            refused.delivery
        ),
        (ErrorOwner::Script, ErrorCode::PAGE_BUSY, Delivery::NotSent),
        "a claim taken during preflight must stop the command at the send boundary"
    );
    assert_eq!(harness.fake.count("Browser.closeTab"), closes);
    assert_eq!(harness.row(operation).await?.sent_at, None);
    Ok(())
}

#[tokio::test]
async fn script_cdp_protocol_errors_are_the_scripts_but_link_and_session_failures_are_neos()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::with_mode::<true>(None, DESKTOP_MODE).await?;
    for (error, owner, delivery) in [
        (
            CdpError::Protocol {
                code: -32000,
                message: "No tab with given id".to_owned(),
            },
            ErrorOwner::Script,
            Delivery::Done,
        ),
        (CdpError::ConnectionLost, ErrorOwner::Neo, Delivery::Unknown),
        (CdpError::SessionGone, ErrorOwner::Neo, Delivery::Unknown),
    ] {
        harness.fake.fail("Browser.getWindows", error);
        let (run, _guard) = harness.run()?;
        let refused = failure(
            harness
                .submit(&run, "neo.cdp", json!({ "method": "Browser.getWindows" }))
                .await,
        )?;
        assert_eq!(
            (refused.error.owner(), refused.delivery),
            (owner, delivery),
            "a protocol answer belongs to the script; an unconfirmed link or session needs Neo"
        );
        if owner == ErrorOwner::Script {
            assert_eq!(
                refused.error.code(),
                ErrorCode::CDP_ERROR,
                "protocol replies use the stable script CDP code"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn script_cdp_close_uses_the_scripts_command_when_the_lookup_cannot_identify_a_tab()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::with_mode::<true>(None, DESKTOP_MODE).await?;
    harness.fake.fail(
        "Browser.getTabInfo",
        CdpError::Protocol {
            code: -32000,
            message: "No web contents in the target".to_owned(),
        },
    );
    harness
        .fake
        .answer("Target.closeTarget", json!({ "success": true }));
    let (run, _guard) = harness.run()?;
    let (op, reply) = harness
        .submit_op(
            &run,
            "neo.cdp",
            json!({
                "method": "Target.closeTarget", "params": { "targetId": "shared-worker" },
            }),
        )
        .await;
    let transported: String = serde_json::from_value(value(reply)?)?;
    let answer: Value = serde_json::from_str(&transported)?;
    assert_eq!(
        answer,
        json!({ "success": true }),
        "a non-tab target can still be closed"
    );
    assert_eq!(
        harness.row(op).await?.delivery,
        Some(Delivery::Done),
        "the original close reached the browser and was confirmed"
    );

    harness.fake.unanswer("Target.closeTarget");
    let refused = failure(
        harness
            .submit(
                &run,
                "neo.cdp",
                json!({
                    "method": "Target.closeTarget", "params": { "targetId": "unknown-target" },
                }),
            )
            .await,
    )?;
    assert_eq!(
        (
            refused.error.owner(),
            refused.error.code(),
            refused.delivery
        ),
        (ErrorOwner::Script, ErrorCode::CDP_ERROR, Delivery::Done),
        "the browser answers the original close, not Neo's lookup"
    );
    assert!(
        refused
            .error
            .message()
            .contains("No target with given id found"),
        "return the original close error rather than the lookup error"
    );

    for error in [CdpError::ConnectionLost, CdpError::SessionGone] {
        harness.fake.fail("Browser.getTabInfo", error);
        let before = harness.fake.count("Target.closeTarget");
        let refused = failure(
            harness
                .submit(
                    &run,
                    "neo.cdp",
                    json!({
                        "method": "Target.closeTarget", "params": { "targetId": "shared-worker" },
                    }),
                )
                .await,
        )?;
        assert_eq!(
            (refused.error.owner(), refused.delivery),
            (ErrorOwner::Neo, Delivery::NotSent),
            "a broken lookup connection or session cannot safely send the close"
        );
        assert_eq!(
            harness.fake.count("Target.closeTarget"),
            before,
            "the broken lookup link must not send a close"
        );
    }
    Ok(())
}

#[tokio::test]
async fn script_cdp_round_trips_params_and_records_a_write() -> Result<(), Box<dyn Error>> {
    let harness = Harness::with_mode::<true>(None, DESKTOP_MODE).await?;
    assert!(
        harness
            .dispatcher
            .registry()
            .listed()
            .iter()
            .any(|spec| spec.name == "neo.cdp"),
        "an enabled host offers the call in the run description"
    );
    harness.fake.answer(
        "Browser.getWindows",
        json!({ "windows": [{ "windowId": 42 }] }),
    );
    let (run, _guard) = harness.run()?;
    let params = json!({ "example": [1, true, { "name": "same" }] });
    let (op, reply) = harness
        .submit_op(
            &run,
            "neo.cdp",
            json!({
                "method": "Browser.getWindows", "params": params,
            }),
        )
        .await;
    let transported: String = serde_json::from_value(value(reply)?)?;
    let answer: Value = serde_json::from_str(&transported)?;
    assert_eq!(answer, json!({ "windows": [{ "windowId": 42 }] }));
    assert_eq!(
        harness.first_params("Browser.getWindows")?,
        params,
        "raw params survive unchanged"
    );
    let row = harness.row(op).await?;
    assert_eq!(
        (row.name.as_str(), row.effect, row.delivery),
        ("neo.cdp", Effect::Write, Some(Delivery::Done)),
        "even a read command is accounted as a possible write"
    );
    Ok(())
}

#[tokio::test]
async fn script_cdp_native_capability_refusals_are_host_owned_and_not_sent()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::with_mode::<true>(None, DESKTOP_MODE).await?;
    let (run, _guard) = harness.run()?;
    let (op, reply) = harness
        .submit_op(&run, "neo.cdp", json!({ "method": "Browser.getProfiles" }))
        .await;
    let refused = failure(reply)?;
    assert_eq!(
        (
            refused.error.owner(),
            refused.error.code(),
            refused.delivery
        ),
        (ErrorOwner::Host, ErrorCode::UNSUPPORTED, Delivery::NotSent),
        "the deployment's unknown native version prevents the command from being sent"
    );
    assert_eq!(harness.fake.count("Browser.getProfiles"), 0);
    assert_eq!(harness.row(op).await?.sent_at, None);
    Ok(())
}

#[tokio::test]
async fn script_cdp_withholds_page_data_when_takeover_blocks_reads_before_reply()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::with_mode::<true>(None, DESKTOP_MODE).await?;
    let page = harness.owner_tab("https://bank.example/").await?;
    harness.fake.answer(
        "Runtime.evaluate",
        json!({ "result": { "value": OWNERS_TEXT } }),
    );
    harness.fake.hold_next("Runtime.evaluate");
    let (run, _guard) = harness.run()?;
    let before = harness.fake.count("Runtime.evaluate");
    let pending = harness.dispatcher.submit(
        &run,
        Call::script(
            "neo.cdp".to_owned(),
            json!({ "method": "Runtime.evaluate", "page": handle(&page) }),
            None,
        ),
    );
    let operation = pending.operation();
    within(async {
        while harness.fake.count("Runtime.evaluate") == before {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let (lock, reply) = tokio::join!(harness.lock(page.tab, Reads::Block), async {
        while !harness.takeover.is_locked(page.tab) {
            time::sleep(Duration::from_millis(5)).await;
        }
        harness.fake.release_held();
        pending.wait().await
    });
    harness.locks.unlock_host(lock?.id)?;
    assert_withheld_by_the_lock(&harness, operation, reply, Answers::Read).await?;
    Ok(())
}

#[test]
fn script_cdp_keeps_sdk_shaped_json_as_plain_data_in_quickjs() -> Result<(), TestError> {
    on_neo_runtime(async {
        let harness = Harness::with_mode::<true>(None, DESKTOP_MODE)
            .await
            .map_err(|error| error.to_string())?;
        let data = json!({
            "bytes": { "$neo": "bytes", "base64": "QQ==" },
            "page": { "$neo": "page", "tab": 999 },
            "nested": [{ "$neo": "bytes", "base64": "Qg==" }],
        });
        harness.fake.answer("Browser.getWindows", data.clone());
        let (run, _guard) = harness.run().map_err(|error| error.to_string())?;
        let script = Script {
            code: "const raw = await neo.cdp('Browser.getWindows');
                   return { raw, bytesIsView: ArrayBuffer.isView(raw.bytes) };"
                .to_owned(),
            page: None,
            pages: Vec::new(),
            observed: None,
        };
        match run_script(&run, &harness.dispatcher, script).await.outcome {
            ScriptOutcome::Returned(value) => assert_eq!(
                value,
                json!({
                    "raw": data, "bytesIsView": false,
                })
            ),
            other => return Err(format!("expected untouched CDP JSON, got {other:?}").into()),
        }
        Ok(())
    })
}

#[tokio::test]
async fn script_cdp_cancellation_after_send_reports_unknown() -> Result<(), Box<dyn Error>> {
    let harness = Harness::with_mode::<true>(None, DESKTOP_MODE).await?;
    harness.fake.hang("Browser.getWindows");
    let (run, _guard) = harness.run()?;
    let pending = harness.dispatcher.submit(
        &run,
        Call::script(
            "neo.cdp".to_owned(),
            json!({ "method": "Browser.getWindows" }),
            None,
        ),
    );
    within(async {
        while harness.fake.count("Browser.getWindows") == 0 {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    assert_eq!(
        harness.first_params("Browser.getWindows")?,
        json!({}),
        "omitted params are an empty object"
    );
    run.stop(StopReason::CallerLeft);
    let refused = failure(within(pending.wait()).await?)?;
    assert_eq!(
        refused.delivery,
        Delivery::Unknown,
        "a sent command with no answer may have acted"
    );
    Ok(())
}

#[tokio::test]
async fn script_cdp_uses_the_admitted_tabs_session_and_takeover_rules() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::with_mode::<true>(None, DESKTOP_MODE).await?;
    let (run, _guard) = harness.run()?;
    let page = harness.owner_tab("https://before.example/").await?;
    harness.fake.serve("https://after.example/", "After");
    let (op, reply) = harness
        .submit_op(
            &run,
            "neo.cdp",
            json!({
                "method": "Page.navigate", "params": { "url": "https://after.example/" },
                "page": handle(&page),
            }),
        )
        .await;
    value(reply)?;
    let listing = run.view().list_pages().await?;
    assert!(
        listing
            .pages
            .iter()
            .any(|listed| listed.tab_id == page.tab && listed.url == "https://after.example/"),
        "the fake navigates only when Page.navigate arrives on this tab's own attached session"
    );
    assert_eq!(
        harness.row(op).await?.tab,
        Some(page.tab),
        "the trace targets the admitted tab"
    );
    let _lock = harness.lock(page.tab, Reads::Allow).await?;
    let before = harness.fake.count("Page.navigate");
    let refused = failure(
        harness
            .submit(
                &run,
                "neo.cdp",
                json!({
                    "method": "Page.navigate", "params": { "url": "https://blocked.example/" },
                    "page": handle(&page),
                }),
            )
            .await,
    )?;
    assert_eq!(
        (refused.error.code(), refused.delivery),
        (ErrorCode::OWNER_HAS_CONTROL, Delivery::NotSent)
    );
    assert_eq!(
        harness.fake.count("Page.navigate"),
        before,
        "a takeover refuses raw page writes too"
    );
    Ok(())
}

#[tokio::test]
async fn solving_without_a_provider_key_is_the_deployments_to_fix() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    assert!(
        !harness
            .dispatcher
            .registry()
            .listed()
            .iter()
            .any(|spec| spec.name == "neo.solveCaptcha"),
        "left out of the run description, so no model is offered what cannot work"
    );
    let (run, _guard) = harness.run()?;
    let owners = harness.owner_tab("https://example.com/").await?;
    let (op, reply) = harness
        .submit_op(&run, "neo.solveCaptcha", json!({ "page": handle(&owners) }))
        .await;
    let refused = failure(reply)?;
    assert_eq!(
        (refused.error.owner(), refused.error.code()),
        (ErrorOwner::Host, ErrorCode::CAPTCHA_UNAVAILABLE),
        "the deployment adds a key, not the script: {}",
        refused.error
    );
    assert_eq!(refused.delivery, Delivery::NotSent);
    assert_eq!(harness.row(op).await?.delivery, Some(Delivery::NotSent));
    Ok(())
}

#[tokio::test]
async fn a_solve_answers_its_verdict_as_a_value() -> Result<(), Box<dyn Error>> {
    let harness = Harness::with_solver(Some(placeholder_solver()?)).await?;
    assert!(
        harness
            .dispatcher
            .registry()
            .listed()
            .iter()
            .any(|spec| spec.name == "neo.solveCaptcha"),
        "with a key the description offers it"
    );
    let (run, _guard) = harness.run()?;
    let owners = harness.owner_tab("https://example.com/").await?;
    let (op, reply) = harness
        .submit_op(&run, "neo.solveCaptcha", json!({ "page": handle(&owners) }))
        .await;
    let answer = value(reply)?;
    // The fake browser answers the frame-tree read detection starts with `{}`, which the solver
    // takes for a browser error: it cannot tell whether a CAPTCHA blocks the page.
    assert_eq!(
        (answer["verdict"].as_str(), answer["reason"].as_str()),
        (Some("unknown"), Some("browser_error")),
        "{answer}"
    );
    assert_eq!(
        answer["message"],
        "Neo could not tell whether the CAPTCHA passed (a browser call failed); look at the page \
         again before acting.",
        "claw-sdk's own line, not the crate's MCP wording"
    );
    assert_eq!(answer["paidTasks"], 0, "no provider was ever asked");
    let row = harness.row(op).await?;
    assert_eq!(
        (row.outcome, row.delivery),
        (Some(Outcome::Ok), Some(Delivery::Done)),
        "every verdict is a value, and the solver may have acted, so it was sent"
    );
    Ok(())
}

#[tokio::test]
async fn a_second_solve_or_a_write_on_a_tab_being_solved_is_refused_unsent()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::with_solver(Some(placeholder_solver()?)).await?;
    let (run, _guard) = harness.run()?;
    let owners = harness.owner_tab("https://example.com/").await?;
    // A solve that claimed the tab now waits at its check that the tab is alive, holding the
    // claim and with nothing sent, until the run stops.
    harness.fake.hang("Browser.getTabInfo");

    let first = harness.dispatcher.submit(&run, solve(&owners)).wait();
    let second = harness.dispatcher.submit(&run, solve(&owners)).wait();
    // As in `Promise.all`: either may claim the tab first, and the other is refused.
    let (refused, holder) = match within(future::select(Box::pin(first), Box::pin(second))).await? {
        Either::Left((refused, holder)) | Either::Right((refused, holder)) => (refused, holder),
    };
    let refused = failure(refused)?;
    assert_eq!(
        (refused.error.owner(), refused.error.code()),
        (ErrorOwner::Script, ErrorCode::PAGE_BUSY),
        "two solves on one tab is the script's overlap: {}",
        refused.error
    );
    assert_eq!(refused.delivery, Delivery::NotSent);

    let goto = json!({ "page": handle(&owners), "url": "https://example.com/next" });
    let write = failure(harness.submit(&run, "page.goto", goto).await)?;
    assert_eq!(
        (write.error.code(), write.delivery),
        (ErrorCode::PAGE_BUSY, Delivery::NotSent),
        "the solver drives the page outside the door, so the door keeps writes off it"
    );
    assert_eq!(harness.fake.count("Page.navigate"), 0);

    run.stop(StopReason::CallerLeft);
    let holder = failure(within(holder).await?)?;
    assert_eq!(
        (holder.error.code(), holder.delivery),
        (ErrorCode::CANCELLED, Delivery::NotSent),
        "stopped before it sent anything"
    );
    let (next, _next_guard) = harness.run()?;
    value(
        harness
            .submit(&next, "page.close", json!({ "page": handle(&owners) }))
            .await,
    )
    .map_err(|error| format!("the claim ended with its op: {error}"))?;
    Ok(())
}

#[tokio::test]
async fn a_solve_the_run_stops_after_it_was_sent_is_unknown() -> Result<(), Box<dyn Error>> {
    let harness = Harness::with_solver(Some(placeholder_solver()?)).await?;
    let (run, _guard) = harness.run()?;
    let owners = harness.owner_tab("https://example.com/").await?;
    // Detection reads the frame tree first, so a held read means the solver is past
    // `mark_sent` and working on the page.
    harness.fake.hang("Page.getFrameTree");
    let pending = harness.dispatcher.submit(&run, solve(&owners));
    let operation = pending.operation();
    within(async {
        while harness.fake.count("Page.getFrameTree") == 0 {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;

    run.stop(StopReason::CallerLeft);
    let stopped = failure(within(pending.wait()).await?)?;
    assert_eq!(
        (stopped.error.owner(), stopped.error.code()),
        (ErrorOwner::Host, ErrorCode::CANCELLED),
        "the caller left, so the stop is the host's: {}",
        stopped.error
    );
    assert_eq!(
        stopped.delivery,
        Delivery::Unknown,
        "the solver may have clicked before it stopped: look before solving again"
    );
    assert!(!stopped.retryable, "a sent write is never repeated blindly");
    assert_eq!(
        harness.row(operation).await?.delivery,
        Some(Delivery::Unknown)
    );
    Ok(())
}

/// A locator brand on `page`, as the prelude builds `getByRole('button')`.
fn locator(page: &Value) -> Value {
    json!({
        "$neo": "locator",
        "page": page,
        "frame": null,
        "chunks": [{ "parts": [{ "name": "internal:role", "body": "button", "source": "button" }] }],
        "display": "getByRole('button')",
    })
}

/// Passes each op at admission and refuses its recheck, as an owner who took the tab while the
/// op waited for its element does.
#[derive(Default)]
struct TakenWhileWaiting {
    admitted: Mutex<HashSet<OperationId>>,
}

impl Policy for TakenWhileWaiting {
    fn check(&self, check: &PolicyCheck<'_>) -> Result<(), OperationError> {
        let admission = self
            .admitted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(check.operation);
        if admission {
            Ok(())
        } else {
            Err(OperationError::host(
                ErrorCode::OWNER_HAS_CONTROL,
                "the owner took the tab while the op waited",
            ))
        }
    }
}

/// `test.env`: what the adapter's `OpEnv` gives neo-playwright, over this op's own context.
/// A refused recheck is kept in `seen` as the crate sees it, then answered as a value anyway, as
/// a crate that dropped it would, or, with `handBack`, returned the way the crate returns it once
/// its `send_effect` is refused.
#[derive(Default)]
struct EnvProbe {
    seen: Arc<Mutex<Vec<Refusal>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct EnvProbeInput {
    hand_back: bool,
}

impl Method for EnvProbe {
    type Input = EnvProbeInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.env", Effect::Write)
    }

    fn target(_input: &EnvProbeInput) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        op: &OpCtx<Self>,
        input: EnvProbeInput,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        let env = OpCtxEnv::new(
            op,
            false,
            Arc::default(),
            Arc::new(FileAccess::new(|| vec!["/".into()], Vec::new())),
        );
        let shares_the_runs_state = Arc::ptr_eq(env.run_state(), &op.run().slot::<RunState>());
        future::ready(match env.recheck() {
            Ok(()) => {
                env.mark_sent();
                Ok(json!({ "sharesTheRunsState": shares_the_runs_state }))
            }
            Err(refused) => {
                self.seen
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(refused.clone());
                if input.hand_back {
                    Err(leaf_error(&PlaywrightError::from(refused)))
                } else {
                    Ok(json!("dropped the refusal"))
                }
            }
        })
    }
}

/// `test.crateFailure`: fails through the adapter's mapping with one of neo-playwright's
/// codes, after marking itself sent when asked, as a crate op that failed after or before its
/// effect.
struct CrateFailure;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CrateFailureInput {
    code: String,
    sent: bool,
}

impl Method for CrateFailure {
    type Input = CrateFailureInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.crateFailure", Effect::Write)
    }

    fn target(_input: &CrateFailureInput) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        op: &OpCtx<Self>,
        input: CrateFailureInput,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        let failed = || {
            let code = PlaywrightCode::ALL
                .iter()
                .copied()
                .find(|code| code.as_str() == input.code)
                .ok_or_else(|| {
                    LeafError::definite(OperationError::script(
                        ErrorCode::INVALID_INPUT,
                        "not one of the crate's codes",
                    ))
                })?;
            if input.sent {
                op.recheck()?;
                op.mark_sent();
            }
            let message = format!("locator.click: {}", code.as_str());
            Err(leaf_error(&PlaywrightError::new(code, message)))
        };
        future::ready(failed())
    }
}

#[tokio::test]
async fn every_playwright_op_is_one_listed_script_method_with_the_crates_spec()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let mut builder = RegistryBuilder::default();
    playwright_ops::playwright_ops(&mut builder, &harness.playwright)?;
    let listed = builder.build().listed();
    let mut names: Vec<&str> = Op::all()
        .map(Op::spec)
        .filter(|spec| Hold::of(spec).is_none())
        .map(|spec| spec.name)
        .collect();
    names.sort_unstable();
    let registered: Vec<&str> = listed.iter().map(|spec| spec.name).collect();
    assert_eq!(
        registered, names,
        "one listed method per op, and no other; the held ops are not offered"
    );
    let mut lines = HashSet::new();
    for spec in &listed {
        let op = Op::from_name(spec.name).ok_or(spec.name)?.spec();
        assert_eq!(spec.effect, op.effect, "{}", spec.name);
        assert_eq!(spec.visibility, Visibility::Script, "{}", spec.name);
        assert_eq!(
            spec.lifetime == Lifetime::Background,
            matches!(
                spec.name,
                "events.listen" | "events.next" | "page.locatorHandlerTriggered"
            ),
            "{}: only the prelude's event work is cut quietly when the run ends",
            spec.name
        );
        let Listing::Grouped(line) = spec.listing else {
            return Err(format!("{} is listed with the Playwright API", spec.name).into());
        };
        lines.insert(line);
    }
    assert_eq!(lines.len(), 1, "the description lists the API once");
    Ok(())
}

/// A `page.goto` to `url` on `page`, as the prelude sends it.
fn goto_input(page: &PageRef, url: &str) -> Value {
    json!({
        "call": { "api": "page.goto", "timeoutMs": 30000 },
        "page": handle(page),
        "url": url,
        "waitUntil": "load",
    })
}

#[tokio::test]
async fn a_goto_never_sends_a_file_url_or_another_unsupported_scheme() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://bank.example/").await?;
    let (run, _guard) = harness.run()?;
    // A script cannot open file: pages, however the scheme is spelled (owner's decision,
    // 2026-11-12), and other unsupported schemes keep their existing refusal.
    let files = [
        "file:///tmp/local.html",
        " FILE:///tmp/local.html",
        "fi\tle:///tmp/local.html",
    ];
    let others = [
        "view-source:https://bank.example/",
        "chrome://version",
        "devtools://devtools/bundled/inspector.html",
        "chrome-extension://abcdefghijklmnop/popup.html",
        "blob:https://bank.example/9d2c",
        "filesystem:https://bank.example/temporary/x",
        // No scheme the URL Standard reads, so none to allow.
        "bank.example/login",
    ];
    for (url, says) in files
        .map(|url| (url, FILE_PAGES))
        .into_iter()
        .chain(others.map(|url| (url, "Neo does not open this URL")))
    {
        let (operation, reply) = harness
            .submit_op(&run, "page.goto", goto_input(&owners, url))
            .await;
        let refused = failure(reply)?;
        assert_eq!(
            (
                refused.error.owner(),
                refused.error.code(),
                refused.delivery
            ),
            (
                ErrorOwner::Script,
                ErrorCode::INVALID_INPUT,
                Delivery::NotSent
            ),
            "{url:?}: {}",
            refused.error
        );
        assert!(
            refused.error.message().contains(says),
            "{url:?} says why: {}",
            refused.error
        );
        assert_eq!(
            harness.row(operation).await?.error.map(|error| error.code),
            Some("invalid_input".to_owned()),
            "{url:?}: the trace records the attempt"
        );
    }
    assert_eq!(
        harness.fake.count("Page.navigate"),
        0,
        "no refused URL reached the tab"
    );
    Ok(())
}

#[tokio::test]
async fn a_goto_sends_web_and_javascript_urls_to_the_browser() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://bank.example/").await?;
    let (run, _guard) = harness.run()?;
    // The browser fails every load it is sent, so a URL the door let through answers at once.
    harness.fake.answer(
        "Page.navigate",
        json!({ "frameId": "frame", "errorText": "net::ERR_ABORTED" }),
    );
    for url in [
        "localhost:3000/",
        "127.0.0.1:3000/",
        "http://bank.example/",
        "HTTPS://bank.example/login",
        "data:text/html,<p>made in the script</p>",
        "about:blank",
        "javascript:void(document.title = 'ran')",
        " JavaScript:void(0)",
        "java\tscript:void(0)",
    ] {
        let failed = failure(
            harness
                .submit(&run, "page.goto", goto_input(&owners, url))
                .await,
        )?;
        assert_eq!(
            (failed.error.code(), failed.delivery),
            (ErrorCode::NAVIGATION_FAILED, Delivery::Done),
            "{url:?} reached the browser: {}",
            failed.error
        );
    }
    let sent: Vec<Value> = harness
        .fake
        .calls()
        .into_iter()
        .filter(|call| call.method == "Page.navigate")
        .map(|call| call.params["url"].clone())
        .collect();
    assert_eq!(
        sent,
        [
            "http://localhost:3000/",
            "http://127.0.0.1:3000/",
            "http://bank.example/",
            "https://bank.example/login",
            "data:text/html,<p>made in the script</p>",
            "about:blank",
            "javascript:void(document.title = 'ran')",
            "javascript:void(0)",
            "javascript:void(0)",
        ]
        .map(Value::from),
        "each URL is normalized as Playwright does before reaching Chromium"
    );
    Ok(())
}

/// The owner opened a web page in a tab and then went to a local file in it, which core's cached
/// URL has not followed. Every op on that tab is refused, a read or a write, with nothing sent to
/// it: the door reads what the tab shows now from the browser before the crate admits the page.
#[tokio::test]
async fn an_op_on_a_tab_that_shows_a_file_page_is_refused_with_nothing_sent()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://bank.example/").await?;
    let (run, _guard) = harness.run()?;
    assert!(
        harness
            .fake
            .navigate_tab(owners.tab, "file:///etc/hosts", "hosts"),
        "the fake has the owner's tab"
    );
    let before = harness.fake.calls().len();
    let page = handle(&owners);
    let locator =
        json!({ "$neo": "locator", "page": page, "frame": null, "chunks": [], "display": "body" });
    for (name, input) in [
        ("page.content", json!({ "page": page })),
        ("page.title", json!({ "page": page })),
        (
            "page.goto",
            json!({ "page": page, "url": "https://bank.example/", "waitUntil": "load" }),
        ),
        ("locator.click", json!({ "target": locator })),
    ] {
        let mut input = input;
        input["call"] = json!({ "api": name, "timeoutMs": 500 });
        let (operation, reply) = harness.submit_op(&run, name, input).await;
        let refused = failure(reply)?;
        assert_eq!(
            (
                refused.error.owner(),
                refused.error.code(),
                refused.delivery
            ),
            (
                ErrorOwner::Script,
                ErrorCode::INVALID_INPUT,
                Delivery::NotSent
            ),
            "{name}: {}",
            refused.error
        );
        assert!(
            refused.error.message().contains(FILE_PAGES),
            "{name} says what is blocked and what to use: {}",
            refused.error
        );
        assert_eq!(
            harness.row(operation).await?.error.map(|error| error.code),
            Some("invalid_input".to_owned()),
            "{name}: the trace records the attempt"
        );
    }
    // The door's tab reads and the browser-level listings go on; no call reached a page domain.
    let reached: Vec<String> = harness.fake.calls()[before..]
        .iter()
        .map(|call| call.method.clone())
        .filter(|method| {
            [
                "Page.",
                "Runtime.",
                "DOM.",
                "Input.",
                "Emulation.",
                "Accessibility.",
            ]
            .iter()
            .any(|domain| method.starts_with(domain))
        })
        .collect();
    assert!(
        reached.is_empty(),
        "nothing reached the tab's page: {reached:?}"
    );
    assert!(
        harness.is_open(owners.tab).await?,
        "the owner's tab stays open"
    );
    // A close reads and opens nothing, so a script can still close the tab.
    value(
        harness
            .submit(&run, "page.close", json!({ "page": page }))
            .await,
    )
    .map_err(|error| format!("a close is not refused: {error}"))?;
    assert!(
        !harness.is_open(owners.tab).await?,
        "the script closed the tab"
    );
    Ok(())
}

/// The owner's web tab moves onto a local file while an op on it waits for the browser (as a page's
/// own `history.back()` onto a `file:` entry does): the door reads the tab again when the op
/// ends, and withholds whatever the op took there.
#[tokio::test]
async fn an_op_whose_tab_moves_to_a_file_page_while_it_runs_is_withheld()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://bank.example/").await?;
    let (run, _guard) = harness.run()?;
    // `page.title` makes its world after admission; its reply waits until the tab has moved.
    harness.fake.hold_next("Page.createIsolatedWorld");
    let title =
        json!({ "call": { "api": "page.title", "timeoutMs": 5000 }, "page": handle(&owners) });
    let (reply, moved) = tokio::join!(harness.submit(&run, "page.title", title), async {
        within(async {
            while harness.fake.count("Page.createIsolatedWorld") == 0 {
                task::yield_now().await;
            }
        })
        .await?;
        let moved = harness
            .fake
            .navigate_tab(owners.tab, "file:///etc/hosts", "hosts");
        harness.fake.release_held();
        Ok::<_, Box<dyn Error>>(moved)
    });
    assert!(moved?, "the fake has the owner's tab");
    let withheld = failure(reply)?;
    assert_eq!(
        (withheld.error.owner(), withheld.error.code()),
        (ErrorOwner::Script, ErrorCode::INVALID_INPUT),
        "{}",
        withheld.error
    );
    assert!(
        withheld.error.message().contains(FILE_PAGES)
            && withheld.error.message().contains("withholds"),
        "the op's own answer never reaches the script: {}",
        withheld.error
    );
    Ok(())
}

/// A tab's history can hold a local file behind a web page, from the owner's own browsing; a step
/// back or forward onto it is refused before the crate steps, and a step onto a web entry goes
/// to the crate as before.
#[tokio::test]
async fn a_history_step_never_opens_a_file_entry() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://bank.example/").await?;
    let (run, _guard) = harness.run()?;
    let step = |name: &str| {
        json!({
            "call": { "api": name, "timeoutMs": 500 },
            "page": handle(&owners),
            "waitUntil": "load",
        })
    };
    let history = |entries: [&str; 3]| {
        json!({
            "currentIndex": 1,
            "entries": entries
                .iter()
                .enumerate()
                .map(|(id, url)| json!({ "id": id, "url": url, "title": "" }))
                .collect::<Vec<_>>(),
        })
    };
    for (name, entries) in [
        (
            "page.goBack",
            [
                "file:///etc/hosts",
                "https://bank.example/",
                "https://bank.example/next",
            ],
        ),
        (
            "page.goForward",
            [
                "https://bank.example/before",
                "https://bank.example/",
                " FILE:///etc/hosts",
            ],
        ),
    ] {
        harness
            .fake
            .answer("Page.getNavigationHistory", history(entries));
        let (operation, reply) = harness.submit_op(&run, name, step(name)).await;
        let refused = failure(reply)?;
        assert_eq!(
            (
                refused.error.owner(),
                refused.error.code(),
                refused.delivery
            ),
            (
                ErrorOwner::Script,
                ErrorCode::INVALID_INPUT,
                Delivery::NotSent
            ),
            "{name}: {}",
            refused.error
        );
        assert!(
            refused.error.message().contains(FILE_PAGES),
            "{name}: {}",
            refused.error
        );
        assert_eq!(
            harness.row(operation).await?.delivery,
            Some(Delivery::NotSent),
            "{name}: the trace records the attempt"
        );
    }
    assert_eq!(
        harness.fake.count("Page.navigateToHistoryEntry"),
        0,
        "no step onto a file: entry reached the tab"
    );

    harness.fake.answer(
        "Page.getNavigationHistory",
        history([
            "https://bank.example/before",
            "https://bank.example/",
            "https://bank.example/next",
        ]),
    );
    // The fake models no frame tree, so the crate's own step fails after it starts; its call log
    // shows the door let it through.
    for name in ["page.goBack", "page.goForward"] {
        let reply = harness.submit(&run, name, step(name)).await;
        let error = reply
            .outcome
            .err()
            .map(|failure| failure.error.to_string())
            .unwrap_or_default();
        assert!(
            !error.contains(FILE_PAGES) && error.contains("waiting for navigation"),
            "{name} onto a web entry reaches the crate's step: {error}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_reload_reloads_only_a_page_on_an_allowed_scheme() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    // The owner opened a browser page in a tab. The agent never navigated it, but a reload would
    // load it again for its next read. (A tab on a `file:` page is refused for every op:
    // an_op_on_a_tab_that_shows_a_file_page_is_refused_with_nothing_sent.)
    let local = harness.owner_tab("chrome://version/").await?;
    let web = harness.owner_tab("https://bank.example/").await?;
    let (run, _guard) = harness.run()?;
    // The browser never finishes a reload, so one the door lets through times out once sent.
    let reload = |page: &PageRef| {
        json!({
            "call": { "api": "page.reload", "timeoutMs": 500 },
            "page": handle(page),
            "waitUntil": "load",
        })
    };
    let (operation, reply) = harness.submit_op(&run, "page.reload", reload(&local)).await;
    let refused = failure(reply)?;
    assert_eq!(
        (
            refused.error.owner(),
            refused.error.code(),
            refused.delivery
        ),
        (
            ErrorOwner::Script,
            ErrorCode::INVALID_INPUT,
            Delivery::NotSent
        ),
        "{}",
        refused.error
    );
    assert_eq!(
        harness.fake.count("Page.reload"),
        0,
        "the browser never saw the reload"
    );
    let row = harness.row(operation).await?;
    assert_eq!(
        (
            row.delivery,
            row.error.map(|error| (error.owner, error.code))
        ),
        (
            Some(Delivery::NotSent),
            Some((ErrorOwner::Script, "invalid_input".to_owned()))
        ),
        "the trace says what the door refused"
    );

    let sent = failure(harness.submit(&run, "page.reload", reload(&web)).await)?;
    assert_eq!(
        (sent.error.code(), harness.fake.count("Page.reload")),
        (ErrorCode::WAIT_TIMEOUT, 1),
        "a page on the web reloads: {}",
        sent.error
    );
    Ok(())
}

#[tokio::test]
async fn registering_the_playwright_ops_twice_fails_the_start() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let mut builder = RegistryBuilder::default();
    playwright_ops::playwright_ops(&mut builder, &harness.playwright)?;
    let again = playwright_ops::playwright_ops(&mut builder, &harness.playwright);
    assert!(
        matches!(again, Err(RegistryError::Duplicate(_))),
        "the registry reports a duplicate instead of keeping two methods"
    );
    Ok(())
}

#[tokio::test]
async fn the_env_hands_the_crate_the_doors_refusal_and_reports_its_marks()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let open =
        harness.dispatcher_with(Vec::new(), |registry| registry.define(EnvProbe::default()))?;
    let (run, _guard) = harness.run()?;
    let pending = open.submit(
        &run,
        Call::script("test.env".to_owned(), json!({ "handBack": false }), None),
    );
    let operation = pending.operation();
    assert_eq!(
        value(pending.wait().await)?,
        json!({ "sharesTheRunsState": true }),
        "every op of a run works on the run's one RunState"
    );
    assert_eq!(
        harness.row(operation).await?.delivery,
        Some(Delivery::Done),
        "the crate's mark_sent is the door's"
    );

    let probe = EnvProbe::default();
    let seen = Arc::clone(&probe.seen);
    let taken = harness.dispatcher_with(
        vec![Arc::new(TakenWhileWaiting::default()) as Arc<dyn Policy>],
        |registry| registry.define(probe),
    )?;
    for hand_back in [false, true] {
        let reply = taken
            .submit(
                &run,
                Call::script(
                    "test.env".to_owned(),
                    json!({ "handBack": hand_back }),
                    None,
                ),
            )
            .wait()
            .await;
        let failure = failure(reply)?;
        assert_eq!(
            (
                failure.error.owner(),
                failure.error.code(),
                failure.delivery
            ),
            (
                ErrorOwner::Host,
                ErrorCode::OWNER_HAS_CONTROL,
                Delivery::NotSent
            ),
            "handBack: {hand_back}: the script sees the rule's own error, whatever the crate made \
             of it"
        );
    }
    let crate_saw = Refusal {
        owner: ErrorOwner::Host,
        code: "owner_has_control".to_owned(),
        message: "the owner took the tab while the op waited".to_owned(),
        host_code: None,
    };
    assert_eq!(
        *seen.lock().unwrap_or_else(PoisonError::into_inner),
        [crate_saw.clone(), crate_saw],
        "the rule's refusal reaches the crate unchanged"
    );
    Ok(())
}

#[tokio::test]
async fn a_crate_failure_keeps_its_owner_and_timeout_and_its_answer_decides_delivery()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let door = harness.dispatcher_with(Vec::new(), |registry| registry.define(CrateFailure))?;
    let (run, _guard) = harness.run()?;
    let cases = [
        // Timeouts are ambiguous: an input sent before the deadline may have landed.
        (
            "not_actionable",
            true,
            ErrorOwner::Website,
            ErrorCode::NOT_ACTIONABLE,
            true,
            Delivery::Unknown,
        ),
        (
            "wait_timeout",
            true,
            ErrorOwner::Website,
            ErrorCode::WAIT_TIMEOUT,
            true,
            Delivery::Unknown,
        ),
        (
            "navigation_observation_timeout",
            true,
            ErrorOwner::Neo,
            ErrorCode::NAVIGATION_OBSERVATION_TIMEOUT,
            true,
            Delivery::Unknown,
        ),
        (
            "no_match",
            false,
            ErrorOwner::Script,
            ErrorCode::NO_MATCH,
            true,
            Delivery::NotSent,
        ),
        // Neo saw these end, so a sent op is done.
        (
            "strict_mode_violation",
            true,
            ErrorOwner::Script,
            ErrorCode::STRICT_MODE_VIOLATION,
            false,
            Delivery::Done,
        ),
        (
            "internal",
            false,
            ErrorOwner::Neo,
            ErrorCode::INTERNAL_ERROR,
            false,
            Delivery::NotSent,
        ),
    ];
    for (code, sent, owner, expected, timeout, delivery) in cases {
        let reply = door
            .submit(
                &run,
                Call::script(
                    "test.crateFailure".to_owned(),
                    json!({ "code": code, "sent": sent }),
                    None,
                ),
            )
            .wait()
            .await;
        let failure = failure(reply)?;
        assert_eq!(
            (
                failure.error.owner(),
                failure.error.code(),
                failure.delivery
            ),
            (owner, expected, delivery),
            "{code}"
        );
        assert_eq!(
            failure.error.is_timeout(),
            timeout,
            "{code}: a Playwright timeout throws TimeoutError"
        );
        assert_eq!(failure.error.message(), format!("locator.click: {code}"));
    }
    Ok(())
}

#[tokio::test]
async fn an_action_runs_in_the_crate_on_the_tab_its_locator_names() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.owner_tab("https://example.com/").await?;
    let (run, _guard) = harness.run()?;
    let input = json!({
        "call": { "api": "page.click", "timeoutMs": 200 },
        "target": locator(&handle(&page)),
        "strict": "strict",
    });
    let (operation, reply) = harness.submit_op(&run, "locator.click", input).await;
    // The fake browser has no isolated world to click in, so the crate's click stops before it
    // sends anything; `tests/playwright_browseros.rs` clicks on a real one. What is pinned here is
    // that the door let the action through to the crate, on the locator's tab.
    let failure = failure(reply)?;
    assert_ne!(
        failure.error.code(),
        ErrorCode::INVALID_INPUT,
        "the pointer input must decode before exercising page admission: {}",
        failure.error
    );
    assert!(
        harness.fake.count("Page.createIsolatedWorld") > 0,
        "the admitted click must attempt the fake browser's missing isolated world: {}",
        failure.error
    );
    assert_eq!(failure.delivery, Delivery::NotSent, "{}", failure.error);
    let message = failure.error.message();
    assert!(
        message.starts_with("page.click: ") && !message.contains("not built"),
        "the crate's own message, named after the API the script called: {}",
        failure.error
    );
    assert_eq!(
        harness.row(operation).await?.tab,
        Some(page.tab),
        "the rules and the trace see the tab the locator names"
    );
    Ok(())
}

/// Teardown may win before prelude setup's task starts. Both calls enter the real registry;
/// only the foreground call belongs to the script's promise obligations.
#[tokio::test]
async fn prelude_listener_setup_is_cancelled_without_a_not_awaited_notice()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.owner_tab("https://example.com/").await?;
    let (run, _guard) = harness.run()?;
    let listen = harness.dispatcher.submit(
        &run,
        Call::script(
            "events.listen".to_owned(),
            json!({
                "call":{"api":"page.on","timeoutMs":0}, "page":handle(&page),
                "event":"workerstate", "enabled":true, "since":0,
            }),
            None,
        ),
    );
    let foreground = harness.dispatcher.submit(
        &run,
        Call::script(
            "page.title".to_owned(),
            json!({
                "call":{"api":"page.title","timeoutMs":0}, "page":handle(&page),
            }),
            None,
        ),
    );
    let listen_id = listen.operation();
    let foreground_id = foreground.operation();
    // This current-thread test does not yield between admission and teardown, so both
    // operations are deterministically pending rather than racing a fast fake browser.
    run.stop(StopReason::ScriptReturned);
    drop(run.close_ops());
    let _listen_reply = listen.wait().await;
    let _foreground_reply = foreground.wait().await;
    let report = run.report();
    let names: Vec<_> = report.not_awaited.iter().map(|line| line.name).collect();
    assert_eq!(
        names,
        ["page.title"],
        "only the script's own unawaited op is reported"
    );
    let listen = harness.row(listen_id).await?;
    assert_eq!(
        (listen.outcome, listen.delivery, listen.error),
        (Some(Outcome::Cancelled), Some(Delivery::NotSent), None),
        "prelude setup is cancelled without a not_awaited trace error"
    );
    let foreground = harness.row(foreground_id).await?;
    assert_eq!(
        foreground.error.map(|error| error.code),
        Some("not_awaited".to_owned()),
        "the foreground operation retains the script's not_awaited error"
    );
    Ok(())
}

#[tokio::test]
async fn the_crate_gets_the_page_the_door_resolved_never_the_lazy_one() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let listen = json!({
        "call": { "api": "page.on", "timeoutMs": 0 },
        "page": { "$neo": "page", "tab": null },
        "event": "dialog",
        "enabled": true,
        "since": 0,
    });
    let (operation, reply) = harness.submit_op(&run, "events.listen", listen).await;
    let opened = reply
        .default_page
        .clone()
        .ok_or("the door opened the run's page for the listener")?;
    assert_eq!(
        value(reply)?,
        Value::Null,
        "the crate admitted a real page: it refuses the lazy page's `id: null`"
    );
    assert_eq!(
        harness.row(operation).await?.page.tab_id,
        Some(opened.tab),
        "the op's page facts name the tab the door opened, which the crate acted on"
    );
    Ok(())
}

#[tokio::test]
async fn a_locator_whose_page_is_a_bare_id_is_the_scripts_invalid_input()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.owner_tab("https://example.com/").await?;
    let (run, _guard) = harness.run()?;
    let input = json!({ "target": locator(&json!(page.tab.0)), "strict": "strict" });
    let failure = failure(harness.submit(&run, "locator.click", input).await)?;
    assert_eq!(
        (
            failure.error.owner(),
            failure.error.code(),
            failure.delivery
        ),
        (
            ErrorOwner::Script,
            ErrorCode::INVALID_INPUT,
            Delivery::NotSent
        )
    );
    assert!(
        failure.error.message().contains("`target.page`"),
        "the message names where the page brand belongs: {}",
        failure.error
    );
    Ok(())
}

#[tokio::test]
async fn a_rule_that_refuses_the_crates_own_recheck_is_what_the_script_sees()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.owner_tab("https://example.com/").await?;
    let door = harness.dispatcher_with(
        vec![Arc::new(TakenWhileWaiting::default()) as Arc<dyn Policy>],
        |registry| playwright_ops::playwright_ops(registry, &harness.playwright),
    )?;
    let (run, _guard) = harness.run()?;
    let reload = json!({
        "call": { "api": "page.reload", "timeoutMs": 5000 },
        "page": handle(&page),
        "waitUntil": "load",
    });
    let reply = within(
        door.submit(&run, Call::script("page.reload".to_owned(), reload, None))
            .wait(),
    )
    .await?;
    let failure = failure(reply)?;
    assert_eq!(
        (
            failure.error.owner(),
            failure.error.code(),
            failure.delivery
        ),
        (
            ErrorOwner::Host,
            ErrorCode::OWNER_HAS_CONTROL,
            Delivery::NotSent
        ),
        "neo-playwright rechecks right before it reloads, and nothing was sent: {}",
        failure.error
    );
    assert_eq!(
        harness.fake.count("Page.reload"),
        0,
        "the reload never reached the browser"
    );
    Ok(())
}

#[tokio::test]
async fn opener_facts_name_the_linked_tab_without_acquiring_it() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let opener = harness.owner_tab("https://opener.example/").await?;
    let popup = harness.owner_tab("https://popup.example/").await?;
    let target = harness
        .link
        .page(opener.tab)
        .await
        .ok_or("opener cached")?
        .target_id;
    harness.fake.answer(
        "Target.getTargetInfo",
        json!({ "targetInfo": { "openerId": target } }),
    );
    let (run, _guard) = harness.run()?;
    let before = harness.tabs.is_open(opener.tab);
    let reply = harness
        .submit(&run, "page.opener", json!({ "page": handle(&popup) }))
        .await;
    assert!(reply.outcome.is_ok(), "{:?}", reply.outcome);
    assert_eq!(
        reply.pages.iter().map(|fact| fact.tab).collect::<Vec<_>>(),
        [popup.tab, opener.tab]
    );
    assert!(reply.pages.iter().all(|fact| !fact.closed));
    assert!(reply.observed.is_some());
    assert_eq!(harness.tabs.is_open(opener.tab), before);
    Ok(())
}

/// Models the independent watcher recording a close while the core cache remains live.
struct RecordedClose(TargetId);
impl EventLog for RecordedClose {
    fn head(&self) -> u64 {
        7
    }
    fn closed(&self, target: &TargetId) -> Option<u64> {
        (target == &self.0).then_some(7)
    }
}

/// A watcher holding a frame-tree version for each of the given tabs.
struct Versions(Vec<TabId>);
impl EventLog for Versions {
    fn head(&self) -> u64 {
        0
    }
    fn closed(&self, _target: &TargetId) -> Option<u64> {
        None
    }
    fn frame_versions(&self) -> Vec<(TabId, u64)> {
        self.0.iter().map(|tab| (*tab, 3)).collect()
    }
}

/// A page read that answers at once.
struct Peek;
#[derive(Deserialize)]
struct PeekInput {
    page: PageTarget,
}
impl Method for Peek {
    type Input = PeekInput;
    type Output = Value;
    fn spec(&self) -> MethodSpec {
        MethodSpec::new("page.title", Effect::Read)
    }
    fn target(input: &PeekInput) -> Option<&PageTarget> {
        Some(&input.page)
    }
    fn call(
        &self,
        _op: &OpCtx<Self>,
        _input: PeekInput,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(Ok(json!("title")))
    }
}

/// A run's frame versions cross the boundary its page facts do. The raw reply, which a script
/// can read through `__neo.onReply`, names the frame activity of no tab the run was not given
/// a fact of, never Neo's own tab, and none of a tab whose reads the owner's lock blocks.
#[tokio::test]
async fn a_reply_carries_frame_versions_only_of_the_runs_readable_listed_tabs()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let listed = harness.owner_tab("https://a.example/").await?;
    let locked = harness.owner_tab("https://b.example/").await?;
    let unlisted = harness.owner_tab("https://c.example/").await?;
    let neo = within(blank_tab(&harness).ensure(Adopt::Allowed))
        .await?
        .ok_or("Neo's tab opened")?;
    let mut registry = RegistryBuilder::default();
    registry.define(Peek)?;
    let claims = Arc::new(CaptchaClaims::default());
    let door = Arc::new(Dispatcher::new(
        registry.build(),
        super::door_rules(&harness.takeover, &claims),
        harness.sink.clone(),
        Arc::clone(&harness.link),
        Arc::clone(&harness.tabs),
        Observer::new(Arc::new(Versions(vec![
            listed.tab,
            locked.tab,
            unlisted.tab,
            neo,
        ]))),
    ));
    let (run, _guard) = harness.run()?;
    let fact = |tab: TabId| PageFact {
        tab,
        task_id: None,
        url: Some("https://a.example/".to_owned()),
        title: None,
        closed: false,
        viewport_size: None,
        workers: None,
    };
    let tabs = |versions: &[FrameVersion]| {
        versions
            .iter()
            .map(|version| version.tab)
            .collect::<Vec<_>>()
    };
    let start = door.listed_frame_versions(&run, &[fact(listed.tab), fact(locked.tab), fact(neo)]);
    assert_eq!(
        tabs(&start),
        [listed.tab, locked.tab],
        "the init names the listed tabs, never Neo's, and no tab it does not list"
    );
    let _lock = harness.lock(locked.tab, Reads::Block).await?;
    let reply = door
        .submit(
            &run,
            Call::script(
                "page.title".into(),
                json!({ "page": handle(&listed) }),
                None,
            ),
        )
        .wait()
        .await;
    assert!(reply.outcome.is_ok(), "{:?}", reply.outcome);
    assert_eq!(
        tabs(&reply.frames),
        [listed.tab],
        "the owner's lock blocks reads of the locked tab: its activity is not named either"
    );
    Ok(())
}

/// An already-delivered event entering the door's normal post-operation fact observation.
struct DeliveredClose;
#[derive(Deserialize)]
struct CloseInput {
    page: PageTarget,
}
impl Method for DeliveredClose {
    type Input = CloseInput;
    type Output = Value;
    fn spec(&self) -> MethodSpec {
        MethodSpec::new("events.wait", Effect::Read)
    }
    fn target(input: &CloseInput) -> Option<&PageTarget> {
        Some(&input.page)
    }
    fn call(
        &self,
        op: &OpCtx<Self>,
        _input: CloseInput,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(
            op.page()
                .map(|page| json!({ "seq": 7, "event": "close", "page": handle(page) })),
        )
    }
}

#[tokio::test]
async fn a_delivered_close_has_a_closed_fact_while_core_still_lists_its_target()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.owner_tab("https://lag.example/").await?;
    let target = harness
        .link
        .page(page.tab)
        .await
        .ok_or("tab cached")?
        .target_id;
    let mut registry = RegistryBuilder::default();
    registry.define(DeliveredClose)?;
    let door = Arc::new(Dispatcher::new(
        registry.build(),
        Vec::new(),
        harness.sink.clone(),
        Arc::clone(&harness.link),
        Arc::clone(&harness.tabs),
        Observer::new(Arc::new(RecordedClose(target))),
    ));
    let (run, _guard) = harness.run()?;
    let reply = door
        .submit(
            &run,
            Call::script("events.wait".into(), json!({ "page": handle(&page) }), None),
        )
        .wait()
        .await;
    assert!(
        harness.link.page(page.tab).await.is_some(),
        "the independent core cache is still live"
    );
    assert_eq!(
        reply.outcome.as_ref().ok().and_then(|v| v.get("event")),
        Some(&json!("close"))
    );
    assert_eq!(reply.pages.len(), 1);
    assert!(reply.pages[0].closed);
    assert_eq!(reply.observed.map(|at| at.events), Some(7));
    Ok(())
}

/// Pauses after the real crate copied and reported its close, before the door observes core.
struct InFlightClose {
    name: &'static str,
    playwright: Arc<PlaywrightFacade>,
    copied: Arc<Notify>,
    resume: Arc<Notify>,
}
#[derive(Deserialize)]
struct EventDeliveryInput {
    page: Option<PageTarget>,
}
impl Method for InFlightClose {
    type Input = EventDeliveryInput;
    type Output = Value;
    fn spec(&self) -> MethodSpec {
        MethodSpec::new(self.name, Effect::Read)
    }
    fn target(input: &EventDeliveryInput) -> Option<&PageTarget> {
        input.page.as_ref()
    }
    async fn call(&self, op: &OpCtx<Self>, _input: EventDeliveryInput) -> Result<Value, LeafError> {
        let args = if self.name == "events.wait" {
            json!({ "call": {"api":"page.waitForEvent", "timeoutMs":10000},
                "page":handle(op.page()?), "event":"close", "since":0 })
        } else {
            json!({ "call": {"api":"events.next", "timeoutMs":0} })
        };
        let operation = Op::from_name(self.name).ok_or_else(|| {
            LeafError::definite(OperationError::neo(
                ErrorCode::INTERNAL_ERROR,
                "missing event operation",
            ))
        })?;
        let env = OpCtxEnv::new(
            op,
            false,
            Arc::default(),
            Arc::new(FileAccess::new(|| vec!["/".into()], Vec::new())),
        );
        let value = self
            .playwright
            .get()
            .await
            .invoke(&env, operation, args)
            .await
            .map_err(|error| leaf_error(&error))?;
        let value = EventRelease::of(self.name).release(op, value)?;
        self.copied.notify_one();
        self.resume.notified().await;
        Ok(value)
    }
}

/// Fills the page-event ring while preserving every event on the fake CDP transport.
async fn fill_event_ring(
    harness: &Harness,
    noise: &PageRef,
    close_seq: u64,
) -> Result<(), Box<dyn Error>> {
    let pw = harness.playwright.get().await;
    // The generic fake returns an empty Target.getTargetInfo for opener reads. Supply the
    // attachment event that a real browser uses to bind this console stream to its target.
    let noise_target = harness
        .link
        .page(noise.tab)
        .await
        .ok_or("noise cached")?
        .target_id;
    assert!(harness.fake.page_event(
        noise.tab,
        "Target.attachedToTarget",
        json!({
            "sessionId": format!("session-{}", noise_target.as_str()),
            "targetInfo": {"targetId":noise_target, "type":"page", "url":"https://noise.example/"}
        })
    ));
    for batch in 1..=64 {
        for call in 0..16 {
            // Each call its own stamp, as Chromium's are: identical reports are one replayed.
            assert!(harness.fake.page_event(
                noise.tab,
                "Runtime.consoleAPICalled",
                json!({
                    "type":"log", "args":[{"type":"string","value":"noise"}],
                    "executionContextId":1, "timestamp": batch * 16 + call
                })
            ));
        }
        // Backpressure the fake's transport so this tests event-log eviction, with every
        // record delivered to the watcher, independently of transport-ring capacity.
        within(async {
            while pw.event_head() < close_seq + batch * 16 {
                time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await?;
    }
    Ok(())
}

/// Two URL-ready fake tabs and their real watcher subscriptions. A plain data bag.
struct EventTabs {
    page: PageRef,
    noise: PageRef,
    target: TargetId,
}

async fn prepare_event_tabs(
    harness: &Harness,
    run: &Arc<RunCtx>,
) -> Result<EventTabs, Box<dyn Error>> {
    // The generic fake supplies only Browser.getTabs. Complete the watcher's separate
    // discovery exchange before creating the tabs whose events this test exercises.
    harness
        .fake
        .answer("Target.getTargets", json!({"targetInfos":[]}));
    let pw = harness.playwright.get().await;
    within(async {
        while harness.fake.count("Browser.setDownloadBehavior") == 0 {
            time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await?;
    let page = harness.owner_tab("https://closing.example/").await?;
    let noise = harness.owner_tab("https://noise.example/").await?;
    let target = harness
        .link
        .page(page.tab)
        .await
        .ok_or("cached target")?
        .target_id;
    for tab in [&page, &noise] {
        let info = harness.link.page(tab.tab).await.ok_or("cached event tab")?;
        assert!(harness.fake.page_event(
            tab.tab,
            "Target.targetInfoChanged",
            json!({
                "targetInfo": {"targetId":info.target_id,"type":"page","url":info.url}
            })
        ));
    }
    within(async {
        while pw.event_head() < 2 {
            time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await?;
    for (tab, event) in [(&page, "close"), (&noise, "console")] {
        let answer = harness
            .submit(
                run,
                "events.listen",
                json!({
                    "call":{"api":"page.on","timeoutMs":0}, "page":handle(tab),
                    "event":event, "enabled":true, "since":0
                }),
            )
            .await;
        value(answer)?;
    }
    Ok(EventTabs {
        page,
        noise,
        target,
    })
}

async fn evicted_close_case(name: &'static str, rebind: bool) -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let EventTabs {
        page,
        noise,
        target,
    } = prepare_event_tabs(&harness, &run).await?;
    let pw = harness.playwright.get().await;
    let copied = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let door = harness.dispatcher_with(Vec::new(), |registry| {
        registry.define(InFlightClose {
            name,
            playwright: Arc::clone(&harness.playwright),
            copied: Arc::clone(&copied),
            resume: Arc::clone(&resume),
        })
    })?;
    let input = if name == "events.wait" {
        json!({"page":handle(&page)})
    } else {
        json!({})
    };
    let pending = door.submit(&run, Call::script(name.into(), input, None));
    // The target watcher sees the destroy while the fake Browser.getTabs still lists T.
    // This reproduces independent CDP consumers without relying on scheduling duration.
    assert!(harness.fake.page_event(
        page.tab,
        "Target.targetDestroyed",
        json!({"targetId":target})
    ));
    within(copied.notified()).await?;
    let close_seq = pw
        .closed_target(&target)
        .ok_or("copied close is retained")?;
    fill_event_ring(&harness, &noise, close_seq).await?;
    assert_eq!(
        pw.closed_target(&target),
        None,
        "the copied close left the ring"
    );
    if rebind {
        assert!(harness.fake.replace_target(page.tab));
        harness.listed(page.tab).await?;
        within(async {
            while harness
                .link
                .page(page.tab)
                .await
                .is_some_and(|info| info.target_id == target)
            {
                time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await?;
    }
    resume.notify_one();
    let reply = pending.wait().await;
    assert!(reply.outcome.is_ok(), "{:?}", reply.outcome);
    if name == "events.next" {
        assert_eq!(
            reply.outcome.as_ref().ok().and_then(|v| v.get("lost")),
            Some(&json!(0))
        );
    }
    let fact = reply
        .pages
        .iter()
        .find(|fact| fact.tab == page.tab)
        .ok_or("delivered tab fact")?;
    assert_eq!(fact.closed, !rebind);
    let observed = reply.observed.as_ref().ok_or("event observation")?;
    assert!(observed.events >= close_seq + 1024);
    assert_eq!(observed.closes.len(), usize::from(!rebind));
    assert_eq!(
        harness.tabs.gone_target(page.tab),
        (!rebind).then(|| target.clone())
    );
    let later = harness
        .submit(&run, "page.opener", json!({"page":handle(&page)}))
        .await;
    assert_eq!(later.pages.first().map(|fact| fact.closed), Some(!rebind),);
    assert!(harness.fake.close_tab(page.tab));
    within(async {
        while harness.link.page(page.tab).await.is_some() {
            time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await?;
    assert_eq!(
        harness.tabs.gone_target(page.tab),
        None,
        "core detach cleaned the binding"
    );
    Ok(())
}

#[tokio::test]
async fn evicted_wait_close_keeps_the_lagging_binding_closed() -> Result<(), Box<dyn Error>> {
    evicted_close_case("events.wait", false).await
}
#[tokio::test]
async fn evicted_pump_close_keeps_the_lagging_binding_closed() -> Result<(), Box<dyn Error>> {
    evicted_close_case("events.next", false).await
}
#[tokio::test]
async fn evicted_wait_close_respects_a_replacement_target() -> Result<(), Box<dyn Error>> {
    evicted_close_case("events.wait", true).await
}
#[tokio::test]
async fn evicted_pump_close_respects_a_replacement_target() -> Result<(), Box<dyn Error>> {
    evicted_close_case("events.next", true).await
}

/// The event ring no longer retains either close copied into the pending pump reply.
struct EvictedCloses;
impl EventLog for EvictedCloses {
    fn head(&self) -> u64 {
        1026
    }
    fn closed(&self, _target: &TargetId) -> Option<u64> {
        None
    }
}

/// A pump can report multiple target-bound subscriptions for the same browser tab.
struct TwoTargetCloses {
    page: PageRef,
    targets: [TargetId; 2],
}

/// Holds a real page payload between crate sampling and the door's fact observation.
struct InFlightPayload {
    name: &'static str,
    playwright: Arc<PlaywrightFacade>,
    copied: Arc<Notify>,
    resume: Arc<Notify>,
}

impl Method for InFlightPayload {
    type Input = CloseInput;
    type Output = Value;
    fn spec(&self) -> MethodSpec {
        MethodSpec::new(
            self.name,
            if self.name == "page.goto" {
                Effect::Write
            } else {
                Effect::Read
            },
        )
    }
    fn target(input: &CloseInput) -> Option<&PageTarget> {
        Some(&input.page)
    }
    async fn call(&self, op: &OpCtx<Self>, _input: CloseInput) -> Result<Value, LeafError> {
        let mut args =
            json!({"page":handle(op.page()?), "call":{"api":self.name,"timeoutMs":5000}});
        if self.name == "page.goto" {
            args["url"] = json!("https://same.example/");
            args["waitUntil"] = json!("commit");
        }
        let operation = Op::from_name(self.name).ok_or_else(|| {
            LeafError::definite(OperationError::neo(
                ErrorCode::INTERNAL_ERROR,
                "missing payload operation",
            ))
        })?;
        let env = OpCtxEnv::new(
            op,
            false,
            Arc::default(),
            Arc::new(FileAccess::new(|| vec!["/".into()], Vec::new())),
        );
        let result = self
            .playwright
            .get()
            .await
            .invoke(&env, operation, args)
            .await
            .map_err(|error| leaf_error(&error))?;
        self.copied.notify_one();
        self.resume.notified().await;
        Ok(result)
    }
}

#[tokio::test]
async fn real_payload_reports_its_binding_before_a_same_url_rebind() -> Result<(), Box<dyn Error>> {
    for name in ["page.goto", "page.frames"] {
        for rebind in [false, true] {
            let harness = Harness::new().await?;
            let page = harness.owner_tab("https://same.example/").await?;
            let target = harness
                .link
                .page(page.tab)
                .await
                .ok_or("cached page")?
                .target_id;
            harness.fake.answer("Page.getFrameTree", json!({ "frameTree": {"frame":{"id":target,"url":"https://same.example/","loaderId":"initial"}} }));
            harness
                .fake
                .answer("Target.getTargets", json!({"targetInfos":[]}));
            let copied = Arc::new(Notify::new());
            let resume = Arc::new(Notify::new());
            let door = harness.dispatcher_with(Vec::new(), |registry| {
                registry.define(InFlightPayload {
                    name,
                    playwright: Arc::clone(&harness.playwright),
                    copied: Arc::clone(&copied),
                    resume: Arc::clone(&resume),
                })
            })?;
            let (run, _guard) = harness.run()?;
            let pending = door.submit(
                &run,
                Call::script(name.into(), json!({"page":handle(&page)}), None),
            );
            if name == "page.goto" {
                within(async {
                    while harness.fake.count("Page.navigate") == 0 {
                        task::yield_now().await;
                    }
                })
                .await?;
                assert!(harness.fake.page_event(page.tab, "Page.frameNavigated", json!({"frame":{"id":target,"url":"https://same.example/","loaderId":"navigated"}})));
            }
            within(copied.notified()).await?;
            if rebind {
                assert!(harness.fake.replace_target(page.tab));
                harness.listed(page.tab).await?;
            }
            resume.notify_one();
            let reply = pending.wait().await;
            assert!(reply.outcome.is_ok(), "{:?}", reply.outcome);
            assert_eq!(reply.pages[0].url.as_deref(), Some("https://same.example/"));
            assert!(!reply.pages[0].closed);
            assert_eq!(
                reply.observed.ok_or("payload observation")?.payload_current,
                Some(!rebind),
                "{name}, rebind={rebind}"
            );
        }
    }
    Ok(())
}
impl Method for TwoTargetCloses {
    type Input = Value;
    type Output = Value;
    fn spec(&self) -> MethodSpec {
        MethodSpec::new("events.next", Effect::Read)
    }
    fn target(_input: &Value) -> Option<&PageTarget> {
        None
    }
    fn call(
        &self,
        op: &OpCtx<Self>,
        _input: Value,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        for (index, target) in self.targets.iter().enumerate() {
            op.report_page(
                &self.page,
                PageChange::Gone {
                    target: target.clone(),
                    seq: index as u64 + 1,
                },
            );
        }
        future::ready(Ok(json!({"events":[
            {"event":"close","seq":1,"page":handle(&self.page)},
            {"event":"close","seq":2,"page":handle(&self.page)}
        ],"lost":0})))
    }
}

#[tokio::test]
async fn a_pump_keeps_all_target_claims_after_eviction_in_either_report_order()
-> Result<(), Box<dyn Error>> {
    for reversed in [false, true] {
        let harness = Harness::new().await?;
        let page = harness.owner_tab("https://replacement.example/").await?;
        let current = harness
            .link
            .page(page.tab)
            .await
            .ok_or("current binding")?
            .target_id;
        let old = TargetId("previous-target".into());
        let targets = if reversed {
            [old, current.clone()]
        } else {
            [current.clone(), old]
        };
        let mut registry = RegistryBuilder::default();
        registry.define(TwoTargetCloses {
            page: page.clone(),
            targets,
        })?;
        let door = Arc::new(Dispatcher::new(
            registry.build(),
            Vec::new(),
            harness.sink.clone(),
            Arc::clone(&harness.link),
            Arc::clone(&harness.tabs),
            Observer::new(Arc::new(EvictedCloses)),
        ));
        let (run, _guard) = harness.run()?;
        let reply = door
            .submit(&run, Call::script("events.next".into(), json!({}), None))
            .wait()
            .await;
        assert!(reply.outcome.is_ok());
        assert_eq!(reply.pages.first().map(|fact| fact.closed), Some(true));
        let observed = reply.observed.ok_or("observation is present")?;
        assert_eq!(observed.events, 1026);
        assert_eq!(
            serde_json::to_value(&observed)?["closes"],
            json!([{ "tab": page.tab, "seq": if reversed { 2 } else { 1 } }])
        );
        assert_eq!(harness.tabs.gone_target(page.tab), Some(current));
    }
    Ok(())
}

#[tokio::test]
async fn a_native_pump_retains_both_target_bound_subscriptions_after_eviction()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let EventTabs {
        page,
        noise,
        target: old,
    } = prepare_event_tabs(&harness, &run).await?;
    let pw = harness.playwright.get().await;
    assert!(harness.fake.replace_target(page.tab));
    harness.listed(page.tab).await?;
    let current = harness
        .link
        .page(page.tab)
        .await
        .ok_or("replacement")?
        .target_id;
    assert_ne!(current, old);
    assert!(harness.fake.page_event(
        page.tab,
        "Target.targetInfoChanged",
        json!({
            "targetInfo":{"targetId":current,"type":"page","url":"https://replacement.example/"}
        })
    ));
    within(async {
        while pw.closed_target(&old).is_none() {
            time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await?;
    value(
        harness
            .submit(
                &run,
                "events.listen",
                json!({
                    "call":{"api":"page.on","timeoutMs":0}, "page":handle(&page),
                    "event":"close", "enabled":true, "since":0
                }),
            )
            .await,
    )?;
    assert!(harness.fake.page_event(
        page.tab,
        "Target.targetDestroyed",
        json!({"targetId":current})
    ));
    within(async {
        while pw.closed_target(&current).is_none() {
            time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await?;
    let copied = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let door = harness.dispatcher_with(Vec::new(), |registry| {
        registry.define(InFlightClose {
            name: "events.next",
            playwright: Arc::clone(&harness.playwright),
            copied: Arc::clone(&copied),
            resume: Arc::clone(&resume),
        })
    })?;
    let pending = door.submit(&run, Call::script("events.next".into(), json!({}), None));
    within(copied.notified()).await?;
    let head = pw.event_head();
    fill_event_ring(&harness, &noise, head).await?;
    assert!(pw.closed_target(&old).is_none() && pw.closed_target(&current).is_none());
    resume.notify_one();
    let reply = pending.wait().await;
    let events = reply
        .outcome
        .as_ref()
        .ok()
        .and_then(|value| value.get("events"))
        .and_then(Value::as_array)
        .ok_or("pump events")?;
    assert_eq!(
        events.len(),
        2,
        "both target-bound subscriptions survive in the pump snapshot"
    );
    assert_eq!(reply.pages.first().map(|fact| fact.closed), Some(true));
    assert_eq!(harness.tabs.gone_target(page.tab), Some(current));
    Ok(())
}

#[tokio::test]
async fn related_opener_facts_obey_the_openers_read_lock() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let opener = harness.owner_tab("https://private.example/secret").await?;
    let popup = harness.owner_tab("https://public.example/").await?;
    let target = harness
        .link
        .page(opener.tab)
        .await
        .ok_or("opener cached")?
        .target_id;
    harness.fake.answer(
        "Target.getTargetInfo",
        json!({"targetInfo":{"openerId":target}}),
    );
    let (run, _guard) = harness.run()?;
    let lock = harness.lock(opener.tab, Reads::Block).await?;
    let reply = harness
        .submit(&run, "page.opener", json!({"page":handle(&popup)}))
        .await;
    assert_eq!(
        reply
            .outcome
            .as_ref()
            .ok()
            .and_then(|value| value.get("opener")),
        Some(&handle(&opener))
    );
    assert!(reply.pages.iter().all(|fact| fact.tab != opener.tab));
    assert!(!serde_json::to_string(&reply.pages)?.contains("secret"));
    harness.locks.unlock_host(lock.id)?;
    let reply = harness
        .submit(&run, "page.opener", json!({"page":handle(&popup)}))
        .await;
    assert_eq!(
        reply
            .pages
            .iter()
            .find(|fact| fact.tab == opener.tab)
            .and_then(|fact| fact.url.as_deref()),
        Some("https://private.example/secret")
    );
    Ok(())
}

#[tokio::test]
async fn withheld_close_keeps_internal_evidence_without_releasing_lifecycle()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let EventTabs {
        page,
        noise,
        target,
    } = prepare_event_tabs(&harness, &run).await?;
    let lock = harness.lock(page.tab, Reads::Block).await?;
    let noise_target = harness
        .link
        .page(noise.tab)
        .await
        .ok_or("noise cached")?
        .target_id;
    assert!(harness.fake.page_event(
        noise.tab,
        "Target.attachedToTarget",
        json!({
            "sessionId":format!("session-{}",noise_target.as_str()),
            "targetInfo":{"targetId":noise_target,"type":"page","url":"https://noise.example/"}
        })
    ));
    let pw = harness.playwright.get().await;
    let head = pw.event_head();
    assert!(
        harness
            .fake
            .page_event(noise.tab, "Runtime.consoleAPICalled", console_log("before"))
    );
    assert!(harness.fake.page_event(
        page.tab,
        "Target.targetDestroyed",
        json!({"targetId":target})
    ));
    assert!(
        harness
            .fake
            .page_event(noise.tab, "Runtime.consoleAPICalled", console_log("after"))
    );
    within(async {
        while pw.event_head() < head + 3 {
            task::yield_now().await;
        }
    })
    .await?;
    let reply = harness
        .submit(
            &run,
            "events.next",
            json!({"call":{"api":"events.next","timeoutMs":0}}),
        )
        .await;
    let batch = reply
        .outcome
        .as_ref()
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(batch["withheld"], json!(1));
    let events = batch["events"].as_array().ok_or("event batch")?;
    assert_eq!(
        events
            .iter()
            .map(|event| event["message"]["text"].as_str())
            .collect::<Vec<_>>(),
        [Some("before"), Some("after")]
    );
    assert!(reply.pages.iter().all(|fact| fact.tab != page.tab));
    assert!(
        reply
            .observed
            .as_ref()
            .is_none_or(|at| at.closes.is_empty())
    );
    assert_eq!(harness.tabs.gone_target(page.tab), Some(target.clone()));
    fill_event_ring(&harness, &noise, pw.event_head()).await?;
    assert!(pw.closed_target(&target).is_none());
    harness.locks.unlock_host(lock.id)?;
    let later = harness
        .submit(&run, "page.opener", json!({"page":handle(&page)}))
        .await;
    assert_eq!(later.pages.first().map(|fact| fact.closed), Some(true));
    Ok(())
}

#[tokio::test]
async fn explicit_close_cleans_a_replacement_with_delivered_dead_evidence()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let EventTabs { page, .. } = prepare_event_tabs(&harness, &run).await?;
    // Keep the native tab around until the controlled reply, allowing a target swap while
    // the close waits for the browser to answer its `Target.closeTarget`.
    harness
        .fake
        .answer("Target.closeTarget", json!({ "success": true }));
    harness.fake.hold_next("Target.closeTarget");
    let pending = harness.dispatcher.submit(
        &run,
        Call::script("page.close".into(), json!({"page":handle(&page)}), None),
    );
    within(async {
        while harness.fake.count("Target.closeTarget") == 0 {
            task::yield_now().await;
        }
    })
    .await?;
    assert!(harness.fake.replace_target(page.tab));
    harness.listed(page.tab).await?;
    let replacement = harness
        .link
        .page(page.tab)
        .await
        .ok_or("replacement cached")?
        .target_id;
    assert!(harness.fake.page_event(
        page.tab,
        "Target.targetInfoChanged",
        json!({
            "targetInfo":{"targetId":replacement,"type":"page","url":"https://replacement.example/"}
        })
    ));
    value(
        harness
            .submit(
                &run,
                "events.listen",
                json!({
                    "call":{"api":"page.on","timeoutMs":0}, "page":handle(&page),
                    "event":"close", "enabled":true, "since":0
                }),
            )
            .await,
    )?;
    assert!(harness.fake.page_event(
        page.tab,
        "Target.targetDestroyed",
        json!({"targetId":replacement})
    ));
    let pw = harness.playwright.get().await;
    within(async {
        while pw.closed_target(&replacement).is_none() {
            task::yield_now().await;
        }
    })
    .await?;
    let delivered = harness
        .submit(
            &run,
            "events.next",
            json!({"call":{"api":"events.next","timeoutMs":0}}),
        )
        .await;
    assert!(delivered.outcome.is_ok());
    assert_eq!(delivered.pages.first().map(|fact| fact.closed), Some(true));
    assert_eq!(harness.tabs.gone_target(page.tab), Some(replacement));
    // The browser finishes the close: the tab leaves its list, which the close waits for.
    assert!(harness.fake.unlist_tab(page.tab));
    harness.fake.release_held();
    value(within(pending.wait()).await?)?;
    assert!(harness.link.page(page.tab).await.is_none());
    assert_eq!(harness.tabs.gone_target(page.tab), None);
    Ok(())
}

#[tokio::test]
async fn withholding_target_facts_cannot_change_the_operations_trace_tab()
-> Result<(), Box<dyn Error>> {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct LinkedReply {
        related: PageRef,
        blocked: Arc<AtomicBool>,
    }
    impl Method for LinkedReply {
        type Input = CloseInput;
        type Output = Value;
        fn spec(&self) -> MethodSpec {
            MethodSpec::new("test.related", Effect::Read)
        }
        fn target(input: &CloseInput) -> Option<&PageTarget> {
            Some(&input.page)
        }
        fn call(
            &self,
            op: &OpCtx<Self>,
            _: CloseInput,
        ) -> impl Future<Output = Result<Value, LeafError>> + Send {
            op.report_page(&self.related, PageChange::Linked);
            self.blocked.store(true, Ordering::SeqCst);
            future::ready(Ok(json!({})))
        }
    }
    struct Denied {
        tab: TabId,
        blocked: Arc<AtomicBool>,
    }
    impl Policy for Denied {
        fn check(&self, check: &PolicyCheck<'_>) -> Result<(), OperationError> {
            if check.tab == Some(self.tab) && self.blocked.load(Ordering::SeqCst) {
                return Err(OperationError::neo(
                    ErrorCode::OWNER_HAS_CONTROL,
                    "reads blocked",
                ));
            }
            Ok(())
        }
    }
    let harness = Harness::new().await?;
    let target = harness.owner_tab("https://target.example/").await?;
    let related = harness.owner_tab("https://related.example/").await?;
    let blocked = Arc::new(AtomicBool::new(false));
    let door = harness.dispatcher_with(
        vec![Arc::new(Denied {
            tab: target.tab,
            blocked: Arc::clone(&blocked),
        })],
        |registry| {
            registry.define(LinkedReply {
                related: related.clone(),
                blocked,
            })
        },
    )?;
    let (run, _guard) = harness.run()?;
    let pending = door.submit(
        &run,
        Call::script("test.related".into(), json!({"page":handle(&target)}), None),
    );
    let operation = pending.operation();
    let reply = pending.wait().await;
    assert!(reply.outcome.is_err());
    assert_eq!(
        reply.pages.iter().map(|fact| fact.tab).collect::<Vec<_>>(),
        [related.tab]
    );
    let row = harness.row(operation).await?;
    assert_eq!(row.tab, Some(target.tab));
    assert_eq!(
        row.page.tab_id, None,
        "the withheld target has no released after-state"
    );
    assert_eq!(row.page.url, None);
    Ok(())
}

#[tokio::test]
async fn a_late_read_lock_withholds_the_close_and_its_binding_fact() -> Result<(), Box<dyn Error>> {
    for rebind in [false, true] {
        let harness = Harness::new().await?;
        let (run, _guard) = harness.run()?;
        let EventTabs { page, target, .. } = prepare_event_tabs(&harness, &run).await?;
        let copied = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        let claims = Arc::new(CaptchaClaims::default());
        let door =
            harness.dispatcher_with(super::door_rules(&harness.takeover, &claims), |registry| {
                registry.define(InFlightClose {
                    name: "events.next",
                    playwright: Arc::clone(&harness.playwright),
                    copied: Arc::clone(&copied),
                    resume: Arc::clone(&resume),
                })
            })?;
        let pending = door.submit(&run, Call::script("events.next".into(), json!({}), None));
        assert!(harness.fake.page_event(
            page.tab,
            "Target.targetDestroyed",
            json!({"targetId":target})
        ));
        // InFlightClose pauses after the actual early EventRelease policy has passed.
        within(copied.notified()).await?;
        if rebind {
            assert!(harness.fake.replace_target(page.tab));
            harness.listed(page.tab).await?;
        }
        let lock = harness.lock(page.tab, Reads::Block).await?;
        resume.notify_one();
        let reply = within(pending.wait()).await?;
        let value = reply
            .outcome
            .as_ref()
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(value["events"], json!([]));
        assert_eq!(value["withheld"], json!(1));
        assert!(reply.pages.iter().all(|fact| fact.tab != page.tab));
        assert!(
            reply
                .observed
                .as_ref()
                .is_none_or(|observed| observed.closes.is_empty())
        );
        assert_eq!(
            harness.tabs.gone_target(page.tab),
            (!rebind).then_some(target)
        );
        harness.locks.unlock_host(lock.id)?;
        let next = harness
            .submit(&run, "page.opener", json!({"page":handle(&page)}))
            .await;
        assert_eq!(next.pages.first().map(|fact| fact.closed), Some(!rebind));
    }
    Ok(())
}

/// Use the full built-in registry: the runtime-only rig intentionally serves fake methods.
#[test]
fn captured_bytes_can_be_saved_from_quickjs_without_admitting_a_tab() -> Result<(), TestError> {
    on_neo_runtime(async {
        let harness = Harness::new().await.map_err(|error| error.to_string())?;
        let own = tempfile::tempdir()?;
        let target = own.path().join("nested/screenshot.png");
        let (run, _guard) = harness.run().map_err(|error| error.to_string())?;
        let script = Script {
            code: format!(
                "await __neo.invoke('page.saveScreenshot', {{ path: {}, base64: 'iVBORw0KGgo=' }}); return true;",
                json!(target)
            ),
            page: None,
            pages: Vec::new(),
            observed: None,
        };
        let end = run_script(&run, &harness.dispatcher, script).await;
        match end.outcome {
            ScriptOutcome::Returned(value) => assert_eq!(value, json!(true)),
            other => return Err(format!("expected saved bytes, got {other:?}").into()),
        }
        assert_eq!(fs::read(target)?, b"\x89PNG\r\n\x1a\n");
        assert_eq!(harness.fake.count("Target.createTarget"), 0);
        Ok(())
    })
}

/// Hosted mode shows no console messages or page errors, by design (`neo-playwright`'s
/// `events/event_op.rs`, `lease_runtime`): a valid wait fails before spending its timeout, and
/// listeners are accepted and stay silent.
#[tokio::test]
async fn hosted_console_and_pageerror_waits_refuse_without_changing_listeners()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::with_mode::<false>(None, HOSTED_MODE).await?;
    let page = harness.owner_tab("https://example.com/events").await?;
    let (run, _guard) = harness.run()?;
    for event in ["console", "pageerror"] {
        let listen = json!({"call":{"api":"page.on","timeoutMs":0},
            "page":handle(&page),"event":event,"enabled":true,"since":0});
        assert_eq!(
            value(harness.submit(&run, "events.listen", listen).await)?,
            Value::Null
        );
        let wait = json!({"call":{"api":"page.waitForEvent","timeoutMs":30000},
            "page":handle(&page),"event":event,"since":0});
        let refused = failure(within(harness.submit(&run, "events.wait", wait)).await?)?;
        assert_eq!(
            (
                refused.error.owner(),
                refused.error.code(),
                refused.delivery
            ),
            (ErrorOwner::Neo, ErrorCode::UNSUPPORTED, Delivery::NotSent)
        );
        assert!(
            refused
                .error
                .message()
                .contains("Hosted BrowserOS does not emit")
        );
        assert!(refused.error.message().contains(event));
    }
    Ok(())
}

#[tokio::test]
async fn a_nonhosted_console_wait_still_receives_native_messages() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.owner_tab("https://example.com/events").await?;
    let (run, _guard) = harness.run()?;
    let listen = json!({"call":{"api":"page.on","timeoutMs":0},
        "page":handle(&page),"event":"console","enabled":true,"since":0});
    value(harness.submit(&run, "events.listen", listen).await)?;
    harness.fake.page_event(
        page.tab,
        "Runtime.consoleAPICalled",
        console_log("native console"),
    );
    let wait = json!({"call":{"api":"page.waitForEvent","timeoutMs":30000},
        "page":handle(&page),"event":"console","since":0});
    let message = value(within(harness.submit(&run, "events.wait", wait)).await?)?;
    assert!(message.to_string().contains("native console"), "{message}");
    Ok(())
}

#[tokio::test]
async fn worker_bookkeeping_can_unsubscribe_after_its_page_closed() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.owner_tab("https://example.com/worker").await?;
    let (run, _guard) = harness.run()?;
    value(
        harness
            .submit(
                &run,
                "events.listen",
                json!({
                    "call":{"api":"page.on","timeoutMs":0},
                    "page":handle(&page),"event":"workerstate","enabled":true,"since":0
                }),
            )
            .await,
    )?;
    value(
        harness
            .submit(
                &run,
                "page.close",
                json!({
                    "call":{"api":"page.close","timeoutMs":0},"page":handle(&page)
                }),
            )
            .await,
    )?;
    let reply = harness
        .submit(
            &run,
            "events.listen",
            json!({
                "call":{"api":"page.off","timeoutMs":0},
                "page":null,"retiredTab":page.tab,"event":"workerstate","enabled":false,"since":0
            }),
        )
        .await;
    assert!(reply.default_page.is_none(), "cleanup must not open a tab");
    assert_eq!(value(reply)?, Value::Null);
    let refused = failure(
        harness
            .submit(
                &run,
                "events.listen",
                json!({
                    "call":{"api":"page.on","timeoutMs":0},
                    "page":null,"retiredTab":page.tab,"event":"workerstate","enabled":true,"since":0
                }),
            )
            .await,
    )?;
    assert!(refused.error.message().contains("retiredTab"));
    Ok(())
}

#[tokio::test]
async fn parallel_unhosted_runs_get_distinct_unowned_default_tabs() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owner = harness.owner_tab("https://owner.example/").await?;
    let (first, _first_guard) = harness.run()?;
    let (second, _second_guard) = harness.run()?;
    let lazy = json!({ "page": { "$neo": "page", "tab": null } });
    let (one, two) = tokio::join!(
        harness.submit(&first, "page.opener", lazy.clone()),
        harness.submit(&second, "page.opener", lazy.clone()),
    );
    let one_page = one.default_page.clone().ok_or("first default")?;
    let two_page = two.default_page.clone().ok_or("second default")?;
    value(one)?;
    value(two)?;
    assert_ne!(
        one_page, two_page,
        "parallel runs cannot inherit each other's tab"
    );
    assert_ne!(one_page, owner);
    assert_ne!(two_page, owner);
    assert_eq!(harness.tabs.owner(one_page.tab), None);
    assert_eq!(harness.tabs.owner(two_page.tab), None);
    assert_eq!(
        harness
            .submit(&first, "page.opener", lazy)
            .await
            .default_page,
        Some(one_page)
    );
    assert_eq!(
        harness.fake.count("Browser.createTab"),
        2,
        "one lazy tab per run"
    );
    Ok(())
}

#[tokio::test]
async fn tasks_list_oldest_first_and_close_only_their_own_tabs() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owner = harness.owner_tab("https://owner.example/").await?;
    let (run, _guard) = harness.run()?;
    let one = value(
        harness
            .submit(&run, "neo.createTask", json!({ "name": " HN top 5 " }))
            .await,
    )?;
    let two = value(harness.submit(&run, "neo.createTask", json!({})).await)?;
    assert!(
        one["id"]
            .as_str()
            .ok_or("task id")?
            .starts_with("hn-top-5-")
    );
    assert_eq!(one["pages"], json!([]));
    let first = page_of(&value(
        harness
            .submit(&run, "task.newPage", json!({ "task": one["id"] }))
            .await,
    )?)?;
    let other = page_of(&value(
        harness
            .submit(&run, "task.newPage", json!({ "task": two["id"] }))
            .await,
    )?)?;
    let second = page_of(&value(
        harness
            .submit(&run, "task.newPage", json!({ "task": one["id"] }))
            .await,
    )?)?;
    let (later, _later_guard) = harness.run()?;
    let fetched = harness
        .submit(&later, "neo.task", json!({ "id": one["id"] }))
        .await;
    assert_eq!(fetched.pages.len(), 2);
    assert!(
        fetched
            .pages
            .iter()
            .all(|fact| fact.task_id.as_deref() == one["id"].as_str())
    );
    assert_eq!(value(fetched)?["pages"], json!([first, second]));
    let closed = value(
        harness
            .submit(&later, "task.close", json!({ "task": one["id"] }))
            .await,
    )?;
    assert_eq!(closed["closed"], json!([first.tab, second.tab]));
    assert!(!harness.is_open(first.tab).await?);
    assert!(!harness.is_open(second.tab).await?);
    assert!(
        harness.is_open(other.tab).await?,
        "the other task stays open"
    );
    assert!(harness.is_open(owner.tab).await?, "a user's tab stays open");
    assert_eq!(
        harness.tabs.owner(first.tab),
        None,
        "ownership ends with the tab"
    );
    let absent = failure(
        harness
            .submit(&later, "neo.task", json!({ "id": one["id"] }))
            .await,
    )?;
    assert_eq!(absent.error.code(), ErrorCode::TASK_NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn a_partial_task_close_keeps_refused_tabs_and_closes_the_rest() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let task = value(harness.submit(&run, "neo.createTask", json!({})).await)?;
    let held = page_of(&value(
        harness
            .submit(&run, "task.newPage", json!({ "task": task["id"] }))
            .await,
    )?)?;
    let free = page_of(&value(
        harness
            .submit(&run, "task.newPage", json!({ "task": task["id"] }))
            .await,
    )?)?;
    let _lock = harness.lock(held.tab, Reads::Allow).await?;
    let refused = failure(
        harness
            .submit(&run, "task.close", json!({ "task": task["id"] }))
            .await,
    )?;
    assert_eq!(refused.error.code(), ErrorCode::OWNER_HAS_CONTROL);
    assert!(harness.is_open(held.tab).await?);
    assert!(
        !harness.is_open(free.tab).await?,
        "one refusal does not abandon the remaining closes"
    );
    let fetched = value(
        harness
            .submit(&run, "neo.task", json!({ "id": task["id"] }))
            .await,
    )?;
    assert_eq!(fetched["pages"], json!([held]));
    Ok(())
}

#[tokio::test]
async fn a_picked_task_reuses_its_last_used_tab_then_newest_then_opens_one()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (creator, _creator_guard) = harness.run()?;
    let task = value(harness.submit(&creator, "neo.createTask", json!({})).await)?;
    let mut pages = Vec::new();
    for _ in 0..3 {
        pages.push(page_of(&value(
            harness.submit(&creator, "context.newPage", json!({})).await,
        )?)?);
    }
    // A read from a run that acts for no task, and one without a script call site (the
    // prelude's own) in a run that picked the task, leave the task on the tab it opened last.
    let (other, _other_guard) = harness.run()?;
    value(
        harness
            .submit_scripted(&other, "page.opener", json!({ "page": pages[0] }))
            .await,
    )?;
    let (unsited, _unsited_guard) = harness.run()?;
    let picked = harness
        .submit(&unsited, "neo.task", json!({ "id": task["id"] }))
        .await;
    assert_eq!(
        picked.picked_page.as_ref(),
        Some(&pages[2]),
        "a task-less run moves nothing"
    );
    value(picked)?;
    value(
        harness
            .submit(&unsited, "page.opener", json!({ "page": pages[0] }))
            .await,
    )?;
    // The task's own script acting on a tab, in a run that picked it, makes that tab its page.
    let (acting, _acting_guard) = harness.run()?;
    let picked = harness
        .submit(&acting, "neo.task", json!({ "id": task["id"] }))
        .await;
    assert_eq!(
        picked.picked_page.as_ref(),
        Some(&pages[2]),
        "nor an unsited read"
    );
    value(picked)?;
    value(
        harness
            .submit_scripted(&acting, "page.opener", json!({ "page": pages[0] }))
            .await,
    )?;
    let lazy = json!({ "page": { "$neo": "page", "tab": null } });
    for expected in [&pages[0], &pages[2], &pages[1]] {
        let (run, _guard) = harness.run()?;
        let picked = harness
            .submit(&run, "neo.task", json!({ "id": task["id"] }))
            .await;
        assert_eq!(
            picked.picked_page.as_ref(),
            Some(expected),
            "the pick binds `page` to the tab its first action would take"
        );
        value(picked)?;
        let reply = harness.submit(&run, "page.opener", lazy.clone()).await;
        assert_eq!(reply.default_page.as_ref(), Some(expected));
        assert_eq!(reply.pages[0].task_id.as_deref(), task["id"].as_str());
        value(reply)?;
        value(
            harness
                .submit(&run, "page.close", json!({ "page": expected }))
                .await,
        )?;
    }
    let (empty, _empty_guard) = harness.run()?;
    let picked = harness
        .submit(&empty, "neo.task", json!({ "id": task["id"] }))
        .await;
    assert_eq!(
        picked.picked_page, None,
        "a task with no open tab leaves `page` to open one at its first action"
    );
    value(picked)?;
    let reply = harness.submit(&empty, "page.opener", lazy).await;
    let fresh = reply.default_page.clone().ok_or("fresh default")?;
    assert!(!pages.contains(&fresh), "no closed tab is reused");
    assert_eq!(reply.pages[0].task_id.as_deref(), task["id"].as_str());
    value(reply)?;
    Ok(())
}

/// An earlier run's task and its one tab. A plain data bag.
struct TaskTab {
    /// `neo.createTask`'s value, `{ id, pages }`.
    task: Value,
    page: PageRef,
}

/// An earlier run's task with one tab showing `url`, as core's cache now lists it.
async fn task_on(harness: &Harness, url: &str) -> Result<TaskTab, Box<dyn Error>> {
    let (creator, _guard) = harness.run()?;
    let task = value(harness.submit(&creator, "neo.createTask", json!({})).await)?;
    let page = page_of(&value(
        harness.submit(&creator, "context.newPage", json!({})).await,
    )?)?;
    assert!(harness.fake.navigate_tab(page.tab, url, "Task"));
    harness.listed(page.tab).await?;
    Ok(TaskTab { task, page })
}

#[tokio::test]
async fn a_task_pick_binds_the_runs_page_and_its_fact_passes_the_read_rules()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let TaskTab { task, page: tab } = task_on(&harness, "https://task.example/held").await?;
    let opened = harness.fake.count("Browser.createTab");
    // Reads blocked: `page` still becomes the task's tab, with nothing of it to read.
    let lock = harness.lock(tab.tab, Reads::Block).await?;
    let (blocked, _blocked_guard) = harness.run()?;
    let picked = harness
        .submit(&blocked, "neo.task", json!({ "id": task["id"] }))
        .await;
    assert_eq!(picked.picked_page.as_ref(), Some(&tab));
    assert!(
        picked.pages.iter().all(|fact| fact.tab != tab.tab),
        "a tab held with blocked reads gives no fact: {:?}",
        picked.pages
    );
    value(picked)?;
    harness.locks.unlock_host(lock.id)?;
    // Reads allowed: the same pick carries the tab's URL and task.
    let lock = harness.lock(tab.tab, Reads::Allow).await?;
    let (allowed, _allowed_guard) = harness.run()?;
    let picked = harness
        .submit(&allowed, "neo.task", json!({ "id": task["id"] }))
        .await;
    assert_eq!(picked.picked_page.as_ref(), Some(&tab));
    let fact = picked
        .pages
        .iter()
        .find(|fact| fact.tab == tab.tab)
        .ok_or("the task tab's fact")?;
    assert_eq!(
        (fact.url.as_deref(), fact.task_id.as_deref(), fact.closed),
        (
            Some("https://task.example/held"),
            task["id"].as_str(),
            false
        )
    );
    value(picked)?;
    harness.locks.unlock_host(lock.id)?;
    assert_eq!(
        harness.fake.count("Browser.createTab"),
        opened,
        "a pick opens nothing"
    );
    Ok(())
}

#[tokio::test]
async fn the_first_tab_a_runs_page_gets_holds_for_the_run() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let TaskTab {
        task: first,
        page: first_tab,
    } = task_on(&harness, "https://first.example/").await?;
    let second = task_on(&harness, "https://second.example/").await?.task;
    let lazy = json!({ "page": { "$neo": "page", "tab": null } });
    let (run, _guard) = harness.run()?;
    let empty = value(harness.submit(&run, "neo.createTask", json!({})).await)?;
    // A task with no open tab binds nothing, so the next pick still can.
    let none = harness
        .submit(&run, "neo.task", json!({ "id": empty["id"] }))
        .await;
    assert_eq!(none.picked_page, None);
    value(none)?;
    let bound = harness
        .submit(&run, "neo.task", json!({ "id": first["id"] }))
        .await;
    assert_eq!(bound.picked_page.as_ref(), Some(&first_tab));
    value(bound)?;
    // Later picks steer new tabs, never the bound `page`.
    let later = harness
        .submit(&run, "neo.task", json!({ "id": second["id"] }))
        .await;
    assert_eq!(later.picked_page, None);
    value(later)?;
    let created = value(harness.submit(&run, "neo.createTask", json!({})).await)?;
    let acted = harness.submit(&run, "page.opener", lazy.clone()).await;
    assert_eq!(acted.default_page.as_ref(), Some(&first_tab));
    value(acted)?;
    let opened = harness.submit(&run, "context.newPage", json!({})).await;
    assert_eq!(opened.pages[0].task_id.as_deref(), created["id"].as_str());
    value(opened)?;
    // A page its first action resolved stays, too.
    let (resolved, _resolved_guard) = harness.run()?;
    let fresh = harness.submit(&resolved, "page.opener", lazy.clone()).await;
    let fresh_tab = fresh.default_page.clone().ok_or("a fresh tab")?;
    value(fresh)?;
    let picked = harness
        .submit(&resolved, "neo.task", json!({ "id": first["id"] }))
        .await;
    assert_eq!(picked.picked_page, None);
    value(picked)?;
    let again = harness.submit(&resolved, "page.opener", lazy).await;
    assert_eq!(again.default_page, Some(fresh_tab));
    value(again)?;
    Ok(())
}

/// The viewer's report (night 4e58b2527, VIEWER.md 4.5): after `await neo.task(id)` the built-in
/// `page` read `about:blank` until its first awaited call. Playwright's getters are right as soon
/// as a script holds a page.
#[test]
fn a_task_picks_page_getters_are_its_tabs_before_any_action() -> Result<(), TestError> {
    on_neo_runtime(async {
        let harness = Harness::new().await.map_err(|error| error.to_string())?;
        let TaskTab { task, page: tab } = task_on(&harness, "https://task.example/at-once")
            .await
            .map_err(|error| error.to_string())?;
        let id = task["id"].as_str().ok_or("a task id")?;
        let (run, _guard) = harness.run().map_err(|error| error.to_string())?;
        let script = Script {
            code: format!(
                "const task = await neo.task({id:?});
                 return {{ url: page.url(), frame: page.mainFrame().url(), tab: page.tabId,
                   task: page.taskId, closed: page.isClosed(),
                   listed: context.pages().includes(page), inTask: task.pages().includes(page) }};"
            ),
            page: None,
            pages: Vec::new(),
            observed: None,
        };
        match run_script(&run, &harness.dispatcher, script).await.outcome {
            ScriptOutcome::Returned(value) => assert_eq!(
                value,
                json!({
                    "url": "https://task.example/at-once", "frame": "https://task.example/at-once",
                    "tab": tab.tab.0, "task": id, "closed": false, "listed": true, "inTask": true
                })
            ),
            other => return Err(format!("expected the task tab's getters, got {other:?}").into()),
        }
        Ok(())
    })
}

#[tokio::test]
async fn the_latest_pick_owns_new_tabs_and_closing_it_clears_the_pick() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let first = value(harness.submit(&run, "neo.createTask", json!({})).await)?;
    let second = value(harness.submit(&run, "neo.createTask", json!({})).await)?;
    let opened = harness.submit(&run, "context.newPage", json!({})).await;
    assert_eq!(opened.pages[0].task_id.as_deref(), second["id"].as_str());
    value(opened)?;
    value(
        harness
            .submit(&run, "task.close", json!({ "task": second["id"] }))
            .await,
    )?;
    let plain = harness.submit(&run, "context.newPage", json!({})).await;
    assert_eq!(
        plain.pages[0].task_id, None,
        "close does not restore an older script pick"
    );
    value(plain)?;
    value(
        harness
            .submit(&run, "neo.task", json!({ "id": first["id"] }))
            .await,
    )?;
    let picked = harness.submit(&run, "context.newPage", json!({})).await;
    assert_eq!(picked.pages[0].task_id.as_deref(), first["id"].as_str());
    value(picked)?;
    Ok(())
}

#[tokio::test]
async fn an_already_resolved_default_stays_unowned_after_a_task_pick() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let opened = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    let lazy = json!({ "page": { "$neo": "page", "tab": null } });
    let default = harness.submit(&run, "page.opener", lazy.clone()).await;
    let default_page = default.default_page.clone().ok_or("default")?;
    value(default)?;
    assert_ne!(opened, default_page);
    assert_eq!(harness.tabs.owner(opened.tab), None);
    assert_eq!(harness.tabs.owner(default_page.tab), None);
    let task = value(harness.submit(&run, "neo.createTask", json!({})).await)?;
    let after = harness.submit(&run, "page.opener", lazy).await;
    assert_eq!(after.default_page, Some(default_page));
    assert_eq!(after.pages[0].task_id, None);
    value(after)?;
    let owned = harness.submit(&run, "context.newPage", json!({})).await;
    assert_eq!(owned.pages[0].task_id.as_deref(), task["id"].as_str());
    value(owned)?;
    Ok(())
}

#[tokio::test]
async fn a_task_fetch_never_lists_a_tab_this_run_closed() -> Result<(), Box<dyn Error>> {
    // Chromium can acknowledge a close while its next listing still includes that tab.
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let task = value(harness.submit(&run, "neo.createTask", json!({})).await)?;
    let closed = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    let open = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    harness.fake.linger_closed_tabs_for(2);
    value(
        harness
            .submit(&run, "page.close", json!({ "page": closed }))
            .await,
    )?;
    let listed = harness
        .submit(&run, "neo.task", json!({ "id": task["id"] }))
        .await;
    assert!(
        listed.pages.iter().all(|fact| fact.tab != closed.tab),
        "no fact revives the closed tab"
    );
    assert_eq!(value(listed)?["pages"], json!([open]));
    Ok(())
}

#[tokio::test]
async fn a_task_closed_by_another_run_leaves_context_and_default_opens_unowned()
-> Result<(), Box<dyn Error>> {
    for lazy in [false, true] {
        let harness = Harness::new().await?;
        let (run, _guard) = harness.run()?;
        let task = value(harness.submit(&run, "neo.createTask", json!({})).await)?;
        let (closer, _closer_guard) = harness.run()?;
        value(
            harness
                .submit(&closer, "task.close", json!({ "task": task["id"] }))
                .await,
        )?;
        let reply = if lazy {
            harness
                .submit(
                    &run,
                    "page.opener",
                    json!({ "page": { "$neo": "page", "tab": null } }),
                )
                .await
        } else {
            harness.submit(&run, "context.newPage", json!({})).await
        };
        assert_eq!(reply.pages[0].task_id, None);
        value(reply)?;
        assert_eq!(harness.fake.count("Browser.createTab"), 1);
    }
    Ok(())
}

#[tokio::test]
async fn a_task_closing_during_an_open_leaves_the_new_tab_unowned() -> Result<(), Box<dyn Error>> {
    for lazy in [false, true] {
        let harness = Harness::new().await?;
        let (run, _guard) = harness.run()?;
        let task = value(harness.submit(&run, "neo.createTask", json!({})).await)?;
        harness.fake.hold_next("Browser.createTab");
        let (name, input) = if lazy {
            (
                "page.opener",
                json!({ "page": { "$neo": "page", "tab": null } }),
            )
        } else {
            ("context.newPage", json!({}))
        };
        let opening = harness
            .dispatcher
            .submit(&run, Call::script(name.to_owned(), input, None));
        within(async {
            while harness.fake.count("Browser.createTab") == 0 {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?;
        let (closer, _closer_guard) = harness.run()?;
        value(
            harness
                .submit(&closer, "task.close", json!({ "task": task["id"] }))
                .await,
        )?;
        harness.fake.release_held();
        let reply = within(opening.wait()).await?;
        assert_eq!(reply.pages[0].task_id, None);
        value(reply)?;
        assert_eq!(
            harness.fake.tabs().len(),
            1,
            "the successful open survives its former task"
        );
    }
    Ok(())
}

#[tokio::test]
async fn missing_tasks_fail_before_any_browser_write() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    for (name, input) in [
        ("neo.task", json!({ "id": "missing" })),
        ("task.newPage", json!({ "task": "missing" })),
        ("task.close", json!({ "task": "missing" })),
    ] {
        let refused = failure(harness.submit(&run, name, input).await)?;
        assert_eq!(refused.error.owner(), ErrorOwner::Script);
        assert_eq!(refused.error.code(), ErrorCode::TASK_NOT_FOUND);
        assert_eq!(refused.delivery, Delivery::NotSent);
        assert!(refused.error.message().contains("context.pages()"));
    }
    assert!(harness.fake.tabs().is_empty());
    Ok(())
}

#[tokio::test]
async fn popups_inherit_their_openers_task_without_a_script_listener() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let task = value(harness.submit(&run, "neo.createTask", json!({})).await)?;
    let opener = page_of(&value(
        harness
            .submit(&run, "task.newPage", json!({ "task": task["id"] }))
            .await,
    )?)?;
    let user = harness.owner_tab("https://owner.example/").await?;
    let popup = harness
        .fake
        .open_popup(opener.tab, "https://popup.example/");
    let user_popup = harness
        .fake
        .open_popup(user.tab, "https://user-popup.example/");
    within(async {
        while harness.tabs.owner(popup).is_none() {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let fetched = harness
        .submit(&run, "neo.task", json!({ "id": task["id"] }))
        .await;
    assert_eq!(
        value(fetched)?["pages"],
        json!([opener, PageRef { tab: popup }])
    );
    assert_eq!(
        harness.tabs.owner(user_popup),
        None,
        "a user's popup has no task"
    );
    value(
        harness
            .submit(&run, "task.close", json!({ "task": task["id"] }))
            .await,
    )?;
    assert!(
        !harness.is_open(popup).await?,
        "task.close includes the inherited popup"
    );
    assert!(harness.is_open(user_popup).await?);
    Ok(())
}

#[tokio::test]
async fn a_task_close_waits_for_its_inflight_open() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let task = value(harness.submit(&run, "neo.createTask", json!({})).await)?;
    harness.fake.hold_next("Browser.createTab");
    let opening = harness.dispatcher.submit(
        &run,
        Call::script(
            "task.newPage".to_owned(),
            json!({ "task": task["id"] }),
            None,
        ),
    );
    within(async {
        while harness.fake.count("Browser.createTab") == 0 {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let closing = harness.dispatcher.submit(
        &run,
        Call::script("task.close".to_owned(), json!({ "task": task["id"] }), None),
    );
    harness.fake.release_held();
    let page = page_of(&value(within(opening.wait()).await?)?)?;
    let closed = value(within(closing.wait()).await?)?;
    assert_eq!(closed["closed"], json!([page.tab]));
    assert!(!harness.is_open(page.tab).await?);
    let absent = failure(
        harness
            .submit(&run, "neo.task", json!({ "id": task["id"] }))
            .await,
    )?;
    assert_eq!(absent.error.code(), ErrorCode::TASK_NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn task_inputs_reject_typos_and_invalid_names() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    for input in [
        json!({ "name": null }),
        json!({ "name": "x".repeat(201) }),
        json!({ "typo": true }),
    ] {
        let refused = failure(harness.submit(&run, "neo.createTask", input).await)?;
        assert_eq!(refused.error.code(), ErrorCode::INVALID_INPUT);
    }
    let empty = value(
        harness
            .submit(&run, "neo.createTask", json!({ "name": "  " }))
            .await,
    )?;
    assert!(empty["id"].as_str().ok_or("id")?.starts_with("t-"));
    let unicode = value(
        harness
            .submit(&run, "neo.createTask", json!({ "name": "é".repeat(200) }))
            .await,
    )?;
    assert!(
        unicode["id"].as_str().ok_or("id")?.starts_with("t-"),
        "the limit counts characters and the slug stays ASCII"
    );
    Ok(())
}

struct TaskPopupEvent;

impl Method for TaskPopupEvent {
    type Input = Value;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("events.next", Effect::Read)
    }
    fn target(_input: &Value) -> Option<&PageTarget> {
        None
    }
    fn call(
        &self,
        op: &OpCtx<Self>,
        input: Value,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(EventRelease::Batch.release(op, input))
    }
}

#[tokio::test]
async fn a_popup_event_assigns_ownership_before_its_first_fact() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let task = value(harness.submit(&run, "neo.createTask", json!({})).await)?;
    let opener = page_of(&value(
        harness
            .submit(&run, "task.newPage", json!({ "task": task["id"] }))
            .await,
    )?)?;
    // No openerId on the fake target: only the delivered popup event can assign this tab.
    let popup = harness.owner_tab("https://event-popup.example/").await?;
    assert_eq!(harness.tabs.owner(popup.tab), None);
    let door = harness.dispatcher_with(Vec::new(), |registry| registry.define(TaskPopupEvent))?;
    let reply = door.submit(&run, Call::script("events.next".to_owned(), json!({
        "events": [{ "seq": 1, "event": "popup", "page": opener,
            "popup": { "page": popup, "url": "https://event-popup.example/" } }], "lost": 0,
    }), None)).wait().await;
    assert_eq!(
        reply
            .pages
            .iter()
            .find(|fact| fact.tab == popup.tab)
            .and_then(|fact| fact.task_id.as_deref()),
        task["id"].as_str()
    );
    value(reply)?;
    Ok(())
}

#[test]
fn task_pages_stay_closed_after_a_stale_browser_listing() -> Result<(), TestError> {
    on_neo_runtime(async {
        let harness = Harness::new().await.map_err(|error| error.to_string())?;
        // Chromium can acknowledge a close while its next listing still includes that tab.
        harness.fake.linger_closed_tabs_for(2);
        let (run, _guard) = harness.run().map_err(|error| error.to_string())?;
        let script = Script {
            code: r"
                const task = await neo.createTask();
                const first = await context.newPage();
                const second = await context.newPage();
                await first.close();
                const fetched = await neo.task(task.id);
                const remaining = fetched.pages().map(p => p.tabId);
                await task.close();
                return { remaining, expected: [second.tabId], after: task.pages(),
                    closed: [first.isClosed(), second.isClosed()],
                    plain: (await context.newPage()).taskId };
            "
            .to_owned(),
            page: None,
            pages: Vec::new(),
            observed: None,
        };
        match run_script(&run, &harness.dispatcher, script).await.outcome {
            ScriptOutcome::Returned(value) => {
                assert_eq!(value["remaining"], value["expected"]);
                assert_eq!(value["after"], json!([]));
                assert_eq!(value["closed"], json!([true, true]));
                assert_eq!(value["plain"], Value::Null);
            }
            other => return Err(format!("expected closed pages, got {other:?}").into()),
        }
        Ok(())
    })
}

#[tokio::test]
async fn an_evicted_pick_and_a_full_task_book_never_block_plain_opens() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let picked = value(harness.submit(&run, "neo.createTask", json!({})).await)?;
    // Keep the remaining empty tasks pinned so this pick is the only eviction candidate.
    let mut pinned = Vec::new();
    for _ in 0..4096 {
        pinned.push(harness.tabs.create_task(None)?);
    }
    let absent = failure(
        harness
            .submit(&run, "neo.task", json!({ "id": picked["id"] }))
            .await,
    )?;
    assert_eq!(absent.error.code(), ErrorCode::TASK_NOT_FOUND);
    let full = failure(harness.submit(&run, "neo.createTask", json!({})).await)?;
    assert_eq!(full.error.code(), ErrorCode::TASK_LIMIT_REACHED);
    for (name, input) in [
        ("context.newPage", json!({})),
        (
            "page.opener",
            json!({ "page": { "$neo": "page", "tab": null } }),
        ),
    ] {
        let reply = harness.submit(&run, name, input).await;
        assert_eq!(reply.pages[0].task_id, None);
        value(reply)?;
    }
    assert_eq!(harness.fake.count("Browser.createTab"), 2);
    Ok(())
}

/// The op `$eval` (`page.$eval`, `frame.$eval`, `elementHandle.$eval`) sends, and its input on
/// `page`: a script that names the first heading, matched now.
const DOLLAR_EVAL: &str = "page.evalOnSelector";

fn dollar_eval(page: &PageRef) -> Value {
    json!({
        "target": {
            "$neo": "locator",
            "page": handle(page),
            "frame": null,
            "chunks": [{ "parts": [{ "name": "internal:role", "body": "heading", "source": "heading" }] }],
            "display": "getByRole('heading')",
        },
        "strict": "firstMatch",
        "selector": "h1",
        "expression": "(el) => el.textContent",
        "isFunction": true,
        "arg": { "value": "{\"v\":\"undefined\"}", "handles": [] },
    })
}

/// How the door answered one call: the value, or who failed, with what code and delivery, and the
/// message with the op's own name and the lock's random id left out.
fn door_outcome(reply: OpReply) -> String {
    match reply.outcome {
        Ok(value) => format!("value {value}"),
        Err(failure) => {
            let message = failure.error.message();
            let message = message.strip_prefix(DOLLAR_EVAL).unwrap_or(message);
            let message = message.split(" (lock ").next().unwrap_or(message);
            format!(
                "{:?} {:?} {:?} retryable={}: {message}",
                failure.error.owner(),
                failure.error.code(),
                failure.delivery,
                failure.retryable,
            )
        }
    }
}

/// `$eval` meets the door as the main-world script it is: with no lock the door lets it through
/// (and the fake browser's reply ends it in the crate), and an owner's lock refuses it with
/// nothing sent, whether it lets reads through or blocks them.
#[tokio::test]
async fn tests_that_dollar_eval_meets_the_door_as_a_main_world_script() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://bank.example/").await?;
    let (run, _guard) = harness.run()?;
    let mut seen = Vec::new();
    for reads in [None, Some(Reads::Allow), Some(Reads::Block)] {
        let lock = match reads {
            Some(reads) => Some(harness.lock(owners.tab, reads).await?),
            None => None,
        };
        let reply = harness
            .submit(&run, DOLLAR_EVAL, dollar_eval(&owners))
            .await;
        seen.push(format!("{reads:?}: {}", door_outcome(reply)));
        if let Some(lock) = lock {
            harness.locks.unlock_host(lock.id)?;
        }
    }
    let owner_has_control = "Host ErrorCode(\"owner_has_control\") NotSent retryable=true: The \
                             owner has control of this tab. Nothing was done. Wait for them to give \
                             control back.";
    assert_eq!(
        seen,
        [
            "None: Neo ErrorCode(\"internal_error\") NotSent retryable=true: : missing field \
             `executionContextId`"
                .to_owned(),
            format!("Some(Allow): {owner_has_control}"),
            format!("Some(Block): {owner_has_control}"),
        ]
    );
    Ok(())
}

/// What a running `$eval` read reaches the script as a running `locator.evaluate`'s would once the
/// owner takes the tab: the door decides by the op's main-world mark, and both ops derive the same
/// one. A lock that blocks reads withholds the value and the page's error alike; one that lets
/// reads through releases the value.
#[tokio::test]
async fn tests_that_a_takeover_while_dollar_eval_runs_decides_as_for_locator_evaluate()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://bank.example/").await?;
    let page_script = |name: &str| {
        Op::from_name(name)
            .map(|op| {
                OpMarks::of(&op.spec(), "the run description's line")
                    .spec
                    .page_script
            })
            .ok_or(format!("{name} is an op"))
    };
    let dollar_eval = page_script("page.evalOnSelector")?;
    assert_eq!(dollar_eval, page_script("locator.evaluate")?);
    for answers in [Answers::Read, Answers::Throws] {
        let (operation, reply) = answer_after_a_takeover(
            &harness,
            Takeover::of(&owners, Reads::Block),
            dollar_eval,
            answers,
        )
        .await?;
        assert_withheld_by_the_lock(&harness, operation, reply, answers).await?;
    }
    let (_, reply) = answer_after_a_takeover(
        &harness,
        Takeover::of(&owners, Reads::Allow),
        dollar_eval,
        Answers::Read,
    )
    .await?;
    assert_eq!(value(reply)?, json!({ "read": OWNERS_TEXT }));
    Ok(())
}

/// Neo's blank tab, armed on this harness's facade as the sidecar arms it.
fn blank_tab(harness: &Harness) -> Arc<NeoTab> {
    let blank = NeoTab::new(
        Arc::clone(&harness.link),
        Arc::clone(&harness.tabs),
        harness.store.clone(),
    );
    harness.playwright.keep(Arc::clone(&blank));
    blank
}

/// The tabs `context.pages()` lists, by id.
fn listed_tabs(pages: &Value) -> Vec<i64> {
    pages["pages"]
        .as_array()
        .map(|pages| {
            pages
                .iter()
                .filter_map(|entry| entry["page"]["tab"].as_i64())
                .collect()
        })
        .unwrap_or_default()
}

/// The owner, 2026-11-12: "keep a blank tab open always". Neo opens one tab of its own, keeps the
/// same one, and no script sees or reaches it: `context.pages()` leaves it out, a handle built for
/// it is refused before anything is sent, and the janitor never counts it as the agent's.
#[tokio::test]
async fn neo_keeps_one_blank_tab_that_no_script_sees_or_reaches() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let blank = blank_tab(&harness);
    let tab = within(blank.ensure(Adopt::Allowed))
        .await?
        .ok_or("Neo's tab opened")?;
    assert_eq!(
        harness.fake.count("Browser.createTab"),
        1,
        "nothing to adopt: opened"
    );
    assert_eq!(
        within(blank.ensure(Adopt::Allowed)).await?,
        Some(tab),
        "the same tab again: never two"
    );
    assert_eq!(harness.fake.count("Browser.createTab"), 1);
    assert!(harness.tabs.is_neo(tab));
    assert!(
        !harness.tabs.agent_tabs().contains(&tab),
        "not the agent's: the janitor never closes it"
    );

    let (run, guard) = harness.run()?;
    let script = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    let pages = value(harness.submit(&run, "context.pages", json!({})).await)?;
    assert_eq!(
        listed_tabs(&pages),
        vec![script.tab.0],
        "context.pages() lists the script's tab, not Neo's"
    );
    let refused = failure(
        harness
            .submit(
                &run,
                "page.opener",
                json!({ "page": { "$neo": "page", "tab": tab.0 } }),
            )
            .await,
    )?;
    assert_eq!(
        (
            refused.error.owner(),
            refused.error.code(),
            refused.delivery
        ),
        (
            ErrorOwner::Script,
            ErrorCode::INVALID_INPUT,
            Delivery::NotSent
        ),
        "a handle a script built for Neo's tab is refused: {}",
        refused.error
    );
    Harness::finish(guard);
    Ok(())
}

/// At a watch start Neo adopts a blank tab no one owns (the launch tab, or its own tab restored
/// by the browser) rather than open another: not a task's, as the trace remembers it after a
/// server restart, and never while the server runs.
#[tokio::test]
async fn neo_adopts_a_free_blank_tab_only_at_a_watch_start() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let tasked = harness.fake.open_tab("about:blank", "");
    let free = harness.fake.open_tab("chrome://newtab/", "New Tab");
    // A run of task `mail` touched the first blank tab before this server started.
    let run = RunId::new();
    harness.sink.run_started(RunStarted {
        id: run,
        code: String::new(),
        request_id: None,
        task_id: Some(TaskId::parse("mail")?),
        host_data: None,
        at: EpochMillis::now(),
    });
    harness.sink.op_started(OperationStarted {
        site: None,
        id: OperationId::new(run, 1),
        parent: None,
        name: "page.title".into(),
        kind: OperationKind::Builtin,
        effect: Effect::Read,
        tab: Some(tasked),
        args: Value::Null,
        at: EpochMillis::now(),
    });
    harness.sink.run_ended(RunEnded {
        id: run,
        status: RunStatus::Ok,
        error: None,
        result: None,
        replay: None,
        at: EpochMillis::now(),
    });
    harness.sink.flush().await?;

    let blank = blank_tab(&harness);
    let opened = within(blank.ensure(Adopt::Never))
        .await?
        .ok_or("Neo's tab")?;
    assert!(
        opened != free && opened != tasked && harness.fake.count("Browser.createTab") == 1,
        "while the server runs it never adopts: a script may be about to use that tab"
    );
    // The browser restarts without it, say: the next watch start may adopt.
    assert!(harness.fake.close_tab(opened), "Neo's tab goes");
    assert_eq!(
        within(blank.ensure(Adopt::Allowed)).await?,
        Some(free),
        "at a watch start the free blank tab is adopted, not the task's"
    );
    assert_eq!(
        harness.fake.count("Browser.createTab"),
        1,
        "adopted, not opened"
    );
    assert!(!harness.tabs.is_neo(tasked), "a task's tab is never Neo's");
    Ok(())
}

/// When Neo's tab closes while the server runs, it opens another at once.
#[tokio::test]
async fn neo_opens_another_blank_tab_when_its_own_closes() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let blank = blank_tab(&harness);
    let shutdown = CancellationToken::new();
    let following = tokio::spawn(Arc::clone(&blank).follow(shutdown.clone()));
    let first = within(async {
        loop {
            if let Some(tab) = blank.current() {
                return tab;
            }
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    assert!(harness.fake.close_tab(first), "the owner closes Neo's tab");
    let second = within(async {
        loop {
            if let Some(tab) = blank.current().filter(|tab| *tab != first) {
                return tab;
            }
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    assert!(harness.tabs.is_neo(second) && !harness.tabs.is_neo(first));
    assert_eq!(
        harness.fake.count("Browser.createTab"),
        2,
        "one at a time, never two"
    );
    shutdown.cancel();
    within(following).await??;
    Ok(())
}

/// "Open it first, then close": a script closing its last tab leaves the browser Neo's blank
/// tab, and the janitor may close an idle agent tab that is the only other tab.
#[tokio::test]
async fn a_close_makes_sure_of_neos_tab_first() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let blank = blank_tab(&harness);
    let (run, guard) = harness.run()?;
    let only = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    value(
        harness
            .submit(&run, "page.close", json!({ "page": handle(&only) }))
            .await,
    )?;
    let calls: Vec<String> = harness
        .fake
        .calls()
        .into_iter()
        .map(|call| call.method)
        .filter(|method| method == "Browser.createTab" || method == "Target.closeTarget")
        .collect();
    assert_eq!(
        calls,
        vec![
            "Browser.createTab",
            "Browser.createTab",
            "Target.closeTarget"
        ],
        "the script's tab, then Neo's tab, then the close"
    );
    let neo = blank.current().ok_or("Neo's tab")?;
    assert!(harness.is_open(neo).await?, "the browser keeps Neo's tab");
    Harness::finish(guard);

    // The janitor closes an idle agent tab even when it is the only other tab.
    let (run, guard) = harness.run()?;
    let idle = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    Harness::finish(guard);
    let later = Instant::now() + harness.settings.tab_idle_close();
    within(harness.janitor().sweep(later))
        .await?
        .ok_or("the sweep ran")?;
    assert!(
        !harness.is_open(idle.tab).await?,
        "closed after the idle time"
    );
    assert!(harness.is_open(neo).await?, "Neo's tab stays");
    Ok(())
}

/// A script waiting for a new page never gets Neo's blank tab: Neo opens it while the wait
/// watches, so its open is among the events the wait reads, and the reader leaves it out. The
/// wait returns the next tab instead.
#[tokio::test]
async fn a_script_waiting_for_a_page_never_gets_neos_tab() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let blank = blank_tab(&harness);
    let (run, guard) = harness.run()?;
    let wait = json!({
        "call": { "api": "browserContext.waitForEvent", "timeoutMs": 10000 },
        "page": null,
        "event": "page",
        "since": 0,
    });
    let pending = harness
        .dispatcher
        .submit(&run, Call::script("events.wait".to_owned(), wait, None));
    within(async {
        while harness.fake.count("Target.getTargets") == 0 {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    // The owner closed Neo's tab, say: Neo opens another while the script waits.
    let neo = within(blank.ensure(Adopt::Never))
        .await?
        .ok_or("Neo's tab")?;
    let owners = harness.fake.open_tab("https://example.com/", "Example");
    let reply = value(within(pending.wait()).await?)?;
    let text = reply.to_string();
    assert!(
        text.contains(&format!("\"tab\":{}", owners.0))
            && !text.contains(&format!("\"tab\":{}", neo.0)),
        "the wait got the owner's new tab, never Neo's: {text}"
    );
    Harness::finish(guard);
    Ok(())
}

/// In a fresh browser Neo's tab is often the one the owner sees. Once they browse in it, it is
/// theirs: a new run lists their page at once, Neo keeps one new blank tab instead (never two),
/// and the janitor never closes the owner's tab.
#[tokio::test]
async fn neo_hands_its_tab_back_once_the_owner_browses_in_it() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let launch = harness.fake.open_tab("about:blank", "");
    let blank = blank_tab(&harness);
    assert_eq!(
        within(blank.ensure(Adopt::Allowed)).await?,
        Some(launch),
        "the launch tab serves"
    );
    assert!(
        harness
            .fake
            .navigate_tab(launch, "https://example.com/", "Example")
    );

    let (run, guard) = harness.run()?;
    let pages = value(harness.submit(&run, "context.pages", json!({})).await)?;
    assert_eq!(
        listed_tabs(&pages),
        vec![launch.0],
        "the run lists the owner's page at once, and not Neo's new tab"
    );
    Harness::finish(guard);
    let fresh = blank.current().ok_or("Neo's new tab")?;
    assert!(
        fresh != launch && harness.tabs.is_neo(fresh) && !harness.tabs.is_neo(launch),
        "the new blank tab is Neo's, the owner's tab is not"
    );
    assert_eq!(
        harness.fake.count("Browser.createTab"),
        1,
        "one new tab: the browser has the owner's and Neo's, never two of Neo's"
    );

    let later = Instant::now() + harness.settings.tab_idle_close();
    let _swept = within(harness.janitor().sweep(later)).await?;
    assert!(
        harness.is_open(launch).await? && harness.is_open(fresh).await?,
        "the janitor closes neither the owner's tab nor Neo's"
    );
    Ok(())
}

/// A script waiting for the owner's next page hears of a tab Neo hands back, as it hears of a
/// tab the owner opens.
#[tokio::test]
async fn a_handed_back_tab_reaches_a_waiting_script_as_a_new_page() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let blank = blank_tab(&harness);
    let (run, guard) = harness.run()?;
    // The engine's watcher starts with the run's first op. The fake answers no
    // `Target.getTargets`, so the launch tab opens after it, as Chromium would report it.
    value(harness.submit(&run, "context.pages", json!({})).await)?;
    within(async {
        while harness.fake.count("Target.getTargets") == 0 {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let playwright = harness.playwright.get().await;
    let head = playwright.event_head();
    let launch = harness.fake.open_tab("about:blank", "");
    // Its open is in the log (about a second later: it announced no URL) before Neo takes it, so
    // only the hand-back can bring it to the script.
    within(async {
        while playwright.event_head() == head {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    // A watch start adopts it.
    assert_eq!(within(blank.ensure(Adopt::Allowed)).await?, Some(launch));
    let listings = harness.fake.count("Browser.getTabs");
    let wait = json!({
        "call": { "api": "browserContext.waitForEvent", "timeoutMs": 10000 },
        "page": null,
        "event": "page",
        "since": 0,
    });
    let pending = harness
        .dispatcher
        .submit(&run, Call::script("events.wait".to_owned(), wait, None));
    // The wait reads the tab's own open first, while the tab is Neo's: it lists the tabs, finds
    // it reserved, and goes on waiting. Only then does the owner browse in it.
    within(async {
        while harness.fake.count("Browser.getTabs") == listings {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    assert!(
        harness
            .fake
            .navigate_tab(launch, "https://example.com/", "Example")
    );
    // Neo's 30-second check, run now.
    let fresh = within(blank.ensure(Adopt::Never))
        .await?
        .ok_or("Neo's new tab")?;
    let reply = value(within(pending.wait()).await?)?;
    let text = reply.to_string();
    assert!(
        text.contains(&format!("\"tab\":{}", launch.0))
            && !text.contains(&format!("\"tab\":{}", fresh.0)),
        "the wait got the owner's page, never Neo's new tab: {text}"
    );
    Harness::finish(guard);
    Ok(())
}

/// `page.close({ runBeforeUnload: true })` closes the tab too, so it makes sure of Neo's tab
/// first, as `page.close` does.
#[tokio::test]
async fn a_close_that_runs_beforeunload_makes_sure_of_neos_tab_first() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let blank = blank_tab(&harness);
    let (run, guard) = harness.run()?;
    let only = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    value(
        harness
            .submit(
                &run,
                "page.runBeforeUnload",
                json!({ "page": handle(&only) }),
            )
            .await,
    )?;
    let calls: Vec<String> = harness
        .fake
        .calls()
        .into_iter()
        .map(|call| call.method)
        .filter(|method| method == "Browser.createTab" || method == "Page.close")
        .collect();
    assert_eq!(
        calls,
        vec!["Browser.createTab", "Browser.createTab", "Page.close"],
        "the script's tab, then Neo's tab, then the close"
    );
    assert!(blank.current().is_some_and(|tab| harness.tabs.is_neo(tab)));
    Harness::finish(guard);
    Ok(())
}

/// When Neo cannot open its blank tab, a close that may be the browser's last is refused before
/// anything is sent; with another tab open, it goes ahead.
#[tokio::test]
async fn a_close_neo_cannot_make_safe_is_refused() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let _blank = blank_tab(&harness);
    let (run, guard) = harness.run()?;
    let only = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    // From now on the browser's answer to an open names no tab.
    harness.fake.answer("Browser.createTab", json!({}));
    let refused = failure(
        harness
            .submit(&run, "page.close", json!({ "page": handle(&only) }))
            .await,
    )?;
    assert_eq!(
        (
            refused.error.owner(),
            refused.error.code(),
            refused.delivery
        ),
        (ErrorOwner::Neo, ErrorCode::CDP_ERROR, Delivery::NotSent),
        "{}",
        refused.error
    );
    assert!(
        harness.is_open(only.tab).await?,
        "the browser keeps its last tab"
    );

    let owners = harness.fake.open_tab("https://example.com/", "Example");
    value(
        harness
            .submit(&run, "page.close", json!({ "page": handle(&only) }))
            .await,
    )?;
    assert!(
        !harness.is_open(only.tab).await? && harness.is_open(owners).await?,
        "with the owner's tab open, the close cannot quit the browser"
    );
    Harness::finish(guard);
    Ok(())
}

/// An open the browser took outlives a caller that stopped waiting: the tab is still marked
/// Neo's, and the next ensure finds it instead of opening a second one.
#[tokio::test]
async fn an_open_outlives_a_caller_that_stops_waiting() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let blank = blank_tab(&harness);
    harness.fake.hold_next("Browser.createTab");
    let caller = tokio::spawn({
        let blank = Arc::clone(&blank);
        async move { blank.ensure(Adopt::Never).await }
    });
    within(async {
        while harness.fake.count("Browser.createTab") == 0 {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    caller.abort();
    harness.fake.release_held();
    let tab = within(blank.ensure(Adopt::Never))
        .await?
        .ok_or("Neo's tab")?;
    assert!(
        harness.tabs.is_neo(tab),
        "the open its caller left was still marked"
    );
    assert_eq!(
        harness.fake.count("Browser.createTab"),
        1,
        "never a second tab"
    );
    Ok(())
}

/// Ruling R45, the start's fail-closed side: a record that names a tab, and a browser whose tab
/// list cannot be had at start, so which tabs the record names cannot be told. Every tab is
/// then treated as having taken a fill, so Neo's camera and a script's screenshot refuse them.
/// The control: the same start with the list answered marks no tab the record does not name.
#[tokio::test]
async fn a_start_that_cannot_list_the_tabs_treats_every_tab_as_filled() -> Result<(), Box<dyn Error>>
{
    for listed in [true, false] {
        let fake = FakeCdp::new();
        let tab = fake.open_tab("https://shop.test/", "Shop");
        let connect = Arc::clone(&fake);
        let link = BrowserLink::start_with(
            NativeCapabilities::default(),
            Arc::new(TabRegistry::new()),
            move || future::ready(connect.connect()),
        );
        within(async {
            while !link.is_ready() {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?;
        if !listed {
            // A reply without its tab list: the start cannot tell which tabs the record names.
            fake.answer("Browser.getTabs", json!({}));
        }
        let dir = tempfile::tempdir()?;
        // The record names a tab the browser no longer has.
        fs::write(dir.path().join("filled-tabs"), "\nGONE-TARGET\n")?;
        let filled = FilledTabs::open(dir.path()).await;
        let file_access = Arc::new(FileAccess::new(Vec::new, Vec::new()));
        let facade = PlaywrightFacade::start(
            Arc::clone(&link),
            false,
            Arc::default(),
            file_access,
            Some(filled),
        );
        let playwright = within(facade.get()).await?;
        assert_eq!(
            playwright.took_private_text(tab),
            !listed,
            "listed: {listed}: a tab the record does not name counts only when the list could \
             not be had"
        );
        assert!(
            !facade.withholds_picture(tab),
            "pictured under the default (ruling R58)"
        );
        facade.set_fill_pictures_never(true);
        assert_eq!(
            facade.withholds_picture(tab),
            !listed,
            "listed: {listed}: under `never`, withheld only when the list could not be had"
        );
    }
    Ok(())
}

/// A recorded script run whose host task is `task`.
fn run_for(harness: &Harness, task: &TaskId) -> Result<(Arc<RunCtx>, RunGuard), Box<dyn Error>> {
    let view = harness.link.view().ok_or("the link is up")?;
    let start = RunStart {
        id: RunId::new(),
        code: String::new(),
        request_id: None,
        task_id: Some(task.clone()),
        host_data: None,
        view,
        deadline: Instant::now() + Defaults::RUN_TIMEOUT,
        default_page: None,
        tabs_scope: TabsScope::Browser,
    };
    Ok(RunCtx::start(&harness.dispatcher, start))
}

/// A recorded script run that read `scope` at its start, for host task `task`.
fn run_scoped(
    harness: &Harness,
    scope: TabsScope,
    task: Option<&TaskId>,
) -> Result<(Arc<RunCtx>, RunGuard), Box<dyn Error>> {
    let view = harness.link.view().ok_or("the link is up")?;
    let start = RunStart {
        id: RunId::new(),
        code: String::new(),
        request_id: None,
        task_id: task.cloned(),
        host_data: None,
        view,
        deadline: Instant::now() + Defaults::RUN_TIMEOUT,
        default_page: None,
        tabs_scope: scope,
    };
    Ok(RunCtx::start(&harness.dispatcher, start))
}

/// The id of a task `run`'s script creates.
async fn created_in(harness: &Harness, run: &Arc<RunCtx>) -> Result<TaskId, Box<dyn Error>> {
    let created = value(harness.submit(run, "neo.createTask", json!({})).await)?;
    Ok(TaskId::parse(created["id"].as_str().ok_or("task id")?)?)
}

/// A task counts as its creating run's own: it joins the family of the run's host task as the run
/// started, whose root a run naming a created task keeps. A run with no host task makes a family
/// root, even after it picked another task.
#[tokio::test]
async fn a_created_task_joins_the_family_of_its_runs_host_task() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let host = TaskId::parse("host-s")?;
    let (hosted, _hosted_guard) = run_for(&harness, &host)?;
    let child = created_in(&harness, &hosted).await?;
    assert_eq!(
        harness.tabs.task(&child)?.family,
        host,
        "created in the host task's run"
    );
    let (naming, _naming_guard) = run_for(&harness, &child)?;
    let grandchild = created_in(&harness, &naming).await?;
    assert_eq!(
        harness.tabs.task(&grandchild)?.family,
        host,
        "created in a run that names the created task: still the host task's family"
    );
    value(
        harness
            .submit(&naming, "task.close", json!({ "task": child.as_str() }))
            .await,
    )?;
    assert!(
        harness.tabs.task(&child).is_err(),
        "the run closed its own empty host task"
    );
    let late = created_in(&harness, &naming).await?;
    assert_eq!(
        harness.tabs.task(&late)?.family,
        host,
        "the run's family was fixed at its start"
    );

    let (free, _free_guard) = harness.run()?;
    value(
        harness
            .submit(&free, "neo.task", json!({ "id": grandchild.as_str() }))
            .await,
    )?;
    let root = created_in(&harness, &free).await?;
    assert_eq!(
        harness.tabs.task(&root)?.family,
        root,
        "a run with no host task makes a family root, though it picked another task first"
    );
    Ok(())
}

/// Under task scope a run picks, opens in and closes only the tasks its scope allows. One it may
/// not use answers exactly as a missing task, so the refusal reveals nothing about it.
#[tokio::test]
async fn a_task_a_run_may_not_use_answers_as_a_missing_one() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let host = TaskId::parse("host-s")?;
    let other = TaskId::parse("host-r")?;
    let (bound, _bound_guard) = run_scoped(&harness, TabsScope::Task, Some(&host))?;
    let (theirs_run, _theirs_guard) = run_scoped(&harness, TabsScope::Task, Some(&other))?;
    let mine = created_in(&harness, &bound).await?;
    let theirs = created_in(&harness, &theirs_run).await?;
    let answer = |reply: OpReply, id: &TaskId| -> Result<_, Box<dyn Error>> {
        let refused = failure(reply)?;
        Ok((
            refused.error.owner(),
            refused.error.code(),
            refused.error.message().replace(id.as_str(), "<id>"),
        ))
    };
    let missing = TaskId::parse("never-made")?;
    let expected = answer(
        harness
            .submit(&bound, "neo.task", json!({ "id": missing.as_str() }))
            .await,
        &missing,
    )?;
    assert_eq!(
        (expected.0, expected.1),
        (ErrorOwner::Script, ErrorCode::TASK_NOT_FOUND)
    );
    let uses = |id: &TaskId| {
        [
            ("neo.task", json!({ "id": id.as_str() })),
            ("task.newPage", json!({ "task": id.as_str() })),
            ("task.close", json!({ "task": id.as_str() })),
        ]
    };
    for id in [&other, &theirs] {
        for (method, input) in uses(id) {
            assert_eq!(
                answer(harness.submit(&bound, method, input).await, id)?,
                expected,
                "{method} on another family's task answers as a missing one"
            );
        }
    }
    value(
        harness
            .submit(&bound, "neo.task", json!({ "id": mine.as_str() }))
            .await,
    )?;

    let (taskless, _taskless_guard) = run_scoped(&harness, TabsScope::Task, None)?;
    for (method, input) in uses(&host) {
        assert_eq!(
            answer(harness.submit(&taskless, method, input).await, &host)?,
            expected,
            "{method}: a run with no task never takes over a host's task"
        );
    }
    value(
        harness
            .submit(&taskless, "neo.task", json!({ "id": theirs.as_str() }))
            .await,
    )?;

    let (unbound, _unbound_guard) = run_scoped(&harness, TabsScope::Browser, None)?;
    value(
        harness
            .submit(&unbound, "neo.task", json!({ "id": host.as_str() }))
            .await,
    )?;
    Ok(())
}

/// The engine leaves out what a bound run may not see, so its `context.pages()` lists only its
/// family's tabs, and the listing is never withheld by the door's backstop.
#[tokio::test]
async fn a_bound_runs_context_pages_lists_only_its_familys_tabs() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let host = TaskId::parse("host-s")?;
    let other = TaskId::parse("host-r")?;
    let mine = harness.owner_tab("https://mine.example/").await?;
    let theirs = harness.owner_tab("https://theirs.example/").await?;
    let free = harness.owner_tab("https://free.example/").await?;
    for (page, task) in [(&mine, &host), (&theirs, &other)] {
        harness.tabs.ensure_task(task)?;
        harness.tabs.touched(page, harness.tabs.seen());
        harness.tabs.own(page, task)?;
    }
    let (bound, _bound_guard) = run_scoped(&harness, TabsScope::Task, Some(&host))?;
    let pages = value(harness.submit(&bound, "context.pages", json!({})).await)?;
    assert_eq!(
        listed_tabs(&pages),
        vec![mine.tab.0],
        "only its family's tab, and the listing is not withheld"
    );
    let (taskless, _taskless_guard) = run_scoped(&harness, TabsScope::Task, None)?;
    let pages = value(harness.submit(&taskless, "context.pages", json!({})).await)?;
    let listed = listed_tabs(&pages);
    for page in [&mine, &theirs, &free] {
        assert!(
            listed.contains(&page.tab.0),
            "a run with no task lists every tab, as before task scope"
        );
    }
    Ok(())
}

/// A bound run's own open is its own from the moment the browser names it, though no task may
/// hold it: here the run closed its host task first, so the new tab joins none.
#[tokio::test]
async fn a_bound_runs_own_open_is_its_own_though_no_task_holds_it() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let host = TaskId::parse("host-s")?;
    let (bound, _bound_guard) = run_scoped(&harness, TabsScope::Task, Some(&host))?;
    value(
        harness
            .submit(&bound, "task.close", json!({ "task": host.as_str() }))
            .await,
    )?;
    let opened = page_of(&value(
        harness.submit(&bound, "context.newPage", json!({})).await,
    )?)?;
    assert_eq!(harness.tabs.owner(opened.tab), None, "no task holds it");
    value(
        harness
            .submit(&bound, "page.opener", json!({ "page": handle(&opened) }))
            .await,
    )?;
    let (other, _other_guard) = run_scoped(&harness, TabsScope::Task, Some(&host))?;
    assert_eq!(
        failure(
            harness
                .submit(&other, "page.opener", json!({ "page": handle(&opened) }))
                .await
        )?
        .error
        .code(),
        ErrorCode::PAGE_NOT_IN_TASK,
        "another run of the family never opened it, and no task holds it"
    );
    Ok(())
}

/// A bound run's own new tab is its own from the moment the browser names it, while `newPage`
/// still waits for the tab's first answer: the run's other ops list it meanwhile.
#[tokio::test]
async fn a_bound_runs_new_tab_is_its_own_while_new_page_readies_it() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let host = TaskId::parse("host-s")?;
    let (bound, _guard) = run_scoped(&harness, TabsScope::Task, Some(&host))?;
    harness.fake.hold_next("Page.setLifecycleEventsEnabled");
    let opening = harness.dispatcher.submit(
        &bound,
        Call::script("context.newPage".to_owned(), json!({}), None),
    );
    within(async {
        while harness.fake.count("Page.setLifecycleEventsEnabled") == 0 {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let pages = value(harness.submit(&bound, "context.pages", json!({})).await)?;
    harness.fake.release_held();
    let opened = page_of(&value(within(opening.wait()).await?)?)?;
    assert_eq!(
        listed_tabs(&pages),
        vec![opened.tab.0],
        "listed while newPage readied it, though no task holds it yet"
    );
    Ok(())
}

/// `Promise.all([context.waitForEvent('page'), context.newPage()])` in a run bound to its task:
/// the browser announces the new tab before the open's reply names it to Neo, and the wait judges
/// the tab only once the run's open is named, so it hears the run's own tab.
#[tokio::test]
async fn a_bound_runs_page_wait_hears_its_own_new_tab() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let host = TaskId::parse("host-s")?;
    let (bound, _guard) = run_scoped(&harness, TabsScope::Task, Some(&host))?;
    let wait = json!({
        "call": { "api": "browserContext.waitForEvent", "timeoutMs": 10000 },
        "page": null,
        "event": "page",
        "since": 0,
    });
    let waiting = harness
        .dispatcher
        .submit(&bound, Call::script("events.wait".to_owned(), wait, None));
    within(async {
        while harness.fake.count("Target.getTargets") == 0 {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    // The browser makes the tab and announces it at once; the reply naming it waits.
    harness.fake.hold_next("Browser.createTab");
    let opening = harness.dispatcher.submit(
        &bound,
        Call::script("context.newPage".to_owned(), json!({}), None),
    );
    within(async {
        while harness.fake.count("Browser.createTab") == 0 {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    // Long enough for the wait to read the announcement (recorded about a second after it, with
    // no URL), while the open is still in flight.
    time::sleep(Duration::from_secs(2)).await;
    harness.fake.release_held();
    let opened = page_of(&value(within(opening.wait()).await?)?)?;
    let heard = value(within(waiting.wait()).await?)?;
    assert!(
        heard
            .to_string()
            .contains(&format!("\"tab\":{}", opened.tab.0)),
        "the wait heard the run's own new tab {}: {heard}",
        opened.tab.0
    );
    Ok(())
}

/// A host may name an agent's task while a close by a run with no task waits for the task's
/// turn: the close asks again once it holds the turn, and leaves the host's task as it is.
#[tokio::test]
#[expect(
    clippy::await_holding_invalid_type,
    reason = "the test holds the task's turn on purpose, so the close waits for it"
)]
async fn a_taskless_close_asks_again_once_it_holds_the_turn() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (taskless, _taskless_guard) = run_scoped(&harness, TabsScope::Task, None)?;
    let created = created_in(&harness, &taskless).await?;
    let record = harness.tabs.task(&created)?;
    let held = record.turn.lock().await;
    let base = Arc::strong_count(&record.turn);
    let pending = harness.dispatcher.submit(
        &taskless,
        Call::script(
            "task.close".to_owned(),
            json!({ "task": created.as_str() }),
            None,
        ),
    );
    // The close has its copy of the task and waits for the turn.
    within(async {
        while Arc::strong_count(&record.turn) == base {
            task::yield_now().await;
        }
    })
    .await?;
    harness.tabs.ensure_task(&created)?;
    drop(held);
    assert_eq!(
        failure(pending.wait().await)?.error.code(),
        ErrorCode::TASK_NOT_FOUND,
        "the task is a host's now"
    );
    assert!(harness.tabs.task(&created).is_ok(), "and it stays open");
    Ok(())
}

/// A run with no task keeps opening in the agent's task it picked, until a host names that task:
/// from then on its opens, its lazy page and `task.newPage` leave the host's task alone.
#[tokio::test]
async fn a_taskless_run_stops_opening_in_a_task_a_host_named() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (taskless, _taskless_guard) = run_scoped(&harness, TabsScope::Task, None)?;
    let created = created_in(&harness, &taskless).await?;
    let before = page_of(&value(
        harness
            .submit(&taskless, "context.newPage", json!({}))
            .await,
    )?)?;
    assert_eq!(
        harness.tabs.owner(before.tab),
        Some(created.clone()),
        "its open joins the agent's task it picked"
    );

    harness.tabs.ensure_task(&created)?;
    let after = page_of(&value(
        harness
            .submit(&taskless, "context.newPage", json!({}))
            .await,
    )?)?;
    assert_eq!(harness.tabs.owner(after.tab), None, "the host's task now");
    value(
        harness
            .submit(
                &taskless,
                "page.opener",
                json!({ "page": { "$neo": "page", "tab": null } }),
            )
            .await,
    )?;
    assert_eq!(
        harness.tabs.task(&created)?.tabs,
        vec![before.tab],
        "neither its lazy page nor a later open joined the host's task"
    );
    assert_eq!(
        failure(
            harness
                .submit(
                    &taskless,
                    "task.newPage",
                    json!({ "task": created.as_str() })
                )
                .await
        )?
        .error
        .code(),
        ErrorCode::TASK_NOT_FOUND
    );
    Ok(())
}

/// `task.newPage` asks again once its tab opened: a host that names the task while the tab opens
/// leaves the new tab in no task, and the run with no task hears the task is gone.
#[tokio::test]
async fn a_taskless_task_new_page_asks_again_after_its_open() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (taskless, _taskless_guard) = run_scoped(&harness, TabsScope::Task, None)?;
    let created = created_in(&harness, &taskless).await?;
    let opens = harness.fake.count("Browser.createTab");
    harness.fake.hold_next("Browser.createTab");
    let opening = harness.dispatcher.submit(
        &taskless,
        Call::script(
            "task.newPage".to_owned(),
            json!({ "task": created.as_str() }),
            None,
        ),
    );
    within(async {
        while harness.fake.count("Browser.createTab") == opens {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    harness.tabs.ensure_task(&created)?;
    harness.fake.release_held();
    assert_eq!(
        failure(opening.wait().await)?.error.code(),
        ErrorCode::TASK_NOT_FOUND,
        "a host named the task while the tab opened"
    );
    assert!(
        harness.tabs.task(&created)?.tabs.is_empty(),
        "the host's task gained no tab"
    );
    Ok(())
}
