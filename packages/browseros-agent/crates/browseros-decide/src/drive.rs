//! What the loop can ask the browser to do.
//!
//! One entry point for running a move rather than a method per tool, so adding
//! a way of acting is one match arm here and one option in the vocabulary.
//!
//! What this no longer carries is worth saying, because it was half the file.
//! The loop used to be able to read, grep, count, check a state and take a
//! picture, and hand what came back to the caller. Every one of those is a tool
//! the caller already has and can aim better: it knows the goal, it can pick a
//! selector, and it is not working to a byte budget. So the types here describe
//! acting on a page and nothing else, and no move produces a value.

use crate::gate::Decision;
use crate::offer::Destination;
use crate::view::PageView;
use crate::vocabulary::Operation;

/// Where a run may go, and whether it takes this page with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Journey {
    /// To a page the caller authored.
    To(Destination),
    /// Back to wherever this run came from.
    Back,
    /// Reload this same page.
    Again,
    /// To a page already open beside this one.
    Tab(u64),
}

/// What the loop asks the browser for.
///
/// Three things, where there were five. The two that went were a re-read, which
/// the loop does at the top of every iteration anyway, and a report, which was
/// the only move that produced something rather than doing something. Observing
/// the page is the caller's job now: it has the same tools, it knows the goal,
/// and it is not working to a byte budget.
#[derive(Debug, Clone, PartialEq)]
pub enum Move {
    /// Changes the page. The loop looks again and carries on.
    ///
    /// Boxed because a decision carries a judgement per argument and every
    /// other move here carries almost nothing, so holding one inline made the
    /// whole enum the size of its largest variant at every call site.
    Act(Box<Decision>),
    /// Changes which page the run is on.
    Travel(Journey),
    /// Waits for the page to settle.
    Settle,
}

/// What running a [`Move`] produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Performed {
    /// The page changed.
    Moved,
    /// It ran, and the page is identical.
    ///
    /// The loop retires whatever was aimed at. A click the page swallows reports
    /// success, so without this the next step chooses the same reference and the
    /// budget drains on one dead control. Measured live: the top ranked link for
    /// an errand sat inside an image caption whose click the page's own photo
    /// viewer intercepted, and the run spent every step it had on it.
    Unmoved,
}

/// Why a move could not be run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActError {
    /// The control is no longer on the page, or something covers it.
    ///
    /// Gone and covered are the same case on purpose: a retry cannot clear
    /// either, and treating "is covered by" as retryable is what let a run spend
    /// four attempts on one control.
    GoneOrCovered(String),
    /// The browser refused, for a reason worth reporting as it came.
    Refused(String),
    /// The browser could not be reached at all.
    Unreachable(String),
    /// The loop asked for something this browser does not offer.
    ///
    /// Not fatal. A peer without `grep` should get a working loop with `grep`
    /// left out of the vocabulary, rather than no loop at all.
    NotOffered(Operation),
}

impl std::fmt::Display for ActError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GoneOrCovered(what) => write!(formatter, "{what}"),
            Self::Refused(what) => write!(formatter, "{what}"),
            Self::Unreachable(what) => write!(formatter, "could not reach the browser: {what}"),
            Self::NotOffered(operation) => {
                write!(
                    formatter,
                    "this browser does not offer {}",
                    operation.as_str()
                )
            }
        }
    }
}

/// The browser, as one decision needs it.
///
/// One entry point for running a move rather than a method per tool, so adding
/// a tool is one match arm here and one option in the vocabulary, rather than a
/// new trait method and a new call site in the loop.
pub trait Driver {
    /// Whether the run has been cancelled.
    fn stopped(&self) -> bool;

    /// The page once it has stopped changing.
    ///
    /// Not "as it is now". A view read while the page is still settling costs
    /// more than a stale one: the loop compares fingerprints to decide whether
    /// a move did anything, so a read taken too early reports `Unmoved` for a
    /// control that worked, and the loop then retires a control that was fine.
    ///
    /// How long to wait, and what counts as stopped, belongs to the
    /// implementation. The promise is only that it waited.
    fn observe(&self) -> impl Future<Output = Result<PageView, String>>;

    /// Runs one move.
    ///
    /// Whether the page moved is not asked of the tool that moved it. The loop
    /// observes before and after and compares page identity itself, which is
    /// authoritative and needs no agreement with another component about how it
    /// words a result.
    fn perform(&self, act: &Move) -> impl Future<Output = Result<Performed, ActError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gone and covered are deliberately one case. Treating "is covered by" as
    /// retryable is what let a run spend four attempts on one control.
    #[test]
    fn a_covered_control_reads_the_same_as_a_missing_one() {
        let covered = ActError::GoneOrCovered("is covered by <div>".to_string());
        let gone = ActError::GoneOrCovered("no longer on the page".to_string());
        assert!(matches!(covered, ActError::GoneOrCovered(_)));
        assert!(matches!(gone, ActError::GoneOrCovered(_)));
    }

    #[test]
    fn a_missing_tool_says_which_one() {
        let refused = ActError::NotOffered(Operation::Navigate);
        assert!(refused.to_string().contains("NAVIGATE"));
    }
}
