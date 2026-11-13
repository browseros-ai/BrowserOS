//! One private entry, from the first look to the last read-back (design section 5).
//!
//! ```text
//!  admit the tab ─ hold its input (until the end) ─ read its address once
//!  look at every box ───────────────── a refusal: refused, not_sent, nothing changed
//!  send_effect: env.recheck, env.mark_sent (once) ─ mark the tab ─ let go of held modifiers
//!  for each box:
//!    env.recheck and a second look (from the second box on)
//!    focus { text } ─ the box marked, its listener armed, focused, selected
//!       noDocumentFocus: Shift on the box's session, then focused again ─ else not_focused
//!    one insertion:  Input.insertText            ┐ on the frame's own session for a box
//!    key by key:     per key: env.recheck,       │ in a cross-site frame, else the tab's
//!                    focused, the key            ┘ root session
//!                    before the last: holds(all but the last) = seen
//!    readBack ─ holds: entered │ other: value_not_taken │ left or gone: seen and the listener
//!  ForgetGuards drop the page-side copy of each value however the entry ends
//! ```
//!
//! The answer keeps the SDK's meanings: `refused` when no character went anywhere, `partial`
//! when the boxes before `at` hold their values and box `at` got none, `uncertain` when box
//! `at` got some; the delivery is the door's derivation (`entry.rs`).

use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use browseros_core::{ProtocolSession, TabId};
use neo_trace::ErrorOwner;
use tokio::runtime::Handle;

use crate::input::{self, Keyboard, PageInput};
use crate::op_scope::{OpScope, Sent};
use crate::page::OpPage;
use crate::private_entry::look_op::{self, Look, LookedBox, MainNavigation, TabAddress};
use crate::private_entry::{
    BoxState, Entry, EntryCode, EntryError, EntryOutcome, FocusState, LookField, PageSecret,
    PrivateAnswer, PrivateField, PrivateStep, Sending, entry,
};
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest};
use crate::world::{IsolatedWorld, Pin};
use crate::{ErrorCode, Playwright, PlaywrightError, Refusal};

/// How a whole entry that did not stop ended.
#[derive(Clone, Copy)]
enum Ended {
    /// Every box holds its value.
    Entered,
    /// The last box's value went in, and the page moved after it.
    EnteredPageMoved,
}

/// How one box's entry ended.
enum BoxEnd {
    /// The box holds its value.
    Entered,
    /// Neo saw the value go into the box, then the box or its document went.
    Moved,
}

/// Why the entry stopped.
struct Stop {
    error: EntryError,
}

/// Where the entry stopped, and why.
struct Stopped {
    /// The index of the field it stopped at.
    at: usize,
    stop: Stop,
}

impl Stopped {
    fn at(at: usize) -> impl FnOnce(Stop) -> Self {
        move |stop| Self { at, stop }
    }
}

/// Where one box is typed: its frame's world, its pin there, the session its keys go out on,
/// and whether that is a cross-site frame's own session.
struct BoxAt<'a> {
    world: &'a IsolatedWorld,
    pin: Pin,
    session: &'a ProtocolSession,
    own_frame: bool,
}

/// What a box held after a key meant to empty it.
#[derive(Clone, Copy)]
struct Emptied {
    empty: bool,
    focused: bool,
}

/// One entry's running state: what it sent, and how many characters each box got.
pub(crate) struct EntryRun<'a> {
    pw: &'a Playwright,
    scope: &'a OpScope<'a>,
    sending: Sending,
    typed: Vec<usize>,
    /// The main frame's navigations since admission, raced with every look and the read-back.
    navigation: Option<MainNavigation>,
}

impl<'a> EntryRun<'a> {
    pub(crate) fn new(pw: &'a Playwright, scope: &'a OpScope<'a>, fields: usize) -> Self {
        Self {
            pw,
            scope,
            sending: Sending::NotYet,
            typed: vec![0; fields],
            navigation: None,
        }
    }

    /// The whole entry. Never fails: every way it ends is an [`Entry`].
    pub(crate) async fn enter(
        mut self,
        tab: TabId,
        url: Option<&str>,
        fields: &[PrivateField],
    ) -> Entry {
        match self.run(tab, url, fields).await {
            Ok(ended) => self.ended(ended),
            Err(Stopped { at, stop }) => self.stopped(at, stop),
        }
    }

    async fn run(
        &mut self,
        tab: TabId,
        url: Option<&str>,
        fields: &[PrivateField],
    ) -> Result<Ended, Stopped> {
        let scope = self.scope;
        // A host policy's refusal at admission passes through with its own owner and code.
        let page = look_op::within(scope, OpPage::admit(self.pw, scope, tab))
            .await
            .map_err(|code| Stopped::at(0)(self.stop(code)))?
            .map_err(|error| Stopped::at(0)(self.refused(&error)))?;
        self.navigation = Some(MainNavigation::watch(self.pw, &page));
        // From the first look to the last read-back no other op's keys reach the tab.
        let mut input = look_op::within(scope, input::hold(scope, &page))
            .await
            .map_err(|code| Stopped::at(0)(self.stop(code)))?
            .map_err(|error| Stopped::at(0)(self.refused(&error)))?;

        // Every box before any character: a page that replaced the password box is refused
        // before the username is typed.
        for (at, field) in fields.iter().enumerate() {
            self.look(&page, field, url, EntryCode::PageChanged)
                .await
                .map_err(Stopped::at(at))?;
        }

        let sent = scope
            .send_effect()
            .map_err(|error| Stopped::at(0)(self.refused(&error)))?;
        self.sending = Sending::Sent;
        self.pw.private_tabs().mark(tab);
        self.clean_keyboard(&sent, &mut input)
            .await
            .map_err(Stopped::at(0))?;

        let last = fields.len().saturating_sub(1);
        for (at, field) in fields.iter().enumerate() {
            // The host's rules, the tab's address and the box, again, right before the box: the
            // looks of the boxes after it scrolled the page, and typing the boxes before it gave
            // the page time to change.
            scope
                .recheck()
                .map_err(|error| Stopped::at(at)(self.refused(&error)))?;
            // A navigation that starts now leaves with the boxes before this one typed.
            let moved = if at == 0 {
                EntryCode::PageChanged
            } else {
                EntryCode::PageMoved
            };
            let looked = self
                .look(&page, field, url, moved)
                .await
                .map_err(Stopped::at(at))?;
            match self
                .type_box(&sent, &page, &mut input, at, field, looked)
                .await
            {
                Ok(BoxEnd::Entered) => {}
                Ok(BoxEnd::Moved) if at == last => return Ok(Ended::EnteredPageMoved),
                // The page moved before the rest of the form was typed.
                Ok(BoxEnd::Moved) => return Err(Stopped::at(at)(self.stop(EntryCode::PageMoved))),
                Err(stop) => return Err(Stopped { at, stop }),
            }
        }
        Ok(Ended::Entered)
    }

    /// One look at `field`'s box, with the tab's address read for it: a box that fails a check
    /// stops the entry there, and a main-frame navigation that starts meanwhile stops it with
    /// `moved`.
    async fn look(
        &mut self,
        page: &OpPage<'_>,
        field: &PrivateField,
        url: Option<&str>,
        moved: EntryCode,
    ) -> Result<LookedBox, Stop> {
        let scope = self.scope;
        let length = u32::try_from(field.value.chars().map(char::len_utf16).sum::<usize>())
            .unwrap_or(u32::MAX);
        let looking = async {
            let address = TabAddress::read(page, scope, url).await?;
            look_op::look_at(
                page,
                scope,
                &LookField::from(&field.spec),
                field.spec.cut,
                Some(length),
                address,
            )
            .await
        };
        let look = match self.navigation.as_mut() {
            Some(navigation) => navigation.race(looking).await,
            None => Some(looking.await),
        };
        match look {
            None => Err(self.stop(moved)),
            Some(Ok(Look::Found(found))) => match found.refusal {
                None => Ok(*found),
                Some(code) => Err(self.stop(code)),
            },
            Some(Err(code) | Ok(Look::Unfound(code))) => Err(self.stop(code)),
        }
    }

    /// Lets go of every key a script left held on the tab: a held Shift would type other
    /// characters.
    async fn clean_keyboard(&self, sent: &Sent<'_>, input: &mut PageInput) -> Result<(), Stop> {
        let mut planned = input.plan();
        let steps = planned.keyboard.release_all();
        if steps.is_empty() {
            return Ok(());
        }
        input
            .send_planned(sent, self.scope, planned, steps)
            .await
            .map_err(|failure| self.stop(burst_code(&failure)))
    }

    /// One box: the focus step, the typing, the read-back.
    async fn type_box(
        &mut self,
        sent: &Sent<'_>,
        page: &OpPage<'_>,
        input: &mut PageInput,
        at: usize,
        field: &PrivateField,
        looked: LookedBox,
    ) -> Result<BoxEnd, Stop> {
        let world = looked.site.world().clone();
        let pin = looked.pin;
        // A box in a cross-site frame is typed on that frame's own session; any other box on
        // the tab's root session, which every input uses.
        let root = page.root_session();
        let own_frame = world.session_id() != root.session_id();
        let session = if own_frame {
            world.session().clone()
        } else {
            root
        };
        let fits = looked
            .facts
            .max_length
            .filter(|_| field.spec.cut && cuts(&looked.facts.kind))
            .and_then(|max| usize::try_from(max).ok());
        let text = PageSecret::fitting(&field.value, fits);
        let spaced = field.spec.spaced;
        let guard = ForgetGuard::new(&world, pin);

        let box_at = BoxAt {
            world: &world,
            pin,
            session: &session,
            own_frame,
        };
        self.focus_step(sent, input, &box_at, &text, spaced).await?;
        // A box with no size that already holds text is emptied by a key before the first
        // character (ruling R51): the browser does not count a selection in such a box against
        // its maxlength, so with a full box the first key empties it and its own character is
        // dropped (measured: 246813 left 46813). A sized box and an empty one need no key: the
        // focus step's selection is replaced by the first character, and a page that submits on
        // its first input event must never see an empty box (B11v).
        if !looked.facts.sized && !looked.facts.empty {
            self.empty_box(sent, input, &box_at, &text, spaced).await?;
        }

        let seen = if field.spec.keys {
            self.keys(sent, input, &session, &world, pin, at, &text, spaced)
                .await?
        } else {
            self.insert(sent, input, &session, at, &text).await?;
            // The box held the focus in the check right before the one insertion.
            true
        };
        self.read_back(&world, pin, seen, guard).await
    }

    /// The focus step (design section 5.4): marks the box, arms its listener, focuses it and
    /// selects what it holds; then the focus must be the box's. A Shift press types nothing and
    /// has no default action; its arrival gives the frame's document the focus and runs the
    /// site's focus handlers. A box in a cross-site frame always gets one, even when the first
    /// check passed (ruling R40): on a tab in front the frame's document can hold the focus at
    /// that check while the top page's theft is still on its way, and only input arriving in the
    /// frame lets it land before the text does (measured: 60 of 60 entries refused with nothing
    /// typed with the press, 9 of 10 sent without it).
    async fn focus_step(
        &self,
        sent: &Sent<'_>,
        input: &mut PageInput,
        box_at: &BoxAt<'_>,
        text: &PageSecret,
        spaced: bool,
    ) -> Result<(), Stop> {
        let focus = PrivateStep::Focus {
            text: text.clone(),
            spaced,
        };
        match self.step(box_at.world, box_at.pin, focus).await? {
            Some(PrivateAnswer::Focus {
                focus: FocusState::Held,
            }) if !box_at.own_frame => Ok(()),
            Some(PrivateAnswer::Focus {
                focus: FocusState::Held | FocusState::NoDocumentFocus,
            }) => {
                self.press(sent, input, box_at.session, "Shift").await?;
                if self.focused(box_at.world, box_at.pin).await? == FocusState::Held {
                    Ok(())
                } else {
                    Err(self.stop(EntryCode::NotFocused))
                }
            }
            _ => Err(self.stop(EntryCode::NotFocused)),
        }
    }

    /// Empties a box with no size that holds text, before its first character (ruling R51):
    /// Delete over the focus step's selection, on the session the text will use (R24), then the
    /// box must be empty and hold the focus. A page may handle one key and not the other (the
    /// first host's digit boxes move the focus back on Backspace), so Backspace is the second
    /// try, after a fresh focus step. A box still not empty, or one that will not keep the focus,
    /// stops the entry before any character: `box_not_cleared`, or `not_focused`.
    async fn empty_box(
        &self,
        sent: &Sent<'_>,
        input: &mut PageInput,
        box_at: &BoxAt<'_>,
        text: &PageSecret,
        spaced: bool,
    ) -> Result<(), Stop> {
        for key in ["Delete", "Backspace"] {
            if key == "Backspace" {
                self.focus_step(sent, input, box_at, text, spaced).await?;
            }
            self.scope.recheck().map_err(|error| self.refused(&error))?;
            self.press(sent, input, box_at.session, key).await?;
            let mut emptied = self.empty_and_focused(box_at).await?;
            if emptied.is_some_and(|empty| !empty.focused) {
                // The key moved the focus: focus the box again and look once more.
                self.focus_step(sent, input, box_at, text, spaced).await?;
                emptied = self.empty_and_focused(box_at).await?;
            }
            if emptied.is_some_and(|empty| empty.empty && empty.focused) {
                return Ok(());
            }
        }
        Err(self.stop(EntryCode::BoxNotCleared))
    }

    /// Whether the box holds no text and holds the focus now; `None` when it left its document.
    async fn empty_and_focused(&self, box_at: &BoxAt<'_>) -> Result<Option<Emptied>, Stop> {
        let none = PrivateStep::Holds {
            text: PageSecret::empty(),
            spaced: false,
        };
        let Some(PrivateAnswer::Holds { holds: empty }) =
            self.step(box_at.world, box_at.pin, none).await?
        else {
            return Ok(None);
        };
        let focused = self.focused(box_at.world, box_at.pin).await? == FocusState::Held;
        Ok(Some(Emptied { empty, focused }))
    }

    /// The whole text in one `Input.insertText`, as `fill` types.
    async fn insert(
        &mut self,
        sent: &Sent<'_>,
        input: &mut PageInput,
        session: &ProtocolSession,
        at: usize,
        text: &PageSecret,
    ) -> Result<(), Stop> {
        let planned = input.plan();
        let steps = Keyboard::insert_text(text.text());
        let burst = input
            .send_planned_on(sent, self.scope, planned, steps, session)
            .await;
        // On the wire once sent, whatever its reply.
        self.set_typed(at, text.char_count());
        burst.map_err(|failure| self.stop(burst_code(&failure)))
    }

    /// Key by key: before each key the host's rules and the focus, and before the last key
    /// whether the box holds all the others (`seen`).
    #[expect(
        clippy::too_many_arguments,
        reason = "one box's typing state, used once"
    )]
    async fn keys(
        &mut self,
        sent: &Sent<'_>,
        input: &mut PageInput,
        session: &ProtocolSession,
        world: &IsolatedWorld,
        pin: Pin,
        at: usize,
        text: &PageSecret,
        spaced: bool,
    ) -> Result<bool, Stop> {
        let count = text.char_count();
        // For one key, "all but the last" is empty: the focus check before it is the evidence.
        let mut seen = count == 1;
        let mut buffer = [0; 4];
        for (index, character) in text.chars().enumerate() {
            self.scope.recheck().map_err(|error| self.refused(&error))?;
            if index + 1 == count && count > 1 {
                let holds = PrivateStep::Holds {
                    text: text.all_but_last(),
                    spaced,
                };
                seen = matches!(
                    self.step(world, pin, holds).await?,
                    Some(PrivateAnswer::Holds { holds: true })
                );
            }
            match self.focused(world, pin).await? {
                FocusState::Held => {}
                _ if index == 0 => return Err(self.stop(EntryCode::NotFocused)),
                // The form left with part of the value.
                FocusState::Moving => return Err(self.stop(EntryCode::PageMoved)),
                FocusState::NoDocumentFocus | FocusState::Elsewhere | FocusState::Gone => {
                    self.mark_focused(world, pin).await;
                    return Err(self.stop(EntryCode::FocusLost));
                }
            }
            let key: &str = character.encode_utf8(&mut buffer);
            let mut planned = input.plan();
            let steps = planned.keyboard.type_text(key, Duration::ZERO);
            let burst = input
                .send_planned_on(sent, self.scope, planned, steps, session)
                .await;
            self.set_typed(at, index + 1);
            burst.map_err(|failure| self.stop(burst_code(&failure)))?;
        }
        Ok(seen)
    }

    /// The read-back (design section 5.6). Once the page answered it, the page side has
    /// forgotten the value, and the guard stands down.
    async fn read_back(
        &mut self,
        world: &IsolatedWorld,
        pin: Pin,
        seen: bool,
        guard: ForgetGuard,
    ) -> Result<BoxEnd, Stop> {
        let request = private(pin, PrivateStep::ReadBack);
        let reading = look_op::within(self.scope, probe::call(world, &request));
        let read = match self.navigation.as_mut() {
            Some(navigation) => navigation.race(reading).await,
            None => Some(reading.await),
        };
        // The main frame began to navigate after the text went out: its document is leaving,
        // as when it was replaced. The guard forgets the value if the page still answers.
        let Some(called) = read else {
            return if seen {
                Ok(BoxEnd::Moved)
            } else {
                Err(self.stop(EntryCode::PageMoved))
            };
        };
        let called = called.map_err(|code| self.stop(code))?;
        if matches!(called, Ok(ProbeReply::Value { .. })) {
            guard.disarm();
        }
        match called {
            Ok(ProbeReply::Value { value }) => match decode(value.into_value()) {
                // The box holds the value, and its page already submitted or is unloading: the
                // value went out with the form (ruling R38).
                Some(PrivateAnswer::ReadBack {
                    state: BoxState::Holds,
                    moving: true,
                    ..
                }) => Ok(BoxEnd::Moved),
                Some(PrivateAnswer::ReadBack {
                    state: BoxState::Holds,
                    moving: false,
                    ..
                }) => Ok(BoxEnd::Entered),
                // The page side marked the text control holding the focus, if it is not the box,
                // before it forgot the box (ruling R26).
                Some(PrivateAnswer::ReadBack {
                    state: BoxState::Other,
                    ..
                }) => Err(self.stop(EntryCode::ValueNotTaken)),
                // The box left a living document: its own listener says whether it held the
                // whole value at its last input event (ruling R10).
                Some(PrivateAnswer::ReadBack {
                    state: BoxState::Left,
                    held_at_last_input,
                    ..
                }) if seen && held_at_last_input => Ok(BoxEnd::Moved),
                Some(PrivateAnswer::ReadBack { .. }) => Err(self.stop(EntryCode::PageMoved)),
                _ => Err(self.stop(EntryCode::Internal)),
            },
            // The document itself was replaced: a form that submitted.
            Err(ProbeFailure::Gone(_)) if seen => Ok(BoxEnd::Moved),
            // No document change Neo can vouch for: the box was never under entry in this
            // world, so nothing says what it held.
            Err(ProbeFailure::Gone(_)) | Ok(ProbeReply::Detached) => {
                Err(self.stop(EntryCode::PageMoved))
            }
            Ok(_) => Err(self.stop(EntryCode::Internal)),
            Err(failure) => Err(self.stop(look_op::failure_code(failure))),
        }
    }

    /// Marks the text control holding the focus in the box's document, when it is not the
    /// box, so a snapshot masks it as well (ruling R26). Best effort: the entry is stopping
    /// either way, and its answer does not wait on this.
    async fn mark_focused(&self, world: &IsolatedWorld, pin: Pin) {
        let _marked = self.step(world, pin, PrivateStep::MarkFocused).await;
    }

    async fn focused(&self, world: &IsolatedWorld, pin: Pin) -> Result<FocusState, Stop> {
        Ok(match self.step(world, pin, PrivateStep::Focused).await? {
            Some(PrivateAnswer::Focus { focus }) => focus,
            _ => FocusState::Gone,
        })
    }

    /// One step on the box; `None` when the box or its document is gone.
    async fn step(
        &self,
        world: &IsolatedWorld,
        pin: Pin,
        step: PrivateStep,
    ) -> Result<Option<PrivateAnswer>, Stop> {
        let request = private(pin, step);
        let called = look_op::within(self.scope, probe::call(world, &request))
            .await
            .map_err(|code| self.stop(code))?;
        match called {
            Ok(ProbeReply::Value { value }) => decode(value.into_value())
                .map(Some)
                .ok_or_else(|| self.stop(EntryCode::Internal)),
            Ok(ProbeReply::Detached) | Err(ProbeFailure::Gone(_)) => Ok(None),
            Ok(_) => Err(self.stop(EntryCode::Internal)),
            Err(failure) => Err(self.stop(look_op::failure_code(failure))),
        }
    }

    async fn press(
        &self,
        sent: &Sent<'_>,
        input: &mut PageInput,
        session: &ProtocolSession,
        key: &str,
    ) -> Result<(), Stop> {
        let mut planned = input.plan();
        let Ok(steps) = planned.keyboard.press(key, Duration::ZERO) else {
            return Err(self.stop(EntryCode::Internal));
        };
        input
            .send_planned_on(sent, self.scope, planned, steps, session)
            .await
            .map_err(|failure| self.stop(burst_code(&failure)))
    }

    fn set_typed(&mut self, at: usize, count: usize) {
        if let Some(typed) = self.typed.get_mut(at) {
            *typed = count;
        }
    }

    /// A stop with one of Neo's own codes, as sending stands now.
    fn stop(&self, code: EntryCode) -> Stop {
        Stop {
            error: EntryError::of(code, self.sending),
        }
    }

    /// A stop for a failure of the door's rules (a lock taken, a busy page), passed through
    /// with the policy's own owner and code, or for one of the crate's own failures.
    fn refused(&self, error: &PlaywrightError) -> Stop {
        Stop {
            error: look_op::error_of(error, self.sending),
        }
    }

    /// The answer of an entry that did not stop.
    fn ended(self, ended: Ended) -> Entry {
        let outcome = match ended {
            Ended::Entered => EntryOutcome::Entered,
            Ended::EnteredPageMoved => EntryOutcome::EnteredPageMoved,
        };
        Entry {
            outcome,
            delivery: entry::delivery_of(self.sending, None),
            at: None,
            typed: self.typed,
            error: None,
        }
    }

    /// The answer of an entry that stopped at box `at`: `uncertain` when that box got
    /// characters or the page moved under it, `refused` when no box got any, else `partial`.
    fn stopped(self, at: usize, stop: Stop) -> Entry {
        let Self { sending, typed, .. } = self;
        let Stop { error } = stop;
        let in_box = typed.get(at).copied().unwrap_or(0);
        let outcome = if in_box > 0 || error.code == EntryCode::PageMoved.as_str() {
            EntryOutcome::Uncertain
        } else if typed.iter().all(|count| *count == 0) {
            EntryOutcome::Refused
        } else {
            EntryOutcome::Partial
        };
        Entry {
            outcome,
            delivery: entry::delivery_of(sending, Some(&error)),
            at: Some(at),
            typed,
            error: Some(error),
        }
    }
}

/// The input types a browser applies `maxlength` to (ruling R23): a cut applies to these only.
fn cuts(kind: &str) -> bool {
    matches!(
        kind,
        "text" | "search" | "url" | "tel" | "email" | "password"
    )
}

fn private(pin: Pin, step: PrivateStep) -> ProbeRequest {
    ProbeRequest::Private { pin, step }
}

fn decode(value: serde_json::Value) -> Option<PrivateAnswer> {
    serde_json::from_value(value).ok()
}

fn burst_code(failure: &input::BurstFailure) -> EntryCode {
    match failure {
        input::BurstFailure::Expired => EntryCode::RunTimeout,
        input::BurstFailure::Cdp(error) => match ErrorCode::from_core(error) {
            ErrorCode::TargetClosed => EntryCode::TargetClosed,
            ErrorCode::BrowserDisconnected => EntryCode::BrowserDisconnected,
            _ => EntryCode::Internal,
        },
    }
}

/// Drops the page-side copy of a box's value and its listener however the entry ends: the
/// read-back forgets on the way through; any other exit (a stop, the host dropping the entry's
/// future) sends `forget` from a spawned task, since `Drop` cannot await.
struct ForgetGuard {
    armed: Option<(IsolatedWorld, Pin)>,
}

impl ForgetGuard {
    fn new(world: &IsolatedWorld, pin: Pin) -> Self {
        Self {
            armed: Some((world.clone(), pin)),
        }
    }

    /// The read-back forgot already.
    fn disarm(mut self) {
        self.armed = None;
    }
}

impl Drop for ForgetGuard {
    fn drop(&mut self) {
        let Some((world, pin)) = self.armed.take() else {
            return;
        };
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        drop(runtime.spawn(async move {
            let _forgot = probe::call(&world, &private(pin, PrivateStep::Forget)).await;
        }));
    }
}

/// The tabs whose documents took a private entry, for the host's camera (design section
/// 10.3) and a script's own pictures (ruling R46): marked before the first character, and kept
/// for the tab's life, because going back in history restores a text box. Kept in this process
/// only: a Neo restart forgets them while the browser keeps the tab, so Neo pictures the tab
/// again, a documented limit until a synced record of the tabs' targets comes (the follow-up of
/// ruling R45). The boxes' marks live in the page's isolated world and survive a restart of the
/// same Neo; an upgrade that changes the probe's protocol number loses them.
#[derive(Debug, Default)]
pub(crate) struct PrivateTabs {
    tabs: Mutex<HashSet<TabId>>,
}

impl PrivateTabs {
    pub(crate) fn mark(&self, tab: TabId) {
        self.tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(tab);
    }

    pub(crate) fn took(&self, tab: TabId) -> bool {
        self.tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&tab)
    }

    /// A script's own picture of `tab` (`page.screenshot`, `locator.screenshot`, `page.pdf`):
    /// refused for the tab's life once it took a private fill, asked before the capture and
    /// again after it, as the host's camera asks, so a capture the fill's start straddles is
    /// thrown away (ruling R46). One host-owned refusal with a fixed sentence.
    pub(crate) fn refuse_picture(&self, tab: TabId) -> Result<(), PlaywrightError> {
        if !self.took(tab) {
            return Ok(());
        }
        Err(Refusal {
            owner: ErrorOwner::Host,
            code: PRIVATE_TAB.to_owned(),
            message: PRIVATE_TAB_MESSAGE.to_owned(),
            host_code: None,
        }
        .into())
    }
}

/// The refusal of a script's picture of a tab that took a private fill (ruling R46).
const PRIVATE_TAB: &str = "private_tab";

/// The refusal's sentence, which reaches the agent's model unchanged: it says the refusal is
/// permanent for the tab and what to read instead, because a model that reads
/// `retryable: true` (nothing was sent, so a repeat is safe) may otherwise try again in a loop.
const PRIVATE_TAB_MESSAGE: &str = "This tab took a private fill, so Neo takes no \
    picture of it for the rest of the tab's life. Trying again will not succeed. Read the page \
    with page.ariaSnapshot() instead.";

#[cfg(test)]
mod tests {
    use browseros_core::TabId;
    use neo_trace::ErrorOwner;

    use super::{PRIVATE_TAB_MESSAGE, PrivateTabs};

    #[test]
    fn tests_that_a_picture_of_a_tab_that_took_a_fill_is_refused_for_good() {
        let tabs = PrivateTabs::default();
        tabs.mark(TabId(3));
        for _repeat in 0..2 {
            let refused = tabs.refuse_picture(TabId(3)).err();
            assert_eq!(
                refused
                    .as_ref()
                    .map(|error| (error.owner(), error.code(), error.message())),
                Some((ErrorOwner::Host, "private_tab", PRIVATE_TAB_MESSAGE)),
                "the same refusal every time"
            );
        }
        assert!(
            PRIVATE_TAB_MESSAGE.contains("Trying again will not succeed")
                && PRIVATE_TAB_MESSAGE.contains("page.ariaSnapshot()"),
            "the sentence says it is permanent and what to read instead"
        );
        assert!(
            tabs.refuse_picture(TabId(4)).is_ok(),
            "another tab is pictured"
        );
    }
}
