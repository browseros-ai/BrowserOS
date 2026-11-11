//! `press` and `pressSequentially` (with `type`) on an element: upstream's `_press` and
//! `_type` (server/dom.ts at v1.64.1), which focus the element and run the page's keyboard.
//!
//! ```text
//!  outer loop (locator: re-resolve │ handle: once); the element only has to be attached
//!    ① resolve { want: element }        ─ pin, and whether it or the control its label names
//!                                          (which a label's focus goes to) is a sensitive field
//!    ── the tab's input lock ─────────────────────────────────────────────────────────────
//!    the burst, built on a copy of the tab's keyboard (an unknown key fails here, unsent)
//!    a copy or cut into a sensitive target: refused here, unsent
//!    send_effect()                      ─ Sent
//!    ② edit { pin, focus { resetSelection } }   (detached: next pass)
//!    a copy or cut: the focus walk, in case the focus did not move (copy_refusal.rs)
//!    the burst on the root session; press then waits for the navigations it requested
//! ```

use std::time::Duration;

use crate::action::copy_refusal::{copy_refused, focus_of, refuse_copy};
use crate::action::form_action::MASKED;
use crate::action::requested_navigations::RequestedNavigations;
use crate::action::subject::Subject;
use crate::backoff::Backoff;
use crate::input::{self, InputDevices, InputStep, UnknownKey};
use crate::op_scope::{OpScope, Sent};
use crate::page::OpPage;
use crate::probe::{Edit, ProbeFailure, ProbeReply, ProbeRequest, Want};
use crate::resolve::{self, Resolved};
use crate::target::{Strictness, Target};
use crate::world::Pin;
use crate::{ErrorCode, Playwright, PlaywrightError};

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

    /// Upstream's call-log line, `elementHandle.press("Enter")`, with the keys masked in a
    /// sensitive field.
    fn log_line(&self, sensitive: bool) -> String {
        let (method, keys) = match self {
            Self::Press { key, .. } => ("press", *key),
            Self::Type { text, .. } => ("type", *text),
        };
        let shown = if sensitive { MASKED } else { keys };
        format!("elementHandle.{method}(\"{shown}\")")
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
            let Ok(located) = self.scope.within(self.subject.locate(pass)).await else {
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
        let (pin, sensitive) = match self.call(located, &resolve).await? {
            Called::Reply(ProbeReply::Element { pin, sensitive, .. }) => (pin, sensitive.is_some()),
            Called::Reply(ProbeReply::None { stale_ref: None }) => return Ok(false),
            Called::Reply(ProbeReply::Detached) => return self.subject.detached().map(|()| false),
            Called::Reply(other) => return Err(self.subject.refused(located, other).await),
            Called::Gone => return self.subject.lost(located).map(|()| false),
        };
        self.matched = true;
        self.scope.log(keys.log_line(sensitive));

        let mut input = input::hold(self.scope, self.page).await?;
        let mut planned = input.plan();
        let steps = keys.burst(&mut planned).map_err(|unknown| {
            self.scope
                .fail(ErrorCode::InvalidInput, &unknown.to_string())
        })?;
        let copies = input::copies(&steps, planned.keyboard.platform());
        if copies && sensitive {
            return Err(copy_refused(self.scope));
        }
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
        if !self.focus(&sent, located, pin).await? {
            return Ok(false);
        }
        // Focusing an element that takes no focus leaves it where it was, perhaps in a
        // sensitive field elsewhere: a copy checks where the keys will land.
        if copies {
            let walked = self.scope.within(focus_of(self.page, self.scope));
            let focus = walked.await.map_err(|_expired| self.timed_out())??;
            refuse_copy(self.scope, focus)?;
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

    /// Upstream's `_focus(resetSelectionIfNotFocused: true)`; `false` once the element or
    /// its world went away. It changes the page, so it takes the op's [`Sent`].
    async fn focus(
        &self,
        _sent: &Sent<'_>,
        located: &Resolved,
        pin: Pin,
    ) -> Result<bool, PlaywrightError> {
        let edit = Edit::Focus {
            reset_selection: true,
        };
        match self
            .call(located, &ProbeRequest::Edit { pin, edit })
            .await?
        {
            Called::Reply(ProbeReply::Value { .. }) => Ok(true),
            Called::Reply(ProbeReply::Detached) => self.subject.detached().map(|()| false),
            Called::Reply(other) => Err(self.subject.refused(located, other).await),
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
        match asked? {
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
