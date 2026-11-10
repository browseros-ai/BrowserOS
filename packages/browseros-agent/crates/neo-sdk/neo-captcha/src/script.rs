//! Neo: The solver's public script value and allowlisted call-log lines. The host translates
//! these facts across the runtime seam without exposing keys, tokens, URLs or legacy tool prose.

use crate::{
    SolveRefusal, events,
    types::{CaptchaEvent, CaptchaKind, SolveOutcome, SolveReason, Verdict},
};
use serde::Serialize;

/// Description appended only when this host has explicitly installed its solver. The flow
/// runs up to 120 s; cancellation can wait longer while a started CDP click drains.
pub const RUN_INSTRUCTION: &str = "neo.solveCaptcha(page) → {verdict, type, reason, message, durationMs, paidTasks} solves a CAPTCHA. The flow runs up to 120 s; input cleanup may take longer. verdict is solved | token_applied | already_clear | unsolved | cancelled | unknown; look at the page again after cancelled or unknown. Await it before acting on that page: input there meanwhile fails. Give the run a timeout of at least 60000.";

/// Finished solve facts a script can branch on, independent of legacy MCP's output shape.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptOutcome {
    verdict: Verdict,
    #[serde(rename = "type")]
    kind: Option<CaptchaKind>,
    reason: SolveReason,
    message: String,
    duration_ms: u64,
    paid_tasks: u32,
}
impl ScriptOutcome {
    pub fn from_outcome(outcome: &SolveOutcome) -> Option<Self> {
        if outcome.reason == SolveReason::CallerCancelled {
            return None;
        }
        let verdict = outcome.verdict();
        Some(Self {
            verdict,
            kind: outcome.kind,
            reason: outcome.reason,
            message: message(verdict, outcome.reason),
            duration_ms: outcome.duration_ms,
            paid_tasks: outcome.paid_tasks,
        })
    }
}

pub fn refusal(refusal: &SolveRefusal) -> (bool, String) {
    match refusal {
        SolveRefusal::InvalidPage(why) => (false, format!("neo.solveCaptcha: the solver cannot reach this page ({why})")),
        SolveRefusal::Busy(_) => (false, "neo.solveCaptcha: another solve is working on this page; await it, then look at the page again".to_owned()),
        SolveRefusal::BrowserUnavailable => (true, "neo.solveCaptcha: the browser is unavailable".to_owned()),
    }
}

/// Use the same field allowlist as tracing; raw event data is not a script/log contract.
pub fn call_log_line(event: &CaptchaEvent) -> String {
    let name = serde_json::to_value(event.event).ok();
    let name = name
        .as_ref()
        .and_then(serde_json::Value::as_str)
        .unwrap_or("captcha");
    format!("{name} at {} ms: {}", event.at_ms, events::log_data(event))
}

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
    use crate::types::{
        Attempts, CaptchaKind, SolveMethod, SolveOutcome, SolveReason, SolveStatus, Verdict,
    };

    use super::{ScriptOutcome, message};

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
            any_click_failed: false,
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
        let value = ScriptOutcome::from_outcome(&outcome(
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
            ScriptOutcome::from_outcome(&outcome(
                SolveStatus::Cancelled,
                SolveReason::CallerCancelled
            )),
            None,
            "it may have clicked before it stopped, so the op fails ambiguous instead"
        );
        assert!(
            ScriptOutcome::from_outcome(&outcome(SolveStatus::Cancelled, SolveReason::TabClosed))
                .is_some(),
            "the page's own cancel (closed, navigated) is still a value"
        );
    }
}

#[cfg(test)]
mod event_tests {
    use super::*;
    use crate::types::CaptchaEventName;

    #[test]
    fn script_log_lines_use_the_same_allowlist_as_tracing_for_every_event() {
        for name in [
            CaptchaEventName::Detected,
            CaptchaEventName::Solving,
            CaptchaEventName::Solved,
            CaptchaEventName::TokenApplied,
            CaptchaEventName::Handoff,
            CaptchaEventName::Failed,
        ] {
            let event = CaptchaEvent {
                event: name,
                request_id: "request".into(),
                page: 1,
                at_ms: 12,
                data: serde_json::json!({"type":"recaptcha_v2","method":"checkbox","attempt":1,"key":"SENTINEL-PRIVATE","url":"SENTINEL-PRIVATE","token":"SENTINEL-PRIVATE","message":"SENTINEL-PRIVATE"}),
            };
            let line = call_log_line(&event);
            assert!(!line.contains("SENTINEL-PRIVATE"));
            assert!(line.ends_with(&events::log_data(&event).to_string()));
        }
    }
}
