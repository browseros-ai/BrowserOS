//! Failures with an owner, and the codes they carry.

use std::error::Error;
use std::fmt;

use neo_trace::{Delivery, ErrorOwner, ErrorRecord, OperationId};
use serde_json::Value;

use crate::browser::TaskError;

/// How many characters of a script-chosen text the door keeps in an error it builds: a thrown
/// error's name and its message, and a prelude refusal's reason (FR/RC C4). One constant, so they
/// cannot drift. The trace writer bounds what it stores on its own.
pub(crate) const TEXT_CHARS: usize = 4096;

/// `text` cut to `chars` characters, with `…` marking the cut. The door's bound on a text the
/// script chose before it goes into one of the door's own errors, so a script cannot make a
/// refusal, or the run's error, as large as it likes.
#[must_use]
pub(crate) fn clip(mut text: String, chars: usize) -> String {
    if let Some((cut, _)) = text.char_indices().nth(chars) {
        text.truncate(cut);
        text.push('…');
    }
    text
}

/// A failure and who must change to fix it (STYLE.md P5). Leaves, rules and the door build one
/// with the owner-named constructors at the point that knows the cause; the door adds delivery,
/// `retryable` and the op id when the op ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OperationError {
    owner: ErrorOwner,
    code: ErrorCode,
    message: String,
    timed_out: bool,
    /// A failed `expect` matcher's facts from neo-playwright, which the script's error carries
    /// as `matcherResult` for the expect prelude to complete. The door never reads them, and the
    /// trace does not store them.
    matcher: Option<Box<Value>>,
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
            matcher: None,
        }
    }

    /// Marks the failure as a timeout, so the script sees a `TimeoutError`, as Playwright
    /// throws for its own timeouts.
    #[must_use]
    pub(crate) fn timed_out(mut self) -> Self {
        self.timed_out = true;
        self
    }

    /// The same failure, carrying a failed matcher's facts for the script's error.
    #[must_use]
    pub(crate) fn with_matcher(mut self, facts: Value) -> Self {
        self.matcher = Some(Box::new(facts));
        self
    }

    /// The same failure without a matcher's facts, for a copy kept after the script has them.
    #[must_use]
    pub(crate) fn without_matcher(mut self) -> Self {
        self.matcher = None;
        self
    }

    /// A failed matcher's facts, for the script's error; `None` for every other failure.
    #[must_use]
    pub(crate) fn matcher(&self) -> Option<&Value> {
        self.matcher.as_deref()
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

    /// The human message.
    #[must_use]
    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    /// Whether the script sees a `TimeoutError`.
    #[must_use]
    pub(crate) fn is_timeout(&self) -> bool {
        self.timed_out
    }

    /// The error as the trace stores it. The writer bounds the message.
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
/// and the run description cannot disagree; each feature adds its codes to it, in
/// `error_codes!` below.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ErrorCode(&'static str);

/// Declares each [`ErrorCode`] constant and [`ErrorCode::KNOWN`] from the same tokens, as
/// neo-playwright's `listed_enum!` does for its enums. A refusal comes back through that
/// crate as a bare string, and [`ErrorCode::known`] reads it back by this list, so a code kept
/// off a hand-kept list would compile and then reach the agent as `internal_error`.
macro_rules! error_codes {
    ($($(#[$doc:meta])* $name:ident = $code:literal;)+) => {
        impl ErrorCode {
            $($(#[$doc])* pub(crate) const $name: Self = Self($code);)+

            /// Every code above, in declaration order.
            const KNOWN: &'static [Self] = &[$(Self::$name),+];
        }
    };
}

error_codes! {
    /// No method has that name, or a script asked for one only Neo may call.
    METHOD_UNAVAILABLE = "method_unavailable";
    /// The input did not decode into the method's input type.
    INVALID_INPUT = "invalid_input";
    /// The task was closed, forgotten, or belongs to a previous Neo process.
    TASK_NOT_FOUND = "task_not_found";
    /// All task slots hold live tabs, so none can be evicted.
    TASK_LIMIT_REACHED = "task_limit_reached";
    /// The requested tab is not among this task's open pages.
    TASK_PAGE_MISSING = "task_page_missing";
    /// The script returned while this op still ran.
    NOT_AWAITED = "not_awaited";
    /// The caller went away, or a crate's own cancel fired.
    CANCELLED = "cancelled";
    /// The run's time budget ran out.
    RUN_TIMEOUT = "run_timeout";
    /// Neo is stopping.
    SHUTTING_DOWN = "shutting_down";
    /// The CDP link is down, or was replaced since the run began.
    BROWSER_DISCONNECTED = "browser_disconnected";
    /// The target tab is gone.
    PAGE_CLOSED = "page_closed";
    /// Task scope: the tab is not one of the run's task's tabs (another task's, no task's, or
    /// none at all, answered alike). Nothing was sent.
    PAGE_NOT_IN_TASK = "page_not_in_task";
    /// A bug in Neo.
    INTERNAL_ERROR = "internal_error";
    /// A CDP call got no answer in time.
    CDP_TIMEOUT = "cdp_timeout";
    /// The browser answered a CDP call with an error.
    CDP_ERROR = "cdp_error";
    /// The owner holds the tab (sdk-traces' takeover lock).
    OWNER_HAS_CONTROL = "owner_has_control";
    /// A script's own picture (`page.screenshot`, `locator.screenshot`, `page.pdf`) of a tab
    /// that took a host's private fill (protected-fill ruling R46).
    PRIVATE_TAB = "private_tab";
    /// The script's code did not compile.
    SYNTAX_ERROR = "syntax_error";
    /// The script threw something other than an operation's `NeoError`.
    SCRIPT_ERROR = "script_error";
    /// One stretch of the script's JavaScript ran past the slice budget without awaiting.
    LONG_TASK = "long_task";
    /// The script filled its run's JavaScript heap.
    OUT_OF_MEMORY = "out_of_memory";
    /// The script's return value is larger than a run may return.
    RESULT_TOO_LARGE = "result_too_large";
    /// The host's `_meta["com.browseros.neo/run"]` is malformed.
    INVALID_META = "invalid_meta";
    /// This `requestId` already ran, and its replay window has passed.
    DUPLICATE_REQUEST = "duplicate_request";
    /// The trace store could not be read, so a `requestId` cannot be checked against it.
    TRACE_UNAVAILABLE = "trace_unavailable";
    /// The browser has not connected yet, so a run has no view to start with.
    BROWSER_UNAVAILABLE = "browser_unavailable";
    /// The page did not load: a network error, a refused connection, a dialog in the way, or
    /// no answer within core's navigation timeout.
    NAVIGATION_FAILED = "navigation_failed";
    /// A `neo.solveCaptcha` holds the page: another solve, or a write while one runs.
    PAGE_BUSY = "page_busy";
    /// This Neo has no CAPTCHA provider key, so `neo.solveCaptcha` is off. The deployment adds
    /// one; the script cannot.
    CAPTCHA_UNAVAILABLE = "captcha_unavailable";
    /// The host did not enable raw CDP in its launch file.
    CDP_UNAVAILABLE = "cdp_unavailable";

    // neo-playwright's codes. The crate's own error table picks each one's owner, whether
    // it is a timeout and whether Neo can vouch for what happened; builtins/playwright_ops.rs
    // carries them over unchanged.

    /// A strict locator matched two or more elements.
    STRICT_MODE_VIOLATION = "strict_mode_violation";
    /// Nothing matched for the op's whole timeout; also `expect` with no element.
    NO_MATCH = "no_match";
    /// An element matched, but stayed hidden, disabled, unstable, not editable or covered.
    NOT_ACTIONABLE = "not_actionable";
    /// An `expect` matcher failed with its element present.
    ASSERTION_FAILED = "assertion_failed";
    /// A ref or element handle whose element or document is gone. Never waits.
    ELEMENT_DETACHED = "element_detached";
    /// An element an action focused to type into did not keep the focus, so nothing was typed.
    /// Never waits.
    NOT_FOCUSED = "not_focused";
    /// A script the agent ran in the page threw (`evaluate`, `waitForFunction`).
    PAGE_ERROR = "page_error";
    /// The page never reached a state the op waited for: a load state, a URL, an event.
    WAIT_TIMEOUT = "wait_timeout";
    /// A download `download.saveAs` waited on was canceled or interrupted, or finished where Neo
    /// cannot read it.
    DOWNLOAD_FAILED = "download_failed";
    /// Neo did not finish observing a document already restored with all load states.
    NAVIGATION_OBSERVATION_TIMEOUT = "navigation_observation_timeout";
    /// A Playwright feature Neo does not support here, such as a popup opened as a window.
    UNSUPPORTED = "unsupported";
    /// The page or frame closed, or left the run's view, while a Playwright op worked on it.
    TARGET_CLOSED = "target_closed";
    /// The script aborted the op through Playwright's `signal` option.
    ABORTED = "aborted";

    // The host's private typing (`POST /v1/tabs/{tabId}/fill`, protected-fill design section
    // 4.4). The look refuses with the first eight before any key; the next four stop an entry.
    // Each message is a fixed sentence that quotes neither the page nor the value.
    // `element_detached`, `not_focused`, `owner_has_control`, `page_busy` and `run_timeout` keep
    // their meaning above.

    /// The named element is not a text input.
    NOT_AN_INPUT = "not_an_input";
    /// The box's frame is not on the origin the host expected.
    WRONG_ORIGIN = "wrong_origin";
    /// The box's type is not the type the host approved: the host's own input disagrees with
    /// itself, as when an agent swapped two refs.
    WRONG_TYPE = "wrong_type";
    /// A fact of the box the host approved (its type, autocomplete, maxlength...) differs now.
    BOX_CHANGED = "box_changed";
    /// The tab is no longer at the address the host sent.
    PAGE_CHANGED = "page_changed";
    /// The box is disabled or read-only.
    BOX_NOT_EDITABLE = "box_not_editable";
    /// The box has a size and does not show.
    BOX_HIDDEN = "box_hidden";
    /// Something else takes the pointer at the box: in its own document (main's covered rule),
    /// or in any page above its frame (ruling R60).
    BOX_COVERED = "box_covered";
    /// The box could not be brought into view (ruling R62), or a page above the box's frame
    /// could not be asked what takes the pointer at the box, cannot show the frame, or changes
    /// the frame's size or angle (ruling R60). Nothing was typed.
    COVER_UNREAD = "cover_unread";
    /// The value is longer than the box's `maxlength`, and the host did not accept a cut.
    VALUE_TOO_LONG = "value_too_long";
    /// The focus left the box after some characters went in.
    FOCUS_LOST = "focus_lost";
    /// The box holds something other than the value at the read-back.
    VALUE_NOT_TAKEN = "value_not_taken";
    /// A box with no size still held text after Neo tried to empty it before the first
    /// character; nothing of the value was typed.
    BOX_NOT_CLEARED = "box_not_cleared";
    /// A box entered under a cover (ruling R55): after the typing, the site drew the value
    /// outside the box. Neo emptied the box, and the page no longer shows the value.
    VALUE_DRAWN = "value_drawn";
    /// As `value_drawn`, but the page still shows the value after Neo emptied the box: reload or
    /// close the page before anything reads it.
    VALUE_STILL_DRAWN = "value_still_drawn";
    /// The page moved before Neo saw all but the last character in the box.
    PAGE_MOVED = "page_moved";
    /// A repeat of a fill's `requestId` whose first call Neo started and never finished: Neo
    /// restarted in the middle of it. The repeat types nothing.
    FILL_INTERRUPTED = "fill_interrupted";
    /// The lock a fill names stopped being held during the fill: the agent is no longer locked
    /// out as the host believed, so the fill stopped.
    LOCK_RELEASED = "lock_released";
}

impl ErrorCode {
    /// The code named `name`, if it is one of claw-sdk's: how a refusal that a crate hands back
    /// as a string keeps its code.
    #[must_use]
    pub(crate) fn known(name: &str) -> Option<Self> {
        Self::KNOWN.iter().copied().find(|code| code.0 == name)
    }

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

    /// The failure itself, for a crate that carries the door's refusal through its own error
    /// type (sdk-playwright's `Refusal`).
    #[must_use]
    pub(crate) fn error(&self) -> &OperationError {
        &self.error
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

impl From<TaskError> for OperationError {
    fn from(error: TaskError) -> Self {
        match &error {
            TaskError::NotFound(_) => Self::script(ErrorCode::TASK_NOT_FOUND, error),
            TaskError::Full => Self::neo(ErrorCode::TASK_LIMIT_REACHED, error),
            TaskError::InvalidId(_) => Self::neo(ErrorCode::INTERNAL_ERROR, error),
        }
    }
}
