//! Playwright: `server/input.ts` `Keyboard`
//! Playwright: `server/input.ts` `Mouse`
//! Playwright: `server/input.ts` `Touchscreen`
//! Neo: `page.keyboard`, `page.mouse` and `page.touchscreen`, served as Playwright's own Page
//! commands (`keyboardDown` … `touchscreenTap`) on the run's tab, over the tab's input devices.
//!
//! ```text
//!  serve(tab, call, progress)
//!   prepare the tab (the watcher answers a dialog the input opens)
//!   key calls: the browser's platform (ControlOrMeta, the mac editing commands)
//!   the tab's input turn, at once: no element to wait for
//!   plan from the tab's devices │ an unknown key → Error, nothing written
//!   nothing to write (a move of no steps, typing "") → the state changes, NotSent
//!   begin ticket → the input writer → Done │ partial → Unknown │ nothing → NotSent
//! ```
//!
//! A device call keeps what it presses: `keyboard.down` leaves its key held for later calls of
//! the run, `mouse.down` its button, and `Dispatcher::finish` releases them at the run's end.
//! None waits for a navigation the input starts, as Playwright's raw input does not.

use std::time::Duration;

use neo_runtime_spec::serve::{
    PageCall, PageMouseClickParamsButton, PageMouseDownParamsButton, PageMouseUpParamsButton,
};

use crate::click::{self, MouseButton, MouseClick, PageError, PressReport};
use crate::input_devices::{self, Devices, EffectSink, Plan};
use crate::keyboard::{self, KeyboardTab, Platform};
use crate::progress::{CallError, PlaywrightError, Progress};

/// One device call, decoded from its Page command.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DeviceCall {
    KeyboardDown {
        key: String,
    },
    KeyboardUp {
        key: String,
    },
    KeyboardPress {
        key: String,
        delay: Option<Duration>,
    },
    KeyboardType {
        text: String,
        delay: Option<Duration>,
    },
    KeyboardInsertText {
        text: String,
    },
    MouseMove {
        x: f64,
        y: f64,
        steps: u32,
    },
    MouseDown {
        button: MouseButton,
        click_count: i64,
    },
    MouseUp {
        button: MouseButton,
        click_count: i64,
    },
    MouseClick {
        x: f64,
        y: f64,
        click: MouseClick,
    },
    MouseWheel {
        delta_x: f64,
        delta_y: f64,
    },
    TouchscreenTap {
        x: f64,
        y: f64,
    },
}

/// Neo: decoding a device call from its Page command.
impl DeviceCall {
    /// The device call `call` names, or `call` back for every other Page command. A click count
    /// above `click::MAX_CLICK_COUNT` and steps above `click::MAX_STEPS` are refused before any
    /// work, as the pointer actions refuse them.
    ///
    /// # Errors
    ///
    /// `Ok(Err(_))`: the refusal; `Err(call)`: not a device call.
    pub(crate) fn from_page_call(
        call: PageCall,
    ) -> Result<Result<Self, PlaywrightError>, PageCall> {
        Ok(match call {
            PageCall::KeyboardDown(params) => Ok(Self::KeyboardDown { key: params.key }),
            PageCall::KeyboardUp(params) => Ok(Self::KeyboardUp { key: params.key }),
            PageCall::KeyboardPress(params) => Ok(Self::KeyboardPress {
                key: params.key,
                delay: click::click_delay(params.delay),
            }),
            PageCall::KeyboardType(params) => Ok(Self::KeyboardType {
                text: params.text,
                delay: click::click_delay(params.delay),
            }),
            PageCall::KeyboardInsertText(params) => {
                Ok(Self::KeyboardInsertText { text: params.text })
            }
            PageCall::MouseMove(params) => {
                click::steps_of(params.steps).map(|steps| Self::MouseMove {
                    x: params.x,
                    y: params.y,
                    steps,
                })
            }
            PageCall::MouseDown(params) => Ok(Self::MouseDown {
                button: match params.button.unwrap_or(PageMouseDownParamsButton::Left) {
                    PageMouseDownParamsButton::Left => MouseButton::Left,
                    PageMouseDownParamsButton::Right => MouseButton::Right,
                    PageMouseDownParamsButton::Middle => MouseButton::Middle,
                },
                // Playwright: `server/input.ts` `Mouse.down`: `const { button = 'left', clickCount = 1 } = options;`
                click_count: params.click_count.unwrap_or(1),
            }),
            PageCall::MouseUp(params) => Ok(Self::MouseUp {
                button: match params.button.unwrap_or(PageMouseUpParamsButton::Left) {
                    PageMouseUpParamsButton::Left => MouseButton::Left,
                    PageMouseUpParamsButton::Right => MouseButton::Right,
                    PageMouseUpParamsButton::Middle => MouseButton::Middle,
                },
                // Playwright: `server/input.ts` `Mouse.up`: `const { button = 'left', clickCount = 1 } = options;`
                click_count: params.click_count.unwrap_or(1),
            }),
            PageCall::MouseClick(params) => {
                click::click_count_of(params.click_count).map(|click_count| Self::MouseClick {
                    x: params.x,
                    y: params.y,
                    click: MouseClick {
                        button: match params.button.unwrap_or(PageMouseClickParamsButton::Left) {
                            PageMouseClickParamsButton::Left => MouseButton::Left,
                            PageMouseClickParamsButton::Right => MouseButton::Right,
                            PageMouseClickParamsButton::Middle => MouseButton::Middle,
                        },
                        click_count,
                        delay: click::click_delay(params.delay),
                    },
                })
            }
            PageCall::MouseWheel(params) => Ok(Self::MouseWheel {
                delta_x: params.delta_x,
                delta_y: params.delta_y,
            }),
            // Deviation: `server/input.ts` `Touchscreen.apiTap`: `if (!this._page.browserContext._options.hasTouch)` Neo's run attaches to the user's BrowserOS context, which has no hasTouch option, and its tab answers Input.dispatchTouchEvent with the events of a hasTouch context, so Neo taps where Playwright refuses.
            PageCall::TouchscreenTap(params) => Ok(Self::TouchscreenTap {
                x: params.x,
                y: params.y,
            }),
            other => return Err(other),
        })
    }

    /// The protocol method, for errors that name the call.
    fn method(&self) -> &'static str {
        match self {
            Self::KeyboardDown { .. } => "keyboardDown",
            Self::KeyboardUp { .. } => "keyboardUp",
            Self::KeyboardPress { .. } => "keyboardPress",
            Self::KeyboardType { .. } => "keyboardType",
            Self::KeyboardInsertText { .. } => "keyboardInsertText",
            Self::MouseMove { .. } => "mouseMove",
            Self::MouseDown { .. } => "mouseDown",
            Self::MouseUp { .. } => "mouseUp",
            Self::MouseClick { .. } => "mouseClick",
            Self::MouseWheel { .. } => "mouseWheel",
            Self::TouchscreenTap { .. } => "touchscreenTap",
        }
    }

    /// Whether the call plans keys, which need the browser's platform.
    fn types_keys(&self) -> bool {
        matches!(
            self,
            Self::KeyboardDown { .. }
                | Self::KeyboardUp { .. }
                | Self::KeyboardPress { .. }
                | Self::KeyboardType { .. }
        )
    }
}

/// Neo: serves one device call on the run's tab. `Ok(())` is Playwright's `{id}` reply; the
/// delivery is in `progress`.
pub(crate) async fn serve<T: KeyboardTab>(
    tab: &T,
    call: &DeviceCall,
    progress: &Progress,
) -> Result<(), CallError> {
    // Every device call is an input method in the spec, so dispatch always gives it a cell.
    let Some(delivery) = progress.delivery() else {
        return Err(PlaywrightError::error(format!(
            "neo-runtime: Page.{} has no delivery cell",
            call.method()
        ))
        .into());
    };
    // Domains and the watcher first: input that opens a dialog freezes the renderer until the
    // watcher answers it, and its acknowledgement never comes back before.
    progress.race(tab.prepare()).await?.map_err(page_error)?;
    let platform = if call.types_keys() {
        Some(progress.race(tab.platform()).await?.map_err(page_error)?)
    } else {
        None
    };
    let input = progress.race(tab.input()).await?;
    let plan = plan(call, &input, platform)?;
    if !plan.commands.iter().any(|command| command.method.is_some()) {
        // Nothing to write: a move of no steps still moves the pointer, as Playwright's does.
        for command in &plan.commands {
            input.apply(&command.effect, true);
        }
        return Ok(());
    }
    // The last chance to stop before an irreversible step.
    progress.check()?;
    let ticket = delivery.begin().map_err(|_| {
        PlaywrightError::error(format!(
            "neo-runtime: Page.{} tried to write its input again",
            call.method()
        ))
    })?;
    // A call that stops here drops the report's future: the writer cuts any delay short, still
    // writes every release it owes, and marks the ticket itself.
    match progress.race(tab.write(plan, ticket, input)).await? {
        PressReport::Acked(acked) => {
            acked.delivered();
            Ok(())
        }
        // Nothing was written because the session's socket was replaced, or the session went
        // mid-write: either way the tab is gone for this run.
        PressReport::NothingWritten | PressReport::Failed { closed: true, .. } => {
            Err(PlaywrightError::target_closed().into())
        }
        PressReport::Failed {
            closed: false,
            message,
        } => Err(PlaywrightError::error(message).into()),
    }
}

/// Neo: the call's input as one plan, from the tab's devices under the turn.
fn plan<D: Devices>(
    call: &DeviceCall,
    input: &D,
    platform: Option<Platform>,
) -> Result<Plan, CallError> {
    let mut simulated = input.snapshot().map_err(page_error)?;
    let owner = input.owner();
    let mut plan = Plan::default();
    // Key calls always read the platform; the others plan no key.
    let platform = platform.unwrap_or(Platform::Linux);
    let unknown = |unknown: keyboard::UnknownKey| -> CallError {
        PlaywrightError::error(unknown.to_string()).into()
    };
    match call {
        DeviceCall::KeyboardDown { key } => {
            keyboard::plan_key_down(&mut plan, &mut simulated, key, platform, &owner)
                .map_err(unknown)?;
        }
        DeviceCall::KeyboardUp { key } => {
            keyboard::plan_key_up(&mut plan, &mut simulated, key, platform, None)
                .map_err(unknown)?;
        }
        DeviceCall::KeyboardPress { key, delay } => {
            keyboard::plan_press(&mut plan, &mut simulated, key, platform, &owner, *delay)
                .map_err(unknown)?;
        }
        DeviceCall::KeyboardType { text, delay } => {
            keyboard::plan_type(&mut plan, &mut simulated, text, platform, &owner, *delay);
        }
        DeviceCall::KeyboardInsertText { text } => {
            keyboard::plan_insert_text(&mut plan, &mut simulated, text);
        }
        DeviceCall::MouseMove { x, y, steps } => {
            input_devices::plan_move(&mut plan, &mut simulated, (*x, *y), *steps);
        }
        DeviceCall::MouseDown {
            button,
            click_count,
        } => {
            input_devices::plan_down(&mut plan, &mut simulated, *button, *click_count, &owner);
        }
        DeviceCall::MouseUp {
            button,
            click_count,
        } => {
            input_devices::plan_up(&mut plan, &mut simulated, *button, *click_count, None);
        }
        DeviceCall::MouseClick { x, y, click } => {
            // Playwright: `server/input.ts` `Mouse.move`: `const { steps = 1 } = options;`
            let steps = 1;
            input_devices::plan_click(&mut plan, &mut simulated, (*x, *y), click, steps, &owner);
        }
        DeviceCall::MouseWheel { delta_x, delta_y } => {
            input_devices::plan_wheel(&mut plan, &mut simulated, *delta_x, *delta_y);
        }
        DeviceCall::TouchscreenTap { x, y } => {
            input_devices::plan_tap(&mut plan, &mut simulated, (*x, *y));
        }
    }
    Ok(plan)
}

/// A page step that could not answer ends the device call, as any terminal page error does.
fn page_error(error: PageError) -> CallError {
    match error {
        PageError::Closed => PlaywrightError::target_closed().into(),
        PageError::NotConnected => {
            PlaywrightError::error("neo-runtime: the tab's input could not be reached").into()
        }
        PageError::Threw(message) | PageError::Contract(message) => {
            PlaywrightError::error(message).into()
        }
    }
}
