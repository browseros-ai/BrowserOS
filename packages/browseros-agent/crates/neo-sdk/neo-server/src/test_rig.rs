//! What one run needs, on the fake browser with a real trace store in a temp dir: the rig
//! runtime/'s and run/'s tests share. The dispatcher's own tests keep their harness
//! (dispatch/tests/harness.rs), which exercises the door one rule at a time.

use std::error::Error;
use std::future::{self, Future};
use std::panic;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use browseros_core::NativeCapabilities;
use browseros_core::pages::NewPageOptions;
use neo_service::Settings;
use neo_trace::{
    Effect, RedactionLimits, Redactor, RunId, TaskId, TraceSink, TraceStore, WriterConfig,
    spawn_writer,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::runtime::Handle;
use tokio::sync::Barrier;
use tokio::task;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::browser::{BrowserLink, PageRef, TabRegistry};
use crate::builtins;
use crate::dispatch::{
    Dispatcher, ErrorCode, LeafError, Lifetime, Method, MethodSpec, Observer, OpCtx,
    OperationError, PageChange, PageTarget, RegistryBuilder, RegistryError, RunCtx, RunGuard,
    RunStart,
};
use crate::run::{RunService, RunServiceParts};
use crate::test_support::FakeCdp;

/// A test's error. `Send`, so a test body can run as a task on Neo's runtime.
pub(crate) type TestError = Box<dyn Error + Send + Sync>;

/// The parts of one Neo process, minus HTTP. A plain data bag.
pub(crate) struct TestRig {
    pub(crate) fake: Arc<FakeCdp>,
    pub(crate) link: Arc<BrowserLink>,
    pub(crate) tabs: Arc<TabRegistry>,
    pub(crate) store: TraceStore,
    pub(crate) sink: TraceSink,
    pub(crate) settings: Settings,
    /// Where `test.gate` waits for the test, so the test acts between two of a script's ops.
    pub(crate) gate: Arc<Barrier>,
    pub(crate) dispatcher: Arc<Dispatcher>,
    /// What SIGTERM would cancel.
    pub(crate) shutdown: CancellationToken,
    /// Holds the store's file for the rig's life.
    pub(crate) dir: TempDir,
}

/// Runs `test` as a task on Neo's own runtime (worker threads with 8 MiB stacks, as `main`
/// builds it), so QuickJS and the SDK call chain get the stacks they get in production instead
/// of the 2 MiB test thread. A panic in the test is resumed, so it fails as a test panic.
pub(crate) fn on_neo_runtime<F>(test: F) -> Result<(), TestError>
where
    F: Future<Output = Result<(), TestError>> + Send + 'static,
{
    let runtime = App::runtime()?;
    runtime.block_on(async {
        match tokio::spawn(test).await {
            Ok(result) => result,
            Err(error) if error.is_panic() => panic::resume_unwind(error.into_panic()),
            Err(error) => Err(error.into()),
        }
    })
}

/// `work`, or an error once `limit` passes: a hang fails the test instead of stalling it.
pub(crate) async fn within<T>(
    limit: Duration,
    work: impl Future<Output = T>,
) -> Result<T, TestError> {
    time::timeout(limit, work)
        .await
        .map_err(|error| format!("not within {limit:?}: {error}").into())
}

impl TestRig {
    /// A linked browser with target discovery on, a fresh store, and the test methods.
    pub(crate) async fn new() -> Result<Self, TestError> {
        Self::with_writer(WriterConfig::default()).await
    }

    /// As [`TestRig::new`], with the trace writer configured, e.g. a replay window that has
    /// already passed.
    pub(crate) async fn with_writer(writer: WriterConfig) -> Result<Self, TestError> {
        let fake = FakeCdp::new();
        let tabs = Arc::new(TabRegistry::new());
        let connect = Arc::clone(&fake);
        let link = BrowserLink::start_with(
            NativeCapabilities::default(),
            Arc::clone(&tabs),
            move || future::ready(connect.connect()),
        );
        within(Duration::from_secs(5), async {
            while !link.is_ready() || fake.count("Target.setDiscoverTargets") == 0 {
                time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?;
        let dir = tempfile::tempdir()?;
        let store = open_store(dir.path().join("trace.db")).await?;
        let redactor = Redactor::new(RedactionLimits::default());
        let sink = spawn_writer(&store, redactor, writer, None);
        let settings = Settings::defaults(Default::default());
        settings.watch({
            let sink = sink.clone();
            move |settings| sink.record_policy(settings.trace_policy())
        });
        let gate = Arc::new(Barrier::new(2));
        let dispatcher = dispatcher(&link, &tabs, &sink, &gate)?;
        Ok(Self {
            fake,
            link,
            tabs,
            store,
            sink,
            settings,
            gate,
            dispatcher,
            shutdown: CancellationToken::new(),
            dir,
        })
    }

    /// The run service over this rig.
    pub(crate) fn run_service(&self, orphan_grace: Duration) -> Arc<RunService> {
        self.service_over(
            Arc::clone(&self.dispatcher),
            Arc::clone(&self.tabs),
            orphan_grace,
        )
    }

    /// A run service as Neo has it after a restart: no memory of tasks or requests, over the
    /// same browser and the same trace.
    pub(crate) fn restarted_service(
        &self,
        orphan_grace: Duration,
    ) -> Result<Arc<RunService>, TestError> {
        let tabs = Arc::new(TabRegistry::new());
        let dispatcher = dispatcher(&self.link, &tabs, &self.sink, &self.gate)?;
        Ok(self.service_over(dispatcher, tabs, orphan_grace))
    }

    fn service_over(
        &self,
        dispatcher: Arc<Dispatcher>,
        tabs: Arc<TabRegistry>,
        orphan_grace: Duration,
    ) -> Arc<RunService> {
        RunService::new(RunServiceParts {
            settings: self.settings.clone(),
            dispatcher,
            browser: Arc::clone(&self.link),
            tabs,
            store: self.store.clone(),
            trace: self.sink.clone(),
            shutdown: self.shutdown.clone(),
            orphan_grace,
        })
    }

    /// A tab the owner opened, as core names it.
    pub(crate) async fn owner_tab(&self, url: &str) -> Result<PageRef, TestError> {
        self.owner_tab_titled(url, "Owner").await
    }

    /// A tab the owner opened whose document is titled `title`, as core names it.
    pub(crate) async fn owner_tab_titled(
        &self,
        url: &str,
        title: &str,
    ) -> Result<PageRef, TestError> {
        let tab = self.fake.open_tab(url, title);
        let run = self.link.view().ok_or("the link is up")?;
        let listing = run.view.list_pages().await?;
        let info = listing
            .pages
            .iter()
            .find(|page| page.tab_id == tab)
            .ok_or("core lists the fake's tab")?;
        Ok(PageRef::from(info))
    }

    /// A recorded run with `budget` left and no default page yet.
    pub(crate) fn run(
        &self,
        budget: Duration,
        task_id: Option<TaskId>,
    ) -> Result<(Arc<RunCtx>, RunGuard), TestError> {
        let view = self.link.view().ok_or("the link is up")?;
        let start = RunStart {
            id: RunId::new(),
            code: "test".to_owned(),
            request_id: None,
            task_id,
            host_data: None,
            view,
            deadline: Instant::now() + budget,
            default_page: None,
        };
        Ok(RunCtx::start(&self.dispatcher, start))
    }
}

/// Opens the store on a blocking thread. `TraceStore::open` runs sqlx transactions whose futures
/// the compiler cannot prove `Send` inside a spawned test body, though they are; `main` opens it
/// under `block_on`, which needs no `Send`, and this does the same off the test's task.
pub(crate) async fn open_store(path: PathBuf) -> Result<TraceStore, TestError> {
    let runtime = Handle::current();
    let opened = task::spawn_blocking(move || runtime.block_on(TraceStore::open(&path))).await?;
    Ok(opened?)
}

fn dispatcher(
    link: &Arc<BrowserLink>,
    tabs: &Arc<TabRegistry>,
    sink: &TraceSink,
    gate: &Arc<Barrier>,
) -> Result<Arc<Dispatcher>, RegistryError> {
    let mut registry = RegistryBuilder::default();
    test_methods(&mut registry, gate)?;
    builtins::register_tasks(&mut registry)?;
    Ok(Arc::new(Dispatcher::new(
        registry.build(),
        Vec::new(),
        sink.clone(),
        Arc::clone(link),
        Arc::clone(tabs),
        Observer::unrecorded(),
    )))
}

/// The methods the rig's runs can call, each written the way a built-in would be.
fn test_methods(registry: &mut RegistryBuilder, gate: &Arc<Barrier>) -> Result<(), RegistryError> {
    registry.define(Echo)?;
    registry.define(Title)?;
    registry.define(Gate(Arc::clone(gate)))?;
    registry.define(Fail)?;
    registry.define(SlowAnswer)?;
    registry.define(Hang)?;
    registry.define(Abortable)?;
    registry.define(Pull)?;
    registry.define(Touch)?;
    registry.define(NewPage)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OnPage {
    page: PageTarget,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}

/// `test.echo`: hands its input back. A `pumped` count orders the answer after that many
/// event-pump replies (`OpCtx::answer_after_pump`), and `"fail": true` answers an error instead.
struct Echo;

impl Method for Echo {
    type Input = Value;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.echo", Effect::Read)
    }

    fn target(_input: &Value) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        op: &OpCtx<Self>,
        input: Value,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        if let Some(note) = input.get("note").and_then(Value::as_str) {
            op.note(note.to_owned());
        }
        if let Some(pumped) = input.get("pumped").and_then(Value::as_u64) {
            op.answer_after_pump(pumped);
        }
        if input.get("fail") == Some(&Value::Bool(true)) {
            return future::ready(Err(LeafError::definite(OperationError::website(
                ErrorCode::CDP_ERROR,
                "the echo failed",
            ))));
        }
        future::ready(Ok(input))
    }
}

/// `test.title`: its page's title, read from the browser, as `page.title()` reads the document's.
struct Title;

impl Method for Title {
    type Input = OnPage;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.title", Effect::Read)
    }

    fn target(input: &OnPage) -> Option<&PageTarget> {
        Some(&input.page)
    }

    async fn call(&self, op: &OpCtx<Self>, _input: OnPage) -> Result<Value, LeafError> {
        let page = op.page()?.clone();
        let listing = op.race(Box::pin(op.run().view().list_pages())).await?;
        let title = listing
            .pages
            .into_iter()
            .find(|info| info.tab_id == page.tab)
            .map(|info| info.title);
        Ok(json!(title))
    }
}

/// `test.gate`: waits until the test meets it at the rig's barrier. Its input is any JSON, kept
/// only by its record, so a test can read a running op's start row.
struct Gate(Arc<Barrier>);

impl Method for Gate {
    type Input = Value;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.gate", Effect::Read)
    }

    fn target(_input: &Value) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, _op: &OpCtx<Self>, _input: Value) -> Result<Value, LeafError> {
        self.0.wait().await;
        Ok(Value::Null)
    }
}

/// `test.fail`: the page's definite refusal.
struct Fail;

impl Method for Fail {
    type Input = Nothing;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.fail", Effect::Read)
    }

    fn target(_input: &Nothing) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        _op: &OpCtx<Self>,
        _input: Nothing,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(Err(LeafError::definite(OperationError::website(
            ErrorCode::CDP_ERROR,
            "the page said no",
        ))))
    }
}

/// `test.slowAnswer`: a browser call that timed out, which scripts see as a `TimeoutError`.
struct SlowAnswer;

impl Method for SlowAnswer {
    type Input = Nothing;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.slowAnswer", Effect::Read)
    }

    fn target(_input: &Nothing) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        _op: &OpCtx<Self>,
        _input: Nothing,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(Err(LeafError::definite(
            OperationError::neo(ErrorCode::CDP_TIMEOUT, "the browser did not answer in time")
                .timed_out(),
        )))
    }
}

/// `test.hang`: a write that is sent and never answers, until its run stops.
struct Hang;

impl Method for Hang {
    type Input = Nothing;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.hang", Effect::Write)
    }

    fn target(_input: &Nothing) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, op: &OpCtx<Self>, _input: Nothing) -> Result<Value, LeafError> {
        op.recheck()?;
        op.mark_sent();
        future::pending().await
    }
}

/// `test.abortable`: waits until the script aborts it (Playwright's `signal`), then fails as a
/// Playwright op's timeout path reports an abort: code `aborted`, the reason as its message.
struct Abortable;

impl Method for Abortable {
    type Input = Nothing;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.abortable", Effect::Read)
    }

    fn target(_input: &Nothing) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, op: &OpCtx<Self>, _input: Nothing) -> Result<Value, LeafError> {
        op.aborted().await;
        let reason = op.abort_reason().unwrap_or_default();
        Err(LeafError::ambiguous(OperationError::script(
            ErrorCode::ABORTED,
            reason,
        )))
    }
}

/// `test.pull`: a background pull, as neo-playwright's event pump is. It answers only when
/// something happens, and here nothing does, so the run's end is what ends it.
struct Pull;

impl Method for Pull {
    type Input = Nothing;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            lifetime: Lifetime::Background,
            ..MethodSpec::new("test.pull", Effect::Read)
        }
    }

    fn target(_input: &Nothing) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, _op: &OpCtx<Self>, _input: Nothing) -> Result<Value, LeafError> {
        future::pending().await
    }
}

/// `test.touch`: reads its page's tab id, so the run's task remembers that tab.
struct Touch;

impl Method for Touch {
    type Input = OnPage;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.touch", Effect::Read)
    }

    fn target(input: &OnPage) -> Option<&PageTarget> {
        Some(&input.page)
    }

    fn call(
        &self,
        op: &OpCtx<Self>,
        _input: OnPage,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(op.page().map(|page| json!({ "tab": page.tab.0 })))
    }
}

/// `context.newPage`, as neo-playwright's op opens one: a background tab, reported
/// opened, returned as Playwright's page entry `{ page, url }`. The prelude sends its call
/// header; the door calls it with `{}` to open a run's lazy default page.
struct NewPage;

/// `context.newPage`'s input: the prelude's call header, which this double does not read.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewPageInput {
    #[serde(default, rename = "call")]
    _call: Option<Value>,
}

impl Method for NewPage {
    type Input = NewPageInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("context.newPage", Effect::Write)
    }

    fn target(_input: &NewPageInput) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, op: &OpCtx<Self>, _input: NewPageInput) -> Result<Value, LeafError> {
        op.recheck()?;
        op.mark_sent();
        let options = NewPageOptions {
            background: Some(true),
            ..NewPageOptions::default()
        };
        // Boxed: core's page open is a large future.
        let info = op
            .race(Box::pin(op.run().view().new_page("about:blank", options)))
            .await?;
        let page = PageRef::from(&info);
        op.report_page(&page, PageChange::Opened);
        Ok(json!({ "page": page, "url": info.url }))
    }
}
