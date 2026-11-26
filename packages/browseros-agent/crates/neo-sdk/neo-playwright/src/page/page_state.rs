//! What the crate keeps about one tab between operations.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use browseros_core::FrameId;
use tokio::time::Instant;

use tokio::sync::Mutex as AsyncMutex;

use crate::action::FocusLease;
use crate::input::{Platform, TabInput};
use crate::page::MediaState;
use crate::tree::{TabOriginState, TabTree};
use crate::world::{FrameSeq, FrameWorlds};

/// The cover of a tab's last covered answer (covered-click spec, "The same layer again"): the
/// document it stood in, its root's number there, the action that met it and when.
#[derive(Clone, Debug)]
pub(crate) struct LastCover {
    pub(crate) frame: FrameId,
    pub(crate) seq: FrameSeq,
    pub(crate) cover: u32,
    pub(crate) action: String,
    pub(crate) at: Instant,
}

/// One tab's state, shared by every op on it. Created on the first op that admits the tab,
/// and kept while the `Playwright` lives: a second one for the same tab would restart its seq
/// counter and could give a seq to two documents.
///
/// | Field | Written by | Lock |
/// |---|---|---|
/// | `frames` | installs, invalidation | std mutex inside, never across an await |
/// | `input` | one burst at a time: a click's arm, input and verdict; a key press | tokio mutex, FIFO |
/// | `focus` | the focus lease of input ops on tabs core did not create | tokio mutex, one CDP call |
/// | `screenshots` | rendering preparation and restoration | tokio mutex, FIFO |
/// | `media` | complete media updates and renderer replay | tokio mutex, across CDP writes |
/// | `tokens` | arming requests | atomic |
/// | `tree` | APC captures, the ref table, the private-node mirror | std mutexes inside, never across an await |
#[derive(Debug)]
pub(crate) struct PageState {
    frames: FrameWorlds,
    /// The tab's one APC engine state (apc-engine-design 2.3): its captures, the refs every
    /// snapshot prints and every `aria-ref` resolves, and the mirror of the host's private
    /// marks. Per tab, beside `frames`, because `DocTable` inside `frames` numbers the
    /// documents its refs are issued under.
    tree: TabTree,
    /// The tab's keyboard and mouse. Holding the lock is holding the tab's input, which keeps
    /// two ops' events from interleaving and a frame's one interceptor slot to one op.
    input: Arc<AsyncMutex<TabInput>>,
    focus: Arc<FocusLease>,
    /// Held across preparation, capture and restoration: another screenshot must never see
    /// the first screenshot's temporary styles or overwrite its animation cleanup.
    screenshots: Arc<AsyncMutex<()>>,
    /// Hit-target interceptor tokens, unique on this tab.
    tokens: AtomicU32,
    /// Page lifetime, including gaps between runs; root and iframe renderers share one setting.
    media: Arc<AsyncMutex<MediaState>>,
    /// The cover of the tab's last covered answer, across ops and runs: an agent that closed
    /// something and met the same layer again is told so.
    last_cover: Mutex<Option<LastCover>>,
}

impl PageState {
    /// A tab whose frames `frames` numbers, in a browser on `platform`. `origin` says whether
    /// the tab was open before this `Playwright` started, which decides whether its documents'
    /// private marks must be read from the page before a snapshot prints them (design 8.1).
    pub(crate) fn new(frames: FrameWorlds, platform: Platform, origin: TabOriginState) -> Self {
        Self {
            frames,
            tree: TabTree::new(origin),
            input: Arc::new(AsyncMutex::new(TabInput::new(platform))),
            focus: Arc::default(),
            screenshots: Arc::default(),
            tokens: AtomicU32::new(1),
            media: Arc::default(),
            last_cover: Mutex::default(),
        }
    }

    /// Keeps `now` as the tab's last covered answer, and returns the one before it when that
    /// named the same cover in the same document.
    pub(crate) fn covered_again(&self, now: LastCover) -> Option<LastCover> {
        let mut last = self
            .last_cover
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let same = last.as_ref().is_some_and(|before| {
            before.frame == now.frame && before.seq == now.seq && before.cover == now.cover
        });
        last.replace(now).filter(|_| same)
    }

    /// The tab's isolated worlds.
    pub(crate) fn frames(&self) -> &FrameWorlds {
        &self.frames
    }

    /// The tab's APC captures, refs and private-node mirror.
    pub(crate) fn tree(&self) -> &TabTree {
        &self.tree
    }

    /// The tab's input, for `PageInput::hold`.
    pub(crate) fn input(&self) -> &Arc<AsyncMutex<TabInput>> {
        &self.input
    }

    /// The tab's focus emulation holders.
    pub(crate) fn focus(&self) -> &Arc<FocusLease> {
        &self.focus
    }

    /// The queue for the complete screenshot lifecycle, including cancellation cleanup.
    pub(crate) fn screenshots(&self) -> &Arc<AsyncMutex<()>> {
        &self.screenshots
    }

    /// Media settings are serialized with renderer setup and cancellation restoration.
    pub(crate) fn media(&self) -> &Arc<AsyncMutex<MediaState>> {
        &self.media
    }

    /// A token no other interceptor armed on this tab has.
    pub(crate) fn next_token(&self) -> u32 {
        self.tokens.fetch_add(1, Ordering::Relaxed)
    }
}
