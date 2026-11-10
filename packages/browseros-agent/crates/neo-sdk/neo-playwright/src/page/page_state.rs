//! What the crate keeps about one tab between operations.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;
use tokio::time::Instant;

use crate::TypedBy;
use crate::action::FocusLease;
use crate::input::{Platform, TabInput};
use crate::op_scope::Sent;
use crate::world::{FrameSeq, FrameWorlds};

/// How long after an agent input op ends a journal entry still counts as its typing. The
/// entry for the last key arrives after the op's last CDP reply. Kept short, because an entry
/// wrongly marked `Agent` is one the host does not register as a secret.
const TYPING_GRACE: Duration = Duration::from_millis(250);

/// One tab's state, shared by every op on it. Created on the first op that admits the tab,
/// and kept while the `Playwright` lives: a second one for the same tab would restart its seq
/// counter and could give a seq to two documents.
///
/// | Field | Written by | Lock |
/// |---|---|---|
/// | `frames` | installs, invalidation | std mutex inside, never across an await |
/// | `typing` | input ops, from `mark_sent` to their end | std mutex |
/// | `input` | one burst at a time: a click's arm, input and verdict; a key press | tokio mutex, FIFO |
/// | `focus` | the focus lease of input ops on tabs core did not create | tokio mutex, one CDP call |
/// | `tokens` | arming requests | atomic |
#[derive(Debug)]
pub(crate) struct PageState {
    frames: FrameWorlds,
    typing: Mutex<TypingWindow>,
    /// The tab's keyboard and mouse. Holding the lock is holding the tab's input, which keeps
    /// two ops' events from interleaving and a frame's one interceptor slot to one op.
    input: Arc<AsyncMutex<TabInput>>,
    focus: Arc<FocusLease>,
    /// Hit-target interceptor tokens, unique on this tab.
    tokens: AtomicU32,
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
    /// A tab whose main frame's first document will take `first`, in a browser on
    /// `platform`.
    pub(crate) fn new(first: FrameSeq, platform: Platform) -> Self {
        Self {
            frames: FrameWorlds::new(first),
            typing: Mutex::default(),
            input: Arc::new(AsyncMutex::new(TabInput::new(platform))),
            focus: Arc::default(),
            tokens: AtomicU32::new(1),
        }
    }

    /// The tab's sealed worlds.
    pub(crate) fn frames(&self) -> &FrameWorlds {
        &self.frames
    }

    /// The tab's input, for `PageInput::hold`.
    pub(crate) fn input(&self) -> &Arc<AsyncMutex<TabInput>> {
        &self.input
    }

    /// The tab's focus emulation holders.
    pub(crate) fn focus(&self) -> &Arc<FocusLease> {
        &self.focus
    }

    /// A token no other interceptor armed on this tab has.
    pub(crate) fn next_token(&self) -> u32 {
        self.tokens.fetch_add(1, Ordering::Relaxed)
    }

    /// Opens the typing window for one input op. It takes the op's [`Sent`], so only an op
    /// that was marked sent can claim the page's typing.
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
    use crate::input::Platform;
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
        let page = PageState::new(FrameSeq(0), Platform::Linux);
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
