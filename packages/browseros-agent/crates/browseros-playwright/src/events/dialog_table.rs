//! Who answers each JavaScript dialog: a listening run, or Neo once nobody listens.
//!
//! ```text
//!  opening ─ alert ─────────────────────────────▶ Closed(Core): core already accepted it
//!          ─ confirm, prompt, beforeunload ──────▶ Open { listeners: runs listening now }
//!  Open ── dialog.accept / dialog.dismiss ───────▶ Answering: the script's send is in flight
//!       ── grace over (250 ms), nobody listening ▶ Closed(Neo): dismissed (beforeunload accepted)
//!       ── reap (1 s tick), every listener gone ─▶ Closed(Neo)
//!       ── closed in the browser, or tab gone ───▶ Closed(Browser)
//!  Answering ── the browser acknowledged it ─────▶ Closed(Script)
//!            ── the send failed, or was dropped ─▶ Open, for the grace or the reap to close
//!            ── closed in the browser ───────────▶ Closed(Browser)
//! ```
//!
//! The grace covers a `page.on('dialog')` whose listen op lands just after the click that opens
//! the dialog. The reap means a run that ends never leaves a page blocked on a dialog it held:
//! listeners are `Weak<RunState>`, which stop upgrading when the host drops the run. Both leave
//! an answering dialog to its answer, and the answer's [`DialogAnswer`] guard reopens the dialog
//! unless the browser acknowledged it, so an answer that never reached the browser cannot hide
//! the dialog from them.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError, Weak};

use browseros_core::{SessionId, TargetId};

use crate::RunState;
use crate::events::page_event::{DialogId, DialogKind};

/// How many answered dialogs the table remembers, so a second answer gets Playwright's error.
const CLOSED_KEPT: usize = 256;

/// Every dialog the watcher saw that is open, or closed recently enough to answer for.
#[derive(Debug, Default)]
pub(crate) struct DialogTable {
    next: u64,
    slots: HashMap<DialogId, Slot>,
    /// Closed dialogs, oldest first, for the bound on `slots`.
    closed: VecDeque<DialogId>,
}

#[derive(Debug)]
enum Slot {
    Open(HeldDialog),
    /// Still open in the browser, while a script's `Page.handleJavaScriptDialog` is in flight.
    Answering(HeldDialog),
    Closed(ClosedBy),
}

/// A dialog that is open in the browser and waits for an answer.
#[derive(Debug)]
pub(crate) struct HeldDialog {
    /// The session it opened on, which is the one to answer on.
    pub(crate) session: SessionId,
    /// The tab, when the watcher could tell (an unknown session has none, and nobody listens).
    pub(crate) target: Option<TargetId>,
    pub(crate) kind: DialogKind,
    /// Runs that listened for dialogs on the tab while the dialog was open. It stays open while
    /// one of them lives.
    listeners: Vec<Weak<RunState>>,
    /// The grace is over: from now on, no live listener means Neo closes it.
    grace_over: bool,
}

/// Who closed a dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClosedBy {
    /// The script, through `dialog.accept` or `dialog.dismiss`.
    Script,
    /// Neo: nobody listened within the grace, or every listening run ended.
    Neo,
    /// Core's page signals, which accept every `alert()` at once (a listed deviation).
    Core,
    /// The browser: the owner answered it, or its tab closed.
    Browser,
}

/// What Neo sends to close a dialog nobody answered.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Unanswered {
    pub(crate) session: SessionId,
    pub(crate) accept: bool,
}

/// Why a script cannot answer a dialog.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Unanswerable {
    /// Another answer to it is in flight.
    Answering,
    /// Already closed, by whom.
    Closed(ClosedBy),
    /// No such dialog, or one of another tab.
    Unknown,
}

/// A script's answer to one dialog, from the moment the table made it `Answering` until the
/// browser acknowledged it ([`DialogAnswer::answered`]).
///
/// Dropping it unanswered reopens the dialog: the send failed, the host refused it, or the op's
/// future was dropped mid-send. Without that, a dialog the browser never heard about would stay
/// marked handled, and neither the grace nor the reap would close it, leaving the page blocked.
#[derive(Debug)]
pub(crate) struct DialogAnswer<'table> {
    table: &'table Mutex<DialogTable>,
    id: DialogId,
    session: SessionId,
    answered: bool,
}

impl HeldDialog {
    /// A dialog held from the moment it opened, for the runs listening then.
    pub(crate) fn new(
        session: SessionId,
        target: Option<TargetId>,
        kind: DialogKind,
        listeners: Vec<Weak<RunState>>,
    ) -> Self {
        Self {
            session,
            target,
            kind,
            listeners,
            grace_over: false,
        }
    }

    fn has_live_listener(&self) -> bool {
        self.listeners
            .iter()
            .any(|listener| listener.strong_count() > 0)
    }

    fn unanswered(&self) -> Unanswered {
        Unanswered {
            session: self.session.clone(),
            accept: self.kind.accept_when_unanswered(),
        }
    }
}

impl DialogTable {
    /// A fresh id for the next dialog.
    pub(crate) fn allocate(&mut self) -> DialogId {
        self.next += 1;
        DialogId(self.next)
    }

    /// Holds `dialog` under `id` until someone answers it.
    pub(crate) fn hold(&mut self, id: DialogId, dialog: HeldDialog) {
        self.slots.insert(id, Slot::Open(dialog));
    }

    /// Remembers `id` as closed by `by`.
    pub(crate) fn close(&mut self, id: DialogId, by: ClosedBy) {
        self.slots.insert(id, Slot::Closed(by));
        self.closed.push_back(id);
        while self.closed.len() > CLOSED_KEPT {
            if let Some(oldest) = self.closed.pop_front() {
                self.slots.remove(&oldest);
            }
        }
    }

    /// The grace of `id` is over; `listening` are the runs listening for dialogs on its tab now.
    /// Returns what to send when nobody listens.
    pub(crate) fn end_grace(
        &mut self,
        id: DialogId,
        listening: Vec<Weak<RunState>>,
    ) -> Option<Unanswered> {
        let (dialog, answering) = match self.slots.get_mut(&id) {
            Some(Slot::Open(dialog)) => (dialog, false),
            Some(Slot::Answering(dialog)) => (dialog, true),
            Some(Slot::Closed(_)) | None => return None,
        };
        // Recorded even while an answer is in flight: should it fail, the reap is all that is
        // left to close the dialog, and the reap skips one still in its grace.
        dialog.grace_over = true;
        dialog.listeners.extend(listening);
        if answering || dialog.has_live_listener() {
            return None;
        }
        let unanswered = dialog.unanswered();
        self.close(id, ClosedBy::Neo);
        Some(unanswered)
    }

    /// The reap tick: closes every open dialog past its grace whose listeners are all gone.
    pub(crate) fn reap(&mut self) -> Vec<Unanswered> {
        let abandoned: Vec<DialogId> = self
            .slots
            .iter()
            .filter_map(|(id, slot)| match slot {
                Slot::Open(dialog) if dialog.grace_over && !dialog.has_live_listener() => Some(*id),
                Slot::Open(_) | Slot::Answering(_) | Slot::Closed(_) => None,
            })
            .collect();
        let mut unanswered = Vec::new();
        for id in abandoned {
            if let Some(Slot::Open(dialog)) = self.slots.get(&id) {
                unanswered.push(dialog.unanswered());
            }
            self.close(id, ClosedBy::Neo);
        }
        unanswered
    }

    /// Every open or answering dialog on `session` closed in the browser.
    pub(crate) fn closed_on_session(&mut self, session: &SessionId) {
        let closed: Vec<DialogId> = self
            .slots
            .iter()
            .filter_map(|(id, slot)| match slot {
                Slot::Open(dialog) | Slot::Answering(dialog) if dialog.session == *session => {
                    Some(*id)
                }
                Slot::Open(_) | Slot::Answering(_) | Slot::Closed(_) => None,
            })
            .collect();
        for id in closed {
            self.close(id, ClosedBy::Browser);
        }
    }

    /// The connection changed epoch: every session id died with the old socket, so no open
    /// dialog can be answered on the one it names any more.
    pub(crate) fn forget_sessions(&mut self) {
        let open: Vec<DialogId> = self
            .slots
            .iter()
            .filter_map(|(id, slot)| {
                matches!(slot, Slot::Open(_) | Slot::Answering(_)).then_some(*id)
            })
            .collect();
        for id in open {
            self.close(id, ClosedBy::Browser);
        }
    }

    /// `Open` to `Answering`, for the tab `target` only: the session to answer on.
    fn start_answer(&mut self, id: DialogId, target: &TargetId) -> Result<SessionId, Unanswerable> {
        let session = match self.slots.get(&id) {
            Some(Slot::Open(dialog)) if dialog.target.as_ref() == Some(target) => {
                dialog.session.clone()
            }
            Some(Slot::Answering(_)) => return Err(Unanswerable::Answering),
            Some(Slot::Closed(by)) => return Err(Unanswerable::Closed(*by)),
            Some(Slot::Open(_)) | None => return Err(Unanswerable::Unknown),
        };
        self.update(id, |slot| match slot {
            Slot::Open(dialog) => Slot::Answering(dialog),
            other @ (Slot::Answering(_) | Slot::Closed(_)) => other,
        });
        Ok(session)
    }

    /// `Answering` to `Open`. A dialog the browser closed meanwhile stays closed.
    fn reopen(&mut self, id: DialogId) {
        self.update(id, |slot| match slot {
            Slot::Answering(dialog) => Slot::Open(dialog),
            other @ (Slot::Open(_) | Slot::Closed(_)) => other,
        });
    }

    /// `Answering` to `Closed(Script)`. One the browser closed first keeps its `Browser`.
    fn finish_answer(&mut self, id: DialogId) {
        if matches!(self.slots.get(&id), Some(Slot::Answering(_))) {
            self.close(id, ClosedBy::Script);
        }
    }

    /// Replaces `id`'s slot with `next` of it, moving its held dialog between states.
    fn update(&mut self, id: DialogId, next: impl FnOnce(Slot) -> Slot) {
        if let Some(slot) = self.slots.remove(&id) {
            self.slots.insert(id, next(slot));
        }
    }
}

impl<'table> DialogAnswer<'table> {
    /// Starts the script's answer to `id` on the tab `target`, or says why it cannot.
    pub(crate) fn start(
        table: &'table Mutex<DialogTable>,
        id: DialogId,
        target: &TargetId,
    ) -> Result<Self, Unanswerable> {
        let session = lock(table).start_answer(id, target)?;
        Ok(Self {
            table,
            id,
            session,
            answered: false,
        })
    }

    /// The session the dialog opened on, which the answer is sent on.
    pub(crate) fn session(&self) -> &SessionId {
        &self.session
    }

    /// The browser acknowledged `Page.handleJavaScriptDialog`: the script closed the dialog.
    pub(crate) fn answered(mut self) {
        self.answered = true;
        lock(self.table).finish_answer(self.id);
    }
}

impl Drop for DialogAnswer<'_> {
    fn drop(&mut self) {
        if !self.answered {
            lock(self.table).reopen(self.id);
        }
    }
}

/// The table, held only for a lookup or a state change, never across an await.
pub(crate) fn lock(table: &Mutex<DialogTable>) -> MutexGuard<'_, DialogTable> {
    table.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, Weak};

    use browseros_core::{SessionId, TargetId};

    use super::{ClosedBy, DialogAnswer, DialogTable, HeldDialog, Unanswerable, Unanswered, lock};
    use crate::RunState;
    use crate::events::page_event::{DialogId, DialogKind};

    fn session() -> SessionId {
        SessionId::from("S".to_owned())
    }

    fn target() -> TargetId {
        TargetId("T".to_owned())
    }

    fn held(kind: DialogKind, listeners: Vec<Weak<RunState>>) -> HeldDialog {
        HeldDialog::new(session(), Some(target()), kind, listeners)
    }

    /// What Neo sends to dismiss a dialog nobody answered.
    fn dismissed() -> Unanswered {
        Unanswered {
            session: session(),
            accept: false,
        }
    }

    /// A table holding one dialog of `kind` for `listeners`.
    fn holding(kind: DialogKind, listeners: Vec<Weak<RunState>>) -> (Mutex<DialogTable>, DialogId) {
        let mut table = DialogTable::default();
        let id = table.allocate();
        table.hold(id, held(kind, listeners));
        (Mutex::new(table), id)
    }

    /// What a script answering `id` on the dialog's tab finds. An answer it could start drops
    /// unanswered at once, as one that never reached the browser.
    fn answer_finds(table: &Mutex<DialogTable>, id: DialogId) -> Result<SessionId, Unanswerable> {
        DialogAnswer::start(table, id, &target()).map(|answer| answer.session().clone())
    }

    #[test]
    fn tests_that_nobody_listening_closes_a_dialog_when_its_grace_ends() {
        let mut table = DialogTable::default();
        let confirm = table.allocate();
        table.hold(confirm, held(DialogKind::Confirm, Vec::new()));
        let unload = table.allocate();
        table.hold(unload, held(DialogKind::BeforeUnload, Vec::new()));
        assert_eq!(
            table.reap(),
            [],
            "the reap leaves a dialog in its grace alone"
        );
        assert_eq!(
            table.end_grace(confirm, Vec::new()),
            Some(dismissed()),
            "a confirm nobody answers is dismissed"
        );
        assert_eq!(
            table.end_grace(unload, Vec::new()),
            Some(Unanswered {
                session: session(),
                accept: true
            }),
            "a beforeunload is accepted, so the page can leave, as upstream"
        );
        assert_eq!(
            answer_finds(&Mutex::new(table), confirm),
            Err(Unanswerable::Closed(ClosedBy::Neo))
        );
    }

    #[test]
    fn tests_that_a_listener_that_arrives_in_the_grace_holds_the_dialog() {
        let run = Arc::new(RunState::default());
        let (table, id) = holding(DialogKind::Prompt, Vec::new());
        assert_eq!(lock(&table).end_grace(id, vec![Arc::downgrade(&run)]), None);
        assert_eq!(
            lock(&table).reap(),
            [],
            "held while the listening run lives"
        );
        assert_eq!(answer_finds(&table, id), Ok(session()));
        drop(run);
        assert_eq!(
            lock(&table).reap(),
            [dismissed()],
            "a run that ends leaves no dialog blocking the page"
        );
    }

    #[test]
    fn tests_that_a_dialog_is_answered_once() {
        let run = Arc::new(RunState::default());
        let (table, id) = holding(DialogKind::Confirm, vec![Arc::downgrade(&run)]);
        assert_eq!(
            DialogAnswer::start(&table, id, &TargetId("other".to_owned())).err(),
            Some(Unanswerable::Unknown),
            "another tab cannot answer it"
        );
        let Ok(answer) = DialogAnswer::start(&table, id, &target()) else {
            panic!("an open dialog takes an answer");
        };
        assert_eq!(
            answer_finds(&table, id),
            Err(Unanswerable::Answering),
            "a second answer while the first is in flight is refused"
        );
        answer.answered();
        assert_eq!(
            answer_finds(&table, id),
            Err(Unanswerable::Closed(ClosedBy::Script)),
            "an acknowledged answer closes it"
        );
        assert_eq!(lock(&table).end_grace(id, Vec::new()), None);
    }

    #[test]
    fn tests_that_an_answer_dropped_in_the_grace_leaves_the_grace_to_close_the_dialog() {
        let (table, id) = holding(DialogKind::Confirm, Vec::new());
        drop(DialogAnswer::start(&table, id, &target()));
        assert_eq!(
            lock(&table).end_grace(id, Vec::new()),
            Some(dismissed()),
            "the dropped answer reopened it, so the grace dismisses it"
        );
    }

    #[test]
    fn tests_that_an_answer_dropped_after_the_grace_leaves_the_reap_to_close_the_dialog() {
        let run = Arc::new(RunState::default());
        let (table, id) = holding(DialogKind::Confirm, vec![Arc::downgrade(&run)]);
        let answer = DialogAnswer::start(&table, id, &target());
        drop(run);
        assert_eq!(
            lock(&table).end_grace(id, Vec::new()),
            None,
            "the grace leaves it to the answer in flight"
        );
        assert_eq!(lock(&table).reap(), [], "and so does the reap");
        drop(answer);
        assert_eq!(
            lock(&table).reap(),
            [dismissed()],
            "the answer never reached the browser, so the reap dismisses it"
        );
    }

    #[test]
    fn tests_that_an_answer_that_failed_can_be_given_again() {
        let run = Arc::new(RunState::default());
        let (table, id) = holding(DialogKind::Prompt, vec![Arc::downgrade(&run)]);
        drop(DialogAnswer::start(&table, id, &target()));
        assert_eq!(
            answer_finds(&table, id),
            Ok(session()),
            "open again for the run that still holds it"
        );
    }

    #[test]
    fn tests_that_a_dialog_closed_in_the_browser_is_closed() {
        let (table, id) = holding(DialogKind::Confirm, Vec::new());
        lock(&table).closed_on_session(&session());
        assert_eq!(
            answer_finds(&table, id),
            Err(Unanswerable::Closed(ClosedBy::Browser))
        );
        assert_eq!(
            lock(&table).end_grace(id, Vec::new()),
            None,
            "nothing left to send"
        );
    }

    #[test]
    fn tests_that_a_dialog_the_browser_closed_mid_answer_stays_closed() {
        let (table, id) = holding(DialogKind::Confirm, Vec::new());
        let answer = DialogAnswer::start(&table, id, &target());
        lock(&table).closed_on_session(&session());
        drop(answer);
        assert_eq!(
            answer_finds(&table, id),
            Err(Unanswerable::Closed(ClosedBy::Browser)),
            "a dropped answer never reopens what the browser closed"
        );
        assert_eq!(
            lock(&table).end_grace(id, Vec::new()),
            None,
            "nothing left to send"
        );
    }
}
