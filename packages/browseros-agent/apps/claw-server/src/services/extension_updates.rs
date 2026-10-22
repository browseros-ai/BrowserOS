//! Coordinates a targeted extension reload over the existing browser connection.
//! The extension owns persisted pending/attempt state; the server only decides
//! when to apply it and verifies the replacement worker. No profile files or
//! third-party extension state are read or modified by this service.

use super::{browser::BrowserService, sessions::Sessions};
use browseros_core::{BrowserSession, SessionId};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{Mutex, Notify},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

const EXTENSION_ID: &str = "pjimfkbpehlcllblajnpfamdfjhhlgkc";
const API_SOURCE: &str = include_str!("../../../../contracts/extension-updates/api.js");
const POLL: Duration = Duration::from_secs(30);
const GRACE_MS: u64 = 30_000;
const MAX_DEFERRAL_MS: u64 = 14 * 60_000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateStatus {
    extension_id: String,
    running_version: String,
    pending_version: Option<String>,
    detected_at: Option<u64>,
    attempted_version: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateOutcome {
    Current,
    Deferred,
    AlreadyAttempted,
    Scheduled(String),
    Activated(String),
}

/// One coordinator per sidecar. Coalesced notifications cannot start overlapping
/// checks; the extension API persists its guard before any destructive reload.
#[derive(Default)]
pub struct ExtensionUpdates {
    wake: Notify,
    check_lock: Mutex<()>,
    verifying: Mutex<Option<(String, u64)>>,
}

impl ExtensionUpdates {
    pub fn notify(&self) {
        self.wake.notify_one();
    }

    pub fn start(
        self: &Arc<Self>,
        browser: Arc<BrowserService>,
        sessions: Arc<Sessions>,
        cancel: CancellationToken,
    ) -> JoinHandle<()> {
        let service = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(POLL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    () = cancel.cancelled() => return,
                    _ = ticker.tick() => {},
                    () = service.wake.notified() => {},
                }
                let Some(browser) = browser.session().await else {
                    continue;
                };
                let mut busy = false;
                for session in sessions.snapshot().await {
                    busy |= session.active_dispatch_count().await > 0;
                }
                // Bound native callbacks and CDP independently of the update deadline.
                let check =
                    service.check(&browser, busy, crate::clock::now_epoch_ms().max(0) as u64);
                tokio::select! {
                    () = cancel.cancelled() => return,
                    result = tokio::time::timeout(Duration::from_secs(20), check) => match result {
                        Ok(Ok(UpdateOutcome::Scheduled(version))) => tracing::info!(%version, "scheduled extension update"),
                        Ok(Ok(UpdateOutcome::Activated(version))) => tracing::info!(%version, "verified extension update"),
                        Ok(Ok(_)) => {},
                        Ok(Err(error)) => tracing::warn!(%error, "extension update check failed"),
                        Err(_) => tracing::warn!("extension update check timed out; no blind reload retry"),
                    }
                }
            }
        })
    }

    pub async fn check(
        &self,
        browser: &Arc<BrowserSession>,
        busy: bool,
        now_ms: u64,
    ) -> Result<UpdateOutcome, String> {
        let _guard = self.check_lock.lock().await;
        let targets = browser
            .cdp("Target.getTargets", json!({}), None)
            .await
            .map_err(|e| e.to_string())?;
        let prefix = format!("chrome-extension://{EXTENSION_ID}/");
        let workers: Vec<&Value> = targets["targetInfos"]
            .as_array()
            .ok_or("missing targets")?
            .iter()
            .filter(|t| {
                t["type"] == "service_worker"
                    && t["url"].as_str().is_some_and(|u| u.starts_with(&prefix))
            })
            .collect();
        // Multiple profiles may expose the same extension ID. Never guess which
        // profile owns a pending update, and never start a worker just to reload it.
        if workers.len() != 1 {
            return Ok(UpdateOutcome::Deferred);
        }
        let target = workers[0]["targetId"]
            .as_str()
            .ok_or("missing worker target")?;
        let attached = browser
            .cdp(
                "Target.attachToTarget",
                json!({"targetId":target,"flatten":true}),
                None,
            )
            .await
            .map_err(|e| e.to_string())?;
        let session = SessionId::from(
            attached["sessionId"]
                .as_str()
                .ok_or("missing attached session")?
                .to_string(),
        );
        let attachment = WorkerAttachment {
            browser: browser.clone(),
            session: session.clone(),
            active: true,
        };
        let result = tokio::time::timeout(
            Duration::from_secs(12),
            self.check_attached(browser, &session, busy, now_ms),
        )
        .await
        .map_err(|_| "extension update API timed out".to_string())
        .and_then(|result| result);
        attachment.detach().await;
        result
    }

    async fn check_attached(
        &self,
        browser: &BrowserSession,
        session: &SessionId,
        busy: bool,
        now_ms: u64,
    ) -> Result<UpdateOutcome, String> {
        // One JS implementation is bundled by the new extension and injected for
        // legacy versions. It reads native preferences in the correct profile.
        let factory = API_SOURCE.replacen("export default ", "", 1);
        let expression = format!(
            "(() => {{ if (chrome.runtime.id !== '{EXTENSION_ID}') throw new Error('Unexpected extension'); globalThis.browserosExtensionUpdates ??= ({factory})(chrome); return globalThis.browserosExtensionUpdates.getStatus(); }})()"
        );
        let status: UpdateStatus =
            serde_json::from_value(evaluate(browser, session, expression).await?)
                .map_err(|e| e.to_string())?;
        if status.extension_id != EXTENSION_ID {
            return Err("unexpected extension identity".into());
        }
        let mut verifying = self.verifying.lock().await;
        if let Some((expected, started)) = verifying.as_ref() {
            if status.running_version == *expected && status.pending_version.is_none() {
                let version = expected.clone();
                *verifying = None;
                return Ok(UpdateOutcome::Activated(version));
            }
            if now_ms.saturating_sub(*started) >= 60_000 {
                *verifying = None;
                return Err("extension reload did not activate the expected version; automatic retry suppressed".into());
            }
        }
        let Some(version) = status.pending_version else {
            return Ok(UpdateOutcome::Current);
        };
        if status.attempted_version.as_ref() == Some(&version) {
            return Ok(UpdateOutcome::AlreadyAttempted);
        }
        let age = now_ms.saturating_sub(
            status
                .detected_at
                .ok_or("pending update lacks detection time")?,
        );
        if age < GRACE_MS || (busy && age < MAX_DEFERRAL_MS) {
            return Ok(UpdateOutcome::Deferred);
        }
        let expression = format!(
            "globalThis.browserosExtensionUpdates.applyPendingUpdate({})",
            serde_json::to_string(&version).map_err(|e| e.to_string())?
        );
        // Record an uncertain attempt before sending. A lost response is not
        // permission to replay a reload; the extension also persists its guard.
        *verifying = Some((version.clone(), now_ms));
        let result = evaluate(browser, session, expression).await?;
        if result["scheduled"] == true {
            Ok(UpdateOutcome::Scheduled(version))
        } else {
            *verifying = None;
            Ok(UpdateOutcome::Deferred)
        }
    }
}

async fn evaluate(
    browser: &BrowserSession,
    session: &SessionId,
    expression: String,
) -> Result<Value, String> {
    let response = browser
        .cdp(
            "Runtime.evaluate",
            json!({"expression":expression,"awaitPromise":true,"returnByValue":true}),
            Some(session),
        )
        .await
        .map_err(|e| e.to_string())?;
    if response.get("exceptionDetails").is_some() {
        return Err("extension update API rejected the request".into());
    }
    response["result"]
        .get("value")
        .cloned()
        .ok_or_else(|| "extension update API returned no value".into())
}

/// A cancelled timeout must not strand a debugger session on the worker. Normal
/// completion detaches synchronously; cancellation schedules the same bounded
/// cleanup. Detaching a worker already destroyed by reload is harmless.
struct WorkerAttachment {
    browser: Arc<BrowserSession>,
    session: SessionId,
    active: bool,
}
impl WorkerAttachment {
    async fn detach(mut self) {
        detach_worker(&self.browser, &self.session).await;
        self.active = false;
    }
}
impl Drop for WorkerAttachment {
    fn drop(&mut self) {
        if self.active
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let browser = self.browser.clone();
            let session = self.session.clone();
            runtime.spawn(async move {
                detach_worker(&browser, &session).await;
            });
        }
    }
}
async fn detach_worker(browser: &BrowserSession, session: &SessionId) {
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        browser.cdp(
            "Target.detachFromTarget",
            json!({"sessionId":session.as_str()}),
            None,
        ),
    )
    .await;
}
