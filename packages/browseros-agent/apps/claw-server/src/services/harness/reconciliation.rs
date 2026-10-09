//! One maintenance owner for MCP and skills. Saved user choices, app discovery,
//! conservative config edits and shared-skill ownership settle under the same lock
//! as Connect/Disconnect; no request needs to trigger or wait for a periodic pass.
use std::{future::Future, time::Duration};

use harness_integrations::{ReconcileInput, SkillReconcileOutcome};

use super::*;

/// Settled changes from one pass; failed apps are retried independently.
#[derive(Debug, Default)]
pub struct ReconciliationOutcome {
    pub connected: usize,
    pub updated: usize,
    pub failed: usize,
    pub skills: SkillReconcileOutcome,
}

impl HarnessService {
    /// Repairs eligible connections and product-owned skill content. The lock moves into the
    /// blocking thread: cancellation cannot release it while disk writes continue.
    pub async fn run_reconciliation(&self, mcp_url: &str) -> AppResult<ReconciliationOutcome> {
        let guard = self.mutex.clone().lock_owned().await;
        let service = self.clone();
        let mcp_url = mcp_url.to_string();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            service.reconcile_blocking(&mcp_url)
        })
        .await?
        .map_err(manager_app_error)
    }

    fn reconcile_blocking(&self, mcp_url: &str) -> Result<ReconciliationOutcome, ManagerError> {
        let preferences = Preferences::initialize(&self.workspace_dir, &self.manager)?;
        let mut outcome = ReconciliationOutcome::default();
        // A broken MCP manifest must not prevent independent baseline skill repair.
        // Per-app failures are isolated so one malformed config cannot starve others.
        match managed_browseros_links(&self.manager, &self.workspace_dir) {
            Ok(links) => {
                let agents = Harness::ALL.map(Harness::agent_id);
                match is_installed(&agents) {
                    Ok(installed) => {
                        for harness in Harness::ALL {
                            let agent = harness.agent_id();
                            if preferences.disconnected.contains(&agent)
                                || (!installed.get(&agent).copied().unwrap_or(false)
                                    && !links.iter().any(|link| link.agent == agent))
                            {
                                continue;
                            }
                            let mut input = ReconcileInput::new(
                                McpServer {
                                    name: BROWSEROS_MCP_SERVER_NAME.into(),
                                    spec: spec_for(agent, mcp_url)?,
                                },
                                agent,
                            );
                            input.aliases = BROWSEROS_LEGACY_MCP_SERVER_NAMES
                                .iter()
                                .map(|name| (*name).into())
                                .collect();
                            match self.manager.reconcile(input) {
                                Ok(summary) => {
                                    outcome.connected += usize::from(summary.created);
                                    outcome.updated += usize::from(summary.updated);
                                    if summary.created {
                                        self.analytics.capture(
                                            events::HARNESS_CONNECTED,
                                            json!({ "harness": harness.as_str() }),
                                        );
                                    }
                                }
                                Err(error) => {
                                    outcome.failed += 1;
                                    tracing::warn!(%agent, %error, "MCP reconciliation failed; will retry");
                                }
                            }
                        }
                    }
                    Err(error) => {
                        outcome.failed += 1;
                        tracing::warn!(%error, "harness discovery failed; will retry");
                    }
                }
            }
            Err(error) => {
                outcome.failed += 1;
                tracing::warn!(%error, "MCP ownership unavailable; will retry");
            }
        }
        if let Some(managed) = &self.managed_skill {
            outcome.skills = reconcile_managed_skill(&self.manager, &self.workspace_dir, managed)?;
        }
        Ok(outcome)
    }

    /// Runs immediately and every minute on the owning HTTP runtime. Passes never
    /// overlap, missed ticks are skipped, and shutdown prevents another pass.
    pub async fn maintain_integrations(&self, mcp_url: &str, shutdown: impl Future<Output = ()>) {
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => return,
                _ = ticker.tick() => {}
            }
            match self.run_reconciliation(mcp_url).await {
                Ok(outcome) => {
                    for warning in &outcome.skills.warnings {
                        tracing::warn!(target = %warning.target.display(), warning = %warning.message,
                            "harness skill reconciliation needs a retry");
                    }
                    if outcome.connected
                        + outcome.updated
                        + outcome.skills.installed
                        + outcome.skills.updated
                        + outcome.skills.removed
                        > 0
                    {
                        tracing::info!(
                            connected = outcome.connected,
                            updated = outcome.updated,
                            installed_skills = outcome.skills.installed,
                            updated_skills = outcome.skills.updated,
                            removed_skills = outcome.skills.removed,
                            "reconciled harness integrations"
                        );
                    }
                }
                Err(error) => tracing::warn!(%error, "harness reconciliation failed; will retry"),
            }
        }
    }
}
