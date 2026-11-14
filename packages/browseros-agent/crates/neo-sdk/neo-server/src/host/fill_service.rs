//! The host's private typing behind `POST /v1/tabs/{tabId}/fill` and `/fill/check`: what the host
//! API's armed door runs (protected-fill design, sections 4 to 10.1).
//!
//! ```text
//!  FillAsk (neo-http: path tab, body bytes unread)
//!   fill:  FillRequest::parse ─ the values become PrivateText; nothing else reads the body
//!          ─ FillLedger::admit(requestId) ─ running: join │ answered: that answer │ fresh:
//!          ─ tokio::spawn (the panic boundary; the fill runs on if the host hangs up)
//!             browser linked? tab open? the named lock held on this tab?  (request errors:
//!                                                                          the id stays free)
//!             ticket.started(): synced to disk, before the op is submitted
//!             RunCtx::start (FILL_CODE; no requestId on the run: fills and runs never share ids)
//!             pass_lock(lockId) ─ FillValues ◀ values ─ submit host.fill as Neo
//!             ‖ the run's deadline, a backstop past the entry's own, and shutdown
//!             drain ─ FillProgress ─ fill_answer ─ the run ends ok, its result the answer
//!             ticket.finish(answer): the record, then every joined repeat
//!   check: CheckRequest::parse ─ spawn: browser? tab? ─ host.fillCheck as Neo (a read)
//! ```
//!
//! Who may call: only this door reaches the two methods. They are `Internal`, so a script naming
//! them gets `method_unavailable`, and neo-http serves the routes only behind the host auth,
//! refusing any request a web page sent.

use std::sync::Arc;

use neo_http::{
    ChecksPicture, FillAsk, FillDoor, FillRefusal, FillReply, FillVerb, TookPrivateText,
};
use neo_sdk_api as api;
use neo_takeover::NamedLock;
use neo_trace::{ErrorOwner, RunId, RunStatus};
use serde::Deserialize;
use serde_json::Value;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

use super::fill_answer::{self, Refused, owner_name};
use super::fill_ledger::{Admission, FillLedger, FillTicket};
use super::fill_request::{CheckRequest, FillRequest};
use super::{FilledTabs, TakeoverLock};
use crate::browser::{BrowserLink, PageRef, TabsScope};
use crate::builtins::{
    CheckCompared, CheckValues, ENTRY_BACKSTOP, FillProgress, FillValues, HOST_FILL,
    HOST_FILL_CHECK, HOST_FILL_CHECK_CODE, HOST_FILL_CODE, boxes_args,
};
use crate::dispatch::{
    Call, Caller, Dispatcher, ErrorCode, OpFailure, OpReply, PendingReply, RunCtx, RunEnd,
    RunStart, StopReason,
};

/// Whether the host's `fill.pictures` is `never` now (ruling R58): the settings' own answer,
/// read as each fill starts.
pub(crate) type PicturesNever = Arc<dyn Fn() -> bool + Send + Sync>;

/// Runs the host's fills and checks. One per process, armed into the host API.
pub(crate) struct FillService {
    dispatcher: Arc<Dispatcher>,
    browser: Arc<BrowserLink>,
    takeover: Arc<TakeoverLock>,
    ledger: Arc<FillLedger>,
    /// The tabs that took a fill, on disk for a restart (ruling R45), written only under
    /// `fill.pictures: never` (ruling R58).
    filled: Arc<FilledTabs>,
    /// Whether the host's `fill.pictures` is `never`, read when a fill starts (ruling R58).
    pictures_never: PicturesNever,
    /// Cancelled on SIGTERM: a fill stops as `neo / shutting_down` and records its end.
    shutdown: CancellationToken,
    /// Which tabs the host API's camera withholds: a tab that took a fill while the host's
    /// `fill.pictures` is `never`.
    took: TookPrivateText,
    /// The app's reading of what a tab shows, for the host API's camera (ruling R45).
    checks: ChecksPicture,
}

/// What the fill service is built from. A plain data bag.
pub(crate) struct FillServiceParts {
    /// The one door.
    pub(crate) dispatcher: Arc<Dispatcher>,
    /// The browser, for each run's view and the tab check.
    pub(crate) browser: Arc<BrowserLink>,
    /// The takeover lock rule, for the lock a fill names.
    pub(crate) takeover: Arc<TakeoverLock>,
    /// The fill's own record of request ids.
    pub(crate) ledger: Arc<FillLedger>,
    /// The tabs that took a fill, kept on disk under `fill.pictures: never` so a Neo restart
    /// still takes no picture of them (rulings R45, R58).
    pub(crate) filled: Arc<FilledTabs>,
    /// Whether the host's `fill.pictures` is `never` now (ruling R58).
    pub(crate) pictures_never: PicturesNever,
    /// Fires on SIGTERM.
    pub(crate) shutdown: CancellationToken,
    /// Whether the camera withholds a picture of a tab: the entry's own answer
    /// (`Playwright::withholds_picture`), true once the tab took a fill while the host's
    /// `fill.pictures` is `never`.
    pub(crate) took: TookPrivateText,
    /// What a tab shows now, read in every frame (`Playwright::picture_check`, ruling R45).
    pub(crate) checks: ChecksPicture,
}

/// The check method's value, as the service reads it back.
#[derive(Deserialize)]
struct Checked {
    fields: Vec<api::FillBoxCheck>,
}

impl FillService {
    /// The service.
    #[must_use]
    pub(crate) fn new(parts: FillServiceParts) -> Arc<Self> {
        let FillServiceParts {
            dispatcher,
            browser,
            takeover,
            ledger,
            filled,
            pictures_never,
            shutdown,
            took,
            checks,
        } = parts;
        Arc::new(Self {
            dispatcher,
            browser,
            takeover,
            ledger,
            filled,
            pictures_never,
            shutdown,
            took,
            checks,
        })
    }

    /// What the host API arms its camera's reading of a tab with (ruling R45).
    #[must_use]
    pub(crate) fn checks_picture(&self) -> ChecksPicture {
        Arc::clone(&self.checks)
    }

    /// What the host API arms its camera with, so a tab this service typed into is not pictured
    /// while the host's `fill.pictures` is `never` (design section 10.3, ruling R58).
    #[must_use]
    pub(crate) fn took_private_text(&self) -> TookPrivateText {
        Arc::clone(&self.took)
    }

    /// What the host API arms its fill routes with.
    #[must_use]
    pub(crate) fn door(self: &Arc<Self>) -> FillDoor {
        let service = Arc::clone(self);
        Arc::new(move |ask: FillAsk| {
            let service = Arc::clone(&service);
            Box::pin(async move { service.ask(ask).await.map_err(FillRefusal::from) })
        })
    }

    async fn ask(&self, ask: FillAsk) -> Result<FillReply, Refused> {
        let FillAsk { verb, tab, body } = ask;
        match verb {
            FillVerb::Fill => {
                // The one parse of the body; its bytes go right after, values and all.
                let request = FillRequest::parse(tab, &body).map_err(Refused::Invalid);
                drop(body);
                let answer = self.fill(request?).await?;
                Ok(FillReply::Fill((*answer).clone()))
            }
            FillVerb::Check => {
                let request = CheckRequest::parse(tab, &body).map_err(Refused::Invalid);
                drop(body);
                Ok(FillReply::Check(self.check(request?).await?))
            }
        }
    }

    /// One fill to its answer. A repeat of its `requestId` joins or gets the stored answer and
    /// types nothing. The fill itself runs in its own task, so a host that hangs up neither
    /// cuts it nor loses its answer for a repeat (design section 4.5).
    pub(crate) async fn fill(&self, request: FillRequest) -> Result<Arc<api::FillResult>, Refused> {
        let run = RunId::new();
        let ticket = match &request.request_id {
            None => None,
            Some(id) => match self.ledger.admit(id, request.task_id.as_ref(), run) {
                Admission::Fresh(ticket) => Some(ticket),
                // A repeat types nothing: its values go now, not after the first call's answer.
                Admission::Join(first) => {
                    drop(request);
                    return first.answer().await;
                }
                Admission::Answer(answer) => return Ok(answer),
                Admission::Refused(refused) => return Err(refused),
            },
        };
        let work = Fill {
            dispatcher: Arc::clone(&self.dispatcher),
            browser: Arc::clone(&self.browser),
            takeover: Arc::clone(&self.takeover),
            filled: Arc::clone(&self.filled),
            pictures_never: Arc::clone(&self.pictures_never),
            shutdown: self.shutdown.clone(),
        };
        match tokio::spawn(work.run(run, request, ticket)).await {
            Ok(answered) => answered,
            // A panic in the fill's task: its guards recorded the run as a crash and its ticket
            // marked the id interrupted for any repeat. The payload may carry text, so it is
            // never printed.
            Err(_) => Ok(Arc::new(fill_answer::crashed(run))),
        }
    }

    /// One check to its answer: each box's facts and the refusal a fill would meet there now.
    pub(crate) async fn check(
        &self,
        request: CheckRequest,
    ) -> Result<api::FillCheckResult, Refused> {
        let work = Fill {
            dispatcher: Arc::clone(&self.dispatcher),
            browser: Arc::clone(&self.browser),
            takeover: Arc::clone(&self.takeover),
            filled: Arc::clone(&self.filled),
            pictures_never: Arc::clone(&self.pictures_never),
            shutdown: self.shutdown.clone(),
        };
        tokio::spawn(work.check(request))
            .await
            .unwrap_or_else(|_| Err(Refused::Internal("the check's task panicked".to_owned())))
    }
}

/// What one fill or check needs, moved into its own task.
struct Fill {
    dispatcher: Arc<Dispatcher>,
    browser: Arc<BrowserLink>,
    takeover: Arc<TakeoverLock>,
    filled: Arc<FilledTabs>,
    pictures_never: PicturesNever,
    shutdown: CancellationToken,
}

impl Fill {
    async fn run(
        self,
        id: RunId,
        request: FillRequest,
        mut ticket: Option<FillTicket>,
    ) -> Result<Arc<api::FillResult>, Refused> {
        let FillRequest {
            tab,
            request_id: _,
            task_id,
            lock,
            timeout,
            url,
            boxes,
            values,
        } = request;
        let refuse = |ticket: Option<FillTicket>, refused: Refused| {
            if let Some(ticket) = ticket {
                ticket.release(refused.clone());
            }
            Err(refused)
        };
        let Some(view) = self.browser.view() else {
            return refuse(ticket, Refused::BrowserUnavailable);
        };
        let Some(page) = self.browser.page(tab).await else {
            return refuse(ticket, Refused::TabNotFound);
        };
        // A named lock that holds nothing here means the host believes the agent is locked out
        // and it is not: a request error, the id unused. A takeover's lock named is the door's
        // to refuse (`owner_has_control`, nothing typed), as any lock the host did not take.
        if let Some(lock) = lock
            && self.takeover.named(lock, tab) == NamedLock::NotHeld
        {
            return refuse(ticket, Refused::LockReleased);
        }
        // Under `fill.pictures: never`, read as the fill starts (ruling R58), the tab's target is
        // on disk before the first character, so a Neo restart while the browser keeps the tab
        // still takes no picture of it (ruling R45). A write that fails refuses the fill before it
        // starts: nothing typed, the id unused. Under the default no record is written, so none
        // can refuse the fill.
        if (self.pictures_never)()
            && let Err(error) = self.filled.remember(tab, page.target_id).await
        {
            tracing::error!(%error, run = %id, "cannot record the tab that takes the fill; it types nothing");
            return refuse(ticket, Refused::FilledTabsUnavailable);
        }
        let recorded = match ticket.as_mut() {
            Some(ticket) => ticket.started().await,
            None => Ok(()),
        };
        if let Err(error) = recorded {
            tracing::error!(%error, run = %id, "cannot record the fill's start; it types nothing");
            return refuse(ticket, Refused::RecordUnavailable);
        }
        let (run, guard) = RunCtx::start(
            &self.dispatcher,
            RunStart {
                id,
                code: HOST_FILL_CODE.to_owned(),
                request_id: None,
                task_id,
                host_data: None,
                view,
                deadline: Instant::now() + timeout + ENTRY_BACKSTOP,
                // The run's tab from its start, so the idle-tab janitor counts it in use until the
                // run ends and restarts its idle clock then: `host.fill` is Neo's op, which the
                // door does not count as a script's use (ruling R27).
                default_page: Some(PageRef { tab }),
                tabs_scope: TabsScope::Browser,
            },
        );
        if let Some(lock) = lock {
            run.pass_lock(lock);
        }
        let fields = boxes.len();
        run.slot::<FillValues>().put(values);
        let call = Call {
            name: HOST_FILL.to_owned(),
            input: boxes_args(tab, url.as_deref(), &boxes),
            caller: Caller::Neo,
            site: None,
            abort: None,
        };
        let reply = self.settle(&run, self.dispatcher.submit(&run, call)).await;
        let entry = run.slot::<FillProgress>().take();
        let answer = Arc::new(fill_answer::fill_answer(
            run.id(),
            &reply.outcome,
            entry,
            fields,
        ));
        // The whole answer is the run's result, also for a refusal (ruling R8): a failed op
        // stores no result, so the run carries `at` and `typed`.
        guard.finish(RunEnd {
            status: status_of(&run),
            error: None,
            result: serde_json::to_value(&*answer).ok(),
            replay: None,
        });
        if let Some(ticket) = ticket {
            ticket.finish(Arc::clone(&answer)).await;
        }
        Ok(answer)
    }

    async fn check(self, request: CheckRequest) -> Result<api::FillCheckResult, Refused> {
        let CheckRequest {
            tab,
            task_id,
            lock,
            timeout,
            url,
            boxes,
            values,
        } = request;
        let Some(view) = self.browser.view() else {
            return Err(Refused::BrowserUnavailable);
        };
        if self.browser.page(tab).await.is_none() {
            return Err(Refused::TabNotFound);
        }
        // As on the fill (ruling R34): the host's own lock passes, a takeover's refuses at the
        // door, and a named lock that holds nothing here is a request error.
        if let Some(lock) = lock
            && self.takeover.named(lock, tab) == NamedLock::NotHeld
        {
            return Err(Refused::LockReleased);
        }
        let (run, guard) = RunCtx::start(
            &self.dispatcher,
            RunStart {
                id: RunId::new(),
                code: HOST_FILL_CHECK_CODE.to_owned(),
                request_id: None,
                task_id,
                host_data: None,
                view,
                deadline: Instant::now() + timeout + ENTRY_BACKSTOP,
                default_page: Some(PageRef { tab }),
                tabs_scope: TabsScope::Browser,
            },
        );
        if let Some(lock) = lock {
            run.pass_lock(lock);
        }
        // Ruling R48: the values wait in the run's slot beside the op, never in its input.
        run.slot::<CheckValues>().put(values);
        let call = Call {
            name: HOST_FILL_CHECK.to_owned(),
            input: boxes_args(tab, url.as_deref(), &boxes),
            caller: Caller::Neo,
            site: None,
            abort: None,
        };
        let reply = self.settle(&run, self.dispatcher.submit(&run, call)).await;
        let fields = match reply.outcome {
            Ok(value) => checked(value, boxes.len()),
            // The door refused the look (a lock that blocks reads) or the tab failed under it:
            // every box answers that refusal, with no facts.
            Err(failure) => vec![refused_box(&failure); boxes.len()],
        };
        let recorded = api::FillCheckResult {
            run_id: run.id().to_string(),
            fields,
        };
        // The run records the answer without the compare's booleans, so a check that compared
        // keeps the shape every check's run has (ruling R48); the host alone gets them.
        let compared = run.slot::<CheckCompared>().take().unwrap_or_default();
        let mut answer = recorded.clone();
        for (field, compared) in answer.fields.iter_mut().zip(compared) {
            if let Some(compared) = compared {
                field.holds = Some(compared.holds);
                field.contains = Some(compared.contains);
            }
        }
        guard.finish(RunEnd {
            status: status_of(&run),
            error: None,
            result: serde_json::to_value(&recorded).ok(),
            replay: None,
        });
        Ok(answer)
    }

    /// The op's reply, with the run's deadline and Neo's shutdown racing it. Both stop the run,
    /// and a stopped op still answers through its guard. The run's ops drain before its end is
    /// recorded, so the op's record precedes the run's.
    async fn settle(&self, run: &Arc<RunCtx>, pending: PendingReply) -> OpReply {
        let reply = pending.wait();
        tokio::pin!(reply);
        let reply = tokio::select! {
            reply = &mut reply => reply,
            () = time::sleep_until(run.deadline()) => {
                run.stop(StopReason::Deadline);
                reply.await
            }
            () = self.shutdown.cancelled() => {
                run.stop(StopReason::Shutdown);
                reply.await
            }
        };
        let mut ops = run.close_ops();
        while ops.join_next().await.is_some() {}
        reply
    }
}

/// The run's status: `ok` when its op answered, whatever the answer was; a stop says why not.
fn status_of(run: &RunCtx) -> RunStatus {
    match run.stop_reason() {
        Some(StopReason::Deadline) => RunStatus::Timeout,
        Some(StopReason::Shutdown | StopReason::CallerLeft) => RunStatus::Cancelled,
        Some(StopReason::Crashed) => RunStatus::Error,
        Some(StopReason::ScriptReturned) | None => RunStatus::Ok,
    }
}

/// The check method's value back in the contract's shape; one that does not read is Neo's bug,
/// answered for every box.
fn checked(value: Value, boxes: usize) -> Vec<api::FillBoxCheck> {
    match serde_json::from_value::<Checked>(value) {
        Ok(checked) if checked.fields.len() == boxes => checked.fields,
        Ok(_) | Err(_) => {
            let refusal = api::OperationError {
                owner: owner_name(ErrorOwner::Neo),
                code: ErrorCode::INTERNAL_ERROR.as_str().to_owned(),
                message: "Neo could not read the look's answer".to_owned(),
                retryable: true,
                host_code: None,
                operation_id: None,
            };
            vec![
                api::FillBoxCheck {
                    facts: None,
                    refusal: Some(Box::new(refusal)),
                    holds: None,
                    contains: None,
                    frame_gone: None,
                };
                boxes
            ]
        }
    }
}

fn refused_box(failure: &OpFailure) -> api::FillBoxCheck {
    api::FillBoxCheck {
        facts: None,
        refusal: Some(Box::new(api::OperationError {
            owner: owner_name(failure.error.owner()),
            code: failure.error.code().as_str().to_owned(),
            message: failure.error.message().to_owned(),
            retryable: failure.retryable,
            host_code: None,
            operation_id: Some(failure.operation.to_string()),
        })),
        holds: None,
        contains: None,
        frame_gone: None,
    }
}
