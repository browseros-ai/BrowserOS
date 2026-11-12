//! The agent's side of Neo's one screen: whether the owner holds it, and the agent's only way to
//! move its front, for Playwright's `page.bringToFront()` (decisions 44 to 46).
//!
//! ```text
//!  door (app), admission and every recheck ── Front::check ── screen_rule::decide(the holds)
//!  op env (app) ── Front::bring_to_front(tab, task, commit):
//!    check ─ the front turn ─ check again
//!      ─ commit (the door's recheck, then mark_sent) ─ Browser.activateTab ─ followers read it
//! ```

use crate::browser_tabs::Mover;
use crate::screen_rule::{self, OwnerHasScreen};
use crate::{Displays, PartError};
use browseros_core::TabId;
use neo_trace::TaskId;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

/// The agent's side of Neo's one screen. The app's door asks [`Front::check`] about every op that
/// moves the front, at admission and at every recheck; the op moves it through
/// [`Front::bring_to_front`], which owns the order that keeps the owner's screen and the
/// front in the owner's hands.
///
/// It is a capability boundary over [`Displays`], not a pass-through: the app reaches these two
/// calls and nothing else of the displays. Cheap to clone.
#[derive(Clone)]
pub struct Front {
    displays: Arc<Displays>,
}

const _: () = {
    const fn shared_across_threads<T: Clone + Send + Sync>() {}
    shared_across_threads::<Front>();
};

/// Why the agent's move did not bring its tab forward. Delivery follows from the variant: only
/// `Activation` comes after `commit` returned Ok.
#[derive(Debug)]
pub enum FrontError<E> {
    /// The owner holds the screen. Nothing was sent, and `commit` never ran.
    OwnerHasScreen(OwnerHasScreen),
    /// `commit` refused: the door's recheck, such as a stop or a replaced connection. Nothing was
    /// sent; the door's own refusal stands as the op's error.
    Refused(E),
    /// `commit` ran, so the op is sent, and `Browser.activateTab` failed (`PartError::Browser`)
    /// or gave no answer within 5 s (`PartError::TimedOut`). The tab may or may not be in front.
    Activation(PartError),
}

impl<E: fmt::Debug> fmt::Display for FrontError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OwnerHasScreen(held) => held.fmt(formatter),
            Self::Refused(refused) => write!(formatter, "the move was refused: {refused:?}"),
            Self::Activation(error) => {
                write!(formatter, "the tab may not have come to the front: {error}")
            }
        }
    }
}

impl<E: fmt::Debug> Error for FrontError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::OwnerHasScreen(held) => Some(held),
            Self::Refused(_) => None,
            Self::Activation(error) => Some(error),
        }
    }
}

impl Front {
    /// Built by [`Displays::front`]; the service keeps one.
    pub(crate) fn new(displays: Arc<Displays>) -> Self {
        Self { displays }
    }

    /// Whether the owner holds the screen now. Sync and in memory: one short lock on the holds
    /// registry, no I/O, never awaits, so the door can ask it.
    ///
    /// # Errors
    /// `OwnerHasScreen`, naming the holder.
    pub fn check(&self) -> Result<(), OwnerHasScreen> {
        screen_rule::decide(self.displays.browser.screen_facts())
    }

    /// Brings `tab` to the front of the screen for the agent's `page.bringToFront()`. `task` is
    /// the run's task: a `followTask` view on it adopts the move. `commit` is the door's
    /// recheck, then `mark_sent`. It runs at most once, synchronously, while every other
    /// activation waits, so it must not block.
    ///
    /// The order keeps these invariants:
    /// - Nothing is refused once `commit` returned Ok, so the op's delivery, which comes from
    ///   `mark_sent`, is never `done` for a move that never happened. After it, only the
    ///   activation itself can fail.
    /// - The last check and the send happen inside one front turn. A fit's activation that
    ///   asks later lands after this move. A takeover or control display that appears after the
    ///   last check is caught by `commit` (the door's recheck reads the same holds), or finds
    ///   the move already landed: a control display's admission waits out the turn.
    /// - Bringing the front tab forward again sends the same activation. A dropped future drops
    ///   its turn, allowing the next mover through.
    ///
    /// # Errors
    /// See [`FrontError`].
    pub async fn bring_to_front<C, E>(
        &self,
        tab: TabId,
        task: Option<TaskId>,
        commit: C,
    ) -> Result<(), FrontError<E>>
    where
        C: FnOnce() -> Result<(), E> + Send,
        E: Send,
    {
        // A quick refusal before the wait; the door asked the same just before.
        self.check().map_err(FrontError::OwnerHasScreen)?;
        let turn = self.displays.browser.front_turn().await;
        // Again under the turn: a holder that began while this waited refuses it.
        self.check().map_err(FrontError::OwnerHasScreen)?;
        commit().map_err(FrontError::Refused)?;
        let activated = turn.activate(tab, Mover::Agent { task }).await;
        drop(turn);
        activated.map_err(FrontError::Activation)?;
        tracing::info!(tab = tab.0, "the agent brought a tab to the front");
        Ok(())
    }
}
