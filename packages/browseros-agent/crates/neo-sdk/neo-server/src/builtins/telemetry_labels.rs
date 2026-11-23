//! Telemetry's navigation and CAPTCHA labels, read from what an op's leaf returned.
//!
//! The one place where neo-playwright's navigation reply and failure, and neo-captcha's solve
//! outcome, become `neo_telemetry::label`s. Each reads the live result the op is about to
//! answer with, never the trace, so the `trace` settings change no number. Every match over a
//! crate's enum is exhaustive (STYLE.md P8): a new kind, status, reason or provider fails the
//! build here instead of reaching a metric under a guessed label.

use neo_captcha::types::{CaptchaKind, ProviderName, SolveOutcome, SolveReason, SolveStatus};
use neo_playwright::{NetFailure, PlaywrightError};
use neo_telemetry::label::{
    CaptchaFamily, CaptchaProvider, CaptchaReason, CaptchaVerdict, HttpStatus, NetError,
};
use neo_telemetry::{CaptchaSolve, CaptchaSpend, Navigated};
use serde_json::Value;

/// The ops that navigate a page's document, each counted once as a navigation.
const NAVIGATIONS: [&str; 4] = ["page.goto", "page.reload", "page.goBack", "page.goForward"];

/// The navigation `op` ran, as telemetry counts it, or `None` for an op that navigates nothing.
/// Its status is the reply's `response.status`, when it is an HTTP one; its error, the network
/// failure the crate kept apart from its message. The op's own outcome is not read: a goto
/// answered 404 is still an `ok` op.
#[must_use]
pub(super) fn navigated(op: &str, result: &Result<Value, PlaywrightError>) -> Option<Navigated> {
    if !NAVIGATIONS.contains(&op) {
        return None;
    }
    Some(match result {
        Ok(reply) => Navigated {
            status: reply
                .pointer("/response/status")
                .and_then(Value::as_u64)
                .and_then(|code| u16::try_from(code).ok())
                .and_then(HttpStatus::new),
            error: None,
        },
        Err(error) => Navigated {
            status: None,
            error: error.net_failure().map(net_error),
        },
    })
}

fn net_error(failure: &NetFailure) -> NetError {
    match failure {
        NetFailure::Named(text) => NetError::of_chromium(text),
        NetFailure::ErrorPage => NetError::Other,
    }
}

/// A solve's outcome, as telemetry counts it.
#[must_use]
pub(super) fn captcha_solve(outcome: &SolveOutcome) -> CaptchaSolve {
    CaptchaSolve {
        family: family(outcome.kind),
        verdict: verdict(outcome.status),
        reason: reason(outcome.reason),
        spend: (outcome.cost_usd > 0.0).then(|| CaptchaSpend {
            provider: outcome.provider.map_or(CaptchaProvider::Unknown, provider),
            usd: outcome.cost_usd,
        }),
    }
}

/// Named here rather than through `CaptchaKind::as_str`, which spells `PerimeterX` as `unknown`
/// for an MCP schema that does not list it.
const fn family(kind: Option<CaptchaKind>) -> CaptchaFamily {
    match kind {
        Some(CaptchaKind::RecaptchaV2) => CaptchaFamily::RecaptchaV2,
        Some(CaptchaKind::RecaptchaV3) => CaptchaFamily::RecaptchaV3,
        Some(CaptchaKind::Turnstile) => CaptchaFamily::Turnstile,
        Some(CaptchaKind::CloudflareChallenge) => CaptchaFamily::CloudflareChallenge,
        Some(CaptchaKind::Hcaptcha) => CaptchaFamily::Hcaptcha,
        Some(CaptchaKind::Geetest) => CaptchaFamily::Geetest,
        Some(CaptchaKind::AwsWaf) => CaptchaFamily::AwsWaf,
        Some(CaptchaKind::Arkose) => CaptchaFamily::Arkose,
        Some(CaptchaKind::PerimeterX) => CaptchaFamily::Perimeterx,
        Some(CaptchaKind::Unknown) => CaptchaFamily::Unknown,
        None => CaptchaFamily::None,
    }
}

const fn verdict(status: SolveStatus) -> CaptchaVerdict {
    match status {
        SolveStatus::Solved => CaptchaVerdict::Solved,
        SolveStatus::TokenApplied => CaptchaVerdict::TokenApplied,
        SolveStatus::AlreadyClear => CaptchaVerdict::AlreadyClear,
        SolveStatus::Unsupported => CaptchaVerdict::Unsupported,
        SolveStatus::Failed => CaptchaVerdict::Failed,
        SolveStatus::Cancelled => CaptchaVerdict::Cancelled,
        SolveStatus::NeedsHuman => CaptchaVerdict::NeedsHuman,
    }
}

const fn reason(reason: SolveReason) -> CaptchaReason {
    match reason {
        SolveReason::AutoPassed => CaptchaReason::AutoPassed,
        SolveReason::CheckboxPassed => CaptchaReason::CheckboxPassed,
        SolveReason::TilesAccepted => CaptchaReason::TilesAccepted,
        SolveReason::TokenAccepted => CaptchaReason::TokenAccepted,
        SolveReason::AwaitingSubmit => CaptchaReason::AwaitingSubmit,
        SolveReason::NoCaptcha => CaptchaReason::NoCaptcha,
        SolveReason::AlreadyPassed => CaptchaReason::AlreadyPassed,
        SolveReason::NotBlocking => CaptchaReason::NotBlocking,
        SolveReason::UnsupportedType => CaptchaReason::UnsupportedType,
        SolveReason::ScanIncomplete => CaptchaReason::ScanIncomplete,
        SolveReason::AcceptanceUnconfirmed => CaptchaReason::AcceptanceUnconfirmed,
        SolveReason::ProviderError => CaptchaReason::ProviderError,
        SolveReason::BrowserError => CaptchaReason::BrowserError,
        SolveReason::DeadlineExceeded => CaptchaReason::DeadlineExceeded,
        SolveReason::CallerCancelled => CaptchaReason::CallerCancelled,
        SolveReason::TabClosed => CaptchaReason::TabClosed,
        SolveReason::Navigated => CaptchaReason::Navigated,
        SolveReason::NoProvider => CaptchaReason::NoProvider,
        SolveReason::ProviderUnavailable => CaptchaReason::ProviderUnavailable,
        SolveReason::TilesExhausted => CaptchaReason::TilesExhausted,
        SolveReason::ChallengeNotPassed => CaptchaReason::ChallengeNotPassed,
        SolveReason::SpendLimitReached => CaptchaReason::SpendLimitReached,
    }
}

const fn provider(provider: ProviderName) -> CaptchaProvider {
    match provider {
        ProviderName::Capsolver => CaptchaProvider::Capsolver,
        ProviderName::TwoCaptcha => CaptchaProvider::TwoCaptcha,
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use neo_captcha::types::{
        Attempts, CaptchaKind, ProviderName, SolveMethod, SolveOutcome, SolveReason, SolveStatus,
    };
    use neo_playwright::{ErrorCode, NetFailure, PlaywrightError};
    use neo_telemetry::label::{
        CaptchaFamily, CaptchaProvider, CaptchaReason, CaptchaVerdict, HttpStatus, NetError,
        RunStatus,
    };
    use neo_telemetry::{
        CaptchaSpend, Navigated, RunFacts, RunTelemetry, RunVerdict, Telemetry, TelemetryConfig,
    };
    use serde_json::{Value, json};

    use super::{captcha_solve, family, navigated, provider, reason, verdict};
    use crate::telemetry_rig::Scrape;

    const KINDS: [CaptchaKind; 10] = [
        CaptchaKind::RecaptchaV2,
        CaptchaKind::RecaptchaV3,
        CaptchaKind::Turnstile,
        CaptchaKind::CloudflareChallenge,
        CaptchaKind::Hcaptcha,
        CaptchaKind::Geetest,
        CaptchaKind::AwsWaf,
        CaptchaKind::Arkose,
        CaptchaKind::PerimeterX,
        CaptchaKind::Unknown,
    ];

    const STATUSES: [SolveStatus; 7] = [
        SolveStatus::Solved,
        SolveStatus::TokenApplied,
        SolveStatus::AlreadyClear,
        SolveStatus::Unsupported,
        SolveStatus::Failed,
        SolveStatus::Cancelled,
        SolveStatus::NeedsHuman,
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

    /// A solve's outcome as neo-captcha returns it.
    fn outcome(
        kind: Option<CaptchaKind>,
        status: SolveStatus,
        reason: SolveReason,
        provider: Option<ProviderName>,
        cost_usd: f64,
    ) -> SolveOutcome {
        SolveOutcome {
            status,
            kind,
            method: SolveMethod::Token,
            reason,
            request_id: "cap_test".to_owned(),
            page: 1,
            duration_ms: 1840,
            attempts: Attempts::default(),
            paid_tasks: u32::from(cost_usd > 0.0),
            provider,
            cost_usd,
            message: String::new(),
            events: Vec::new(),
        }
    }

    /// The wire name serde gives `value`, as the script reads it.
    fn wire<T: serde::Serialize>(value: T) -> Result<String, Box<dyn Error>> {
        Ok(serde_json::to_value(value)?
            .as_str()
            .ok_or("a unit variant serializes as a string")?
            .to_owned())
    }

    fn counted_run(telemetry: &Telemetry) -> neo_telemetry::RunSpan {
        telemetry.start_run(RunFacts {
            run_id: &"run-1",
            task_id: None,
            parent: None,
        })
    }

    /// A navigation op's reply, as neo-playwright serializes it.
    fn reply(status: &Value) -> Value {
        json!({
            "response": {
                "url": "https://example.com/",
                "status": status,
                "statusText": "",
                "httpVersion": "HTTP/1.1",
                "ok": false,
                "headers": {},
            },
            "url": "https://example.com/",
            "frameSnapshotAttempted": true,
        })
    }

    fn failed(code: ErrorCode, net: Option<NetFailure>) -> Result<Value, PlaywrightError> {
        let error = PlaywrightError::new(code, "page.goto: failed".to_owned());
        Err(match net {
            Some(net) => error.with_net_failure(net),
            None => error,
        })
    }

    #[test]
    fn only_the_four_navigation_ops_are_navigations() {
        for op in ["page.goto", "page.reload", "page.goBack", "page.goForward"] {
            assert!(
                navigated(op, &Ok(reply(&json!(200)))).is_some(),
                "{op} navigates"
            );
        }
        for op in [
            "page.waitForNavigation",
            "page.waitForURL",
            "page.evaluate",
            "page.title",
            "neo.solveCaptcha",
        ] {
            assert_eq!(
                navigated(op, &Ok(reply(&json!(200)))),
                None,
                "{op} may see a navigation, but none of its own"
            );
        }
    }

    #[test]
    fn a_status_is_an_http_one_or_none() {
        for (status, expected) in [
            (json!(200), HttpStatus::new(200)),
            (json!(404), HttpStatus::new(404)),
            (json!(599), HttpStatus::new(599)),
            (json!(0), None),
            (json!(99), None),
            (json!(600), None),
            (json!(70_000), None),
            (json!(-1), None),
            (json!("404"), None),
            (Value::Null, None),
        ] {
            assert_eq!(
                navigated("page.goto", &Ok(reply(&status))),
                Some(Navigated {
                    status: expected,
                    error: None
                }),
                "a reply with status {status}"
            );
        }
        assert_eq!(
            navigated("page.goBack", &Ok(json!({ "response": null, "url": null }))),
            Some(Navigated {
                status: None,
                error: None
            }),
            "a same-document step reads no response"
        );
    }

    #[test]
    fn only_a_network_failure_is_a_net_error() {
        let cases = [
            (
                failed(
                    ErrorCode::NavigationFailed,
                    Some(NetFailure::Named("net::ERR_NAME_NOT_RESOLVED".into())),
                ),
                Some(NetError::NameNotResolved),
            ),
            (
                failed(
                    ErrorCode::NavigationFailed,
                    Some(NetFailure::Named("net::ERR_QUIC_PROTOCOL_ERROR".into())),
                ),
                Some(NetError::Other),
            ),
            (
                failed(ErrorCode::NavigationFailed, Some(NetFailure::ErrorPage)),
                Some(NetError::Other),
            ),
            (failed(ErrorCode::NavigationFailed, None), None),
            (failed(ErrorCode::WaitTimeout, None), None),
            (failed(ErrorCode::TargetCrashed, None), None),
        ];
        for (result, error) in cases {
            assert_eq!(
                navigated("page.reload", &result),
                Some(Navigated {
                    status: None,
                    error
                }),
                "{result:?}"
            );
        }
    }

    /// What the mapping hands over, through the live pipeline to the scrape: a 404 goto and a
    /// net error are both navigations, and only the second is a navigation error.
    #[test]
    fn navigations_reach_the_scrape_under_their_closed_labels() -> Result<(), Box<dyn Error>> {
        let (telemetry, _probe) = Telemetry::in_memory(&TelemetryConfig::default());
        let run = counted_run(&telemetry);
        for result in [
            Ok(reply(&json!(404))),
            Ok(reply(&json!(200))),
            failed(
                ErrorCode::NavigationFailed,
                Some(NetFailure::Named("net::ERR_CONNECTION_REFUSED".into())),
            ),
        ] {
            let navigation = navigated("page.goto", &result).ok_or("a goto navigates")?;
            run.ops().navigated(navigation);
        }
        run.finish(RunVerdict {
            status: RunStatus::Ok,
            error: None,
        });
        assert_eq!(
            Scrape::of(&telemetry)?
                .lines_of(&["neo_navigations_total", "neo_navigation_errors_total"]),
            [
                r#"neo_navigation_errors_total{error_code="connection_refused"} 1"#,
                "neo_navigations_total 1",
                r#"neo_navigations_total{status_code="200"} 1"#,
                r#"neo_navigations_total{status_code="404"} 1"#,
            ],
            "a 404 goto is a navigation like any other; the refused one is also an error"
        );
        Ok(())
    }

    #[test]
    fn every_label_keeps_the_spelling_the_script_reads_but_perimeterx_is_named()
    -> Result<(), Box<dyn Error>> {
        for kind in KINDS {
            let label = family(Some(kind)).as_str();
            match kind {
                // The crate spells it `unknown` for claw-server's MCP schema; the metric names it.
                CaptchaKind::PerimeterX => {
                    assert_eq!((kind.as_str(), label), ("unknown", "perimeterx"));
                }
                _ => assert_eq!(label, kind.as_str(), "{kind:?}"),
            }
        }
        assert_eq!(family(None), CaptchaFamily::None, "no CAPTCHA found");
        for status in STATUSES {
            assert_eq!(verdict(status).as_str(), wire(status)?, "{status:?}");
        }
        for one in REASONS {
            assert_eq!(reason(one).as_str(), wire(one)?, "{one:?}");
        }
        for name in [ProviderName::Capsolver, ProviderName::TwoCaptcha] {
            assert_eq!(provider(name).as_str(), wire(name)?, "{name:?}");
        }
        assert_eq!(
            CaptchaFamily::ALL.len(),
            KINDS.len() + 1,
            "each kind and `none`"
        );
        assert_eq!(CaptchaVerdict::ALL.len(), STATUSES.len());
        assert_eq!(CaptchaReason::ALL.len(), REASONS.len());
        Ok(())
    }

    #[test]
    fn only_a_charged_solve_books_spend_and_never_drops_it() {
        let free = captcha_solve(&outcome(
            Some(CaptchaKind::Turnstile),
            SolveStatus::Solved,
            SolveReason::CheckboxPassed,
            None,
            0.0,
        ));
        assert_eq!(free.spend, None, "a free solve has no provider and no cost");
        let paid = captcha_solve(&outcome(
            Some(CaptchaKind::RecaptchaV2),
            SolveStatus::Failed,
            SolveReason::ProviderError,
            Some(ProviderName::TwoCaptcha),
            0.003,
        ));
        assert_eq!(
            paid.spend,
            Some(CaptchaSpend {
                provider: CaptchaProvider::TwoCaptcha,
                usd: 0.003
            }),
            "a charged failure still costs money"
        );
        let orphan = captcha_solve(&outcome(
            None,
            SolveStatus::Failed,
            SolveReason::ProviderError,
            None,
            0.001,
        ));
        assert_eq!(
            orphan.spend.map(|spend| spend.provider),
            Some(CaptchaProvider::Unknown),
            "a cost without a provider is kept"
        );
    }

    /// Every outcome neo-captcha can return, through the live pipeline: each solve counted once
    /// on both counters, its spend once, and every series under a closed label.
    #[test]
    fn every_solve_outcome_reaches_the_scrape_once() -> Result<(), Box<dyn Error>> {
        let (telemetry, _probe) = Telemetry::in_memory(&TelemetryConfig::default());
        let run = counted_run(&telemetry);
        let ops: RunTelemetry = run.ops();
        let mut solves = 0_u64;
        for kind in KINDS.map(Some).into_iter().chain([None]) {
            for status in STATUSES {
                for one in REASONS {
                    ops.captcha_solved(captcha_solve(&outcome(kind, status, one, None, 0.0)));
                    solves += 1;
                }
            }
        }
        ops.captcha_solved(captcha_solve(&outcome(
            Some(CaptchaKind::PerimeterX),
            SolveStatus::Unsupported,
            SolveReason::UnsupportedType,
            Some(ProviderName::Capsolver),
            0.0025,
        )));
        solves += 1;
        run.finish(RunVerdict {
            status: RunStatus::Ok,
            error: None,
        });
        let scrape = Scrape::of(&telemetry)?;
        let total = |name: &str| -> Result<u64, String> {
            scrape
                .lines_of(&[name])
                .iter()
                .map(|line| line.rsplit(' ').next().unwrap_or_default().parse::<u64>())
                .sum::<Result<u64, _>>()
                .map_err(|error| error.to_string())
        };
        assert_eq!(total("neo_captcha_solves_total")?, solves);
        assert_eq!(total("neo_captcha_reasons_total")?, solves);
        assert_eq!(
            scrape.lines_of(&["neo_captcha_solves_total"]).len(),
            11 * 7,
            "families x verdicts, `none` and `perimeterx` included"
        );
        assert_eq!(scrape.lines_of(&["neo_captcha_reasons_total"]).len(), 22);
        assert_eq!(
            scrape.lines_of(&["neo_captcha_spend_usd_total"]),
            [r#"neo_captcha_spend_usd_total{provider="capsolver"} 0.0025"#]
        );
        assert_eq!(
            scrape.whole(
                "neo_captcha_solves_total",
                &[("family", "perimeterx"), ("verdict", "unsupported")]
            ),
            Some(23),
            "`PerimeterX` under its own name, never folded into `unknown`"
        );
        Ok(())
    }
}
