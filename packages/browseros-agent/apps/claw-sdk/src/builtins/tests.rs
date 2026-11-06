//! The built-ins through the real door, on the fake browser, with a real trace store read back
//! from a temp dir.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_captcha::CaptchaSolver;
use browseros_captcha::config::{CAPSOLVER_KEY_FILE_ENV, CAPSOLVER_URL_ENV, CaptchaConfig};
use browseros_core::sensitive_fields::{FieldValues, JournalEntry, SensitiveKind};
use browseros_core::{NativeCapabilities, PageId, TabId};
use browseros_playwright::{
    Capability, ErrorCode as PlaywrightCode, Op, OpEnv, PlaywrightError, PlaywrightHooks, RunState,
    TypedBy,
};
use browseros_takeover::{Lock, LockBinding, LockTtl, Locks, Reads};
use browseros_trace::{
    Delivery, Effect, EpochMillis, ErrorOwner, OperationId, OperationRow, Outcome, Privacy,
    PrivateReason, RedactionLimits, Redactor, RunId, RunStatus, SecretSet, SecretSetConfig,
    TraceSink, TraceStore, WriterConfig, spawn_writer,
};
use futures_util::future::{self, Either};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::time::{self, Instant};

use super::op_marks::Hold;
use super::playwright_ops::{self, OpCtxEnv, leaf_error};
use super::{BuiltinDeps, CaptchaClaims, PlaywrightFacade, PrivacyHooks, TabJanitor};
use crate::browser::{BrowserLink, PageRef, TabRegistry};
use crate::config::{Defaults, SecretFiles};
use crate::dispatch::{
    Call, Dispatcher, ErrorCode, LeafError, Lifetime, Listing, Method, MethodSpec, OpCtx,
    OpFailure, OpReply, OperationError, PageTarget, PendingReply, Policy, PolicyCheck,
    RegistryBuilder, RegistryError, RunCtx, RunGuard, RunStart, StopReason, Visibility,
};
use crate::host::{FlagRecovery, TakeoverLock};
use crate::test_support::{FakeCdp, test_config};

/// The built-ins' door on the fake browser, with the app's own rules in the app's order
/// (`door_rules`): the takeover lock over browseros-takeover's real `Locks`, then tab privacy,
/// then the CAPTCHA claims. A plain data bag.
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
    /// The privacy callbacks the facade's crate calls, for tests that play its journal.
    hooks: Arc<PrivacyHooks>,
    /// Tab privacy, which the host API's routes and a takeover set.
    privacy: Arc<Privacy>,
    dispatcher: Arc<Dispatcher>,
    /// Keeps the store's directory for the test's life.
    _dir: TempDir,
}

impl Harness {
    /// A Neo with no CAPTCHA provider key.
    async fn new() -> Result<Self, Box<dyn Error>> {
        Self::with_solver(None).await
    }

    /// A Neo that names no secret files, since no test that uploads uses it.
    async fn with_solver(solver: Option<Arc<CaptchaSolver>>) -> Result<Self, Box<dyn Error>> {
        Self::build(solver, SecretFiles::Named(Vec::new())).await
    }

    /// A Neo whose config located `secret_files`, which its hooks name to the crate's deny list.
    async fn naming(secret_files: SecretFiles) -> Result<Self, Box<dyn Error>> {
        Self::build(None, secret_files).await
    }

    async fn build(
        solver: Option<Arc<CaptchaSolver>>,
        secret_files: SecretFiles,
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
        // `page.opener` is the page read these tests use: browseros-playwright's other reads run
        // in a sealed world the fake browser does not have, and this one is a single CDP call.
        fake.answer("Target.getTargetInfo", json!({ "targetInfo": {} }));
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
        let privacy = Privacy::new(Arc::clone(&secrets), Arc::new(sink.clone()), HashMap::new());
        let hooks = Arc::new(PrivacyHooks::new(
            Arc::clone(&privacy),
            Arc::clone(&tabs),
            FlagRecovery::none(),
            secret_files,
            Defaults::TYPED_QUIET,
        ));
        let playwright = PlaywrightFacade::start(Arc::clone(&link), Arc::clone(&hooks) as _);
        let mut registry = RegistryBuilder::default();
        let deps = BuiltinDeps {
            solver,
            claims: Arc::clone(&claims),
            playwright: Arc::clone(&playwright),
        };
        super::builtin_set(&mut registry, deps)?;
        let dispatcher = Arc::new(Dispatcher::new(
            registry.build(),
            super::door_rules(&takeover, &hooks, &claims),
            secrets,
            sink.clone(),
            hooks.typed(),
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
            playwright,
            hooks,
            privacy,
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
            Arc::new(SecretSet::new(SecretSetConfig::default())),
            self.sink.clone(),
            self.hooks.typed(),
            Arc::clone(&self.link),
            Arc::clone(&self.tabs),
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

/// Neo's secret files when its host token, its state and its browser profile all sit under
/// `root`, located as `AppConfig::load` locates them: the token `root/host-token`, the trace
/// store's directory `root/claw-sdk` and the profile `root/profile`.
fn secret_files_under(root: &Path) -> SecretFiles {
    let mut config = test_config(root);
    config.env.host_api.token_file = Some(root.join("host-token"));
    SecretFiles::locate(&config.env, &config.launch, &Ok(PathBuf::from("/")))
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

/// The page a `context.newPage` entry `{ page, url }` names.
fn page_of(entry: &Value) -> Result<PageRef, Box<dyn Error>> {
    let id = entry["page"]["id"]
        .as_u64()
        .ok_or("a page brand has an id")?;
    let tab = entry["page"]["tab"]
        .as_i64()
        .ok_or("a page brand has a tab")?;
    Ok(PageRef {
        id: PageId(u32::try_from(id)?),
        tab: TabId(tab),
    })
}

/// One keystroke as the seal's journal reports it: the field's whole value.
fn keystroke(value: &str) -> JournalEntry {
    JournalEntry {
        kind: SensitiveKind::Password,
        value: value.to_owned().into(),
    }
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
async fn concurrent_calls_on_the_lazy_page_open_one_tab() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;

    let pending: Vec<PendingReply> = (0..5)
        .map(|_| {
            let call = Call::script(
                "page.opener".to_owned(),
                json!({ "page": { "$neo": "page", "id": null } }),
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
        "page": { "$neo": "page", "id": null },
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

#[tokio::test]
async fn a_capture_that_ends_after_its_tab_went_private_is_withheld_closed_or_not()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = harness.run()?;
    for closed in [false, true] {
        for (op, capture, pixels) in [
            ("page.screenshot", "Page.captureScreenshot", "cGl4ZWxz"),
            ("page.pdf", "Page.printToPDF", "JVBERi0xLjQ="),
        ] {
            let owners = harness.owner_tab("https://bank.example/").await?;
            harness.fake.answer(capture, json!({ "data": pixels }));
            harness.fake.hold_next(capture);
            let input =
                json!({ "call": { "api": op, "timeoutMs": 30000 }, "page": handle(&owners) });
            let sent = harness.fake.count(capture);
            let pending = harness
                .dispatcher
                .submit(&run, Call::script(op.to_owned(), input, None));
            // The browser took the capture while the tab was not private yet. The owner takes
            // the tab before the pixels reach Neo, and may type a password into the frame they
            // show. A tab that then closes ends its flag, but not what that frame shows.
            within(async {
                while harness.fake.count(capture) == sent {
                    time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await?;
            harness
                .privacy
                .make_private(owners.tab, PrivateReason::Takeover, EpochMillis::now())
                .await?;
            if closed {
                harness.privacy.tab_closed(owners.tab).await;
            }
            harness.fake.release_held();
            let withheld = failure(within(pending.wait()).await?)
                .map_err(|error| format!("{op}, closed: {closed}: {error}"))?;
            assert_eq!(
                (
                    withheld.error.owner(),
                    withheld.error.code(),
                    withheld.delivery
                ),
                (ErrorOwner::Host, ErrorCode::PAGE_PRIVATE, Delivery::NotSent),
                "{op}, closed: {closed}: {}",
                withheld.error
            );
        }
    }
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
    // browseros-playwright's target watcher subscribes to the browser's events before its first
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
async fn no_tab_is_captured_until_neo_made_up_for_lost_private_flags() -> Result<(), Box<dyn Error>>
{
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://bank.example/").await?;
    harness.tabs.touched(&owners, None);
    // A replaced trace store, before the host API has made the open tabs private.
    let dir = tempfile::tempdir()?;
    let recovery = FlagRecovery::before_replacing(&dir.path().join("trace.db")).await?;
    let hooks = PrivacyHooks::new(
        Arc::clone(&harness.privacy),
        Arc::clone(&harness.tabs),
        Arc::clone(&recovery),
        SecretFiles::Named(Vec::new()),
        Defaults::TYPED_QUIET,
    );
    assert_eq!(
        hooks
            .capture_allowed(owners.id.clone())
            .err()
            .map(|refusal| refusal.code),
        Some("page_private".to_owned()),
        "no capture while it is unknown which tabs were private"
    );
    recovery.recover(&harness.privacy, &[]).await;
    assert!(
        hooks.capture_allowed(owners.id.clone()).is_ok(),
        "once the open tabs are private, a tab that is not may be captured"
    );
    Ok(())
}

/// The crate asks the hooks for its deny list on every upload: they answer with Neo's own secret
/// files as config located them, each by its absolute path, or with nothing while one has no
/// known place, and then the crate uploads no file from Neo's disk.
#[tokio::test]
async fn the_hooks_name_neos_secret_files_to_the_crate() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let harness = Harness::naming(secret_files_under(root.path())).await?;
    assert_eq!(
        harness.hooks.secret_paths(),
        Some(vec![
            root.path().join("host-token"),
            root.path().join("claw-sdk"),
            root.path().join("profile"),
        ]),
        "the host token, the trace store's directory and the browser profile"
    );

    let mut config = test_config(root.path());
    config.launch.execution_dir = None;
    let unplaced = SecretFiles::locate(&config.env, &config.launch, &Ok(PathBuf::from("/")));
    let harness = Harness::naming(unplaced).await?;
    assert_eq!(
        harness.hooks.secret_paths(),
        None,
        "with the profile's place unknown, the crate refuses every upload from a path"
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

/// `test.env`: what the adapter's `OpEnv` gives browseros-playwright, over this op's own context.
/// A refused recheck is reported as the crate sees it, or, with `handBack`, returned the way the
/// crate returns it once its `send_effect` is refused.
struct EnvProbe;

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
        let env = OpCtxEnv::new(op);
        let shares_the_runs_state = Arc::ptr_eq(env.run_state(), &op.run().slot::<RunState>());
        future::ready(match env.recheck() {
            Ok(()) => {
                env.mark_sent();
                Ok(json!({ "sharesTheRunsState": shares_the_runs_state }))
            }
            Err(refused) if input.hand_back => Err(leaf_error(&PlaywrightError::from(refused))),
            Err(refused) => Ok(json!({ "refused": {
                "owner": refused.owner,
                "code": refused.code,
                "message": refused.message,
            } })),
        })
    }
}

/// `test.crateFailure`: fails through the adapter's mapping with one of browseros-playwright's
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
        assert_eq!(spec.sensitive_args, op.sensitive_args, "{}", spec.name);
        assert!(
            spec.secret_args.is_empty(),
            "{}: no Playwright argument is always secret",
            spec.name
        );
        assert_eq!(spec.visibility, Visibility::Script, "{}", spec.name);
        assert_eq!(
            spec.lifetime == Lifetime::Background,
            spec.name == "events.next",
            "{}: only the event pump is cut quietly when the run ends",
            spec.name
        );
        let Listing::Grouped(line) = spec.listing else {
            return Err(format!("{} is listed with the Playwright API", spec.name).into());
        };
        lines.insert(line);
    }
    assert_eq!(lines.len(), 1, "the description lists the API once");
    let fill = listed
        .iter()
        .find(|spec| spec.name == "locator.fill")
        .ok_or("locator.fill")?;
    assert_eq!(
        fill.sensitive_args,
        ["/value"],
        "a fill into a password field never stores the typed value"
    );
    Ok(())
}

/// Each Playwright API that runs the agent's script in the main world, as the prelude sends it:
/// the crate op its alias shares, and the input, whose `call.api` names the API. Some name the
/// run's lazy page and some `owners_page`; the hold answers both before the door resolves a page.
fn main_world_calls(owners_page: &Value) -> [(&'static str, Value); 10] {
    let lazy = json!({ "$neo": "page", "id": null });
    let field = json!({
        "$neo": "element", "page": owners_page, "seq": 0, "node": 12,
        "display": "locator('input[type=password]')",
    });
    let read_value = "(field) => field.value";
    let read_values = "(fields) => fields.map((field) => field.value)";
    [
        (
            "page.evaluate",
            json!({
                "call": { "api": "page.evaluate", "timeoutMs": 0 },
                "page": lazy,
                "expression": "() => document.querySelector('input[type=password]').value",
                "isFunction": true,
            }),
        ),
        (
            "page.evaluate",
            json!({
                "call": { "api": "frame.evaluate", "timeoutMs": 0 },
                "page": owners_page, "frame": "F1",
                "expression": "() => document.querySelector('input[type=password]').value",
                "isFunction": true,
            }),
        ),
        (
            "page.waitForFunction",
            json!({
                "call": { "api": "page.waitForFunction", "timeoutMs": 1000 },
                "page": lazy,
                "expression": "() => document.readyState === 'complete'",
                "isFunction": true,
                "polling": "raf",
            }),
        ),
        (
            "locator.evaluate",
            json!({
                "call": { "api": "locator.evaluate", "timeoutMs": 5000 },
                "target": locator(owners_page), "strict": "strict",
                "expression": read_value, "isFunction": true,
            }),
        ),
        (
            "locator.evaluate",
            json!({
                "call": { "api": "page.$eval", "timeoutMs": 0 },
                "target": locator(&lazy), "strict": "firstMatch", "noWait": true,
                "expression": read_value, "isFunction": true,
            }),
        ),
        (
            "locator.evaluate",
            json!({
                "call": { "api": "elementHandle.evaluate", "timeoutMs": 0 },
                "target": field, "strict": "strict",
                "expression": read_value, "isFunction": true,
            }),
        ),
        (
            "locator.evaluateAll",
            json!({
                "call": { "api": "locator.evaluateAll", "timeoutMs": 0 },
                "target": locator(owners_page),
                "expression": read_values, "isFunction": true,
            }),
        ),
        (
            "locator.evaluateAll",
            json!({
                "call": { "api": "page.$$eval", "timeoutMs": 0 },
                "target": locator(&lazy),
                "expression": read_values, "isFunction": true,
            }),
        ),
        (
            "page.setContent",
            json!({
                "call": { "api": "page.setContent", "timeoutMs": 30000 },
                "page": lazy,
                "html": "<script>document.title = window.appState.password</script>",
                "waitUntil": "load",
            }),
        ),
        (
            "page.setContent",
            json!({
                "call": { "api": "frame.setContent", "timeoutMs": 30000 },
                "page": owners_page, "frame": "F1",
                "html": "<script>document.title = parent.document.cookie</script>",
                "waitUntil": "load",
            }),
        ),
    ]
}

#[tokio::test]
async fn the_ops_that_run_script_in_the_pages_main_world_are_held() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owners = harness.owner_tab("https://bank.example/").await?;
    let (run, _guard) = harness.run()?;
    let calls = main_world_calls(&handle(&owners));
    let tried: HashSet<&str> = calls.iter().map(|(name, _)| *name).collect();
    let main_world: HashSet<&str> = Op::all()
        .map(Op::spec)
        .filter(|spec| spec.uses(Capability::MainWorld))
        .map(|spec| spec.name)
        .collect();
    assert_eq!(
        tried, main_world,
        "every op that declares the main world is tried"
    );

    let before = harness.fake.calls().len();
    for (name, input) in calls {
        let api = input["call"]["api"].clone();
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
                ErrorCode::METHOD_UNAVAILABLE,
                Delivery::NotSent
            ),
            "{api} ({name}): {}",
            refused.error
        );
        let message = refused.error.message();
        assert!(
            message.contains(&format!("`{name}` is held")) && message.contains("main world"),
            "{api}: the message names the op and says why, so the agent reads the page another \
             way: {}",
            refused.error
        );
        assert_eq!(
            harness.row(operation).await?.error.map(|error| error.code),
            Some("method_unavailable".to_owned()),
            "{api}: the trace records the attempt"
        );
    }
    // The browser hears only the listings Neo makes in the background, never a page opened,
    // attached to or run in.
    assert_eq!(
        sent_since(&harness, before),
        Vec::<String>::new(),
        "refused before the door resolves a page: nothing reached the browser"
    );
    Ok(())
}

/// The browser calls Neo makes since `before` other than its own background ones (tab listings,
/// target discovery, the download policy, the crate's one read of the platform): what an op
/// sent the browser.
fn sent_since(harness: &Harness, before: usize) -> Vec<String> {
    harness
        .fake
        .calls()
        .into_iter()
        .skip(before)
        .map(|call| call.method)
        .filter(|method| {
            ![
                "Browser.getTabs",
                "Target.getTargets",
                "Target.setDiscoverTargets",
                "Browser.setDownloadBehavior",
                "Browser.getVersion",
            ]
            .contains(&method.as_str())
        })
        .collect()
}

/// `locator.setInputFiles` of `path` into a file input on `page`, as the prelude sends it.
fn upload(page: &PageRef, path: &Path) -> Value {
    json!({
        "call": { "api": "locator.setInputFiles", "timeoutMs": 500 },
        "target": locator(&handle(page)),
        "strict": "strict",
        "files": { "paths": [path] },
    })
}

/// An upload of one of Neo's own secret files is refused before the crate touches the page, and
/// names no path; any other file goes on to the page. The fake browser has no sealed world to
/// find the input in, so that upload stops there, before anything is sent:
/// `tests/uploads_browseros.rs` hands a real page the file.
#[tokio::test]
async fn a_file_upload_refuses_neos_own_secrets_and_uploads_the_rest() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    let token = root.path().join("host-token");
    fs::write(&token, "the token a host presents")?;
    let harness = Harness::naming(secret_files_under(root.path())).await?;
    // The agent's own file, in a directory of its own.
    let own = tempfile::tempdir()?;
    let report = own.path().join("report.csv");
    fs::write(&report, "id,total\n")?;
    let page = harness.owner_tab("https://forms.example/").await?;
    let (run, _guard) = harness.run()?;

    let before = harness.fake.calls().len();
    let (operation, reply) = harness
        .submit_op(&run, "locator.setInputFiles", upload(&page, &token))
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
            ErrorCode::SECRET_UPLOAD_REFUSED,
            Delivery::NotSent
        ),
        "{}",
        refused.error
    );
    let message = refused.error.message();
    assert!(
        !message.contains(&token.display().to_string()) && !message.contains("host-token"),
        "the refusal names no path: {message}"
    );
    assert_eq!(
        harness.row(operation).await?.error.map(|error| error.code),
        Some("neo_secret_upload_refused".to_owned()),
        "the trace records the refusal"
    );
    assert_eq!(
        sent_since(&harness, before),
        Vec::<String>::new(),
        "refused before the crate touched the page"
    );

    let before = harness.fake.calls().len();
    let passed = failure(
        harness
            .submit(&run, "locator.setInputFiles", upload(&page, &report))
            .await,
    )?;
    assert_ne!(
        passed.error.code(),
        ErrorCode::SECRET_UPLOAD_REFUSED,
        "an ordinary file passes the deny list: {}",
        passed.error
    );
    assert!(
        !sent_since(&harness, before).is_empty(),
        "and the crate goes on to the page to find its input: {}",
        passed.error
    );
    assert_eq!(
        harness.fake.count("DOM.setFileInputFiles"),
        0,
        "the secret never reached the browser"
    );
    Ok(())
}

/// SQLite deletes the trace store's WAL when the last connection closes and writes it again on
/// the next write, so the WAL is often absent when Neo locates its secret files. The trace
/// store's directory is the entry, and the crate resolves it again on each upload.
#[tokio::test]
async fn a_trace_wal_absent_at_start_is_refused_once_it_appears() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let trace_dir = root.path().join("claw-sdk");
    fs::create_dir(&trace_dir)?;
    fs::write(trace_dir.join("trace.db"), "")?;
    let harness = Harness::naming(secret_files_under(root.path())).await?;
    let wal = trace_dir.join("trace.db-wal");
    assert!(!wal.exists(), "no WAL when Neo located its secret files");
    fs::write(&wal, "frames a later write left")?;
    let page = harness.owner_tab("https://forms.example/").await?;
    let (run, _guard) = harness.run()?;

    let refused = failure(
        harness
            .submit(&run, "locator.setInputFiles", upload(&page, &wal))
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
            ErrorCode::SECRET_UPLOAD_REFUSED,
            Delivery::NotSent
        ),
        "{}",
        refused.error
    );
    assert_eq!(harness.fake.count("DOM.setFileInputFiles"), 0);
    Ok(())
}

#[tokio::test]
async fn a_goto_opens_only_http_https_data_and_about_urls() -> Result<(), Box<dyn Error>> {
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
    // Each would show the agent what it must not see (a local file such as Neo's host token,
    // the browser's own pages, a page's source) or run script in the tab's main world, however
    // the scheme is spelled.
    for url in [
        "file:///etc/hosts",
        " FILE:///etc/hosts",
        "fi\tle:///run/neo/host-token",
        "view-source:https://bank.example/",
        "chrome://version",
        "devtools://devtools/bundled/inspector.html",
        "chrome-extension://abcdefghijklmnop/popup.html",
        "blob:https://bank.example/9d2c",
        "filesystem:https://bank.example/temporary/x",
        "javascript:void(document.title = document.querySelector('input[type=password]').value)",
        " JavaScript:void(0)",
        "java\tscript:void(0)",
        // No scheme the URL Standard reads, so none to allow.
        "bank.example/login",
        // Its scheme is `localhost` to the URL Standard, as to the browser. Playwright's own goto
        // would add `http://`; Neo reads the URL as written, and the script writes the scheme.
        "localhost:3000/",
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

    let allowed = [
        "http://bank.example/",
        "HTTPS://bank.example/login",
        "data:text/html,<p>made in the script</p>",
        "about:blank",
    ];
    for url in allowed {
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
        allowed.map(Value::from),
        "each sent as the script wrote it"
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
    let open = harness.dispatcher_with(Vec::new(), |registry| registry.define(EnvProbe))?;
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

    let taken = harness.dispatcher_with(
        vec![Arc::new(TakenWhileWaiting::default()) as Arc<dyn Policy>],
        |registry| registry.define(EnvProbe),
    )?;
    let seen = taken
        .submit(
            &run,
            Call::script("test.env".to_owned(), json!({ "handBack": false }), None),
        )
        .wait()
        .await;
    assert_eq!(
        value(seen)?,
        json!({ "refused": {
            "owner": "host",
            "code": "owner_has_control",
            "message": "the owner took the tab while the op waited",
        } }),
        "the rule's refusal reaches the crate unchanged"
    );
    let handed_back = taken
        .submit(
            &run,
            Call::script("test.env".to_owned(), json!({ "handBack": true }), None),
        )
        .wait()
        .await;
    let failure = failure(handed_back)?;
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
        "the script sees the rule's own error, whatever the crate made of it"
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
        "page": { "$neo": "page", "id": null },
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
    let input = json!({ "target": locator(&json!(page.id.0)), "strict": "strict" });
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
        "browseros-playwright rechecks right before it reloads, and nothing was sent: {}",
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
async fn a_field_typed_a_key_at_a_time_masks_its_final_value_and_no_prefix()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    // The owner's tab shows a prefix and the whole value, and every read's page facts come back
    // through the door's mask.
    let tab = harness
        .fake
        .open_tab("https://bank.example/", "hunte or hunter2?");
    let page = harness.listed(tab).await?;
    for value in ["h", "hu", "hun", "hunt", "hunte", "hunter", "hunter2"] {
        harness
            .hooks
            .field_entered(page.id.clone(), keystroke(value), TypedBy::NotAgent);
    }
    time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(250)).await;
    // Read by a run that begins now. A run going while the field was typed holds each prefix it
    // may have read, and its realm sees none of them (decision 35b), so only a later run shows
    // what the process's set learned.
    let (run, _guard) = harness.run()?;
    let reply = harness
        .submit(&run, "page.opener", json!({ "page": handle(&page) }))
        .await;
    assert_eq!(
        reply.pages.first().and_then(|fact| fact.title.clone()),
        Some("hunte or [private]?".to_owned()),
        "once the field went quiet its value is a secret, and `hunte`, a prefix, is still text"
    );
    Ok(())
}

#[tokio::test]
async fn a_read_between_two_keystrokes_registers_no_prefix() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let tab = harness
        .fake
        .open_tab("https://bank.example/", "hunt, hunte or hunter2?");
    let page = harness.listed(tab).await?;
    harness
        .hooks
        .field_entered(page.id.clone(), keystroke("hunt"), TypedBy::NotAgent);
    // A read sweeps the field before the journal reports the next key. The read masked what it
    // found out of its own result already.
    harness.hooks.values_swept(
        page.id.clone(),
        &FieldValues::new(["hunte".to_owned().into()]),
    );
    for value in ["hunter", "hunter2"] {
        harness
            .hooks
            .field_entered(page.id.clone(), keystroke(value), TypedBy::NotAgent);
    }
    time::sleep(Defaults::TYPED_QUIET + Duration::from_millis(250)).await;
    // A run that begins after the field went quiet holds none of its prefixes, so its read shows
    // what the process's set learned.
    let (run, _guard) = harness.run()?;
    let reply = harness
        .submit(&run, "page.opener", json!({ "page": handle(&page) }))
        .await;
    assert_eq!(
        reply.pages.first().and_then(|fact| fact.title.clone()),
        Some("hunt, hunte or [private]?".to_owned()),
        "once the field went quiet its value is a secret, and the prefix the read saw is not"
    );
    Ok(())
}
