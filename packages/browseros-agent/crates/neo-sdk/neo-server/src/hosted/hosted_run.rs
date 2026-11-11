use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use neo_trace::RunId;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

use super::HostedError;
use crate::browser::{BrowserLink, PageRef, TabRegistry};
use crate::config::Defaults;
use crate::dispatch::{Dispatcher, RunCtx, RunGuard, RunStart, StopReason};
use crate::run::{RunReply, RunService};
use crate::runtime::{Bridge, ScriptEnd, ScriptOutcome};

/// One external JS execution. The host keeps it alive until `finish`; dropping it cancels it.
pub struct HostedRun {
    context: Arc<RunCtx>,
    bridge: Arc<Bridge>,
    executor: Handle,
    initial_page: Option<PageRef>,
    init: String,
    completion: Completion,
}

/// A waiter does not own the public run, so an abandoned JS object can still cancel its work.
#[derive(Clone)]
pub(super) struct Completion {
    reply: watch::Receiver<Option<Arc<RunReply>>>,
}

impl Completion {
    pub(super) fn is_finished(&self) -> bool {
        self.reply.borrow().is_some()
    }

    pub(super) async fn wait(mut self) -> Result<Arc<RunReply>, HostedError> {
        loop {
            if let Some(reply) = self.reply.borrow_and_update().as_ref() {
                return Ok(Arc::clone(reply));
            }
            self.reply
                .changed()
                .await
                .map_err(HostedError::from_error)?;
        }
    }
}

impl HostedRun {
    pub(super) fn start(
        dispatcher: Arc<Dispatcher>,
        browser: &BrowserLink,
        tabs: &TabRegistry,
        shutdown: CancellationToken,
        timeout: Duration,
    ) -> Result<Self, HostedError> {
        let view = browser.view().ok_or_else(|| {
            HostedError("the browser is not connected; retry once it reconnects".to_owned())
        })?;
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| HostedError("the run timeout is too large".to_owned()))?;
        // RunService::default_page(None, ..) is exactly this lookup: no task history to read.
        // `initialize` touches it, as `RunService::execute` does, once it can ask core.
        let page = tabs.default_page(None);
        let (context, guard) = RunCtx::start(
            &dispatcher,
            RunStart {
                id: RunId::new(),
                code: "/* JavaScript supplied by the hosted-run caller */".to_owned(),
                request_id: None,
                task_id: None,
                host_data: None,
                view,
                deadline,
                default_page: page.clone(),
            },
        );
        let bridge = Arc::new(Bridge::new(Arc::clone(&context), Arc::clone(&dispatcher)));
        let (sender, reply) = watch::channel(None);
        // Own the guard in an independent task: dropping a finish future cannot lose the end
        // record, and the deadline still fires while Node is busy or no longer references us.
        tokio::spawn(Self::complete(
            Arc::clone(&context),
            Arc::clone(&bridge),
            dispatcher,
            guard,
            shutdown,
            sender,
        ));
        Ok(Self {
            context,
            bridge,
            executor: Handle::current(),
            initial_page: page,
            init: String::new(),
            completion: Completion { reply },
        })
    }

    /// The async half of `RunService::execute`'s run start. The default page is touched only
    /// once core confirms it is open, under a liveness ticket taken just before asking, so the
    /// idle-tab janitor leaves it alone without a closed tab being revived. The init carries the
    /// same masked listing and observation `run_script` hands QuickJS.
    pub(super) async fn initialize(
        &mut self,
        service: &RunService,
        browser: &BrowserLink,
        tabs: &TabRegistry,
    ) {
        if let Some(page) = &self.initial_page {
            let seen = tabs.seen();
            if browser.page(page.tab).await.is_some() {
                tabs.touched(page, seen, None);
            }
        }
        let facts = service.hosted_open_pages(&self.context).await;
        self.init = json!({
            "page": self.initial_page,
            "pages": facts.pages,
            "observed": facts.observed,
        })
        .to_string();
    }

    /// The run's trace identity.
    #[must_use]
    pub fn id(&self) -> String {
        self.context.id().to_string()
    }

    /// The product prelude's `RunInit` JSON: default page and masked facts for every open tab.
    #[must_use]
    pub fn init(&self) -> &str {
        &self.init
    }

    /// Admits at this call, in JS call order. Entering Tokio only supplies a spawn context;
    /// neither admission nor returning this future waits for the browser. Always an envelope.
    pub fn invoke(
        &self,
        name: String,
        input_json: &str,
        site: &str,
    ) -> impl Future<Output = String> + Send + use<> {
        let _entered = self.executor.enter();
        let pending = self.bridge.submit(name, input_json, site);
        let bridge = Arc::clone(&self.bridge);
        async move { bridge.answer(pending).await }
    }

    /// Records one console line through the product's masking and log limits.
    pub fn log(&self, level: &str, text: String) {
        self.bridge.log(level, text);
    }

    /// The bootstrap's clock, bounded by this run's stop and deadline.
    pub fn sleep(&self, milliseconds: f64) -> impl Future<Output = ()> + Send + use<> {
        let bridge = Arc::clone(&self.bridge);
        async move { bridge.sleep(milliseconds).await }
    }

    /// Stops admission now, then waits for operation drain and the end record. Repeated and
    /// concurrent calls return exactly the same reply; dropping the future does not stop cleanup.
    pub fn finish(&self) -> impl Future<Output = Result<String, HostedError>> + Send + use<> {
        self.context.stop(StopReason::ScriptReturned);
        let completion = self.completion.clone();
        async move {
            let reply = completion.wait().await?;
            serde_json::to_string(&reply).map_err(HostedError::from_error)
        }
    }

    pub(super) fn completion(&self) -> Completion {
        self.completion.clone()
    }

    async fn complete(
        context: Arc<RunCtx>,
        bridge: Arc<Bridge>,
        dispatcher: Arc<Dispatcher>,
        guard: RunGuard,
        shutdown: CancellationToken,
        sender: watch::Sender<Option<Arc<RunReply>>>,
    ) {
        tokio::select! {
            () = context.cancel_token().cancelled() => {}
            () = time::sleep_until(context.deadline()) => context.stop(StopReason::Deadline),
            () = shutdown.cancelled() => context.stop(StopReason::Shutdown),
        }
        context.stop(StopReason::ScriptReturned);
        let mut ops = context.close_ops();
        let drained = time::timeout(Defaults::TEARDOWN_GRACE, async {
            while let Some(joined) = ops.join_next().await {
                if let Err(error) = joined
                    && error.is_panic()
                {
                    tracing::error!(%error, "a hosted operation panicked; its guard settled it");
                }
            }
        })
        .await;
        if drained.is_err() {
            ops.shutdown().await;
        }
        let end = ScriptEnd {
            outcome: ScriptOutcome::Returned(Value::Null),
            logs: bridge.take_logs(),
        };
        let held = context.held_values();
        let mut reply = RunReply::build(&context, end, dispatcher.secrets());
        reply.mask_held(&held);
        guard.finish(reply.trace_end(None), held.set());
        sender.send_replace(Some(Arc::new(reply)));
    }
}

impl Drop for HostedRun {
    fn drop(&mut self) {
        self.context.stop(StopReason::CallerLeft);
    }
}

impl fmt::Debug for HostedRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostedRun")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}
