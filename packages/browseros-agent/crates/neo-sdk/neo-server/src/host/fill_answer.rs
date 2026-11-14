//! What a host fill answers, from how its one op ended (protected-fill design, sections 4.3,
//! 4.4 and 9).
//!
//! ```text
//!  host.fill's OpReply: the door's outcome, delivery, retryable and error
//!    + FillProgress: the entry's outcome, at, typed
//!    ─▶ api::FillResult ─▶ the HTTP answer, the run's result, the fill's request record
//! ```
//!
//! Delivery, `retryable` and the error are the door's, so the answer, the op's record and its
//! event cannot disagree (ruling R5); the entry adds only where it stopped and how many
//! characters went in. Nothing here holds text from the page or a value.

use neo_http::FillRefusal;
use neo_playwright::{Entry, EntryOutcome};
use neo_sdk_api as api;
use neo_trace::{Delivery, ErrorOwner, RunId};
use serde_json::Value;

use crate::dispatch::{ErrorCode, OpFailure};

/// A call refused before anything ran: a request error, not an answer about the page. Clone,
/// because a repeat that joined the first call gets the first call's refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// 400: the body does not decode or breaks a limit. The message is a fixed sentence.
    Invalid(String),
    /// 404: no open tab has the id.
    TabNotFound,
    /// 409 `lock_released`: the `lockId` named is not held on this tab.
    LockReleased,
    /// 503: the browser has not linked yet.
    BrowserUnavailable,
    /// 503: the fill's request record cannot be written, so Neo cannot promise one typing.
    RecordUnavailable,
    /// 503: the record of the tabs that took a fill cannot be written (ruling R45), so a Neo
    /// restart could picture this tab again: any fill is refused, with a `requestId` or not.
    FilledTabsUnavailable,
    /// 500: a failure inside Neo, with its cause for the log.
    Internal(String),
}

impl From<Refused> for FillRefusal {
    fn from(refused: Refused) -> Self {
        match refused {
            Refused::Invalid(message) => Self::Invalid(message),
            Refused::TabNotFound => Self::TabNotFound,
            Refused::LockReleased => Self::LockReleased(
                "the lock this fill names is not held on this tab, so the agent is not locked \
                 out; nothing was typed"
                    .to_owned(),
            ),
            Refused::BrowserUnavailable => Self::BrowserUnavailable(
                "the browser is not connected yet; retry in a few seconds".to_owned(),
            ),
            Refused::RecordUnavailable => Self::TraceUnavailable(
                "Neo cannot write the fill's request record, so it cannot promise this \
                 requestId types once; nothing was typed"
                    .to_owned(),
            ),
            Refused::FilledTabsUnavailable => Self::TraceUnavailable(
                "Neo cannot write its record of the tabs that took a fill, so it types nothing"
                    .to_owned(),
            ),
            Refused::Internal(cause) => Self::Internal(cause),
        }
    }
}

/// Ruling R64: the fields typed after a look that found their limit moved, or absent; never an
/// empty list.
fn moved(entry: &Entry) -> Option<Vec<i32>> {
    (!entry.max_length_moved.is_empty()).then(|| counts(&entry.max_length_moved))
}

/// The fill's answer from its op's end. `entry` is what the entry answered; `None` when the door
/// refused the op before the entry ran (a lock, a CAPTCHA solve, a closed tab) or cut it short.
#[must_use]
pub(crate) fn fill_answer(
    run: RunId,
    outcome: &Result<Value, OpFailure>,
    entry: Option<Entry>,
    fields: usize,
) -> api::FillResult {
    let run = run.to_string();
    match (outcome, entry) {
        (Ok(_), Some(entry)) => api::FillResult {
            at: entry.at.map(wire_count),
            typed: Some(counts(&entry.typed)),
            max_length_moved: moved(&entry),
            ..api::FillResult::new(
                run,
                entry.outcome.as_str().to_owned(),
                delivery(Delivery::Done),
            )
        },
        (Err(failure), Some(entry)) => {
            if entry.delivery != failure.delivery {
                // The seam promises they agree (ruling R5); the door's is the one recorded.
                tracing::warn!(entry = ?entry.delivery, door = ?failure.delivery, "the entry's delivery differs from the door's");
            }
            api::FillResult {
                at: entry.at.map(wire_count),
                typed: Some(counts(&entry.typed)),
                max_length_moved: moved(&entry),
                error: Some(Box::new(wire_error(failure))),
                ..api::FillResult::new(
                    run,
                    entry.outcome.as_str().to_owned(),
                    delivery(failure.delivery),
                )
            }
        }
        // The door refused before the entry ran: nothing went out, at the first box. Cut short
        // after the op was sent: its counts and its place went with the entry, so the answer
        // says neither.
        (Err(failure), None) => {
            let sent = failure.delivery != Delivery::NotSent;
            let outcome = if sent {
                EntryOutcome::Uncertain
            } else {
                EntryOutcome::Refused
            };
            api::FillResult {
                at: (!sent).then_some(0),
                typed: (!sent).then(|| vec![0; fields]),
                error: Some(Box::new(wire_error(failure))),
                ..api::FillResult::new(run, outcome.as_str().to_owned(), delivery(failure.delivery))
            }
        }
        // A method that returned always left its entry first; if not, say nothing certain.
        (Ok(_), None) => api::FillResult {
            error: Some(Box::new(neo_error(
                ErrorCode::INTERNAL_ERROR,
                "the fill ended without its entry's answer; look at the page before trying again",
            ))),
            ..api::FillResult::new(
                run,
                EntryOutcome::Uncertain.as_str().to_owned(),
                delivery(Delivery::Unknown),
            )
        },
    }
}

/// The answer to a repeat of a `requestId` whose first call Neo started and never finished: Neo
/// restarted in the middle of it (design section 4.6). This call types nothing.
#[must_use]
pub(crate) fn interrupted(first: RunId) -> api::FillResult {
    api::FillResult {
        error: Some(Box::new(neo_error(
            ErrorCode::FILL_INTERRUPTED,
            "Neo restarted while the first call with this requestId was typing, so it cannot say \
             what reached the page; this call typed nothing; look at the page before trying again",
        ))),
        ..api::FillResult::new(
            first.to_string(),
            EntryOutcome::Uncertain.as_str().to_owned(),
            delivery(Delivery::Unknown),
        )
    }
}

/// The answer to the first call when the fill's own task failed inside Neo (it panicked): what
/// reached the page is unknown, so the host looks before trying again. A repeat of its
/// `requestId` answers `fill_interrupted` (the ledger's ticket marked it), and types nothing.
#[must_use]
pub(crate) fn crashed(run: RunId) -> api::FillResult {
    api::FillResult {
        error: Some(Box::new(neo_error(
            ErrorCode::INTERNAL_ERROR,
            "the fill failed inside Neo, so it cannot say what reached the page; look at the page \
             before trying again",
        ))),
        ..api::FillResult::new(
            run.to_string(),
            EntryOutcome::Uncertain.as_str().to_owned(),
            delivery(Delivery::Unknown),
        )
    }
}

/// The door's failure in the contract's `OperationError`.
fn wire_error(failure: &OpFailure) -> api::OperationError {
    api::OperationError {
        owner: owner_name(failure.error.owner()),
        code: failure.error.code().as_str().to_owned(),
        message: failure.error.message().to_owned(),
        retryable: failure.retryable,
        host_code: None,
        operation_id: Some(failure.operation.to_string()),
    }
}

fn neo_error(code: ErrorCode, message: &str) -> api::OperationError {
    api::OperationError {
        owner: owner_name(ErrorOwner::Neo),
        code: code.as_str().to_owned(),
        message: message.to_owned(),
        retryable: false,
        host_code: None,
        operation_id: None,
    }
}

/// The wire name of an owner.
pub(crate) fn owner_name(owner: ErrorOwner) -> String {
    owner.as_str().to_owned()
}

fn delivery(delivery: Delivery) -> String {
    match delivery {
        Delivery::NotSent => "not_sent",
        Delivery::Done => "done",
        Delivery::Unknown => "unknown",
    }
    .to_owned()
}

fn counts(typed: &[usize]) -> Vec<i32> {
    typed.iter().copied().map(wire_count).collect()
}

/// A count on the wire: a box takes at most 256 characters, a form 16 boxes.
fn wire_count(count: usize) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}
