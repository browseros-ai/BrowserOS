//! The element actions without pointer input: `focus`, `blur` and `dispatchEvent`, which wait
//! only for the element to be attached (D6), and `scrollIntoViewIfNeeded`, which waits for it
//! to be stable once scrolled.
//!
//! ```text
//!  focus, blur, dispatchEvent   outer loop: resolve { want: element } → pin
//!                                           ── the tab's input lock ──────────────────────
//!                                           send_effect() → edit { pin, … }   (detached: again)
//!  scrollIntoViewIfNeeded       outer loop, then upstream's inner retry:
//!                                           act { scroll: ifNeeded } → 16 ms → act → same rect
//! ```

use std::time::Duration;

use crate::action::subject::Subject;
use crate::backoff::Backoff;
use crate::op_scope::{OpScope, Sent};
use crate::page::OpPage;
use crate::probe::{
    Edit, ElementState, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget, Rect, Retarget,
    ScrollPlan, Want,
};
use crate::resolve::{self, Resolved};
use crate::target::{Strictness, Target};
use crate::world::Pin;
use crate::{ErrorCode, PlaywrightError, input};

/// Two measurements this far apart with the same rect make a scrolled element stable.
const STABLE_GAP: Duration = Duration::from_millis(16);

/// Runs one element action on one locator or element handle; the page applies it, and no device
/// input is sent.
pub(crate) struct ElementRunner<'a> {
    scope: &'a OpScope<'a>,
    page: &'a OpPage<'a>,
    subject: Subject<'a>,
    strictness: Strictness,
}

impl<'a> ElementRunner<'a> {
    pub(crate) fn new(
        scope: &'a OpScope<'a>,
        page: &'a OpPage<'a>,
        target: &'a Target,
        strictness: Strictness,
    ) -> Self {
        Self {
            scope,
            page,
            subject: Subject::new(page, scope, target, strictness),
            strictness,
        }
    }

    /// Resolves the element, then applies `edit` to it: upstream's `focus`, `blur` and
    /// `dispatchEvent`, retried while the element is missing or detaches in between.
    pub(crate) async fn edit(&self, edit: Edit) -> Result<(), PlaywrightError> {
        self.subject.log_waiting();
        let backoff = Backoff::outer(self.scope.timeout());
        for attempt in 0.. {
            if self.scope.sleep(backoff.wait(attempt)).await.is_err() {
                return Err(self.scope.timed_out(ErrorCode::NoMatch));
            }
            let Some(located) = self.locate(attempt).await? else {
                continue;
            };
            let resolve = ProbeRequest::Resolve {
                target: self.subject.first_target(),
                strictness: self.strictness,
                want: Want::Element,
            };
            let pin = match self.call(&located, &resolve).await? {
                Some(ProbeReply::Element { pin, .. }) => pin,
                Some(ProbeReply::None { stale_ref: None } | ProbeReply::Detached) => {
                    self.subject.detached()?;
                    continue;
                }
                None => continue,
                Some(other) => return Err(self.subject.refused(other)),
            };
            // The edit can move the focus (a focus, a blur, a page handler of the event), so it
            // waits for the tab's input, as keys and a fill hold it: between a fill's select and
            // its insertText, or between a delayed type's keys, it would send the rest of the
            // keys into another field.
            let input = input::hold(self.scope, self.page).await?;
            // The edit changes the page, so the host's policies run again right before it.
            let sent = self.scope.send_effect()?;
            let edited = self.edit_pinned(&sent, &located, pin, edit.clone()).await?;
            drop(input);
            match edited {
                Some(ProbeReply::Value { .. }) => return Ok(()),
                Some(ProbeReply::Detached) => self.subject.detached()?,
                None => {}
                // The probe threw before the edit changed anything (a bad event init).
                Some(other) => return Err(self.subject.refused(other)),
            }
        }
        Err(self.scope.timed_out(ErrorCode::NoMatch))
    }

    /// Sends `edit` to the pinned element. It changes the page, so it takes the op's [`Sent`].
    async fn edit_pinned(
        &self,
        _sent: &Sent<'_>,
        located: &Resolved,
        pin: Pin,
        edit: Edit,
    ) -> Result<Option<ProbeReply>, PlaywrightError> {
        self.call(located, &ProbeRequest::Edit { pin, edit }).await
    }

    /// `scrollIntoViewIfNeeded`, as upstream's `_waitAndScrollIntoViewIfNeeded`: scroll until
    /// the element has a box and holds still. Scrolling inside a frame scrolls every frame
    /// above it too, cross-site ones included (measured).
    pub(crate) async fn scroll_into_view(&self) -> Result<(), PlaywrightError> {
        self.scroll(&[]).await
    }

    /// Screenshot handles must be visible as well as stable before their box is captured.
    pub(crate) async fn scroll_for_screenshot(&self) -> Result<(), PlaywrightError> {
        self.scroll(&[ElementState::Visible]).await
    }

    async fn scroll(&self, required_states: &[ElementState]) -> Result<(), PlaywrightError> {
        self.subject.log_waiting();
        let outer = Backoff::outer(self.scope.timeout());
        let mut matched = false;
        let timed_out = |matched: bool| {
            self.scope.timed_out(if matched {
                ErrorCode::NotActionable
            } else {
                ErrorCode::NoMatch
            })
        };
        for attempt in 0.. {
            if self.scope.sleep(outer.wait(attempt)).await.is_err() {
                return Err(timed_out(matched));
            }
            let Some(located) = self.locate(attempt).await? else {
                continue;
            };
            let inner = Backoff::inner();
            let mut target = self.subject.first_target();
            for retry in 0.. {
                let wait = inner.wait(retry);
                self.scope.log(if retry == 0 {
                    "attempting scroll into view action".to_owned()
                } else {
                    "retrying scroll into view action".to_owned()
                });
                if retry > 0 && !wait.is_zero() {
                    self.scope.log(format!("  waiting {}ms", wait.as_millis()));
                }
                if self.scope.sleep(wait).await.is_err() {
                    return Err(timed_out(matched));
                }
                self.scope
                    .log("  waiting for element to be stable".to_owned());
                let (pin, rect) = match self
                    .measure(&located, target.clone(), true, required_states)
                    .await?
                {
                    Measured::Rect(pin, rect) => (pin, rect),
                    Measured::Hidden => {
                        matched = true;
                        self.scope.log("  element is not visible".to_owned());
                        continue;
                    }
                    Measured::Gone => {
                        self.subject.detached()?;
                        break;
                    }
                };
                matched = true;
                target = ProbeTarget::Pin(pin);
                if self.scope.sleep(STABLE_GAP).await.is_err() {
                    return Err(timed_out(matched));
                }
                match self
                    .measure(&located, target.clone(), false, required_states)
                    .await?
                {
                    Measured::Rect(_, again) if again == rect => return Ok(()),
                    Measured::Rect(..) => {
                        self.scope.log("  element is not stable".to_owned());
                    }
                    Measured::Hidden => self.scope.log("  element is not visible".to_owned()),
                    Measured::Gone => {
                        self.subject.detached()?;
                        break;
                    }
                }
            }
        }
        Err(timed_out(matched))
    }

    /// One measurement after optional scrolling. Screenshot callers additionally require
    /// visibility; ordinary scrollIntoViewIfNeeded waits only for layout and stability.
    async fn measure(
        &self,
        located: &Resolved,
        target: ProbeTarget,
        scroll: bool,
        required_states: &[ElementState],
    ) -> Result<Measured, PlaywrightError> {
        let request = ProbeRequest::Act {
            target,
            strictness: self.strictness,
            retarget: Retarget::None,
            states: required_states.to_vec(),
            frame_visible: located.frame_visible(),
            scroll: if scroll {
                ScrollPlan::IfNeeded
            } else {
                ScrollPlan::None
            },
            position: None,
            arm: None,
        };
        Ok(match self.call(located, &request).await? {
            Some(ProbeReply::Measured { pin, rect, .. }) => Measured::Rect(pin, rect),
            Some(ProbeReply::State { .. }) => Measured::Hidden,
            Some(ProbeReply::None { stale_ref: None } | ProbeReply::Detached) | None => {
                Measured::Gone
            }
            Some(other) => return Err(self.subject.refused(other)),
        })
    }

    /// One resolution on outer pass `pass`, bounded by the op's timeout. `None` while nothing
    /// matches yet.
    async fn locate(&self, pass: usize) -> Result<Option<Resolved>, PlaywrightError> {
        match self.scope.within(self.subject.locate(pass)).await {
            Ok(located) => located,
            Err(_expired) => Err(self.scope.timed_out(ErrorCode::NoMatch)),
        }
    }

    /// One probe call in the located world; `None` once that world went away, after the
    /// liveness rule forgot it.
    async fn call(
        &self,
        located: &Resolved,
        request: &ProbeRequest,
    ) -> Result<Option<ProbeReply>, PlaywrightError> {
        let Ok(asked) = self.scope.within(self.subject.call(located, request)).await else {
            return Err(self.scope.timed_out(ErrorCode::NoMatch));
        };
        match asked {
            Ok(reply) => Ok(Some(reply)),
            Err(ProbeFailure::Gone(_)) => self.subject.lost(located).map(|()| None),
            Err(failure) => Err(resolve::failed(self.scope, failure)),
        }
    }
}

/// One measurement of a scrolled element.
enum Measured {
    Rect(Pin, Rect),
    /// It has no box: upstream's `error:notvisible`.
    Hidden,
    /// It left the page, or its frame did: resolve again.
    Gone,
}
