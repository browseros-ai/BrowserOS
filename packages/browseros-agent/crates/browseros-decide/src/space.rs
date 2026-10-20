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

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::kind::{self, Kind};
use crate::relevance;
use crate::view::{Control, PageView};

/// How many candidates one question may offer.
///
/// The protocol ceiling is 255 options and the escape option takes one of them,
/// so 254 is the real ceiling. Asking for 255 is rejected with "choice target
/// has 256 options".
///
/// This used to be 40, on the argument that confidence is measured against an
/// even split and collapses when the split is tiny. The argument is sound about
/// the share and irrelevant to the decision, because the gate reads the margin
/// between the top two options, which does not care how many options sit below
/// them. What 40 actually did was hide the answer: the same target question on
/// one measured page scored 0.42 with the right control cut from the list and
/// 0.90 with the whole page offered.
pub const MAX_OFFERED: usize = 254;

/// How many of each kind the goal names are pulled into the first batch.
///
/// Promoting rather than reserving. Reserving slots was the right answer under
/// a cap, where lifting a kind above the tiebreak pushed something else off the
/// end and rescued one errand by losing another. Nothing falls off the end any
/// more: what does not fit in this batch is offered in the next one, so moving
/// a kind forward costs the thing behind it a batch and not its existence.
pub const PROMOTED_PER_KIND: usize = 3;

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
    /// The page gives this a pointer cursor, so it belongs in the click head
    /// whatever its role says.
    pub clickable: bool,
    /// What travels to the model: an object carrying the control's current
    /// state, not a flattened label. The option values of a question accept
    /// structured JSON, so there is no reason to hide state in prose.
    pub descriptor: Value,
    /// The selectable values, for a control that has them.
    pub options: Vec<String>,
    pub role: String,
    /// What this control is for, decided in code. Carried so the reservation
    /// pass and the question builder do not each work it out again.
    pub kind: Kind,
    pub guard: u64,
}

/// The candidates offered for one decision, and what was left out.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ActionSpace {
    pub offered: Vec<Candidate>,
    /// How many offerable controls are not in this batch. Non-zero is why the
    /// escape option exists, and why there may be another batch to ask about.
    pub omitted: usize,
    /// Which batch of the page this is, counting from zero.
    ///
    /// Carried because the question builder has to put it in the state. A later
    /// batch asked with the first batch's state reads as a question about a
    /// page position the run is not at, which cost 0.33 of confidence when it
    /// was measured: 0.57 against 0.90 for the same control.
    pub batch: usize,
}

impl ActionSpace {
    /// Every offerable candidate for a goal, best first.
    ///
    /// `retired` names controls that were acted on and changed nothing. They are
    /// dropped before scoring, because a retry cannot clear whatever swallowed
    /// the first attempt, and leaving them in means the run spends its whole
    /// budget on one dead reference.
    ///
    /// Nothing is capped here. The cap belongs to a batch, and a batch is one
    /// question; the page as a whole is reachable across as many questions as it
    /// takes.
    #[must_use]
    /// `insist` names kinds to pull forward regardless of what the goal says.
    /// The goal's own words are not the only reason a kind matters: a run that
    /// has just been told nothing listed can advance it has a reason to look at
    /// this page's search field that the goal never mentioned.
    pub fn ordered(
        view: &PageView,
        goal: &str,
        last_acted: Option<&str>,
        retired: &[String],
        insist: &[Kind],
    ) -> Vec<Candidate> {
        let mut scored: Vec<(i32, &Control)> = view
            .controls
            .iter()
            .filter(|control| control.is_offerable())
            .filter(|control| !retired.iter().any(|dead| dead == &control.reference))
            .map(|control| (relevance::score(goal, control, last_acted), control))
            .collect();

        // Best first. The sort is stable, so equal scores keep page order and
        // the same page always produces the same order, which is what makes a
        // batch index mean anything.
        scored.sort_by_key(|(score, _)| -score);

        let mut implied: Vec<Kind> = insist.to_vec();
        for wanted in kind::implied_by(goal) {
            if !implied.contains(&wanted) {
                implied.push(wanted);
            }
        }
        let mut ordered: Vec<Candidate> = Vec::new();
        // Which of a row of identically named controls is the one this goal
        // means. Only one of them can be offered, so it had better be the right
        // one.
        let speaks_for = speaks_for_its_description(view, goal);

        // One place that decides whether a candidate can still be added, so the
        // two passes below cannot drift apart on the rules.
        let add = |ordered: &mut Vec<Candidate>, control: &Control| -> bool {
            // A control whose name it shares with others on the page is offered
            // only as the row the goal names. Which one survived used to be
            // whichever the ordering reached first, and on a commercial results
            // page that is the sponsored row at the top: asked for Sensodyne,
            // three runs out of three added a Colgate and reported the goal met.
            let candidate = candidate_of(control);
            if let Some(winner) = speaks_for.get(&candidate.descriptor.to_string())
                && *winner != control.reference
            {
                return false;
            }
            // A duplicate is worse than an omission. Two options the model
            // cannot tell apart divide the probability between them, so neither
            // leads the other and the margin the gate reads is zero. Describing
            // them by their row instead was measured and was worse: it grew one
            // page from 363 candidates to 499, cost twice the tokens, and the
            // margins fell from 0.19 to 0.25 down to 0.01 to 0.19, because rows
            // that differ only by a price are no easier to tell apart than
            // names that do not differ at all.
            if ordered.iter().any(|kept| {
                kept.reference == candidate.reference || kept.descriptor == candidate.descriptor
            }) {
                return false;
            }
            ordered.push(candidate);
            true
        };

        // Pass one: pull a few of each kind the goal names to the front, so they
        // land in the first batch rather than wherever relevance put them.
        for wanted in &implied {
            let mut promoted = 0usize;
            for (_, control) in &scored {
                if promoted >= PROMOTED_PER_KIND {
                    break;
                }
                if kind::of(control) != *wanted {
                    continue;
                }
                if add(&mut ordered, control) {
                    promoted += 1;
                }
            }
        }

        // Pass two: everything else, in relevance order.
        for (_, control) in &scored {
            add(&mut ordered, control);
        }

        ordered
    }

    /// One batch of an ordered candidate list.
    ///
    /// `batch` counts from zero. A batch past the end is empty rather than an
    /// error, so a loop can ask for the next one without first checking whether
    /// there is one.
    #[must_use]
    pub fn batch(ordered: &[Candidate], batch: usize) -> Self {
        let from = batch.saturating_mul(MAX_OFFERED);
        let offered: Vec<Candidate> = ordered
            .iter()
            .skip(from)
            .take(MAX_OFFERED)
            .cloned()
            .collect();
        Self {
            omitted: ordered.len().saturating_sub(from + offered.len()),
            offered,
            batch,
        }
    }

    /// How many batches an ordered list takes.
    #[must_use]
    pub fn batches(ordered: &[Candidate]) -> usize {
        ordered.len().div_ceil(MAX_OFFERED)
    }

    /// The first batch for a goal, which is what a fresh look at a page asks
    /// about.
    #[must_use]
    pub fn build(
        view: &PageView,
        goal: &str,
        last_acted: Option<&str>,
        retired: &[String],
    ) -> Self {
        Self::batch(&Self::ordered(view, goal, last_acted, retired, &[]), 0)
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
        // A pointer-marked control belongs in the click head whatever its role
        // is. Narrowing by role alone excluded exactly the controls the naming
        // work had just made usable: a brand facet reached the offered set,
        // scored third, and was then kept out of the only question that could
        // have chosen it, so the model correctly answered that none of the
        // product links it was shown would narrow by brand.
        let clickable_head = roles.contains(&"button") && roles.contains(&"link");
        let mut options: std::collections::BTreeMap<String, Value> = self
            .offered
            .iter()
            .filter(|candidate| {
                roles.contains(&candidate.role.as_str()) || (clickable_head && candidate.clickable)
            })
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
/// Roles a model can act on by name. Anything else that is clickable is
/// described functionally instead.
const MEANINGFUL_ROLES: &[&str] = &[
    "button",
    "link",
    "textbox",
    "searchbox",
    "combobox",
    "listbox",
    "checkbox",
    "radio",
    "switch",
    "slider",
    "spinbutton",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "tab",
    "treeitem",
];

/// For each control description shared by more than one control, which one the
/// goal means.
///
/// A listing page calls every one of its buttons "Add to cart", and only one of
/// them can be offered, because options the model cannot tell apart split the
/// answer between them. The question is therefore not whether to drop the
/// duplicates but which one to keep, and page order is the wrong answer: the top
/// row of a commercial results page is an advertisement for something else.
///
/// So each duplicate is scored by the row it sits in, which is the product title
/// nearest above it in page order, against the words of the goal. Keyed on the
/// whole description rather than the name, because two controls can share a name
/// and still be different options: a ticked and an unticked filter both called
/// "Sony" are two answers, not one.
///
/// Nothing here calls the model. A model call to choose what to ask the model
/// about would be circular.
fn speaks_for_its_description(view: &PageView, goal: &str) -> HashMap<String, String> {
    let mut shared: HashMap<String, usize> = HashMap::new();
    for control in &view.controls {
        if control.is_offerable() {
            *shared
                .entry(candidate_of(control).descriptor.to_string())
                .or_default() += 1;
        }
    }
    shared.retain(|_, count| *count > 1);
    if shared.is_empty() {
        return HashMap::new();
    }

    let wanted = relevance::words(goal);
    let mut best: HashMap<String, (usize, String)> = HashMap::new();
    let mut row: Vec<String> = Vec::new();
    for control in &view.controls {
        if control.name.chars().count() > ROW_WORTH_KEEPING {
            row = relevance::words(&control.name);
        }
        let described = candidate_of(control).descriptor.to_string();
        if !shared.contains_key(&described) {
            continue;
        }
        let overlap = row
            .iter()
            .filter(|word| wanted.iter().any(|want| want == *word))
            .count();
        let entry = best
            .entry(described)
            .or_insert((0, control.reference.clone()));
        // Strictly greater, so a tie keeps the earlier row and the ordering
        // stays the page's own.
        if overlap > entry.0 {
            *entry = (overlap, control.reference.clone());
        }
    }
    best.into_iter()
        .map(|(described, (_, reference))| (described, reference))
        .collect()
}

/// A name long enough to be a product title rather than a label or a price.
const ROW_WORTH_KEEPING: usize = 18;

/// How one control is described to the model, as the option space describes it.
///
/// Public so a question asked about a control already chosen names it the same
/// way the question that chose it did.
#[must_use]
pub fn describe(control: &Control) -> serde_json::Value {
    candidate_of(control).descriptor
}

fn candidate_of(control: &Control) -> Candidate {
    // A role the accessibility tree invented is not a description. A facet
    // rendered as a styled label arrives as `LabelText`, which tells a model
    // nothing about what clicking it would do, and a model asked which control
    // filters by brand answered that none of them would: the one that would was
    // described to it as "LabelText". So a clickable control whose role carries
    // no meaning is described by what it is instead.
    let role = if MEANINGFUL_ROLES.contains(&control.role.as_str()) {
        control.role.clone()
    } else if control.clickable {
        "clickable filter or option".to_string()
    } else {
        control.role.clone()
    };
    let label = if control.name.is_empty() {
        role
    } else {
        format!("{role} {:?}", control.name)
    };
    let mut descriptor = json!({ "control": label });
    // What tells one of sixty-five identical buttons from another. Withheld
    // when it only repeats the control's own name, which describes nothing.
    if let Some(group) = control.group.as_deref()
        && !group.is_empty()
        && !group.eq_ignore_ascii_case(control.name.trim())
    {
        descriptor["inside"] = json!(group);
    }
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
        clickable: control.clickable,
        descriptor,
        options: control.options.clone(),
        role: control.role.clone(),
        kind: kind::of(control),
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
            clickable: false,
            role: role.to_string(),
            name: name.to_string(),
            value: None,
            state: ControlState::default(),
            options: Vec::new(),
            guard: 0,
            group: None,
        }
    }

    /// The short-name bonus breaks ties and must not overturn a control that
    /// matched the goal more times. Measured on a live results page: a goal
    /// naming page 2 and the pagination ranked `link "Results"` above
    /// `button "Go to next page, page 2"`, which then fell outside the offered
    /// forty, and the run scrolled three times and stalled with the control it
    /// needed never shown to it.
    #[test]
    fn more_matches_beats_a_shorter_name() {
        let goal = "go to page 2 of the search results using the pagination at the bottom";
        let pagination = control("e215", "button", "Go to next page, page 2");
        let incidental = control("e2", "link", "Results");
        let (strong, weak) = (
            crate::relevance::score(goal, &pagination, None),
            crate::relevance::score(goal, &incidental, None),
        );
        assert!(
            strong > weak,
            "the control the goal named scored {strong} and an incidental match scored {weak}"
        );
    }

    /// And the bonus still does the job it was added for: a facet whose whole
    /// name is what the goal named beats a verbose product title that merely
    /// mentions it.
    #[test]
    fn a_short_exact_name_still_beats_a_verbose_title() {
        let goal = "Narrow this RAM listing to the ADATA brand using the brand filter";
        let facet = control("e32", "LabelText", "ADATA");
        let product = control(
            "e99",
            "link",
            "ADATA Laptop DDR5 8 GB (Single Channel) Laptop (AD5S56008G-S)",
        );
        let (strong, weak) = (
            crate::relevance::score(goal, &facet, None),
            crate::relevance::score(goal, &product, None),
        );
        assert!(
            strong > weak,
            "the facet scored {strong} and the product title scored {weak}"
        );
    }

    fn view_with(controls: Vec<Control>) -> PageView {
        PageView {
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            text: String::new(),
            controls,
            fingerprint: 1,
            arrangement: 1,
            unlabelled_images: 0,
        }
    }

    /// Measured on a live Flipkart search page: four listings whose names,
    /// roles and states were byte identical, which split the confidence between
    /// them so none could clear its bar. The run refused to open any, and the
    /// caller had already said the colour variants were equivalent.
    #[test]
    fn options_the_model_cannot_tell_apart_are_offered_once() {
        let view = view_with(vec![
            control("e1", "link", "SONY WH-1000XM5 Wireless Noise Cancellation"),
            control("e2", "link", "SONY WH-1000XM5 Wireless Noise Cancellation"),
            control("e3", "link", "SONY WH-1000XM5 Wireless Noise Cancellation"),
            control("e4", "link", "SONY WF-1000XM5 Truly Wireless Earbuds"),
        ]);
        let space = ActionSpace::build(&view, "open the first WH-1000XM5 listing", None, &[]);

        assert_eq!(space.offered.len(), 2, "three became one, plus the earbuds");
        assert_eq!(
            space.offered[0].reference, "e1",
            "the first survivor is page order, which is what \"the first one\" means"
        );
        assert_eq!(
            space.omitted, 0,
            "a collapsed duplicate is not an omission: its twin is right there, \
             and omitted now means how many controls are waiting in a later batch"
        );
    }

    /// Controls that merely share a name are still distinguishable by their
    /// state, so they are not duplicates and both are offered.
    #[test]
    fn controls_that_differ_in_state_are_both_offered() {
        let mut ticked = control("e1", "checkbox", "Sony");
        ticked.state.checked = Some(true);
        let mut unticked = control("e2", "checkbox", "Sony");
        unticked.state.checked = Some(false);
        let view = view_with(vec![ticked, unticked]);

        let space = ActionSpace::build(&view, "tick Sony", None, &[]);
        assert_eq!(space.offered.len(), 2);
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
            &[],
        );
        let options = space.choice_options();
        assert!(options.contains_key(NONE_OF_THESE));
        assert_eq!(options.len(), 2, "one control plus the escape option");

        let empty = ActionSpace::build(&view_with(Vec::new()), "x", None, &[]);
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
        let space = ActionSpace::build(&view_with(many), "open item 7", None, &[]);
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
            &[],
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
            &[],
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
        let space = ActionSpace::build(&view_with(vec![ticked]), "x", None, &[]);
        let descriptor = &space.offered[0].descriptor;
        assert_eq!(descriptor["current"], serde_json::json!("checked"));
        assert_eq!(
            descriptor["control"],
            serde_json::json!("checkbox \"Corsair\"")
        );
    }

    /// The cap used to crowd unnamed controls out, so they were given a floor of
    /// reserved slots. Nothing is crowded out any more: a control that does not
    /// fit in this batch is offered in the next one.
    #[test]
    fn a_control_past_the_cap_is_reachable_in_a_later_batch() {
        let mut controls: Vec<Control> = (0..MAX_OFFERED + 20)
            .map(|index| control(&format!("n{index}"), "link", &format!("Named {index}")))
            .collect();
        for index in 0..10 {
            let mut unnamed = control(&format!("u{index}"), "combobox", "");
            // Distinct values, because real unnamed controls are distinct and
            // byte-identical ones would be collapsed by the duplicate rule.
            unnamed.options = vec![format!("option {index}")];
            controls.push(unnamed);
        }
        let view = view_with(controls);
        let ordered = ActionSpace::ordered(&view, "pick one", None, &[], &[]);
        assert_eq!(ActionSpace::batches(&ordered), 2);

        let first = ActionSpace::batch(&ordered, 0);
        assert_eq!(first.offered.len(), MAX_OFFERED);
        assert_eq!(first.batch, 0);
        assert_eq!(first.omitted, 30, "the rest are in the next batch");

        let second = ActionSpace::batch(&ordered, 1);
        assert_eq!(second.offered.len(), 30);
        assert_eq!(second.batch, 1);
        assert_eq!(second.omitted, 0, "nothing is left after the last batch");

        let reachable: Vec<&str> = ordered
            .iter()
            .map(|candidate| candidate.reference.as_str())
            .collect();
        for index in 0..10 {
            let unnamed = format!("u{index}");
            assert!(
                reachable.contains(&unnamed.as_str()),
                "{unnamed} is offered somewhere"
            );
        }
    }

    /// Asking for a batch past the end is how a loop finds out there are no
    /// more, so it answers rather than panicking.
    #[test]
    fn a_batch_past_the_end_is_empty() {
        let view = view_with(vec![control("e1", "button", "Only")]);
        let ordered = ActionSpace::ordered(&view, "press it", None, &[], &[]);
        let past = ActionSpace::batch(&ordered, 4);
        assert!(past.offered.is_empty());
        assert_eq!(past.omitted, 0);
    }

    /// One question may offer 254 options, because the escape option takes the
    /// 255th. Asking for 255 is refused with "choice target has 256 options".
    #[test]
    fn a_batch_leaves_room_for_the_escape_option() {
        assert_eq!(MAX_OFFERED, 254);
        let controls: Vec<Control> = (0..300)
            .map(|index| control(&format!("e{index}"), "link", &format!("Link {index}")))
            .collect();
        let view = view_with(controls);
        let ordered = ActionSpace::ordered(&view, "click one", None, &[], &[]);
        let first = ActionSpace::batch(&ordered, 0);
        assert_eq!(first.choice_options().len(), MAX_OFFERED + 1);
    }

    /// The order has to be stable, or a batch index means nothing between one
    /// look at a page and the next.
    #[test]
    fn the_same_page_orders_the_same_way_every_time() {
        let controls: Vec<Control> = (0..50)
            .map(|index| control(&format!("e{index}"), "link", &format!("Link {index}")))
            .collect();
        let view = view_with(controls);
        let once = ActionSpace::ordered(&view, "click a link", None, &[], &[]);
        let twice = ActionSpace::ordered(&view, "click a link", None, &[], &[]);
        assert_eq!(once, twice);
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
        let space = ActionSpace::build(
            &view_with(vec![sort]),
            "sort by price low to high",
            None,
            &[],
        );
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
        let space = ActionSpace::build(&view_with(controls), "choose something", None, &[]);
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
        let space = ActionSpace::build(&view_with(vec![long, short]), "sort by price", None, &[]);
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
        let space = ActionSpace::build(&view_with(vec![long, short]), "pick a capacity", None, &[]);
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
        let space = ActionSpace::build(&view_with(controls), "choose something", None, &[]);
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
            &[],
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
            &[],
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
        let first = ActionSpace::build(&view_with(controls.clone()), "tick Corsair", None, &[]);
        let second = ActionSpace::build(&view_with(controls), "tick Corsair", None, &[]);
        assert_eq!(first.offered, second.offered);
    }

    /// The Hacker News case, which cost a live errand twice over.
    ///
    /// "More" is the last control in a long document and shares no word with
    /// the goal, so it ties on score with every other link and page order
    /// decides. At a cap of forty it was out of reach, and at a cap of a
    /// hundred and twenty it was still out of reach, which is why the fix is a
    /// reservation rather than a bigger cap.
    #[test]
    fn a_pagination_control_at_the_bottom_of_a_long_page_is_still_offered() {
        let mut controls: Vec<Control> = (0..200)
            .map(|index| {
                control(
                    &format!("e{index}"),
                    "link",
                    &format!("Story number {index}"),
                )
            })
            .collect();
        controls.push(control("more", "link", "More"));

        let space = ActionSpace::build(
            &view_with(controls),
            "go to the second page of stories",
            None,
            &[],
        );

        assert!(
            space
                .offered
                .iter()
                .any(|candidate| candidate.reference == "more"),
            "the only control that can satisfy the goal has to be in front of the model"
        );
    }

    /// Reserving must not be reordering. Promoting a kind was tried first and
    /// rescued one errand by pushing another below the cap.
    #[test]
    fn reserving_a_kind_does_not_displace_what_already_earned_its_place() {
        let mut controls = vec![control("wanted", "checkbox", "Corsair")];
        controls.extend(
            (0..200).map(|index| control(&format!("e{index}"), "link", &format!("Item {index}"))),
        );
        controls.push(control("more", "link", "More"));

        let space = ActionSpace::build(
            &view_with(controls),
            "tick the Corsair filter, then go to the next page",
            None,
            &[],
        );

        assert_eq!(
            space.offered[0].reference, "wanted",
            "the control the goal names is still first"
        );
        assert!(
            space
                .offered
                .iter()
                .any(|candidate| candidate.reference == "more"),
            "and the reserved kind got in beside it"
        );
    }

    /// A goal that names a kind must not be shown a smaller page than one that
    /// does not, so an unspent reservation is given back.
    #[test]
    fn an_unspent_reservation_is_given_back_to_the_page() {
        let controls: Vec<Control> = (0..500)
            .map(|index| control(&format!("e{index}"), "link", &format!("Item {index}")))
            .collect();

        // "open" implies a result link, so slots are held back, but there is no
        // pagination or facet to spend them on beyond the links already offered.
        let space = ActionSpace::build(&view_with(controls), "open item 7", None, &[]);

        assert_eq!(
            space.offered.len(),
            MAX_OFFERED,
            "holding slots back must not mean wasting them"
        );
    }

    /// A control that was acted on and changed nothing is never offered again.
    ///
    /// On a live errand the highest ranked link sat inside an image caption
    /// whose click the page's own photo viewer swallowed. The click reported
    /// success, nothing moved, and the run spent its whole budget choosing the
    /// same reference over and over.
    #[test]
    fn a_retired_control_is_not_offered_again() {
        let controls = vec![
            control("dead", "link", "Floyd-Steinberg dithering"),
            control("alive", "link", "Floyd-Steinberg dithering in practice"),
        ];
        let retired = vec!["dead".to_string()];

        let space = ActionSpace::build(
            &view_with(controls),
            "open the article about Floyd-Steinberg dithering",
            None,
            &retired,
        );

        assert!(
            space
                .offered
                .iter()
                .all(|candidate| candidate.reference != "dead"),
            "a retry cannot clear whatever swallowed the first attempt"
        );
        assert!(
            space
                .offered
                .iter()
                .any(|candidate| candidate.reference == "alive"),
            "and the next best candidate is still there to be chosen"
        );
    }

    /// Retiring is not omitting. A control held back by the cap is counted so
    /// the caller knows the page was bigger than the question; a retired one is
    /// gone on purpose.
    #[test]
    fn a_retired_control_is_not_counted_as_omitted() {
        let controls = vec![
            control("dead", "link", "Something"),
            control("alive", "link", "Something else"),
        ];
        let space = ActionSpace::build(
            &view_with(controls),
            "open something",
            None,
            &["dead".to_string()],
        );

        assert_eq!(space.omitted, 0, "nothing was held back by the cap");
    }
}
