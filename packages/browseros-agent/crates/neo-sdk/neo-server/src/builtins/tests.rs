//! The built-ins through the real door, on the fake browser, with a real trace store read back
//! from a temp dir.

use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_core::{NativeCapabilities, TabId, TargetId};
use futures_util::future::{self, Either};
use neo_captcha::CaptchaSolver;
use neo_captcha::config::{CAPSOLVER_KEY_FILE_ENV, CAPSOLVER_URL_ENV, CaptchaConfig};
use neo_playwright::{ErrorCode as PlaywrightCode, Op, OpEnv, PlaywrightError, Refusal, RunState};
use neo_takeover::{Lock, LockBinding, LockTtl, Locks, Reads};
use neo_trace::{
    Delivery, Effect, ErrorOwner, OperationId, OperationRow, Outcome, RedactionLimits, Redactor,
    RunId, RunStatus, TraceSink, TraceStore, WriterConfig, spawn_writer,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::Notify;
use tokio::task;
use tokio::time::{self, Instant};

use super::event_release::EventRelease;
use super::op_marks::Hold;
use super::playwright_ops::{self, OpCtxEnv, leaf_error};
use super::{BuiltinDeps, CaptchaClaims, PlaywrightFacade, TabJanitor};
use crate::browser::{BrowserLink, PageRef, TabRegistry};
use crate::config::Defaults;
use crate::dispatch::{
    Call, Dispatcher, ErrorCode, EventLog, LeafError, Lifetime, Listing, Method, MethodSpec,
    Observer, OpCtx, OpFailure, OpReply, OperationError, PageChange, PageScript, PageTarget,
    PendingReply, Policy, PolicyCheck, RegistryBuilder, RegistryError, RunCtx, RunGuard, RunStart,
    StopReason, Visibility,
};
use crate::host::TakeoverLock;
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
    /// What the host API's lock routes write; the door and the janitor read it through
    /// `takeover`.
    locks: Arc<Locks>,
    takeover: Arc<TakeoverLock>,
    /// The facade every Playwright op of `dispatcher` runs on, as app.rs builds it.
    playwright: Arc<PlaywrightFacade>,
    dispatcher: Arc<Dispatcher>,
    /// Keeps the store's directory for the test's life.
    _dir: TempDir,
}

impl Harness {
    /// A Neo with no CAPTCHA provider key.
    async fn new() -> Result<Self, Box<dyn Error>> {
        Self::with_solver(None).await
    }

    /// A Neo with the chosen CAPTCHA solver.
    async fn with_solver(solver: Option<Arc<CaptchaSolver>>) -> Result<Self, Box<dyn Error>> {
        Self::with_mode(solver, false).await
    }

    /// A browser with explicit event capabilities: the fake can emit native console events.
    async fn with_mode(
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
        // in a sealed world the fake browser does not have, and this one is a single CDP call.
        fake.answer("Target.getTargetInfo", json!({ "targetInfo": {} }));
        let dir = tempfile::tempdir()?;
        let store = TraceStore::open(&dir.path().join("trace.db")).await?;
        let redactor = Redactor::new(RedactionLimits::default());
        let sink = spawn_writer(&store, redactor, WriterConfig::default(), None);
        // Armed the way the host API arms it once the service starts.
        let locks = Locks::new(store.clone(), sink.clone());
        let takeover = Arc::new(TakeoverLock::default());
        takeover.arm(Arc::clone(&locks));
        let claims = Arc::new(CaptchaClaims::default());
        let playwright = PlaywrightFacade::start(Arc::clone(&link), hosted);
        let mut registry = RegistryBuilder::default();
        let deps = BuiltinDeps {
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
            playwright,
            dispatcher,
            _dir: dir,
        })
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
        };
        Ok(RunCtx::start(&self.dispatcher, start))
    }

    /// One script call, answered.
    async fn submit(&self, run: &Arc<RunCtx>, name: &str, input: Value) -> OpReply {
        self.submit_op(run, name, input).await.1
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
            Arc::clone(&self.takeover),
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
        harness
            .tabs
            .idle_agent_tabs(Duration::ZERO, Instant::now())
            .contains(&page),
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
    assert_eq!(value(reply)?, json!({}));
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
    let close = json!({ "page": handle(&page), "runBeforeUnload": true });
    let (close_op, reply) = harness.submit_op(&run, "page.close", close).await;
    let fact = reply
        .pages
        .first()
        .cloned()
        .ok_or("close reports its page")?;
    assert_eq!(value(reply)?, json!({}));
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
        harness
            .tabs
            .idle_agent_tabs(Duration::ZERO, Instant::now())
            .contains(&opened),
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
/// read: what the owner typed into a field no seal masks.
const OWNERS_TEXT: &str = "the owner typed 4 Elm St";

/// `test.write`: a write on a page that marks itself sent, then answers only once the test says
/// so, so that the owner can take a tab while it runs. `page_script` says what it stands for:
/// a main-world script, as `OpMarks` marks `page.evaluate`, or a sealed write such as a click.
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

/// A sealed write's end goes out as it ended, a reads-blocking lock or not: the door's release
/// check is for what a script read, and a click's own result is not that.
#[tokio::test]
async fn a_sealed_write_is_released_as_it_ended_under_a_reads_blocking_takeover()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://bank.example/").await?;
    let (_, reply) = answer_after_a_takeover(
        &harness,
        Takeover::of(&owners, Reads::Block),
        PageScript::Sealed,
        Answers::Read,
    )
    .await?;
    assert_eq!(value(reply)?, json!({ "read": OWNERS_TEXT }));
    let (_, thrown) = answer_after_a_takeover(
        &harness,
        Takeover::of(&owners, Reads::Block),
        PageScript::Sealed,
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
            "on": true,
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
    let (run, _guard) = harness.run()?;
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
    let lock = harness.lock(held.tab, Reads::Block).await?;
    let janitor = harness.janitor();

    assert_eq!(
        janitor.sweep(Instant::now()).await,
        None,
        "nothing has been idle for 30 minutes yet"
    );
    let later = Instant::now() + Defaults::TAB_IDLE;
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
        within(janitor.sweep(Instant::now() + Defaults::TAB_IDLE)).await?,
        None,
        "with only a held tab idle there is nothing to close, and no run is recorded"
    );
    harness.locks.unlock_host(lock.id)?;
    let again = Instant::now() + Defaults::TAB_IDLE;
    assert!(
        within(janitor.sweep(again)).await?.is_some(),
        "the held tab stayed a candidate"
    );
    assert!(
        !harness.is_open(held.tab).await?,
        "once the owner lets go, the next sweep closes it"
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
        let env = OpCtxEnv::new(op, false);
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
            matches!(spec.name, "events.next" | "page.locatorHandlerTriggered"),
            "{}: only the prelude's pulls are cut quietly when the run ends",
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

#[tokio::test]
async fn a_goto_sends_web_file_and_javascript_urls_to_the_browser() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://bank.example/").await?;
    let (run, _guard) = harness.run()?;
    // The browser fails every load it is sent, so a URL the door let through answers at once.
    harness.fake.answer(
        "Page.navigate",
        json!({ "frameId": "frame", "errorText": "net::ERR_ABORTED" }),
    );
    let goto = |url: &str| {
        json!({
            "call": { "api": "page.goto", "timeoutMs": 30000 },
            "page": handle(&owners),
            "url": url,
            "waitUntil": "load",
        })
    };
    // Other unsupported schemes keep their existing refusal.
    for url in [
        "view-source:https://bank.example/",
        "chrome://version",
        "devtools://devtools/bundled/inspector.html",
        "chrome-extension://abcdefghijklmnop/popup.html",
        "blob:https://bank.example/9d2c",
        "filesystem:https://bank.example/temporary/x",
        // No scheme the URL Standard reads, so none to allow.
        "bank.example/login",
    ] {
        let (operation, reply) = harness.submit_op(&run, "page.goto", goto(url)).await;
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

    for url in [
        "localhost:3000/",
        "127.0.0.1:3000/",
        "http://bank.example/",
        "HTTPS://bank.example/login",
        "data:text/html,<p>made in the script</p>",
        "about:blank",
        "file:///tmp/local.html",
        " FILE:///tmp/local.html",
        "fi\tle:///tmp/local.html",
        "javascript:void(document.title = 'ran')",
        " JavaScript:void(0)",
        "java\tscript:void(0)",
    ] {
        let failed = failure(harness.submit(&run, "page.goto", goto(url)).await)?;
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
            "file:///tmp/local.html",
            "file:///tmp/local.html",
            "file:///tmp/local.html",
            "javascript:void(document.title = 'ran')",
            "javascript:void(0)",
            "javascript:void(0)",
        ]
        .map(Value::from),
        "each URL is normalized as Playwright does before reaching Chromium"
    );
    Ok(())
}

#[tokio::test]
async fn a_reload_reloads_only_a_page_on_an_allowed_scheme() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    // The owner opened a local file in a tab. The agent never navigated it, but a reload would
    // load the file again for its next read.
    let local = harness.owner_tab("file:///etc/hosts").await?;
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
        "options": {},
    });
    let (operation, reply) = harness.submit_op(&run, "locator.click", input).await;
    // The fake browser has no sealed world to click in, so the crate's click stops before it
    // sends anything; `tests/playwright_browseros.rs` clicks on a real one. What is pinned here is
    // that the door let the action through to the crate, on the locator's tab.
    let failure = failure(reply)?;
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

#[tokio::test]
async fn the_crate_gets_the_page_the_door_resolved_never_the_lazy_one() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let listen = json!({
        "call": { "api": "page.on", "timeoutMs": 0 },
        "page": { "$neo": "page", "tab": null },
        "event": "dialog",
        "on": true,
        "since": 0,
    });
    let (operation, reply) = harness.submit_op(&run, "events.listen", listen).await;
    let opened = reply
        .default_page
        .clone()
        .ok_or("the door opened the run's page for the listener")?;
    assert_eq!(
        value(reply)?,
        json!({}),
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
        let env = OpCtxEnv::new(op, false);
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
        for _ in 0..16 {
            assert!(harness.fake.page_event(
                noise.tab,
                "Runtime.consoleAPICalled",
                json!({
                    "type":"log", "args":[{"type":"string","value":"noise"}],
                    "executionContextId":1, "timestamp":0
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
                    "event":event, "on":true, "since":0
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
        let env = OpCtxEnv::new(op, false);
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
                    "event":"close", "on":true, "since":0
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
    // Keep the native tab around until the controlled reply, allowing a target swap
    // while PageManager::close holds the old pre-transport PageInfo.
    harness.fake.answer("Browser.closeTab", json!({}));
    harness.fake.hold_next("Browser.closeTab");
    let pending = harness.dispatcher.submit(
        &run,
        Call::script("page.close".into(), json!({"page":handle(&page)}), None),
    );
    within(async {
        while harness.fake.count("Browser.closeTab") == 0 {
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
                    "event":"close", "on":true, "since":0
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

/// Hosted mode cannot produce these events; a valid wait fails before spending its timeout,
/// while ordinary listeners retain their existing accepted-but-silent behavior.
#[tokio::test]
async fn hosted_console_and_pageerror_waits_refuse_without_changing_listeners()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::with_mode(None, true).await?;
    let page = harness.owner_tab("https://example.com/events").await?;
    let (run, _guard) = harness.run()?;
    for event in ["console", "pageerror"] {
        let listen = json!({"call":{"api":"page.on","timeoutMs":0},
            "page":handle(&page),"event":event,"on":true,"since":0});
        assert_eq!(
            value(harness.submit(&run, "events.listen", listen).await)?,
            json!({})
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
        "page":handle(&page),"event":"console","on":true,"since":0});
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
