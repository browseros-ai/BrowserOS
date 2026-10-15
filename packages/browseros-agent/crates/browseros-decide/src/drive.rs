//! What the loop can ask the browser to do.
//!
//! The trait this replaces had four methods, and that was the ceiling on the
//! whole design. The browser offers nine tools; three of them were reachable as
//! decisions, `screenshot` and `tabs` were called by code without the model ever
//! choosing them, and `read`, `grep` and `evaluate` were not wired in at all.
//! Widening the vocabulary without widening this would have produced options
//! that could not be executed.
//!
//! The split between acting and reporting is in the types rather than in a
//! comment, because it is not a convention anyone should be free to break. A
//! decision model can neither carry text to a later step nor return it, so a
//! move that produces text has nowhere to continue to. [`Performed::Answer`]
//! leaves the loop no continue arm, and the compiler keeps it that way.

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
    /// Re-reads the page without changing it.
    Resnapshot,
    /// Waits for the page to settle.
    Settle,
    /// Produces something only the caller can use. There is nowhere to continue
    /// to, so the run ends here.
    Report(Ask),
}

/// A question about the page whose answer only the caller can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// Hand back the page's text.
    Read,
    /// Hand back the lines matching this.
    ///
    /// The pattern comes from code, derived from the goal's distinctive words,
    /// because the model cannot write one.
    Grep(String),
    /// Hand back how many things match this, counted in code.
    ///
    /// Counting is documented as unreliable in the model and the error grows
    /// with the size of the thing counted, so it is done here instead.
    Count(String),
    /// Hand back whether one control is in this state.
    State { reference: String, state: String },
    /// Hand back a picture, for meaning the accessibility tree does not carry.
    Picture,
    /// Hand back what a script computed.
    ///
    /// The script comes from code for the same reason the pattern does. Lowest
    /// value of the five, and the last worth wiring up.
    Evaluate(String),
}

impl Ask {
    /// A short word a caller can branch on.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Grep(_) => "grep",
            Self::Count(_) => "count",
            Self::State { .. } => "check_state",
            Self::Picture => "screenshot",
            Self::Evaluate(_) => "evaluate",
        }
    }
}

/// What the caller gets back from a report.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Answer {
    /// What was asked, so the caller knows how to read the rest.
    pub asked: String,
    /// The text, for everything but a picture.
    pub text: Option<String>,
    /// Whether a picture was taken and handed over separately.
    ///
    /// The image itself does not travel through this crate: it is a protocol
    /// content block, and keeping it out of here is what lets the whole decision
    /// layer stay testable without a browser.
    pub pictured: bool,
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
    /// Terminal. The caller gets this and the run is over.
    Answer(Answer),
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

    #[test]
    fn an_ask_names_itself_for_the_caller() {
        assert_eq!(Ask::Read.as_str(), "read");
        assert_eq!(Ask::Grep("rust".to_string()).as_str(), "grep");
        assert_eq!(
            Ask::State {
                reference: "e1".to_string(),
                state: "checked".to_string()
            }
            .as_str(),
            "check_state"
        );
    }

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
        let refused = ActError::NotOffered(Operation::Grep);
        assert!(refused.to_string().contains("GREP"));
    }
}
