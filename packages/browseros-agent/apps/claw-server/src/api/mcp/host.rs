//! Composes claw's `HostCtx`: the shared MCP host layer branded "BrowserOS neo" and wired
//! to claw's cockpit activity, durable tab ledger, saved agent profiles, audit timeline,
//! run-failure reports, and its skill and human-help tools.

use crate::{
    VERSION,
    analytics::AnalyticsService,
    api::mcp::{
        local_tools::{
            AwaitHumanHelpTool, HumanHelpTools, MarkSkillRunTool, RequestHumanHelpTool,
            SaveSkillTool,
        },
        observers::{audit::AuditObserver, run_failure::RunFailureObserver},
        prompt::mcp_instructions,
    },
    db::{AuditLog, SessionTabLedger},
    services::{
        agent_settings::AgentSettingsStore,
        audit::AuditWorker,
        browser::BrowserService,
        cockpit::{CockpitQuery, RecordToolInput, SessionVisualService, TabActivityService},
        help::HelpRegistry,
        profiles::ProfileService,
        run_failures::RunFailureReporter,
        screenshots::ScreenshotService,
        sessions::{Sessions, TargetBindings},
        skill_runs::SkillRunService,
        skills::SkillService,
    },
};
use browseros_mcp_host::{
    HostBranding, HostCore, HostCtx, HostResult,
    identity::ProfileView,
    seams::{ProfileDirectory, TabActivity, TabClaim, TabJournal},
};
use futures_util::future::BoxFuture;
use std::{path::PathBuf, sync::Arc};

/// `_meta` key claw's stateless session handle rides under.
pub const NEO_SESSION_META_KEY: &str = "com.browseros.neo/session";
pub const NEO_SERVER_NAME: &str = "browseros-neo";
pub const NEO_SERVER_TITLE: &str = "BrowserOS neo";

/// The "BrowserOS neo" identity. Instructions follow the live human-help setting, so a
/// client that initializes after the user flips it gets the matching guidance.
#[must_use]
pub fn neo_branding(agent_settings: Arc<AgentSettingsStore>) -> HostBranding {
    HostBranding {
        server_name: NEO_SERVER_NAME.to_string(),
        server_title: NEO_SERVER_TITLE.to_string(),
        server_version: VERSION.to_string(),
        product_name: "BrowserOS neo".to_string(),
        session_meta_key: NEO_SESSION_META_KEY.to_string(),
        instructions: Arc::new(move |native| mcp_instructions(agent_settings.human_help(), native)),
    }
}

/// Everything claw's MCP host reads, borrowed from `AppState` while it is assembled.
pub struct ClawHostParts {
    pub browserclaw_dir: PathBuf,
    pub browser: Arc<BrowserService>,
    pub sessions: Arc<Sessions>,
    pub targets: Arc<TargetBindings>,
    pub audit_log: Arc<AuditLog>,
    pub audit_worker: Arc<AuditWorker>,
    pub session_tabs: Arc<SessionTabLedger>,
    pub tab_activity: Arc<TabActivityService>,
    pub visuals: Arc<SessionVisualService>,
    pub screenshots: Arc<ScreenshotService>,
    pub analytics: Arc<AnalyticsService>,
    pub run_failures: Arc<RunFailureReporter>,
    pub profiles: Arc<ProfileService>,
    pub skills: Arc<SkillService>,
    pub skill_runs: Arc<SkillRunService>,
    pub help: Arc<HelpRegistry>,
    pub cockpit: Arc<CockpitQuery>,
    pub agent_settings: Arc<AgentSettingsStore>,
}

/// Builds the host. Order matters twice: local tools are listed after `name_session` in
/// registration order, and observers run audit first, then the run-failure report.
#[must_use]
pub fn build_claw_host(parts: ClawHostParts) -> Arc<HostCtx> {
    let help_tools = Arc::new(HumanHelpTools {
        agent_settings: parts.agent_settings.clone(),
        help: parts.help,
        cockpit: parts.cockpit,
    });
    HostCtx::builder(HostCore {
        browser: parts.browser,
        sessions: parts.sessions,
        targets: parts.targets,
        helpers_root: parts.browserclaw_dir,
    })
    .branding(neo_branding(parts.agent_settings))
    .tab_journal(Arc::new(ClawTabJournal {
        tab_activity: parts.tab_activity,
        session_tabs: parts.session_tabs,
    }))
    .profiles(Arc::new(ClawProfiles(parts.profiles)))
    .local_tool(Arc::new(SaveSkillTool {
        skills: parts.skills,
    }))
    .local_tool(Arc::new(MarkSkillRunTool {
        skill_runs: parts.skill_runs,
    }))
    .local_tool(Arc::new(RequestHumanHelpTool(help_tools.clone())))
    .local_tool(Arc::new(AwaitHumanHelpTool(help_tools)))
    // The distiller (auto-capture of a successful run into a candidate helper) is
    // intentionally not wired. Self-healing keeps its agent-driven surface
    // (saveHelper/listHelpers/readHelper, discovery, hot-load) but does not
    // auto-distill. Add `distill::DistillObserver` here to re-enable.
    .observer(Arc::new(AuditObserver {
        audit_worker: parts.audit_worker,
        audit_log: parts.audit_log,
        visuals: parts.visuals,
        screenshots: parts.screenshots,
    }))
    .observer(Arc::new(RunFailureObserver {
        run_failures: parts.run_failures,
        analytics: parts.analytics,
    }))
    .build()
}

/// Mirrors agent tab work into the cockpit's live activity view and the durable
/// session-tab ledger (which drives cockpit tab lists and finished-agent tab cleanup).
pub struct ClawTabJournal {
    pub tab_activity: Arc<TabActivityService>,
    pub session_tabs: Arc<SessionTabLedger>,
}

impl TabJournal for ClawTabJournal {
    fn record_activity(&self, activity: TabActivity) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.tab_activity
                .record_tool(RecordToolInput {
                    target_id: activity.target_id,
                    tab_id: activity.tab_id,
                    page_id: activity.page_id,
                    session_id: activity.session_id,
                    agent_id: activity.convo_id,
                    slug: activity.slug,
                    tool_name: activity.tool_name,
                })
                .await;
        })
    }

    fn forget_incarnation<'a>(&'a self, page_id: u32, target_id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.tab_activity
                .remove_incarnation(page_id, target_id)
                .await;
        })
    }

    fn claim_tab(&self, claim: TabClaim) {
        self.session_tabs.enqueue_claim_tab_for_session(
            claim.tab_id,
            claim.target_id,
            claim.session_id,
            claim.convo_id,
            claim.claimed_at_ms,
        );
    }

    fn release_tab(&self, tab_id: i64, session_id: String) {
        self.session_tabs
            .enqueue_release_tab_for_session(tab_id, session_id);
    }
}

/// Saved cockpit agent profiles, which a client identity may resolve to.
pub struct ClawProfiles(pub Arc<ProfileService>);

impl ProfileDirectory for ClawProfiles {
    fn profiles(&self) -> BoxFuture<'_, HostResult<Vec<ProfileView>>> {
        Box::pin(async move {
            let profiles = self.0.list_profiles().await?;
            Ok(profiles.iter().map(ProfileView::from).collect())
        })
    }
}
