//! What the loop can do, and how risky each one is.
//!
//! The vocabulary is drawn from what the browser's act tool already supports
//! rather than from what is convenient to ask for. The previous implementation
//! could emit five of the seventeen kinds act accepts, which is why a dropdown
//! could only be attempted as a click and never worked: a native select draws
//! its options outside the document, so there is nothing at those coordinates
//! to click.

/// One thing the loop can do to a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operation {
    Click,
    /// Sets a value on a control that has a fixed set of them. Never a click.
    Select,
    Check,
    Uncheck,
    /// A keyboard key, which is the only route to a control a click cannot
    /// reach.
    Press,
    /// Needs text, which this model is not trained to generate, so the loop
    /// hands back for it.
    Fill,
    ScrollDown,
    ScrollUp,
    Wait,
    Done,
    Blocked,
}

/// How much it costs to get an operation wrong.
///
/// The published guidance is to gate different actions at different levels
/// according to the consequence of being wrong, with a floor below which
/// nothing should act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consequence {
    /// Costs a round trip and nothing else.
    Low,
    /// Changes the page.
    Medium,
    /// Submits, pays, deletes, or leaves the page for good.
    High,
}

impl Consequence {
    /// The confidence an operation of this consequence needs before it runs.
    #[must_use]
    pub fn bar(self) -> f64 {
        match self {
            // The documented floor: below this, nothing acts.
            Self::Low => 0.60,
            Self::Medium => 0.75,
            Self::High => 0.85,
        }
    }
}

/// Keys the loop may press. A closed set, because an open one would be a
/// generated string and this model does not generate.
pub const PRESS_KEYS: &[&str] = &["Enter", "Escape", "Tab", "ArrowDown", "ArrowUp", "Space"];

/// Words in a control's name that mean getting it wrong is expensive.
const HIGH_STAKES: &[&str] = &[
    "submit",
    "pay",
    "buy",
    "order",
    "checkout",
    "delete",
    "remove",
    "confirm",
    "send",
    "place",
    "subscribe",
    "cancel",
];

impl Operation {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Click => "CLICK",
            Self::Select => "SELECT",
            Self::Check => "CHECK",
            Self::Uncheck => "UNCHECK",
            Self::Press => "PRESS",
            Self::Fill => "FILL",
            Self::ScrollDown => "SCROLL_DOWN",
            Self::ScrollUp => "SCROLL_UP",
            Self::Wait => "WAIT",
            Self::Done => "DONE",
            Self::Blocked => "BLOCKED",
        }
    }

    /// Named `parse` rather than `from_str` so it is not mistaken for the
    /// standard trait, whose error type this does not need.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "CLICK" => Self::Click,
            "SELECT" => Self::Select,
            "CHECK" => Self::Check,
            "UNCHECK" => Self::Uncheck,
            "PRESS" => Self::Press,
            "FILL" => Self::Fill,
            "SCROLL_DOWN" => Self::ScrollDown,
            "SCROLL_UP" => Self::ScrollUp,
            "WAIT" => Self::Wait,
            "DONE" => Self::Done,
            "BLOCKED" => Self::Blocked,
            _ => return None,
        })
    }

    /// What the model is told this operation does.
    ///
    /// Written as what it achieves rather than how it is implemented, since the
    /// model is choosing an outcome and not calling a function.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Click => "Click a button, link, menu option or suggestion.",
            Self::Select => {
                "Set a dropdown to one of its listed values. Use this for a sort order or any \
                 control that offers a fixed set of values, never a click."
            }
            Self::Check => "Tick a checkbox, switch or radio that is not already ticked.",
            Self::Uncheck => "Untick a checkbox or switch that is currently ticked.",
            Self::Press => {
                "Press a key on a control. Use this when a control cannot be clicked, for \
                 instance because something covers it."
            }
            Self::Fill => {
                "Enter text in a field. The caller supplies the text, so choose this only when \
                 typing is what the goal needs next."
            }
            Self::ScrollDown => "Scroll down to bring more of the page into view.",
            Self::ScrollUp => "Scroll up to bring earlier content back into view.",
            Self::Wait => {
                "Wait for the page to finish updating. Choose this only if nothing else can progress."
            }
            Self::Done => {
                "Every requirement of the goal is visibly satisfied on the page as it is now."
            }
            Self::Blocked => "No listed operation can make progress on this page.",
        }
    }

    /// Whether this operation needs a target control.
    #[must_use]
    pub fn needs_target(self) -> bool {
        matches!(
            self,
            Self::Click | Self::Select | Self::Check | Self::Uncheck | Self::Press | Self::Fill
        )
    }

    /// Whether this operation ends the run without touching the page.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Blocked)
    }

    /// How expensive it is to get this wrong, given what it is aimed at.
    ///
    /// The operation alone is not enough: a click on a filter and a click on
    /// "Place order" are the same operation with very different consequences.
    #[must_use]
    pub fn consequence(self, target_name: Option<&str>) -> Consequence {
        if matches!(self, Self::ScrollDown | Self::ScrollUp | Self::Wait) {
            return Consequence::Low;
        }
        let name = target_name.unwrap_or_default().to_lowercase();
        if HIGH_STAKES.iter().any(|word| name.contains(word)) {
            return Consequence::High;
        }
        Consequence::Medium
    }

    /// The act tool's kind for this operation, where one applies.
    #[must_use]
    pub fn act_kind(self) -> Option<&'static str> {
        Some(match self {
            Self::Click => "click",
            Self::Select => "select",
            Self::Check => "check",
            Self::Uncheck => "uncheck",
            Self::Press => "press",
            Self::Fill => "fill",
            Self::ScrollDown | Self::ScrollUp => "scroll",
            Self::Wait | Self::Done | Self::Blocked => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every operation that touches the page maps onto a kind the act tool
    /// already accepts. That is the whole reason this vocabulary is reachable
    /// without new browser work.
    #[test]
    fn every_acting_operation_maps_to_an_act_kind() {
        const ACT_KINDS: &[&str] = &[
            "click",
            "click_at",
            "type",
            "type_at",
            "fill",
            "press",
            "hover",
            "hover_at",
            "focus",
            "check",
            "uncheck",
            "select",
            "scroll",
            "drag",
            "drag_at",
            "dialog_accept",
            "dialog_dismiss",
        ];
        for operation in [
            Operation::Click,
            Operation::Select,
            Operation::Check,
            Operation::Uncheck,
            Operation::Press,
            Operation::Fill,
            Operation::ScrollDown,
            Operation::ScrollUp,
        ] {
            let kind = operation
                .act_kind()
                .expect("an acting operation has a kind");
            assert!(
                ACT_KINDS.contains(&kind),
                "{operation:?} maps to {kind}, which act does not accept"
            );
        }
        assert!(Operation::Wait.act_kind().is_none());
        assert!(Operation::Done.act_kind().is_none());
    }

    #[test]
    fn operation_names_round_trip() {
        for operation in [
            Operation::Click,
            Operation::Select,
            Operation::Check,
            Operation::Uncheck,
            Operation::Press,
            Operation::Fill,
            Operation::ScrollDown,
            Operation::ScrollUp,
            Operation::Wait,
            Operation::Done,
            Operation::Blocked,
        ] {
            assert_eq!(Operation::parse(operation.as_str()), Some(operation));
        }
        assert_eq!(Operation::parse("NONSENSE"), None);
    }

    /// Scrolling costs a round trip; clicking changes the page; clicking
    /// something called "Place order" is a different matter again.
    #[test]
    fn consequence_follows_what_the_operation_is_aimed_at() {
        assert_eq!(Operation::ScrollDown.consequence(None), Consequence::Low);
        assert_eq!(
            Operation::Click.consequence(Some("Corsair")),
            Consequence::Medium
        );
        assert_eq!(
            Operation::Click.consequence(Some("Place your order")),
            Consequence::High
        );
        assert_eq!(
            Operation::Click.consequence(Some("Delete account")),
            Consequence::High
        );
    }

    /// The bars rise with the consequence, and the lowest is the documented
    /// floor below which nothing should act.
    #[test]
    fn the_bars_rise_with_the_consequence() {
        assert!((Consequence::Low.bar() - 0.60).abs() < f64::EPSILON);
        assert!(Consequence::Low.bar() < Consequence::Medium.bar());
        assert!(Consequence::Medium.bar() < Consequence::High.bar());
    }

    #[test]
    fn only_the_operations_that_aim_at_something_need_a_target() {
        assert!(Operation::Click.needs_target());
        assert!(Operation::Select.needs_target());
        assert!(!Operation::ScrollDown.needs_target());
        assert!(!Operation::Done.needs_target());
        assert!(Operation::Done.is_terminal());
        assert!(!Operation::Click.is_terminal());
    }

    /// The press keys are a closed set, because an open one would be generated
    /// text and this model is not trained to generate.
    #[test]
    fn the_press_keys_are_a_closed_set() {
        assert!(PRESS_KEYS.contains(&"Enter"));
        assert!(!PRESS_KEYS.is_empty());
        assert!(PRESS_KEYS.len() < 10, "small enough to be a real choice");
    }
}
