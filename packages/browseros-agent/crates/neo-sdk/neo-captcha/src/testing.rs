//! Doors for the crate's integration tests. Hidden from the documented API; nothing in
//! production calls them.
//!
//! [`replay_brief`] runs the verify check's decision path over one recorded page
//! (`tests/briefs/*.json`): tier 1's fold and rules, then the judge on hints. It is the code
//! `verify::standing` ends with, minus the reads and the waits, so `tests/verify_replay.rs` can
//! pin what the check concludes on the Nov 19 pages with a fake judge and no browser.

use serde::Deserialize;
use serde_json::Value;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use std::time::Duration;

use crate::{
    evidence::{FrameText, PageBrief},
    judge::Judge,
    turnstile_state::TurnstileWidgetState,
    types::{
        Attempts, CaptchaKind, Challenge, ChallengeState, Detection, DocumentId, FlowResult,
        SolveMethod, SolveReason, SolveStatus, VerifyReport,
    },
    verify::{self, Basis, Claim, Look, Next, TokenReading, VerifyBudget},
};

/// What the verify check concluded on one recorded page.
#[derive(Debug, Clone, PartialEq)]
pub struct Replay {
    /// The wire summary hosts see in `SolveOutcome::verify`.
    pub report: VerifyReport,
    /// The evidence text the brief renders: exactly what the judge was sent, when it was asked.
    pub evidence: String,
    /// What the solve answers after the check, from the claim the brief's basis stands for
    /// (`no_captcha`, `already_passed`, `checkbox_passed`, `token_applied`, `token_accepted`,
    /// a cleared wall); `None` when the check asks for a second round with a remedy.
    pub answer: Option<Answer>,
}

/// A solve's final status and reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Answer {
    pub status: SolveStatus,
    pub reason: SolveReason,
}

/// Replays one recorded page (`tests/briefs/*.json`) through the verify check's decision path.
/// `judge` stands in for Jev; `None` replays a host without one. Fields the replay does not read
/// (the case name, its source, its label, its expectation) are left to the test.
///
/// # Errors
///
/// A case that does not parse, or a probe answer of the wrong shape.
pub async fn replay_brief(case: &str, judge: Option<&dyn Judge>) -> Result<Replay, String> {
    let case: Case = serde_json::from_str(case).map_err(|error| error.to_string())?;
    let document = DocumentId {
        url: "https://replay.test/".into(),
        loader_id: "claim".into(),
    };
    let now = if case.look.new_document {
        DocumentId {
            url: "https://replay.test/next".into(),
            loader_id: "next".into(),
        }
    } else {
        document.clone()
    };
    let before = case
        .before
        .map(PageBrief::from_probe)
        .transpose()
        .map_err(|error| error.to_string())?;
    let basis = case.claim.basis.into();
    let claim = Claim {
        basis,
        kind: case.claim.kind.or_else(|| {
            case.claim
                .challenge
                .as_ref()
                .map(|challenge| challenge.kind)
        }),
        challenge: case.claim.challenge.as_ref(),
        document: &document,
        before: before.as_ref(),
        callback_ran: case.claim.callback_ran,
        applied: None,
    };
    let brief = match case.look.probe {
        Some(probe) => {
            let mut brief = PageBrief::from_probe(probe).map_err(|error| error.to_string())?;
            brief.frames = case
                .look
                .frames
                .into_iter()
                .map(|frame| FrameText {
                    host: frame.host,
                    text: frame.text,
                })
                .collect();
            Some(match &before {
                Some(before) => brief.new_since(before),
                None => brief,
            })
        }
        None => None,
    };
    let look = Look {
        document: Some(now),
        navigated: case.look.new_document && basis != Basis::WallCleared,
        brief,
        rescan: case.look.rescan,
        face: case.look.face.map(Into::into),
        clearance: case.look.clearance,
        policy_wall: case.look.policy_wall,
        token: case.look.token.into(),
        partial: case.look.partial,
    };
    let decision = verify::decide(&verify::signals(&look, &claim), claim.basis);
    let mut budget = VerifyBudget {
        until: Instant::now() + verify::JUDGE_BUDGET * 10,
        judge_calls_left: verify::MAX_JUDGE_CALLS,
    };
    let verification = verify::conclude(
        decision,
        look,
        &claim,
        judge,
        &mut budget,
        &CancellationToken::new(),
    )
    .await
    .map_err(|error| error.to_string())?;
    let passed = case
        .claim
        .challenge
        .as_ref()
        .is_some_and(|challenge| challenge.state == ChallengeState::Passed);
    let (status, reason) = match basis {
        Basis::NothingFound => (SolveStatus::AlreadyClear, SolveReason::NoCaptcha),
        Basis::WidgetPassed if passed => (SolveStatus::AlreadyClear, SolveReason::AlreadyPassed),
        Basis::WidgetPassed => (SolveStatus::Solved, SolveReason::CheckboxPassed),
        Basis::TokenInjected => (SolveStatus::TokenApplied, SolveReason::AwaitingSubmit),
        Basis::TokenAccepted => (SolveStatus::Solved, SolveReason::TokenAccepted),
        Basis::WallCleared => (SolveStatus::Solved, SolveReason::AutoPassed),
    };
    let flow = FlowResult {
        status,
        method: SolveMethod::None,
        reason,
        attempts: Attempts::default(),
        paid_tasks: 0,
        provider: None,
        cost_usd: 0.0,
        callback_ran: case.claim.callback_ran,
        message: String::new(),
    };
    let answer = match verify::next_step(&flow, &verification, &claim, 0, Duration::from_secs(100))
    {
        Next::Return(answer) => Some(Answer {
            status: answer.status,
            reason: answer.reason,
        }),
        Next::Retry(_) => None,
    };
    Ok(Replay {
        answer,
        report: verify::report(
            &verification,
            1,
            verify::MAX_JUDGE_CALLS - budget.judge_calls_left,
        ),
        evidence: verification
            .brief
            .as_ref()
            .map(PageBrief::to_evidence)
            .unwrap_or_default(),
    })
}

/// One recorded page and the claim the check was asked about.
#[derive(Deserialize)]
struct Case {
    claim: ClaimCase,
    /// The probe's answer just before the flow ran, when the recording has one.
    #[serde(default)]
    before: Option<Value>,
    look: LookCase,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimCase {
    basis: BasisCase,
    #[serde(default)]
    kind: Option<CaptchaKind>,
    /// The challenge routing recorded: `active` for a flow's answer, `passed` for
    /// `already_passed`.
    #[serde(default)]
    challenge: Option<Challenge>,
    #[serde(default)]
    callback_ran: bool,
}

/// One look, as the check's reads would have gathered it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LookCase {
    /// The top document is another than the claim's.
    #[serde(default)]
    new_document: bool,
    /// The verify probe's raw answer; absent when the probe could not run.
    #[serde(default)]
    probe: Option<Value>,
    #[serde(default)]
    rescan: Option<Detection>,
    #[serde(default)]
    face: Option<FaceCase>,
    #[serde(default)]
    clearance: bool,
    #[serde(default)]
    policy_wall: bool,
    /// Neo's applied token as the page held it (rows 9 and 11), when the recording knows.
    #[serde(default)]
    token: TokenCase,
    #[serde(default)]
    partial: bool,
    #[serde(default)]
    frames: Vec<FrameCase>,
}

#[derive(Deserialize)]
struct FrameCase {
    host: String,
    text: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BasisCase {
    WidgetPassed,
    TokenInjected,
    TokenAccepted,
    WallCleared,
    NothingFound,
}

impl From<BasisCase> for Basis {
    fn from(basis: BasisCase) -> Self {
        match basis {
            BasisCase::WidgetPassed => Self::WidgetPassed,
            BasisCase::TokenInjected => Self::TokenInjected,
            BasisCase::TokenAccepted => Self::TokenAccepted,
            BasisCase::WallCleared => Self::WallCleared,
            BasisCase::NothingFound => Self::NothingFound,
        }
    }
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TokenCase {
    #[default]
    Unknown,
    Fresh,
    Stale,
}

impl From<TokenCase> for TokenReading {
    fn from(token: TokenCase) -> Self {
        match token {
            TokenCase::Unknown => Self::Unknown,
            TokenCase::Fresh => Self::Fresh,
            TokenCase::Stale => Self::Stale,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FaceCase {
    Success,
    Failed,
    Expired,
    Verifying,
    Interactive,
    Unreadable,
}

impl From<FaceCase> for TurnstileWidgetState {
    fn from(face: FaceCase) -> Self {
        match face {
            FaceCase::Success => Self::Success,
            FaceCase::Failed => Self::Failed,
            FaceCase::Expired => Self::Expired,
            FaceCase::Verifying => Self::Verifying,
            FaceCase::Interactive => Self::Interactive,
            FaceCase::Unreadable => Self::Unreadable,
        }
    }
}
