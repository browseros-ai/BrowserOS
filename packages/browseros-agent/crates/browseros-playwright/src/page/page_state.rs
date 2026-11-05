//! What the crate keeps about one tab between operations.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

use crate::TypedBy;
use crate::op_scope::Sent;
use crate::world::{FrameSeq, FrameWorlds};

/// How long after an agent input op ends a journal entry still counts as its typing. The
/// entry for the last key arrives after the op's last CDP reply. Kept short, because an entry
/// wrongly marked `Agent` is one the host does not register as a secret.
const TYPING_GRACE: Duration = Duration::from_millis(250);

/// One tab's state, shared by every op on it. Created on the first op that admits the tab,
/// and kept while the `Playwright` lives: a second one for the same tab would restart its seq
/// counter and could give a seq to two documents.
#[derive(Debug)]
pub(crate) struct PageState {
    frames: FrameWorlds,
    typing: Mutex<TypingWindow>,
}

/// Whether an agent input op is typing on the page, for the journal's [`TypedBy`].
#[derive(Debug, Default)]
struct TypingWindow {
    /// Input ops between `mark_sent` and their end.
    open: u32,
    /// When the last one ended.
    closed_at: Option<Instant>,
}

/// An open typing window; closing it is its `Drop`, so a cancelled op closes it too.
#[must_use = "the window closes when this is dropped"]
#[derive(Debug)]
pub(crate) struct Typing<'page> {
    page: &'page PageState,
}

impl PageState {
    /// A tab whose main frame's first document will take `first`.
    pub(crate) fn new(first: FrameSeq) -> Self {
        Self {
            frames: FrameWorlds::new(first),
            typing: Mutex::default(),
        }
    }

    /// The tab's sealed worlds.
    pub(crate) fn frames(&self) -> &FrameWorlds {
        &self.frames
    }

    /// Opens the typing window for one input op. It takes the op's [`Sent`], so only an op
    /// that was marked sent can claim the page's typing.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "fills, typing and key presses open it")
    )]
    pub(crate) fn typing(&self, _sent: &Sent<'_>) -> Typing<'_> {
        self.lock_typing().open += 1;
        Typing { page: self }
    }

    /// Who typed a value the journal reports now.
    pub(crate) fn typed_by(&self) -> TypedBy {
        let window = self.lock_typing();
        let recent = window
            .closed_at
            .is_some_and(|closed| closed.elapsed() <= TYPING_GRACE);
        if window.open > 0 || recent {
            TypedBy::Agent
        } else {
            TypedBy::NotAgent
        }
    }

    fn lock_typing(&self) -> MutexGuard<'_, TypingWindow> {
        self.typing.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Typing<'_> {
    fn drop(&mut self) {
        let mut window = self.page.lock_typing();
        window.open = window.open.saturating_sub(1);
        window.closed_at = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Duration;

    use tokio::time;

    use super::{PageState, TYPING_GRACE};
    use crate::TypedBy;
    use crate::op::Op;
    use crate::op_scope::OpScope;
    use crate::target::CallHeader;
    use crate::test_env::TestEnv;
    use crate::world::FrameSeq;

    #[tokio::test(start_paused = true)]
    async fn tests_that_the_typing_window_covers_the_op_and_a_short_grace()
    -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let op = Op::from_name("locator.fill").ok_or("locator.fill is an op")?;
        let scope = OpScope::new(&env, op, CallHeader::default());
        let page = PageState::new(FrameSeq(0));
        assert_eq!(page.typed_by(), TypedBy::NotAgent, "nobody typed yet");
        let sent = scope.send_effect()?;
        let typing = page.typing(&sent);
        assert_eq!(page.typed_by(), TypedBy::Agent, "the op is typing");
        drop(typing);
        assert_eq!(
            page.typed_by(),
            TypedBy::Agent,
            "its last entry may still arrive"
        );
        time::advance(TYPING_GRACE + Duration::from_millis(1)).await;
        assert_eq!(
            page.typed_by(),
            TypedBy::NotAgent,
            "after the grace, an entry is the owner's or the page's"
        );
        Ok(())
    }
}
