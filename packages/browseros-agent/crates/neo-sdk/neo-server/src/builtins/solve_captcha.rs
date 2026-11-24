//! `neo.solveCaptcha(page)` over neo-captcha (design D10).

use std::panic;
use std::sync::Arc;

use browseros_core::CoreError;
use neo_captcha::types::{
    CaptchaEvent, CaptchaKind, ProviderName, SolveMethod, SolveOutcome, SolveReason,
};
use neo_captcha::{CaptchaSolver, HumanHelp, SolveRefusal};
use neo_trace::Effect;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::captcha_claims::{self, CaptchaClaims};
use super::captcha_solve;
use crate::dispatch::{
    ErrorCode, LeafError, Listing, Method, MethodSpec, OpCtx, OperationError, PageTarget,
    RegistryBuilder, RegistryError,
};

/// The method's own line in the `run` description, shown only when this Neo has a provider key.
/// Its time limit is neo-captcha's [`neo_captcha::SOLVE_DEADLINE`], which a test checks.
const LISTED: &str = "neo.solveCaptcha(page) → { attempted, type, method, reason, message, \
    durationMs, paidTasks, provider }: runs one fresh solve attempt on the page's CAPTCHA, in up \
    to 120 s. It reports what Neo did, never whether the site accepted it: afterwards inspect a \
    fresh page.ariaSnapshot() and page.screenshot(), and call it again if the page is still \
    blocked. Await it before any other action on that page: a write there meanwhile fails \
    `page_busy`.";
/// Why every call fails without a key: the deployment's to fix, not the script's (decision 7).
const NO_KEY: &str = "this Neo has no CAPTCHA provider key, so neo.solveCaptcha is off; the \
    deployment must configure one";

/// Registers `neo.solveCaptcha`: listed with a provider key, registered but unlisted without
/// one, so a script that calls it anyway learns why it cannot work.
pub(crate) fn solve_captcha(
    builder: &mut RegistryBuilder,
    solver: Option<Arc<CaptchaSolver>>,
    claims: Arc<CaptchaClaims>,
) -> Result<(), RegistryError> {
    builder.define(SolveCaptcha { solver, claims })
}

/// `neo.solveCaptcha`, over the process's one solver: its paid-task budget and its page
/// reservations are per instance, so a second solver would double the budget.
///
/// The door's rules run before the solve is sent, not during it: an owner who takes the tab
/// mid-solve stops the agent's next op, while the solver itself runs on to its own end (120 s
/// at most). The lock sees this op still open, so it reports itself unsettled meanwhile.
struct SolveCaptcha {
    /// `None` without a provider key.
    solver: Option<Arc<CaptchaSolver>>,
    /// Shared with the door's rules, where the same claims refuse other writes to the page.
    claims: Arc<CaptchaClaims>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SolveInput {
    page: PageTarget,
}

/// What the script gets back. Every completed solve is a value carrying only what Neo did and
/// observed, never a claim that the site accepted it. A plain data bag.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SolveOutput {
    attempted: bool,
    #[serde(rename = "type")]
    kind: Option<CaptchaKind>,
    method: SolveMethod,
    reason: SolveReason,
    /// Neo SDK's own line; neo-captcha's names claw-server's MCP tools.
    message: String,
    duration_ms: u64,
    paid_tasks: u32,
    provider: Option<ProviderName>,
}

impl Method for SolveCaptcha {
    type Input = SolveInput;
    type Output = SolveOutput;

    fn spec(&self) -> MethodSpec {
        let listing = match self.solver {
            Some(_) => Listing::Own(LISTED),
            None => Listing::Hidden,
        };
        MethodSpec {
            listing,
            ..MethodSpec::new("neo.solveCaptcha", Effect::Write)
        }
    }

    fn target(input: &SolveInput) -> Option<&PageTarget> {
        Some(&input.page)
    }

    /// The order is the point: everything that can refuse runs before `mark_sent`, so a
    /// refusal is `not_sent`, and the claim comes before any wait, so a busy tab is refused
    /// rather than queued.
    async fn call(&self, op: &OpCtx<Self>, _input: SolveInput) -> Result<SolveOutput, LeafError> {
        let solver = self.solver.as_ref().ok_or_else(|| {
            LeafError::definite(OperationError::host(ErrorCode::CAPTCHA_UNAVAILABLE, NO_KEY))
        })?;
        let page = op.page()?.clone();
        let _claim = self
            .claims
            .try_claim(page.tab, op.id())
            .map_err(|holder| LeafError::definite(captcha_claims::busy(holder)))?;
        // A tab that closed since the door resolved it fails here, with nothing sent.
        op.race(op.run().view().page(op.core_page()?.clone()))
            .await?;
        // Held across the solve, as claw-server holds it, so no viewport change lands mid-solve.
        let _viewport = op.viewport(&page).await?;
        op.recheck()?;
        // The solver may click or inject a token from here on.
        op.mark_sent();
        // Its widget probes and token writes run in the page's main world (neo-captcha's
        // `PageJs::page_world`). That is Neo's script, not the agent's, so the spec's
        // `page_script` stays `Isolated`: it decides how the door releases an agent script's
        // answer, and this solve returns none.
        op.mark_main_world();
        let solver = Arc::clone(solver);
        let view = op.run().view().clone();
        let core_page = op.core_page()?.clone();
        let cancel = op.cancel_token().clone();
        let telemetry = op.run().telemetry().clone();
        // Spawned, not run inside this op: the door drops the op the moment the run stops, but
        // the solver watches the same stop through its token and returns at once with what it
        // paid for. Its task still counts that outcome, the caller-cancelled one included.
        let solve = tokio::spawn(async move {
            let solved = solver
                .solve(view, core_page, cancel, HumanHelp::Unavailable)
                .await;
            if let Ok(outcome) = &solved {
                telemetry.captcha_solved(captcha_solve::captcha_solve(outcome));
            }
            solved
        });
        // Raced like any browser call (STYLE.md P7): the run's stop wins ties, and a reconnect
        // during the solve makes its answer ambiguous, since the solver may have acted on
        // either socket.
        let outcome = op
            .race(async { Ok::<_, CoreError>(solve.await) })
            .await?
            .unwrap_or_else(|error| panic::resume_unwind(error.into_panic()))
            .map_err(refused)?;
        for event in &outcome.events {
            op.note(event_line(event));
        }
        solve_value(&outcome).ok_or_else(|| op.stopped())
    }
}

/// Whether the wire `attempted` flag is set, used to pick the closing and browser-error wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Attempt {
    Ran,
    Skipped,
}

/// Whether the solver actually ran a step (a non-`none` method, any attempt counter above zero,
/// or any paid task), as distinct from `attempted`, which follows the reason-level wire rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SolverWork {
    Acted,
    StoppedEarly,
}

/// The script's value for an outcome, or `None` for a solve the crate reports cancelled by its
/// caller: it may have clicked before it stopped, so it is the stop's ambiguous error (delivery
/// `unknown`), never a `done` value. The crate says so when its token fires, which is the run's
/// stop, and when a provider cancelled the task.
fn solve_value(outcome: &SolveOutcome) -> Option<SolveOutput> {
    if outcome.reason == SolveReason::CallerCancelled {
        return None;
    }
    let did_attempt = attempted(outcome);
    let attempt = if did_attempt {
        Attempt::Ran
    } else {
        Attempt::Skipped
    };
    let work = solver_work(outcome);
    Some(SolveOutput {
        attempted: did_attempt,
        kind: outcome.kind,
        method: outcome.method,
        reason: outcome.reason,
        message: message_for(outcome.reason, outcome.kind, attempt, work),
        duration_ms: outcome.duration_ms,
        paid_tasks: outcome.paid_tasks,
        provider: outcome.provider,
    })
}

/// Whether the solver actually clicked, waited with a method, or made a provider request.
const fn solver_work(outcome: &SolveOutcome) -> SolverWork {
    let did_work = !matches!(outcome.method, SolveMethod::None)
        || outcome.attempts.checkbox > 0
        || outcome.attempts.tile_rounds > 0
        || outcome.attempts.token > 0
        || outcome.paid_tasks > 0;
    if did_work {
        SolverWork::Acted
    } else {
        SolverWork::StoppedEarly
    }
}

/// Whether Neo ran a solve attempt rather than stopping before acting. Exhaustive on
/// [`SolveReason`] so a new reason does not compile until classified.
fn attempted(outcome: &SolveOutcome) -> bool {
    let acted =
        outcome.paid_tasks > 0 || outcome.attempts.checkbox > 0 || outcome.attempts.tile_rounds > 0;
    match outcome.reason {
        SolveReason::NoCaptcha
        | SolveReason::NotBlocking
        | SolveReason::AlreadyPassed
        | SolveReason::UnsupportedType
        | SolveReason::ScanIncomplete => false,
        SolveReason::NoProvider | SolveReason::SpendLimitReached => acted,
        SolveReason::AutoPassed
        | SolveReason::CheckboxPassed
        | SolveReason::TilesAccepted
        | SolveReason::TokenAccepted
        | SolveReason::AwaitingSubmit
        | SolveReason::AcceptanceUnconfirmed
        | SolveReason::TilesExhausted
        | SolveReason::ChallengeNotPassed
        | SolveReason::DeadlineExceeded => true,
        SolveReason::ProviderUnavailable
        | SolveReason::ProviderError
        | SolveReason::BrowserError
        | SolveReason::CallerCancelled
        | SolveReason::TabClosed
        | SolveReason::Navigated => {
            acted || outcome.method != SolveMethod::None || outcome.attempts.token > 0
        }
    }
}

/// A refusal from inside `solve`, after `mark_sent`. The crate guarantees a refusal detected,
/// clicked and paid for nothing, so the answer is definite.
fn refused(refusal: SolveRefusal) -> LeafError {
    let error = match refusal {
        // The tab closed between the check above and the solver's own.
        SolveRefusal::InvalidPage(why) => OperationError::neo(
            ErrorCode::PAGE_CLOSED,
            format_args!("the page is gone: {why}"),
        ),
        // Only a solve that bypassed the claims could hold the page, and none does.
        SolveRefusal::Busy(active) => OperationError::script(
            ErrorCode::PAGE_BUSY,
            format_args!(
                "another CAPTCHA solve ({}) holds this page",
                active.request_id
            ),
        ),
        SolveRefusal::BrowserUnavailable => OperationError::neo(
            ErrorCode::BROWSER_DISCONNECTED,
            "the browser is unavailable",
        ),
    };
    LeafError::definite(error)
}

/// One trace note per solver step, e.g. `captcha.solving at 1840 ms: {"method":"checkbox"}`.
/// The crate's events carry no URL and no token.
fn event_line(event: &CaptchaEvent) -> String {
    let name = serde_json::to_value(event.event).ok();
    let name = name.as_ref().and_then(Value::as_str).unwrap_or("captcha");
    format!("{name} at {} ms: {}", event.at_ms, event.data)
}

/// Closing when Neo ran a solve attempt: never claims site acceptance.
const CLOSING_ATTEMPTED: &str = "Neo cannot confirm site acceptance — inspect a fresh snapshot \
    and screenshot before continuing, retry neo.solveCaptcha(page) if still blocked, or hand off \
    to the user.";

/// Closing when Neo attempted nothing: never claims the page is clear.
const CLOSING_NOT_ATTEMPTED: &str = "Neo cannot tell whether the page is clear — inspect a fresh \
    snapshot and screenshot before continuing, and hand off to the user if a check still blocks \
    it.";

/// Neo SDK's one line for an outcome: what Neo did, then how the caller must verify the page.
/// Exhaustive on [`SolveReason`] and [`CaptchaKind`] so new variants do not compile until worded.
fn message_for(
    reason: SolveReason,
    kind: Option<CaptchaKind>,
    attempt: Attempt,
    work: SolverWork,
) -> String {
    let action = what_neo_did(reason, kind, attempt, work);
    let close = match attempt {
        Attempt::Ran => CLOSING_ATTEMPTED,
        Attempt::Skipped => CLOSING_NOT_ATTEMPTED,
    };
    format!("{action}; {close}")
}

/// The family's display name for modifier positions (e.g. `the Turnstile widget`), falling back
/// to `CAPTCHA` when the family is absent or unknown so articles stay grammatical.
const fn family_label(kind: Option<CaptchaKind>) -> &'static str {
    match kind {
        Some(CaptchaKind::RecaptchaV2 | CaptchaKind::RecaptchaV3) => "reCAPTCHA",
        Some(CaptchaKind::Turnstile) => "Turnstile",
        Some(CaptchaKind::CloudflareChallenge) => "Cloudflare",
        Some(CaptchaKind::Hcaptcha) => "hCaptcha",
        Some(CaptchaKind::Geetest) => "GeeTest",
        Some(CaptchaKind::AwsWaf) => "AWS WAF",
        Some(CaptchaKind::Arkose) => "Arkose",
        Some(CaptchaKind::PerimeterX) => "PerimeterX",
        Some(CaptchaKind::Unknown) | None => "CAPTCHA",
    }
}

/// The family's display name for standalone noun positions (e.g. `solves Turnstile`), saying
/// `the CAPTCHA` when the family is absent or unknown.
const fn family_name(kind: Option<CaptchaKind>) -> &'static str {
    match kind {
        Some(CaptchaKind::Unknown) | None => "the CAPTCHA",
        other => family_label(other),
    }
}

/// Factual summary of what Neo did for `reason`, naming the family from `kind`.
fn what_neo_did(
    reason: SolveReason,
    kind: Option<CaptchaKind>,
    attempt: Attempt,
    work: SolverWork,
) -> String {
    let label = family_label(kind);
    let name = family_name(kind);
    let article = if matches!(kind, Some(CaptchaKind::AwsWaf | CaptchaKind::Arkose)) {
        "an"
    } else {
        "a"
    };
    match reason {
        SolveReason::AutoPassed if kind == Some(CaptchaKind::CloudflareChallenge) => {
            "Neo waited and Cloudflare's full-page check went away".to_owned()
        }
        SolveReason::ChallengeNotPassed if kind == Some(CaptchaKind::CloudflareChallenge) => {
            "Neo waited on Cloudflare's full-page check and it did not go away".to_owned()
        }
        SolveReason::ChallengeNotPassed
        | SolveReason::TilesExhausted
        | SolveReason::AcceptanceUnconfirmed
        | SolveReason::DeadlineExceeded
            if work == SolverWork::StoppedEarly =>
        {
            format!(
                "Neo found the {label} widget and stopped before it clicked or requested anything"
            )
        }
        SolveReason::AutoPassed => {
            format!("Neo waited and the {label} widget produced a new response on its own")
        }
        SolveReason::CheckboxPassed => {
            format!("Neo clicked the {label} checkbox and the widget produced a new response")
        }
        SolveReason::TilesAccepted => {
            format!(
                "Neo answered the {label} image challenge and the widget produced a new response"
            )
        }
        SolveReason::TokenAccepted => {
            format!("Neo applied {article} {label} token and the page changed after it")
        }
        SolveReason::AwaitingSubmit => format!("Neo applied {article} {label} token"),
        SolveReason::NoCaptcha => {
            "Neo found no CAPTCHA it knows on the page and attempted nothing".to_owned()
        }
        SolveReason::NotBlocking => format!(
            "Neo found only a passive {label} script or badge, no visible widget, and attempted \
             nothing"
        ),
        SolveReason::AlreadyPassed => {
            format!("Neo attempted nothing: the {label} widget's own script reports it complete")
        }
        SolveReason::UnsupportedType if kind == Some(CaptchaKind::PerimeterX) => {
            "Neo detected PerimeterX's press-and-hold human check and attempted nothing: only a \
             person can pass it"
                .to_owned()
        }
        SolveReason::UnsupportedType => {
            format!("Neo detected {name} and attempted nothing: it cannot solve this type")
        }
        SolveReason::ScanIncomplete => {
            "Neo could not fully scan the page and attempted nothing".to_owned()
        }
        SolveReason::AcceptanceUnconfirmed => {
            format!("Neo attempted {article} {label} solve and could not confirm the result")
        }
        SolveReason::ProviderError => {
            format!(
                "Neo asked a CAPTCHA provider for {article} {label} answer and the provider failed"
            )
        }
        SolveReason::BrowserError => match attempt {
            Attempt::Ran => format!("A browser call failed during Neo's {label} attempt"),
            Attempt::Skipped => "A browser call failed before Neo could attempt a solve".to_owned(),
        },
        SolveReason::DeadlineExceeded => {
            format!("Neo's {label} attempt timed out before it finished")
        }
        SolveReason::CallerCancelled => "The run stopped during Neo's attempt".to_owned(),
        SolveReason::TabClosed => "The tab closed during Neo's attempt".to_owned(),
        SolveReason::Navigated => "The page navigated during Neo's attempt".to_owned(),
        SolveReason::NoProvider => {
            format!("Neo requested no token: no provider of this Neo solves {name}")
        }
        SolveReason::ProviderUnavailable => match attempt {
            Attempt::Ran => format!(
                "Neo asked a CAPTCHA provider for {article} {label} answer and the provider was \
                 unavailable"
            ),
            Attempt::Skipped => {
                "Neo requested no token: the CAPTCHA provider is unavailable".to_owned()
            }
        },
        SolveReason::TilesExhausted => {
            format!("Neo tried the {label} image challenge and every round failed")
        }
        SolveReason::ChallengeNotPassed => {
            format!("Neo attempted the {label} challenge and the widget produced no response")
        }
        SolveReason::SpendLimitReached => {
            "Neo requested no token: this Neo's CAPTCHA budget is used up".to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;

    use neo_captcha::types::{
        Attempts, CaptchaKind, ProviderName, SolveMethod, SolveOutcome, SolveReason, SolveStatus,
    };
    use serde_json::json;

    use super::{
        Attempt, CLOSING_ATTEMPTED, CLOSING_NOT_ATTEMPTED, LISTED, SolverWork, attempted,
        message_for, solve_value,
    };

    const REASONS: [SolveReason; 22] = [
        SolveReason::AutoPassed,
        SolveReason::CheckboxPassed,
        SolveReason::TilesAccepted,
        SolveReason::TokenAccepted,
        SolveReason::AwaitingSubmit,
        SolveReason::NoCaptcha,
        SolveReason::AlreadyPassed,
        SolveReason::NotBlocking,
        SolveReason::UnsupportedType,
        SolveReason::ScanIncomplete,
        SolveReason::AcceptanceUnconfirmed,
        SolveReason::ProviderError,
        SolveReason::BrowserError,
        SolveReason::DeadlineExceeded,
        SolveReason::CallerCancelled,
        SolveReason::TabClosed,
        SolveReason::Navigated,
        SolveReason::NoProvider,
        SolveReason::ProviderUnavailable,
        SolveReason::TilesExhausted,
        SolveReason::ChallengeNotPassed,
        SolveReason::SpendLimitReached,
    ];

    const KINDS: [Option<CaptchaKind>; 11] = [
        None,
        Some(CaptchaKind::RecaptchaV2),
        Some(CaptchaKind::RecaptchaV3),
        Some(CaptchaKind::Turnstile),
        Some(CaptchaKind::CloudflareChallenge),
        Some(CaptchaKind::Hcaptcha),
        Some(CaptchaKind::Geetest),
        Some(CaptchaKind::AwsWaf),
        Some(CaptchaKind::Arkose),
        Some(CaptchaKind::PerimeterX),
        Some(CaptchaKind::Unknown),
    ];

    const FORBIDDEN: [&str; 6] = [
        "carry on",
        "is solved",
        "already passed",
        "No CAPTCHA blocks",
        "request_human_help",
        "solve_captcha",
    ];

    fn outcome(status: SolveStatus, reason: SolveReason) -> SolveOutcome {
        SolveOutcome {
            status,
            kind: Some(CaptchaKind::Turnstile),
            method: SolveMethod::None,
            reason,
            request_id: "cap_test".to_owned(),
            page: 1,
            duration_ms: 1840,
            attempts: Attempts::default(),
            paid_tasks: 0,
            provider: None,
            cost_usd: 0.0,
            message: "Call request_human_help with kind \"captcha\".".to_owned(),
            events: Vec::new(),
        }
    }

    #[test]
    fn every_reason_and_kind_produces_an_honest_single_line() {
        for reason in REASONS {
            for kind in KINDS {
                for attempt in [Attempt::Ran, Attempt::Skipped] {
                    for work in [SolverWork::Acted, SolverWork::StoppedEarly] {
                        let line = message_for(reason, kind, attempt, work);
                        assert!(
                            !line.contains('\n'),
                            "message must be a single line ({reason:?} / {kind:?} / {work:?}): \
                             {line}"
                        );
                        assert!(
                            line.contains("snapshot and screenshot"),
                            "message must tell the caller to inspect snapshot and screenshot \
                             ({reason:?} / {kind:?} / {work:?}): {line}"
                        );
                        let expected_closing = match attempt {
                            Attempt::Ran => CLOSING_ATTEMPTED,
                            Attempt::Skipped => CLOSING_NOT_ATTEMPTED,
                        };
                        assert!(
                            line.ends_with(&format!("; {expected_closing}")),
                            "message must use the matching closing ({reason:?} / {kind:?}): {line}"
                        );
                        for banned in FORBIDDEN {
                            assert!(
                                !line.contains(banned),
                                "message must not contain `{banned}` ({reason:?} / {kind:?}): \
                                 {line}"
                            );
                        }
                        assert!(
                            !line.contains("the the ")
                                && !line.contains("a the ")
                                && !line.contains("Neo's the "),
                            "message must stay grammatical when kind is absent or unknown \
                             ({reason:?} / {kind:?}): {line}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn serialized_output_has_the_eight_contract_keys_and_no_verdict() -> Result<(), Box<dyn Error>>
    {
        let mut solved = outcome(SolveStatus::TokenApplied, SolveReason::AwaitingSubmit);
        solved.method = SolveMethod::Token;
        solved.paid_tasks = 1;
        solved.provider = Some(ProviderName::Capsolver);
        let output = solve_value(&solved).ok_or("awaiting_submit is a value")?;
        let value = serde_json::to_value(output)?;
        let object = value
            .as_object()
            .ok_or("SolveOutput must serialize as a JSON object")?;
        let keys: BTreeSet<&str> = object.keys().map(String::as_str).collect();
        let expected: BTreeSet<&str> = BTreeSet::from([
            "attempted",
            "type",
            "method",
            "reason",
            "message",
            "durationMs",
            "paidTasks",
            "provider",
        ]);
        assert_eq!(
            keys, expected,
            "wire key set must match the shared contract: {value}"
        );
        assert!(
            !object.contains_key("verdict"),
            "verdict must not appear on the wire: {value}"
        );
        Ok(())
    }

    #[test]
    fn turnstile_awaiting_submit_matches_the_plan_example_byte_for_byte()
    -> Result<(), serde_json::Error> {
        let mut solved = outcome(SolveStatus::TokenApplied, SolveReason::AwaitingSubmit);
        solved.kind = Some(CaptchaKind::Turnstile);
        solved.method = SolveMethod::Token;
        solved.duration_ms = 1234;
        solved.paid_tasks = 1;
        solved.provider = Some(ProviderName::Capsolver);
        let value = solve_value(&solved).map(serde_json::to_value).transpose()?;
        assert_eq!(
            value,
            Some(json!({
                "attempted": true,
                "type": "turnstile",
                "method": "token",
                "reason": "awaiting_submit",
                "message": "Neo applied a Turnstile token; Neo cannot confirm site acceptance — \
                            inspect a fresh snapshot and screenshot before continuing, retry \
                            neo.solveCaptcha(page) if still blocked, or hand off to the user.",
                "durationMs": 1234,
                "paidTasks": 1,
                "provider": "capsolver",
            })),
            "Turnstile awaiting_submit must match the contract example byte for byte"
        );
        Ok(())
    }

    #[test]
    fn attempted_classifies_every_solve_reason() {
        let always_false = [
            SolveReason::NoCaptcha,
            SolveReason::NotBlocking,
            SolveReason::AlreadyPassed,
            SolveReason::UnsupportedType,
            SolveReason::ScanIncomplete,
        ];
        for reason in always_false {
            let mut sample = outcome(SolveStatus::AlreadyClear, reason);
            assert!(!attempted(&sample), "{reason:?} default must be false");
            sample.paid_tasks = 1;
            sample.method = SolveMethod::Token;
            assert!(!attempted(&sample), "{reason:?} stays false with counters");
        }

        let only_if_acted = [SolveReason::NoProvider, SolveReason::SpendLimitReached];
        for reason in only_if_acted {
            let idle = outcome(SolveStatus::NeedsHuman, reason);
            assert!(!attempted(&idle), "{reason:?} idle must be false");

            let mut clicked = outcome(SolveStatus::NeedsHuman, reason);
            clicked.attempts.checkbox = 1;
            assert!(
                attempted(&clicked),
                "{reason:?} checkbox click must be true"
            );

            let mut tiled = outcome(SolveStatus::NeedsHuman, reason);
            tiled.attempts.tile_rounds = 1;
            assert!(attempted(&tiled), "{reason:?} tile rounds must be true");

            let mut paid = outcome(SolveStatus::NeedsHuman, reason);
            paid.paid_tasks = 1;
            assert!(attempted(&paid), "{reason:?} paid task must be true");

            let mut token_only = outcome(SolveStatus::NeedsHuman, reason);
            token_only.method = SolveMethod::Token;
            token_only.attempts.token = 1;
            assert!(
                !attempted(&token_only),
                "{reason:?} depends strictly on acted (paid_tasks, checkbox, tile_rounds)"
            );
        }

        let always_true = [
            SolveReason::AutoPassed,
            SolveReason::CheckboxPassed,
            SolveReason::TilesAccepted,
            SolveReason::TokenAccepted,
            SolveReason::AwaitingSubmit,
            SolveReason::AcceptanceUnconfirmed,
            SolveReason::TilesExhausted,
            SolveReason::ChallengeNotPassed,
            SolveReason::DeadlineExceeded,
        ];
        for reason in always_true {
            assert!(
                attempted(&outcome(SolveStatus::Solved, reason)),
                "{reason:?} must be true"
            );
        }

        let acted_or_method_or_token = [
            SolveReason::ProviderUnavailable,
            SolveReason::ProviderError,
            SolveReason::BrowserError,
            SolveReason::CallerCancelled,
            SolveReason::TabClosed,
            SolveReason::Navigated,
        ];
        for reason in acted_or_method_or_token {
            assert!(
                !attempted(&outcome(SolveStatus::Failed, reason)),
                "{reason:?} with method none and zero attempts must be false"
            );
            let mut with_token_method = outcome(SolveStatus::Failed, reason);
            with_token_method.method = SolveMethod::Token;
            assert!(
                attempted(&with_token_method),
                "{reason:?} with method token must be true"
            );
            let mut with_token_attempt = outcome(SolveStatus::Failed, reason);
            with_token_attempt.attempts.token = 1;
            assert!(
                attempted(&with_token_attempt),
                "{reason:?} with attempts.token > 0 must be true"
            );
            let mut with_checkbox = outcome(SolveStatus::Failed, reason);
            with_checkbox.attempts.checkbox = 1;
            assert!(
                attempted(&with_checkbox),
                "{reason:?} with a checkbox attempt must be true"
            );
        }

        assert_eq!(
            always_false.len()
                + only_if_acted.len()
                + always_true.len()
                + acted_or_method_or_token.len(),
            REASONS.len(),
            "every SolveReason variant is tested in one of the four groups"
        );
    }

    /// `PerimeterX`'s `type` stays `unknown` (claw-server's closed schema does not list it), so its
    /// refusal names the family and a person in the message.
    #[test]
    fn a_perimeterx_refusal_names_the_family_and_a_person() -> Result<(), serde_json::Error> {
        let mut refused = outcome(SolveStatus::Unsupported, SolveReason::UnsupportedType);
        refused.kind = Some(CaptchaKind::PerimeterX);
        let value = solve_value(&refused)
            .map(serde_json::to_value)
            .transpose()?;
        assert_eq!(
            value,
            Some(json!({
                "attempted": false,
                "type": "unknown",
                "method": "none",
                "reason": "unsupported_type",
                "message": "Neo detected PerimeterX's press-and-hold human check and attempted \
                            nothing: only a person can pass it; Neo cannot tell whether the page \
                            is clear — inspect a fresh snapshot and screenshot before continuing, \
                            and hand off to the user if a check still blocks it.",
                "durationMs": 1840,
                "paidTasks": 0,
                "provider": null,
            }))
        );
        refused.kind = Some(CaptchaKind::Hcaptcha);
        let hcaptcha_message = solve_value(&refused).map(|output| output.message);
        assert_eq!(
            hcaptcha_message.as_deref(),
            Some(
                "Neo detected hCaptcha and attempted nothing: it cannot solve this type; Neo \
                 cannot tell whether the page is clear — inspect a fresh snapshot and screenshot \
                 before continuing, and hand off to the user if a check still blocks it."
            )
        );
        Ok(())
    }

    #[test]
    fn cloudflare_wall_and_browser_error_wording_follow_the_plan() {
        assert_eq!(
            message_for(
                SolveReason::AutoPassed,
                Some(CaptchaKind::CloudflareChallenge),
                Attempt::Ran,
                SolverWork::StoppedEarly,
            ),
            format!("Neo waited and Cloudflare's full-page check went away; {CLOSING_ATTEMPTED}")
        );
        assert_eq!(
            message_for(
                SolveReason::ChallengeNotPassed,
                Some(CaptchaKind::CloudflareChallenge),
                Attempt::Ran,
                SolverWork::StoppedEarly,
            ),
            format!(
                "Neo waited on Cloudflare's full-page check and it did not go away; \
                 {CLOSING_ATTEMPTED}"
            )
        );
        assert_eq!(
            message_for(
                SolveReason::BrowserError,
                Some(CaptchaKind::Turnstile),
                Attempt::Ran,
                SolverWork::Acted,
            ),
            format!("A browser call failed during Neo's Turnstile attempt; {CLOSING_ATTEMPTED}")
        );
        assert_eq!(
            message_for(
                SolveReason::BrowserError,
                None,
                Attempt::Skipped,
                SolverWork::StoppedEarly,
            ),
            format!(
                "A browser call failed before Neo could attempt a solve; {CLOSING_NOT_ATTEMPTED}"
            )
        );
    }

    #[test]
    fn stopped_early_reasons_do_not_claim_an_attempt() -> Result<(), Box<dyn Error>> {
        let cases = [
            (
                SolveReason::ChallengeNotPassed,
                CaptchaKind::AwsWaf,
                "AWS WAF",
                "Neo attempted the AWS WAF challenge and the widget produced no response",
            ),
            (
                SolveReason::TilesExhausted,
                CaptchaKind::RecaptchaV2,
                "reCAPTCHA",
                "Neo tried the reCAPTCHA image challenge and every round failed",
            ),
            (
                SolveReason::AcceptanceUnconfirmed,
                CaptchaKind::Turnstile,
                "Turnstile",
                "Neo attempted a Turnstile solve and could not confirm the result",
            ),
            (
                SolveReason::DeadlineExceeded,
                CaptchaKind::Turnstile,
                "Turnstile",
                "Neo's Turnstile attempt timed out before it finished",
            ),
        ];
        for (reason, kind, label, acted_prefix) in cases {
            let mut stopped = outcome(SolveStatus::Failed, reason);
            stopped.kind = Some(kind);
            let output = solve_value(&stopped).ok_or("reason returns a value")?;
            assert!(output.attempted, "{reason:?} keeps attempted = true");
            assert_eq!(
                output.message,
                format!(
                    "Neo found the {label} widget and stopped before it clicked or requested \
                     anything; {CLOSING_ATTEMPTED}"
                ),
                "{reason:?} with no method, attempts, or paid tasks must not claim an attempt"
            );

            let mut clicked = stopped.clone();
            clicked.attempts.checkbox = 1;
            let clicked_out = solve_value(&clicked).ok_or("reason returns a value")?;
            assert_eq!(
                clicked_out.message,
                format!("{acted_prefix}; {CLOSING_ATTEMPTED}"),
                "{reason:?} with a checkbox click keeps its acted wording"
            );

            let mut token_tried = stopped;
            token_tried.attempts.token = 1;
            let token_out = solve_value(&token_tried).ok_or("reason returns a value")?;
            assert_eq!(
                token_out.message,
                format!("{acted_prefix}; {CLOSING_ATTEMPTED}"),
                "{reason:?} with a token attempt keeps its acted wording"
            );
        }
        Ok(())
    }

    #[test]
    fn acceptance_unconfirmed_and_deadline_exceeded_do_not_overclaim() {
        assert_eq!(
            message_for(
                SolveReason::AcceptanceUnconfirmed,
                Some(CaptchaKind::CloudflareChallenge),
                Attempt::Ran,
                SolverWork::Acted,
            ),
            format!(
                "Neo attempted a Cloudflare solve and could not confirm the result; \
                 {CLOSING_ATTEMPTED}"
            )
        );
        assert_eq!(
            message_for(
                SolveReason::AcceptanceUnconfirmed,
                Some(CaptchaKind::AwsWaf),
                Attempt::Ran,
                SolverWork::Acted,
            ),
            format!(
                "Neo attempted an AWS WAF solve and could not confirm the result; \
                 {CLOSING_ATTEMPTED}"
            )
        );
        assert_eq!(
            message_for(
                SolveReason::DeadlineExceeded,
                Some(CaptchaKind::Turnstile),
                Attempt::Ran,
                SolverWork::Acted,
            ),
            format!("Neo's Turnstile attempt timed out before it finished; {CLOSING_ATTEMPTED}")
        );
    }

    /// The listed time limit is the solver's own deadline, and `LISTED` advertises the eight
    /// contract fields without verdict or carry-on claims.
    #[test]
    fn the_listed_line_names_the_deadline_and_contract_without_verdict() {
        let said = format!("in up to {} s", neo_captcha::SOLVE_DEADLINE.as_secs());
        assert!(LISTED.contains("neo.solveCaptcha(page)"), "names the call");
        assert!(LISTED.contains(&said), "the line says: {said}");
        assert!(
            LISTED.contains(
                "{ attempted, type, method, reason, message, durationMs, paidTasks, provider }"
            ),
            "lists the eight fields in contract order: {LISTED}"
        );
        assert!(!LISTED.contains('\n'), "stays one entry: {LISTED}");
        assert!(!LISTED.contains("verdict"), "no verdict: {LISTED}");
        for banned in FORBIDDEN {
            assert!(
                !LISTED.contains(banned),
                "LISTED must not contain `{banned}`: {LISTED}"
            );
        }
    }

    #[test]
    fn a_solve_its_caller_cancelled_is_never_a_value() {
        assert_eq!(
            solve_value(&outcome(
                SolveStatus::Cancelled,
                SolveReason::CallerCancelled
            )),
            None,
            "it may have clicked before it stopped, so the op fails ambiguous instead"
        );
        assert!(
            solve_value(&outcome(SolveStatus::Cancelled, SolveReason::TabClosed)).is_some(),
            "the page's own cancel (closed, navigated) is still a value"
        );
    }

    #[test]
    fn refused_token_provider_unavailable_reports_attempted_and_honest_wording()
    -> Result<(), Box<dyn Error>> {
        let mut refused = outcome(SolveStatus::NeedsHuman, SolveReason::ProviderUnavailable);
        refused.kind = Some(CaptchaKind::RecaptchaV2);
        refused.method = SolveMethod::Token;
        refused.attempts.token = 1;
        let output = solve_value(&refused).ok_or("provider_unavailable returns a value")?;
        assert!(
            output.attempted,
            "a refused token request must report attempted = true"
        );
        assert_eq!(
            output.message,
            format!(
                "Neo asked a CAPTCHA provider for a reCAPTCHA answer and the provider was \
                 unavailable; {CLOSING_ATTEMPTED}"
            )
        );
        assert!(
            !output.message.contains("requested no token"),
            "must not claim no token was requested after sending a provider request: {}",
            output.message
        );

        let mut skipped = outcome(SolveStatus::NeedsHuman, SolveReason::ProviderUnavailable);
        skipped.kind = Some(CaptchaKind::RecaptchaV2);
        let skipped_out = solve_value(&skipped).ok_or("provider_unavailable returns a value")?;
        assert!(!skipped_out.attempted);
        assert_eq!(
            skipped_out.message,
            format!(
                "Neo requested no token: the CAPTCHA provider is unavailable; \
                 {CLOSING_NOT_ATTEMPTED}"
            )
        );
        Ok(())
    }
}
