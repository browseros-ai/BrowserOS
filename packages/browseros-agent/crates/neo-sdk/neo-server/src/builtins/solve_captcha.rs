//! `neo.solveCaptcha(page)` over neo-captcha (design D10).

use std::sync::Arc;

use browseros_core::CoreError;
use neo_captcha::types::{CaptchaEvent, CaptchaKind, SolveOutcome, SolveReason, Verdict};
use neo_captcha::{CaptchaSolver, HumanHelp, SolveRefusal};
use neo_trace::Effect;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::captcha_claims::{self, CaptchaClaims};
use crate::dispatch::{
    ErrorCode, LeafError, Listing, Method, MethodSpec, OpCtx, OperationError, PageTarget,
    RegistryBuilder, RegistryError,
};

/// The method's own line in the `run` description, shown only when this Neo has a provider key.
const LISTED: &str = "neo.solveCaptcha(page) → { verdict, type, reason, message, durationMs, \
    paidTasks }: solves the CAPTCHA on the page, in up to 120 s. verdict is solved | \
    token_applied | already_clear | unsolved | cancelled | unknown; branch on it, and look at \
    the page again after cancelled or unknown. Await it before any other action on that page: a \
    write there meanwhile fails page_busy.";
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

/// What the script gets back. Every verdict is a value, `unsolved` included, so a script can
/// branch on it (decision 8); the crate guarantees a verdict is never a guess. A plain data bag.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SolveOutput {
    verdict: Verdict,
    #[serde(rename = "type")]
    kind: Option<CaptchaKind>,
    reason: SolveReason,
    /// claw-sdk's own line; the crate's names claw-server's MCP tools (decision 9).
    message: String,
    duration_ms: u64,
    paid_tasks: u32,
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
        // Boxed: the solver's future is about 25 KiB, and the op's own would carry it inline.
        let solve = Box::pin(solver.solve(
            op.run().view().clone(),
            op.core_page()?.clone(),
            op.cancel_token().clone(),
            HumanHelp::Unavailable,
        ));
        // Raced like any browser call (STYLE.md P7): the run's stop wins ties, and a reconnect
        // during the solve makes its answer ambiguous, since the solver may have acted on
        // either socket.
        let outcome = op
            .race(async { Ok::<_, CoreError>(solve.await) })
            .await?
            .map_err(refused)?;
        for event in &outcome.events {
            op.note(event_line(event));
        }
        verdict_value(&outcome).ok_or_else(|| op.stopped())
    }
}

/// The script's value for an outcome, or `None` for a solve the crate reports cancelled by its
/// caller: it may have clicked before it stopped, so it is the stop's ambiguous error (delivery
/// `unknown`), never a `done` value. The crate says so when its token fires, which is the run's
/// stop, and when a provider cancelled the task.
fn verdict_value(outcome: &SolveOutcome) -> Option<SolveOutput> {
    if outcome.reason == SolveReason::CallerCancelled {
        return None;
    }
    let verdict = outcome.verdict();
    Some(SolveOutput {
        verdict,
        kind: outcome.kind,
        reason: outcome.reason,
        message: message(verdict, outcome.reason),
        duration_ms: outcome.duration_ms,
        paid_tasks: outcome.paid_tasks,
    })
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

/// claw-sdk's one line for an outcome, worded for a script with no human-help tool: what the
/// verdict means, why, then what to do. The verdict leads, so the line never contradicts what
/// the script branches on. Exhaustive on both enums, so a new verdict or reason does not
/// compile until someone words it.
fn message(verdict: Verdict, reason: SolveReason) -> String {
    let (said, next) = match verdict {
        Verdict::Solved => ("The CAPTCHA is solved", "carry on with the task."),
        Verdict::TokenApplied => (
            "A CAPTCHA token is in the page",
            "submit the form, and the site checks the token then.",
        ),
        Verdict::AlreadyClear => ("No CAPTCHA blocks the page", "carry on with the task."),
        Verdict::Unsolved => (
            "The CAPTCHA still blocks the page",
            "stop and tell the user it needs their help.",
        ),
        Verdict::Cancelled => (
            "The solve stopped early",
            "look at the page again before acting.",
        ),
        Verdict::Unknown => (
            "Neo could not tell whether the CAPTCHA passed",
            "look at the page again before acting.",
        ),
    };
    format!("{said} ({}); {next}", cause(reason))
}

/// Why, in a few words.
fn cause(reason: SolveReason) -> &'static str {
    match reason {
        SolveReason::AutoPassed => "it passed on its own",
        SolveReason::CheckboxPassed => "its checkbox was clicked",
        SolveReason::TilesAccepted => "its image challenge was solved",
        SolveReason::TokenAccepted => "a solver's token was accepted",
        SolveReason::AwaitingSubmit => "the site has not checked it yet",
        SolveReason::NoCaptcha => "no CAPTCHA was found",
        SolveReason::AlreadyPassed => "it had already passed",
        SolveReason::NotBlocking => "its widget does not block the page",
        SolveReason::UnsupportedType => "Neo cannot solve this CAPTCHA type",
        SolveReason::ScanIncomplete => "the page could not be fully scanned",
        SolveReason::AcceptanceUnconfirmed => "the site's acceptance could not be confirmed",
        SolveReason::ProviderError => "the CAPTCHA provider failed",
        SolveReason::BrowserError => "a browser call failed",
        SolveReason::DeadlineExceeded => "it ran past its 120 s limit",
        SolveReason::CallerCancelled => "the run stopped",
        SolveReason::TabClosed => "the tab closed",
        SolveReason::Navigated => "the page navigated, maybe past the CAPTCHA",
        SolveReason::NoProvider => "no provider of this Neo solves this type",
        SolveReason::ProviderUnavailable => "the CAPTCHA provider is unavailable",
        SolveReason::TilesExhausted => "every image round failed",
        SolveReason::ChallengeNotPassed => "the challenge was attempted and not passed",
        SolveReason::SpendLimitReached => "this Neo's CAPTCHA budget is used up",
    }
}

#[cfg(test)]
mod tests {
    use neo_captcha::types::{
        Attempts, CaptchaKind, SolveMethod, SolveOutcome, SolveReason, SolveStatus, Verdict,
    };

    use super::{message, verdict_value};

    const VERDICTS: [Verdict; 6] = [
        Verdict::Solved,
        Verdict::TokenApplied,
        Verdict::AlreadyClear,
        Verdict::Unsolved,
        Verdict::Cancelled,
        Verdict::Unknown,
    ];

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

    fn outcome(status: SolveStatus, reason: SolveReason) -> SolveOutcome {
        SolveOutcome {
            status,
            kind: Some(CaptchaKind::Turnstile),
            method: SolveMethod::Checkbox,
            reason,
            request_id: "cap_test".to_owned(),
            page: 1,
            duration_ms: 1840,
            attempts: Attempts::default(),
            paid_tasks: 0,
            cost_usd: 0.0,
            message: "Call request_human_help with kind \"captcha\".".to_owned(),
            events: Vec::new(),
        }
    }

    #[test]
    fn each_verdict_has_claw_sdks_own_line() {
        let table = [
            (
                Verdict::Solved,
                SolveReason::CheckboxPassed,
                "The CAPTCHA is solved (its checkbox was clicked); carry on with the task.",
            ),
            (
                Verdict::TokenApplied,
                SolveReason::AwaitingSubmit,
                "A CAPTCHA token is in the page (the site has not checked it yet); submit the \
                 form, and the site checks the token then.",
            ),
            (
                Verdict::AlreadyClear,
                SolveReason::NoCaptcha,
                "No CAPTCHA blocks the page (no CAPTCHA was found); carry on with the task.",
            ),
            (
                Verdict::Unsolved,
                SolveReason::SpendLimitReached,
                "The CAPTCHA still blocks the page (this Neo's CAPTCHA budget is used up); stop \
                 and tell the user it needs their help.",
            ),
            (
                Verdict::Cancelled,
                SolveReason::Navigated,
                "The solve stopped early (the page navigated, maybe past the CAPTCHA); look at \
                 the page again before acting.",
            ),
            (
                Verdict::Unknown,
                SolveReason::DeadlineExceeded,
                "Neo could not tell whether the CAPTCHA passed (it ran past its 120 s limit); \
                 look at the page again before acting.",
            ),
        ];
        for (verdict, reason, line) in table {
            assert_eq!(message(verdict, reason), line, "{verdict:?} / {reason:?}");
        }
    }

    #[test]
    fn no_line_points_at_claw_servers_tools() {
        for verdict in VERDICTS {
            for reason in REASONS {
                let line = message(verdict, reason);
                assert!(
                    !line.contains('\n')
                        && !line.contains("request_human_help")
                        && !line.contains("solve_captcha")
                        && !line.contains("snapshot"),
                    "claw-sdk has none of claw-server's MCP tools, so its one line names none: \
                     {line}"
                );
            }
        }
    }

    #[test]
    fn a_verdict_is_a_value_with_the_crates_facts_and_claw_sdks_line() {
        let value = verdict_value(&outcome(
            SolveStatus::NeedsHuman,
            SolveReason::TilesExhausted,
        ));
        let value = value.map(|value| serde_json::to_value(value).ok());
        assert_eq!(
            value,
            Some(Some(serde_json::json!({
                "verdict": "unsolved",
                "type": "turnstile",
                "reason": "tiles_exhausted",
                "message": "The CAPTCHA still blocks the page (every image round failed); stop \
                            and tell the user it needs their help.",
                "durationMs": 1840,
                "paidTasks": 0,
            }))),
            "unsolved is a value a script branches on, not a thrown error"
        );
    }

    #[test]
    fn a_solve_its_caller_cancelled_is_never_a_value() {
        assert_eq!(
            verdict_value(&outcome(
                SolveStatus::Cancelled,
                SolveReason::CallerCancelled
            )),
            None,
            "it may have clicked before it stopped, so the op fails ambiguous instead"
        );
        assert!(
            verdict_value(&outcome(SolveStatus::Cancelled, SolveReason::TabClosed)).is_some(),
            "the page's own cancel (closed, navigated) is still a value"
        );
    }
}
