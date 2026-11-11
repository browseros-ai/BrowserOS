//! What the crate keeps about one tab between operations.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::Mutex as AsyncMutex;

use crate::action::FocusLease;
use crate::input::{Platform, TabInput};
use crate::world::{FrameSeq, FrameWorlds};

/// One tab's state, shared by every op on it. Created on the first op that admits the tab,
/// and kept while the `Playwright` lives: a second one for the same tab would restart its seq
/// counter and could give a seq to two documents.
///
/// | Field | Written by | Lock |
/// |---|---|---|
/// | `frames` | installs, invalidation | std mutex inside, never across an await |
/// | `input` | one burst at a time: a click's arm, input and verdict; a key press | tokio mutex, FIFO |
/// | `focus` | the focus lease of input ops on tabs core did not create | tokio mutex, one CDP call |
/// | `tokens` | arming requests | atomic |
#[derive(Debug)]
pub(crate) struct PageState {
    frames: FrameWorlds,
    /// The tab's keyboard and mouse. Holding the lock is holding the tab's input, which keeps
    /// two ops' events from interleaving and a frame's one interceptor slot to one op.
    input: Arc<AsyncMutex<TabInput>>,
    focus: Arc<FocusLease>,
    /// Hit-target interceptor tokens, unique on this tab.
    tokens: AtomicU32,
}

impl PageState {
    /// A tab whose main frame's first document will take `first`, in a browser on
    /// `platform`.
    pub(crate) fn new(first: FrameSeq, platform: Platform) -> Self {
        Self {
            frames: FrameWorlds::new(first),
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
}
