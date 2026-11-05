//! The built-ins through the real door, on the fake browser, with a real trace store read back
//! from a temp dir.

use std::error::Error;
use std::future::{self, Future};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_core::{NativeCapabilities, PageId, TabId};
use browseros_trace::{
    Delivery, Effect, ErrorOwner, OperationId, OperationRow, Outcome, RedactionLimits, Redactor,
    RunId, RunStatus, SecretSet, SecretSetConfig, TraceSink, TraceStore, WriterConfig,
    spawn_writer,
};
use futures_util::future::join_all;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::time::{self, Instant};

use super::{TabJanitor, builtin_set};
use crate::browser::{BrowserLink, PageRef, TabRegistry};
use crate::config::Defaults;
use crate::dispatch::{
    Call, Dispatcher, ErrorCode, OpFailure, OpReply, OperationError, PendingReply, Policy,
    PolicyCheck, RegistryBuilder, RunCtx, RunGuard, RunStart,
};
use crate::test_support::FakeCdp;

/// The built-ins' door on the fake browser. A plain data bag.
struct Harness {
    fake: Arc<FakeCdp>,
    link: Arc<BrowserLink>,
    tabs: Arc<TabRegistry>,
    store: TraceStore,
    sink: TraceSink,
    lock: Arc<TabLock>,
    dispatcher: Arc<Dispatcher>,
    /// Keeps the store's directory for the test's life.
    _dir: TempDir,
}

/// sdk-traces' takeover lock on one tab: refuses writes to the tab it holds.
#[derive(Default)]
struct TabLock {
    held: Mutex<Option<TabId>>,
}

impl Harness {
    async fn new() -> Result<Self, Box<dyn Error>> {
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
        let lock = Arc::new(TabLock::default());
        let mut registry = RegistryBuilder::default();
        builtin_set(&mut registry)?;
        let dispatcher = Arc::new(Dispatcher::new(
            registry.build(),
            vec![Arc::clone(&lock) as Arc<dyn Policy>],
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
            lock,
            dispatcher,
            _dir: dir,
        })
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
        )
    }
}

impl TabLock {
    fn hold(&self, tab: TabId) {
        *self.held.lock().unwrap_or_else(PoisonError::into_inner) = Some(tab);
    }
}

impl Policy for TabLock {
    fn check(&self, check: &PolicyCheck<'_>) -> Result<(), OperationError> {
        let held = *self.held.lock().unwrap_or_else(PoisonError::into_inner);
        match (check.method.effect, check.tab) {
            (Effect::Write, Some(tab)) if Some(tab) == held => Err(OperationError::host(
                ErrorCode::OWNER_HAS_CONTROL,
                "the owner has control of this tab",
            )),
            (Effect::Write | Effect::Read, _) => Ok(()),
        }
    }
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
    let replies = within(join_all(pending.into_iter().map(PendingReply::wait))).await?;
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
async fn the_janitor_closes_idle_agent_tabs_through_the_door() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    let idle = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    let locked = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    let owners = harness.owner_tab("https://example.com/").await?;
    // A script op touched the owner's tab too, so it is known to the registry, and still Found.
    value(
        harness
            .submit(&run, "page.title", json!({ "page": handle(&owners) }))
            .await,
    )?;
    harness.lock.hold(locked.tab);
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
        harness.is_open(locked.tab).await?,
        "the owner holds this one, so the lock kept it open"
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
    let closes: Vec<(Option<TabId>, Option<Delivery>, Option<String>)> = detail
        .operations
        .iter()
        .map(|row| {
            assert_eq!(row.name, "page.close", "the janitor only closes");
            (
                row.tab,
                row.delivery,
                row.error.as_ref().map(|error| error.code.clone()),
            )
        })
        .collect();
    assert_eq!(closes.len(), 2, "one op per idle agent tab: {closes:?}");
    assert!(
        closes.contains(&(Some(idle.tab), Some(Delivery::Done), None)),
        "the trace says what Neo closed"
    );
    assert!(
        closes.contains(&(
            Some(locked.tab),
            Some(Delivery::NotSent),
            Some("owner_has_control".to_owned())
        )),
        "and what the lock refused"
    );

    harness.lock.hold(TabId(0));
    let again = Instant::now() + Defaults::TAB_IDLE;
    assert!(
        within(janitor.sweep(again)).await?.is_some(),
        "the refused tab is still a candidate"
    );
    assert!(
        !harness.is_open(locked.tab).await?,
        "once the owner lets go, the next sweep closes it"
    );
    Ok(())
}
