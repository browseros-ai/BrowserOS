//! `HostCtx`: everything the shared MCP layer reads from its host, built once per process
//! and shared by every MCP transport session. It replaces claw's `AppState` inside the
//! MCP layer, so the layer never sees a database, analytics SDK or cockpit directly.

use crate::{
    admission::{LocalToolProvider, SessionSpec},
    branding::HostBranding,
    browser::BrowserService,
    error::HostResult,
    mcp::{
        dispatch::{DEFAULT_EFFECTS, DEFAULT_GUARDS},
        effects::tab_groups::{
            apply_agent_tab_group_title, close_agent_tab_group, collapse_agent_tab_group,
        },
        name_session::NameSessionTool,
        naming::normalize_small_name,
    },
    noop::{NoopBrowserSessionObserver, NoopProfiles, NoopSessionJournal, NoopTabJournal},
    seams::{
        DispatchEffect, DispatchGuard, DispatchObserver, HostAnalytics, LocalTool,
        ProfileDirectory, SessionJournal, TabJournal,
    },
    sessions::{RetainedGroupAction, Session, Sessions, TargetBindings},
    storage::JsonStore,
};
use browseros_core::NativeCapabilities;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// The stateful core a host owns and may share with its own surfaces (claw's cockpit
/// reads the same `Sessions` and `BrowserService`).
pub struct HostCore {
    pub browser: Arc<BrowserService>,
    pub sessions: Arc<Sessions>,
    /// Persisted per-task browser target, linked into `sessions` for Stop and sweeps.
    pub targets: Arc<TargetBindings>,
    /// Root of the code-mode helper store (`<root>/helpers/<host>/<name>.md`).
    pub helpers_root: PathBuf,
}

/// What a host without its own journal, recordings or cockpit needs to bring up a core.
#[derive(Debug, Clone)]
pub struct HostCoreConfig {
    pub cdp_port: u16,
    /// Owns `target-bindings.json` and the helper store.
    pub data_root: PathBuf,
    /// Launch-config identity of the browser on `cdp_port`; gates profile tools.
    pub native: NativeCapabilities,
    pub session_idle: Duration,
    pub session_retention: Duration,
    pub session_sweep_interval: Duration,
}

impl HostCore {
    /// Builds a core with no-op journal, analytics and session observer.
    pub async fn open(config: HostCoreConfig) -> HostResult<Self> {
        tokio::fs::create_dir_all(&config.data_root)
            .await
            .map_err(|source| crate::HostError::Io {
                path: Some(config.data_root.clone()),
                source,
            })?;
        let targets = TargetBindings::load(JsonStore::new(config.data_root.clone())).await?;
        let sessions = Sessions::new(
            Arc::new(NoopSessionJournal),
            config.session_idle,
            config.session_retention,
            config.session_sweep_interval,
        );
        sessions.set_target_bindings(targets.clone());
        let browser = BrowserService::new(
            config.cdp_port,
            sessions.ownership(),
            Arc::new(NoopBrowserSessionObserver),
            config.native,
        );
        Ok(Self {
            browser,
            sessions,
            targets,
            helpers_root: config.data_root,
        })
    }
}

pub struct HostCtx {
    pub(crate) browser: Arc<BrowserService>,
    pub(crate) sessions: Arc<Sessions>,
    pub(crate) targets: Arc<TargetBindings>,
    pub(crate) helpers_root: PathBuf,
    pub(crate) branding: HostBranding,
    pub(crate) tabs: Arc<dyn TabJournal>,
    pub(crate) profiles: Arc<dyn ProfileDirectory>,
    /// `name_session` first, then the host's tools in registration order; this is also
    /// their `tools/list` order after the catalog.
    pub(crate) local_tools: Vec<RegisteredLocalTool>,
    /// Runtime tool sources, listed after `local_tools` in registration order.
    pub(crate) providers: Vec<Arc<dyn LocalToolProvider>>,
    /// Complete ordered pipeline: built-ins first, host additions after.
    pub(crate) guards: Vec<Arc<dyn DispatchGuard>>,
    pub(crate) effects: Vec<Arc<dyn DispatchEffect>>,
    pub(crate) observers: Vec<Arc<dyn DispatchObserver>>,
}

impl HostCtx {
    #[must_use]
    pub fn builder(core: HostCore) -> HostCtxBuilder {
        HostCtxBuilder {
            core,
            branding: HostBranding::browseros(env!("CARGO_PKG_VERSION")),
            tabs: Arc::new(NoopTabJournal),
            profiles: Arc::new(NoopProfiles),
            local_tools: Vec::new(),
            providers: Vec::new(),
            guards: Vec::new(),
            effects: Vec::new(),
            observers: Vec::new(),
        }
    }

    #[must_use]
    pub fn browser(&self) -> &Arc<BrowserService> {
        &self.browser
    }

    #[must_use]
    pub fn sessions(&self) -> &Arc<Sessions> {
        &self.sessions
    }

    #[must_use]
    pub fn targets(&self) -> &Arc<TargetBindings> {
        &self.targets
    }

    #[must_use]
    pub fn branding(&self) -> &HostBranding {
        &self.branding
    }

    /// Root of the code-mode helper store.
    #[must_use]
    pub fn helpers_root(&self) -> &std::path::Path {
        &self.helpers_root
    }

    /// The process-fixed native identity; gates profile tools and instructions.
    #[must_use]
    pub fn native(&self) -> NativeCapabilities {
        self.browser.native()
    }

    pub(crate) fn journal(&self) -> &Arc<dyn SessionJournal> {
        self.sessions.journal()
    }

    pub(crate) fn analytics(&self) -> &Arc<dyn HostAnalytics> {
        self.sessions.analytics()
    }

    /// A registered local tool by name (`name_session` or one the host added).
    #[must_use]
    pub fn local_tool(&self, name: &str) -> Option<Arc<dyn LocalTool>> {
        self.find_local_tool(name).map(|entry| entry.tool.clone())
    }

    pub(crate) fn find_local_tool(&self, name: &str) -> Option<&RegisteredLocalTool> {
        self.local_tools
            .iter()
            .find(|entry| entry.definition.name == name)
    }

    /// Names of the dispatch effects in run order (built-ins, then the host's).
    #[must_use]
    pub fn effect_names(&self) -> Vec<&'static str> {
        self.effects.iter().map(|effect| effect.name()).collect()
    }

    /// Names of the dispatch observers in run order.
    #[must_use]
    pub fn observer_names(&self) -> Vec<&'static str> {
        self.observers
            .iter()
            .map(|observer| observer.name())
            .collect()
    }

    #[must_use]
    pub fn guard_count(&self) -> usize {
        self.guards.len()
    }

    /// The live session bound to `spec`, minted (or re-minted after it ended) under the
    /// spec's deterministic id. Idempotent, so an admission hook may call it per request.
    /// A changed `label` renames the session and retitles its tab group. A session stopped
    /// through `Sessions::cancel_by_session` (a cockpit Stop) stays stopped: a re-mint
    /// answers, but its browser target refuses execution, as Stop promises.
    pub async fn bind_session(&self, spec: &SessionSpec) -> HostResult<Arc<Session>> {
        let profiles = self.profiles.profiles().await?;
        let agent = crate::identity::ClientIdentity::resolve(&spec.client, &profiles);
        let (session, _minted) = self
            .sessions
            .bind(spec.session_id(), agent, &spec.key, spec.client.clone())
            .await?;
        if let Some(label) = spec.label.as_deref().map(normalize_small_name)
            && !label.is_empty()
            && session.label().await != label
        {
            session.rename(label).await;
            let browser = self.browser.session().await;
            apply_agent_tab_group_title(
                browser.as_ref(),
                &self.sessions.ownership(),
                session.convo_id(),
                session.as_ref(),
                session.child_token(),
            )
            .await;
        }
        Ok(session)
    }

    /// Ends the session bound to `spec` the way an idle sweep would: dispatches drain, its
    /// groups are retained (collapsed) until retention. Returns whether one was live.
    pub async fn end_session(&self, spec: &SessionSpec, reason: &str) -> HostResult<bool> {
        self.sessions
            .remove(&spec.session_id(), "closed", Some(reason))
            .await
    }

    /// Starts the browser reattach loop and the idle-session sweeper. claw starts both
    /// itself, inside its ordered runtime; a lean host calls this once after `build`.
    #[must_use]
    pub fn spawn_background(&self, cancel: CancellationToken) -> Vec<JoinHandle<()>> {
        vec![
            self.browser.start(),
            self.sessions.clone().spawn_idle_sweeper(cancel),
        ]
    }

    /// Stops reattaching and tears every live session down (end rows, retained groups).
    pub async fn shutdown(&self) -> HostResult<usize> {
        self.browser.stop();
        self.sessions.shutdown().await
    }
}

/// A local tool with its definition read once at build: definitions are fixed for the
/// process, and lookups by name happen on every `tools/call`.
pub(crate) struct RegisteredLocalTool {
    pub(crate) definition: rmcp::model::Tool,
    pub(crate) tool: Arc<dyn LocalTool>,
}

pub struct HostCtxBuilder {
    core: HostCore,
    branding: HostBranding,
    tabs: Arc<dyn TabJournal>,
    profiles: Arc<dyn ProfileDirectory>,
    local_tools: Vec<Arc<dyn LocalTool>>,
    providers: Vec<Arc<dyn LocalToolProvider>>,
    guards: Vec<Arc<dyn DispatchGuard>>,
    effects: Vec<Arc<dyn DispatchEffect>>,
    observers: Vec<Arc<dyn DispatchObserver>>,
}

impl HostCtxBuilder {
    #[must_use]
    pub fn branding(mut self, branding: HostBranding) -> Self {
        self.branding = branding;
        self
    }

    #[must_use]
    pub fn tab_journal(mut self, tabs: Arc<dyn TabJournal>) -> Self {
        self.tabs = tabs;
        self
    }

    #[must_use]
    pub fn profiles(mut self, profiles: Arc<dyn ProfileDirectory>) -> Self {
        self.profiles = profiles;
        self
    }

    /// Adds a tool after `name_session`; a later tool with an existing name is ignored
    /// by lookup, so register each name once.
    #[must_use]
    pub fn local_tool(mut self, tool: Arc<dyn LocalTool>) -> Self {
        self.local_tools.push(tool);
        self
    }

    /// Adds a runtime tool source, listed after the local tools.
    #[must_use]
    pub fn tool_provider(mut self, provider: Arc<dyn LocalToolProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    #[must_use]
    pub fn guard(mut self, guard: Arc<dyn DispatchGuard>) -> Self {
        self.guards.push(guard);
        self
    }

    #[must_use]
    pub fn effect(mut self, effect: Arc<dyn DispatchEffect>) -> Self {
        self.effects.push(effect);
        self
    }

    #[must_use]
    pub fn observer(mut self, observer: Arc<dyn DispatchObserver>) -> Self {
        self.observers.push(observer);
        self
    }

    /// Finishes the context and wires the parts of the core that depend on it: the
    /// retained-group hook (collapse/close a finished task's groups through the live
    /// browser) and the product name agent-facing target errors use.
    #[must_use]
    pub fn build(self) -> Arc<HostCtx> {
        let HostCore {
            browser,
            sessions,
            targets,
            helpers_root,
        } = self.core;
        install_retained_group_hook(&sessions, &browser);
        targets.set_product_name(&self.branding.product_name);
        let local_tools = std::iter::once(Arc::new(NameSessionTool) as Arc<dyn LocalTool>)
            .chain(self.local_tools)
            .map(|tool| RegisteredLocalTool {
                definition: tool.definition(),
                tool,
            })
            .collect();
        let mut guards: Vec<Arc<dyn DispatchGuard>> = DEFAULT_GUARDS
            .iter()
            .map(|guard| Arc::new(*guard) as Arc<dyn DispatchGuard>)
            .collect();
        guards.extend(self.guards);
        let mut effects: Vec<Arc<dyn DispatchEffect>> = DEFAULT_EFFECTS
            .iter()
            .map(|effect| Arc::new(*effect) as Arc<dyn DispatchEffect>)
            .collect();
        effects.extend(self.effects);
        Arc::new(HostCtx {
            browser,
            sessions,
            targets,
            helpers_root,
            branding: self.branding,
            tabs: self.tabs,
            profiles: self.profiles,
            local_tools,
            providers: self.providers,
            guards,
            effects,
            observers: self.observers,
        })
    }
}

/// The hook holds the browser weakly: sessions outlive any single connection epoch, and
/// a strong reference here would keep the browser service alive through shutdown.
fn install_retained_group_hook(sessions: &Sessions, browser: &Arc<BrowserService>) {
    let browser = Arc::downgrade(browser);
    sessions.set_retained_group_hook(Arc::new(move |ownership, key, action| {
        let browser = browser.clone();
        Box::pin(async move {
            let session = match browser.upgrade() {
                Some(browser) => browser.session().await,
                None => None,
            };
            match action {
                RetainedGroupAction::Collapse => {
                    collapse_agent_tab_group(session.as_ref(), &ownership, &key).await
                }
                RetainedGroupAction::Close => {
                    close_agent_tab_group(session.as_ref(), &ownership, &key).await
                }
            }
        })
    }));
}
