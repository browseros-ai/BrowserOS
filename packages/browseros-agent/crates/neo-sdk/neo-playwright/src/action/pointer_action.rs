//! Upstream's pointer actions, driven from Rust: `_retryWithProgressIfNotConnected` (the outer
//! loop, server/frames.ts) around `_retryPointerAction` and `_performPointerAction` (the inner
//! loop, server/dom.ts) at v1.64.1, with their call-log lines. `action.rs` has the diagram.
//!
//! Three invariants carry the design:
//! - **Input after `Sent`.** Each attempt calls `OpScope::send_effect` right before its first
//!   input event and never earlier, so the host re-runs its policies before every attempt
//!   that sends, and a refused recheck sends nothing. Only [`PageInput`] dispatches, and each
//!   of its sends takes the `Sent`.
//! - **Nothing left behind.** The interceptor is disarmed by its [`ArmGuard`] and held keys
//!   and buttons are released by [`PageInput`] on every exit, the host dropping the future
//!   included; `dragTo`'s button, held between its halves, by its [`Drag`]; the focus lease is
//!   restored by its guard in `action_op.rs`.
//! - **No button before the route.** In an out-of-process frame no button goes down until the
//!   frame saw the attempt's move arrive ([`PointerRunner::route`]), `force` or not. The browser
//!   routes root-session input by its own copy of the layout, which trails a scroll in another
//!   process.
//!
//! The page text an attempt reports, a strict violation's previews and the element a hit test
//! found in the way, is masked with the site of the frame it came from (`site_sweep.rs`), swept
//! only when a reply carries such text, so a click that lands sends no sweep. The engine cuts
//! that text in the page first, so the mask goes there too: every request asks through
//! [`Subject::call`] (and the verdict and parents' hit tests through `resolve::ask`), and a
//! reply that would hold cut text answers `needsMask` and is asked again with the site's values.

use std::sync::Arc;
use std::time::Duration;

use browseros_core::SessionId;
use browseros_core::sensitive_fields::FieldValues;
use browseros_core::viewport::ViewportMode;
use serde::Deserialize;

use crate::action::actionability::{PointerKind, Requirements, requirements, state_name};
use crate::action::arm_guard::ArmGuard;
use crate::action::frame_projection::{self, Projection};
use crate::action::requested_navigations::RequestedNavigations;
use crate::action::subject::Subject;
use crate::backoff::Backoff;
use crate::input::{
    self, BurstFailure, ClickOptions, InputDevices, InputStep, Modifiers, MouseButton, PageInput,
    Point as PagePoint, UnknownKey,
};
use crate::op_scope::{OpScope, Sent};
use crate::page::{OpPage, site_sweep};
use crate::probe::{
    ArmPlan, Hit, HitAction, PageText, Point, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget,
    Read, Rect, ScrollPlan, Want,
};
use crate::resolve::{self, Resolved};
use crate::target::{Strictness, Target};
use crate::world::{Pin, SealedWorld};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// Two measurements at least this far apart, with the same rect, make an element stable. One
/// animation frame, as upstream's `stableRafCount: 1` waits, but on Rust's clock.
const STABLE_GAP: Duration = Duration::from_millis(16);

/// The route check's tries before the attempt starts over, and the gap between them: the
/// browser's copy of the layout moves on when the scrolled frame's process draws again, a
/// frame or two after the scroll (measured: the release of a burst whose press went astray
/// reached the right frame 4 ms later).
const ROUTE_CHECKS: usize = 8;
const ROUTE_GAP: Duration = Duration::from_millis(16);

/// How long an interceptor outlives the latest moment its op can still send an event: the op's
/// timeout, or the waits of its burst for an op without one. The probe's expiry only frees a
/// page whose Rust lost it (a dropped connection); one that fired while the burst still ran
/// would let its later events through unintercepted, and a `trial` would click.
const ARM_MARGIN: Duration = Duration::from_secs(3);

/// The longest delay a page's `setTimeout` keeps; a longer one fires at once.
const MAX_TIMER_MS: u64 = 2_147_483_647;

/// Upstream's scroll alignment per attempt: the protocol's if-needed first, then bottom,
/// center and top, which gets an element out from under sticky headers and footers.
const SCROLL_ROTATION: [ScrollPlan; 4] = [
    ScrollPlan::IfNeeded,
    ScrollPlan::End,
    ScrollPlan::Center,
    ScrollPlan::Start,
];

/// One pointer action: which of upstream's actions it is and the input it plans.
#[derive(Clone, Debug)]
pub(crate) struct PointerAction {
    pub(crate) kind: PointerKind,
    pub(crate) perform: Perform,
    pub(crate) wait_after: WaitAfter,
    /// Skips the states, stability and every hit test, as upstream's `force`.
    pub(crate) force: bool,
    /// Sends the input with the interceptor blocking all of it, as upstream's `trial`.
    pub(crate) trial: bool,
    /// The point inside the element's padding box; its middle when `None`.
    pub(crate) position: Option<Point>,
    /// Upstream's `modifiers` option: held for the input, as names (`ControlOrMeta` resolves
    /// by the browser's platform). `None` leaves the keyboard as it is.
    pub(crate) modifiers: Option<Vec<&'static str>>,
    /// `false` is 1.64.1's `scroll: 'none'`.
    pub(crate) scroll: bool,
}

/// `dragTo`'s two pointer actions, in upstream's order.
#[derive(Clone, Debug)]
pub(crate) struct DragPhases {
    /// Upstream's `move and down` on the dragged element: a move, then the left button down.
    pub(crate) down: PointerAction,
    /// Upstream's `move and up` on the drop target: a move in `steps`, then the button up.
    pub(crate) up: PointerAction,
}

/// What the input does at the element's point.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Perform {
    /// `mouse.click`: a move, then a press and a release per click.
    Click(ClickOptions),
    /// `mouse.move` in one step.
    Move,
    /// `dragTo`'s first half: a move, then the left button down and held.
    MoveAndDown,
    /// `dragTo`'s second half: a move in `steps`, then the left button up.
    MoveAndUp { steps: u32 },
}

/// What a pointer action waits for once its input is out, upstream's `waitAfter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WaitAfter {
    /// `click`: its verdict, then every navigation its input requested.
    Navigations,
    /// Upstream's `"disabled"` (dblclick, hover, check, drag): its verdict only.
    Verdict,
    /// `click({ noWaitAfter: true })`, upstream's `false`: neither. The interceptor is still
    /// stopped, and a miss is not retried.
    Nothing,
}

/// Where the route check left the attempt.
enum Route {
    /// The element's frame saw the move: its buttons may follow.
    Reached,
    /// It never did within [`ROUTE_CHECKS`]: measure again.
    Lost,
    /// The element's world went away under the check.
    Reresolve,
}

/// Why the inner loop handed control back.
enum Inner {
    Done,
    /// The element went away, or its frame did: resolve the locator again.
    Reresolve,
}

/// A `checkable` read's value (protocol.ts `Checkable`): upstream's `elementState(el,
/// 'checked')`, which follows a label to its control.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkable {
    checked: bool,
    radio: bool,
}

/// One probe call's outcome, with the op's timeout and the liveness rule applied.
enum Called {
    Reply(ProbeReply),
    /// The world went away under the call.
    Gone,
    Expired,
}

/// `dragTo` from before its press to after its release: the tab's input, held across both
/// halves so no other op's input comes between them, and the left button let go on every exit
/// that did not release it (a drop target never found, a timeout, a refused recheck, the host
/// dropping the op), so the tab's next input does not start from a button the page still holds.
/// Upstream leaves it down. `Drop` cannot await: the release is owed, and goes out before any
/// later input (`PageInput`).
struct Drag {
    input: PageInput,
    /// A `trial` drag's press interceptor, which blocks every event in the dragged element's
    /// frame. Kept unread until the drag ends, and dropped after `input`, so a release owed at
    /// the press reaches the page blocked: the trial clicks nothing. Its disarm takes the lock
    /// and sends what is owed first (`ArmGuard`).
    trial_press: Option<ArmGuard>,
}

impl Drop for Drag {
    fn drop(&mut self) {
        self.input.let_go(MouseButton::Left);
    }
}

/// Runs one pointer action on one locator or element handle: finds the element, waits until it
/// is ready, retries, and plans the mouse input. [`PageInput`] sends it.
pub(crate) struct PointerRunner<'a> {
    pw: &'a Playwright,
    scope: &'a OpScope<'a>,
    page: &'a OpPage<'a>,
    subject: Subject<'a>,
    strictness: Strictness,
    /// Something matched at least once, so a timeout is `not_actionable`, not `no_match`.
    matched: bool,
}

impl<'a> PointerRunner<'a> {
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

    /// Runs `action` to completion, a definite failure, or the op's timeout.
    pub(crate) async fn run(&mut self, action: &PointerAction) -> Result<(), PlaywrightError> {
        self.run_holding(action, None).await
    }

    /// `dragTo`: `phases.down` on this runner's element, then `phases.up` on `to`'s, both under
    /// one [`Drag`].
    pub(crate) async fn drag_to(
        mut self,
        to: &'a Target,
        phases: &DragPhases,
    ) -> Result<(), PlaywrightError> {
        let mut drag = Drag {
            input: self.hold().await?,
            trial_press: None,
        };
        self.run_holding(&phases.down, Some(&mut drag)).await?;
        PointerRunner::new(self.pw, self.scope, self.page, to, self.strictness)
            .run_holding(&phases.up, Some(&mut drag))
            .await
    }

    /// [`PointerRunner::run`], every attempt under `drag`'s hold of the tab's input when there
    /// is one, and under a lock of its own otherwise.
    async fn run_holding(
        &mut self,
        action: &PointerAction,
        mut drag: Option<&mut Drag>,
    ) -> Result<(), PlaywrightError> {
        self.subject.log_waiting();
        let backoff = Backoff::outer(self.scope.timeout());
        for attempt in 0.. {
            let Some(located) = self.next_location(backoff, attempt, action.force).await? else {
                continue;
            };
            match self
                .on_element(&located, None, action, drag.as_deref_mut())
                .await?
            {
                Inner::Done => return Ok(()),
                Inner::Reresolve => {}
            }
        }
        Err(self.timed_out())
    }

    /// `check`, `uncheck` and `setChecked`, as upstream's `_setChecked`: read the state, click
    /// only if it differs, then verify the click changed it. A state that is already right
    /// sends nothing, and neither does unchecking a checked radio, which no click can do.
    pub(crate) async fn set_checked(
        &mut self,
        checked: bool,
        action: &PointerAction,
    ) -> Result<(), PlaywrightError> {
        self.subject.log_waiting();
        let backoff = Backoff::outer(self.scope.timeout());
        for attempt in 0.. {
            let Some(located) = self.next_location(backoff, attempt, action.force).await? else {
                continue;
            };
            let Some(pin) = self.pin(&located).await? else {
                continue;
            };
            let Some(before) = self.checkable(&located, pin).await? else {
                continue;
            };
            if before.checked == checked {
                return Ok(());
            }
            if !checked && before.radio {
                return Err(self.scope.fail(
                    ErrorCode::PageError,
                    "Cannot uncheck radio button. Radio buttons can only be unchecked by \
                     selecting another radio button in the same group.",
                ));
            }
            match self.on_element(&located, Some(pin), action, None).await? {
                Inner::Done => {}
                Inner::Reresolve => continue,
            }
            if action.trial {
                return Ok(());
            }
            return match self.checkable(&located, pin).await? {
                Some(after) if after.checked == checked => Ok(()),
                Some(_) => Err(self.scope.fail(
                    ErrorCode::PageError,
                    "Clicking the checkbox did not change its state",
                )),
                None => Err(self.scope.fail(
                    ErrorCode::ElementDetached,
                    "Element is not attached to the DOM",
                )),
            };
        }
        Err(self.timed_out())
    }

    /// The outer loop's wait before attempt `attempt`, then one resolution. `None` while the
    /// locator matches nothing yet, or its element sits in a hidden frame.
    async fn next_location(
        &mut self,
        backoff: Backoff,
        attempt: usize,
        force: bool,
    ) -> Result<Option<Resolved>, PlaywrightError> {
        if self.scope.sleep(backoff.wait(attempt)).await.is_err() {
            return Err(self.timed_out());
        }
        let Ok(located) = self.scope.within(self.subject.locate(attempt)).await else {
            return Err(self.timed_out());
        };
        let Some(located) = located? else {
            return Ok(None);
        };
        if !force && !located.frame_visible() {
            self.matched = true;
            self.scope
                .log("  element is inside a hidden frame, retrying".to_owned());
            return Ok(None);
        }
        Ok(Some(located))
    }

    /// Pins the locator's element for `check`, which reads it before it clicks it.
    async fn pin(&mut self, located: &Resolved) -> Result<Option<Pin>, PlaywrightError> {
        let request = ProbeRequest::Resolve {
            target: self.subject.first_target(),
            strictness: self.strictness,
            want: Want::Element,
        };
        match self.call(located, &request).await? {
            Called::Reply(ProbeReply::Element { pin, .. }) => {
                self.matched = true;
                Ok(Some(pin))
            }
            Called::Reply(ProbeReply::None { stale_ref: None } | ProbeReply::Detached) => Ok(None),
            Called::Reply(other) => Err(self.subject.refused(located, other).await),
            Called::Gone => self.subject.lost(located).map(|()| None),
            Called::Expired => Err(self.timed_out()),
        }
    }

    /// Whether the pinned checkbox or radio is checked, and whether it is a radio; `None` once
    /// it left the page.
    async fn checkable(
        &mut self,
        located: &Resolved,
        pin: Pin,
    ) -> Result<Option<Checkable>, PlaywrightError> {
        let request = ProbeRequest::Read {
            target: ProbeTarget::Pin(pin),
            strictness: self.strictness,
            frame_visible: located.frame_visible(),
            read: Read::Checkable,
        };
        match self.call(located, &request).await? {
            Called::Reply(ProbeReply::Read { value, .. }) => {
                serde_json::from_value(value.textless())
                    .map(Some)
                    .map_err(|error| {
                        self.scope.fail(
                            ErrorCode::Internal,
                            &format!("a checkable read did not decode: {error}"),
                        )
                    })
            }
            Called::Reply(ProbeReply::Detached | ProbeReply::None { stale_ref: None }) => Ok(None),
            Called::Reply(other) => Err(self.subject.refused(located, other).await),
            Called::Gone => self.subject.lost(located).map(|()| None),
            Called::Expired => Err(self.timed_out()),
        }
    }

    /// The inner loop on one located element: upstream's `_retryPointerAction`.
    #[expect(
        clippy::too_many_lines,
        reason = "one attempt is upstream's _performPointerAction read top to bottom; its steps \
                  share the attempt's pin, measurement, lock and guard"
    )]
    async fn on_element(
        &mut self,
        located: &Resolved,
        mut pin: Option<Pin>,
        action: &PointerAction,
        mut drag: Option<&mut Drag>,
    ) -> Result<Inner, PlaywrightError> {
        let req = requirements(action.kind);
        let trial = if action.trial { " (trial run)" } else { "" };
        let inner = Backoff::inner();
        for retry in 0.. {
            let wait = inner.wait(retry);
            if retry == 0 {
                self.scope
                    .log(format!("attempting {} action{trial}", req.name));
            } else {
                self.scope
                    .log(format!("retrying {} action{trial}", req.name));
                if !wait.is_zero() {
                    self.scope.log(format!("  waiting {}ms", wait.as_millis()));
                }
            }
            if self.scope.sleep(wait).await.is_err() {
                return Err(self.timed_out());
            }

            // ① Resolve or reuse the pin, check the states, scroll, measure.
            if !action.force {
                self.scope
                    .log(format!("  waiting for element to be {}", req.waits_for()));
            }
            let scroll = if action.scroll {
                SCROLL_ROTATION
                    .get(retry % SCROLL_ROTATION.len())
                    .copied()
                    .unwrap_or(ScrollPlan::IfNeeded)
            } else {
                ScrollPlan::None
            };
            let target = pin.map_or_else(|| self.subject.first_target(), ProbeTarget::Pin);
            let measure = self.act_request(located, action, &req, target, scroll, None);
            let measured = match self.call(located, &measure).await? {
                Called::Reply(ProbeReply::Measured {
                    pin: pinned,
                    rect,
                    point,
                    in_view,
                    ..
                }) => {
                    self.matched = true;
                    pin = Some(pinned);
                    Measurement {
                        rect,
                        point,
                        in_view,
                    }
                }
                Called::Reply(ProbeReply::State { missing }) => {
                    self.matched = true;
                    if action.force {
                        return Err(self.forced("Element is not visible"));
                    }
                    self.scope
                        .log(format!("  element is not {}", state_name(missing)));
                    continue;
                }
                Called::Reply(ProbeReply::None { stale_ref: None } | ProbeReply::Detached) => {
                    return self.detached_from_dom();
                }
                Called::Reply(other) => return Err(self.subject.refused(located, other).await),
                Called::Gone => return self.subject.lost(located).map(|()| Inner::Reresolve),
                Called::Expired => return Err(self.timed_out()),
            };
            if !measured.in_view {
                if action.force {
                    return Err(self.forced("Element is outside of the viewport"));
                }
                self.scope
                    .log("  element is outside of the viewport".to_owned());
                continue;
            }
            if !action.force && self.scope.sleep(STABLE_GAP).await.is_err() {
                return Err(self.timed_out());
            }
            let Some(pin) = pin else {
                continue;
            };

            // ── The tab's input lock, from the parents' hit tests to the verdict, or longer
            // when the caller holds it (a drag, from its press to its release). ──
            let root = self.page.root_session();
            let root_id = root.session_id().cloned();
            let scale = self.scope.view().pointer_scale(self.page.id());
            let mut own = None;
            let input = match drag.as_deref_mut() {
                Some(drag) => &mut drag.input,
                None => own.insert(self.hold().await?),
            };
            let page_point = match self.place(located, measured.point, !action.force).await? {
                Placed::At(point) => point,
                Placed::Covered { by, world } => {
                    self.log_covered(&world, by).await?;
                    continue;
                }
                Placed::Reresolve => return Ok(Inner::Reresolve),
            };

            // The burst is built on a copy of the tab's devices, which the tab takes only as the
            // burst goes out, so a refused attempt leaves the mouse where it was.
            let mut planned = input.plan();
            let burst = burst(&mut planned, page_point, action)?;

            // ② Arm the interceptor while the element still has ①'s rect.
            let mut guard = None;
            if !action.force {
                let token = self.page.state().next_token();
                let plan = ArmPlan {
                    action: req.hit,
                    block_all: action.trial,
                    token,
                    expiry_ms: arm_expiry(self.scope.timeout(), &burst),
                    expect_rect: Some(measured.rect),
                };
                let arm = self.act_request(
                    located,
                    action,
                    &req,
                    ProbeTarget::Pin(pin),
                    ScrollPlan::None,
                    Some(plan),
                );
                match self.call(located, &arm).await? {
                    Called::Reply(ProbeReply::Armed { .. }) => {
                        guard = Some(ArmGuard::new(
                            located.world().clone(),
                            token,
                            Arc::clone(self.page.state().input()),
                            root.clone(),
                            scale,
                        ));
                    }
                    Called::Reply(ProbeReply::Measured { in_view, .. }) => {
                        self.scope.log(if in_view {
                            "  element is not stable".to_owned()
                        } else {
                            "  element is outside of the viewport".to_owned()
                        });
                        continue;
                    }
                    Called::Reply(ProbeReply::State { missing }) => {
                        self.scope
                            .log(format!("  element is not {}", state_name(missing)));
                        continue;
                    }
                    Called::Reply(ProbeReply::Covered { by }) => {
                        self.log_covered(located.world(), by).await?;
                        continue;
                    }
                    Called::Reply(ProbeReply::None { stale_ref: None } | ProbeReply::Detached) => {
                        return self.detached_from_dom();
                    }
                    Called::Reply(other) => {
                        return Err(self.subject.refused(located, other).await);
                    }
                    Called::Gone => {
                        return self.subject.lost(located).map(|()| Inner::Reresolve);
                    }
                    Called::Expired => return Err(self.timed_out()),
                }
                self.scope.log(format!("  element is {}", req.waits_for()));
            }
            self.scope.log("  scrolling into view if needed".to_owned());
            self.scope.log("  done scrolling".to_owned());
            if action.force {
                self.scope.log("  forcing action".to_owned());
            }

            // ③④⑤ The input. `send_effect` runs here, right before the first event, on every
            // attempt; a refusal returns before anything is marked or sent.
            // The frame's count of trusted moves before the approach, which the route check
            // waits to see grow. It needs no interceptor: a forced press waits for it too.
            let route = match input::approach(&burst)
                .filter(|_| self.browser_routes(located, root_id.as_ref(), req.hit))
            {
                None => None,
                Some(approach) => match self.call(located, &ProbeRequest::Moves).await? {
                    Called::Reply(ProbeReply::Moves { moves }) => Some((moves, approach)),
                    Called::Reply(other) => return Err(self.subject.refused(located, other).await),
                    Called::Gone => return self.subject.lost(located).map(|()| Inner::Reresolve),
                    Called::Expired => return Err(self.timed_out()),
                },
            };
            let sent = self.scope.send_effect()?;
            let dialogs_from = self.pw.events().head();
            let navigations = (action.wait_after == WaitAfter::Navigations).then(|| {
                let sessions = root_id
                    .iter()
                    .chain(located.world().session_id())
                    .cloned()
                    .collect();
                RequestedNavigations::watch(self.pw.browser().cdp_events(), sessions)
            });
            self.scope.log(format!("  performing {} action", req.name));
            let sending = match route {
                None => input.send_planned(&sent, self.scope, planned, burst).await,
                Some((before, approach)) => {
                    if let Err(failure) = input.send_first(&sent, self.scope, burst, approach).await
                    {
                        return Err(self.burst_failed(failure, dialogs_from));
                    }
                    match self
                        .route(located, input, &sent, before, dialogs_from)
                        .await?
                    {
                        Route::Reached => input.resume(&sent, self.scope, planned).await,
                        Route::Lost => continue,
                        Route::Reresolve => return Ok(Inner::Reresolve),
                    }
                }
            };
            if let Err(failure) = sending {
                return Err(self.burst_failed(failure, dialogs_from));
            }

            // ⑥ The verdict: did the trusted events reach the element?
            if let Some(mut guard) = guard {
                let trial_press = action.trial && matches!(action.perform, Perform::MoveAndDown);
                if action.wait_after == WaitAfter::Nothing {
                    // Upstream stops the interceptor without waiting for what it saw; the
                    // guard's disarm does the same.
                    drop(guard);
                } else if let Some(drag) = drag.as_deref_mut().filter(|_| trial_press) {
                    // Its verdict goes unread: arming already hit-tested the point, and a trial
                    // presses nothing the page sees.
                    drag.trial_press = Some(guard);
                } else {
                    let verdict = guard.verdict(self.page, self.scope);
                    let Ok(verdict) = self.scope.within(verdict).await else {
                        return Err(self.input_timed_out(dialogs_from));
                    };
                    let verdict = verdict.map_err(PlaywrightError::after_input)?;
                    // A world gone under the verdict means the input navigated or closed the
                    // frame: it landed, as upstream reads a failed `stop()`.
                    match verdict {
                        Ok(ProbeReply::Verdict { hit: Hit::Done(_) })
                        | Err(ProbeFailure::Gone(_)) => {}
                        Ok(ProbeReply::Verdict {
                            hit: Hit::Missed { by },
                        }) => {
                            self.log_covered(located.world(), by)
                                .await
                                .map_err(PlaywrightError::after_input)?;
                            continue;
                        }
                        Ok(other) => {
                            return Err(self.subject.refused(located, other).await.after_input());
                        }
                        Err(failure) => {
                            return Err(resolve::failed(self.scope, failure).after_input());
                        }
                    }
                }
            }
            drop(own);
            let done = if action.trial { "trial " } else { "" };
            self.scope.log(format!("  {done}{} action done", req.name));
            self.scope
                .log("  waiting for scheduled navigations to finish".to_owned());
            if let Some(navigations) = navigations
                && navigations.settle(self.scope).await.is_err()
            {
                return Err(self.scope.timed_out(ErrorCode::WaitTimeout));
            }
            self.scope.log("  navigations have finished".to_owned());
            return Ok(Inner::Done);
        }
        Err(self.timed_out())
    }

    /// The tab's input lock, in arrival order, within the op's timeout.
    async fn hold(&self) -> Result<PageInput, PlaywrightError> {
        let scale = self.scope.view().pointer_scale(self.page.id());
        let input = Arc::clone(self.page.state().input());
        let held = PageInput::hold(input, self.page.root_session(), scale);
        self.scope
            .within(held)
            .await
            .map_err(|_expired| self.timed_out())
    }

    /// Whether the browser, not the page, picks the frame this element's input goes to: the
    /// element's world is on another session than the tab's root, an out-of-process frame or a
    /// frame inside one. Then the route check runs, unless nothing would show the move: the
    /// mobile preview's touch emulation keeps pointer moves from the page, and a drop's moves
    /// reach it as drag events.
    fn browser_routes(&self, located: &Resolved, root: Option<&SessionId>, hit: HitAction) -> bool {
        let touch = self
            .scope
            .view()
            .current_viewport(self.page.id())
            .is_some_and(|viewport| viewport.mode() == ViewportMode::Mobile);
        located.world().session_id() != root && hit != HitAction::Drag && !touch
    }

    /// Sends the approach's last move again, [`ROUTE_GAP`] apart, until the element's frame
    /// counts more trusted moves than the `before` it had ahead of the approach. Measured on F8
    /// (an OOPIF three frames down): right after the click's own scroll moved an ancestor in
    /// another process, the browser handed the burst's first events to that ancestor, on its
    /// `<iframe>` element, and once to a button under the old layout; the interceptor saw only
    /// the release, so the verdict read done for a click that never happened. A forced click,
    /// with no interceptor, sent its press to the ancestor's button the same way.
    async fn route(
        &self,
        located: &Resolved,
        input: &mut PageInput,
        sent: &Sent<'_>,
        before: u64,
        dialogs_from: u64,
    ) -> Result<Route, PlaywrightError> {
        let request = ProbeRequest::Moves;
        for check in 0..ROUTE_CHECKS {
            if check > 0 {
                if self.scope.sleep(ROUTE_GAP).await.is_err() {
                    return Err(self.input_timed_out(dialogs_from));
                }
                if let Err(failure) = input.repeat_move(sent, self.scope).await {
                    return Err(self.burst_failed(failure, dialogs_from));
                }
            }
            let called = self
                .call(located, &request)
                .await
                .map_err(PlaywrightError::after_input)?;
            match called {
                Called::Reply(ProbeReply::Moves { moves }) if moves > before => {
                    return Ok(Route::Reached);
                }
                Called::Reply(ProbeReply::Moves { .. }) => {
                    if check == 0 {
                        self.scope.log(
                            "  waiting for the pointer to reach the element's frame".to_owned(),
                        );
                    }
                }
                Called::Reply(other) => {
                    return Err(self.subject.refused(located, other).await.after_input());
                }
                Called::Gone => return self.subject.lost(located).map(|()| Route::Reresolve),
                Called::Expired => return Err(self.input_timed_out(dialogs_from)),
            }
        }
        Ok(Route::Lost)
    }

    /// A burst that could not go out whole.
    fn burst_failed(&self, failure: BurstFailure, dialogs_from: u64) -> PlaywrightError {
        match failure {
            BurstFailure::Expired => self.input_timed_out(dialogs_from),
            BurstFailure::Cdp(error) => self.scope.core_error(&error).after_input(),
        }
    }

    /// Builds the `act` request for this action's row.
    fn act_request(
        &self,
        located: &Resolved,
        action: &PointerAction,
        req: &Requirements,
        target: ProbeTarget,
        scroll: ScrollPlan,
        arm: Option<ArmPlan>,
    ) -> ProbeRequest {
        ProbeRequest::Act {
            target,
            strictness: self.strictness,
            retarget: req.retarget,
            states: if action.force {
                Vec::new()
            } else {
                req.states.to_vec()
            },
            frame_visible: located.frame_visible(),
            scroll,
            position: action.position,
            arm,
        }
    }

    /// The element's frame-local point in the main viewport, with every parent frame's hit
    /// test unless `force`.
    async fn place(
        &mut self,
        located: &Resolved,
        point: Point,
        hit_test: bool,
    ) -> Result<Placed, PlaywrightError> {
        let projected =
            frame_projection::project(self.page, self.scope, located.path(), point, hit_test);
        let Ok(projected) = self.scope.within(projected).await else {
            return Err(self.timed_out());
        };
        match projected? {
            Projection::At(at) => {
                let at = frame_projection::round_point(at);
                Ok(Placed::At(PagePoint { x: at.x, y: at.y }))
            }
            Projection::Covered { by, world } => Ok(Placed::Covered { by, world }),
            Projection::Transformed => Err(self.scope.fail(
                ErrorCode::Unsupported,
                "an iframe on the way to the element has a CSS transform, which Neo cannot \
                 place input through yet",
            )),
            // A parent's document or its iframe went away: the frames are re-entered on the
            // next pass.
            Projection::Gone => Ok(Placed::Reresolve),
        }
    }

    /// Upstream's `<element> intercepts pointer events` line. The element is the page's own
    /// markup in `world`'s frame, the element's or a parent's, so its description is masked
    /// with that frame's site first, within the op's timeout.
    async fn log_covered(&self, world: &SealedWorld, by: PageText) -> Result<(), PlaywrightError> {
        let swept =
            site_sweep::swept_now(self.page, self.scope, world.clone(), FieldValues::default());
        let Ok(values) = self.scope.within(swept).await else {
            return Err(self.timed_out());
        };
        let by = values?.mask_line(by);
        self.scope.log(format!("  {by} intercepts pointer events"));
        Ok(())
    }

    /// One request in the located world, rooted at the handle's node when there is one.
    async fn call(
        &self,
        located: &Resolved,
        request: &ProbeRequest,
    ) -> Result<Called, PlaywrightError> {
        let Ok(asked) = self.scope.within(self.subject.call(located, request)).await else {
            return Ok(Called::Expired);
        };
        match asked? {
            Ok(reply) => Ok(Called::Reply(reply)),
            Err(ProbeFailure::Gone(_)) => Ok(Called::Gone),
            Err(failure) => Err(resolve::failed(self.scope, failure)),
        }
    }

    /// The element went away between two requests: a locator resolves again, a handle fails.
    fn detached_from_dom(&self) -> Result<Inner, PlaywrightError> {
        self.subject.detached()?;
        Ok(Inner::Reresolve)
    }

    /// Upstream's `TimeoutError`: `no_match` while nothing matched, `not_actionable` after.
    fn timed_out(&self) -> PlaywrightError {
        self.scope.timed_out(if self.matched {
            ErrorCode::NotActionable
        } else {
            ErrorCode::NoMatch
        })
    }

    /// The op's timeout passed while its input or verdict was in flight. That is ambiguous
    /// whatever else holds. A dialog the page opened meanwhile is the usual cause: Chromium
    /// acknowledges no input and answers no page call until it closes (measured), and Neo
    /// holds it for a run that listens for dialogs. The call log names it.
    fn input_timed_out(&self, dialogs_from: u64) -> PlaywrightError {
        if let Some(kind) = self
            .pw
            .events()
            .open_dialog_after(dialogs_from, self.page.target_id())
        {
            self.scope.log(format!(
                "  a {kind} dialog is open and blocks the page; accept or dismiss it in the \
                 page.on('dialog') listener"
            ));
        }
        self.scope.timed_out(ErrorCode::NotActionable)
    }

    /// Upstream's `NonRecoverableDOMError` under `force`: the element cannot take the input
    /// at all, and `force` does not wait.
    fn forced(&self, message: &str) -> PlaywrightError {
        self.scope.fail(ErrorCode::NotActionable, message)
    }
}

/// ①'s answer, in the element frame's pixels.
#[derive(Clone, Copy, Debug)]
struct Measurement {
    rect: Rect,
    point: Point,
    in_view: bool,
}

/// Where the input goes.
enum Placed {
    At(PagePoint),
    /// A parent frame's element is in the way: its description, page text of `world`.
    /// The world is boxed: it is large, and the other variants carry little.
    Covered {
        by: PageText,
        world: Box<SealedWorld>,
    },
    Reresolve,
}

/// The interceptor's expiry, in the probe's milliseconds: [`ARM_MARGIN`] past the op's
/// `timeout`, or past `burst`'s own waits when the op waits forever.
fn arm_expiry(timeout: Option<Duration>, burst: &[InputStep]) -> u64 {
    let lasts = timeout
        .unwrap_or_else(|| input::waits(burst))
        .saturating_add(ARM_MARGIN);
    u64::try_from(lasts.as_millis())
        .unwrap_or(MAX_TIMER_MS)
        .min(MAX_TIMER_MS)
}

/// The input of one attempt, built on a copy of the tab's devices (`PageInput::plan`), which
/// advances through all of it: upstream's `ensureModifiers`, the action's mouse events, and the
/// modifiers put back.
fn burst(
    devices: &mut InputDevices,
    at: PagePoint,
    action: &PointerAction,
) -> Result<Vec<InputStep>, PlaywrightError> {
    let modifier_failed = |error: UnknownKey| {
        PlaywrightError::new(ErrorCode::Internal, format!("a modifier key: {error}"))
    };
    let mut steps = Vec::new();
    let restore = match &action.modifiers {
        Some(names) => {
            let wanted = names
                .iter()
                .filter_map(|name| devices.keyboard.modifier_named(name))
                .fold(Modifiers::default(), Modifiers::with);
            let (restore, down) = devices
                .keyboard
                .ensure_modifiers(wanted)
                .map_err(modifier_failed)?;
            steps.extend(down);
            Some(restore)
        }
        None => None,
    };
    let held = devices.keyboard.modifiers();
    match action.perform {
        Perform::Click(options) => steps.extend(devices.mouse.click(at, options, held)),
        Perform::Move => steps.extend(devices.mouse.move_to(at, 1, held)),
        Perform::MoveAndDown => {
            steps.extend(devices.mouse.move_to(at, 1, held));
            steps.extend(devices.mouse.down(MouseButton::Left, 1, held));
        }
        Perform::MoveAndUp { steps: moves } => {
            steps.extend(devices.mouse.move_to(at, moves, held));
            steps.extend(devices.mouse.up(MouseButton::Left, 1, held));
        }
    }
    if let Some(restore) = restore {
        let (_, up) = devices
            .keyboard
            .ensure_modifiers(restore)
            .map_err(modifier_failed)?;
        steps.extend(up);
    }
    Ok(steps)
}

#[cfg(test)]
mod tests;
