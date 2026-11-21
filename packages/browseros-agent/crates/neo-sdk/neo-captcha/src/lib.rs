//! CAPTCHA detection and solving for one BrowserOS page, shared by claw-server (the MCP
//! `solve_captcha` tool) and claw-sdk (`neo.solveCaptcha`).
//!
//! The crate has no notion of a server mode. It detects the challenge, runs the per-type flow
//! (free paths first, then a paid provider) and returns a [`types::SolveOutcome`]. Hosts branch
//! on [`types::SolveOutcome::verdict`], whose `Unsolved` is never a guess. Outcome and busy
//! messages are still worded for claw-server's MCP tools (`solve_captcha`, `request_human_help`).
//! The host keeps three jobs:
//! - deciding whether to offer solving at all (claw-server gates it to hosted mode);
//! - refusing its own page writes while [`CaptchaSolver::reservations`] holds the page, because
//!   the solver drives the page directly through `browseros_core`, not through the host;
//! - holding any viewport lease around [`CaptchaSolver::solve`].
//!
//! ```text
//! host ──solve(view, page)──▶ reserve page ─▶ detect ─▶ flow (recaptcha | turnstile | …)
//!                                  │                       ├─ page: LivePage over BrowserView
//!   host guards ◀── busy_message ──┘                       └─ providers: CapSolver → 2Captcha
//!                                                          │
//!            outcome ◀── verify gate (verify.rs; judge.rs) ◀┘ success claims only
//! ```
//!
//! The verify gate checks every answer that tells the agent to carry on (`solved`,
//! `token_applied`, `already_clear`) before it leaves the crate: deterministic page facts first,
//! a host-injected or default [`judge::Judge`] (Jev) only when those are unclear.

mod arkose;
mod aws_waf;
mod cloudflare;
pub mod config;
mod detect;
pub mod events;
mod evidence;
mod geetest;
pub mod judge;
mod page;
mod policy;
mod providers;
mod recaptcha;
pub mod reservation;
#[doc(hidden)]
pub mod testing;
mod turnstile;
mod turnstile_state;
mod twocaptcha;
pub mod types;
mod verify;

use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};

use browseros_core::{BrowserView, PageId};
use futures_util::FutureExt;
use serde_json::{Value, json};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use self::{
    events::{CaptchaEventBus, EventLog},
    page::{CaptchaPage, LivePage},
    providers::{CaptchaBackend, ProviderChain},
    reservation::{ActiveSolve, SolveLease, TabReservations},
    types::{
        Attempts, CaptchaEventName, CaptchaKind, Challenge, ChallengeState, Detection, DocumentId,
        FlowResult, PageError, ProviderAnswer, ProviderError, ProviderName, SolveMethod,
        SolveOutcome, SolveReason, SolveStatus, SolveStep, VerifyReport,
    },
};

/// How long one solve may run, from detection to its verdict. Hosts state it to their agents
/// (neo-server's `run` description reads it from here), so it is one named number.
pub const SOLVE_DEADLINE: Duration = Duration::from_secs(120);

/// Kept back from each flow's deadline while the verify check is on, so the check (rescan,
/// probe, settle, one judge call) always has time after the flow.
const VERIFY_RESERVE: Duration = Duration::from_secs(8);
/// The most the page brief taken before a flow may cost.
const BEFORE_BRIEF_BUDGET: Duration = Duration::from_millis(1_500);
/// Paid tasks a second round may buy: one token for a remedy, never another tile solve. The
/// chain checks it against the solve's charged tasks before every vendor attempt.
const RETRY_MAX_PAID: u32 = 1;

/// How `page_failure` words a failed page read. Hosts that pass a browser error through to
/// their own users match on it rather than on flow messages, which name claw-server's tools.
pub const PAGE_CHECK_FAILED: &str = "Checking the CAPTCHA page";

/// Confirmed charges belong to the request. The flow can be dropped while it verifies a paid answer.
#[derive(Default)]
struct Payments {
    provider: Option<ProviderName>,
    /// A vendor charged before the current backend call returned its combined reply.
    pending: Option<ProviderName>,
    count: u32,
    cost_usd: f64,
    /// The fingerprint of the last token a provider sold this solve, and when: the token a
    /// `token_applied` claim rests on. The token itself is never kept.
    applied: Option<verify::AppliedToken>,
    /// The family and round of the flow running now, stamped on a token it buys.
    flow_kind: Option<CaptchaKind>,
    round: u32,
}

impl Payments {
    fn finish_task(&mut self, reply: &Result<ProviderAnswer, ProviderError>) {
        // The chain's final reply already accounts for its earlier vendor attempts.
        self.pending = None;
        match reply {
            Ok(answer) => {
                self.count += 1;
                self.provider = Some(answer.provider);
                if let (types::SolveAnswer::Token(token), Some(kind)) =
                    (&answer.answer, self.flow_kind)
                {
                    self.applied = Some(verify::AppliedToken {
                        mark: verify::token_mark(token),
                        at: Instant::now(),
                        kind,
                        round: self.round,
                    });
                }
                self.cost_usd += answer
                    .cost_usd
                    .filter(|cost| cost.is_finite() && *cost >= 0.0)
                    .unwrap_or(0.0);
            }
            Err(error) if error.charged => {
                self.count += 1;
                self.provider = error.provider;
            }
            Err(_) => {}
        }
    }

    fn apply_to(&self, flow: &mut FlowResult) {
        flow.provider = self.pending.or(self.provider);
        flow.paid_tasks = self.count + u32::from(self.pending.is_some());
        flow.cost_usd = self.cost_usd;
    }
}

pub(crate) struct SolveCtx<'a, P: CaptchaPage, B: CaptchaBackend> {
    pub page: &'a P,
    pub backend: &'a B,
    payments: &'a Mutex<Payments>,
    pub events: &'a EventLog,
    pub lease: &'a SolveLease,
    pub cancel: &'a CancellationToken,
    pub deadline: tokio::time::Instant,
    pub page_url: String,
    pub document: DocumentId,
    pub user_agent: Option<String>,
    pub forced_method: Option<config::ForcedMethod>,
    /// The fix a second round applies before its flow runs again (`verify::next_step` chose
    /// it from the block the check found). `None` in the first round.
    pub remedy: Option<verify::Remedy>,
}

/// What every round of one solve shares: the request's lease, events, payments and deadline,
/// and the routing record the gate reads after each round.
struct RoundCtx<'a> {
    page: u32,
    lease: &'a SolveLease,
    events: &'a EventLog,
    payments: &'a Arc<Mutex<Payments>>,
    cancel: &'a CancellationToken,
    deadline: Instant,
    routed: Mutex<Option<Routed>>,
}

impl RoundCtx<'_> {
    fn route(
        &self,
        kind: Option<CaptchaKind>,
        challenge: Option<&Challenge>,
        document: &DocumentId,
        before: Option<evidence::PageBrief>,
    ) {
        *self.routed.lock().unwrap_or_else(|e| e.into_inner()) = Some(Routed {
            kind,
            challenge: challenge.cloned(),
            document: document.clone(),
            before,
        });
    }
}

/// The gate's answer for one round.
enum Checked {
    /// Return this answer (and the latest check, if one ran).
    Done(FlowResult, Option<VerifyReport>),
    /// Run another round with this remedy, meant for this family's flow. Carries the round's
    /// answer in case the bound is hit.
    Again(
        verify::Remedy,
        Option<CaptchaKind>,
        FlowResult,
        VerifyReport,
    ),
}

/// What changes from one round to the next.
#[derive(Debug, Clone, Copy, Default)]
struct RoundPlan {
    round: u32,
    /// The check's fix, and the family it was chosen for. A fresh scan may route another
    /// family; that flow then runs plainly, since another family's fix could misfire on it.
    remedy: Option<(verify::Remedy, CaptchaKind)>,
    /// The most paid tasks the solve may have after this round (`None`: the chain's own caps).
    paid_ceiling: Option<u32>,
}

impl<P: CaptchaPage, B: CaptchaBackend> SolveCtx<'_, P, B> {
    fn record_payment(&self, flow: &mut FlowResult, reply: &Result<ProviderAnswer, ProviderError>) {
        let mut paid = self.payments.lock().unwrap_or_else(|e| e.into_inner());
        paid.finish_task(reply);
        paid.apply_to(flow);
    }
}

/// The solver a host holds for its whole process. Instances have their own total budget and
/// page reservations. Paid tasks also count against a process-wide budget per site host.
pub struct CaptchaSolver {
    providers: ProviderChain,
    reservations: Arc<TabReservations>,
    events: CaptchaEventBus,
    forced_method: Option<config::ForcedMethod>,
    /// The verify check's second tier; `None` runs the deterministic tier only, and an unclear
    /// page then reads `unknown`.
    judge: Option<Arc<dyn judge::Judge>>,
    verify: config::VerifyMode,
    /// The last token Neo applied on each page, so a later call on the same document can tell
    /// Neo's fresh token from a stale one (the check reads whether the field still holds it).
    /// Entries past every vendor's token life are dropped on the next write.
    applied_tokens: Mutex<std::collections::HashMap<u32, RememberedToken>>,
}

/// A token Neo applied, with the document and family it was applied to.
struct RememberedToken {
    document: DocumentId,
    kind: CaptchaKind,
    token: verify::AppliedToken,
}

/// Longer than any vendor's token life (Turnstile's is the longest, 300 s).
const REMEMBER_TOKENS_FOR: Duration = Duration::from_secs(300);

/// What routing chose, kept for the verify gate: the gate runs after the routed work returns,
/// so routing records here which widget and document a success claim is about.
struct Routed {
    kind: Option<CaptchaKind>,
    challenge: Option<Challenge>,
    document: DocumentId,
    /// The page as it was just before the flow ran; `None` for terminal answers.
    before: Option<evidence::PageBrief>,
}

/// Whether the host gives the agent a way to hand the page to a person. It picks the closing
/// instruction in handoff messages and the `humanHelp` field of the handoff event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HumanHelp {
    Available,
    Unavailable,
}

/// Why a solve never started. A refusal is not an outcome: nothing was detected, clicked or paid.
#[derive(Debug)]
pub enum SolveRefusal {
    InvalidPage(String),
    /// Another solve owns the page; hosts report it instead of queueing behind it.
    Busy(ActiveSolve),
    /// Reserved for hosts; [`CaptchaSolver::solve`] never returns it.
    BrowserUnavailable,
}

impl CaptchaSolver {
    /// `None` when no provider key is configured. Hosts treat that as "solving not offered",
    /// even though the free paths (auto-pass, checkbox) would not need a key.
    ///
    /// The verify check's judge is the default Jev client when the config names a key file
    /// (`NEO_JEV_KEY_FILE`), so hosts need no code to turn it on.
    pub fn new(config: config::CaptchaConfig) -> Option<Self> {
        let judge = config.judge.clone().map(|judge_config| {
            let judge: Arc<dyn judge::Judge> = Arc::new(judge::HttpJudge::new(judge_config));
            judge
        });
        Self::new_with_judge(config, judge)
    }

    /// For a host that brings its own decision model (a shared Jev pool, or a licence-bearer
    /// oracle). `None` runs the deterministic tier only, whatever the config names.
    pub fn new_with_judge(
        config: config::CaptchaConfig,
        judge: Option<Arc<dyn judge::Judge>>,
    ) -> Option<Self> {
        if !config.has_provider() {
            return None;
        }
        Some(Self {
            providers: ProviderChain::new(&config),
            reservations: Arc::default(),
            events: CaptchaEventBus::new(),
            forced_method: config.force_method,
            judge,
            verify: config.verify,
            applied_tokens: Mutex::default(),
        })
    }

    pub fn reservations(&self) -> &Arc<TabReservations> {
        &self.reservations
    }
    pub fn events(&self) -> &CaptchaEventBus {
        &self.events
    }

    /// Detects and solves the CAPTCHA on `page` within [`SOLVE_DEADLINE`]. `Err` only when the
    /// solve never started; every started solve, cancelled or not, returns an outcome and frees
    /// the page.
    pub async fn solve(
        &self,
        view: BrowserView,
        page: PageId,
        cancel: CancellationToken,
        human_help: HumanHelp,
    ) -> Result<SolveOutcome, SolveRefusal> {
        let human_help_available = human_help == HumanHelp::Available;
        let deadline = Instant::now() + SOLVE_DEADLINE;
        let live = LivePage::new(view, page.clone());
        let validation = tokio::select! {
            biased;
            _ = cancel.cancelled() => None,
            _ = tokio::time::sleep_until(deadline) => None,
            result = live.view.page(page.clone()) => Some(result),
        };
        if let Some(result) = validation {
            result.map_err(|error| SolveRefusal::InvalidPage(error.to_string()))?;
        }
        self.solve_with(
            &live,
            page,
            cancel,
            human_help_available,
            deadline,
            &LiveDriver,
        )
        .boxed()
        .await
    }

    async fn solve_with<P: CaptchaPage>(
        &self,
        page: &P,
        page_id: PageId,
        cancel: CancellationToken,
        human_help_available: bool,
        deadline: Instant,
        driver: &impl SolveDriver,
    ) -> Result<SolveOutcome, SolveRefusal> {
        let started = Instant::now();
        let request_id = format!("cap_{}", ulid::Ulid::new());
        let lease = self
            .reservations()
            .try_reserve(page_id.0, &request_id)
            .map_err(SolveRefusal::Busy)?;
        let events = EventLog::new(self.events().clone(), request_id.clone(), page_id.0);
        let payments = Arc::new(Mutex::new(Payments::default()));
        let mut host = None;
        let rounds_started: Mutex<Vec<usize>> = Mutex::new(Vec::new());
        let rc = RoundCtx {
            page: page_id.0,
            lease: &lease,
            events: &events,
            payments: &payments,
            cancel: &cancel,
            deadline,
            routed: Mutex::new(None),
        };
        // At most `1 + MAX_RESOLVES` rounds: route and run, then check the claim. Only a
        // tier-1 block with a remedy asks for another round, and the bound is checked here, so
        // no reading of the page can make this loop.
        let checked = async {
            let mut budget = verify::VerifyBudget {
                until: deadline,
                judge_calls_left: verify::MAX_JUDGE_CALLS,
            };
            let mut plan = RoundPlan::default();
            let mut report: Option<VerifyReport> = None;
            loop {
                rounds_started
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(events.events().len());
                let mut flow = self.run_round(page, driver, &rc, plan, &mut host).await;
                // The round's own paid-task ceiling stopped the flow, not a real budget: the
                // page is still blocked after the re-solve it was allowed.
                if plan
                    .paid_ceiling
                    .is_some_and(|ceiling| paid_so_far(&payments) >= ceiling)
                    && flow.reason == SolveReason::SpendLimitReached
                {
                    flow = verify::still_blocked(&flow);
                }
                let routed = rc.routed.lock().unwrap_or_else(|e| e.into_inner()).take();
                match self
                    .check_claim(page, driver, &rc, routed, flow, plan.round, &mut budget)
                    .await
                {
                    Checked::Done(flow, Some(now)) => break (flow, Some(now)),
                    // This round ran no check; the last report still describes the page, and
                    // it counts every round that ran.
                    Checked::Done(flow, None) => {
                        let rounds = plan.round + 1;
                        break (flow, report.map(|last| VerifyReport { rounds, ..last }));
                    }
                    Checked::Again(remedy, kind, _, now) if plan.round < verify::MAX_RESOLVES => {
                        plan = RoundPlan {
                            round: plan.round + 1,
                            remedy: kind.map(|kind| (remedy, kind)),
                            paid_ceiling: Some(paid_so_far(&payments) + RETRY_MAX_PAID),
                        };
                        report = Some(now);
                    }
                    Checked::Again(_, _, flow, now) => {
                        break (verify::still_blocked(&flow), Some(now));
                    }
                }
            }
        };
        // The rounds and their checks run inside one race, so cancel, the deadline and a
        // closed page stop the check too.
        let (mut flow, verify_report) = tokio::select! {
            biased;
            _ = cancel.cancelled() => (page_failure(PageError::Cancelled), None),
            _ = tokio::time::sleep_until(deadline) => (terminal(SolveStatus::Failed, SolveReason::DeadlineExceeded, "The CAPTCHA solve exceeded its 120-second deadline. Take a fresh snapshot before retrying."), None),
            _ = wait_for_page_close(page) => (page_failure(PageError::Closed), None),
            result = checked => result,
        };
        {
            let paid = payments.lock().unwrap_or_else(|e| e.into_inner());
            if paid.count > 0 || paid.pending.is_some() {
                paid.apply_to(&mut flow);
            }
        }
        let kind = self
            .reservations()
            .active(page_id.0)
            .and_then(|active| active.kind);
        // Attempts are numbered per round (each flow counts from one), so fold each round to
        // its highest attempt per method and add the rounds up.
        let starts = rounds_started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut per_round = vec![Attempts::default(); starts.len().max(1)];
        let mut last_method = SolveMethod::None;
        for (index, event) in events.events().iter().enumerate() {
            if event.event != CaptchaEventName::Solving {
                continue;
            }
            let Some(method) = event
                .data
                .get("method")
                .and_then(|value| serde_json::from_value::<SolveMethod>(value.clone()).ok())
            else {
                continue;
            };
            last_method = method;
            let number = |key: &str| {
                event
                    .data
                    .get(key)
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
            };
            let attempt = number("attempt").unwrap_or(0);
            let round = starts
                .iter()
                .rposition(|start| *start <= index)
                .unwrap_or(0);
            let Some(slot) = per_round.get_mut(round) else {
                continue;
            };
            match method {
                SolveMethod::Checkbox => slot.checkbox = slot.checkbox.max(attempt),
                SolveMethod::Token => slot.token = slot.token.max(attempt),
                SolveMethod::Tiles => {
                    slot.tile_rounds = slot.tile_rounds.max(number("round").unwrap_or(attempt));
                }
                SolveMethod::None | SolveMethod::AutoPass => {}
            }
        }
        let folded = per_round
            .iter()
            .fold(Attempts::default(), |sum, round| Attempts {
                checkbox: sum.checkbox + round.checkbox,
                tile_rounds: sum.tile_rounds + round.tile_rounds,
                token: sum.token + round.token,
            });
        flow.attempts.checkbox = flow.attempts.checkbox.max(folded.checkbox);
        flow.attempts.tile_rounds = flow.attempts.tile_rounds.max(folded.tile_rounds);
        flow.attempts.token = flow.attempts.token.max(folded.token);
        if flow.method == SolveMethod::None {
            flow.method = last_method;
        }
        if matches!(
            flow.status,
            SolveStatus::NeedsHuman | SolveStatus::Unsupported | SolveStatus::PolicyWall
        ) {
            flow.message.push_str(if human_help_available {
                " Call request_human_help with kind \"captcha\"."
            } else if flow.status == SolveStatus::PolicyWall {
                " Ask the user to take over this page."
            } else {
                " Stop and tell the user that this CAPTCHA needs their help."
            });
        }
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match flow.status {
            SolveStatus::Solved => events.emit(
                CaptchaEventName::Solved,
                json!({"type":kind,"method":flow.method,"durationMs":duration_ms,"provider":flow.provider}),
            ),
            SolveStatus::TokenApplied => events.emit(
                CaptchaEventName::TokenApplied,
                json!({"type":kind,"method":flow.method,"durationMs":duration_ms,"callbackRan":flow.callback_ran,"provider":flow.provider}),
            ),
            SolveStatus::NeedsHuman | SolveStatus::PolicyWall => events.emit(
                CaptchaEventName::Handoff,
                json!({"type":kind,"reason":flow.reason,"humanHelp":human_help_available,"provider":flow.provider}),
            ),
            SolveStatus::Failed | SolveStatus::Cancelled | SolveStatus::Unsupported => {
                let mut data = json!({"type":kind,"status":flow.status,"reason":flow.reason,"provider":flow.provider});
                if flow.reason == SolveReason::BrowserError {
                    data["cause"] = json!(flow.message);
                }
                events.emit(CaptchaEventName::Failed, with_family(data, kind));
            }
            SolveStatus::AlreadyClear => {}
        }
        let outcome = SolveOutcome {
            status: flow.status,
            kind,
            method: flow.method,
            reason: flow.reason,
            request_id,
            page: page_id.0,
            duration_ms,
            attempts: flow.attempts,
            paid_tasks: flow.paid_tasks,
            provider: flow.provider,
            cost_usd: flow.cost_usd,
            message: flow.message,
            events: events.events(),
            verify: verify_report,
        };
        events::log_outcome(&outcome, host.as_deref(), outcome.provider);
        Ok(outcome)
    }
}

impl CaptchaSolver {
    /// A solver with no provider key and the verify check off, for tests that drive
    /// `solve_with` directly. One constructor, so a new field does not break every test file.
    #[cfg(test)]
    pub(crate) fn unkeyed() -> Self {
        Self {
            providers: ProviderChain::new(&config::CaptchaConfig::default()),
            reservations: Arc::default(),
            events: CaptchaEventBus::new(),
            forced_method: None,
            judge: None,
            verify: config::VerifyMode::Off,
            applied_tokens: Mutex::default(),
        }
    }

    /// One round: policy wall, detect, route, then the chosen flow or a terminal answer. Routing
    /// records in `rc.routed` which widget and document a success claim is about, for the gate.
    async fn run_round<P: CaptchaPage>(
        &self,
        page: &P,
        driver: &impl SolveDriver,
        rc: &RoundCtx<'_>,
        plan: RoundPlan,
        host: &mut Option<String>,
    ) -> FlowResult {
        let mut user_agent = None;
        let mut read_user_agent = false;
        for rescan in 0..=1 {
            // An explicit access restriction takes precedence over any widget on the page.
            match policy::policy_wall(page).await {
                Ok(Some(wall)) => {
                    *host = wall.host;
                    return terminal(
                        SolveStatus::PolicyWall,
                        SolveReason::PolicyWall,
                        "This site blocks automated agents. Stop acting on the page.",
                    );
                }
                Ok(None) => {}
                Err(PageError::Navigated) if rescan == 0 => continue,
                // A page still loading can throw or stall this read; detection tolerates the
                // same errors, so a failed read is "no wall seen", not a failed solve.
                Err(PageError::Browser(_) | PageError::Timeout) => {
                    tracing::debug!(target: "captcha", "policy-wall read failed; treated as no wall");
                }
                Err(error) => return page_failure(error),
            }
            let detection = match driver.detect(page).await {
                Err(PageError::Navigated) if rescan == 0 => continue,
                result => match result {
                    Ok(detection) => detection,
                    Err(error) => return page_failure(error),
                },
            };
            // The Cloudflare observer needs the original document even if it just cleared.
            // Other flows act on the page, so their detection must still be current.
            if !detection.cloudflare_wall {
                let current = match page.document().await {
                    Ok(document) => document,
                    Err(PageError::Navigated) if rescan == 0 => continue,
                    Err(error) => return page_failure(error),
                };
                if current != detection.document {
                    if rescan == 0 {
                        continue;
                    }
                    return page_failure(PageError::Navigated);
                }
            }
            if let Some(challenge) = detection
                .challenges
                .iter()
                .find(|challenge| detection.supports(challenge))
            {
                rc.lease
                    .set_progress(Some(challenge.kind), SolveStep::Detecting);
            }
            let detection = match wait_for_ready_detection(page, driver, detection).await {
                Ok(detection) => detection,
                Err(error) => return page_failure(error),
            };
            *host = url::Url::parse(&detection.document.url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned));
            // PerimeterX's check covers the page, so a widget under it cannot be reached: it
            // goes first, and the answer says a person must pass it.
            let selected = detection
                .challenges
                .iter()
                .find(|challenge| {
                    challenge.kind == CaptchaKind::PerimeterX
                        && challenge.state == ChallengeState::Active
                })
                .or_else(|| {
                    detection.challenges.iter().find(|challenge| {
                        challenge.kind == CaptchaKind::CloudflareChallenge
                            && detection.supports(challenge)
                    })
                })
                .or_else(|| {
                    detection.challenges.iter().find(|challenge| {
                        detection.supports(challenge) && challenge.state == ChallengeState::Active
                    })
                })
                .or_else(|| {
                    detection.challenges.iter().find(|challenge| {
                        !detection.supports(challenge)
                            && challenge.kind != CaptchaKind::Unknown
                            && matches!(
                                challenge.state,
                                ChallengeState::Active | ChallengeState::Unknown
                            )
                    })
                })
                .or_else(|| detection.challenges.first());
            if selected.is_some_and(|challenge| {
                detection.supports(challenge)
                    && challenge.state == ChallengeState::Active
                    && challenge.kind != CaptchaKind::CloudflareChallenge
            }) && !read_user_agent
            {
                user_agent = page
                    .eval_main("navigator.userAgent")
                    .await
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned));
                read_user_agent = true;
                match page.document().await {
                    Ok(document) if document != detection.document => {
                        if rescan == 0 {
                            continue;
                        }
                        return page_failure(PageError::Navigated);
                    }
                    Err(error) => return page_failure(error),
                    _ => {}
                }
            }
            if let Some(challenge) = selected {
                rc.lease
                    .set_progress(Some(challenge.kind), SolveStep::Detecting);
                rc.events.emit(
                    CaptchaEventName::Detected,
                    with_family(
                        json!({"type":challenge.kind,"state":challenge.state,"complete":detection.complete,"challenges":detection.challenges.len()}),
                        Some(challenge.kind),
                    ),
                );
                if detection.supports(challenge) && challenge.state == ChallengeState::Active {
                    let providers = self
                        .providers
                        .for_solve(Arc::clone(rc.payments), host.as_deref());
                    let backend = providers.with_paid_ceiling(plan.paid_ceiling);
                    // The page before the flow, so the check can tell new alerts from old ones.
                    // Bounded: a page whose scripts stall must not eat the flow's time. Without a
                    // before-brief the check simply cannot tell new alerts from old ones.
                    let before = if self.verify == config::VerifyMode::Off {
                        None
                    } else {
                        tokio::time::timeout(BEFORE_BRIEF_BUDGET, evidence::brief(page))
                            .await
                            .ok()
                            .and_then(Result::ok)
                    };
                    {
                        let mut paid = rc.payments.lock().unwrap_or_else(|e| e.into_inner());
                        paid.flow_kind = Some(challenge.kind);
                        paid.round = plan.round;
                    }
                    let ctx = SolveCtx {
                        page,
                        backend: &backend,
                        payments: rc.payments,
                        events: rc.events,
                        lease: rc.lease,
                        cancel: rc.cancel,
                        deadline: self.flow_deadline(rc.deadline),
                        page_url: detection.document.url.clone(),
                        document: detection.document.clone(),
                        user_agent,
                        forced_method: self.forced_method,
                        remedy: plan
                            .remedy
                            .filter(|(_, kind)| *kind == challenge.kind)
                            .map(|(remedy, _)| remedy),
                    };
                    rc.route(
                        Some(challenge.kind),
                        Some(challenge),
                        &detection.document,
                        before,
                    );
                    return driver.solve(&ctx, challenge).await;
                }
                if !detection.supports(challenge)
                    && challenge.kind != CaptchaKind::Unknown
                    && matches!(
                        challenge.state,
                        ChallengeState::Active | ChallengeState::Unknown
                    )
                {
                    return terminal(
                        SolveStatus::Unsupported,
                        SolveReason::UnsupportedType,
                        unsupported_message(challenge.kind),
                    );
                }
            }
            // A widget still running its own check outlasted the readiness wait. The page is
            // neither clear nor ready to solve, whatever else the scan read.
            if detection
                .challenges
                .iter()
                .any(|challenge| challenge.state == ChallengeState::Pending)
            {
                return terminal(
                    SolveStatus::Failed,
                    SolveReason::ChallengePending,
                    STILL_CHECKING,
                );
            }
            if !detection.complete {
                return terminal(
                    SolveStatus::Failed,
                    SolveReason::ScanIncomplete,
                    "The CAPTCHA scan was incomplete. Take a fresh snapshot and retry.",
                );
            }
            if detection.challenges.is_empty() {
                rc.route(None, None, &detection.document, None);
                return terminal(
                    SolveStatus::AlreadyClear,
                    SolveReason::NoCaptcha,
                    "No CAPTCHA was found. Continue the task.",
                );
            }
            if detection
                .challenges
                .iter()
                .all(|challenge| challenge.state == ChallengeState::Passed)
            {
                let passed = detection.challenges.first();
                rc.route(passed.map(|c| c.kind), passed, &detection.document, None);
                return terminal(
                    SolveStatus::AlreadyClear,
                    SolveReason::AlreadyPassed,
                    "The CAPTCHA has already passed. Continue the task.",
                );
            }
            if detection.challenges.iter().all(|challenge| {
                matches!(
                    challenge.state,
                    ChallengeState::Passed | ChallengeState::WidgetOnly
                )
            }) {
                rc.route(None, None, &detection.document, None);
                return terminal(
                    SolveStatus::AlreadyClear,
                    SolveReason::NotBlocking,
                    "The CAPTCHA widget is not blocking the page. Continue the task.",
                );
            }
            return terminal(
                SolveStatus::Failed,
                SolveReason::ScanIncomplete,
                "The CAPTCHA state could not be determined. Take a fresh snapshot and retry.",
            );
        }
        page_failure(PageError::Navigated)
    }

    /// The token a claim rests on. A `token_applied` claim names the token this round's flow
    /// bought for this family (and the solver remembers it for the page); any other claim on the
    /// same document and family may name the one an earlier call applied there.
    fn applied_token(
        &self,
        rc: &RoundCtx<'_>,
        routed: &Routed,
        basis: verify::Basis,
        round: u32,
    ) -> Option<verify::AppliedToken> {
        let mut remembered = self
            .applied_tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let bought = (basis == verify::Basis::TokenInjected)
            .then(|| {
                rc.payments
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .applied
                    .clone()
            })
            .flatten()
            // Only this round's token for this widget's family: a rescan may route another
            // family in round two, and its claim must not rest on round one's token.
            .filter(|token| Some(token.kind) == routed.kind && token.round == round);
        if let (Some(token), Some(kind)) = (&bought, routed.kind) {
            remembered.retain(|_, entry| entry.token.at.elapsed() < REMEMBER_TOKENS_FOR);
            remembered.insert(
                rc.page,
                RememberedToken {
                    document: routed.document.clone(),
                    kind,
                    token: token.clone(),
                },
            );
            return bought;
        }
        remembered
            .get(&rc.page)
            .filter(|entry| entry.document == routed.document && Some(entry.kind) == routed.kind)
            .map(|entry| entry.token.clone())
    }

    /// The deadline a flow gets: the solve's, less what the check needs after it.
    fn flow_deadline(&self, deadline: Instant) -> Instant {
        match self.verify {
            config::VerifyMode::Off => deadline,
            config::VerifyMode::On | config::VerifyMode::Shadow => {
                deadline.checked_sub(VERIFY_RESERVE).unwrap_or(deadline)
            }
        }
    }

    /// The verify gate: confirms a success claim before it reaches the host, or replaces it with
    /// the truthful answer (`still_blocked`, `challenge_pending`, `acceptance_unconfirmed`, …), or
    /// asks for one more round with a remedy. `Shadow` reports the check but keeps the claim.
    /// Any answer that is not a success claim passes untouched.
    #[expect(
        clippy::too_many_arguments,
        reason = "the round's shared state is already one struct; the rest changes per round"
    )]
    async fn check_claim<P: CaptchaPage>(
        &self,
        page: &P,
        driver: &impl SolveDriver,
        rc: &RoundCtx<'_>,
        routed: Option<Routed>,
        flow: FlowResult,
        round: u32,
        budget: &mut verify::VerifyBudget,
    ) -> Checked {
        if self.verify == config::VerifyMode::Off || !flow.status.claims_success() {
            return Checked::Done(flow, None);
        }
        // Every success path routes; one that did not is not checked, so it is not confirmed.
        let Some(routed) = routed else {
            if self.verify == config::VerifyMode::Shadow {
                return Checked::Done(flow, None);
            }
            return Checked::Done(unconfirmed(&flow), None);
        };
        let basis = verify::Basis::of(&flow, routed.kind);
        let applied = self.applied_token(rc, &routed, basis, round);
        let claim = verify::Claim {
            basis,
            kind: routed.kind,
            challenge: routed.challenge.as_ref(),
            document: &routed.document,
            before: routed.before.as_ref(),
            callback_ran: flow.callback_ran,
            applied: applied.as_ref(),
        };
        let started = Instant::now();
        let verification = match driver
            .standing(page, &claim, self.judge.as_deref(), budget, rc.cancel)
            .await
        {
            Ok(verification) => verification,
            // Shadow never changes an answer, so a check cut short (the claim's own callback
            // navigated, say) leaves the flow's answer as it was.
            Err(error) if self.verify == config::VerifyMode::Shadow => {
                tracing::info!(target: "captcha", error = %error, "verify check stopped early in shadow mode");
                return Checked::Done(flow, None);
            }
            Err(error) => return Checked::Done(page_failure(error), None),
        };
        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        // A plain clear `already_clear` stays silent, so an empty page's answer has no events.
        if routed.challenge.is_some()
            || verification.standing != verify::Standing::Clear
            || verification.judged.is_some()
        {
            rc.events.emit(
                CaptchaEventName::Verified,
                verify::event_data(&verification, routed.kind, round, ms),
            );
        }
        let report = verify::report(
            &verification,
            round + 1,
            verify::MAX_JUDGE_CALLS - budget.judge_calls_left,
        );
        if self.verify == config::VerifyMode::Shadow {
            return Checked::Done(flow, Some(report));
        }
        let left = rc.deadline.saturating_duration_since(Instant::now());
        match verify::next_step(&flow, &verification, &claim, round, left) {
            verify::Next::Return(answer) => Checked::Done(answer, Some(report)),
            verify::Next::Retry(remedy) => Checked::Again(remedy, routed.kind, flow, report),
        }
    }
}

trait SolveDriver: Send + Sync {
    fn detect<P: CaptchaPage>(
        &self,
        page: &P,
    ) -> impl Future<Output = Result<Detection, PageError>> + Send;
    fn solve<P: CaptchaPage, B: CaptchaBackend>(
        &self,
        ctx: &SolveCtx<'_, P, B>,
        challenge: &Challenge,
    ) -> impl Future<Output = FlowResult> + Send;
    /// The verify check. A seam so orchestration tests can script what the check finds.
    fn standing<P: CaptchaPage>(
        &self,
        page: &P,
        claim: &verify::Claim<'_>,
        judge: Option<&dyn judge::Judge>,
        budget: &mut verify::VerifyBudget,
        cancel: &CancellationToken,
    ) -> impl Future<Output = Result<verify::Verification, PageError>> + Send {
        verify::standing(page, claim, judge, budget, cancel)
    }
}
struct LiveDriver;
impl SolveDriver for LiveDriver {
    async fn detect<P: CaptchaPage>(&self, page: &P) -> Result<Detection, PageError> {
        detect::detect(page, Duration::from_secs(2)).await
    }
    async fn solve<P: CaptchaPage, B: CaptchaBackend>(
        &self,
        ctx: &SolveCtx<'_, P, B>,
        challenge: &Challenge,
    ) -> FlowResult {
        match challenge.kind {
            CaptchaKind::RecaptchaV2 => recaptcha::solve(ctx, challenge).await,
            CaptchaKind::Turnstile => turnstile::solve(ctx, challenge).await,
            CaptchaKind::CloudflareChallenge => cloudflare::observe(ctx).await,
            CaptchaKind::Geetest => geetest::solve(ctx, challenge).await,
            CaptchaKind::AwsWaf => aws_waf::solve(ctx, challenge).await,
            CaptchaKind::Arkose => arkose::solve(ctx, challenge).await,
            _ => terminal(
                SolveStatus::Unsupported,
                SolveReason::UnsupportedType,
                "This CAPTCHA type is not supported.",
            ),
        }
    }
}
/// The answer while a widget is still running its own check (`ChallengeState::Pending`).
const STILL_CHECKING: &str = "The CAPTCHA is still running its own check. Wait a few seconds, then take a fresh snapshot and retry; do not submit the form yet.";

/// Gives a supported widget up to five seconds to settle before routing: one still loading
/// (`Unknown`, or an incomplete scan) or still running its own check (`Pending`) is scanned again
/// every half second, until one is ready to solve or nothing is left to wait for.
async fn wait_for_ready_detection<P: CaptchaPage>(
    page: &P,
    driver: &impl SolveDriver,
    mut detection: Detection,
) -> Result<Detection, PageError> {
    if !detection
        .challenges
        .iter()
        .any(|challenge| detection.supports(challenge))
    {
        return Ok(detection);
    }
    let document = detection.document.clone();
    let until = Instant::now() + Duration::from_secs(5);
    while !detection
        .challenges
        .iter()
        .any(|challenge| detection.supports(challenge) && challenge.state == ChallengeState::Active)
        && (!detection.complete
            || detection.challenges.iter().any(|challenge| {
                detection.supports(challenge)
                    && matches!(
                        challenge.state,
                        ChallengeState::Unknown | ChallengeState::Pending
                    )
            }))
    {
        tokio::time::sleep_until((Instant::now() + Duration::from_millis(500)).min(until)).await;
        if Instant::now() >= until {
            break;
        }
        match tokio::time::timeout_at(until, async {
            let next = driver.detect(page).await?;
            if next.document != document || page.document().await? != document {
                return Err(PageError::Navigated);
            }
            Ok(next)
        })
        .await
        {
            Ok(next) => detection = next?,
            Err(_) => break,
        }
    }
    Ok(detection)
}

/// The answer's first sentence for a family Neo detects and does not solve. The caller adds what
/// to do next: ask for a person (claw-server's `request_human_help`), or stop and tell the user.
fn unsupported_message(kind: CaptchaKind) -> &'static str {
    match kind {
        CaptchaKind::PerimeterX => {
            "PerimeterX's press-and-hold human check covers the page, and only a person can pass it."
        }
        _ => "This CAPTCHA type is not supported.",
    }
}

/// Event data that names the family when its wire `type` cannot: `PerimeterX`'s is `unknown` (see
/// [`CaptchaKind::PerimeterX`]). Every other kind's data is left as it was.
fn with_family(mut data: Value, kind: Option<CaptchaKind>) -> Value {
    if kind == Some(CaptchaKind::PerimeterX)
        && let Value::Object(fields) = &mut data
    {
        fields.insert("family".to_owned(), json!("perimeterx"));
    }
    data
}

fn terminal(status: SolveStatus, reason: SolveReason, message: &str) -> FlowResult {
    FlowResult {
        status,
        method: SolveMethod::None,
        reason,
        attempts: Attempts::default(),
        paid_tasks: 0,
        provider: None,
        cost_usd: 0.0,
        callback_ran: false,
        message: message.into(),
    }
}

fn token_applied_message(callback_ran: bool) -> String {
    let callback = if callback_ran {
        "the callback ran"
    } else {
        "no callback was declared"
    };
    format!(
        "The CAPTCHA token is in the page; {callback}. Submit the form or continue the action and check the result. If the site rejects it, call solve_captcha again."
    )
}
/// Paid tasks this solve has bought, counting a vendor charged inside a call still running.
fn paid_so_far(payments: &Mutex<Payments>) -> u32 {
    let paid = payments.lock().unwrap_or_else(|e| e.into_inner());
    paid.count + u32::from(paid.pending.is_some())
}

/// A success claim the gate could not check, as the truthful `unknown` answer.
fn unconfirmed(flow: &FlowResult) -> FlowResult {
    FlowResult {
        status: SolveStatus::Failed,
        reason: SolveReason::AcceptanceUnconfirmed,
        message: "Neo could not confirm the page is past the CAPTCHA. Take a fresh snapshot before acting.".into(),
        ..flow.clone()
    }
}

fn page_failure(error: PageError) -> FlowResult {
    let (status, reason, message) = match error {
        PageError::Closed => (
            SolveStatus::Cancelled,
            SolveReason::TabClosed,
            "The page closed during the CAPTCHA solve.",
        ),
        PageError::Navigated => (
            SolveStatus::Cancelled,
            SolveReason::Navigated,
            "The page navigated during the CAPTCHA solve. Take a fresh snapshot.",
        ),
        PageError::Cancelled => (
            SolveStatus::Cancelled,
            SolveReason::CallerCancelled,
            "The CAPTCHA solve was cancelled by the caller.",
        ),
        PageError::Timeout => (
            SolveStatus::Failed,
            SolveReason::BrowserError,
            "Checking the CAPTCHA page timed out.",
        ),
        PageError::Browser(message) => {
            let detail = events::browser_error_detail(&message);
            return terminal(
                SolveStatus::Failed,
                SolveReason::BrowserError,
                &format!("{PAGE_CHECK_FAILED} failed ({detail})."),
            );
        }
    };
    terminal(status, reason, message)
}
async fn wait_for_page_close(page: &impl CaptchaPage) {
    loop {
        if matches!(page.document().await, Err(PageError::Closed)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_that_coordination_stubs_use_stable_constructors() -> anyhow::Result<()> {
        let reservations = Arc::<TabReservations>::default();
        let lease = reservations
            .try_reserve(42, "cap_test")
            .map_err(|_| anyhow::anyhow!("unexpected busy page"))?;
        let log = EventLog::new(CaptchaEventBus::new(), "cap_test".into(), 42);
        assert_eq!(lease.page(), 42);
        assert_eq!(lease.request_id(), "cap_test");
        assert!(log.events().is_empty());
        Ok(())
    }
}

#[cfg(test)]
mod orchestration_tests {
    use super::*;
    use serde_json::{Value, json};
    use std::{
        collections::VecDeque,
        sync::{
            Mutex,
            atomic::{AtomicU32, Ordering},
        },
        time::Duration,
    };
    use types::{
        CaptchaEventName, CaptchaKind, Challenge, ChallengeState, Detection, FlowResult, FrameRef,
        PageError, Rect, ScanOutcome, SolveAnswer, SolveStep,
    };

    fn document(loader: &str) -> DocumentId {
        DocumentId {
            url: "https://fixture.test/captcha?private=yes".into(),
            loader_id: loader.into(),
        }
    }

    struct FakePage {
        document: Result<DocumentId, PageError>,
        clicks: AtomicU32,
        /// What the policy-wall read sees; `None` reads as an ordinary page.
        notice: Option<Value>,
        /// The policy-wall read throws, as on a page still loading.
        notice_fails: bool,
        /// What the check's read of the response fields' fingerprints returns.
        token_marks: Option<Value>,
        /// Every DOM scan hangs, as on a page whose scripts stall.
        dom_stalls: bool,
    }
    impl FakePage {
        fn new(document: Result<DocumentId, PageError>) -> Self {
            Self {
                document,
                clicks: AtomicU32::new(0),
                notice: None,
                notice_fails: false,
                token_marks: None,
                dom_stalls: false,
            }
        }
    }
    impl CaptchaPage for FakePage {
        async fn eval_main(&self, expression: &str) -> Result<Value, PageError> {
            if expression.contains("captcha:policy-wall") {
                if self.notice_fails {
                    return Err(PageError::Browser("Execution context was destroyed".into()));
                }
                if let Some(notice) = &self.notice {
                    return Ok(notice.clone());
                }
            }
            Ok(json!("fake-agent"))
        }
        async fn document(&self) -> Result<DocumentId, PageError> {
            self.document.clone()
        }
        async fn frames(&self) -> Result<Vec<FrameRef>, PageError> {
            Ok(vec![])
        }
        async fn eval_in_frame(&self, _: &FrameRef, _: &str) -> Result<Value, PageError> {
            Ok(Value::Null)
        }
        async fn click_at(&self, _: f64, _: f64) -> Result<(), PageError> {
            self.clicks.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        async fn scan_dom(&self, _: Option<&FrameRef>, function: &str) -> Result<Value, PageError> {
            if self.dom_stalls {
                std::future::pending::<()>().await;
            }
            match &self.token_marks {
                Some(marks) if function.contains("captcha:verify-token") => Ok(marks.clone()),
                _ => Err(PageError::Browser("DOM scan unavailable".into())),
            }
        }
        async fn screenshot_clip(&self, _: Rect) -> Result<String, PageError> {
            Ok(String::new())
        }
        async fn query_rect_in_frame(
            &self,
            _: &FrameRef,
            _: &str,
        ) -> Result<Option<Rect>, PageError> {
            Ok(None)
        }
    }

    struct FakeDriver {
        scans: Mutex<VecDeque<Result<Detection, PageError>>>,
        stall_after: Option<ProviderAnswer>,
        result: Option<FlowResult>,
        expected_forced_method: Option<config::ForcedMethod>,
        /// Paid once per flow run, through the round's backend, then recorded like a real task.
        pays: Option<ProviderAnswer>,
        /// What the verify check finds, one per check; when empty the real check runs.
        findings: Mutex<VecDeque<verify::Verification>>,
        /// The remedy each flow run was given.
        remedies: Mutex<Vec<Option<verify::Remedy>>>,
        /// Whether each check was handed a judge.
        judged: Mutex<Vec<bool>>,
        /// The answer of every flow run after the first, when it differs from `result`.
        second: Option<FlowResult>,
        /// The check stops with this error instead of finding anything.
        check_error: Option<PageError>,
        /// The fingerprint of the applied token each check was handed.
        applied_seen: Mutex<Vec<Option<String>>>,
        /// Only the first flow run buys a token.
        pays_first_only: bool,
    }
    impl FakeDriver {
        fn new(detection: Detection) -> Self {
            Self {
                scans: Mutex::new(VecDeque::from([Ok(detection)])),
                stall_after: None,
                result: None,
                expected_forced_method: None,
                pays: None,
                findings: Mutex::new(VecDeque::new()),
                remedies: Mutex::new(Vec::new()),
                judged: Mutex::new(Vec::new()),
                second: None,
                check_error: None,
                applied_seen: Mutex::new(Vec::new()),
                pays_first_only: false,
            }
        }
        fn finding(standing: verify::Standing, tier: types::VerifyTier) -> verify::Verification {
            verify::Verification {
                standing,
                tier,
                brief: None,
                judged: None,
            }
        }
    }
    impl SolveDriver for FakeDriver {
        async fn detect<P: CaptchaPage>(&self, _: &P) -> Result<Detection, PageError> {
            let mut scans = self
                .scans
                .lock()
                .map_err(|_| PageError::Browser("poisoned".into()))?;
            if scans.len() > 1 {
                scans.pop_front()
            } else {
                scans.front().cloned()
            }
            .ok_or_else(|| PageError::Browser("no scan".into()))?
        }
        async fn solve<P: CaptchaPage, B: CaptchaBackend>(
            &self,
            ctx: &SolveCtx<'_, P, B>,
            challenge: &Challenge,
        ) -> FlowResult {
            assert_eq!(ctx.forced_method, self.expected_forced_method);
            let (method, reason) = match challenge.kind {
                CaptchaKind::RecaptchaV2 => (SolveMethod::Checkbox, SolveReason::CheckboxPassed),
                CaptchaKind::Turnstile => (SolveMethod::AutoPass, SolveReason::AutoPassed),
                CaptchaKind::Arkose => (SolveMethod::Token, SolveReason::AwaitingSubmit),
                _ => unreachable!("unsupported challenge reached flow"),
            };
            ctx.lease
                .set_progress(Some(challenge.kind), SolveStep::Checkbox);
            ctx.events.emit(
                CaptchaEventName::Solving,
                json!({"type":challenge.kind,"method":method,"attempt":1}),
            );
            let earlier_runs = self
                .remedies
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .len();
            let later = self.second.clone().filter(|_| earlier_runs > 0);
            let mut flow = later
                .or_else(|| self.result.clone())
                .unwrap_or_else(|| FlowResult {
                    status: SolveStatus::Solved,
                    method,
                    reason,
                    attempts: Attempts {
                        checkbox: u32::from(method == SolveMethod::Checkbox),
                        ..Default::default()
                    },
                    paid_tasks: 0,
                    provider: None,
                    cost_usd: 0.0,
                    callback_ran: false,
                    message: ctx.user_agent.clone().unwrap_or_default(),
                });
            if let Some(answer) = &self.stall_after {
                ctx.record_payment(&mut flow, &Ok(answer.clone()));
                std::future::pending::<()>().await;
            }
            self.remedies
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(ctx.remedy);
            if let Some(answer) = self
                .pays
                .as_ref()
                .filter(|_| !self.pays_first_only || earlier_runs == 0)
            {
                // Through the round's backend, so a second round's one-task limit applies.
                let task = types::SolveTask::TurnstileToken(types::TurnstileParams {
                    website_url: ctx.page_url.clone(),
                    website_key: "sitekey".into(),
                    action: None,
                    cdata: None,
                });
                let reply = match ctx.backend.solve(&task, ctx.deadline, ctx.cancel).await {
                    Err(error) if error.kind == types::ProviderErrorKind::NotConfigured => {
                        Ok(answer.clone())
                    }
                    reply => reply,
                };
                ctx.record_payment(&mut flow, &reply);
            }
            flow
        }
        async fn standing<P: CaptchaPage>(
            &self,
            page: &P,
            claim: &verify::Claim<'_>,
            judge: Option<&dyn judge::Judge>,
            budget: &mut verify::VerifyBudget,
            cancel: &CancellationToken,
        ) -> Result<verify::Verification, PageError> {
            self.judged
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(judge.is_some());
            self.applied_seen
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(claim.applied.map(|token| token.mark.clone()));
            if let Some(error) = &self.check_error {
                return Err(error.clone());
            }
            let scripted = self
                .findings
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop_front();
            match scripted {
                Some(finding) => {
                    if finding.tier == types::VerifyTier::Jev {
                        budget.judge_calls_left = budget.judge_calls_left.saturating_sub(1);
                    }
                    Ok(finding)
                }
                None => verify::standing(page, claim, judge, budget, cancel).await,
            }
        }
    }
    pub(super) fn service() -> CaptchaSolver {
        CaptchaSolver::unkeyed()
    }
    fn scan(challenges: &[(CaptchaKind, ChallengeState)], complete: bool) -> Detection {
        Detection {
            outcome: if challenges.is_empty() {
                ScanOutcome::None
            } else {
                ScanOutcome::Detected
            },
            complete,
            document: document("current"),
            cloudflare_wall: false,
            challenges: challenges
                .iter()
                .map(|&(kind, state)| Challenge {
                    kind,
                    state,
                    frame: None,
                    challenge_frame: None,
                    host_frame: None,
                    sitekey: None,
                    invisible: false,
                    enterprise: false,
                    recaptcha_s: None,
                    action: None,
                    cdata: None,
                    callback: None,
                })
                .collect(),
        }
    }
    async fn run(driver: &FakeDriver, human_help: bool) -> anyhow::Result<SolveOutcome> {
        service()
            .solve_with(
                &FakePage::new(Ok(document("current"))),
                PageId(42),
                CancellationToken::new(),
                human_help,
                tokio::time::Instant::now() + Duration::from_secs(120),
                driver,
            )
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))
    }

    #[tokio::test]
    async fn tests_that_browser_error_detail_reaches_the_outcome_event_and_trace()
    -> anyhow::Result<()> {
        let log = tempfile::NamedTempFile::new()?;
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen()?)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let driver = FakeDriver {
            scans: Mutex::new(VecDeque::from([Err(PageError::Browser(
                "Frame is no longer available: apiKey=test-secret".into(),
            ))])),
            ..FakeDriver::new(scan(&[], true))
        };
        let outcome = run(&driver, false).await?;
        assert_eq!(outcome.reason, SolveReason::BrowserError);
        assert!(outcome.message.contains("Frame is no longer available"));
        assert!(!outcome.message.contains("test-secret"));
        let event = outcome
            .events
            .last()
            .ok_or_else(|| anyhow::anyhow!("missing failure event"))?;
        assert_eq!(event.event, CaptchaEventName::Failed);
        assert_eq!(event.data.get("cause"), Some(&json!(outcome.message)));
        let logs = std::fs::read_to_string(log.path())?;
        for name in ["captcha.failed", "captcha.outcome"] {
            assert!(
                logs.lines().any(|line| {
                    line.contains(name) && line.contains("Frame is no longer available")
                }),
                "missing cause in {name}: {logs}"
            );
        }
        assert!(!logs.contains("test-secret"));
        Ok(())
    }

    #[tokio::test]
    async fn a_paid_solve_names_its_provider_in_the_outcome_event_and_log() -> anyhow::Result<()> {
        let log = tempfile::NamedTempFile::new()?;
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(log.reopen()?)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let driver = FakeDriver {
            result: Some(FlowResult {
                method: SolveMethod::Tiles,
                paid_tasks: 1,
                provider: Some(ProviderName::Capsolver),
                ..terminal(SolveStatus::Solved, SolveReason::TilesAccepted, "")
            }),
            ..FakeDriver::new(scan(
                &[(CaptchaKind::RecaptchaV2, ChallengeState::Active)],
                true,
            ))
        };
        let outcome = run(&driver, false).await?;
        assert_eq!(outcome.provider, Some(ProviderName::Capsolver));
        let event = outcome
            .events
            .last()
            .ok_or_else(|| anyhow::anyhow!("missing solved event"))?;
        assert_eq!(event.event, CaptchaEventName::Solved);
        assert_eq!(event.data["provider"], "capsolver");
        let logs = std::fs::read_to_string(log.path())?;
        assert!(
            logs.lines()
                .any(|line| line.contains("CAPTCHA solve finished")
                    && line.contains(r#"provider="capsolver""#)),
            "{logs}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_token_application_emits_its_event_without_human_handoff()
    -> anyhow::Result<()> {
        for kind in [CaptchaKind::RecaptchaV2, CaptchaKind::Turnstile] {
            for callback_ran in [false, true] {
                for human_help in [false, true] {
                    let driver = FakeDriver {
                        result: Some(FlowResult {
                            status: SolveStatus::TokenApplied,
                            method: SolveMethod::Token,
                            reason: SolveReason::AwaitingSubmit,
                            attempts: Attempts {
                                token: 1,
                                ..Default::default()
                            },
                            paid_tasks: 1,
                            provider: Some(ProviderName::TwoCaptcha),
                            cost_usd: 0.001,
                            callback_ran,
                            message: token_applied_message(callback_ran),
                        }),
                        ..FakeDriver::new(scan(&[(kind, ChallengeState::Active)], true))
                    };
                    let outcome = run(&driver, human_help).await?;
                    assert_eq!(outcome.provider, Some(ProviderName::TwoCaptcha));
                    let event = outcome
                        .events
                        .last()
                        .ok_or_else(|| anyhow::anyhow!("missing event"))?;
                    assert_eq!(event.event, CaptchaEventName::TokenApplied);
                    assert_eq!(
                        event.data,
                        json!({"type":kind,"method":"token",
                        "durationMs":outcome.duration_ms,"callbackRan":callback_ran,"provider":"2captcha"})
                    );
                    assert!(!outcome.events.iter().any(|event| matches!(
                        event.event,
                        CaptchaEventName::Failed | CaptchaEventName::Handoff
                    )));
                    assert!(!outcome.message.contains("request_human_help"));
                    assert!(!outcome.message.contains("tell the user"));
                    assert!(outcome.message.contains("Submit the form"));
                    assert!(outcome.message.contains("call solve_captcha again"));
                }
            }
        }
        Ok(())
    }

    /// A paid token flow on reCAPTCHA (`token_applied`), as the gate's tests see it.
    fn token_flow_driver(findings: Vec<verify::Verification>) -> FakeDriver {
        FakeDriver {
            result: Some(FlowResult {
                status: SolveStatus::TokenApplied,
                method: SolveMethod::Token,
                reason: SolveReason::AwaitingSubmit,
                attempts: Attempts::default(),
                paid_tasks: 0,
                provider: None,
                cost_usd: 0.0,
                callback_ran: true,
                message: token_applied_message(true),
            }),
            pays: Some(ProviderAnswer {
                answer: SolveAnswer::Token("token".into()),
                provider: ProviderName::Capsolver,
                task_id: None,
                cost_usd: Some(0.001),
            }),
            findings: Mutex::new(findings.into()),
            ..FakeDriver::new(scan(
                &[(CaptchaKind::RecaptchaV2, ChallengeState::Active)],
                true,
            ))
        }
    }

    /// A checkbox pass (`solved`), whose claim rests on the widget, not on a token.
    fn checkbox_flow_driver(findings: Vec<verify::Verification>) -> FakeDriver {
        FakeDriver {
            result: Some(FlowResult {
                status: SolveStatus::Solved,
                method: SolveMethod::Checkbox,
                reason: SolveReason::CheckboxPassed,
                attempts: Attempts::default(),
                paid_tasks: 0,
                provider: None,
                cost_usd: 0.0,
                callback_ran: false,
                message: "The CAPTCHA is solved.".into(),
            }),
            ..token_flow_driver(findings)
        }
    }

    async fn run_checked(
        solver: &CaptchaSolver,
        page: &FakePage,
        driver: &FakeDriver,
    ) -> anyhow::Result<SolveOutcome> {
        solver
            .solve_with(
                page,
                PageId(42),
                CancellationToken::new(),
                true,
                tokio::time::Instant::now() + Duration::from_secs(120),
                driver,
            )
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))
    }

    fn checking(mode: config::VerifyMode) -> CaptchaSolver {
        CaptchaSolver {
            verify: mode,
            ..CaptchaSolver::unkeyed()
        }
    }

    fn verified_rounds(outcome: &SolveOutcome) -> Vec<Value> {
        outcome
            .events
            .iter()
            .filter(|event| event.event == CaptchaEventName::Verified)
            .map(|event| event.data["round"].clone())
            .collect()
    }

    #[tokio::test]
    async fn tests_that_a_confirmed_claim_passes_with_its_report() -> anyhow::Result<()> {
        use types::{VerifyResult, VerifyTier};
        let driver = token_flow_driver(vec![FakeDriver::finding(
            verify::Standing::Clear,
            VerifyTier::Deterministic,
        )]);
        let outcome = run_checked(
            &checking(config::VerifyMode::On),
            &FakePage::new(Ok(document("current"))),
            &driver,
        )
        .await?;
        assert_eq!(
            (outcome.status, outcome.reason),
            (SolveStatus::TokenApplied, SolveReason::AwaitingSubmit)
        );
        assert_eq!(
            outcome.verify.map(|report| (report.result, report.rounds)),
            Some((VerifyResult::Clear, 1))
        );
        assert_eq!(verified_rounds(&outcome), [json!(0)]);
        assert_eq!(outcome.paid_tasks, 1);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_proven_block_gets_one_remedy_round_then_still_blocked()
    -> anyhow::Result<()> {
        use types::{BlockCause, VerifyResult, VerifyTier};
        let blocked = || {
            FakeDriver::finding(
                verify::Standing::Blocked(BlockCause::ChallengeOpen),
                VerifyTier::Deterministic,
            )
        };
        let driver = token_flow_driver(vec![blocked(), blocked()]);
        let outcome = run_checked(
            &checking(config::VerifyMode::On),
            &FakePage::new(Ok(document("current"))),
            &driver,
        )
        .await?;
        assert_eq!(
            (outcome.status, outcome.reason),
            (SolveStatus::NeedsHuman, SolveReason::StillBlocked)
        );
        assert_eq!(outcome.verdict(), types::Verdict::Unsolved);
        assert_eq!(
            *driver.remedies.lock().unwrap_or_else(|e| e.into_inner()),
            [None, Some(verify::Remedy::DismissChallenge)],
            "the second round is told what the check found"
        );
        assert_eq!(verified_rounds(&outcome), [json!(0), json!(1)]);
        assert_eq!(
            outcome.verify.map(|report| (report.result, report.rounds)),
            Some((VerifyResult::Blocked, 2))
        );
        assert_eq!(outcome.paid_tasks, 2, "one token per round, never more");
        assert_eq!(
            outcome.attempts.checkbox, 2,
            "attempts add up across rounds"
        );
        assert!(outcome.message.contains("request_human_help"));
        assert!(
            outcome
                .events
                .iter()
                .any(|event| event.event == CaptchaEventName::Handoff)
        );

        // A block the remedy fixes ends in the flow's own claim.
        let driver = token_flow_driver(vec![
            blocked(),
            FakeDriver::finding(verify::Standing::Clear, VerifyTier::Deterministic),
        ]);
        let outcome = run_checked(
            &checking(config::VerifyMode::On),
            &FakePage::new(Ok(document("current"))),
            &driver,
        )
        .await?;
        assert_eq!(outcome.status, SolveStatus::TokenApplied);
        assert_eq!(
            outcome.verify.map(|report| (report.result, report.rounds)),
            Some((VerifyResult::Clear, 2))
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_judge_only_block_hands_off_without_a_retry_or_money() -> anyhow::Result<()>
    {
        use types::{BlockCause, VerifyTier};
        let driver = checkbox_flow_driver(vec![FakeDriver::finding(
            verify::Standing::Blocked(BlockCause::SiteRejected),
            VerifyTier::Jev,
        )]);
        let outcome = run_checked(
            &checking(config::VerifyMode::On),
            &FakePage::new(Ok(document("current"))),
            &driver,
        )
        .await?;
        assert_eq!(
            (outcome.status, outcome.reason),
            (SolveStatus::NeedsHuman, SolveReason::StillBlocked)
        );
        assert_eq!(
            *driver.remedies.lock().unwrap_or_else(|e| e.into_inner()),
            [None]
        );
        assert_eq!(
            outcome.paid_tasks, 1,
            "no paid task after a judge's reading"
        );
        assert_eq!(
            outcome
                .verify
                .map(|report| (report.tier, report.judge_calls)),
            Some((VerifyTier::Jev, 1))
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_unsure_check_is_unknown_and_shadow_keeps_the_claim() -> anyhow::Result<()>
    {
        use types::{BlockCause, VerifyResult, VerifyTier};
        let unsure = || {
            vec![FakeDriver::finding(
                verify::Standing::Unsure(verify::Unsure::JudgeUnavailable),
                VerifyTier::Deterministic,
            )]
        };
        let outcome = run_checked(
            &checking(config::VerifyMode::On),
            &FakePage::new(Ok(document("current"))),
            &checkbox_flow_driver(unsure()),
        )
        .await?;
        assert_eq!(
            (outcome.status, outcome.reason, outcome.verdict()),
            (
                SolveStatus::Failed,
                SolveReason::AcceptanceUnconfirmed,
                types::Verdict::Unknown
            ),
            "an unavailable judge never clears and never blocks"
        );
        // A token waiting for the visitor's submit keeps its next step: only tier-1 proof
        // overrides `token_applied`, and the report says the check was unsure.
        let outcome = run_checked(
            &checking(config::VerifyMode::On),
            &FakePage::new(Ok(document("current"))),
            &token_flow_driver(unsure()),
        )
        .await?;
        assert_eq!(
            (outcome.status, outcome.verify.map(|report| report.result)),
            (SolveStatus::TokenApplied, Some(VerifyResult::Unsure))
        );

        let driver = token_flow_driver(vec![FakeDriver::finding(
            verify::Standing::Blocked(BlockCause::ChallengeOpen),
            VerifyTier::Deterministic,
        )]);
        let outcome = run_checked(
            &checking(config::VerifyMode::Shadow),
            &FakePage::new(Ok(document("current"))),
            &driver,
        )
        .await?;
        assert_eq!(
            outcome.status,
            SolveStatus::TokenApplied,
            "shadow changes no answer"
        );
        assert_eq!(
            outcome.verify.map(|report| report.result),
            Some(VerifyResult::Blocked)
        );
        assert_eq!(
            *driver.remedies.lock().unwrap_or_else(|e| e.into_inner()),
            [None]
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_policy_wall_short_circuits_the_rounds_and_the_check() -> anyhow::Result<()>
    {
        let page = FakePage {
            notice: Some(
                json!({"title": "Access restricted: unauthorized AI agent", "host": "shop.test"}),
            ),
            ..FakePage::new(Ok(document("current")))
        };
        let driver = token_flow_driver(Vec::new());
        let outcome = run_checked(&checking(config::VerifyMode::On), &page, &driver).await?;
        assert_eq!(
            (outcome.status, outcome.reason),
            (SolveStatus::PolicyWall, SolveReason::PolicyWall)
        );
        assert!(
            driver
                .remedies
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty()
        );
        assert!(
            driver
                .judged
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty()
        );
        assert!(verified_rounds(&outcome).is_empty());
        assert_eq!(outcome.paid_tasks, 0);
        assert_eq!(outcome.verify, None);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_host_judge_reaches_the_check() -> anyhow::Result<()> {
        struct Silent;
        impl judge::Judge for Silent {
            fn ask<'a>(
                &'a self,
                _: &'a judge::JudgeRequest,
                _: Duration,
            ) -> futures_util::future::BoxFuture<'a, Result<judge::JudgeReply, judge::JudgeFault>>
            {
                Box::pin(std::future::ready(Err(judge::JudgeFault::Unavailable(
                    "silent",
                ))))
            }
        }
        let solver = CaptchaSolver {
            judge: Some(Arc::new(Silent)),
            ..checking(config::VerifyMode::On)
        };
        let driver = token_flow_driver(vec![FakeDriver::finding(
            verify::Standing::Clear,
            types::VerifyTier::Deterministic,
        )]);
        run_checked(&solver, &FakePage::new(Ok(document("current"))), &driver).await?;
        assert_eq!(
            *driver.judged.lock().unwrap_or_else(|e| e.into_inner()),
            [true]
        );
        Ok(())
    }

    /// The re-solve's one paid task is counted on what was charged. When the round's own ceiling
    /// stops the flow, the answer is the truth about the page (`still_blocked`), not "budget used
    /// up"; and the report counts both rounds.
    #[tokio::test]
    async fn tests_that_the_retry_ceiling_reads_still_blocked_with_both_rounds_counted()
    -> anyhow::Result<()> {
        use types::{BlockCause, VerifyTier};
        let driver = FakeDriver {
            second: Some(FlowResult {
                status: SolveStatus::NeedsHuman,
                method: SolveMethod::Token,
                reason: SolveReason::SpendLimitReached,
                attempts: Attempts::default(),
                paid_tasks: 0,
                provider: None,
                cost_usd: 0.0,
                callback_ran: false,
                message: "the verify re-solve's paid task is used".into(),
            }),
            ..token_flow_driver(vec![FakeDriver::finding(
                verify::Standing::Blocked(BlockCause::ChallengeOpen),
                VerifyTier::Deterministic,
            )])
        };
        let outcome = run_checked(
            &checking(config::VerifyMode::On),
            &FakePage::new(Ok(document("current"))),
            &driver,
        )
        .await?;
        assert_eq!(
            (outcome.status, outcome.reason, outcome.paid_tasks),
            (SolveStatus::NeedsHuman, SolveReason::StillBlocked, 2)
        );
        assert_eq!(
            outcome.verify.map(|report| report.rounds),
            Some(2),
            "round two ran, though only round one was checked"
        );
        Ok(())
    }

    /// The check learns which token is Neo's: the one this solve bought, by fingerprint, and on
    /// a later call the one an earlier call applied to the same document and family.
    #[tokio::test]
    async fn tests_that_the_check_is_told_which_token_neo_applied() -> anyhow::Result<()> {
        use types::VerifyTier;
        let clear = || {
            vec![FakeDriver::finding(
                verify::Standing::Clear,
                VerifyTier::Deterministic,
            )]
        };
        let solver = checking(config::VerifyMode::On);
        let mark = Some(verify::token_mark("token"));
        let first = token_flow_driver(clear());
        run_checked(&solver, &FakePage::new(Ok(document("current"))), &first).await?;
        assert_eq!(
            first
                .applied_seen
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_slice(),
            std::slice::from_ref(&mark)
        );

        let again = FakeDriver {
            findings: Mutex::new(clear().into()),
            ..FakeDriver::new(scan(
                &[(CaptchaKind::RecaptchaV2, ChallengeState::Passed)],
                true,
            ))
        };
        let outcome = run_checked(&solver, &FakePage::new(Ok(document("current"))), &again).await?;
        assert_eq!(outcome.reason, SolveReason::AlreadyPassed);
        assert_eq!(
            *again.applied_seen.lock().unwrap_or_else(|e| e.into_inner()),
            [mark],
            "an `already_passed` over Neo's own token is checked as Neo's token"
        );

        let mut moved = scan(&[(CaptchaKind::RecaptchaV2, ChallengeState::Passed)], true);
        moved.document = document("next");
        let elsewhere = FakeDriver {
            findings: Mutex::new(clear().into()),
            ..FakeDriver::new(moved)
        };
        run_checked(&solver, &FakePage::new(Ok(document("next"))), &elsewhere).await?;
        assert_eq!(
            *elsewhere
                .applied_seen
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
            [None],
            "a token belongs to the document it was applied to"
        );
        Ok(())
    }

    /// A rescan that routes another family in round two gets no token of round one's.
    #[tokio::test]
    async fn tests_that_a_token_is_named_only_for_its_own_family_and_round() -> anyhow::Result<()> {
        use types::{BlockCause, VerifyTier};
        let mut driver = FakeDriver {
            pays_first_only: true,
            ..token_flow_driver(vec![
                FakeDriver::finding(
                    verify::Standing::Blocked(BlockCause::ChallengeOpen),
                    VerifyTier::Deterministic,
                ),
                FakeDriver::finding(verify::Standing::Clear, VerifyTier::Deterministic),
            ])
        };
        driver.scans = Mutex::new(VecDeque::from([
            Ok(scan(
                &[(CaptchaKind::RecaptchaV2, ChallengeState::Active)],
                true,
            )),
            Ok(scan(
                &[(CaptchaKind::Turnstile, ChallengeState::Active)],
                true,
            )),
        ]));
        run_checked(
            &checking(config::VerifyMode::On),
            &FakePage::new(Ok(document("current"))),
            &driver,
        )
        .await?;
        assert_eq!(
            *driver
                .applied_seen
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
            [Some(verify::token_mark("token")), None],
            "the Turnstile claim must not rest on the reCAPTCHA token"
        );
        Ok(())
    }

    /// The page brief taken before a flow is bounded: a page whose scripts stall costs the
    /// solve 1.5 s, never its deadline.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_stalled_page_brief_costs_at_most_its_budget() -> anyhow::Result<()> {
        let page = FakePage {
            dom_stalls: true,
            ..FakePage::new(Ok(document("current")))
        };
        let driver = token_flow_driver(vec![FakeDriver::finding(
            verify::Standing::Clear,
            types::VerifyTier::Deterministic,
        )]);
        let started = Instant::now();
        let outcome = run_checked(&checking(config::VerifyMode::On), &page, &driver).await?;
        assert_eq!(outcome.status, SolveStatus::TokenApplied);
        assert!(started.elapsed() >= BEFORE_BRIEF_BUDGET);
        assert!(started.elapsed() < BEFORE_BRIEF_BUDGET + Duration::from_secs(1));
        Ok(())
    }

    /// Arkose keeps its token outside the fields the check reads. A field that is not there
    /// proves nothing, so the claim keeps telling the agent to submit, and no money moves.
    #[tokio::test]
    async fn tests_that_an_arkose_token_applied_keeps_its_answer() -> anyhow::Result<()> {
        let page = FakePage {
            token_marks: Some(json!([])),
            ..FakePage::new(Ok(document("current")))
        };
        let mut driver = token_flow_driver(Vec::new());
        // Arkose's flow declares no callback, so nothing excuses a missing field but the rule
        // that a field which is not there proves nothing.
        if let Some(result) = &mut driver.result {
            result.callback_ran = false;
        }
        driver.scans = Mutex::new(VecDeque::from([Ok(scan(
            &[(CaptchaKind::Arkose, ChallengeState::Active)],
            true,
        ))]));
        let outcome = run_checked(&checking(config::VerifyMode::On), &page, &driver).await?;
        assert_eq!(
            (outcome.status, outcome.reason, outcome.paid_tasks),
            (SolveStatus::TokenApplied, SolveReason::AwaitingSubmit, 1)
        );
        assert_eq!(
            *driver.remedies.lock().unwrap_or_else(|e| e.into_inner()),
            [None],
            "no second round"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_remedy_reaches_only_the_family_it_was_chosen_for() -> anyhow::Result<()> {
        use types::{BlockCause, VerifyTier};
        let mut driver = token_flow_driver(vec![
            FakeDriver::finding(
                verify::Standing::Blocked(BlockCause::ChallengeOpen),
                VerifyTier::Deterministic,
            ),
            FakeDriver::finding(verify::Standing::Clear, VerifyTier::Deterministic),
        ]);
        driver.scans = Mutex::new(VecDeque::from([
            Ok(scan(
                &[(CaptchaKind::RecaptchaV2, ChallengeState::Active)],
                true,
            )),
            Ok(scan(
                &[(CaptchaKind::Turnstile, ChallengeState::Active)],
                true,
            )),
        ]));
        run_checked(
            &checking(config::VerifyMode::On),
            &FakePage::new(Ok(document("current"))),
            &driver,
        )
        .await?;
        assert_eq!(
            *driver.remedies.lock().unwrap_or_else(|e| e.into_inner()),
            [None, None],
            "a reCAPTCHA fix is not handed to the Turnstile flow the rescan found"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_check_cut_short_keeps_the_claim_only_in_shadow() -> anyhow::Result<()> {
        for (mode, status) in [
            (config::VerifyMode::Shadow, SolveStatus::TokenApplied),
            (config::VerifyMode::On, SolveStatus::Cancelled),
        ] {
            let driver = FakeDriver {
                check_error: Some(PageError::Navigated),
                ..token_flow_driver(Vec::new())
            };
            let outcome = run_checked(
                &checking(mode),
                &FakePage::new(Ok(document("current"))),
                &driver,
            )
            .await?;
            assert_eq!(outcome.status, status, "{mode:?}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_an_unrouted_success_claim_is_never_passed_unchecked() -> anyhow::Result<()>
    {
        let solver = checking(config::VerifyMode::On);
        let reservations = Arc::<TabReservations>::default();
        let lease = reservations
            .try_reserve(42, "cap_unrouted")
            .map_err(|_| anyhow::anyhow!("busy"))?;
        let events = EventLog::new(CaptchaEventBus::new(), "cap_unrouted".into(), 42);
        let payments = Arc::new(Mutex::new(Payments::default()));
        let cancel = CancellationToken::new();
        let rc = RoundCtx {
            page: 42,
            lease: &lease,
            events: &events,
            payments: &payments,
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(60),
            routed: Mutex::new(None),
        };
        let claim = terminal(SolveStatus::AlreadyClear, SolveReason::NoCaptcha, "clear");
        let mut budget = verify::VerifyBudget {
            until: Instant::now() + Duration::from_secs(60),
            judge_calls_left: 0,
        };
        let driver = FakeDriver::new(scan(&[], true));
        let page = FakePage::new(Ok(document("current")));
        let Checked::Done(answer, _) = solver
            .check_claim(&page, &driver, &rc, None, claim, 0, &mut budget)
            .await
        else {
            anyhow::bail!("an unrouted claim never asks for another round");
        };
        assert_eq!(
            (answer.status, answer.reason),
            (SolveStatus::Failed, SolveReason::AcceptanceUnconfirmed)
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_failed_policy_read_is_no_wall_and_the_solve_goes_on() -> anyhow::Result<()>
    {
        let page = FakePage {
            notice_fails: true,
            ..FakePage::new(Ok(document("current")))
        };
        let driver = token_flow_driver(Vec::new());
        let outcome = run_checked(&checking(config::VerifyMode::Off), &page, &driver).await?;
        assert_eq!(outcome.status, SolveStatus::TokenApplied);
        Ok(())
    }

    #[test]
    fn tests_that_page_read_failures_start_with_the_shared_wording() -> anyhow::Result<()> {
        for error in [
            PageError::Timeout,
            PageError::Browser("Frame was detached".into()),
        ] {
            assert!(
                page_failure(error).message.starts_with(PAGE_CHECK_FAILED),
                "neo-server passes on exactly these lines"
            );
        }
        Ok(())
    }

    #[test]
    fn tests_that_flows_get_less_time_only_while_the_check_runs() -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(120);
        assert_eq!(
            checking(config::VerifyMode::Off).flow_deadline(deadline),
            deadline
        );
        for mode in [config::VerifyMode::On, config::VerifyMode::Shadow] {
            assert_eq!(
                checking(mode).flow_deadline(deadline),
                deadline - VERIFY_RESERVE
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_the_verify_gate_keeps_unconfirmed_claims_truthful() -> anyhow::Result<()> {
        let applied = FlowResult {
            status: SolveStatus::TokenApplied,
            method: SolveMethod::Token,
            reason: SolveReason::AwaitingSubmit,
            attempts: Attempts::default(),
            paid_tasks: 1,
            provider: Some(ProviderName::TwoCaptcha),
            cost_usd: 0.001,
            callback_ran: true,
            message: token_applied_message(true),
        };
        let mut seen = Vec::new();
        for mode in [
            config::VerifyMode::Off,
            config::VerifyMode::Shadow,
            config::VerifyMode::On,
        ] {
            let solver = CaptchaSolver {
                verify: mode,
                ..CaptchaSolver::unkeyed()
            };
            let driver = FakeDriver {
                result: Some(applied.clone()),
                ..FakeDriver::new(scan(
                    &[(CaptchaKind::RecaptchaV2, ChallengeState::Active)],
                    true,
                ))
            };
            let outcome = solver
                .solve_with(
                    &FakePage::new(Ok(document("current"))),
                    PageId(42),
                    CancellationToken::new(),
                    false,
                    tokio::time::Instant::now() + Duration::from_secs(120),
                    &driver,
                )
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            let verified = outcome
                .events
                .iter()
                .any(|event| event.event == CaptchaEventName::Verified);
            seen.push((
                mode,
                outcome.status,
                outcome.reason,
                outcome.verify.map(|v| v.result),
                verified,
                outcome.paid_tasks,
            ));
        }
        // The fake page has nothing the check can read, so the check is unsure. A token waiting
        // for the visitor's submit keeps `token_applied` on unsure (only tier-1 proof overrides
        // it); `shadow` keeps every claim. Both report the check beside the answer.
        assert_eq!(
            seen,
            [
                (
                    config::VerifyMode::Off,
                    SolveStatus::TokenApplied,
                    SolveReason::AwaitingSubmit,
                    None,
                    false,
                    1
                ),
                (
                    config::VerifyMode::Shadow,
                    SolveStatus::TokenApplied,
                    SolveReason::AwaitingSubmit,
                    Some(types::VerifyResult::Unsure),
                    true,
                    1
                ),
                (
                    config::VerifyMode::On,
                    SolveStatus::TokenApplied,
                    SolveReason::AwaitingSubmit,
                    Some(types::VerifyResult::Unsure),
                    true,
                    1
                ),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_complete_empty_scan_has_the_contract_shape_and_no_events()
    -> anyhow::Result<()> {
        let outcome = run(&FakeDriver::new(scan(&[], true)), false).await?;
        assert_eq!(
            (outcome.status, outcome.reason),
            (SolveStatus::AlreadyClear, SolveReason::NoCaptcha)
        );
        assert!(outcome.events.is_empty());
        assert!(outcome.request_id.starts_with("cap_"));
        ulid::Ulid::from_string(&outcome.request_id[4..])?;
        let value = serde_json::to_value(&outcome)?;
        assert_eq!(value.as_object().map(|object| object.len()), Some(13));
        assert_eq!(value.get("provider"), Some(&Value::Null));
        assert_eq!(value["type"], Value::Null);
        assert_eq!(
            value["attempts"],
            json!({"checkbox":0,"tileRounds":0,"token":0})
        );
        assert_eq!(value["paidTasks"], 0);
        assert_eq!(value["costUsd"], 0.0);
        assert_eq!(value["page"], 42);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_startup_force_method_reaches_the_selected_flow() -> anyhow::Result<()> {
        let key = tempfile::NamedTempFile::new()?;
        std::fs::write(key.path(), "test-key")?;
        let config = config::CaptchaConfig::from_lookup(|name| match name {
            config::CAPSOLVER_KEY_FILE_ENV => Some(key.path().to_string_lossy().into_owned()),
            config::TEST_FORCE_METHOD_ENV => Some("token".into()),
            _ => None,
        });
        let service =
            CaptchaSolver::new(config).ok_or_else(|| anyhow::anyhow!("missing service"))?;
        // The check is on by default; this test is about the flow, so the check finds it clear.
        let driver = FakeDriver {
            expected_forced_method: Some(config::ForcedMethod::Token),
            findings: Mutex::new(VecDeque::from([FakeDriver::finding(
                verify::Standing::Clear,
                types::VerifyTier::Deterministic,
            )])),
            ..FakeDriver::new(scan(
                &[(CaptchaKind::RecaptchaV2, ChallengeState::Active)],
                true,
            ))
        };
        let outcome = service
            .solve_with(
                &FakePage::new(Ok(document("current"))),
                PageId(42),
                CancellationToken::new(),
                false,
                Instant::now() + Duration::from_secs(120),
                &driver,
            )
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(outcome.status, SolveStatus::Solved);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_incomplete_scans_never_claim_the_page_is_clear() -> anyhow::Result<()> {
        for challenges in [
            vec![],
            vec![(CaptchaKind::RecaptchaV2, ChallengeState::Passed)],
        ] {
            let outcome = run(&FakeDriver::new(scan(&challenges, false)), false).await?;
            assert_eq!(
                (outcome.status, outcome.reason),
                (SolveStatus::Failed, SolveReason::ScanIncomplete)
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_new_active_types_stay_unsupported_without_payment_or_clicks()
    -> anyhow::Result<()> {
        for kind in [
            CaptchaKind::Geetest,
            CaptchaKind::AwsWaf,
            CaptchaKind::Arkose,
        ]
        .into_iter()
        .filter(|kind| !kind.is_supported())
        {
            let service = service();
            let page = FakePage::new(Ok(document("current")));
            let driver = FakeDriver::new(scan(&[(kind, ChallengeState::Active)], true));
            let outcome = service
                .solve_with(
                    &page,
                    PageId(42),
                    CancellationToken::new(),
                    false,
                    Instant::now() + Duration::from_secs(120),
                    &driver,
                )
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            assert_eq!(
                (outcome.status, outcome.reason),
                (SolveStatus::Unsupported, SolveReason::UnsupportedType)
            );
            assert_eq!(outcome.kind, Some(kind));
            assert_eq!(outcome.attempts, Attempts::default());
            assert_eq!(outcome.paid_tasks, 0);
            assert_eq!(service.providers.paid_tasks(), 0);
            assert_eq!(page.clicks.load(Ordering::Relaxed), 0);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_pages_without_frame_main_support_return_the_default_error() {
        let page = FakePage::new(Ok(document("current")));
        let frame = FrameRef {
            frame_id: "child".into(),
            url: "https://fixture.test".into(),
            rect: None,
        };
        assert_eq!(
            page.eval_in_frame_main(&frame, "1").await,
            Err(PageError::Browser(
                "main-world frame evaluation unavailable".into()
            ))
        );
    }

    #[tokio::test]
    async fn tests_that_active_hcaptcha_is_unsupported_and_has_ordered_events() -> anyhow::Result<()>
    {
        let outcome = run(
            &FakeDriver::new(scan(
                &[(CaptchaKind::Hcaptcha, ChallengeState::Active)],
                true,
            )),
            true,
        )
        .await?;
        assert_eq!(
            (outcome.status, outcome.reason),
            (SolveStatus::Unsupported, SolveReason::UnsupportedType)
        );
        assert!(outcome.message.contains("request_human_help"));
        assert_eq!(
            outcome
                .events
                .iter()
                .map(|event| event.event)
                .collect::<Vec<_>>(),
            [CaptchaEventName::Detected, CaptchaEventName::Failed]
        );
        assert_eq!(
            outcome.events[0].data,
            json!({"type":"hcaptcha","state":"active","complete":true,"challenges":1})
        );
        assert_eq!(
            outcome.events[1].data,
            json!({"type":"hcaptcha","status":"unsupported","reason":"unsupported_type","provider":null})
        );
        assert!(
            outcome
                .events
                .iter()
                .all(|event| event.page == 42 && event.request_id == outcome.request_id)
        );
        Ok(())
    }

    /// `PerimeterX`'s check covers the page: it wins over a solvable widget under it, even on an
    /// incomplete scan, and is refused without a click or a provider call. Its wire `type` stays
    /// `unknown` (claw-server's closed schema), so the message and the events name the family.
    #[tokio::test]
    async fn tests_that_perimeterx_is_refused_by_name_over_a_widget_it_covers() -> anyhow::Result<()>
    {
        for (human_help, next) in [
            (true, "Call request_human_help with kind \"captcha\"."),
            (
                false,
                "Stop and tell the user that this CAPTCHA needs their help.",
            ),
        ] {
            let driver = FakeDriver::new(scan(
                &[
                    (CaptchaKind::Turnstile, ChallengeState::Active),
                    (CaptchaKind::PerimeterX, ChallengeState::Active),
                ],
                false,
            ));
            let outcome = run(&driver, human_help).await?;
            assert_eq!(
                (outcome.status, outcome.reason, outcome.kind),
                (
                    SolveStatus::Unsupported,
                    SolveReason::UnsupportedType,
                    Some(CaptchaKind::PerimeterX)
                )
            );
            assert_eq!(outcome.verdict(), types::Verdict::Unsolved);
            assert_eq!(
                outcome.message,
                format!(
                    "PerimeterX's press-and-hold human check covers the page, and only a person \
                     can pass it. {next}"
                )
            );
            assert_eq!(
                (outcome.attempts, outcome.paid_tasks),
                (Attempts::default(), 0)
            );
            let wire = serde_json::to_value(&outcome)?;
            assert_eq!(wire.get("type"), Some(&json!("unknown")));
            assert_eq!(
                outcome
                    .events
                    .iter()
                    .map(|event| event.data.clone())
                    .collect::<Vec<_>>(),
                [
                    json!({"type":"unknown","state":"active","complete":false,"challenges":2,
                           "family":"perimeterx"}),
                    json!({"type":"unknown","status":"unsupported","reason":"unsupported_type",
                           "family":"perimeterx","provider":null}),
                ]
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_passed_and_nonblocking_widgets_are_already_clear() -> anyhow::Result<()> {
        for (kind, state, reason) in [
            (
                CaptchaKind::Hcaptcha,
                ChallengeState::Passed,
                SolveReason::AlreadyPassed,
            ),
            (
                CaptchaKind::RecaptchaV3,
                ChallengeState::WidgetOnly,
                SolveReason::NotBlocking,
            ),
        ] {
            let outcome = run(&FakeDriver::new(scan(&[(kind, state)], true)), false).await?;
            assert_eq!(
                (outcome.status, outcome.reason),
                (SolveStatus::AlreadyClear, reason)
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_supported_active_challenges_take_priority_and_reach_their_flow()
    -> anyhow::Result<()> {
        for (kind, method) in [
            (CaptchaKind::RecaptchaV2, SolveMethod::Checkbox),
            (CaptchaKind::Turnstile, SolveMethod::AutoPass),
        ] {
            let outcome = run(
                &FakeDriver::new(scan(
                    &[
                        (CaptchaKind::Hcaptcha, ChallengeState::Active),
                        (kind, ChallengeState::Active),
                    ],
                    true,
                )),
                false,
            )
            .await?;
            assert_eq!(
                (outcome.status, outcome.kind, outcome.method),
                (SolveStatus::Solved, Some(kind), method)
            );
            assert_eq!(outcome.message, "fake-agent");
            assert_eq!(outcome.provider, None);
            assert_eq!(
                outcome
                    .events
                    .iter()
                    .map(|event| event.event)
                    .collect::<Vec<_>>(),
                [
                    CaptchaEventName::Detected,
                    CaptchaEventName::Solving,
                    CaptchaEventName::Solved
                ]
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_stale_detection_is_rescanned_once_before_routing() -> anyhow::Result<()> {
        let mut stale = scan(&[(CaptchaKind::Hcaptcha, ChallengeState::Active)], true);
        stale.document = document("old");
        let driver = FakeDriver {
            scans: Mutex::new(VecDeque::from([Ok(stale.clone()), Ok(scan(&[], true))])),
            ..FakeDriver::new(scan(&[], true))
        };
        assert_eq!(run(&driver, false).await?.reason, SolveReason::NoCaptcha);
        let driver = FakeDriver {
            scans: Mutex::new(VecDeque::from([Ok(stale.clone()), Ok(stale)])),
            ..FakeDriver::new(scan(&[], true))
        };
        assert_eq!(run(&driver, false).await?.reason, SolveReason::Navigated);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_busy_pages_are_refused_without_replacing_the_lease() -> anyhow::Result<()> {
        let service = service();
        let lease = service
            .reservations
            .try_reserve(42, "cap_existing")
            .map_err(|_| anyhow::anyhow!("busy"))?;
        let result = service
            .solve_with(
                &FakePage::new(Ok(document("current"))),
                PageId(42),
                CancellationToken::new(),
                false,
                tokio::time::Instant::now() + Duration::from_secs(120),
                &FakeDriver::new(scan(&[], true)),
            )
            .await;
        assert!(
            matches!(result,Err(SolveRefusal::Busy(active)) if active.request_id == lease.request_id())
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_cancellation_and_deadlines_release_reservations() -> anyhow::Result<()> {
        for cancelled in [false, true] {
            let service = service();
            let cancel = CancellationToken::new();
            let driver = FakeDriver {
                stall_after: Some(ProviderAnswer {
                    answer: SolveAnswer::Tiles(vec![]),
                    provider: ProviderName::TwoCaptcha,
                    task_id: None,
                    cost_usd: Some(0.002),
                }),
                ..FakeDriver::new(scan(
                    &[(CaptchaKind::RecaptchaV2, ChallengeState::Active)],
                    true,
                ))
            };
            if cancelled {
                let token = cancel.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    token.cancel();
                });
            }
            let outcome = service
                .solve_with(
                    &FakePage::new(Ok(document("current"))),
                    PageId(42),
                    cancel,
                    false,
                    tokio::time::Instant::now() + Duration::from_millis(10),
                    &driver,
                )
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            assert_eq!(
                outcome.reason,
                if cancelled {
                    SolveReason::CallerCancelled
                } else {
                    SolveReason::DeadlineExceeded
                }
            );
            assert_eq!(outcome.attempts.checkbox, 1);
            assert_eq!(outcome.kind, Some(CaptchaKind::RecaptchaV2));
            assert_eq!(outcome.provider, Some(ProviderName::TwoCaptcha));
            assert_eq!(outcome.paid_tasks, 1);
            assert_eq!(outcome.cost_usd, 0.002);
            assert_eq!(
                outcome
                    .events
                    .last()
                    .and_then(|event| event.data.get("provider")),
                Some(&json!("2captcha"))
            );
            assert_eq!(
                outcome.events.last().map(|event| event.event),
                Some(CaptchaEventName::Failed)
            );
            assert!(service.reservations.active(42).is_none());
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_closed_pages_cancel_and_handoffs_explain_the_next_step()
    -> anyhow::Result<()> {
        let outcome = service()
            .solve_with(
                &FakePage::new(Err(PageError::Closed)),
                PageId(42),
                CancellationToken::new(),
                false,
                tokio::time::Instant::now() + Duration::from_secs(120),
                &FakeDriver::new(scan(&[], true)),
            )
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(
            (outcome.status, outcome.reason),
            (SolveStatus::Cancelled, SolveReason::TabClosed)
        );
        for human_help in [false, true] {
            let driver = FakeDriver {
                result: Some(FlowResult {
                    status: SolveStatus::NeedsHuman,
                    method: SolveMethod::Token,
                    reason: SolveReason::ProviderUnavailable,
                    attempts: Attempts {
                        token: 1,
                        ..Default::default()
                    },
                    paid_tasks: 2,
                    provider: Some(ProviderName::Capsolver),
                    cost_usd: 0.002,
                    callback_ran: false,
                    message: "ERROR_ZERO_BALANCE".into(),
                }),
                ..FakeDriver::new(scan(
                    &[(CaptchaKind::Turnstile, ChallengeState::Active)],
                    true,
                ))
            };
            let outcome = run(&driver, human_help).await?;
            assert_eq!(outcome.paid_tasks, 2);
            assert_eq!(outcome.provider, Some(ProviderName::Capsolver));
            assert_eq!(outcome.cost_usd, 0.002);
            assert!(outcome.message.contains("ERROR_ZERO_BALANCE"));
            assert_eq!(outcome.message.contains("request_human_help"), human_help);
            assert_eq!(outcome.message.contains("tell the user"), !human_help);
            assert_eq!(
                outcome.events.last().map(|event| &event.data),
                Some(
                    &json!({"type":"turnstile","reason":"provider_unavailable","humanHelp":human_help,"provider":"capsolver"})
                )
            );
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_loading_supported_widget_is_rescanned_before_routing()
    -> anyhow::Result<()> {
        for (state, complete) in [
            (ChallengeState::Unknown, true),
            (ChallengeState::Unknown, false),
            (ChallengeState::WidgetOnly, false),
        ] {
            let mut driver = FakeDriver::new(scan(&[(CaptchaKind::RecaptchaV2, state)], complete));
            driver.scans = Mutex::new(VecDeque::from([
                Ok(scan(&[(CaptchaKind::RecaptchaV2, state)], complete)),
                Ok(scan(
                    &[(CaptchaKind::RecaptchaV2, ChallengeState::Unknown)],
                    false,
                )),
                Ok(scan(
                    &[(CaptchaKind::RecaptchaV2, ChallengeState::Active)],
                    true,
                )),
            ]));
            let started = tokio::time::Instant::now();
            let outcome = run(&driver, false).await?;
            assert_eq!(outcome.reason, SolveReason::CheckboxPassed);
            assert_eq!(outcome.status, SolveStatus::Solved);
            assert!(started.elapsed() >= Duration::from_secs(1));
            assert!(started.elapsed() < Duration::from_secs(5));
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_loading_widget_can_become_already_passed() -> anyhow::Result<()> {
        let mut driver = FakeDriver::new(scan(
            &[(CaptchaKind::Turnstile, ChallengeState::Unknown)],
            false,
        ));
        driver.scans = Mutex::new(VecDeque::from([
            Ok(scan(
                &[(CaptchaKind::Turnstile, ChallengeState::Unknown)],
                false,
            )),
            Ok(scan(
                &[(CaptchaKind::Turnstile, ChallengeState::Passed)],
                true,
            )),
        ]));
        let outcome = run(&driver, false).await?;
        assert_eq!(outcome.status, SolveStatus::AlreadyClear);
        assert_eq!(outcome.reason, SolveReason::AlreadyPassed);
        assert_eq!(outcome.attempts, Attempts::default());
        Ok(())
    }

    /// A widget still running its own check is waited on like one still loading, and if it is
    /// still checking after the five seconds the answer says so: never clear, never a solve.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_still_checking_widget_is_waited_on_and_never_called_clear()
    -> anyhow::Result<()> {
        for others in [
            vec![],
            vec![(CaptchaKind::RecaptchaV3, ChallengeState::WidgetOnly)],
            vec![(CaptchaKind::Hcaptcha, ChallengeState::Passed)],
        ] {
            let mut challenges = others.clone();
            challenges.push((CaptchaKind::Turnstile, ChallengeState::Pending));
            let started = tokio::time::Instant::now();
            let outcome = run(&FakeDriver::new(scan(&challenges, true)), false).await?;
            assert_eq!(
                (outcome.status, outcome.reason),
                (SolveStatus::Failed, SolveReason::ChallengePending),
                "{others:?}"
            );
            assert_eq!(outcome.message, STILL_CHECKING);
            assert_eq!(outcome.verdict(), types::Verdict::Unknown);
            assert_eq!(outcome.attempts, Attempts::default());
            assert_eq!(outcome.paid_tasks, 0);
            assert_eq!(started.elapsed(), Duration::from_secs(5));
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_still_checking_widget_is_routed_once_it_settles() -> anyhow::Result<()> {
        for (settled, status, reason) in [
            (
                ChallengeState::Passed,
                SolveStatus::AlreadyClear,
                SolveReason::AlreadyPassed,
            ),
            (
                ChallengeState::Active,
                SolveStatus::Solved,
                SolveReason::AutoPassed,
            ),
        ] {
            let mut driver = FakeDriver::new(scan(
                &[(CaptchaKind::Turnstile, ChallengeState::Pending)],
                true,
            ));
            driver.scans = Mutex::new(VecDeque::from([
                Ok(scan(
                    &[(CaptchaKind::Turnstile, ChallengeState::Pending)],
                    true,
                )),
                Ok(scan(
                    &[(CaptchaKind::Turnstile, ChallengeState::Pending)],
                    true,
                )),
                Ok(scan(&[(CaptchaKind::Turnstile, settled)], true)),
            ]));
            let started = tokio::time::Instant::now();
            let outcome = run(&driver, false).await?;
            assert_eq!((outcome.status, outcome.reason), (status, reason));
            assert!(started.elapsed() >= Duration::from_secs(1));
            assert!(started.elapsed() < Duration::from_secs(5));
        }
        Ok(())
    }

    /// A check still running outranks an incomplete scan in the answer: it is what to wait for.
    /// An unsupported widget is not waited on, but is never called clear either.
    #[tokio::test(start_paused = true)]
    async fn tests_that_still_checking_is_the_answer_even_beside_an_incomplete_scan()
    -> anyhow::Result<()> {
        for (kind, complete, waited) in [
            (CaptchaKind::Turnstile, false, Duration::from_secs(5)),
            (CaptchaKind::Hcaptcha, true, Duration::ZERO),
        ] {
            let started = tokio::time::Instant::now();
            let outcome = run(
                &FakeDriver::new(scan(&[(kind, ChallengeState::Pending)], complete)),
                false,
            )
            .await?;
            assert_eq!(
                (outcome.status, outcome.reason),
                (SolveStatus::Failed, SolveReason::ChallengePending),
                "{kind:?}"
            );
            assert_eq!(outcome.message, STILL_CHECKING, "{kind:?}");
            assert_eq!(started.elapsed(), waited, "{kind:?}");
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_unreadable_supported_widgets_get_only_five_seconds_to_load()
    -> anyhow::Result<()> {
        let started = tokio::time::Instant::now();
        let outcome = run(
            &FakeDriver::new(scan(
                &[(CaptchaKind::RecaptchaV2, ChallengeState::Unknown)],
                false,
            )),
            false,
        )
        .await?;
        assert_eq!(outcome.status, SolveStatus::Failed);
        assert_eq!(outcome.reason, SolveReason::ScanIncomplete);
        assert_eq!(outcome.paid_tasks, 0);
        assert_eq!(outcome.attempts, Attempts::default());
        assert_eq!(started.elapsed(), Duration::from_secs(5));
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_readiness_waits_preserve_cancellation_and_the_overall_deadline()
    -> anyhow::Result<()> {
        for cancelled in [false, true] {
            let service = service();
            let cancel = CancellationToken::new();
            if cancelled {
                let token = cancel.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(750)).await;
                    token.cancel();
                });
            }
            let driver = FakeDriver::new(scan(
                &[(CaptchaKind::RecaptchaV2, ChallengeState::Unknown)],
                false,
            ));
            let outcome = service
                .solve_with(
                    &FakePage::new(Ok(document("current"))),
                    PageId(42),
                    cancel,
                    false,
                    tokio::time::Instant::now() + Duration::from_secs(1),
                    &driver,
                )
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            assert_eq!(
                outcome.reason,
                if cancelled {
                    SolveReason::CallerCancelled
                } else {
                    SolveReason::DeadlineExceeded
                }
            );
            assert_eq!(outcome.attempts, Attempts::default());
            assert_eq!(outcome.kind, Some(CaptchaKind::RecaptchaV2));
            assert!(service.reservations.active(42).is_none());
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_readiness_survives_an_empty_incomplete_rescan() -> anyhow::Result<()> {
        let mut driver = FakeDriver::new(scan(
            &[(CaptchaKind::RecaptchaV2, ChallengeState::Unknown)],
            false,
        ));
        driver.scans = Mutex::new(VecDeque::from([
            Ok(scan(
                &[(CaptchaKind::RecaptchaV2, ChallengeState::Unknown)],
                false,
            )),
            Ok(scan(&[], false)),
            Ok(scan(
                &[(CaptchaKind::RecaptchaV2, ChallengeState::Active)],
                true,
            )),
        ]));
        let outcome = run(&driver, false).await?;
        assert_eq!(outcome.reason, SolveReason::CheckboxPassed);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_readiness_navigation_cancels_before_the_flow() -> anyhow::Result<()> {
        let mut next = scan(&[(CaptchaKind::RecaptchaV2, ChallengeState::Passed)], true);
        next.document = document("navigated");
        let mut driver = FakeDriver::new(scan(
            &[(CaptchaKind::RecaptchaV2, ChallengeState::Unknown)],
            false,
        ));
        driver.scans = Mutex::new(VecDeque::from([
            Ok(scan(
                &[(CaptchaKind::RecaptchaV2, ChallengeState::Unknown)],
                false,
            )),
            Ok(next),
        ]));
        let outcome = run(&driver, false).await?;
        assert_eq!(outcome.status, SolveStatus::Cancelled);
        assert_eq!(outcome.reason, SolveReason::Navigated);
        assert_eq!(outcome.attempts, Attempts::default());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_readiness_rescans_cannot_run_past_the_five_second_budget()
    -> anyhow::Result<()> {
        struct SlowDriver(std::sync::atomic::AtomicBool);
        impl SolveDriver for SlowDriver {
            async fn detect<P: CaptchaPage>(&self, _: &P) -> Result<Detection, PageError> {
                if self.0.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    std::future::pending::<()>().await;
                }
                Ok(scan(
                    &[(CaptchaKind::RecaptchaV2, ChallengeState::Unknown)],
                    false,
                ))
            }
            async fn solve<P: CaptchaPage, B: CaptchaBackend>(
                &self,
                _: &SolveCtx<'_, P, B>,
                _: &Challenge,
            ) -> FlowResult {
                panic!("an unreadable widget must not reach a solve flow");
            }
        }
        let started = Instant::now();
        let outcome = service()
            .solve_with(
                &FakePage::new(Ok(document("current"))),
                PageId(42),
                CancellationToken::new(),
                false,
                started + Duration::from_secs(120),
                &SlowDriver(false.into()),
            )
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(outcome.reason, SolveReason::ScanIncomplete);
        assert_eq!(started.elapsed(), Duration::from_secs(5));
        Ok(())
    }
}
