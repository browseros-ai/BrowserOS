//! The per-observation action space, and the arity rule that the evaluation
//! protocol forces on it.
//!
//! A choice question accepts 2 to 255 options. That is not an edge case to
//! handle later: a page with a single button produces a one-option head, and
//! the client refuses the whole request rather than that one question. So every
//! head is resolved into one of three shapes before anything is sent.

use serde::Serialize;
use std::collections::BTreeMap;

/// Upper bound the protocol places on a single choice question.
pub const PROTOCOL_MAX_OPTIONS: usize = 255;

/// How many candidates a target head offers at most.
///
/// Held below [`PROTOCOL_MAX_OPTIONS`] because one probability is returned per
/// option, so the option count is a cost on the response as well as the
/// request. A head at the protocol ceiling returns roughly 2,400 output tokens
/// for a single decision, against under 150 for a small page.
pub const MAX_TARGET_OPTIONS: usize = 250;

/// One element the page offered, reduced to what a decision needs.
///
/// Deliberately not a browser type: the policy is testable without a browser,
/// and the conversion from a snapshot happens at the edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Element {
    /// The ref the browser already assigned, such as `e12`.
    pub reference: String,
    pub role: String,
    pub name: String,
    /// What the control currently holds, when it holds anything.
    ///
    /// Carried here rather than left to the rendered tree, because the tree is
    /// trimmed to fit a budget and this is the difference between filling an
    /// empty field and overwriting a correct one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// State annotations the renderer attached, such as `disabled` or
    /// `checked`. A disabled control that looks enabled is a wasted step.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub state: Vec<String>,
}

impl Element {
    pub fn new(
        reference: impl Into<String>,
        role: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            reference: reference.into(),
            role: role.into(),
            name: name.into(),
            value: None,
            state: Vec::new(),
        }
    }

    /// Whether text can be typed into this element.
    #[must_use]
    pub fn is_editable(&self) -> bool {
        matches!(
            self.role.as_str(),
            "textbox" | "searchbox" | "combobox" | "spinbutton"
        )
    }

    /// The label offered to the model, carrying the ref so the answer and the
    /// rendered tree can be lined up by a reader.
    ///
    /// The current value and any state ride on the label, so a candidate is
    /// self describing even when the tree around it was trimmed away.
    #[must_use]
    pub fn label(&self) -> String {
        let mut label = if self.name.is_empty() {
            format!("[{}] {}", self.reference, self.role)
        } else {
            format!("[{}] {} {}", self.reference, self.role, self.name)
        };
        if !self.state.is_empty() {
            label.push_str(&format!(" [{}]", self.state.join(" ")));
        }
        match self.value.as_deref() {
            Some(value) if !value.is_empty() => label.push_str(&format!(" = {value:?}")),
            Some(_) => label.push_str(" = empty"),
            None => {}
        }
        label
    }
}

/// Reads the value and state annotations the renderer attached to each ref,
/// and copies them onto the matching elements.
///
/// The browser's ref table carries role and name but not the current value, so
/// the rendered line is the only place it exists. The format is the renderer's
/// own and is stable: states sit in brackets before the ref, and a value
/// follows the ref after a colon.
///
/// ```text
/// - textbox "Search" [ref=e3]: "abc"
/// - button "Load more" [disabled] [ref=e2]
/// ```
pub fn annotate_from_tree(elements: &mut [Element], tree: &str) {
    let mut found: BTreeMap<&str, (Option<String>, Vec<String>)> = BTreeMap::new();
    for line in tree.lines() {
        let Some(marker) = line.find(" [ref=") else {
            continue;
        };
        let after = &line[marker + " [ref=".len()..];
        let Some(close) = after.find(']') else {
            continue;
        };
        let reference = &after[..close];
        if reference.is_empty() {
            continue;
        }
        let value = after[close + 1..]
            .strip_prefix(": ")
            .map(|raw| raw.trim().trim_matches('"').to_string());
        let state = line[..marker]
            .match_indices('[')
            .filter_map(|(open, _)| {
                let rest = &line[open + 1..];
                rest.find(']').map(|end| rest[..end].to_string())
            })
            .filter(|token| !token.is_empty() && !token.starts_with("ref="))
            .collect();
        found.insert(reference, (value, state));
    }
    for element in elements {
        if let Some((value, state)) = found.get(element.reference.as_str()) {
            element.value = value.clone();
            element.state = state.clone();
        }
    }
}

/// What an operation's target head resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    /// Nothing supports this operation, so the operation itself is not offered.
    Unavailable,
    /// Exactly one candidate. A question needs two, and asking would be
    /// pointless anyway: if the operation is chosen, this is the target.
    Implied(String),
    /// Two or more candidates, so a real choice question.
    Question {
        options: BTreeMap<String, String>,
        /// Candidates dropped by [`MAX_TARGET_OPTIONS`], reported so a caller
        /// can tell a capped page from a small one.
        dropped: usize,
    },
}

impl Head {
    /// Resolves candidates into one of the three shapes.
    #[must_use]
    pub fn from_candidates(candidates: &[&Element]) -> Self {
        match candidates.len() {
            0 => Self::Unavailable,
            1 => Self::Implied(candidates[0].reference.clone()),
            _ => {
                let dropped = candidates.len().saturating_sub(MAX_TARGET_OPTIONS);
                // Taking the first 250 in document order spends the budget on
                // whatever sits at the top of the page, which on a search result
                // or an article is navigation, filters and footnote markers, and
                // drops the content. A model choosing by label can do nothing
                // with an unnamed control anyway, so named candidates are kept
                // first and anything dropped is the least useful of them.
                // Document order is preserved within each group.
                let (named, unnamed): (Vec<&&Element>, Vec<&&Element>) = candidates
                    .iter()
                    .partition(|element| !element.name.trim().is_empty());
                let options = named
                    .into_iter()
                    .chain(unnamed)
                    .take(MAX_TARGET_OPTIONS)
                    .map(|element| (element.reference.clone(), element.label()))
                    .collect();
                Self::Question { options, dropped }
            }
        }
    }

    /// Whether the operation this head belongs to can be offered at all.
    #[must_use]
    pub fn is_available(&self) -> bool {
        !matches!(self, Self::Unavailable)
    }
}

/// The operations a decision may choose between.
///
/// `run` and `evaluate` are deliberately absent and must never be added: both
/// execute JavaScript, and the property that makes this safe is that a model
/// answer is an index into elements the browser already observed, never code,
/// a selector or a coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operation {
    Click,
    TypeText,
    ScrollDown,
    ScrollUp,
    Wait,
    Done,
    Blocked,
}

impl Operation {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Click => "CLICK",
            Self::TypeText => "TYPE_TEXT",
            Self::ScrollDown => "SCROLL_DOWN",
            Self::ScrollUp => "SCROLL_UP",
            Self::Wait => "WAIT",
            Self::Done => "DONE",
            Self::Blocked => "BLOCKED",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "CLICK" => Self::Click,
            "TYPE_TEXT" => Self::TypeText,
            "SCROLL_DOWN" => Self::ScrollDown,
            "SCROLL_UP" => Self::ScrollUp,
            "WAIT" => Self::Wait,
            "DONE" => Self::Done,
            "BLOCKED" => Self::Blocked,
            _ => return None,
        })
    }

    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Click => "Click an element, button, menu option, suggestion or calendar day.",
            Self::TypeText => "Enter or replace text in an editable field.",
            Self::ScrollDown => "Reveal content below the viewport.",
            Self::ScrollUp => "Reveal content above the viewport.",
            Self::Wait => {
                "The control you need is absent or disabled, or submitted results are loading."
            }
            Self::Done => "Every requirement of the goal is visibly satisfied.",
            Self::Blocked => "No supported operation can make progress.",
        }
    }

    /// Whether choosing this operation ends the run.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Blocked)
    }

    /// Whether this operation needs a target element.
    #[must_use]
    pub fn needs_target(self) -> bool {
        matches!(self, Self::Click | Self::TypeText)
    }
}

/// Everything one observation offers, with each head already resolved.
#[derive(Debug, Clone)]
pub struct ActionSpace {
    pub elements: Vec<Element>,
    pub click: Head,
    pub type_text: Head,
    pub can_scroll_down: bool,
    pub can_scroll_up: bool,
}

impl ActionSpace {
    /// Partitions the observed elements into the heads a decision needs.
    #[must_use]
    pub fn new(elements: Vec<Element>, can_scroll_down: bool, can_scroll_up: bool) -> Self {
        // Every observed element can be clicked, including editable ones: opening
        // a combobox before typing into it is a normal step.
        let clickable: Vec<&Element> = elements.iter().collect();
        let editable: Vec<&Element> = elements.iter().filter(|e| e.is_editable()).collect();
        Self {
            click: Head::from_candidates(&clickable),
            type_text: Head::from_candidates(&editable),
            can_scroll_down,
            can_scroll_up,
            elements,
        }
    }

    /// The operations worth offering for this observation.
    ///
    /// An operation whose head is unavailable is left out entirely, so the model
    /// is never offered something that cannot be executed. `DONE` and `BLOCKED`
    /// are always present, which also guarantees the operation question itself
    /// always has the two options the protocol requires.
    #[must_use]
    pub fn available_operations(&self) -> Vec<Operation> {
        let mut operations = Vec::new();
        if self.click.is_available() {
            operations.push(Operation::Click);
        }
        if self.type_text.is_available() {
            operations.push(Operation::TypeText);
        }
        if self.can_scroll_down {
            operations.push(Operation::ScrollDown);
        }
        if self.can_scroll_up {
            operations.push(Operation::ScrollUp);
        }
        operations.push(Operation::Wait);
        operations.push(Operation::Done);
        operations.push(Operation::Blocked);
        operations
    }

    /// The head backing an operation, if it has one.
    #[must_use]
    pub fn head(&self, operation: Operation) -> Option<&Head> {
        match operation {
            Operation::Click => Some(&self.click),
            Operation::TypeText => Some(&self.type_text),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(roles: &[(&str, &str)]) -> Vec<Element> {
        roles
            .iter()
            .enumerate()
            .map(|(index, (role, name))| Element::new(format!("e{}", index + 1), *role, *name))
            .collect()
    }

    /// The case that refuses a whole request if it is not handled: one button.
    #[test]
    fn a_single_candidate_is_implied_rather_than_asked() {
        let space = ActionSpace::new(page(&[("button", "Search")]), false, false);
        assert_eq!(space.click, Head::Implied("e1".to_string()));
        assert_eq!(
            space.type_text,
            Head::Unavailable,
            "nothing is editable, so the operation must not be offered"
        );
        assert!(!space.available_operations().contains(&Operation::TypeText));
        assert!(space.available_operations().contains(&Operation::Click));
    }

    #[test]
    fn no_candidates_removes_the_operation_entirely() {
        let space = ActionSpace::new(Vec::new(), false, false);
        assert_eq!(space.click, Head::Unavailable);
        assert_eq!(space.type_text, Head::Unavailable);
        let operations = space.available_operations();
        assert!(!operations.contains(&Operation::Click));
        assert!(!operations.contains(&Operation::TypeText));
        assert!(
            operations.len() >= 2,
            "the operation question itself needs two options: {operations:?}"
        );
        assert!(operations.contains(&Operation::Done));
        assert!(operations.contains(&Operation::Blocked));
    }

    #[test]
    fn two_or_more_candidates_become_a_question_carrying_the_refs() {
        let space = ActionSpace::new(
            page(&[("textbox", "Where from?"), ("textbox", "Where to?")]),
            false,
            false,
        );
        let Head::Question { options, dropped } = &space.type_text else {
            panic!("expected a question, got {:?}", space.type_text);
        };
        assert_eq!(*dropped, 0);
        assert_eq!(options.len(), 2);
        assert_eq!(
            options.get("e1").map(String::as_str),
            Some("[e1] textbox Where from?")
        );
    }

    /// The option count is a cost on both sides of the request, so the cap has
    /// to hold and the number dropped has to be reported.
    #[test]
    fn candidates_are_capped_below_the_protocol_limit_and_the_drop_is_reported() {
        let many: Vec<Element> = (1..=MAX_TARGET_OPTIONS + 7)
            .map(|index| Element::new(format!("e{index}"), "button", format!("Button {index}")))
            .collect();
        let space = ActionSpace::new(many, false, false);
        let Head::Question { options, dropped } = &space.click else {
            panic!("expected a question");
        };
        assert_eq!(options.len(), MAX_TARGET_OPTIONS);
        assert_eq!(*dropped, 7);
        assert!(
            options.len() <= PROTOCOL_MAX_OPTIONS,
            "a head over the protocol limit is refused outright"
        );
    }

    /// The bug this closes: the cap was spent on whatever came first in document
    /// order, which on a real page is navigation chrome and footnote markers,
    /// and the named content was what got dropped.
    #[test]
    fn the_cap_is_spent_on_named_candidates_before_unnamed_ones() {
        // Unnamed controls first, as a page's chrome and footnote markers are.
        let mut elements: Vec<Element> = (1..=MAX_TARGET_OPTIONS)
            .map(|index| Element::new(format!("chrome{index}"), "link", ""))
            .collect();
        elements.push(Element::new("content1", "link", "Add to basket"));
        elements.push(Element::new("content2", "link", "32GB DDR5 6000MHz"));

        let space = ActionSpace::new(elements, false, false);
        let Head::Question { options, dropped } = &space.click else {
            panic!("expected a question");
        };
        assert_eq!(options.len(), MAX_TARGET_OPTIONS);
        assert_eq!(
            *dropped, 2,
            "the count dropped is unchanged by the ordering"
        );
        assert!(
            options.contains_key("content1") && options.contains_key("content2"),
            "a named candidate must survive the cap"
        );
    }

    /// Document order still decides between candidates of the same kind, so the
    /// offered set stays stable for a page that has not changed.
    #[test]
    fn named_candidates_keep_their_document_order() {
        let elements: Vec<Element> = (1..=MAX_TARGET_OPTIONS + 3)
            .map(|index| Element::new(format!("e{index}"), "button", format!("Button {index}")))
            .collect();
        let space = ActionSpace::new(elements, false, false);
        let Head::Question { options, .. } = &space.click else {
            panic!("expected a question");
        };
        assert!(options.contains_key("e1"), "the first is kept");
        assert!(
            !options.contains_key(&format!("e{}", MAX_TARGET_OPTIONS + 3)),
            "the last past the cap is dropped"
        );
    }

    #[test]
    fn editable_roles_are_the_only_type_text_candidates() {
        let space = ActionSpace::new(
            page(&[
                ("button", "Search"),
                ("textbox", "Origin"),
                ("link", "Help"),
                ("searchbox", "Query"),
                ("combobox", "Class"),
                ("spinbutton", "Adults"),
                ("checkbox", "Direct only"),
            ]),
            false,
            false,
        );
        let Head::Question { options, .. } = &space.type_text else {
            panic!("expected a question");
        };
        let mut refs: Vec<&str> = options.keys().map(String::as_str).collect();
        refs.sort_unstable();
        assert_eq!(refs, vec!["e2", "e4", "e5", "e6"]);
    }

    /// An editable field is clickable too, because opening a combobox before
    /// typing is a normal step.
    #[test]
    fn editable_elements_remain_clickable() {
        let space = ActionSpace::new(
            page(&[("textbox", "Origin"), ("button", "Search")]),
            false,
            false,
        );
        let Head::Question { options, .. } = &space.click else {
            panic!("expected a question");
        };
        assert!(options.contains_key("e1"));
        assert!(options.contains_key("e2"));
    }

    #[test]
    fn scroll_operations_follow_the_viewport() {
        let space = ActionSpace::new(page(&[("button", "Go")]), true, false);
        let operations = space.available_operations();
        assert!(operations.contains(&Operation::ScrollDown));
        assert!(!operations.contains(&Operation::ScrollUp));
    }

    /// The gap this closes: the browser's ref table has role and name but not
    /// the current value, so without reading it off the rendered line a filled
    /// field is indistinguishable from an empty one.
    #[test]
    fn values_and_states_are_read_off_the_rendered_lines() {
        let tree = concat!(
            "- banner\n",
            "  - link \"Home\" [ref=e1]\n",
            "  - textbox \"Where to?\" [ref=e2]: \"Zurich\"\n",
            "  - textbox \"Where from?\" [ref=e3]\n",
            "  - button \"Load more\" [disabled] [ref=e4]\n",
            "  - checkbox \"Direct only\" [checked] [ref=e5]"
        );
        let mut elements = vec![
            Element::new("e1", "link", "Home"),
            Element::new("e2", "textbox", "Where to?"),
            Element::new("e3", "textbox", "Where from?"),
            Element::new("e4", "button", "Load more"),
            Element::new("e5", "checkbox", "Direct only"),
        ];
        annotate_from_tree(&mut elements, tree);

        assert_eq!(elements[1].value.as_deref(), Some("Zurich"));
        assert_eq!(elements[2].value, None, "an empty field carries no value");
        assert_eq!(elements[3].state, vec!["disabled".to_string()]);
        assert_eq!(elements[4].state, vec!["checked".to_string()]);
        assert!(elements[0].state.is_empty());

        // And the label a candidate is offered under says so, which is what
        // survives the tree being trimmed.
        assert_eq!(elements[1].label(), "[e2] textbox Where to? = \"Zurich\"");
        assert_eq!(elements[3].label(), "[e4] button Load more [disabled]");
    }

    #[test]
    fn annotating_ignores_lines_without_a_ref() {
        let mut elements = vec![Element::new("e1", "button", "Go")];
        annotate_from_tree(&mut elements, "- text \"nothing here\"\n- heading \"Hi\"");
        assert_eq!(elements[0].value, None);
        assert!(elements[0].state.is_empty());
    }

    #[test]
    fn operation_names_round_trip() {
        for operation in [
            Operation::Click,
            Operation::TypeText,
            Operation::ScrollDown,
            Operation::ScrollUp,
            Operation::Wait,
            Operation::Done,
            Operation::Blocked,
        ] {
            assert_eq!(Operation::parse(operation.as_str()), Some(operation));
        }
        assert_eq!(
            Operation::parse("RUN"),
            None,
            "code execution is not an operation"
        );
        assert_eq!(Operation::parse("EVALUATE"), None);
    }
}
