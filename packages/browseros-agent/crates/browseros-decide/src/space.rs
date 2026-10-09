//! The candidate set, owned by code.
//!
//! Three documented properties of the model shape everything here. Its answer
//! is affected by the order of a Choice's options and it leans toward the first.
//! Its accuracy falls as irrelevant state grows, so fewer candidates is an
//! accuracy measure and not only a cost one. And a Choice should carry an
//! escape option whenever the list might not cover every input, which ours
//! provably might not.
//!
//! There is no model call in this module.

use serde_json::{Value, json};

use crate::relevance;
use crate::view::{Control, PageView};

/// How many candidates are offered for one operation.
///
/// Well under the protocol ceiling of 255, and chosen so the confidence
/// statistic can discriminate: confidence is measured against an even split, so
/// at 250 options an even split is 0.004 and the statistic collapses toward the
/// top probability alone.
pub const MAX_OFFERED: usize = 40;

/// Candidates with no name are kept a few slots, so a page whose controls are
/// all unnamed is not left with nothing to choose from.
pub const MIN_OFFERED_UNNAMED: usize = 4;

/// The option id that means "none of the listed controls will do".
///
/// Reserved, so no control can collide with it. Choosing it is never an
/// execution: code responds by exploring or handing back.
pub const NONE_OF_THESE: &str = "none_of_these";

/// How many control-and-value pairs a select question may offer in total.
///
/// Under the protocol's 255 with room for the escape option. Flattening a
/// select's values multiplies them, so this is a ceiling on the product rather
/// than on any one control.
pub const MAX_SELECT_OPTIONS: usize = 200;

/// One offered candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub reference: String,
    /// What travels to the model: an object carrying the control's current
    /// state, not a flattened label. The option values of a question accept
    /// structured JSON, so there is no reason to hide state in prose.
    pub descriptor: Value,
    /// The selectable values, for a control that has them.
    pub options: Vec<String>,
    pub role: String,
    pub guard: u64,
}

/// The candidates offered for one decision, and what was left out.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionSpace {
    pub offered: Vec<Candidate>,
    /// How many offerable controls did not fit. Non-zero is why the escape
    /// option exists.
    pub omitted: usize,
}

impl ActionSpace {
    /// Builds the offered set for a goal: filter, score, order, cap.
    #[must_use]
    pub fn build(view: &PageView, goal: &str, last_acted: Option<&str>) -> Self {
        let mut scored: Vec<(i32, &Control)> = view
            .controls
            .iter()
            .filter(|control| control.is_offerable())
            .map(|control| (relevance::score(goal, control, last_acted), control))
            .collect();

        // Best first. The sort is stable, so equal scores keep page order and the
        // same page always produces the same offered set.
        scored.sort_by_key(|(score, _)| -score);

        let total = scored.len();
        let named = scored
            .iter()
            .filter(|(_, control)| !control.name.is_empty())
            .count();
        let unnamed_slots = MIN_OFFERED_UNNAMED.min(total.saturating_sub(named));
        let named_slots = MAX_OFFERED.saturating_sub(unnamed_slots);

        let mut offered = Vec::new();
        let mut taken_named = 0;
        let mut taken_unnamed = 0;
        for (_, control) in scored {
            let is_named = !control.name.is_empty();
            if is_named && taken_named >= named_slots {
                continue;
            }
            if !is_named && taken_unnamed >= unnamed_slots {
                continue;
            }
            if is_named {
                taken_named += 1;
            } else {
                taken_unnamed += 1;
            }
            offered.push(candidate_of(control));
            if offered.len() >= MAX_OFFERED {
                break;
            }
        }

        Self {
            omitted: total.saturating_sub(offered.len()),
            offered,
        }
    }

    /// Every offerable candidate in page order, uncapped.
    ///
    /// Only used to measure what page order would have offered, which is the
    /// comparison the ordering decision rests on.
    #[must_use]
    pub fn build_unordered(view: &PageView) -> Vec<Candidate> {
        view.controls
            .iter()
            .filter(|control| control.is_offerable())
            .map(candidate_of)
            .collect()
    }

    /// The descriptors for the candidates one operation could act on.
    ///
    /// Narrowed per operation for two reasons, one measured and one obvious. A
    /// question's options are input tokens, and every target head repeats the
    /// list, so offering all forty to all of them multiplies the cost: measured
    /// at 7,696 input tokens for one step, against 4,700 once each head carries
    /// only what it can act on. And a question asking which control to tick
    /// should not offer a link, because that option cannot be right.
    #[must_use]
    pub fn options_for(&self, roles: &[&str]) -> std::collections::BTreeMap<String, Value> {
        let mut options: std::collections::BTreeMap<String, Value> = self
            .offered
            .iter()
            .filter(|candidate| roles.contains(&candidate.role.as_str()))
            .map(|candidate| (candidate.reference.clone(), candidate.descriptor.clone()))
            .collect();
        if options.is_empty() {
            return options;
        }
        options.insert(NONE_OF_THESE.to_string(), none_of_these());
        options
    }

    /// The descriptors, keyed by reference, as a Choice's options.
    ///
    /// The escape option is always present. The list provably may not cover
    /// what the goal needs, both because the cap drops candidates and because
    /// the control may simply not be on this page yet.
    #[must_use]
    pub fn choice_options(&self) -> std::collections::BTreeMap<String, Value> {
        let mut options: std::collections::BTreeMap<String, Value> = self
            .offered
            .iter()
            .map(|candidate| (candidate.reference.clone(), candidate.descriptor.clone()))
            .collect();
        options.insert(NONE_OF_THESE.to_string(), none_of_these());
        options
    }

    /// The candidates that offer a fixed set of values, flattened one option per
    /// control and value.
    ///
    /// Questions cannot read each other's answers, so a value question cannot
    /// depend on which control a target question chose. Flattening removes the
    /// dependency: each option names a control and one of its values.
    #[must_use]
    pub fn select_options(&self) -> std::collections::BTreeMap<String, Value> {
        let mut options = std::collections::BTreeMap::new();
        // Capped, because flattening multiplies: two dropdowns of 150 values
        // each would exceed the protocol's 255 options on their own and the
        // request would be refused before it was sent, taking the whole step
        // with it even though the goal may have needed a different control.
        //
        // Spread across the controls rather than filled from the first, so one
        // long dropdown cannot crowd out every other one. Candidates arrive
        // scored, so the share goes to the most relevant controls first.
        let with_values: Vec<&Candidate> = self
            .offered
            .iter()
            .filter(|candidate| !candidate.options.is_empty())
            .collect();
        if with_values.is_empty() {
            return options;
        }
        // Nothing is dropped when everything fits. Dividing the cap by the
        // number of controls dropped values for no reason: two dropdowns of 110
        // and 2 values gave each a share of 100, losing ten of the first even
        // though all 112 fit.
        let total: usize = with_values
            .iter()
            .map(|candidate| candidate.options.len())
            .sum();
        let share = if total <= MAX_SELECT_OPTIONS {
            usize::MAX
        } else {
            // Over the cap, every control keeps a share so one long dropdown
            // cannot crowd out the others, and the rest of the room is filled
            // in relevance order below.
            (MAX_SELECT_OPTIONS / with_values.len()).max(1)
        };
        for candidate in &with_values {
            for value in candidate.options.iter().take(share) {
                if options.len() >= MAX_SELECT_OPTIONS {
                    return options;
                }
                options.insert(
                    format!("{}::{value}", candidate.reference),
                    json!({
                        "control": candidate.descriptor.get("control"),
                        "sets_it_to": value,
                    }),
                );
            }
        }
        // Any room left goes to the most relevant controls first, since the
        // offered list arrives scored.
        for candidate in &with_values {
            for value in candidate
                .options
                .iter()
                .skip(share.min(candidate.options.len()))
            {
                if options.len() >= MAX_SELECT_OPTIONS {
                    return options;
                }
                options.insert(
                    format!("{}::{value}", candidate.reference),
                    json!({
                        "control": candidate.descriptor.get("control"),
                        "sets_it_to": value,
                    }),
                );
            }
        }
        options
    }

    /// Splits a flattened select option back into its control and value.
    #[must_use]
    pub fn split_select(option: &str) -> Option<(&str, &str)> {
        option.split_once("::")
    }

    /// The offered candidate with this reference.
    #[must_use]
    pub fn candidate(&self, reference: &str) -> Option<&Candidate> {
        self.offered
            .iter()
            .find(|candidate| candidate.reference == reference)
    }
}

fn none_of_these() -> Value {
    json!({
        "control": "none of the listed controls can advance the goal",
        "when_to_pick": "the control the goal needs is not listed, or nothing listed applies",
    })
}

/// Describes a control as an object, with its current state named.
///
/// A model that can see a checkbox is already ticked does not need a rule
/// telling it not to tick it again.
fn candidate_of(control: &Control) -> Candidate {
    let label = if control.name.is_empty() {
        control.role.clone()
    } else {
        format!("{} {:?}", control.role, control.name)
    };
    let mut descriptor = json!({ "control": label });
    if let Some(state) = control.state.describe() {
        descriptor["current"] = json!(state);
    }
    if let Some(value) = &control.value {
        descriptor["holds"] = json!(value);
    }
    if control.state.required {
        descriptor["required"] = json!(true);
    }
    if !control.options.is_empty() {
        descriptor["selectable"] = json!(control.options);
    }
    Candidate {
        reference: control.reference.clone(),
        descriptor,
        options: control.options.clone(),
        role: control.role.clone(),
        guard: control.guard,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MAX_CHOICE_OPTIONS;
    use crate::view::{Control, ControlState};

    fn control(reference: &str, role: &str, name: &str) -> Control {
        Control {
            reference: reference.to_string(),
            role: role.to_string(),
            name: name.to_string(),
            value: None,
            state: ControlState::default(),
            options: Vec::new(),
            guard: 0,
        }
    }

    fn view_with(controls: Vec<Control>) -> PageView {
        PageView {
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            text: String::new(),
            controls,
            fingerprint: 1,
        }
    }

    /// The escape option is always offered, because the list provably may not
    /// cover what the goal needs: the cap drops candidates, and the control may
    /// simply not be on this page yet.
    #[test]
    fn the_escape_option_is_always_offered() {
        let space = ActionSpace::build(
            &view_with(vec![control("e1", "button", "Apply")]),
            "x",
            None,
        );
        let options = space.choice_options();
        assert!(options.contains_key(NONE_OF_THESE));
        assert_eq!(options.len(), 2, "one control plus the escape option");

        let empty = ActionSpace::build(&view_with(Vec::new()), "x", None);
        assert!(
            empty.choice_options().contains_key(NONE_OF_THESE),
            "even with nothing to offer, the model can say so"
        );
    }

    /// Never more options than the protocol accepts, including the escape
    /// option, so the request cannot be rejected for its size.
    #[test]
    fn the_offered_set_stays_within_the_protocol_ceiling() {
        let many = (0..500)
            .map(|index| control(&format!("e{index}"), "link", &format!("Item {index}")))
            .collect();
        let space = ActionSpace::build(&view_with(many), "open item 7", None);
        assert_eq!(space.offered.len(), MAX_OFFERED);
        assert!(space.choice_options().len() <= MAX_CHOICE_OPTIONS);
        assert_eq!(space.omitted, 500 - MAX_OFFERED, "the rest are counted");
    }

    /// A goal-matching control outranks a navigation link, which is the whole
    /// reason ordering exists: the model leans toward the first option.
    #[test]
    fn a_goal_matching_control_is_offered_before_a_navigation_link() {
        let controls = vec![
            control("e1", "link", "Your Account"),
            control("e2", "link", "Customer Service"),
            control("e3", "link", "Gift Cards"),
            control("e4", "checkbox", "Corsair"),
        ];
        let space = ActionSpace::build(
            &view_with(controls),
            "Tick the Corsair brand filter checkbox",
            None,
        );
        assert_eq!(
            space.offered[0].reference, "e4",
            "the checkbox the goal names comes first, not the nav link that was first in the page"
        );
    }

    /// A control already in the requested state is worth less, because acting on
    /// it again undoes the work.
    #[test]
    fn a_control_already_in_the_requested_state_ranks_lower() {
        let mut ticked = control("e1", "checkbox", "Corsair");
        ticked.state.checked = Some(true);
        let unticked = control("e2", "checkbox", "Corsair");
        let space = ActionSpace::build(
            &view_with(vec![ticked, unticked]),
            "Tick the Corsair filter",
            None,
        );
        assert_eq!(
            space.offered[0].reference, "e2",
            "the one still needing a click comes first"
        );
    }

    /// A descriptor carries the control's current state, so the model can see a
    /// checkbox is already ticked rather than being told in prose not to tick it.
    #[test]
    fn a_descriptor_carries_the_controls_current_state() {
        let mut ticked = control("e1", "checkbox", "Corsair");
        ticked.state.checked = Some(true);
        let space = ActionSpace::build(&view_with(vec![ticked]), "x", None);
        let descriptor = &space.offered[0].descriptor;
        assert_eq!(descriptor["current"], serde_json::json!("checked"));
        assert_eq!(
            descriptor["control"],
            serde_json::json!("checkbox \"Corsair\"")
        );
    }

    /// A few unnamed controls keep their slots, so a page whose controls are all
    /// unnamed is not left with nothing to choose from.
    #[test]
    fn unnamed_controls_keep_a_floor_of_slots() {
        let mut controls: Vec<Control> = (0..MAX_OFFERED + 20)
            .map(|index| control(&format!("n{index}"), "link", &format!("Named {index}")))
            .collect();
        for index in 0..10 {
            let mut unnamed = control(&format!("u{index}"), "combobox", "");
            unnamed.options = vec!["one".to_string()];
            controls.push(unnamed);
        }
        let space = ActionSpace::build(&view_with(controls), "pick one", None);
        let unnamed = space
            .offered
            .iter()
            .filter(|candidate| candidate.reference.starts_with('u'))
            .count();
        assert_eq!(unnamed, MIN_OFFERED_UNNAMED);
        assert_eq!(space.offered.len(), MAX_OFFERED);
    }

    /// Flattening is how a value is chosen without a question depending on
    /// another question's answer: one option per control and value.
    #[test]
    fn a_selects_values_flatten_to_one_option_each() {
        let mut sort = control("e9", "combobox", "Sort by");
        sort.options = vec![
            "Featured".to_string(),
            "Price: Low to High".to_string(),
            "Newest".to_string(),
        ];
        let space = ActionSpace::build(&view_with(vec![sort]), "sort by price low to high", None);
        let options = space.select_options();
        assert_eq!(options.len(), 3);
        assert!(options.contains_key("e9::Price: Low to High"));

        let (reference, value) =
            ActionSpace::split_select("e9::Price: Low to High").expect("splits back");
        assert_eq!(reference, "e9");
        assert_eq!(value, "Price: Low to High");
    }

    /// Flattening multiplies, so a page with several long dropdowns could push
    /// one question past the protocol's 255 options and have the whole step
    /// refused before it was sent.
    #[test]
    fn a_select_question_stays_within_the_protocol_ceiling() {
        let mut controls = Vec::new();
        for index in 0..4 {
            let mut select = control(&format!("s{index}"), "combobox", &format!("Filter {index}"));
            select.options = (0..150).map(|value| format!("value {value}")).collect();
            controls.push(select);
        }
        let space = ActionSpace::build(&view_with(controls), "choose something", None);
        let options = space.select_options();
        assert!(
            options.len() <= MAX_SELECT_OPTIONS,
            "{} options offered, over the {MAX_SELECT_OPTIONS} cap",
            options.len()
        );
        assert!(options.len() < crate::client::MAX_CHOICE_OPTIONS);
    }

    /// One long dropdown must not crowd out the others, or a goal about the
    /// second one could not be answered at all.
    #[test]
    fn every_dropdown_gets_a_share_of_the_select_options() {
        let mut long = control("s1", "combobox", "Huge");
        long.options = (0..400).map(|value| format!("v{value}")).collect();
        let mut short = control("s2", "combobox", "Sort by");
        short.options = vec!["Price: Low to High".to_string()];
        let space = ActionSpace::build(&view_with(vec![long, short]), "sort by price", None);
        let options = space.select_options();
        assert!(
            options.keys().any(|key| key.starts_with("s2::")),
            "the short dropdown still appears: {:?}",
            options.keys().take(3).collect::<Vec<_>>()
        );
    }

    /// Nothing is dropped when everything fits. Dividing the cap by the number
    /// of controls lost values for no reason, and a goal naming one of them
    /// then could not be answered.
    #[test]
    fn every_value_is_offered_when_they_all_fit() {
        let mut long = control("s1", "combobox", "Capacity");
        long.options = (0..110).map(|value| format!("{value} GB")).collect();
        let mut short = control("s2", "combobox", "Sort by");
        short.options = vec!["Price".to_string(), "Newest".to_string()];
        let space = ActionSpace::build(&view_with(vec![long, short]), "pick a capacity", None);
        let options = space.select_options();
        assert_eq!(options.len(), 112, "all of them fit under the cap");
        assert!(options.contains_key("s1::109 GB"), "including the last one");
    }

    /// Over the cap, the room left after each control's share goes to the most
    /// relevant controls, so the cap is actually used rather than left short.
    #[test]
    fn the_cap_is_filled_when_the_values_do_not_fit() {
        let controls = (0..4)
            .map(|index| {
                let mut select =
                    control(&format!("s{index}"), "combobox", &format!("Filter {index}"));
                select.options = (0..150).map(|value| format!("v{value}")).collect();
                select
            })
            .collect();
        let space = ActionSpace::build(&view_with(controls), "choose something", None);
        assert_eq!(
            space.select_options().len(),
            MAX_SELECT_OPTIONS,
            "the cap is used, not undershot"
        );
    }

    /// A control with no fixed values contributes no select option, so nothing
    /// invents a closed set the page does not have.
    #[test]
    fn a_control_without_values_contributes_no_select_option() {
        let space = ActionSpace::build(
            &view_with(vec![control("e1", "button", "Apply")]),
            "x",
            None,
        );
        assert!(space.select_options().is_empty());
    }

    /// A disabled control never reaches the offered set.
    #[test]
    fn a_disabled_control_is_filtered_before_offering() {
        let mut disabled = control("e1", "button", "Apply");
        disabled.state.disabled = true;
        let space = ActionSpace::build(
            &view_with(vec![disabled, control("e2", "button", "Clear")]),
            "apply",
            None,
        );
        assert_eq!(space.offered.len(), 1);
        assert_eq!(space.offered[0].reference, "e2");
    }

    /// Ordering is stable for the same page, so a decision is reproducible and
    /// a comparison between two runs means something.
    #[test]
    fn ordering_is_stable_for_the_same_page() {
        let controls = vec![
            control("e1", "link", "Alpha"),
            control("e2", "checkbox", "Corsair"),
            control("e3", "button", "Apply"),
        ];
        let first = ActionSpace::build(&view_with(controls.clone()), "tick Corsair", None);
        let second = ActionSpace::build(&view_with(controls), "tick Corsair", None);
        assert_eq!(first.offered, second.offered);
    }
}
