//! A CAPTCHA solve's outcome as telemetry counts it: the family it found, how and why it ended,
//! and what it cost.
//!
//! It reads the outcome the solver returned, never the trace, so the `trace` settings change no
//! number. Every match over neo-captcha's enums is exhaustive (STYLE.md P8): a new kind, status,
//! reason or provider fails the build here instead of reaching a metric under a guessed label.

use neo_captcha::types::{CaptchaKind, ProviderName, SolveOutcome, SolveReason, SolveStatus};
use neo_telemetry::label::{CaptchaFamily, CaptchaProvider, CaptchaReason, CaptchaVerdict};
use neo_telemetry::{CaptchaSolve, CaptchaSpend};

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
    use neo_telemetry::label::{
        CaptchaFamily, CaptchaProvider, CaptchaReason, CaptchaVerdict, RunStatus,
    };
    use neo_telemetry::{
        CaptchaSpend, RunFacts, RunTelemetry, RunVerdict, Telemetry, TelemetryConfig,
    };

    use super::{captcha_solve, family, provider, reason, verdict};
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
            scrape.lines_of(&["neo_captcha_spend_total"]),
            [r#"neo_captcha_spend_total{neo_captcha_provider="capsolver"} 0.0025"#]
        );
        assert_eq!(
            scrape.whole(
                "neo_captcha_solves_total",
                &[
                    ("neo_captcha_family", "perimeterx"),
                    ("neo_captcha_verdict", "unsupported")
                ]
            ),
            Some(23),
            "`PerimeterX` under its own name, never folded into `unknown`"
        );
        Ok(())
    }
}
