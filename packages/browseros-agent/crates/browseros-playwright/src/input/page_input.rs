//! A tab's input, held for one op's burst, and what a burst that stops part-way still owes.
//!
//! ```text
//!  TabInput (PageState, one per tab) ── tokio mutex, FIFO: holding it is holding the tab's input
//!  PageInput::hold ── lock ── send what an earlier burst still owes ── plan(): the burst, on a copy
//!     send_planned(&Sent, planned, steps) ── the copy becomes the tab's devices, then
//!                        ├─ Dispatch: Input.* on the tab's root session, pointer scale applied
//!                        └─ Wait: OpScope::sleep, Rust's clock
//!  dropped part-way (error, timeout, the host dropping the op)
//!     └─ the releases it owes go into TabInput.owed; a spawned flush, or the next holder,
//!        whichever takes the lock first, sends them before anything else
//! ```
//!
//! A burst is built on a copy of the devices, which advances through all of it, and the copy
//! becomes the tab's only once the burst goes out, after the op's recheck: the state is where
//! the finished burst leaves it (`input_devices.rs`), and a burst refused before it went out
//! leaves the state as it found it. If the burst stops after pressing Shift or
//! the left button, the browser would still hold them while the state says they are up, and
//! the tab's next burst would start from a lie. So a burst that stops sends the matching
//! releases from its unsent rest, and they go out before any later burst, because every
//! holder flushes them first. An event counts as sent once it is on the wire, even if its
//! reply never came, so nothing is released twice.

use std::collections::VecDeque;
use std::sync::Arc;
use std::{fmt, mem};

use browseros_core::{CoreError, ProtocolSession};
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::input_devices::{Held, Hold, InputDevices, InputEvent, InputStep};
use super::keyboard::Platform;
use super::mouse::MouseButton;
use crate::op_scope::{OpScope, Sent};

/// One tab's input state behind its input lock.
pub(crate) struct TabInput {
    devices: InputDevices,
    /// Releases a burst that stopped part-way still owes the browser, oldest first.
    owed: Vec<InputEvent>,
}

/// Events can carry typed text, which never prints (STYLE P4); the count is enough.
impl fmt::Debug for TabInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TabInput")
            .field("devices", &self.devices)
            .field("owed", &self.owed.len())
            .finish()
    }
}

impl TabInput {
    /// A tab as upstream starts it: nothing held, the mouse at (0, 0).
    pub(crate) fn new(platform: Platform) -> Self {
        Self {
            devices: InputDevices::new(platform),
            owed: Vec::new(),
        }
    }

    /// Sends what earlier bursts owe, without their waits. Every holder of the lock calls it
    /// before anything else. A tab that closed holds nothing, so a failed send is dropped with
    /// the rest.
    pub(crate) async fn flush_owed(&mut self, session: &ProtocolSession, pointer_scale: f64) {
        for event in mem::take(&mut self.owed) {
            let _released = dispatch(session, pointer_scale, &event).await;
        }
    }
}

/// The tab's input for one op. Holding it is holding the tab's input lock.
#[must_use = "dropping it frees the tab's input lock, so another op's input can land between \
              this op's events"]
pub(crate) struct PageInput {
    tab: OwnedMutexGuard<TabInput>,
    /// The tab's root session, which routes keys to the focused frame and the mouse by hit
    /// testing, out-of-process iframes included.
    session: ProtocolSession,
    /// Core's factor for this tab's root session (`BrowserView::pointer_scale`).
    pointer_scale: f64,
    /// What the current burst pressed and has not released, and its unsent rest.
    held: Vec<Held>,
    rest: VecDeque<InputStep>,
    /// The last event sent, for [`PageInput::repeat_move`].
    last: Option<InputEvent>,
}

/// Why a burst stopped part-way.
#[derive(Debug)]
pub(crate) enum BurstFailure {
    /// The op's own Playwright timeout passed.
    Expired,
    Cdp(CoreError),
}

impl PageInput {
    /// Waits for the tab's input, in arrival order, and first sends what an earlier burst
    /// owes. Those sends are best effort: they only matter while the tab lives.
    pub(crate) async fn hold(
        tab: Arc<Mutex<TabInput>>,
        session: ProtocolSession,
        pointer_scale: f64,
    ) -> Self {
        let mut tab = tab.lock_owned().await;
        tab.flush_owed(&session, pointer_scale).await;
        Self {
            tab,
            session,
            pointer_scale,
            held: Vec::new(),
            rest: VecDeque::new(),
            last: None,
        }
    }

    /// A copy of the tab's keyboard and mouse to build a burst on. The tab keeps its own state
    /// until the burst goes out ([`PageInput::send_planned`], [`PageInput::resume`]), so a
    /// burst refused before it went out (an unknown key, a copy in a sensitive field, a refused
    /// recheck) leaves nothing pressed or moved. Nothing else hands out the tab's devices.
    pub(crate) fn plan(&self) -> InputDevices {
        self.tab.devices.clone()
    }

    /// Makes `planned` the tab's devices, then sends `steps`, the burst built on them, in order.
    /// It takes the op's [`Sent`]: input goes out only after the host re-ran its policies and
    /// marked the op sent. From here on the burst counts as sent: one that stops part-way owes
    /// its releases.
    pub(crate) async fn send_planned(
        &mut self,
        sent: &Sent<'_>,
        scope: &OpScope<'_>,
        planned: InputDevices,
        steps: Vec<InputStep>,
    ) -> Result<(), BurstFailure> {
        self.tab.devices = planned;
        self.send_first(sent, scope, steps, usize::MAX).await
    }

    /// Sends the first `count` of `steps` and keeps the rest for [`PageInput::resume`]. The rest
    /// stays the burst's: dropped before it went out, it still releases what the first part
    /// pressed. The tab's devices stay as they were until the rest goes out, so a burst given up
    /// after its approach (moves, and keys its rest lets go) leaves no button the state says is
    /// down.
    pub(crate) async fn send_first(
        &mut self,
        _sent: &Sent<'_>,
        scope: &OpScope<'_>,
        steps: Vec<InputStep>,
        count: usize,
    ) -> Result<(), BurstFailure> {
        self.rest = steps.into();
        self.drain(scope, count).await
    }

    /// Makes `planned`, the devices [`PageInput::send_first`]'s burst was built on, the tab's,
    /// then sends what it kept.
    pub(crate) async fn resume(
        &mut self,
        _sent: &Sent<'_>,
        scope: &OpScope<'_>,
        planned: InputDevices,
    ) -> Result<(), BurstFailure> {
        self.tab.devices = planned;
        self.drain(scope, usize::MAX).await
    }

    /// Lets go of `button` if the tab's mouse still holds it: the release is planned on the
    /// tab's devices and owed, so it goes out once this input is dropped, before any later
    /// input. A `dragTo` press holds the left button across both halves; on an exit that never
    /// released it, this is its release.
    pub(crate) fn let_go(&mut self, button: MouseButton) {
        let tab = &mut *self.tab;
        if !tab.devices.mouse.holds(button) {
            return;
        }
        let modifiers = tab.devices.keyboard.modifiers();
        for step in tab.devices.mouse.up(button, 1, modifiers) {
            if let InputStep::Dispatch(event) = step {
                tab.owed.push(event);
            }
        }
    }

    /// Sends the last event again when it was a mouse move, outside the burst: a move holds
    /// nothing, so its repeat owes no release, and the pointer is already where it says.
    pub(crate) async fn repeat_move(
        &mut self,
        _sent: &Sent<'_>,
        scope: &OpScope<'_>,
    ) -> Result<(), BurstFailure> {
        let Some(event) = self.last.as_ref().filter(|event| event.is_mouse_move()) else {
            return Ok(());
        };
        let dispatched = dispatch(&self.session, self.pointer_scale, event);
        scope
            .within(dispatched)
            .await
            .map_err(|_expired| BurstFailure::Expired)?
            .map_err(BurstFailure::Cdp)?;
        Ok(())
    }

    async fn drain(&mut self, scope: &OpScope<'_>, count: usize) -> Result<(), BurstFailure> {
        for _ in 0..count {
            let Some(step) = self.rest.pop_front() else {
                break;
            };
            match step {
                InputStep::Wait(wait) => scope
                    .sleep(wait)
                    .await
                    .map_err(|_expired| BurstFailure::Expired)?,
                InputStep::Dispatch(event) => {
                    // Counted before the reply: the event is on the wire once sent.
                    self.note(&event);
                    let dispatched = dispatch(&self.session, self.pointer_scale, &event);
                    let replied = scope.within(dispatched).await;
                    self.last = Some(event);
                    replied
                        .map_err(|_expired| BurstFailure::Expired)?
                        .map_err(BurstFailure::Cdp)?;
                }
            }
        }
        Ok(())
    }

    fn note(&mut self, event: &InputEvent) {
        match event.hold() {
            Some(Hold::Press(held)) => self.held.push(held),
            Some(Hold::Release(held)) => {
                if let Some(at) = self.held.iter().position(|pressed| *pressed == held) {
                    self.held.remove(at);
                }
            }
            None => {}
        }
    }

    /// The releases still owed: for each held key or button, its first release in the rest.
    fn owed(&mut self) -> Vec<InputEvent> {
        let mut held = mem::take(&mut self.held);
        let mut owed = Vec::new();
        for step in mem::take(&mut self.rest) {
            if let InputStep::Dispatch(event) = step
                && let Some(Hold::Release(released)) = event.hold()
                && let Some(at) = held.iter().position(|pressed| *pressed == released)
            {
                held.remove(at);
                owed.push(event);
            }
        }
        owed
    }
}

/// A burst that stopped part-way leaves its owed releases with the tab, beside any
/// [`PageInput::let_go`] owes, then spawns a flush in case no other op comes. `Drop` cannot
/// await, and the lock is released right after this.
impl Drop for PageInput {
    fn drop(&mut self) {
        let owed = self.owed();
        self.tab.owed.extend(owed);
        if self.tab.owed.is_empty() {
            return;
        }
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        let tab = Arc::clone(OwnedMutexGuard::mutex(&self.tab));
        let session = self.session.clone();
        let scale = self.pointer_scale;
        drop(runtime.spawn(async move {
            let mut tab = tab.lock_owned().await;
            tab.flush_owed(&session, scale).await;
        }));
    }
}

async fn dispatch(
    session: &ProtocolSession,
    pointer_scale: f64,
    event: &InputEvent,
) -> Result<Value, CoreError> {
    session
        .send_value(event.method(), event.params(pointer_scale))
        .await
}

#[cfg(test)]
mod tests;
