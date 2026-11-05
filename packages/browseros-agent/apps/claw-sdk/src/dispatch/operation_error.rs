//! Failures with an owner, and the codes they carry.

use std::error::Error;
use std::fmt;

use browseros_trace::{Delivery, ErrorOwner, ErrorRecord, OperationId, SecretSet};

/// A failure and who must change to fix it (STYLE.md P5). Leaves, rules and the door build one
/// with the owner-named constructors at the point that knows the cause; the door adds delivery,
/// `retryable` and the op id when the op ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OperationError {
    owner: ErrorOwner,
    code: ErrorCode,
    message: String,
    timed_out: bool,
}

impl OperationError {
    /// Neo, the browser or the OS failed: a lost connection, a closed target, a limit.
    pub(crate) fn neo(code: ErrorCode, message: impl fmt::Display) -> Self {
        Self::new(ErrorOwner::Neo, code, message)
    }

    /// The agent's own code was wrong: bad input, an unknown method, a stale handle.
    pub(crate) fn script(code: ErrorCode, message: impl fmt::Display) -> Self {
        Self::new(ErrorOwner::Script, code, message)
    }

    /// The page did it: a failed navigation, an element that never became actionable.
    pub(crate) fn website(code: ErrorCode, message: impl fmt::Display) -> Self {
        Self::new(ErrorOwner::Website, code, message)
    }

    /// The host's rules, services or owner: a takeover lock, a caller that went away.
    pub(crate) fn host(code: ErrorCode, message: impl fmt::Display) -> Self {
        Self::new(ErrorOwner::Host, code, message)
    }

    fn new(owner: ErrorOwner, code: ErrorCode, message: impl fmt::Display) -> Self {
        Self {
            owner,
            code,
            message: message.to_string(),
            timed_out: false,
        }
    }

    /// Marks the failure as a timeout, so the script sees a `TimeoutError`, as Playwright
    /// throws for its own timeouts.
    #[must_use]
    pub(crate) fn timed_out(mut self) -> Self {
        self.timed_out = true;
        self
    }

    /// Who must change to fix it.
    #[must_use]
    pub(crate) fn owner(&self) -> ErrorOwner {
        self.owner
    }

    /// The stable code.
    #[must_use]
    pub(crate) fn code(&self) -> ErrorCode {
        self.code
    }

    /// The human message. The door masks it before a script sees it.
    #[must_use]
    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    /// Whether the script sees a `TimeoutError`.
    #[must_use]
    pub(crate) fn is_timeout(&self) -> bool {
        self.timed_out
    }

    /// Replaces every known secret in the message.
    pub(super) fn mask(&mut self, secrets: &SecretSet) {
        secrets.mask_str(&mut self.message);
    }

    /// The error as the trace stores it. The writer masks and bounds the message.
    pub(super) fn record(&self, retryable: bool, operation: Option<OperationId>) -> ErrorRecord {
        ErrorRecord {
            owner: self.owner,
            code: self.code.as_str().to_owned(),
            message: self.message.clone(),
            host_code: None,
            retryable,
            operation_id: operation,
        }
    }
}

impl fmt::Display for OperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} / {}: {}",
            self.owner.as_str(),
            self.code.as_str(),
            self.message
        )
    }
}

impl Error for OperationError {}

/// A stable `snake_case` code. The one list lives here, so the trace, the script's `NeoError`
/// and the run description cannot disagree; each feature adds its codes to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ErrorCode(&'static str);

impl ErrorCode {
    /// No method has that name, or a script asked for one only Neo may call.
    pub(crate) const METHOD_UNAVAILABLE: Self = Self("method_unavailable");
    /// The input did not decode into the method's input type.
    pub(crate) const INVALID_INPUT: Self = Self("invalid_input");
    /// The script returned while this op still ran.
    pub(crate) const NOT_AWAITED: Self = Self("not_awaited");
    /// The caller went away, or a crate's own cancel fired.
    pub(crate) const CANCELLED: Self = Self("cancelled");
    /// The run's time budget ran out.
    pub(crate) const RUN_TIMEOUT: Self = Self("run_timeout");
    /// Neo is stopping.
    pub(crate) const SHUTTING_DOWN: Self = Self("shutting_down");
    /// The CDP link is down, or was replaced since the run began.
    pub(crate) const BROWSER_DISCONNECTED: Self = Self("browser_disconnected");
    /// The target tab is gone.
    pub(crate) const PAGE_CLOSED: Self = Self("page_closed");
    /// The handle names a page id that now belongs to another tab: it predates a Neo restart.
    pub(crate) const STALE_PAGE: Self = Self("stale_page");
    /// A bug in Neo.
    pub(crate) const INTERNAL_ERROR: Self = Self("internal_error");
    /// A CDP call got no answer in time.
    pub(crate) const CDP_TIMEOUT: Self = Self("cdp_timeout");
    /// The browser answered a CDP call with an error.
    pub(crate) const CDP_ERROR: Self = Self("cdp_error");
    /// The owner holds the tab (sdk-traces' takeover lock).
    pub(crate) const OWNER_HAS_CONTROL: Self = Self("owner_has_control");
    /// The script's code did not compile.
    pub(crate) const SYNTAX_ERROR: Self = Self("syntax_error");
    /// The script threw something other than an operation's `NeoError`.
    pub(crate) const SCRIPT_ERROR: Self = Self("script_error");
    /// One stretch of the script's JavaScript ran past the slice budget without awaiting.
    pub(crate) const LONG_TASK: Self = Self("long_task");
    /// The script filled its run's JavaScript heap.
    pub(crate) const OUT_OF_MEMORY: Self = Self("out_of_memory");
    /// The script's return value is larger than a run may return.
    pub(crate) const RESULT_TOO_LARGE: Self = Self("result_too_large");
    /// The host's `_meta["com.browseros.neo/run"]` is malformed.
    pub(crate) const INVALID_META: Self = Self("invalid_meta");
    /// This `requestId` already ran, and its replay window has passed.
    pub(crate) const DUPLICATE_REQUEST: Self = Self("duplicate_request");
    /// The trace store could not be read, so a `requestId` cannot be checked against it.
    pub(crate) const TRACE_UNAVAILABLE: Self = Self("trace_unavailable");
    /// The browser has not connected yet, so a run has no view to start with.
    pub(crate) const BROWSER_UNAVAILABLE: Self = Self("browser_unavailable");
    /// The page did not load: a network error, a refused connection, a dialog in the way, or
    /// no answer within core's navigation timeout.
    pub(crate) const NAVIGATION_FAILED: Self = Self("navigation_failed");

    /// The stored and wire name.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        self.0
    }
}

/// Whether the browser answered a call that failed. The door turns it into delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// The browser said what happened: `done` if the op was sent.
    Definite,
    /// The write may have happened: `unknown` if the op was sent.
    Ambiguous,
}

/// A leaf's failure. `answer` is `None` when the leaf did not classify it, and the door then
/// treats it as `Ambiguous` once the op was sent: a forgotten classification reports `unknown`
/// (look before repeating), never a false `done`, which is how a timeout after "Pay" would be
/// retried into a double charge.
#[derive(Debug)]
pub(crate) struct LeafError {
    pub(super) error: OperationError,
    pub(super) answer: Option<Answer>,
}

impl LeafError {
    /// The browser answered: delivery is `done` if the op was sent.
    #[must_use]
    pub(crate) fn definite(error: OperationError) -> Self {
        Self {
            error,
            answer: Some(Answer::Definite),
        }
    }

    /// The write may have happened: delivery is `unknown` if the op was sent.
    #[must_use]
    pub(crate) fn ambiguous(error: OperationError) -> Self {
        Self {
            error,
            answer: Some(Answer::Ambiguous),
        }
    }
}

impl From<OperationError> for LeafError {
    fn from(error: OperationError) -> Self {
        Self {
            error,
            answer: None,
        }
    }
}

/// What a caller gets when an op fails: the error plus what the door derived. A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpFailure {
    /// Who failed, and how.
    pub(crate) error: OperationError,
    /// Whether the op's side effect happened, as far as Neo can know.
    pub(crate) delivery: Delivery,
    /// Whether repeating the op is safe: nothing was sent, or it only reads.
    pub(crate) retryable: bool,
    /// The op that failed, `{run}.{seq}`.
    pub(crate) operation: OperationId,
}
