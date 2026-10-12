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
    /// Values the caller has made available for typing, in the order they
    /// should be offered.
    ///
    /// The model cannot produce a string, but choosing which of a few given
    /// strings belongs in a field is a closed set, which is the one thing it
    /// does. So a caller that already holds the value never has to be asked
    /// for it.
    pub values: Vec<NamedValue>,
    /// Where the caller said this run may go. Empty means it stays put.
    pub destinations: Vec<Destination>,
    /// Destinations already used, by label.
    ///
    /// Not offered again: a destination that has been visited cannot advance
    /// the goal a second time, and a new-tab destination leaves this page
    /// unchanged, so without this the model could spend its whole budget
    /// opening the same tab.
    pub travelled: Vec<String>,
    /// How many times this run has moved.
    ///
    /// Going back is offered only above zero, so the loop can never reverse off
    /// the page the caller handed it.
    pub navigations: u32,
    /// Text the caller has supplied, by control reference.
    pub supplied: BTreeMap<String, Supplied>,
    /// What the most recent decision believed about whether the goal was
    /// already satisfied. Carried for the caller, read by nothing in the loop.
    pub goal_met: Option<f64>,
    /// How many times this run has asked the caller for a value.
    ///
    /// Bounded, because a field that will not hold still would otherwise ask
    /// forever: each ask costs the caller a round trip and takes no action, so
    /// nothing else in the loop would stop it.
    pub asks: u32,
    /// What the run is waiting for, when it parked.
    pub pending: Option<Pending>,
}

/// A value the caller has made available for typing into a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedValue {
    /// What it is, which is the hint the model judges by: `pincode`, `search
    /// term`, `quoted text`. Never shown to the page, only to the model.
    pub name: String,
    pub text: String,
}

impl NamedValue {
    #[must_use]
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            text: text.into(),
        }
    }
}

/// One page the caller said this run may move to.
///
/// Authored by the caller, never by the model: a url is open-ended text and
/// this model does not generate. Choosing between labels is a closed set, which
/// is the one shape it is good at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    /// What this page is, in the words the goal would use.
    pub label: String,
    pub url: String,
    /// Open it in a background tab in the caller's tab group and leave this
    /// page where it is.
    ///
    /// The caller's choice rather than the model's, so the decision about
    /// whether leaving the page is acceptable stays with the party that knows
    /// the wider intent.
    pub new_tab: bool,
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
        /// Other empty fields the decision could see, as reference and label.
        ///
        /// Carried so one round trip can answer a whole form. Filling one field
        /// does not change which field the next one is, so a caller that
        /// answers several at once saves a park for each of them, and a caller
        /// that answers only the one asked about loses nothing.
        others: Vec<(String, String)>,
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
            values: Vec::new(),
            destinations: Vec::new(),
            travelled: Vec::new(),
            navigations: 0,
            supplied: BTreeMap::new(),
            goal_met: None,
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

impl Run {
    /// The destinations still worth offering, with the key each is chosen by.
    ///
    /// Keyed by position rather than by label, because a label is caller text
    /// and an option id has to be something the model can return exactly.
    #[must_use]
    pub fn unvisited(&self) -> Vec<(String, &Destination)> {
        self.destinations
            .iter()
            .enumerate()
            .filter(|(_, destination)| !self.travelled.contains(&destination.label))
            .map(|(index, destination)| (format!("d{index}"), destination))
            .collect()
    }

    /// The destination behind an option id the model returned.
    #[must_use]
    pub fn destination(&self, key: &str) -> Option<&Destination> {
        let index = key.strip_prefix('d')?.parse::<usize>().ok()?;
        self.destinations.get(index)
    }
}

/// How many values one question may offer.
///
/// Small on purpose. Confidence is measured against an even split, and the
/// caller rarely has more than a handful of values that could belong in one
/// field, so a long list would weaken the statistic the gate reads without
/// covering any real case.
pub const MAX_VALUES: usize = 12;

impl Run {
    /// The values as a question's options, keyed so the sort order the wire
    /// imposes is the order they were given in.
    ///
    /// Zero padded because option order biases the answer and a plain `v10`
    /// would sort between `v1` and `v2`.
    #[must_use]
    pub fn value_options(&self) -> Vec<(String, &NamedValue)> {
        self.values
            .iter()
            .take(MAX_VALUES)
            .enumerate()
            .map(|(index, value)| (format!("v{index:02}"), value))
            .collect()
    }

    /// The value behind an option id the model returned.
    #[must_use]
    pub fn value(&self, key: &str) -> Option<&NamedValue> {
        let index = key.strip_prefix('v')?.parse::<usize>().ok()?;
        self.values.get(index)
    }
}

/// How many values a goal may contribute.
///
/// The caller's own values come first and these fill what is left, so this only
/// bounds the guessing.
pub const MAX_MINED_VALUES: usize = 6;

/// The shortest digit run worth offering as a value.
///
/// Three or fewer catches quantities, prices and ordinals that mostly belong in
/// prose rather than in a field, and a page with a quantity box is better
/// served by the caller naming it.
const MIN_DIGITS: usize = 4;

/// Values a goal names, for a caller that did not pass any.
///
/// The caller's intent usually contains the string already: "set the delivery
/// pincode to 600042" carries its own answer. Extraction is a lexical job with
/// a right answer, and literal scanning is one of this model's documented weak
/// spots, so code does the finding and the model only does the choosing.
///
/// Deliberately narrow. A goal with nothing that looks like a value yields
/// nothing, which leaves the run exactly as it was before this existed.
#[must_use]
pub fn values_in_goal(goal: &str) -> Vec<NamedValue> {
    let mut found: Vec<NamedValue> = Vec::new();
    let mut push = |name: &str, text: &str| {
        let text = text.trim();
        if text.is_empty()
            || found.len() >= MAX_MINED_VALUES
            || found.iter().any(|value| value.text == text)
        {
            return;
        }
        found.push(NamedValue::new(name, text));
    };

    for (open, close) in [('"', '"'), ('\'', '\'')] {
        let mut rest = goal;
        while let Some(start) = rest.find(open) {
            let after = &rest[start + open.len_utf8()..];
            let Some(end) = after.find(close) else { break };
            push(
                label_before(goal, &rest[start..]).unwrap_or("quoted"),
                &after[..end],
            );
            rest = &after[end + close.len_utf8()..];
        }
    }

    for word in goal.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '.')) {
        let bare = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-');
        if bare.is_empty() {
            continue;
        }
        let digits = bare.chars().filter(char::is_ascii_digit).count();
        let letters = bare.chars().filter(|c| c.is_alphabetic()).count();
        if letters == 0 && digits >= MIN_DIGITS {
            push(label_before(goal, bare).unwrap_or("number"), bare);
        } else if digits > 0 && letters > 0 && bare.contains('-') && bare.len() >= 5 {
            push("model number", bare);
        }
    }
    found
}

/// The word just before a value in the goal, which is usually what the value
/// is: "pincode 600042" names itself.
fn label_before<'a>(goal: &'a str, at: &str) -> Option<&'a str> {
    let offset = goal.find(at)?;
    let before = goal[..offset].trim_end().trim_end_matches([':', '=']);
    // Walked backwards rather than taken from one position, because the word
    // immediately before a value is usually a preposition: "set the postcode to
    // 600042" has its answer two words back.
    const EMPTY: &[&str] = &[
        "to", "the", "is", "a", "an", "of", "for", "into", "in", "with", "and", "at", "as", "be",
        "set", "under", "over", "below", "above", "this", "that",
    ];
    before
        .rsplit(|c: char| c.is_whitespace())
        .filter(|word| !word.is_empty())
        .take(3)
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric()))
        .find(|word| word.len() >= 2 && !EMPTY.contains(&word.to_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real goals from the session that prompted this, both of which
    /// carried their own answer and both of which parked anyway.
    #[test]
    fn the_goals_that_parked_carry_their_own_values() {
        let found = values_in_goal(
            "On Flipkart, search for \"Sony WH-1000XM5\" wireless headphones and open the \
             product page",
        );
        assert!(
            found.iter().any(|value| value.text == "Sony WH-1000XM5"),
            "{found:?}"
        );

        let found = values_in_goal(
            "On this Flipkart product page, set the delivery location to pincode 600042",
        );
        let pin = found
            .iter()
            .find(|value| value.text == "600042")
            .expect("the pincode");
        assert_eq!(pin.name, "pincode", "the goal names its own value");
    }

    /// A goal with nothing that looks like a value leaves the run exactly as it
    /// was before this existed.
    #[test]
    fn a_goal_with_no_values_yields_none() {
        assert!(
            values_in_goal(
                "sort these results by price, lowest first, and narrow them to the Corsair brand"
            )
            .is_empty()
        );
        assert!(values_in_goal("switch to the Cheapest tab").is_empty());
    }

    /// Short digit runs are quantities and ordinals that mostly belong in prose.
    /// A page with a quantity box is better served by the caller naming it.
    #[test]
    fn short_numbers_are_left_alone() {
        assert!(values_in_goal("set the quantity to 3").is_empty());
        assert!(values_in_goal("open page 2 of the results").is_empty());
    }

    #[test]
    fn a_model_number_is_recognised_without_quotes() {
        let found = values_in_goal("find the WH-1000XM5 headphones");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "WH-1000XM5");
        assert_eq!(found[0].name, "model number");
    }

    /// Words that say nothing about what a value is do not become its name.
    #[test]
    fn a_filler_word_is_not_used_as_a_name() {
        let found = values_in_goal("change the postcode to 600042");
        assert_eq!(found[0].name, "postcode", "not \"to\"");
    }

    #[test]
    fn the_same_value_is_not_offered_twice() {
        let found = values_in_goal("set pincode 600042 and confirm pincode 600042");
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn the_guessing_is_bounded() {
        let goal = "codes 1111 2222 3333 4444 5555 6666 7777 8888 9999";
        assert!(values_in_goal(goal).len() <= MAX_MINED_VALUES);
    }
}
