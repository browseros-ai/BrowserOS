//! Scores a control against the goal, in code, with no model call.
//!
//! What this is actually for, measured rather than assumed.
//!
//! The documented reason to order options is that the model leans toward
//! whichever comes first. Measured against the live endpoint on a page with one
//! unambiguous match, that bias did not show: the right control was chosen six
//! times out of six whether it was offered first or last.
//!
//! The reason that did show is selection, not order. A listing page offers more
//! controls than fit in one question, so something has to choose which ones
//! travel. Taking them in page order drops the answer: on a 203 control page
//! with 40 slots, the brand filter the goal named was never offered, and the
//! model correctly answered "none of these" three times out of three. Scored
//! first, it was offered and chosen every time.
//!
//! So this earns its place by deciding what is visible, and the ordering is a
//! free side effect of scoring rather than the point.
//!
//! The signals are deliberately cheap and deterministic. A model call to rank
//! candidates for a model call would be circular, and the official pattern for
//! a large candidate set is to shortlist in code first.

use crate::view::Control;

/// Words too common to carry any signal about which control a goal means.
const NOISE: &[&str] = &[
    "the", "a", "an", "and", "or", "to", "of", "in", "on", "for", "with", "by", "at", "is", "it",
    "this", "that", "its", "so", "then", "from", "use", "using", "page", "site",
];

/// Splits text into comparable words, lowercased, with noise and very short
/// fragments dropped.
fn words(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.len() > 2)
        .map(str::to_lowercase)
        .filter(|word| !NOISE.contains(&word.as_str()))
        .collect()
}

/// Roles a goal's verb implies, so "tick the filter" favours a checkbox over a
/// link even when both mention the brand.
fn roles_implied_by(goal: &str) -> &'static [&'static str] {
    let goal = goal.to_lowercase();
    if ["tick", "check", "untick", "uncheck", "toggle"]
        .iter()
        .any(|verb| goal.contains(verb))
    {
        return &["checkbox", "switch", "radio", "menuitemcheckbox"];
    }
    if ["sort", "choose", "select", "pick"]
        .iter()
        .any(|verb| goal.contains(verb))
    {
        return &["combobox", "listbox", "option"];
    }
    if ["type", "enter", "fill", "search for", "write"]
        .iter()
        .any(|verb| goal.contains(verb))
    {
        return &["textbox", "searchbox", "combobox"];
    }
    if ["open", "click", "go to", "follow", "visit"]
        .iter()
        .any(|verb| goal.contains(verb))
    {
        return &["link", "button"];
    }
    &[]
}

/// How well this control matches the goal. Higher is better; zero is no signal.
///
/// The weights are not tuned against anything yet, which is why the live gate
/// for this phase compares the model's chosen target with and without ordering
/// rather than asserting the formula is right.
#[must_use]
pub fn score(goal: &str, control: &Control, last_acted: Option<&str>) -> i32 {
    let goal_words = words(goal);
    let name_words = words(&control.name);
    let mut score = 0;

    // A word the goal and the control's name share is the strongest cheap
    // signal that this is the control the goal is about.
    let mut shared = 0;
    for word in &name_words {
        if goal_words.contains(word) {
            shared += 1;
            score += 10;
        }
    }

    // A control whose whole name is what the goal named beats one that merely
    // mentions it. Measured on a real listing: a goal naming a brand put nine
    // product titles above the brand facet, because each title shared two or
    // three words with the goal while the facet shared only its one. The facet
    // is the control the goal meant.
    if shared > 0 && name_words.len() <= 2 {
        score += 12;
    }

    // A goal's value may name an option rather than the control, as "sort by
    // price" names the option and not the "Sort by" select.
    for option in &control.options {
        for word in words(option) {
            if goal_words.contains(&word) {
                score += 6;
            }
        }
    }

    if roles_implied_by(goal).contains(&control.role.as_str()) {
        score += 5;
    }

    // A control already in the state the goal asks for is worth less, not more:
    // acting on it again undoes the work.
    if control.state.checked == Some(true) || control.state.selected == Some(true) {
        score -= 4;
    }

    // Proximity to the last action, which is how multi-step work on one panel
    // stays on that panel.
    if last_acted.is_some_and(|reference| reference == control.reference) {
        score -= 2;
    }

    score
}
