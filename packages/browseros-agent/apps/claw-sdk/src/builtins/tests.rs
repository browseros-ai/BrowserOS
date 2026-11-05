//! The built-ins through the real door, on the fake browser, with a real trace store read back
//! from a temp dir.

use std::error::Error;
use std::fs;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use browseros_captcha::CaptchaSolver;
use browseros_captcha::config::{CAPSOLVER_KEY_FILE_ENV, CAPSOLVER_URL_ENV, CaptchaConfig};
use browseros_core::{NativeCapabilities, PageId, TabId};
use browseros_takeover::{Lock, LockBinding, LockTtl, Locks, Reads};
use browseros_trace::{
    Delivery, Effect, ErrorOwner, OperationId, OperationRow, Outcome, RedactionLimits, Redactor,
    RunId, RunStatus, SecretSet, SecretSetConfig, TraceSink, TraceStore, WriterConfig,
    spawn_writer,
};
use futures_util::future::{self, Either};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::time::{self, Instant};

use super::{BuiltinDeps, CaptchaClaims, TabJanitor};
use crate::browser::{BrowserLink, PageRef, TabRegistry};
use crate::config::Defaults;
use crate::dispatch::{
    Call, Dispatcher, ErrorCode, OpFailure, OpReply, PendingReply, Policy, RegistryBuilder, RunCtx,
    RunGuard, RunStart, StopReason,
};
use crate::host::TakeoverLock;
use crate::test_support::FakeCdp;

/// The built-ins' door on the fake browser, with the app's own rules in the app's order: the
/// takeover lock over browseros-takeover's real `Locks`, then the CAPTCHA claims. A plain data
/// bag.
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
    dispatcher: Arc<Dispatcher>,
    /// Keeps the store's directory for the test's life.
    _dir: TempDir,
}

impl Harness {
    /// A Neo with no CAPTCHA provider key.
    async fn new() -> Result<Self, Box<dyn Error>> {
        Self::with_solver(None).await
    }

    async fn with_solver(solver: Option<Arc<CaptchaSolver>>) -> Result<Self, Box<dyn Error>> {
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
        let dir = tempfile::tempdir()?;
        let store = TraceStore::open(&dir.path().join("trace.db")).await?;
        let secrets = Arc::new(SecretSet::new(SecretSetConfig::default()));
        let redactor = Redactor::new(Arc::clone(&secrets), RedactionLimits::default());
        let sink = spawn_writer(&store, redactor, WriterConfig::default(), None);
        // Armed the way the host API arms it once the service starts.
        let locks = Locks::new(store.clone(), sink.clone());
        let takeover = Arc::new(TakeoverLock::default());
        takeover.arm(Arc::clone(&locks));
        let claims = Arc::new(CaptchaClaims::default());
        let mut registry = RegistryBuilder::default();
        let deps = BuiltinDeps {
            solver,
            claims: Arc::clone(&claims),
        };
        super::builtin_set(&mut registry, deps)?;
        let dispatcher = Arc::new(Dispatcher::new(
            registry.build(),
            vec![
                Arc::clone(&takeover) as Arc<dyn Policy>,
                claims as Arc<dyn Policy>,
            ],
            secrets,
            sink.clone(),
            Arc::clone(&link),
            Arc::clone(&tabs),
        ));
        Ok(Self {
            fake,
            link,
            tabs,
            store,
            sink,
            locks,
            takeover,
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
        Ok(RunCtx::start(&self.sink, start))
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

    fn janitor(&self) -> TabJanitor {
        TabJanitor::new(
            Arc::clone(&self.dispatcher),
            Arc::clone(&self.link),
            Arc::clone(&self.tabs),
            self.sink.clone(),
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
    json!({ "$neo": "page", "id": page.id.0, "tab": page.tab.0 })
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

fn page_of(value: &Value) -> Result<PageRef, Box<dyn Error>> {
    let id = value["id"].as_u64().ok_or("a page brand has an id")?;
    let tab = value["tab"].as_i64().ok_or("a page brand has a tab")?;
    Ok(PageRef {
        id: PageId(u32::try_from(id)?),
        tab: TabId(tab),
    })
}

/// `work`, or an error after 10 s: a deadlock fails the test instead of hanging it.
async fn within<T>(work: impl Future<Output = T>) -> Result<T, Box<dyn Error>> {
    let limit = Duration::from_secs(10);
    time::timeout(limit, work)
        .await
        .map_err(|error| format!("not within {limit:?}: {error}").into())
}

#[tokio::test]
async fn a_script_opens_navigates_reads_and_closes_a_page() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;

    let (opened_op, reply) = harness.submit_op(&run, "context.newPage", json!({})).await;
    let opened = value(reply)?;
    assert_eq!(opened["$neo"], "page", "a brand the bootstrap revives");
    let page = page_of(&opened)?;
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

    // The fake's `Page.navigate` changes nothing, so the test plays the browser's load.
    let url = "https://example.com/";
    harness.fake.navigate_tab(page.tab, url, "Example Domain");
    let input = json!({ "page": handle(&page), "url": url });
    let (goto_op, reply) = harness.submit_op(&run, "page.goto", input).await;
    let fact = reply
        .pages
        .first()
        .cloned()
        .ok_or("goto reports its page")?;
    assert_eq!(
        fact.url.as_deref(),
        Some(url),
        "the door refreshed the navigated page before reading its facts"
    );
    assert_eq!(value(reply)?, json!({ "status": null, "download": false }));
    assert_eq!(harness.first_params("Page.navigate")?["url"], url);

    let on_page = json!({ "page": handle(&page) });
    let (title_op, reply) = harness.submit_op(&run, "page.title", on_page.clone()).await;
    assert_eq!(
        value(reply)?,
        json!("Example Domain"),
        "the tab-strip title"
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
        (goto_op, Effect::Write, Delivery::Done),
        (title_op, Effect::Read, Delivery::NotSent),
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

    let input = json!({ "page": handle(&page), "url": "https://nowhere.invalid/" });
    let failure = failure(harness.submit(&run, "page.goto", input).await)?;
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
async fn new_page_with_a_url_reports_its_tab_when_the_load_fails() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    harness.fake.answer(
        "Page.navigate",
        json!({ "frameId": "frame", "errorText": "net::ERR_CONNECTION_REFUSED" }),
    );

    let reply = harness
        .submit(
            &run,
            "context.newPage",
            json!({ "url": "http://localhost:1/" }),
        )
        .await;
    let fact = reply
        .pages
        .first()
        .cloned()
        .ok_or("the new tab is in the reply")?;
    let failure = failure(reply)?;
    assert_eq!(
        (failure.error.owner(), failure.error.code()),
        (ErrorOwner::Website, ErrorCode::NAVIGATION_FAILED)
    );
    let page = PageRef {
        id: fact.id,
        tab: fact.tab,
    };
    assert!(
        harness
            .tabs
            .idle_agent_tabs(Duration::ZERO, Instant::now())
            .contains(&page),
        "the tab stayed open, so it is the janitor's to close"
    );
    Ok(())
}

#[tokio::test]
async fn a_lock_on_a_new_tab_refuses_the_load_new_page_was_asked_for() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    // The tab exists as soon as the browser made it, and `Browser.createTab`'s reply comes
    // late, so the owner takes the tab between its opening and the load newPage asked for.
    harness.fake.hold_next("Browser.createTab");
    let call = Call::script(
        "context.newPage".to_owned(),
        json!({ "url": "https://example.com/" }),
        None,
    );
    let pending = harness.dispatcher.submit(&run, call);
    let opener = pending.operation();
    within(async {
        while harness.fake.count("Browser.createTab") == 0 {
            time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let tab = *harness
        .fake
        .tabs()
        .last()
        .ok_or("the browser made the tab")?;
    let lock = harness.lock(tab, Reads::Allow).await?;
    harness.fake.release_held();

    let reply = within(pending.wait()).await?;
    let fact = reply
        .pages
        .first()
        .cloned()
        .ok_or("the new tab is in the reply")?;
    assert_eq!(fact.tab, tab, "the script can still find the tab it opened");
    let refused = failure(reply)?;
    assert_eq!(
        (refused.error.owner(), refused.error.code()),
        (ErrorOwner::Host, ErrorCode::OWNER_HAS_CONTROL),
        "the owner holds the new tab: {}",
        refused.error
    );
    assert!(
        refused.error.message().contains(&lock.id.to_string()),
        "the refusal names the lock: {}",
        refused.error
    );
    assert_eq!(
        refused.delivery,
        Delivery::Done,
        "the tab opened, so newPage was sent; only its load was refused"
    );
    assert_eq!(
        harness.fake.count("Page.navigate"),
        0,
        "the browser never loaded the page under the owner's lock"
    );

    harness.sink.flush().await?;
    let detail = harness
        .store
        .run(run.id())
        .await?
        .ok_or("the run is recorded")?;
    let load = detail
        .operations
        .iter()
        .find(|row| row.name == "page.goto")
        .ok_or("the load is an op of its own")?;
    assert_eq!(
        (load.parent, load.tab, load.delivery),
        (Some(opener), Some(tab), Some(Delivery::NotSent)),
        "newPage's load is its child, on the new tab, and refused before anything was sent"
    );
    assert_eq!(
        load.error
            .as_ref()
            .map(|error| (error.owner, error.code.as_str())),
        Some((ErrorOwner::Host, "owner_has_control")),
        "the trace says the lock refused the load"
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
async fn concurrent_calls_on_the_lazy_page_open_one_tab() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;

    let pending: Vec<PendingReply> = (0..5)
        .map(|_| {
            let call = Call::script(
                "page.title".to_owned(),
                json!({ "page": { "$neo": "page" } }),
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
            json!("about:blank"),
            "a new tab starts blank"
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
async fn the_owners_lock_keeps_the_agent_off_their_tab() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let owners = harness.owner_tab("https://example.com/").await?;
    let agents = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    let lock = harness.lock(owners.tab, Reads::Allow).await?;
    assert!(lock.settled, "nothing was in flight on the tab");

    let goto = json!({ "page": handle(&owners), "url": "https://example.com/pay" });
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
            .submit(&run, "page.title", json!({ "page": handle(&owners) }))
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
            .submit(&run, "page.title", json!({ "page": handle(&owners) }))
            .await,
    )?;
    assert_eq!(
        (read.error.code(), read.delivery),
        (ErrorCode::OWNER_HAS_CONTROL, Delivery::NotSent),
        "reads: block refuses reads too: the owner may be typing a password"
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
            .submit(&run, "page.title", json!({ "page": handle(&owners) }))
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
