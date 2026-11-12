//! `press` and `pressSequentially` (with `type`) on an element: upstream's `_press` and
//! `_type` (server/dom.ts at v1.64.1), which focus the element and run the page's keyboard.
//!
//! ```text
//!  outer loop (locator: re-resolve │ handle: once); the element only has to be attached
//!    ① resolve { want: element }        ─ pin
//!    ── the tab's input lock ─────────────────────────────────────────────────────────────
//!    the burst, built on a copy of the tab's keyboard (an unknown key fails here, unsent)
//!    send_effect()                      ─ Sent
//!    ② edit { pin, focus { resetSelection } }   press           (detached: next pass)
//!      edit { pin, focusToType }               pressSequentially (detached: next pass │
//!                                              notFocused: fails, nothing typed)
//!    the burst on the root session; press then waits for the navigations it requested
//! ```
//!
//! Keys go to whatever has the focus. Upstream's `_type` focuses and types without looking, so
//! a page that replaces the box as it takes the focus gets the keys nowhere, and one that moves
//! the focus on (or a box that cannot take it) gets them in another element. Neo's
//! `focusToType` checks first: a deliberate deviation, on the route row. `press` keeps
//! upstream's plain focus: a key name is not text typed into an element, and pressing into the
//! page (`locator('canvas').press('ArrowLeft')`) stays as upstream has it.

use std::time::Duration;

use crate::action::requested_navigations::RequestedNavigations;
use crate::action::subject::Subject;
use crate::backoff::Backoff;
use crate::input::{self, InputDevices, InputStep, UnknownKey};
use crate::locator_handler::Prechecks;
use crate::op_scope::{OpScope, Sent};
use crate::page::OpPage;
use crate::probe::{Edit, ProbeFailure, ProbeReply, ProbeRequest, Want};
use crate::resolve::{self, Resolved};
use crate::target::{Strictness, Target};
use crate::world::Pin;
use crate::{ErrorCode, Playwright, PlaywrightError, call_log};

/// What the keyboard does once the element has the focus.
pub(crate) enum Keys<'k> {
    /// `press(key, { delay, noWaitAfter })`: one key or chord, then the navigations it
    /// requested, unless `wait_after` is false.
    Press {
        key: &'k str,
        delay: Duration,
        wait_after: bool,
    },
    /// `pressSequentially(text, { delay })` and `type`: a press per character.
    Type { text: &'k str, delay: Duration },
}

impl Keys<'_> {
    /// The burst on the tab's keyboard.
    fn burst(&self, devices: &mut InputDevices) -> Result<Vec<InputStep>, UnknownKey> {
        match self {
            Self::Press { key, delay, .. } => devices.keyboard.press(key, *delay),
            Self::Type { text, delay } => Ok(devices.keyboard.type_text(text, *delay)),
        }
    }

    /// Upstream's call-log line, `elementHandle.press("Enter")`; typed text shows as its length,
    /// `elementHandle.type(<5 characters>)` (`call_log::entered_text`), and a key name as it is.
    fn log_line(&self) -> String {
        match self {
            Self::Press { key, .. } => format!("elementHandle.press(\"{key}\")"),
            Self::Type { text, .. } => {
                format!("elementHandle.type({})", call_log::entered_text(text))
            }
        }
    }

    /// The edit that gives the element the focus before the keys.
    const fn focus_edit(&self) -> Edit {
        match self {
            Self::Press { .. } => Edit::Focus {
                reset_selection: true,
            },
            Self::Type { .. } => Edit::FocusToType,
        }
    }

    fn waits_after(&self) -> bool {
        matches!(
            self,
            Self::Press {
                wait_after: true,
                ..
            }
        )
    }
}

/// One probe call's outcome, with the op's timeout applied.
enum Called {
    Reply(ProbeReply),
    Gone,
}

/// Runs one press or typing on one locator or element handle: focuses the element and plans
/// the keys. `PageInput` sends them.
pub(crate) struct KeyRunner<'a> {
    pw: &'a Playwright,
    scope: &'a OpScope<'a>,
    page: &'a OpPage<'a>,
    subject: Subject<'a>,
    strictness: Strictness,
    /// Something matched at least once, so a timeout is `not_actionable`, not `no_match`.
    matched: bool,
}

impl<'a> KeyRunner<'a> {
    pub(crate) fn new(
        pw: &'a Playwright,
        scope: &'a OpScope<'a>,
        page: &'a OpPage<'a>,
        target: &'a Target,
        strictness: Strictness,
    ) -> Self {
        Self {
            pw,
            scope,
            page,
            subject: Subject::new(page, scope, target, strictness),
            strictness,
            matched: false,
        }
    }

    /// Focuses the element and sends `keys`, as upstream's `_press` and `_type`.
    pub(crate) async fn run(&mut self, keys: &Keys<'_>) -> Result<(), PlaywrightError> {
        self.subject.log_waiting();
        let backoff = Backoff::outer(self.scope.timeout());
        for pass in 0.. {
            if self.scope.sleep(backoff.wait(pass)).await.is_err() {
                return Err(self.timed_out());
            }
            let locating = self.subject.locate(pass, Prechecks::LocatorPasses);
            let Ok(located) = self.scope.within(locating).await else {
                return Err(self.timed_out());
            };
            let Some(located) = located? else {
                continue;
            };
            if self.on_element(&located, keys).await? {
                return Ok(());
            }
        }
        Err(self.timed_out())
    }

    /// One pass on the located element; `false` when it went away and the next pass looks
    /// again.
    async fn on_element(
        &mut self,
        located: &Resolved,
        keys: &Keys<'_>,
    ) -> Result<bool, PlaywrightError> {
        let resolve = ProbeRequest::Resolve {
            target: self.subject.first_target(),
            strictness: self.strictness,
            want: Want::Element,
        };
        let pin = match self.call(located, &resolve).await? {
            Called::Reply(ProbeReply::Element { pin, resolved, .. }) => {
                // Upstream's line for the pass that resolved the locator, before the keys'.
                if let Some(resolved) = resolved {
                    self.scope.log(resolved.log_line());
                }
                pin
            }
            Called::Reply(ProbeReply::None { stale_ref: None }) => return Ok(false),
            Called::Reply(ProbeReply::Detached) => return self.subject.detached().map(|()| false),
            Called::Reply(other) => return Err(self.subject.refused(other)),
            Called::Gone => return self.subject.lost(located).map(|()| false),
        };
        self.matched = true;
        self.scope.log(keys.log_line());

        let mut input = input::hold(self.scope, self.page).await?;
        let mut planned = input.plan();
        let steps = keys.burst(&mut planned).map_err(|unknown| {
            self.scope
                .fail(ErrorCode::InvalidInput, &unknown.to_string())
        })?;
        let sent = self.scope.send_effect()?;
        let navigations = keys.waits_after().then(|| {
            let root = self.page.root_session();
            let sessions = root
                .session_id()
                .into_iter()
                .chain(located.world().session_id())
                .cloned()
                .collect();
            RequestedNavigations::watch(self.pw.browser().cdp_events(), sessions)
        });
        if !self.focus(&sent, located, pin, keys.focus_edit()).await? {
            return Ok(false);
        }
        input
            .send_planned(&sent, self.scope, planned, steps)
            .await
            .map_err(|failure| input::burst_failed(self.scope, failure))?;
        drop(input);
        if let Some(navigations) = navigations
            && navigations.settle(self.scope).await.is_err()
        {
            return Err(self.scope.timed_out(ErrorCode::WaitTimeout));
        }
        Ok(true)
    }

    /// Upstream's `_focus(resetSelectionIfNotFocused: true)`, as `edit` (`Keys::focus_edit`);
    /// `false` once the element or its world went away. It changes the page, so it takes the
    /// op's [`Sent`].
    async fn focus(
        &self,
        _sent: &Sent<'_>,
        located: &Resolved,
        pin: Pin,
        edit: Edit,
    ) -> Result<bool, PlaywrightError> {
        match self
            .call(located, &ProbeRequest::Edit { pin, edit })
            .await?
        {
            Called::Reply(ProbeReply::Value { .. }) => Ok(true),
            Called::Reply(ProbeReply::Detached) => self.subject.detached().map(|()| false),
            Called::Reply(ProbeReply::NotFocused { holder }) => {
                Err(self.subject.not_focused(holder.as_deref()))
            }
            Called::Reply(other) => Err(self.subject.refused(other)),
            Called::Gone => self.subject.lost(located).map(|()| false),
        }
    }

    async fn call(
        &self,
        located: &Resolved,
        request: &ProbeRequest,
    ) -> Result<Called, PlaywrightError> {
        let Ok(asked) = self.scope.within(self.subject.call(located, request)).await else {
            return Err(self.timed_out());
        };
        match asked {
            Ok(reply) => Ok(Called::Reply(reply)),
            Err(ProbeFailure::Gone(_)) => Ok(Called::Gone),
            Err(failure) => Err(resolve::failed(self.scope, failure)),
        }
    }

    /// Upstream's `TimeoutError`: `no_match` while nothing matched, `not_actionable` after.
    fn timed_out(&self) -> PlaywrightError {
        self.scope.timed_out(if self.matched {
            ErrorCode::NotActionable
        } else {
            ErrorCode::NoMatch
        })
    }
}
