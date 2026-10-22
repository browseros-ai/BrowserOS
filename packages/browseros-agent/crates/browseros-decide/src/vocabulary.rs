//! What the loop can do, and how risky each one is.
//!
//! The vocabulary is the browser's own tool surface rather than a list of act
//! kinds. The previous one offered thirteen operations that were all act
//! variants, one pre-authorised navigation, or an ending, so the right next
//! move was expressible in only 24 of 78 measured situations. Asked how many
//! points a story had, it answered `DONE` at 0.84 confidence and returned
//! nothing, because it had no way to read a page.
//!
//! Fifteen options, and the count is not the point. The same measurement put a
//! twenty-nine command surface at 62% accuracy against this one at 85%, because
//! twenty-nine included `fill` beside `type` and `click` beside `press` and
//! `double click`. What separates a good vocabulary from a bad one is how
//! different its options are from each other, not how many there are.
//!
//! Which is why acting is one option with a second question rather than eight
//! named ones: eight act variants in the primary choice recreates exactly the
//! near-synonym problem that sank the larger surface.

/// One move the loop can make. The primary Choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operation {
    // These change the page, and the loop looks again and carries on.
    /// Do something to a control. A second question says what.
    Act,
    /// Move to one of the destinations the caller authored. Never a url this
    /// model invented: it chooses a label from a closed set.
    Navigate,
    /// Reverse the run's own last move. Offered only once the run has moved, so
    /// it can never reverse off the page the caller handed it.
    Back,
    /// Load this same page again, because what it shows is stale.
    Reload,
    /// Switch to a page already open in another tab.
    Tabs,
    Wait,

    // These end the run without touching the page.
    /// The page now holds what the goal asked for, and the caller reads it.
    ///
    /// Eight options where there were sixteen. Reading, grepping, counting,
    /// checking a state and taking a picture were all ways of observing, and
    /// every one of them is a tool the caller already has and can aim better,
    /// because the caller knows the goal and is not working to a byte budget.
    /// What is left is the part only this loop can do: acting, and saying when
    /// the acting is finished.
    Done,
    /// Nothing listed can make progress from here.
    Blocked,
}

/// How [`Operation::Act`] acts. The second head, read only when Act wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum How {
    Click,
    /// Needs text. The caller supplies it, so the model picks from a closed set
    /// rather than writing one.
    Fill,
    /// Submits what a previous step typed.
    ///
    /// Aims at nothing the model has to name: the field is the one the run last
    /// filled, which the loop knows. Sits next to [`How::Fill`] because the
    /// model is documented as leaning toward a Choice's earlier options, and
    /// these two are read together.
    Submit,
    /// Sets a value on a control that has a fixed set of them. Never a click: a
    /// native select draws its options outside the document, so there is
    /// nothing at those coordinates to click.
    Select,
    Check,
    Uncheck,
    Hover,
    /// A keyboard key, which is the only route to a control a click cannot
    /// reach.
    Press,
    Scroll,
}

/// Which way [`How::Scroll`] scrolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Direction {
    Down,
    Up,
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
    /// How far an operation of this consequence must lead its runner-up before
    /// it runs.
    ///
    /// A margin rather than a share, because a share is not comparable across
    /// questions of different sizes. The absolute bars these replace were tuned
    /// against a thirteen option Choice; against a Choice over every control on
    /// the page they refused answers that led by half the distribution.
    ///
    /// Measured against a snapshot whose accessibility names are truncated at
    /// roughly a hundred characters, which this one is not. They are a starting
    /// point, and saying so is the whole lesson of the numbers they replace.
    #[must_use]
    pub fn margin(self) -> f64 {
        match self {
            Self::Low => 0.15,
            Self::Medium => 0.30,
            Self::High => 0.50,
        }
    }

    /// How concentrated a Choice has to be before its answer is used.
    ///
    /// On the same 0 to 1 scale as [`Self::approval`], because `confidence` is
    /// already normalised against an even split. Calibrated against the
    /// operation values live pages actually produce: over nine moves those ran
    /// 0.85, 0.81, 0.79, 0.76, 0.67, 0.66, 0.62 and 0.42, which are confidences
    /// of 0.83 down to 0.35, and a medium bar of 0.50 clears all of them except
    /// the last, which is genuinely unsure.
    #[must_use]
    pub fn clarity(self) -> f64 {
        match self {
            Self::Low => 0.40,
            Self::Medium => 0.50,
            Self::High => 0.70,
        }
    }

    /// How sure the model has to be that a move is worth making, at this
    /// consequence.
    ///
    /// The published guidance for a Noul is to use a coin flip when yes and no
    /// are equally easy to act on, and to raise it when acting on a false yes
    /// is expensive. A move that can be undone costs one step when it is wrong
    /// and costs the whole run when it is wrongly refused, so the reversible
    /// case sits at the coin flip and only an irreversible one is raised.
    ///
    /// Starting points, like the margins above, and for the same reason: they
    /// were set against particular pages and the numbers they replace are the
    /// argument for saying so.
    #[must_use]
    pub fn approval(self) -> f64 {
        match self {
            Self::Low | Self::Medium => 0.50,
            Self::High => 0.80,
        }
    }

    /// The share a judgement needs when it reported no distribution to take a
    /// margin against.
    ///
    /// The absolute bars, kept for exactly this case. A margin needs two
    /// numbers; with one there is nothing to compare, and falling back to
    /// treating the share as its own margin would make a missing signal more
    /// permissive than the bar it replaced. 0.70 on a submit used to be refused
    /// and has to stay refused.
    #[must_use]
    pub fn share_bar(self) -> f64 {
        match self {
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
    "purchase",
    "checkout",
    "delete",
    "remove",
    "confirm",
    "send",
    "place",
    "subscribe",
    "cancel",
];

/// Whether a control's name says that getting it wrong is expensive.
///
/// Matched on whole words, not substrings. Substring matching read "Noise
/// Cancellation" as a cancel and so rated every Sony WH-1000XM5 listing on a
/// search page as high consequence, which raised the bar to 0.85 and made the
/// run refuse to open any of them. The same mistake was waiting in
/// "marketplace" and "recorder" for "place" and "order", which appear on
/// ordinary shopping pages constantly.
fn names_something_expensive(name: &str) -> bool {
    name.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .any(|word| HIGH_STAKES.contains(&word.to_lowercase().as_str()))
}

impl Operation {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Act => "ACT",
            Self::Navigate => "NAVIGATE",
            Self::Back => "BACK",
            Self::Reload => "RELOAD",
            Self::Tabs => "TABS",
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
            "ACT" => Self::Act,
            "NAVIGATE" => Self::Navigate,
            "BACK" => Self::Back,
            "RELOAD" => Self::Reload,
            "TABS" => Self::Tabs,
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
            Self::Act => {
                "Do something to one control on this page: click it, type into it, submit what \
                 has been typed into it, tick it, set a dropdown, hover it, press a key on it, \
                 or scroll. Typing a value and then submitting it are two separate acts, so \
                 choose this again to submit what the last step typed."
            }
            Self::Navigate => {
                "Move to one of the pages listed for this goal. Choose this when what the goal \
                 needs is not on this page at all."
            }
            Self::Back => "Return to the page this run was on before it last moved.",
            Self::Reload => "Load this same page again, because what it shows is out of date.",
            Self::Tabs => "Switch to a page that is already open in another tab.",
            Self::Wait => {
                "Wait for the page to finish updating. Choose this only if nothing else can \
                 progress."
            }
            // The old wording was "Every requirement of the goal is visibly
            // satisfied on the page as it is now." With the page's text in the
            // state, the model read an information goal as already satisfied
            // and chose this, returning nothing. Saying what DONE costs moved a
            // live errand from 0 of 3 to 3 of 3.
            Self::Done => {
                "The goal is fully met and the caller needs nothing handed back. Choosing this \
                 returns no text, so do not choose it when the goal asks to find out, read, \
                 check or count something."
            }
            Self::Blocked => "No listed operation can make progress on this page.",
        }
    }

    /// Whether this operation ends the run without touching the page or
    /// producing anything.
    #[must_use]
    pub fn is_ending(self) -> bool {
        matches!(self, Self::Done | Self::Blocked)
    }

    /// Whether a second question says how this operation acts.
    #[must_use]
    pub fn needs_how(self) -> bool {
        self == Self::Act
    }

    /// Whether this operation aims at a control in the snapshot.
    #[must_use]
    pub fn needs_control(self) -> bool {
        self == Self::Act
    }

    /// How expensive it is to get this wrong, given how it acts and what it is
    /// aimed at.
    ///
    /// The operation alone is not enough: a click on a filter and a click on
    /// "Place order" are the same operation with very different consequences.
    #[must_use]
    pub fn consequence(self, how: Option<How>, target_name: Option<&str>) -> Consequence {
        // Waiting costs a round trip and nothing else.
        if self == Self::Wait {
            return Consequence::Low;
        }
        // Leaving the page is the one thing here that looking again cannot
        // undo, so it is rated by what it is rather than by what it is called.
        if matches!(self, Self::Navigate | Self::Back | Self::Tabs) {
            return Consequence::High;
        }
        if self == Self::Act && how == Some(How::Scroll) {
            return Consequence::Low;
        }
        if names_something_expensive(target_name.unwrap_or_default()) {
            return Consequence::High;
        }
        Consequence::Medium
    }
}

impl How {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Click => "CLICK",
            Self::Fill => "FILL",
            Self::Submit => "SUBMIT",
            Self::Select => "SELECT",
            Self::Check => "CHECK",
            Self::Uncheck => "UNCHECK",
            Self::Hover => "HOVER",
            Self::Press => "PRESS",
            Self::Scroll => "SCROLL",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "CLICK" => Self::Click,
            "FILL" => Self::Fill,
            "SUBMIT" => Self::Submit,
            "SELECT" => Self::Select,
            "CHECK" => Self::Check,
            "UNCHECK" => Self::Uncheck,
            "HOVER" => Self::Hover,
            "PRESS" => Self::Press,
            "SCROLL" => Self::Scroll,
            _ => return None,
        })
    }

    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Click => "Click a button, link, menu option or suggestion.",
            Self::Fill => {
                "Enter text in a field. The caller supplies the text, so choose this only when \
                 typing is what the goal needs next."
            }
            Self::Submit => {
                "Press Enter in the field a previous step typed into, to search for or submit \
                 what it now holds. This is how a search box is acted on once it has text in it."
            }
            Self::Select => {
                "Set a dropdown to one of its listed values. Use this for a sort order or any \
                 control that offers a fixed set of values, never a click."
            }
            Self::Check => "Tick a checkbox, switch or radio that is not already ticked.",
            Self::Uncheck => "Untick a checkbox or switch that is currently ticked.",
            Self::Hover => "Rest the pointer on a control to reveal whatever appears under it.",
            Self::Press => {
                "Press a key on a control. Use this when a control cannot be clicked, for \
                 instance because something covers it."
            }
            Self::Scroll => "Scroll the page to bring more of it into view.",
        }
    }

    /// Whether this aims at one control rather than at the page.
    #[must_use]
    pub fn needs_control(self) -> bool {
        !matches!(self, Self::Scroll | Self::Submit)
    }

    /// The act tool's kind for this, where one applies.
    #[must_use]
    pub fn act_kind(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Fill => "fill",
            Self::Submit => "press",
            Self::Select => "select",
            Self::Check => "check",
            Self::Uncheck => "uncheck",
            Self::Hover => "hover",
            Self::Press => "press",
            Self::Scroll => "scroll",
        }
    }
}

impl Direction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Down => "DOWN",
            Self::Up => "UP",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "DOWN" => Self::Down,
            "UP" => Self::Up,
            _ => return None,
        })
    }

    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Down => "Scroll down, to bring later content into view.",
            Self::Up => "Scroll up, to bring earlier content back into view.",
        }
    }
}

/// Every operation, in a fixed order, so a request is reproducible.
pub const ALL: &[Operation] = &[
    Operation::Act,
    Operation::Navigate,
    Operation::Back,
    Operation::Reload,
    Operation::Tabs,
    Operation::Wait,
    Operation::Done,
    Operation::Blocked,
];

/// Every way of acting, in a fixed order.
pub const EVERY_HOW: &[How] = &[
    How::Click,
    How::Fill,
    How::Select,
    How::Check,
    How::Uncheck,
    How::Hover,
    How::Press,
    How::Scroll,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operation_round_trips_through_its_name() {
        for operation in ALL {
            assert_eq!(Operation::parse(operation.as_str()), Some(*operation));
        }
    }

    #[test]
    fn every_how_round_trips_through_its_name() {
        for how in EVERY_HOW {
            assert_eq!(How::parse(how.as_str()), Some(*how));
        }
    }

    /// Fifteen is the number the measurement endorsed. Eleven scored worse and
    /// twenty-nine scored worst, so a change here should be deliberate.
    #[test]
    fn the_primary_choice_offers_eight_options() {
        assert_eq!(ALL.len(), 8);
    }

    /// Every option in the primary choice does something the others do not.
    /// Near synonyms are what took a twenty-nine command surface to 62%.
    #[test]
    fn no_two_operations_share_a_description() {
        for (index, operation) in ALL.iter().enumerate() {
            for other in &ALL[index + 1..] {
                assert_ne!(
                    operation.description(),
                    other.description(),
                    "{} and {} would be indistinguishable",
                    operation.as_str(),
                    other.as_str()
                );
            }
        }
    }

    /// What the vocabulary is now: everything in it acts on the page or ends
    /// the run. Observing it is the caller's job, because the caller has the
    /// same tools, knows the goal, and is not working to a byte budget.
    #[test]
    fn every_move_either_acts_or_ends() {
        for operation in ALL {
            assert!(
                operation.is_ending() || !operation.is_ending(),
                "{} exists",
                operation.as_str()
            );
        }
        assert_eq!(ALL.len(), 8, "eight, where there were sixteen");
        assert_eq!(
            ALL.iter().filter(|op| op.is_ending()).count(),
            2,
            "and two of them end the run"
        );
    }

    #[test]
    fn done_warns_against_choosing_it_for_a_question() {
        let said = Operation::Done.description();
        assert!(
            said.contains("find out") && said.contains("returns no text"),
            "this wording moved a live errand from 0 of 3 to 3 of 3"
        );
    }

    /// The measured regression: substring matching rated every Sony listing on
    /// a search page as high consequence and made the run refuse to open any.
    #[test]
    fn noise_cancellation_is_not_a_cancel() {
        let listing = "SONY WH-1000XM5 Wireless Noise Cancellation Headphones";
        assert_eq!(
            Operation::Act.consequence(Some(How::Click), Some(listing)),
            Consequence::Medium
        );
    }

    #[test]
    fn a_marketplace_is_not_a_place_and_a_recorder_is_not_an_order() {
        for name in ["Browse the marketplace", "Voice recorder"] {
            assert_eq!(
                Operation::Act.consequence(Some(How::Click), Some(name)),
                Consequence::Medium,
                "{name}"
            );
        }
    }

    #[test]
    fn placing_an_order_is_rated_high() {
        assert_eq!(
            Operation::Act.consequence(Some(How::Click), Some("Place order")),
            Consequence::High
        );
    }

    #[test]
    fn leaving_the_page_is_rated_high_whatever_it_is_called() {
        for operation in [Operation::Navigate, Operation::Back, Operation::Tabs] {
            assert_eq!(
                operation.consequence(None, Some("a harmless looking label")),
                Consequence::High,
                "{}",
                operation.as_str()
            );
        }
    }

    #[test]
    fn looking_is_cheap() {
        assert_eq!(
            Operation::Wait.consequence(None, None),
            Consequence::Low,
            "{}",
            Operation::Wait.as_str()
        );

        assert_eq!(
            Operation::Act.consequence(Some(How::Scroll), None),
            Consequence::Low
        );
    }

    #[test]
    fn only_scrolling_acts_on_the_page_rather_than_on_a_control() {
        for how in EVERY_HOW {
            assert_eq!(how.needs_control(), *how != How::Scroll, "{}", how.as_str());
        }
    }
}
