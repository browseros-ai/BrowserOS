//! Methods that exist only to exercise the door, each written the way a built-in would be.
//! Those that never wait answer with a ready future: clippy refuses an `async fn` without an
//! await, even in a trait impl.

use std::future::{self, Future};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use browseros_core::pages::NewPageOptions;
use browseros_core::viewport::{ViewportMode, ViewportRequest};
use neo_trace::Effect;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Barrier, Notify};
use tokio::task;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

use super::harness::TabLock;
use crate::browser::PageRef;
use crate::dispatch::{
    ErrorCode, LeafError, Lifetime, Method, MethodSpec, OpCtx, OperationError, PageChange,
    PageScript, PageTarget, RegistryBuilder, RegistryError, Visibility,
};
use crate::test_support::FakeCdp;

/// What the harness shares with its methods. A plain data bag.
pub(super) struct Shared {
    pub(super) fake: Arc<FakeCdp>,
    pub(super) lock: Arc<TabLock>,
    pub(super) gate: Arc<Barrier>,
    pub(super) hung: Arc<Notify>,
}

/// Registers every test method, plus a `context.newPage` that opens the lazy default page.
pub(super) fn test_methods(
    registry: &mut RegistryBuilder,
    shared: &Shared,
) -> Result<(), RegistryError> {
    registry.define(Write)?;
    registry.define(MainWorldWrite)?;
    registry.define(ShortFrameRead)?;
    registry.define(Read)?;
    registry.define(TakeoverRead {
        lock: Arc::clone(&shared.lock),
    })?;
    registry.define(Sticky {
        lock: Arc::clone(&shared.lock),
    })?;
    registry.define(Panics)?;
    registry.define(Hang {
        hung: Arc::clone(&shared.hung),
    })?;
    registry.define(Hidden)?;
    registry.define(Typed)?;
    registry.define(Parent)?;
    registry.define(Navigate {
        fake: Arc::clone(&shared.fake),
    })?;
    registry.define(Close)?;
    registry.define(Lease)?;
    registry.define(LeaseChild)?;
    registry.define(NewPage)?;
    registry.define(Gate {
        barrier: Arc::clone(&shared.gate),
    })?;
    registry.define(Count)?;
    registry.define(Fill)?;
    registry.define(Budget)?;
    registry.define(Echo)?;
    registry.define(AnyInput)?;
    registry.define(MainWorldScript)?;
    registry.define(Pump)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OnPage {
    page: PageTarget,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}

/// A write that passes its recheck and sends.
struct Write;

impl Method for Write {
    type Input = OnPage;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.write", Effect::Write)
    }

    fn target(input: &OnPage) -> Option<&PageTarget> {
        Some(&input.page)
    }

    fn call(
        &self,
        op: &OpCtx<Self>,
        _input: OnPage,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(send(op).map(|()| json!({ "written": true })))
    }
}

/// A write whose effect is script in the page's main world, as sdk-playwright reports one
/// through `OpEnv::main_world`.
struct MainWorldWrite;

impl Method for MainWorldWrite {
    type Input = OnPage;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.main_world", Effect::Write)
    }

    fn target(input: &OnPage) -> Option<&PageTarget> {
        Some(&input.page)
    }

    fn call(
        &self,
        op: &OpCtx<Self>,
        _input: OnPage,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(send(op).map(|()| {
            op.mark_main_world();
            json!({ "ran": true })
        }))
    }
}

/// A frame-tree read that came back short, as sdk-playwright reports one through
/// `OpEnv::frames_incomplete`.
struct ShortFrameRead;

impl Method for ShortFrameRead {
    type Input = OnPage;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.frames_incomplete", Effect::Read)
    }

    fn target(input: &OnPage) -> Option<&PageTarget> {
        Some(&input.page)
    }

    fn call(
        &self,
        op: &OpCtx<Self>,
        _input: OnPage,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        op.mark_frames_incomplete();
        future::ready(Ok(json!({ "incomplete": true })))
    }
}

/// What a write does right before its side effect (STYLE.md P6).
fn send<M>(op: &OpCtx<M>) -> Result<(), LeafError> {
    op.recheck()?;
    op.mark_sent();
    Ok(())
}

/// Reads its page's tab id.
struct Read;

impl Method for Read {
    type Input = OnPage;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.read", Effect::Read)
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

/// A read during which the owner takes the tab with a lock that blocks reads.
struct TakeoverRead {
    lock: Arc<TabLock>,
}

impl Method for TakeoverRead {
    type Input = OnPage;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.takeover_read", Effect::Read)
    }

    fn target(input: &OnPage) -> Option<&PageTarget> {
        Some(&input.page)
    }

    fn call(
        &self,
        _op: &OpCtx<Self>,
        _input: OnPage,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        self.lock.reads.store(true, Ordering::SeqCst);
        future::ready(Ok(json!({ "text": "what the owner is typing" })))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StickyInput {
    page: PageTarget,
    /// Whether the lock lifts and a second recheck passes before the method fails.
    recover: bool,
}

/// A write whose recheck is refused, which then reports an error of its own, the way a crate
/// that maps refusals to its own error type would.
struct Sticky {
    lock: Arc<TabLock>,
}

impl Method for Sticky {
    type Input = StickyInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.sticky", Effect::Write)
    }

    fn target(input: &StickyInput) -> Option<&PageTarget> {
        Some(&input.page)
    }

    fn call(
        &self,
        op: &OpCtx<Self>,
        input: StickyInput,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(self.refused(op, input.recover))
    }
}

impl Sticky {
    fn refused(&self, op: &OpCtx<Self>, recover: bool) -> Result<Value, LeafError> {
        self.lock.writes.store(true, Ordering::SeqCst);
        if op.recheck().is_ok() {
            return Ok(json!({ "refused": false }));
        }
        if !recover {
            return Err(LeafError::from(OperationError::website(
                ErrorCode::CDP_ERROR,
                "the crate's own error for its refusal",
            )));
        }
        self.lock.writes.store(false, Ordering::SeqCst);
        send(op)?;
        Err(LeafError::definite(OperationError::website(
            ErrorCode::CDP_ERROR,
            "the page answered with an error",
        )))
    }
}

/// A method with a bug.
struct Panics;

impl Method for Panics {
    type Input = Nothing;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.panic", Effect::Read)
    }

    fn target(_input: &Nothing) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, _op: &OpCtx<Self>, _input: Nothing) -> Result<Value, LeafError> {
        // Mid-flight, as a real bug would be: after the op task first suspended.
        task::yield_now().await;
        panic!("a bug in a method");
    }
}

/// Sends, says so, then never answers: only the run's stop or its teardown ends it.
struct Hang {
    hung: Arc<Notify>,
}

impl Method for Hang {
    type Input = OnPage;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.hang", Effect::Write)
    }

    fn target(input: &OnPage) -> Option<&PageTarget> {
        Some(&input.page)
    }

    async fn call(&self, op: &OpCtx<Self>, _input: OnPage) -> Result<Value, LeafError> {
        send(op)?;
        self.hung.notify_one();
        future::pending().await
    }
}

/// Only Neo may call it.
struct Hidden;

impl Method for Hidden {
    type Input = Nothing;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            visibility: Visibility::Internal,
            ..MethodSpec::new("test.hidden", Effect::Read)
        }
    }

    fn target(_input: &Nothing) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        _op: &OpCtx<Self>,
        _input: Nothing,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(Ok(json!({ "hidden": true })))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TypedInput {
    outer: Outer,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Outer {
    inner: u32,
}

/// A nested input, so `invalid_input` has a field path to name.
struct Typed;

impl Method for Typed {
    type Input = TypedInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.typed", Effect::Read)
    }

    fn target(_input: &TypedInput) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        _op: &OpCtx<Self>,
        input: TypedInput,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(Ok(json!({ "inner": input.outer.inner })))
    }
}

/// A write whose child writes first, after which the parent fails without saying how.
struct Parent;

impl Method for Parent {
    type Input = OnPage;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.parent", Effect::Write)
    }

    fn target(input: &OnPage) -> Option<&PageTarget> {
        Some(&input.page)
    }

    async fn call(&self, op: &OpCtx<Self>, _input: OnPage) -> Result<Value, LeafError> {
        let page = op.page()?;
        op.invoke("test.write", json!({ "page": page }))
            .await
            .map_err(|failure| LeafError::from(failure.error))?;
        Err(LeafError::from(OperationError::neo(
            ErrorCode::INTERNAL_ERROR,
            "failed after its child wrote",
        )))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NavigateInput {
    page: PageTarget,
    url: String,
}

/// A navigation the browser has already made, reported twice.
struct Navigate {
    fake: Arc<FakeCdp>,
}

impl Method for Navigate {
    type Input = NavigateInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.navigate", Effect::Write)
    }

    fn target(input: &NavigateInput) -> Option<&PageTarget> {
        Some(&input.page)
    }

    fn call(
        &self,
        op: &OpCtx<Self>,
        input: NavigateInput,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(self.navigate(op, &input.url))
    }
}

impl Navigate {
    fn navigate(&self, op: &OpCtx<Self>, url: &str) -> Result<Value, LeafError> {
        let page = op.page()?;
        send(op)?;
        if !self.fake.navigate_tab(page.tab, url, "Next") {
            return Err(LeafError::definite(OperationError::neo(
                ErrorCode::PAGE_CLOSED,
                "the fake has no such tab",
            )));
        }
        op.report_page(page, PageChange::Navigated);
        op.report_page(page, PageChange::Navigated);
        Ok(json!({}))
    }
}

/// Closes its page through core.
struct Close;

impl Method for Close {
    type Input = OnPage;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.close", Effect::Write)
    }

    fn target(input: &OnPage) -> Option<&PageTarget> {
        Some(&input.page)
    }

    async fn call(&self, op: &OpCtx<Self>, _input: OnPage) -> Result<Value, LeafError> {
        let page = op.page()?;
        send(op)?;
        op.race(op.run().view().close_page(op.core_page()?.clone()))
            .await?;
        op.report_page(page, PageChange::Closed);
        Ok(json!({}))
    }
}

/// Holds its page's viewport lease while a viewport change queues for the page, then has a
/// child take the same lease. Core's page lock is write-preferring, so a child that took the
/// lease again would wait behind the queued change, which waits for this op: a deadlock.
struct Lease;

impl Method for Lease {
    type Input = OnPage;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.lease", Effect::Read)
    }

    fn target(input: &OnPage) -> Option<&PageTarget> {
        Some(&input.page)
    }

    async fn call(&self, op: &OpCtx<Self>, _input: OnPage) -> Result<Value, LeafError> {
        let page = op.page()?;
        let _viewport = op.viewport(page).await?;
        let request = ViewportRequest::new(ViewportMode::Mobile, None).map_err(|error| {
            LeafError::from(OperationError::neo(ErrorCode::INTERNAL_ERROR, error))
        })?;
        let view = op.run().view().clone();
        let id = op.core_page()?.clone();
        let change = tokio::spawn(async move {
            let cancel = CancellationToken::new();
            view.set_viewport(id, request, &cancel).await
        });
        // Long enough for the change to reach the page's lock and queue there.
        time::sleep(Duration::from_millis(50)).await;
        let child = op.invoke("test.lease_child", json!({ "page": page })).await;
        change.abort();
        child.map_err(|failure| LeafError::from(failure.error))
    }
}

/// Takes the page's lease from inside its parent.
struct LeaseChild;

impl Method for LeaseChild {
    type Input = OnPage;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.lease_child", Effect::Read)
    }

    fn target(input: &OnPage) -> Option<&PageTarget> {
        Some(&input.page)
    }

    async fn call(&self, op: &OpCtx<Self>, _input: OnPage) -> Result<Value, LeafError> {
        let _viewport = op.viewport(op.page()?).await?;
        Ok(json!({ "leased": true }))
    }
}

/// The run's page opener, as neo-playwright's `context.newPage` is: a background tab,
/// reported opened, returned as Playwright's page entry `{ page, url }`.
struct NewPage;

impl Method for NewPage {
    type Input = Nothing;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("context.newPage", Effect::Write)
    }

    fn target(_input: &Nothing) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, op: &OpCtx<Self>, _input: Nothing) -> Result<Value, LeafError> {
        send(op)?;
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

/// Returns only once every one of [`super::harness::GATE_PARTIES`] overlapping calls arrived.
struct Gate {
    barrier: Arc<Barrier>,
}

impl Method for Gate {
    type Input = Nothing;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.gate", Effect::Read)
    }

    fn target(_input: &Nothing) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, _op: &OpCtx<Self>, _input: Nothing) -> Result<Value, LeafError> {
        self.barrier.wait().await;
        Ok(json!({}))
    }
}

/// Per-tab state, as sdk-playwright keeps its facade.
#[derive(Default)]
struct Hits {
    count: AtomicU32,
}

/// Counts the ops on its tab in a tab slot, and names its own op.
struct Count;

impl Method for Count {
    type Input = OnPage;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.count", Effect::Read)
    }

    fn target(input: &OnPage) -> Option<&PageTarget> {
        Some(&input.page)
    }

    fn call(
        &self,
        op: &OpCtx<Self>,
        _input: OnPage,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        let counted = op.tab_slot::<Hits>().map(|hits| {
            let count = hits.count.fetch_add(1, Ordering::SeqCst) + 1;
            json!({ "count": count, "operation": op.id().to_string() })
        });
        future::ready(counted)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FillInput {
    page: PageTarget,
    #[expect(dead_code, reason = "only the trace reads it, from the raw input")]
    value: String,
    #[expect(dead_code, reason = "only the trace reads it, from the raw input")]
    secret: String,
}

/// Types into a field; the trace receives the full input independently of its field names.
struct Fill;

impl Method for Fill {
    type Input = FillInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.fill", Effect::Write)
    }

    fn target(input: &FillInput) -> Option<&PageTarget> {
        Some(&input.page)
    }

    fn call(
        &self,
        op: &OpCtx<Self>,
        _input: FillInput,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(send(op).map(|()| json!({})))
    }
}

/// What a method sees of its run's budget.
struct Budget;

impl Method for Budget {
    type Input = Nothing;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.budget", Effect::Read)
    }

    fn target(_input: &Nothing) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        op: &OpCtx<Self>,
        _input: Nothing,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        let left = op.deadline().saturating_duration_since(Instant::now());
        future::ready(Ok(json!({
            "secondsLeft": left.as_secs(),
            "cancelled": op.cancel_token().is_cancelled(),
        })))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EchoInput {
    text: String,
    /// Whether to fail with the text in the error message instead.
    fail: bool,
}

/// Hands its text back, in its value or in its error.
struct Echo;

impl Method for Echo {
    type Input = EchoInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.echo", Effect::Read)
    }

    fn target(_input: &EchoInput) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        _op: &OpCtx<Self>,
        input: EchoInput,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        let echoed = if input.fail {
            Err(LeafError::definite(OperationError::website(
                ErrorCode::CDP_ERROR,
                format_args!("the page said {}", input.text),
            )))
        } else {
            Ok(json!({ "text": input.text }))
        };
        future::ready(echoed)
    }
}

/// Hands back whatever input it gets. It has no input type of its own, so it runs on any JSON
/// the door lets through: whether it ran shows whether the door read an input as a call or as
/// a refusal brand.
struct AnyInput;

impl Method for AnyInput {
    type Input = Value;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec::new("test.any", Effect::Read)
    }

    fn target(_input: &Value) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        _op: &OpCtx<Self>,
        input: Value,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(Ok(input))
    }
}

/// `test.script`: a main-world script as `OpMarks` marks `page.evaluate`, answering its input,
/// so a test chooses what the script read of the page.
struct MainWorldScript;

impl Method for MainWorldScript {
    type Input = Value;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            page_script: PageScript::MainWorld,
            ..MethodSpec::new("test.script", Effect::Write)
        }
    }

    fn target(_input: &Value) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        _op: &OpCtx<Self>,
        input: Value,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(Ok(input))
    }
}

/// A background pull, as sdk-playwright's event pump: it never returns, and the run's end cuts
/// it without a line.
struct Pump;

impl Method for Pump {
    type Input = Nothing;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            lifetime: Lifetime::Background,
            ..MethodSpec::new("test.pump", Effect::Read)
        }
    }

    fn target(_input: &Nothing) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, _op: &OpCtx<Self>, _input: Nothing) -> Result<Value, LeafError> {
        future::pending().await
    }
}
