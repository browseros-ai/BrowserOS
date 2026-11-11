//! Every failure the crate returns, in claw-sdk's error model.
//!
//! A leaf picks an [`ErrorCode`], and that code's row in one table ([`ErrorCode::row`]) fixes
//! who owns the failure, whether it is a Playwright timeout (the bootstrap then throws
//! `TimeoutError`) and whether Neo can vouch for what happened. No leaf can pair a code with
//! the wrong owner. Messages are Playwright-shaped (`locator.click: Timeout 5000ms exceeded.`,
//! then the call log) and quote only what the sealed world returned, so they carry no field
//! values.
//!
//! The answer column is what holds once the op may have sent something. The dispatcher reports
//! `not_sent` for an op that never called `OpScope::send_effect`, whatever the answer says
//! (skeleton D7.5), so a code needs no "before or after the effect" variant: timeouts, a closed
//! target and a lost connection are ambiguous, and every failure Neo saw end, a CDP error reply
//! included (sdk-traces D2), is definite.
//!
//! One exception widens the answer. An action that sent input and then lost track of what it
//! did (its hit-target verdict came back unreadable) is ambiguous whatever its code, because
//! the input may have landed; the leaf says so with [`PlaywrightError::after_input`]. The
//! owner and the code stay the code's.
//!
//! A host policy's refusal is not one of these codes. It passes through with the policy's own
//! owner, code and host code (`host / owner_has_control`).

use std::error::Error;
use std::fmt;

use browseros_cdp::CdpError;
use browseros_core::CoreError;
use neo_trace::ErrorOwner;
use serde_json::Value;

use crate::Refusal;
use crate::listed_enum::listed_enum;

listed_enum! {
    /// The crate's error vocabulary. [`ErrorCode::as_str`] is the stable wire name.
    pub enum ErrorCode {
        /// A strict call matched two or more elements.
        StrictModeViolation,
        /// Nothing matched for the op's whole timeout; also `expect` with no element.
        NoMatch,
        /// Matched, but stayed hidden, disabled, unstable, not editable or covered.
        NotActionable,
        /// An `expect` matcher failed with its element present.
        AssertionFailed,
        /// A ref whose element or document is gone, or a detached element handle. Never waits.
        ElementDetached,
        /// A main-world script threw (`evaluate`, `waitForFunction`).
        PageError,
        /// A navigation failed with a net error.
        NavigationFailed,
        /// A download the script waited on, through `download.saveAs`, was canceled or
        /// interrupted, or finished where Neo cannot read it.
        DownloadFailed,
        /// The page never reached a state the op waited for: a load state, a URL, an event, a
        /// function's truthy result, a finished download.
        WaitTimeout,
        /// Neo did not finish observing a document already restored with all load states.
        NavigationObservationTimeout,
        /// A bad selector, role, option or argument, a ref Neo never prints, or an input that
        /// does not decode.
        InvalidInput,
        /// A Playwright feature Neo does not support here, such as a popup that opened as a
        /// window.
        Unsupported,
        /// The page or frame closed, or left the run's view, under the op.
        TargetClosed,
        /// The CDP connection dropped, changed epoch or stopped answering.
        BrowserDisconnected,
        /// The script aborted the op through Playwright's `signal` option. Like a timeout, the
        /// op's input may already have landed, but the script chose to stop it, so it is not
        /// one of Playwright's timeouts: the prelude throws `AbortError`.
        Aborted,
        /// A Neo bug: a stale engine asset, or a probe reply Rust cannot read.
        Internal,
    }
}

/// Whether Neo knows what happened to an op that may have sent something.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Answer {
    /// The outcome is known: success, or a failure Neo saw end.
    Definite,
    /// A timeout or a lost page or connection after the op may have sent input or a
    /// navigation. The host reports `unknown` if the op was marked sent, else `not_sent`.
    Ambiguous,
}

/// Whether a failure is a Playwright timeout, so the bootstrap throws `TimeoutError`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimedOut {
    /// Not a timeout.
    No,
    /// The op's own Playwright timeout passed.
    Yes,
}

/// One row of the error table.
struct Row {
    name: &'static str,
    owner: ErrorOwner,
    timed_out: TimedOut,
    answer: Answer,
}

const fn row(name: &'static str, owner: ErrorOwner, timed_out: TimedOut, answer: Answer) -> Row {
    Row {
        name,
        owner,
        timed_out,
        answer,
    }
}

impl ErrorCode {
    /// The error table. Each code's owner says whose behavior must change to fix it (P5).
    const fn row(self) -> Row {
        use Answer::{Ambiguous, Definite};
        use ErrorOwner::{Neo, Script, Website};
        use TimedOut::{No, Yes};
        match self {
            Self::StrictModeViolation => row("strict_mode_violation", Script, No, Definite),
            Self::NoMatch => row("no_match", Script, Yes, Ambiguous),
            Self::NotActionable => row("not_actionable", Website, Yes, Ambiguous),
            Self::AssertionFailed => row("assertion_failed", Website, Yes, Ambiguous),
            Self::ElementDetached => row("element_detached", Website, No, Definite),
            Self::PageError => row("page_error", Website, No, Definite),
            Self::NavigationFailed => row("navigation_failed", Website, No, Definite),
            Self::DownloadFailed => row("download_failed", Website, No, Definite),
            Self::WaitTimeout => row("wait_timeout", Website, Yes, Ambiguous),
            Self::NavigationObservationTimeout => {
                row("navigation_observation_timeout", Neo, Yes, Ambiguous)
            }
            Self::InvalidInput => row("invalid_input", Script, No, Definite),
            Self::Unsupported => row("unsupported", Neo, No, Definite),
            Self::TargetClosed => row("target_closed", Neo, No, Ambiguous),
            Self::BrowserDisconnected => row("browser_disconnected", Neo, No, Ambiguous),
            Self::Aborted => row("aborted", Script, No, Ambiguous),
            Self::Internal => row("internal", Neo, No, Definite),
        }
    }

    /// The stored and wire name, such as `strict_mode_violation`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.row().name
    }

    /// Whose behavior must change to fix the failure.
    #[must_use]
    pub const fn owner(self) -> ErrorOwner {
        self.row().owner
    }

    /// Whether the failure is a Playwright timeout.
    #[must_use]
    pub const fn timed_out(self) -> TimedOut {
        self.row().timed_out
    }

    /// Whether Neo can vouch for what happened, once the op may have sent something.
    #[must_use]
    pub const fn answer(self) -> Answer {
        self.row().answer
    }

    /// How a core or CDP failure reads as one of these codes. Deliberately not a
    /// `From<CoreError>`: a leaf first handles the core errors it can recover from (a stale
    /// execution context re-installs the world, a gone frame session is evicted and retried)
    /// and converts only what is left, visibly, through `OpScope::core_error`.
    pub(crate) fn from_core(error: &CoreError) -> Self {
        match error {
            CoreError::Cdp(cdp) => match cdp {
                CdpError::SessionGone => Self::TargetClosed,
                // Tab/target lookup can race a close before the detached event reaches the
                // journal. These exact missing-identity replies are closure evidence; other
                // protocol failures remain Internal.
                CdpError::Protocol { message, .. }
                    if message == "No tab with given id"
                        || message == "No target with given id found" =>
                {
                    Self::TargetClosed
                }
                CdpError::ConnectionLost
                | CdpError::NotConnected
                | CdpError::ConnectionEpochChanged { .. }
                | CdpError::Timeout { .. }
                | CdpError::Transport(_)
                | CdpError::Discovery(_) => Self::BrowserDisconnected,
                CdpError::Protocol { .. } | CdpError::Json(_) => Self::Internal,
            },
            CoreError::UnknownPage(_) | CoreError::UnknownPageShort(_) | CoreError::Target(_) => {
                Self::TargetClosed
            }
            CoreError::PageScript(_) => Self::PageError,
            CoreError::NativeFeatureUnavailable { .. } => Self::Unsupported,
            CoreError::UnknownRef(_)
            | CoreError::StaleRef { .. }
            | CoreError::DocumentChanged
            | CoreError::CrossFrameDrag
            | CoreError::InvalidDragTarget
            | CoreError::ElementCovered { .. }
            | CoreError::ClickPointUnavailable(_)
            | CoreError::ClickFallbackUnavailable { .. }
            | CoreError::ClickOutcomeUnknown(_)
            | CoreError::IsolatedWorldUnavailable
            | CoreError::Message(_) => Self::Internal,
        }
    }
}

/// An operation failed. claw-sdk's adapter (`crates/neo-sdk/neo-server/src/builtins/playwright_ops.rs`)
/// reads it through the accessors and builds the dispatcher's `LeafError` from them.
#[derive(Clone, Debug)]
pub struct PlaywrightError {
    failure: Failure,
    /// Playwright-shaped, call log included.
    message: String,
    /// The op sent input and cannot tell what it did: the answer is ambiguous.
    outcome_lost: bool,
    /// What a failed `expect` matcher saw (`{ actual?, ariaSnapshot?, log, timedOut }`), which
    /// the expect prelude completes into Playwright's `error.matcherResult`. Hosts pass it to
    /// the script as it is; boxed, so every other error stays small.
    matcher: Option<Box<Value>>,
}

/// What failed: one of the crate's codes, or a host policy that refused.
#[derive(Clone, Debug)]
enum Failure {
    Code(ErrorCode),
    Refused {
        owner: ErrorOwner,
        code: String,
        host_code: Option<String>,
    },
}

impl PlaywrightError {
    /// A failure with one of the crate's codes; its owner, timeout and answer come from the
    /// code's row. The crate's leaves write the message as Playwright does
    /// (`locator.click: ...`); hosts build these only to test their own mapping.
    #[must_use]
    pub fn new(code: ErrorCode, message: String) -> Self {
        Self {
            failure: Failure::Code(code),
            message,
            outcome_lost: false,
            matcher: None,
        }
    }

    /// Whose behavior must change to fix it.
    #[must_use]
    pub fn owner(&self) -> ErrorOwner {
        match &self.failure {
            Failure::Code(code) => code.owner(),
            Failure::Refused { owner, .. } => *owner,
        }
    }

    /// The stable code: one of [`ErrorCode::as_str`], or the refusing policy's own code.
    #[must_use]
    pub fn code(&self) -> &str {
        match &self.failure {
            Failure::Code(code) => code.as_str(),
            Failure::Refused { code, .. } => code,
        }
    }

    /// The crate's code, for callers that branch on it; `None` for a host refusal.
    #[must_use]
    pub fn error_code(&self) -> Option<ErrorCode> {
        match &self.failure {
            Failure::Code(code) => Some(*code),
            Failure::Refused { .. } => None,
        }
    }

    /// The Playwright-shaped message, call log included.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The host's own code, carried by a refusal whose code is `host_error`.
    #[must_use]
    pub fn host_code(&self) -> Option<&str> {
        match &self.failure {
            Failure::Code(_) => None,
            Failure::Refused { host_code, .. } => host_code.as_deref(),
        }
    }

    /// See [`TimedOut`]. A refusal never times out.
    #[must_use]
    pub fn timed_out(&self) -> TimedOut {
        match &self.failure {
            Failure::Code(code) => code.timed_out(),
            Failure::Refused { .. } => TimedOut::No,
        }
    }

    /// See [`Answer`]. A refusal comes before anything is sent, so it is definite.
    #[must_use]
    pub fn answer(&self) -> Answer {
        match &self.failure {
            Failure::Code(_) if self.outcome_lost => Answer::Ambiguous,
            Failure::Code(code) => code.answer(),
            Failure::Refused { .. } => Answer::Definite,
        }
    }

    /// A failed `expect` matcher's facts for the script's `error.matcherResult`; `None` for
    /// every other failure. Opaque to hosts: the expect prelude reads it.
    #[must_use]
    pub fn matcher_details(&self) -> Option<&Value> {
        self.matcher.as_deref()
    }

    /// The same failure, carrying a failed matcher's facts (see [`Self::matcher_details`]).
    #[must_use]
    pub(crate) fn with_matcher(mut self, details: Value) -> Self {
        self.matcher = Some(Box::new(details));
        self
    }

    /// The same failure, raised after the op sent input it can no longer account for, such as
    /// a click whose verdict came back unreadable: the answer becomes ambiguous.
    #[must_use]
    pub(crate) fn after_input(mut self) -> Self {
        self.outcome_lost = true;
        self
    }

    /// `script / invalid_input`, for inputs that fail before an op's scope exists.
    pub(crate) fn invalid_input(message: String) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }
}

/// A host policy's refusal, passed through unchanged: the policy owns the failure.
impl From<Refusal> for PlaywrightError {
    fn from(refusal: Refusal) -> Self {
        Self {
            failure: Failure::Refused {
                owner: refusal.owner,
                code: refusal.code,
                host_code: refusal.host_code,
            },
            message: refusal.message,
            outcome_lost: false,
            matcher: None,
        }
    }
}

impl fmt::Display for PlaywrightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for PlaywrightError {}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use browseros_cdp::CdpError;
    use browseros_core::{CoreError, PageId, TargetError};
    use neo_trace::ErrorOwner;

    use super::{Answer, ErrorCode, PlaywrightError, TimedOut};
    use crate::Refusal;

    /// The error table as the sdk-playwright design states it, plus `wait_timeout`, written
    /// out apart from the code so a wrong row there fails here.
    const TABLE: [(&str, ErrorOwner, TimedOut, Answer); 16] = [
        (
            "strict_mode_violation",
            ErrorOwner::Script,
            TimedOut::No,
            Answer::Definite,
        ),
        (
            "no_match",
            ErrorOwner::Script,
            TimedOut::Yes,
            Answer::Ambiguous,
        ),
        (
            "not_actionable",
            ErrorOwner::Website,
            TimedOut::Yes,
            Answer::Ambiguous,
        ),
        (
            "assertion_failed",
            ErrorOwner::Website,
            TimedOut::Yes,
            Answer::Ambiguous,
        ),
        (
            "element_detached",
            ErrorOwner::Website,
            TimedOut::No,
            Answer::Definite,
        ),
        (
            "page_error",
            ErrorOwner::Website,
            TimedOut::No,
            Answer::Definite,
        ),
        (
            "navigation_failed",
            ErrorOwner::Website,
            TimedOut::No,
            Answer::Definite,
        ),
        (
            "download_failed",
            ErrorOwner::Website,
            TimedOut::No,
            Answer::Definite,
        ),
        (
            "wait_timeout",
            ErrorOwner::Website,
            TimedOut::Yes,
            Answer::Ambiguous,
        ),
        (
            "navigation_observation_timeout",
            ErrorOwner::Neo,
            TimedOut::Yes,
            Answer::Ambiguous,
        ),
        (
            "invalid_input",
            ErrorOwner::Script,
            TimedOut::No,
            Answer::Definite,
        ),
        (
            "unsupported",
            ErrorOwner::Neo,
            TimedOut::No,
            Answer::Definite,
        ),
        (
            "target_closed",
            ErrorOwner::Neo,
            TimedOut::No,
            Answer::Ambiguous,
        ),
        (
            "browser_disconnected",
            ErrorOwner::Neo,
            TimedOut::No,
            Answer::Ambiguous,
        ),
        (
            "aborted",
            ErrorOwner::Script,
            TimedOut::No,
            Answer::Ambiguous,
        ),
        ("internal", ErrorOwner::Neo, TimedOut::No, Answer::Definite),
    ];

    #[test]
    fn tests_that_every_code_has_its_owner_timeout_and_answer() {
        let rows: Vec<_> = ErrorCode::ALL
            .iter()
            .map(|code| (code.as_str(), code.owner(), code.timed_out(), code.answer()))
            .collect();
        assert_eq!(
            rows, TABLE,
            "the code table must match the design's error table"
        );
    }

    #[test]
    fn tests_that_wire_names_are_unique_snake_case() {
        let mut names = HashSet::new();
        for code in ErrorCode::ALL {
            let name = code.as_str();
            assert!(names.insert(name), "{name} is used twice");
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{name} must be snake_case"
            );
        }
    }

    #[test]
    fn tests_that_every_timeout_is_ambiguous() {
        for code in ErrorCode::ALL {
            if code.timed_out() == TimedOut::Yes {
                assert_eq!(
                    code.answer(),
                    Answer::Ambiguous,
                    "{} times out, and a timed-out effect may have landed",
                    code.as_str()
                );
            }
        }
    }

    #[test]
    fn tests_that_an_error_reports_its_code_row() {
        let error = PlaywrightError::new(
            ErrorCode::NotActionable,
            "locator.click: Timeout 5000ms exceeded.".to_owned(),
        );
        assert_eq!(error.owner(), ErrorOwner::Website);
        assert_eq!(error.code(), "not_actionable");
        assert_eq!(error.error_code(), Some(ErrorCode::NotActionable));
        assert_eq!(error.timed_out(), TimedOut::Yes);
        assert_eq!(error.answer(), Answer::Ambiguous);
        assert_eq!(error.host_code(), None);
        assert_eq!(error.to_string(), "locator.click: Timeout 5000ms exceeded.");
    }

    #[test]
    fn tests_that_a_refusal_passes_through_unchanged() {
        let error = PlaywrightError::from(Refusal {
            owner: ErrorOwner::Host,
            code: "host_error".to_owned(),
            message: "the owner has control of this tab".to_owned(),
            host_code: Some("takeover".to_owned()),
        });
        assert_eq!(
            error.owner(),
            ErrorOwner::Host,
            "the refusing policy owns it"
        );
        assert_eq!(error.code(), "host_error");
        assert_eq!(
            error.error_code(),
            None,
            "a refusal is not one of the crate's codes"
        );
        assert_eq!(error.host_code(), Some("takeover"));
        assert_eq!(error.message(), "the owner has control of this tab");
        assert_eq!(error.timed_out(), TimedOut::No);
        assert_eq!(
            error.answer(),
            Answer::Definite,
            "nothing was sent before a refusal"
        );
    }

    #[test]
    fn missing_tab_and_target_replies_mean_closed_but_other_protocol_errors_do_not() {
        for (message, expected) in [
            ("No tab with given id", ErrorCode::TargetClosed),
            ("No target with given id found", ErrorCode::TargetClosed),
            ("Cannot find context with specified id", ErrorCode::Internal),
        ] {
            let error = CoreError::Cdp(CdpError::Protocol {
                code: -32000,
                message: message.to_owned(),
            });
            assert_eq!(ErrorCode::from_core(&error), expected, "{message}");
        }
    }

    #[test]
    fn tests_that_core_failures_map_to_their_owner() {
        let cases = [
            (
                CoreError::Cdp(CdpError::SessionGone),
                ErrorCode::TargetClosed,
            ),
            (
                CoreError::Cdp(CdpError::ConnectionLost),
                ErrorCode::BrowserDisconnected,
            ),
            (
                CoreError::Cdp(CdpError::NotConnected),
                ErrorCode::BrowserDisconnected,
            ),
            (
                CoreError::Cdp(CdpError::Timeout {
                    method: "Input.dispatchMouseEvent".to_owned(),
                }),
                ErrorCode::BrowserDisconnected,
            ),
            (
                CoreError::Cdp(CdpError::Protocol {
                    code: -32000,
                    message: "Cannot find context with specified id".to_owned(),
                }),
                ErrorCode::Internal,
            ),
            (CoreError::UnknownPage(PageId(7)), ErrorCode::TargetClosed),
            (
                CoreError::Target(TargetError::PageOutOfScope(PageId(7))),
                ErrorCode::TargetClosed,
            ),
            (
                CoreError::PageScript("TypeError: x is undefined".to_owned()),
                ErrorCode::PageError,
            ),
            (CoreError::IsolatedWorldUnavailable, ErrorCode::Internal),
        ];
        for (error, code) in cases {
            assert_eq!(ErrorCode::from_core(&error), code, "{error}");
        }
    }
}
