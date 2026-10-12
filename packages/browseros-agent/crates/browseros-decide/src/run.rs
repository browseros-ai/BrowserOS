//! Everything one run carries, including between calls.
//!
//! This used to be a dozen locals inside `pursue`, which is why an ending could
//! only ever be reported and never continued: the trail, the counters and the
//! time already spent died with the call, so a caller that answered a question
//! started a new run that re-observed, re-ranked and re-decided from nothing.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::pursue::{Budget, Step};

/// One pursuit of one goal, which may span more than one call.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub goal: String,
    pub budget: Budget,
    /// Time already spent, totalled across every call that has advanced this
    /// run.
    ///
    /// An `Instant` cannot cross a call, so each call adds its own elapsed time
    /// to this and writes it back before returning. Without it a run that
    /// parked and resumed would get a fresh time allowance on every answer.
    pub spent: Duration,
    pub trail: Vec<Step>,
    pub recent: Vec<String>,
    pub decisions: u32,
    pub input_tokens: u64,
    // A counter each, because they are different situations with different
    // answers: three refusals to act is not three actions that did nothing, and
    // one shared budget meant whichever came first spent the other's.
    pub unchanged: u32,
    pub declined: u32,
    pub moved: u32,
    pub refused: u32,
    pub waits: u32,
    pub last_acted: Option<String>,
    /// Set once a control has refused a click, so the keyboard fallback is
    /// offered when it is actually needed rather than on every step.
    pub fallback_needed: bool,
    /// The url the run started on, which is the first call's url and not the
    /// one a later call happened to resume from.
    ///
    /// An option rather than an empty string, so a resumed run can tell "not
    /// observed yet" from "observed, and this is what it said".
    pub url_before: Option<String>,
    /// Text the caller has supplied, by control reference.
    pub supplied: BTreeMap<String, Supplied>,
    /// How many times this run has asked the caller for a value.
    ///
    /// Bounded, because a field that will not hold still would otherwise ask
    /// forever: each ask costs the caller a round trip and takes no action, so
    /// nothing else in the loop would stop it.
    pub asks: u32,
    /// What the run is waiting for, when it parked.
    pub pending: Option<Pending>,
}

/// Text the caller gave for one control, and what that control looked like
/// when the run asked for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Supplied {
    pub text: String,
    /// The control's guard at the moment the run asked.
    ///
    /// Checked before the text is typed. A reference is only a handle into a
    /// snapshot, so after the page moves the same `e18` can denote a different
    /// control, and typing the caller's text into it would be the one harm this
    /// whole exchange exists to avoid. `None` means the caller volunteered text
    /// for a control the run never asked about, so there is no baseline to
    /// check and none is enforced.
    pub guard: Option<u64>,
}

/// What a parked run is waiting for.
#[derive(Debug, Clone, PartialEq)]
pub enum Pending {
    /// A value for a field the model chose and the gate would otherwise have
    /// filled.
    Text {
        reference: String,
        /// The control as the caller would see it in a snapshot, in the same
        /// words the offered-controls list uses.
        control: String,
        /// How sure the run was that this is the right field. A low number is
        /// the caller's cue to look at the field before answering.
        confidence: f64,
        /// The control's guard at the moment of the ask, carried so the answer
        /// can be bound to the control that was asked about.
        guard: Option<u64>,
        asking: Asking,
    },
}

/// Why the run is asking.
///
/// A second ask for the same field is not the same event as the first, and a
/// caller that is told only "needs text" twice has no way to tell that its
/// answer was received and then discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asking {
    /// The run chose to type here and does not generate text.
    FirstTime,
    /// The caller answered, but the field changed before the text could be
    /// typed, so the answer was not used.
    FieldChanged,
}

impl Run {
    #[must_use]
    pub fn new(goal: impl Into<String>, budget: Budget) -> Self {
        Self {
            goal: goal.into(),
            budget,
            spent: Duration::ZERO,
            trail: Vec::new(),
            recent: Vec::new(),
            decisions: 0,
            input_tokens: 0,
            unchanged: 0,
            declined: 0,
            moved: 0,
            refused: 0,
            waits: 0,
            last_acted: None,
            fallback_needed: false,
            url_before: None,
            supplied: BTreeMap::new(),
            asks: 0,
            pending: None,
        }
    }

    /// The url this run started on, for a caller that wants a string.
    #[must_use]
    pub fn started_at(&self) -> String {
        self.url_before.clone().unwrap_or_default()
    }

    /// The text the caller supplied for one control, if the control is still
    /// the one the run asked about.
    ///
    /// `guard_now` is the control's guard on the page as it is at this moment.
    /// A mismatch returns `None`, which is the same answer as having no text at
    /// all, and the loop asks again rather than typing into something else.
    #[must_use]
    pub fn text_for(&self, reference: &str, guard_now: Option<u64>) -> Option<&str> {
        let supplied = self.supplied.get(reference)?;
        match supplied.guard {
            Some(asked) if Some(asked) != guard_now => None,
            _ => Some(supplied.text.as_str()),
        }
    }

    /// Whether the caller answered for this control but the control has since
    /// changed, which is a different thing to tell them than "I need text".
    #[must_use]
    pub fn answer_went_stale(&self, reference: &str, guard_now: Option<u64>) -> bool {
        self.supplied
            .get(reference)
            .and_then(|supplied| supplied.guard)
            .is_some_and(|asked| Some(asked) != guard_now)
    }

    /// Records text the caller supplied.
    ///
    /// Replaces an earlier answer for the same control, because the newer one
    /// is the caller's current intent. The guard comes from what the run was
    /// parked on, so an answer to a question the run asked is checked and an
    /// answer the caller volunteered is not.
    pub fn supply(&mut self, reference: impl Into<String>, text: impl Into<String>) {
        let reference = reference.into();
        let guard = match &self.pending {
            Some(Pending::Text {
                reference: asked,
                guard,
                ..
            }) if *asked == reference => *guard,
            _ => None,
        };
        self.supplied.insert(
            reference,
            Supplied {
                text: text.into(),
                guard,
            },
        );
    }

    /// Forgets an answer the loop could not use, so the next ask is a fresh
    /// question rather than the same stale one.
    pub fn forget(&mut self, reference: &str) {
        self.supplied.remove(reference);
    }
}
