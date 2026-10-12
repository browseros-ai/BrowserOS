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
    pub supplied: BTreeMap<String, String>,
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
        }
    }

    /// The url this run started on, for a caller that wants a string.
    #[must_use]
    pub fn started_at(&self) -> String {
        self.url_before.clone().unwrap_or_default()
    }

    /// The text the caller has supplied for one control, if any.
    #[must_use]
    pub fn text_for(&self, reference: &str) -> Option<&str> {
        self.supplied.get(reference).map(String::as_str)
    }

    /// Records text the caller supplied. Replaces an earlier answer for the
    /// same control, because the newer one is the caller's current intent.
    pub fn supply(&mut self, reference: impl Into<String>, text: impl Into<String>) {
        self.supplied.insert(reference.into(), text.into());
    }
}
