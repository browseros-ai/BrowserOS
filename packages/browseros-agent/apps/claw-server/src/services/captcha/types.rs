use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptchaKind {
    RecaptchaV2,
    RecaptchaV3,
    Turnstile,
    CloudflareChallenge,
    Hcaptcha,
    Geetest,
    AwsWaf,
    Arkose,
    Unknown,
}
impl CaptchaKind {
    pub fn is_supported(self) -> bool {
        match self {
            Self::RecaptchaV2 | Self::Turnstile => true,
            Self::Geetest => super::geetest::SUPPORTED,
            Self::AwsWaf => super::aws_waf::SUPPORTED,
            Self::Arkose => super::arkose::SUPPORTED,
            _ => false,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RecaptchaV2 => "recaptcha_v2",
            Self::RecaptchaV3 => "recaptcha_v3",
            Self::Turnstile => "turnstile",
            Self::CloudflareChallenge => "cloudflare_challenge",
            Self::Hcaptcha => "hcaptcha",
            Self::Geetest => "geetest",
            Self::AwsWaf => "aws_waf",
            Self::Arkose => "arkose",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeState {
    Active,
    Passed,
    WidgetOnly,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
} // top-level viewport CSS px
impl Rect {
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameRef {
    pub frame_id: String,
    pub url: String,
    pub rect: Option<Rect>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentId {
    pub url: String,
    pub loader_id: String,
} // main frame

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Challenge {
    pub kind: CaptchaKind,
    pub state: ChallengeState,
    pub frame: Option<FrameRef>, // reCAPTCHA anchor / Turnstile widget iframe
    pub challenge_frame: Option<FrameRef>, // reCAPTCHA bframe (image challenge), when open
    #[serde(default)]
    pub host_frame: Option<FrameRef>,
    pub sitekey: Option<String>,
    pub invisible: bool,
    pub enterprise: bool,
    pub recaptcha_s: Option<String>, // reCAPTCHA data-s
    pub action: Option<String>,      // reCAPTCHA sa / Turnstile action
    pub cdata: Option<String>,       // Turnstile cData
    pub callback: Option<String>,    // name of a page global holding the site callback
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanOutcome {
    Detected,
    None,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    pub outcome: ScanOutcome,
    pub challenges: Vec<Challenge>,
    pub complete: bool,
    pub document: DocumentId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileGrid {
    ThreeByThree,
    FourByFour,
    Single,
}
impl TileGrid {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ThreeByThree => "3x3",
            Self::FourByFour => "4x4",
            Self::Single => "1x1",
        }
    }
    pub fn tiles(self) -> usize {
        match self {
            Self::ThreeByThree => 9,
            Self::FourByFour => 16,
            Self::Single => 1,
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct TilePuzzle {
    pub image_base64: String,
    pub question_id: Option<String>,
    pub question_text: String,
    pub grid: TileGrid,
}

impl std::fmt::Debug for TilePuzzle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TilePuzzle")
            .field(
                "image_base64",
                &format_args!("<{} base64 chars>", self.image_base64.len()),
            )
            .field("question_id", &self.question_id)
            .field("question_text", &self.question_text)
            .field("grid", &self.grid)
            .finish()
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct RecaptchaParams {
    pub website_url: String,
    pub website_key: String,
    pub invisible: bool,
    pub enterprise: bool,
    pub enterprise_s: Option<String>,
    pub page_action: Option<String>,
    pub user_agent: Option<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct TurnstileParams {
    pub website_url: String,
    pub website_key: String,
    pub action: Option<String>,
    pub cdata: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeetestV3Params {
    pub website_url: String,
    pub gt: String,
    pub challenge: String,
    pub api_server: Option<String>,
    pub user_agent: Option<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct GeetestV4Params {
    pub website_url: String,
    pub captcha_id: String,
    pub risk_type: Option<String>,
    pub api_server: Option<String>,
    pub user_agent: Option<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct AwsWafParams {
    pub website_url: String,
    pub key: Option<String>,
    pub iv: Option<String>,
    pub context: Option<String>,
    pub challenge_script: Option<String>,
    pub captcha_script: Option<String>,
    pub jsapi_script: Option<String>,
    pub api_key: Option<String>,
    pub existing_token: Option<String>,
    pub problem_url: Option<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ArkoseParams {
    pub website_url: String,
    pub public_key: String,
    pub api_subdomain: Option<String>,
    pub data_blob: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SolveTask {
    RecaptchaTiles(TilePuzzle),
    RecaptchaToken(RecaptchaParams),
    TurnstileToken(TurnstileParams),
    GeetestV3(GeetestV3Params),
    GeetestV4(GeetestV4Params),
    AwsWaf(AwsWafParams),
    Arkose(ArkoseParams),
}
impl SolveTask {
    pub fn label(&self) -> &'static str {
        match self {
            Self::RecaptchaTiles(_) => "recaptcha_tiles",
            Self::RecaptchaToken(_) => "recaptcha_token",
            Self::TurnstileToken(_) => "turnstile_token",
            Self::GeetestV3(_) => "geetest_v3_token",
            Self::GeetestV4(_) => "geetest_v4_token",
            Self::AwsWaf(_) => "aws_waf_token",
            Self::Arkose(_) => "arkose_token",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum SolveAnswer {
    Tiles(Vec<usize>), /* zero-based, row-major */
    Token(String),
    Fields(BTreeMap<String, String>),
}
impl std::fmt::Debug for SolveAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tiles(tiles) => f.debug_tuple("Tiles").field(tiles).finish(),
            Self::Token(token) => write!(f, "Token(<redacted, {} chars>)", token.chars().count()),
            Self::Fields(fields) => {
                struct Redacted(usize);
                impl std::fmt::Debug for Redacted {
                    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(f, "<redacted, {} chars>", self.0)
                    }
                }
                f.write_str("Fields(")?;
                f.debug_map()
                    .entries(
                        fields
                            .iter()
                            .map(|(key, value)| (key, Redacted(value.chars().count()))),
                    )
                    .finish()?;
                f.write_str(")")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderName {
    Capsolver,
    #[serde(rename = "2captcha")]
    TwoCaptcha,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderAnswer {
    pub answer: SolveAnswer,
    pub provider: ProviderName,
    pub task_id: Option<String>,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    NotConfigured,
    Auth,
    NoBalance,
    Unsupported,
    BadRequest,
    Unsolvable,
    RateLimited,
    Timeout,
    Transport,
    Uncertain,
    SpendLimit,
    Cancelled,
}
impl ProviderErrorKind {
    pub fn may_fallback(self) -> bool {
        !matches!(self, Self::Cancelled | Self::SpendLimit | Self::Uncertain)
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{provider:?} {kind:?}: {message}")]
pub struct ProviderError {
    pub provider: Option<ProviderName>,
    pub kind: ProviderErrorKind,
    pub message: String, /* vendor errorCode + description; never the key */
    pub charged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PageError {
    #[error("page closed")]
    Closed,
    #[error("page navigated")]
    Navigated,
    #[error("cancelled")]
    Cancelled,
    #[error("timed out")]
    Timeout,
    #[error("browser error: {0}")]
    Browser(String),
}

// ----- terminal result: the MCP output (serialized exactly as contract.md) -----
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveStatus {
    Solved,
    TokenApplied,
    AlreadyClear,
    Unsupported,
    Failed,
    Cancelled,
    NeedsHuman,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveMethod {
    None,
    AutoPass,
    Checkbox,
    Tiles,
    Token,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveReason {
    AutoPassed,
    CheckboxPassed,
    TilesAccepted,
    TokenAccepted,  // solved
    AwaitingSubmit, // token_applied
    NoCaptcha,
    AlreadyPassed,
    NotBlocking,     // already_clear
    UnsupportedType, // unsupported
    ScanIncomplete,
    AcceptanceUnconfirmed,
    ProviderError,
    BrowserError,
    DeadlineExceeded, // failed
    CallerCancelled,
    TabClosed,
    Navigated, // cancelled
    NoProvider,
    ProviderUnavailable,
    TilesExhausted,
    ChallengeNotPassed,
    SpendLimitReached, // needs_human
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempts {
    pub checkbox: u32,
    pub tile_rounds: u32,
    pub token: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptchaEventName {
    #[serde(rename = "captcha.detected")]
    Detected,
    #[serde(rename = "captcha.solving")]
    Solving,
    #[serde(rename = "captcha.solved")]
    Solved,
    #[serde(rename = "captcha.token_applied")]
    TokenApplied,
    #[serde(rename = "captcha.failed")]
    Failed,
    #[serde(rename = "captcha.handoff")]
    Handoff,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptchaEvent {
    pub event: CaptchaEventName,
    pub request_id: String,
    pub page: u32,
    pub at_ms: u64,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolveOutcome {
    pub status: SolveStatus,
    #[serde(rename = "type")]
    pub kind: Option<CaptchaKind>,
    pub method: SolveMethod,
    pub reason: SolveReason,
    pub request_id: String,
    pub page: u32,
    pub duration_ms: u64,
    pub attempts: Attempts,
    pub paid_tasks: u32,
    pub cost_usd: f64,
    pub message: String,
    pub events: Vec<CaptchaEvent>,
}

/// What a flow (recaptcha.rs / turnstile.rs) returns; mod.rs adds request id, page, timing, events.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowResult {
    pub status: SolveStatus,
    pub method: SolveMethod,
    pub reason: SolveReason,
    pub attempts: Attempts,
    pub paid_tasks: u32,
    pub cost_usd: f64,
    pub callback_ran: bool,
    pub message: String,
}

/// Active step shown by the busy text and the snapshot status line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveStep {
    Detecting,
    AutoPass,
    Checkbox,
    Tiles { round: u32 },
    Token,
    Verifying,
}

#[cfg(test)]
pub(super) fn new_token_tasks() -> [(CaptchaKind, SolveTask, &'static str); 4] {
    [
        (
            CaptchaKind::Geetest,
            SolveTask::GeetestV3(GeetestV3Params {
                website_url: "https://fixture.test".into(),
                gt: "gt".into(),
                challenge: "challenge".into(),
                api_server: None,
                user_agent: None,
            }),
            "geetest_v3_token",
        ),
        (
            CaptchaKind::Geetest,
            SolveTask::GeetestV4(GeetestV4Params {
                website_url: "https://fixture.test".into(),
                captcha_id: "captcha-id".into(),
                risk_type: None,
                api_server: None,
                user_agent: None,
            }),
            "geetest_v4_token",
        ),
        (
            CaptchaKind::AwsWaf,
            SolveTask::AwsWaf(AwsWafParams {
                website_url: "https://fixture.test".into(),
                key: None,
                iv: None,
                context: None,
                challenge_script: None,
                captcha_script: None,
                jsapi_script: None,
                api_key: None,
                existing_token: None,
                problem_url: None,
            }),
            "aws_waf_token",
        ),
        (
            CaptchaKind::Arkose,
            SolveTask::Arkose(ArkoseParams {
                website_url: "https://fixture.test".into(),
                public_key: "public-key".into(),
                api_subdomain: None,
                data_blob: None,
                user_agent: None,
            }),
            "arkose_token",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, to_value};

    #[test]
    fn tests_that_new_token_tasks_have_stable_labels() {
        for (_, task, label) in new_token_tasks() {
            assert_eq!(task.label(), label);
        }
    }

    #[test]
    fn tests_that_fields_debug_redacts_every_value() {
        let answer = SolveAnswer::Fields(std::collections::BTreeMap::from([
            ("validate".into(), "private-validation".into()),
            ("seccode".into(), "秘密".into()),
        ]));
        let debug = format!("{answer:?}");
        assert_eq!(
            debug,
            "Fields({\"seccode\": <redacted, 2 chars>, \"validate\": <redacted, 18 chars>})"
        );
        assert!(!debug.contains("private-validation"));
        assert!(!debug.contains("秘密"));
        assert_eq!(
            format!("{:?}", SolveAnswer::Fields(Default::default())),
            "Fields({})"
        );
    }

    #[test]
    fn tests_that_challenge_host_frame_is_optional_on_the_wire() -> anyhow::Result<()> {
        let mut wire = json!({"kind":"geetest", "state":"active", "frame":null,
            "challengeFrame":null, "sitekey":null, "invisible":false, "enterprise":false,
            "recaptchaS":null, "action":null, "cdata":null, "callback":null});
        let legacy: Challenge = serde_json::from_value(wire.clone())?;
        assert_eq!(legacy.host_frame, None);
        wire["hostFrame"] = json!({"frameId":"host", "url":"https://fixture.test", "rect":null});
        let challenge: Challenge = serde_json::from_value(wire.clone())?;
        assert_eq!(
            challenge
                .host_frame
                .as_ref()
                .map(|frame| frame.frame_id.as_str()),
            Some("host")
        );
        assert_eq!(to_value(challenge)?, wire);
        Ok(())
    }

    #[test]
    fn tests_that_token_applied_values_round_trip() -> anyhow::Result<()> {
        let status: SolveStatus = serde_json::from_value(json!("token_applied"))?;
        let reason: SolveReason = serde_json::from_value(json!("awaiting_submit"))?;
        let event: CaptchaEventName = serde_json::from_value(json!("captcha.token_applied"))?;
        assert_eq!(to_value(status)?, "token_applied");
        assert_eq!(to_value(reason)?, "awaiting_submit");
        assert_eq!(to_value(event)?, "captcha.token_applied");
        Ok(())
    }

    #[test]
    fn tests_that_each_terminal_status_matches_the_wire_contract() -> anyhow::Result<()> {
        for (status, status_wire, reason, reason_wire, kind, method, method_wire) in [
            (
                SolveStatus::TokenApplied,
                "token_applied",
                SolveReason::AwaitingSubmit,
                "awaiting_submit",
                Some(CaptchaKind::RecaptchaV2),
                SolveMethod::Token,
                "token",
            ),
            (
                SolveStatus::Solved,
                "solved",
                SolveReason::CheckboxPassed,
                "checkbox_passed",
                Some(CaptchaKind::RecaptchaV2),
                SolveMethod::Checkbox,
                "checkbox",
            ),
            (
                SolveStatus::AlreadyClear,
                "already_clear",
                SolveReason::NoCaptcha,
                "no_captcha",
                None,
                SolveMethod::None,
                "none",
            ),
            (
                SolveStatus::Unsupported,
                "unsupported",
                SolveReason::UnsupportedType,
                "unsupported_type",
                Some(CaptchaKind::Hcaptcha),
                SolveMethod::None,
                "none",
            ),
            (
                SolveStatus::Failed,
                "failed",
                SolveReason::BrowserError,
                "browser_error",
                Some(CaptchaKind::Turnstile),
                SolveMethod::Token,
                "token",
            ),
            (
                SolveStatus::Cancelled,
                "cancelled",
                SolveReason::CallerCancelled,
                "caller_cancelled",
                None,
                SolveMethod::None,
                "none",
            ),
            (
                SolveStatus::NeedsHuman,
                "needs_human",
                SolveReason::TilesExhausted,
                "tiles_exhausted",
                Some(CaptchaKind::RecaptchaV2),
                SolveMethod::Tiles,
                "tiles",
            ),
        ] {
            let outcome = SolveOutcome {
                status,
                kind,
                method,
                reason,
                request_id: "cap_test".into(),
                page: 42,
                duration_ms: 3120,
                attempts: Attempts {
                    checkbox: 1,
                    tile_rounds: 2,
                    token: 3,
                },
                paid_tasks: 5,
                cost_usd: 0.01,
                message: "test result".into(),
                events: vec![CaptchaEvent {
                    event: CaptchaEventName::Detected,
                    request_id: "cap_test".into(),
                    page: 42,
                    at_ms: 140,
                    data: json!({"state": "active", "complete": true, "challenges": 1}),
                }],
            };
            let wire = to_value(&outcome)?;
            assert_eq!(
                wire,
                json!({
                    "status": status_wire, "type": kind, "method": method_wire, "reason": reason_wire,
                    "requestId": "cap_test", "page": 42, "durationMs": 3120,
                    "attempts": {"checkbox": 1, "tileRounds": 2, "token": 3},
                    "paidTasks": 5, "costUsd": 0.01, "message": "test result",
                    "events": [{"event": "captcha.detected", "requestId": "cap_test", "page": 42,
                        "atMs": 140, "data": {"state": "active", "complete": true, "challenges": 1}}]
                })
            );
            assert_eq!(serde_json::from_value::<SolveOutcome>(wire)?, outcome);
        }
        Ok(())
    }

    #[test]
    fn tests_that_enums_match_the_contract_spellings() -> anyhow::Result<()> {
        for (kind, wire) in [
            (CaptchaKind::RecaptchaV2, "recaptcha_v2"),
            (CaptchaKind::RecaptchaV3, "recaptcha_v3"),
            (CaptchaKind::Turnstile, "turnstile"),
            (CaptchaKind::CloudflareChallenge, "cloudflare_challenge"),
            (CaptchaKind::Hcaptcha, "hcaptcha"),
            (CaptchaKind::Geetest, "geetest"),
            (CaptchaKind::AwsWaf, "aws_waf"),
            (CaptchaKind::Arkose, "arkose"),
            (CaptchaKind::Unknown, "unknown"),
        ] {
            assert_eq!(to_value(kind)?, wire);
            assert_eq!(kind.as_str(), wire);
            assert_eq!(
                kind.is_supported(),
                match kind {
                    CaptchaKind::RecaptchaV2 | CaptchaKind::Turnstile => true,
                    CaptchaKind::Geetest => super::super::geetest::SUPPORTED,
                    CaptchaKind::AwsWaf => super::super::aws_waf::SUPPORTED,
                    CaptchaKind::Arkose => super::super::arkose::SUPPORTED,
                    _ => false,
                }
            );
        }
        for (event, wire) in [
            (CaptchaEventName::Detected, "captcha.detected"),
            (CaptchaEventName::Solving, "captcha.solving"),
            (CaptchaEventName::Solved, "captcha.solved"),
            (CaptchaEventName::TokenApplied, "captcha.token_applied"),
            (CaptchaEventName::Failed, "captcha.failed"),
            (CaptchaEventName::Handoff, "captcha.handoff"),
        ] {
            assert_eq!(to_value(event)?, wire);
        }
        assert_eq!(to_value(SolveMethod::AutoPass)?, "auto_pass");
        assert_eq!(to_value(ChallengeState::WidgetOnly)?, "widget_only");
        assert_eq!(to_value(ScanOutcome::Unknown)?, "unknown");
        assert_eq!(to_value(ProviderName::Capsolver)?, "capsolver");
        assert_eq!(to_value(SolveStep::AutoPass)?, "auto_pass");
        Ok(())
    }

    #[test]
    fn tests_that_token_debug_redacts_the_secret() {
        let token = "unique-secret-token";
        let debug = format!("{:?}", SolveAnswer::Token(token.into()));
        assert!(!debug.contains(token));
        assert_eq!(debug, format!("Token(<redacted, {} chars>)", token.len()));
        assert_eq!(
            format!("{:?}", SolveAnswer::Tiles(vec![0, 3])),
            "Tiles([0, 3])"
        );
    }
}
