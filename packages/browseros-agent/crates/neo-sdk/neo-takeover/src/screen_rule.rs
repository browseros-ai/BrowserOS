//! The screen rule: whether the owner holds Neo's one screen, so that the agent may not move its
//! front and Neo's own followers pause (decision 44). It is pure, like `lock_decision`, over the
//! facts every holder keeps in one registry (`BrowserTabs`' front, which every part's clone
//! shares). The door (`Front::check`), the agent's move (`Front::bring_to_front`, under the front
//! turn), follow's pause and decision 41's admission all read this one function over the same
//! facts, so they cannot disagree.
//!
//! A host lock alone is not here on purpose: a lock guards its tab's content, never the screen,
//! and `Locks::check` already refuses every write to a locked tab. A new kind of owner hold is a
//! new [`ScreenHolder`] its part keeps as a guard, and every reader sees it.

use std::fmt;

/// The owner's live holds on the screen at one instant, counted by kind: several fits or control
/// displays can hold it at once, and each guard's drop is one decrement. Plain data, so the rule
/// is a table test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScreenFacts {
    /// A takeover's screen guard lives, from its start to its release. It also covers the gap
    /// while the host's control relay reconnects (synthesis item 2).
    pub(crate) takeover: usize,
    /// A control display is admitted, with or without input: the owner's next key goes to
    /// whatever is in front.
    pub(crate) control: usize,
    /// A fit is claiming the surface or is active, a host's or a takeover's: the screen and the
    /// window are shaped around its tab, and a phone fit has no tab strip.
    pub(crate) fit: usize,
}

impl ScreenFacts {
    /// The count of live holds of kind `by`.
    pub(crate) fn count(&mut self, by: ScreenHolder) -> &mut usize {
        match by {
            ScreenHolder::Takeover => &mut self.takeover,
            ScreenHolder::ControlDisplay => &mut self.control,
            ScreenHolder::Fit => &mut self.fit,
        }
    }
}

/// What holds the screen for its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenHolder {
    /// A takeover, from its start to its release.
    Takeover,
    /// An open control display.
    ControlDisplay,
    /// A fit, from the start of its claim to the end of its restore.
    Fit,
}

impl fmt::Display for ScreenHolder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Takeover => "a takeover",
            Self::ControlDisplay => "a control display",
            Self::Fit => "a fit",
        })
    }
}

/// The agent's move of the front was refused because the owner holds the screen. The app's door
/// turns it into `host / owner_has_control`, `not_sent`: the code a tab lock gets, so an agent
/// needs no new handling, with a message that names the holder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("The owner has the screen ({by}). Nothing was done. Wait for them to give it back.")]
pub struct OwnerHasScreen {
    /// The holder named, when several hold it: a takeover, then a control display, then a fit.
    pub by: ScreenHolder,
}

/// The rule: none held is `Ok`; otherwise the most specific holder, a takeover first (its fit
/// and its control display are part of it), then a control display, then a fit.
pub(crate) const fn decide(facts: ScreenFacts) -> Result<(), OwnerHasScreen> {
    if facts.takeover > 0 {
        Err(OwnerHasScreen {
            by: ScreenHolder::Takeover,
        })
    } else if facts.control > 0 {
        Err(OwnerHasScreen {
            by: ScreenHolder::ControlDisplay,
        })
    } else if facts.fit > 0 {
        Err(OwnerHasScreen {
            by: ScreenHolder::Fit,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{OwnerHasScreen, ScreenFacts, ScreenHolder, decide};

    fn held(by: ScreenHolder) -> Result<(), OwnerHasScreen> {
        Err(OwnerHasScreen { by })
    }

    #[test]
    fn the_owner_holds_the_screen_through_a_takeover_a_control_display_or_a_fit() {
        use ScreenHolder::{ControlDisplay, Fit, Takeover};
        let cases = [
            ((0, 0, 0), Ok(()), "nothing holds it"),
            ((0, 0, 1), held(Fit), "a fit"),
            ((0, 1, 0), held(ControlDisplay), "a control display"),
            (
                (0, 1, 1),
                held(ControlDisplay),
                "a control display over a fit",
            ),
            ((1, 0, 0), held(Takeover), "a takeover without a viewer"),
            ((1, 0, 1), held(Takeover), "a takeover and its fit"),
            (
                (1, 1, 0),
                held(Takeover),
                "a takeover and its control display",
            ),
            (
                (1, 1, 1),
                held(Takeover),
                "a takeover, its control display and its fit",
            ),
            ((0, 2, 3), held(ControlDisplay), "the counts read as any"),
        ];
        for ((takeover, control, fit), expected, case) in cases {
            let facts = ScreenFacts {
                takeover,
                control,
                fit,
            };
            assert_eq!(decide(facts), expected, "{case}");
        }
    }

    #[test]
    fn the_refusal_names_its_holder() {
        assert_eq!(
            OwnerHasScreen {
                by: ScreenHolder::Fit
            }
            .to_string(),
            "The owner has the screen (a fit). Nothing was done. Wait for them to give it back."
        );
    }
}
