//! Neo: Host-owned CAPTCHA input, admitted synchronously and driven on the run's tracker.
//! The claim refuses new input; the ordinary tab turn orders input admitted before the claim.
//! A stop cancels the host token, never drops its future or releases either ownership guard.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use neo_runtime_spec::{MethodFlag, MethodMeta};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::PageId;
use crate::browser_representations::page::PageSession;
use crate::progress::{CallError, PlaywrightError, Progress};

/// A host's solver, called while the runtime owns this tab's input turn. Hosts must observe
/// stop and drain any started input before returning; the runtime never abandons this future.
pub trait CaptchaHost: Send + Sync + 'static {
    fn solve(&self, page: PageId, stop: CancellationToken) -> BoxFuture<'static, SolveReply>;
}

/// Facts the runtime needs to distinguish a finished solve, an unsent refusal and a stop.
pub enum SolveReply {
    /// `input_settled` proves every input event was written and acknowledged over the entire
    /// solve. A later successful gesture cannot erase an earlier failed event.
    Value {
        value: Value,
        log: Vec<String>,
        input_settled: bool,
    },
    Stopped,
    Refused {
        closed: bool,
        message: String,
    },
}

/// Neo: this host extension writes input, with no separate call timeout (SDK timeout: 0).
pub(crate) static META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "neo:solveCaptcha",
    title: Some("Solve CAPTCHA"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Input],
};

/// Neo: the runtime can always decode the extension; availability is a host admission policy.
pub(crate) const UNAVAILABLE: &str = "neo.solveCaptcha is unavailable: this Neo offers no CAPTCHA solver (requires explicit opt-in, hosted mode and a provider key file)";
/// Neo: refuse a second solve rather than making its own timeout look like a solver failure.
const BUSY: &str = "neo.solveCaptcha: another solve is working on this page; await it, then look at the page again";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Params {}

/// Runtime-wide admission state, shared by every run and clone, separate from waiting turns.
#[derive(Clone, Default)]
pub(crate) struct Claims(Arc<Mutex<HashSet<PageId>>>);

pub(crate) struct Claim {
    claims: Claims,
    page: PageId,
}

impl Claims {
    pub(crate) fn claim(&self, page: &PageId) -> Result<Claim, PlaywrightError> {
        let mut claims = self.0.lock().map_err(|_| PlaywrightError::error(BUSY))?;
        if !claims.insert(page.clone()) {
            return Err(PlaywrightError::error(BUSY));
        }
        Ok(Claim {
            claims: self.clone(),
            page: page.clone(),
        })
    }
    pub(crate) fn check_input(&self, page: &PageId, title: &str) -> Result<(), PlaywrightError> {
        if self.0.lock().map_or(true, |claims| claims.contains(page)) {
            return Err(PlaywrightError::error(format!(
                "{title}: neo.solveCaptcha is working on this page; await it, then look at the page again"
            )));
        }
        Ok(())
    }
}
impl Drop for Claim {
    fn drop(&mut self) {
        if let Ok(mut claims) = self.claims.0.lock() {
            claims.remove(&self.page);
        }
    }
}

/// Moved from synchronous admission into the eventual host task, never reacquired after a wait.
pub(crate) struct Admitted {
    pub(crate) host: Arc<dyn CaptchaHost>,
    pub(crate) claim: Claim,
}

pub(crate) async fn serve(
    page: Arc<PageSession>,
    id: PageId,
    admitted: Admitted,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    progress
        .race(page.prepare())
        .await?
        .map_err(|error| PlaywrightError::error(error.to_string()))?;
    let permit = progress.race(page.captcha_turn()).await?;
    progress.check()?;
    let ticket = progress
        .delivery()
        .ok_or_else(|| PlaywrightError::error("CAPTCHA input delivery cell is missing"))?
        .begin()
        .map_err(|_| PlaywrightError::error("CAPTCHA input already started"))?;
    let stop = CancellationToken::new();
    let _cancel_on_exit = stop.clone().drop_guard();
    let (reply, receiver) = oneshot::channel();
    let recording = progress.clone();
    let retained = Arc::clone(&page);
    page.spawn(async move {
        // Holding these through the host's drain prevents input interleaving and session
        // detach, even after Progress stops waiting or Dispatcher::finish returns its report.
        let ownership = (permit, admitted.claim, retained);
        let result = match admitted.host.solve(id, stop).await {
            SolveReply::Value {
                value,
                log,
                input_settled,
            } => {
                if input_settled {
                    ticket.acked().delivered();
                } else {
                    drop(ticket);
                }
                for line in log {
                    recording.log(line);
                }
                Ok(Some(value))
            }
            SolveReply::Refused { closed, message } => {
                ticket.unsent();
                Err(if closed {
                    PlaywrightError::target_closed()
                } else {
                    PlaywrightError::error(message)
                }
                .into())
            }
            SolveReply::Stopped => {
                drop(ticket);
                Err(PlaywrightError::error("neo.solveCaptcha stopped before completing").into())
            }
        };
        // Release admission and the turn before publishing the result: the awaiting script
        // may issue its next input on another thread as soon as this oneshot wakes it.
        drop(ownership);
        let _ = reply.send(result);
    });
    progress
        .race(receiver)
        .await?
        .map_err(|_| PlaywrightError::error("CAPTCHA host task ended without a reply"))?
}
