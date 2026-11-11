use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_core::NativeCapabilities;
use neo_captcha::CaptchaSolver;
use neo_trace::{RedactionLimits, Redactor, TraceSink, WriterConfig};
use tokio::time;
use tokio_util::sync::CancellationToken;

use super::hosted_run::Completion;
use super::{HostedError, HostedOptions, HostedRun};
use crate::App;
use crate::browser::{BrowserLink, TabRegistry};
use crate::builtins::{
    self, BuiltinDeps, CaptchaClaims, PlaywrightFacade, PrivacyHooks, TabJanitor,
};
use crate::config::Defaults;
use crate::dispatch::{Dispatcher, Observer, RegistryBuilder};
use crate::host::TakeoverLock;
use crate::run::{RunService, RunServiceParts};

/// The sidecar's dependency graph, with no HTTP, MCP, host socket or screenshot camera.
pub(super) struct HostedStack {
    dispatcher: Arc<Dispatcher>,
    service: Arc<RunService>,
    browser: Arc<BrowserLink>,
    tabs: Arc<TabRegistry>,
    trace: TraceSink,
    shutdown: CancellationToken,
    completions: Mutex<Vec<Completion>>,
}

impl HostedStack {
    pub(super) async fn start(
        options: HostedOptions,
        link: impl FnOnce(NativeCapabilities, Arc<TabRegistry>) -> Arc<BrowserLink>,
    ) -> Result<Arc<Self>, HostedError> {
        let config = options.config()?;
        let store = App::hosted_trace(&config.env.trace_dir())
            .await
            .map_err(HostedError::from_error)?;
        let redactor = Redactor::new(RedactionLimits::default());
        let trace = neo_trace::spawn_writer(
            &store,
            redactor,
            WriterConfig {
                replay_window: Defaults::REPLAY_WINDOW,
                ..WriterConfig::default()
            },
            None,
        );
        let tabs = Arc::new(TabRegistry::new());
        let native =
            NativeCapabilities::from_browseros_version(config.launch.browseros_version.as_deref());
        let browser = link(native, Arc::clone(&tabs));
        let hooks = Arc::new(PrivacyHooks::new(config.secret_files));
        let playwright = PlaywrightFacade::start(Arc::clone(&browser), Arc::clone(&hooks) as _);
        let solver = CaptchaSolver::new(config.captcha).map(Arc::new);
        let claims = Arc::new(CaptchaClaims::default());
        let mut methods = RegistryBuilder::default();
        builtins::builtin_set(
            &mut methods,
            BuiltinDeps {
                solver,
                claims: Arc::clone(&claims),
                playwright: Arc::clone(&playwright),
            },
        )
        .map_err(HostedError::from_error)?;
        let takeover = Arc::new(TakeoverLock::default());
        let dispatcher = Arc::new(Dispatcher::new(
            methods.build(),
            builtins::door_rules(&takeover, &claims),
            trace.clone(),
            Arc::clone(&browser),
            Arc::clone(&tabs),
            Observer::new(Arc::clone(&playwright) as _),
        ));
        let shutdown = CancellationToken::new();
        let service = RunService::new(RunServiceParts {
            dispatcher: Arc::clone(&dispatcher),
            browser: Arc::clone(&browser),
            tabs: Arc::clone(&tabs),
            store,
            trace: trace.clone(),
            shutdown: shutdown.clone(),
            orphan_grace: Defaults::ORPHAN_GRACE,
        });
        tokio::spawn(App::hosted_housekeeping(
            TabJanitor::new(
                Arc::clone(&dispatcher),
                Arc::clone(&browser),
                Arc::clone(&tabs),
                takeover,
            ),
            trace.clone(),
            shutdown.clone(),
        ));
        time::timeout(Duration::from_secs(30), async {
            browser.linked().await;
            playwright.get().await;
        })
        .await
        .map_err(|error| HostedError(format!("connecting to BrowserOS: {error}")))?;
        Ok(Arc::new(Self {
            dispatcher,
            service,
            browser,
            tabs,
            trace,
            shutdown,
            completions: Mutex::default(),
        }))
    }

    pub(super) async fn open_run(&self, timeout: Duration) -> Result<HostedRun, HostedError> {
        let mut run = {
            // Register before awaiting the initial listing. close cannot miss an opening run.
            let mut completions = self
                .completions
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if self.shutdown.is_cancelled() {
                return Err(HostedError("the hosted runtime is closed".to_owned()));
            }
            completions.retain(|completion| !completion.is_finished());
            let run = HostedRun::start(
                &self.dispatcher,
                &self.browser,
                &self.tabs,
                self.shutdown.clone(),
                timeout,
            )?;
            completions.push(run.completion());
            run
        };
        run.initialize(&self.service, &self.browser, &self.tabs)
            .await;
        Ok(run)
    }

    pub(super) fn stop(&self) {
        self.shutdown.cancel();
    }

    pub(super) async fn close(&self) -> Result<(), HostedError> {
        self.stop();
        let completions = self
            .completions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for completion in completions {
            completion.wait().await?;
        }
        time::timeout(Defaults::SHUTDOWN_FLUSH, self.trace.flush())
            .await
            .map_err(HostedError::from_error)?
            .map_err(HostedError::from_error)
    }
}
