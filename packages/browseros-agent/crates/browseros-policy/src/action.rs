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
    #[must_use]
    pub fn label(&self) -> String {
        if self.name.is_empty() {
            format!("[{}] {}", self.reference, self.role)
        } else {
            format!("[{}] {} {}", self.reference, self.role, self.name)
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
                let options = candidates
                    .iter()
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
