//! `fill`, `selectOption` and `selectText`: upstream's `_retryAction` edits of a form field
//! (server/dom.ts at v1.64.1), with their states, retries and call-log lines.
//!
//! ```text
//!  outer loop (locator: re-resolve │ handle: once)      fill and selectOption wait for the
//!    inner loop (the pinned element, Backoff::inner)     frames on the way to be visible
//!      ① act { chunk | pin, states }   ─ measured { pin, sensitive } │ state: retry │ detached
//!         (force: resolve { want: element }, no states)
//!      the tab's input lock ──────────────────────────────────────────────────────────────
//!      send_effect()                   ─ Sent, right before the edit
//!      ② edit { pin, fill | selectOptions | selectText }
//!         fill → needsinput: Input.insertText(value), or Delete for '' (root session)
//!         selectOptions → options not found or not enabled: retry
//! ```
//!
//! A warm fill is 3 CDP messages (①, ②, the insertText). The probe reports with ① whether the
//! field the edit changes is a password, card or one-time-code field: the element, or the
//! control its label names, as upstream's edits follow a label (the forced path too). The op's
//! target is marked sensitive right then, before the value goes anywhere, so the trace redacts
//! it. The call log names a fill's value only for a field that is not sensitive.
//! `selectOption`'s values are the options' own, page text, so they are masked with the
//! element's site before the op returns them; a sensitive select's selected values are its
//! value, and are masked whatever its sweep holds.

use std::time::Duration;

use browseros_core::sensitive_fields::FieldValues;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::action::actionability::state_name;
use crate::action::subject::Subject;
use crate::backoff::Backoff;
use crate::input::{self, Keyboard};
use crate::op_scope::{OpScope, Sent};
use crate::page::{OpPage, site_sweep};
use crate::probe::{
    Edit, ElementState, FillValue, PageText, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget,
    Retarget, ScrollPlan, SelectOption, Want,
};
use crate::resolve::{self, Resolved};
use crate::target::{Strictness, Target};
use crate::world::Pin;
use crate::{ErrorCode, PlaywrightError};

/// The mask the call log shows for a sensitive field's typed value, the seal's own.
pub(crate) const MASKED: &str = "••••";

/// One form edit and what it carries.
pub(crate) enum FormEdit<'v> {
    /// `fill(value)`, and `clear()` as `fill('')`.
    Fill(&'v FillValue),
    SelectOptions(&'v [SelectOption]),
    SelectText,
}

/// One edit's row of design D6.
struct Row {
    /// Upstream's action name in `attempting fill action`.
    name: &'static str,
    states: &'static [ElementState],
    /// `waiting for element to be ...`.
    waits_for: &'static str,
    /// Upstream's `waitForFrameVisible`: the element's states count its frames' visibility,
    /// and an element in a hidden frame waits. `selectText` passes `frameVisible: true`.
    frames: bool,
}

impl FormEdit<'_> {
    const fn row(&self) -> Row {
        use ElementState::{Editable, Enabled, Visible};
        match self {
            Self::Fill(_) => Row {
                name: "fill",
                states: &[Visible, Enabled, Editable],
                waits_for: "visible, enabled and editable",
                frames: true,
            },
            Self::SelectOptions(_) => Row {
                name: "select option",
                states: &[Visible, Enabled],
                waits_for: "visible and enabled",
                frames: true,
            },
            Self::SelectText => Row {
                name: "selectText",
                states: &[Visible],
                waits_for: "visible",
                frames: false,
            },
        }
    }
}

/// A fill's answer (protocol.ts `Filled`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
enum Filled {
    #[serde(rename = "done")]
    Done,
    #[serde(rename = "needsinput")]
    NeedsInput,
}

/// A selectOptions' answer (protocol.ts `Selected`).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Selected {
    Values(Vec<String>),
    Retry(SelectRetry),
}

/// Why upstream's selectOptions asks for another attempt.
#[derive(Clone, Copy, Debug, Deserialize)]
enum SelectRetry {
    #[serde(rename = "error:optionsnotfound")]
    OptionsNotFound,
    #[serde(rename = "error:optionnotenabled")]
    OptionNotEnabled,
}

/// Where ① left the attempt.
enum Ready {
    At {
        pin: Pin,
        sensitive: bool,
    },
    /// The first state that does not hold yet.
    Missing(ElementState),
    /// The element or its world went away: the next outer pass.
    Gone,
}

/// Where ② left the attempt.
enum Applied {
    Done(Value),
    /// Upstream retries this, with its call-log line.
    Retry(&'static str),
    Gone,
}

/// One probe call's outcome, with the op's timeout applied.
enum Called {
    Reply(ProbeReply),
    Gone,
}

/// Runs one form edit on one locator or element handle; the page applies the edit, and a fill
/// that needs typing sends it through `PageInput`.
pub(crate) struct FormRunner<'a> {
    scope: &'a OpScope<'a>,
    page: &'a OpPage<'a>,
    subject: Subject<'a>,
    strictness: Strictness,
    /// Upstream's `force`: no states, no frame visibility.
    force: bool,
    /// Something matched at least once, so a timeout is `not_actionable`, not `no_match`.
    matched: bool,
}

impl<'a> FormRunner<'a> {
    pub(crate) fn new(
        scope: &'a OpScope<'a>,
        page: &'a OpPage<'a>,
        target: &'a Target,
        strictness: Strictness,
        force: bool,
    ) -> Self {
        Self {
            scope,
            page,
            subject: Subject::new(page, scope, target, strictness),
            strictness,
            force,
            matched: false,
        }
    }

    /// Runs `edit` to completion, a definite failure, or the op's timeout. `selectOption`
    /// answers the selected values; the others null.
    pub(crate) async fn run(&mut self, edit: &FormEdit<'_>) -> Result<Value, PlaywrightError> {
        let row = edit.row();
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
            if row.frames && !self.force && !located.frame_visible() {
                self.matched = true;
                self.scope
                    .log("  element is inside a hidden frame, retrying".to_owned());
                continue;
            }
            if let Some(value) = self.on_element(&located, edit, &row).await? {
                return Ok(value);
            }
        }
        Err(self.timed_out())
    }

    /// The inner loop on one located element: upstream's `_retryAction`.
    async fn on_element(
        &mut self,
        located: &Resolved,
        edit: &FormEdit<'_>,
        row: &Row,
    ) -> Result<Option<Value>, PlaywrightError> {
        let inner = Backoff::inner();
        let mut pin = None;
        let mut announced = false;
        for retry in 0.. {
            let wait = inner.wait(retry);
            if retry == 0 {
                self.scope.log(format!("attempting {} action", row.name));
            } else {
                self.scope.log(format!("retrying {} action", row.name));
                if !wait.is_zero() {
                    self.scope.log(format!("  waiting {}ms", wait.as_millis()));
                }
            }
            if self.scope.sleep(wait).await.is_err() {
                return Err(self.timed_out());
            }
            if !self.force {
                self.scope
                    .log(format!("  waiting for element to be {}", row.waits_for));
            }
            let (pinned, sensitive) = match self.ready(located, pin, row).await? {
                Ready::At { pin, sensitive } => {
                    self.matched = true;
                    if sensitive {
                        self.scope.mark_sensitive_target();
                    }
                    if !announced {
                        announced = true;
                        self.announce(edit, sensitive);
                    }
                    (pin, sensitive)
                }
                Ready::Missing(state) => {
                    self.matched = true;
                    self.scope
                        .log(format!("  element is not {}", state_name(state)));
                    continue;
                }
                Ready::Gone => return Ok(None),
            };
            pin = Some(pinned);
            match self.apply(located, pinned, edit, sensitive).await? {
                Applied::Done(value) => return Ok(Some(value)),
                Applied::Retry(line) => self.scope.log(line.to_owned()),
                Applied::Gone => return Ok(None),
            }
        }
        Err(self.timed_out())
    }

    /// ①: the element pinned once its states hold, and whether it is a sensitive field.
    async fn ready(
        &self,
        located: &Resolved,
        pin: Option<Pin>,
        row: &Row,
    ) -> Result<Ready, PlaywrightError> {
        let target = pin.map_or_else(|| self.subject.first_target(), ProbeTarget::Pin);
        let request = if self.force {
            ProbeRequest::Resolve {
                target,
                strictness: self.strictness,
                want: Want::Element,
            }
        } else {
            ProbeRequest::Act {
                target,
                strictness: self.strictness,
                retarget: Retarget::None,
                states: row.states.to_vec(),
                frame_visible: !row.frames || located.frame_visible(),
                scroll: ScrollPlan::None,
                position: None,
                arm: None,
            }
        };
        Ok(match self.call(located, &request).await? {
            Called::Reply(
                ProbeReply::Measured { pin, sensitive, .. }
                | ProbeReply::Element { pin, sensitive, .. },
            ) => Ready::At {
                pin,
                sensitive: sensitive.is_some(),
            },
            Called::Reply(ProbeReply::State { missing }) => Ready::Missing(missing),
            Called::Reply(ProbeReply::None { stale_ref: None } | ProbeReply::Detached) => {
                self.subject.detached()?;
                Ready::Gone
            }
            Called::Reply(other) => return Err(self.subject.refused(located, other).await),
            Called::Gone => {
                self.subject.lost(located)?;
                Ready::Gone
            }
        })
    }

    /// ②: the edit itself, after `send_effect`. `sensitive` is ①'s word on the element.
    async fn apply(
        &self,
        located: &Resolved,
        pin: Pin,
        edit: &FormEdit<'_>,
        sensitive: bool,
    ) -> Result<Applied, PlaywrightError> {
        match edit {
            FormEdit::Fill(value) => self.fill(located, pin, value).await,
            FormEdit::SelectOptions(options) => {
                // Both selection edits hold the tab's input as a fill does: selectText moves the
                // focus and a select's change handlers may, and neither must land between
                // another op's keys.
                let input = input::hold(self.scope, self.page).await?;
                let sent = self.scope.send_effect()?;
                let edit = Edit::SelectOptions {
                    options: options.to_vec(),
                };
                let edited = self.edit(&sent, located, pin, edit).await?;
                drop(input);
                let Some(selected) = edited else {
                    return Ok(Applied::Gone);
                };
                let selected = self.masked(located, selected, sensitive).await?;
                Ok(match decode(self.scope, selected)? {
                    Selected::Values(values) => {
                        self.scope.log("  selected specified option(s)".to_owned());
                        Applied::Done(Value::from(values))
                    }
                    Selected::Retry(SelectRetry::OptionsNotFound) => {
                        Applied::Retry("  did not find some options")
                    }
                    Selected::Retry(SelectRetry::OptionNotEnabled) => {
                        Applied::Retry("  option being selected is not enabled")
                    }
                })
            }
            FormEdit::SelectText => {
                let _input = input::hold(self.scope, self.page).await?;
                let sent = self.scope.send_effect()?;
                Ok(
                    match self.edit(&sent, located, pin, Edit::SelectText).await? {
                        Some(_) => Applied::Done(Value::Null),
                        None => Applied::Gone,
                    },
                )
            }
        }
    }

    /// Upstream's `_fill`: the probe sets the value of an input that takes no typing, or
    /// focuses and selects the field, and the value is then typed in one `Input.insertText`
    /// (an empty one presses Delete). The tab's input lock is held from before the edit to
    /// the last key, so no other op's keys reach the field in between.
    async fn fill(
        &self,
        located: &Resolved,
        pin: Pin,
        value: &FillValue,
    ) -> Result<Applied, PlaywrightError> {
        let mut input = input::hold(self.scope, self.page).await?;
        let sent = self.scope.send_effect()?;
        let edit = Edit::Fill {
            value: value.clone(),
        };
        let Some(filled) = self.edit(&sent, located, pin, edit).await? else {
            return Ok(Applied::Gone);
        };
        if decode::<Filled>(self.scope, filled.textless())? == Filled::NeedsInput {
            let mut planned = input.plan();
            let steps = if value.0.is_empty() {
                planned
                    .keyboard
                    .press("Delete", Duration::ZERO)
                    .map_err(|error| self.scope.fail(ErrorCode::Internal, &error.to_string()))?
            } else {
                Keyboard::insert_text(&value.0)
            };
            input
                .send_planned(&sent, self.scope, planned, steps)
                .await
                .map_err(|failure| input::burst_failed(self.scope, failure))?;
        }
        Ok(Applied::Done(Value::Null))
    }

    /// Sends `edit` to the pinned element. It changes the page, so it takes the op's [`Sent`].
    /// `None` once the element or its world went away.
    async fn edit(
        &self,
        _sent: &Sent<'_>,
        located: &Resolved,
        pin: Pin,
        edit: Edit,
    ) -> Result<Option<PageText>, PlaywrightError> {
        match self
            .call(located, &ProbeRequest::Edit { pin, edit })
            .await?
        {
            Called::Reply(ProbeReply::Value { value }) => Ok(Some(value)),
            Called::Reply(ProbeReply::Detached) => self.subject.detached().map(|()| None),
            // Upstream's own refusals, thrown before the edit changed anything ("Element is
            // not a <select> element").
            Called::Reply(other) => Err(self.subject.refused(located, other).await),
            Called::Gone => self.subject.lost(located).map(|()| None),
        }
    }

    /// A `selectOptions` answer as the op may return it: masked with the element's site,
    /// swept after the edit so the sweep holds the values it set, and with a sensitive select's
    /// own selected values besides, which a multiple select's sweep holds only the first of.
    /// The edit is out, so a sweep that fails leaves the answer ambiguous.
    async fn masked(
        &self,
        located: &Resolved,
        selected: PageText,
        sensitive: bool,
    ) -> Result<Value, PlaywrightError> {
        let own = if sensitive {
            selected.field_values()
        } else {
            FieldValues::default()
        };
        let swept = site_sweep::swept_now(self.page, self.scope, located.world().clone(), own);
        let Ok(values) = self.scope.within(swept).await else {
            return Err(self.timed_out().after_input());
        };
        Ok(values.map_err(PlaywrightError::after_input)?.mask(selected))
    }

    /// Upstream's `fill("value")` line, with the value masked in a sensitive field.
    fn announce(&self, edit: &FormEdit<'_>, sensitive: bool) {
        if let FormEdit::Fill(value) = edit {
            let shown = if sensitive { MASKED } else { value.0.as_str() };
            self.scope.log(format!("  fill(\"{shown}\")"));
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

/// An edit's answer in the shape protocol.ts gives it; anything else is a stale asset.
fn decode<T: DeserializeOwned>(scope: &OpScope<'_>, value: Value) -> Result<T, PlaywrightError> {
    serde_json::from_value(value).map_err(|error| {
        scope.fail(
            ErrorCode::Internal,
            &format!("the probe answered an edit this build cannot read: {error}"),
        )
    })
}
