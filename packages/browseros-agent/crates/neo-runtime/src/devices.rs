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
    #[expect(
        clippy::result_large_err,
        reason = "the Err hands the generated PageCall back unchanged, once per Page call; its largest params are the screenshot's and the PDF's"
    )]
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
            "neo-runtime: the device call {} has no delivery cell",
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
            "neo-runtime: the device call {} tried to write its input again",
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

/// Neo: a page step that could not answer ends the device call, as any terminal page error
/// does.
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

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::time;

    use super::*;
    use crate::click::tests::{FakeTab, fake_owner};
    use crate::input_devices::{InputDevices, KeyboardState, MouseState};
    use crate::progress::ErrorName;
    use crate::run::Delivery;

    const LEFT_CLICK: MouseClick = MouseClick {
        button: MouseButton::Left,
        click_count: 1,
        delay: None,
    };

    /// Serves `call` on `tab` with Playwright's `timeout`, then lets a writer the call left
    /// behind finish: the outcome, and the delivery the writer recorded.
    async fn run(
        tab: &FakeTab,
        call: &DeviceCall,
        timeout_ms: f64,
    ) -> (Result<(), CallError>, Delivery) {
        let progress = Progress::for_test(timeout_ms);
        let result = serve(tab, call, &progress).await;
        time::sleep(Duration::from_mins(1)).await;
        let delivery = progress.delivery().expect("an input call").delivery();
        (result, delivery)
    }

    fn error_of(result: Result<(), CallError>) -> PlaywrightError {
        match result {
            Err(CallError::Playwright(error)) => error,
            other => panic!("expected a Playwright error, got {other:?}"),
        }
    }

    fn devices(tab: &FakeTab) -> InputDevices {
        tab.devices.lock().expect("devices").clone()
    }

    #[expect(
        clippy::result_large_err,
        reason = "returns `DeviceCall::from_page_call`'s answer as it is"
    )]
    fn decode(
        method: &str,
        params: serde_json::Value,
    ) -> Result<Result<DeviceCall, PlaywrightError>, PageCall> {
        DeviceCall::from_page_call(PageCall::decode(method, Some(params)).expect("a Page command"))
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_key_held_by_down_reaches_later_input_until_up() {
        let tab = FakeTab::ready();
        for call in [
            DeviceCall::KeyboardDown {
                key: "Shift".to_string(),
            },
            DeviceCall::MouseClick {
                x: 10.0,
                y: 10.0,
                click: LEFT_CLICK,
            },
            DeviceCall::KeyboardPress {
                key: "a".to_string(),
                delay: None,
            },
            DeviceCall::TouchscreenTap { x: 10.0, y: 10.0 },
            DeviceCall::KeyboardUp {
                key: "Shift".to_string(),
            },
            DeviceCall::MouseMove {
                x: 20.0,
                y: 20.0,
                steps: 1,
            },
        ] {
            let (result, delivery) = run(&tab, &call, 30_000.0).await;
            assert!(result.is_ok(), "{call:?}: {result:?}");
            assert_eq!(delivery, Delivery::Done, "{call:?}");
        }
        assert_eq!(
            tab.calls(),
            [
                "platform",
                "input",
                "rawKeyDown Shift",
                "input released",
                "input",
                "mouseMoved button=none buttons=0 modifiers=8",
                "mousePressed button=left buttons=1 clickCount=1 modifiers=8",
                "mouseReleased button=left buttons=0 clickCount=1 modifiers=8",
                "input released",
                "platform",
                "input",
                "keyDown a",
                "keyUp a",
                "input released",
                "input",
                "touchStart modifiers=8",
                "touchEnd modifiers=8",
                "input released",
                "platform",
                "input",
                "keyUp Shift",
                "input released",
                "input",
                "mouseMoved button=none buttons=0 modifiers=0",
                "input released",
            ],
            "Shift stays held across calls until its up"
        );
        assert_eq!(devices(&tab).keyboard, KeyboardState::default());
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_mouse_down_holds_its_button_for_later_moves_until_up() {
        let tab = FakeTab::ready();
        let calls = [
            DeviceCall::MouseDown {
                button: MouseButton::Left,
                click_count: 1,
            },
            DeviceCall::MouseDown {
                button: MouseButton::Right,
                click_count: 2,
            },
            DeviceCall::MouseMove {
                x: 5.0,
                y: 5.0,
                steps: 1,
            },
        ];
        for call in &calls {
            let (result, delivery) = run(&tab, call, 30_000.0).await;
            assert!(result.is_ok(), "{call:?}: {result:?}");
            assert_eq!(delivery, Delivery::Done, "{call:?}");
        }
        let held = devices(&tab).mouse;
        assert_eq!(
            held.buttons,
            [
                (MouseButton::Left, fake_owner()),
                (MouseButton::Right, fake_owner())
            ]
        );
        assert_eq!(held.last_button, Some(MouseButton::Right));
        for call in [
            DeviceCall::MouseUp {
                button: MouseButton::Left,
                click_count: 1,
            },
            DeviceCall::MouseUp {
                button: MouseButton::Right,
                click_count: 1,
            },
            DeviceCall::MouseMove {
                x: 6.0,
                y: 6.0,
                steps: 1,
            },
        ] {
            let (result, delivery) = run(&tab, &call, 30_000.0).await;
            assert!(result.is_ok(), "{call:?}: {result:?}");
            assert_eq!(delivery, Delivery::Done, "{call:?}");
        }
        let events: Vec<String> = tab
            .calls()
            .into_iter()
            .filter(|call| call.starts_with("mouse"))
            .collect();
        assert_eq!(
            events,
            [
                "mousePressed button=left buttons=1 clickCount=1 modifiers=0",
                "mousePressed button=right buttons=3 clickCount=2 modifiers=0",
                "mouseMoved button=right buttons=3 modifiers=0",
                "mouseReleased button=left buttons=2 clickCount=1 modifiers=0",
                "mouseReleased button=right buttons=0 clickCount=1 modifiers=0",
                "mouseMoved button=none buttons=0 modifiers=0",
            ]
        );
        assert_eq!(devices(&tab).mouse.buttons, []);
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_call_with_nothing_to_write_changes_state_and_sends_nothing() {
        let tab = FakeTab::ready();
        let (result, delivery) = run(
            &tab,
            &DeviceCall::MouseMove {
                x: 50.0,
                y: 60.0,
                steps: 0,
            },
            30_000.0,
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(delivery, Delivery::NotSent);
        let mouse = devices(&tab).mouse;
        assert_eq!(
            (mouse.x, mouse.y),
            (50.0, 60.0),
            "a move of no steps still moves"
        );
        let (result, delivery) = run(
            &tab,
            &DeviceCall::KeyboardType {
                text: String::new(),
                delay: None,
            },
            30_000.0,
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(delivery, Delivery::NotSent);
        assert_eq!(
            tab.calls(),
            [
                "input",
                "input released",
                "platform",
                "input",
                "input released"
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_an_unknown_key_rejects_before_any_input() {
        let tab = FakeTab::ready();
        for call in [
            DeviceCall::KeyboardDown {
                key: "Foo".to_string(),
            },
            DeviceCall::KeyboardPress {
                key: "Shift+Foo".to_string(),
                delay: None,
            },
        ] {
            let (result, delivery) = run(&tab, &call, 30_000.0).await;
            assert_eq!(
                error_of(result),
                PlaywrightError::error("Unknown key: \"Foo\""),
                "{call:?}"
            );
            assert_eq!(delivery, Delivery::NotSent, "{call:?}");
        }
        assert_eq!(
            tab.calls(),
            [
                "platform",
                "input",
                "input released",
                "platform",
                "input",
                "input released"
            ],
            "nothing written, not even the Shift before the unknown key"
        );
        assert_eq!(devices(&tab), InputDevices::default());
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_stopped_device_call_releases_what_it_pressed() {
        let tab = FakeTab::ready();
        let (result, delivery) = run(
            &tab,
            &DeviceCall::KeyboardType {
                text: "ab".to_string(),
                delay: Some(Duration::from_secs(10)),
            },
            100.0,
        )
        .await;
        assert_eq!(error_of(result).name, ErrorName::TimeoutError);
        assert_eq!(delivery, Delivery::Unknown, "b was never typed");
        let (result, delivery) = run(
            &tab,
            &DeviceCall::MouseClick {
                x: 1.0,
                y: 1.0,
                click: MouseClick {
                    click_count: 2,
                    delay: Some(Duration::from_secs(10)),
                    ..LEFT_CLICK
                },
            },
            100.0,
        )
        .await;
        assert_eq!(error_of(result).name, ErrorName::TimeoutError);
        assert_eq!(
            delivery,
            Delivery::Unknown,
            "the second click was never sent"
        );
        assert_eq!(
            tab.calls(),
            [
                "platform",
                "input",
                "keyDown a",
                "keyUp a",
                "input released",
                "input",
                "mouseMoved button=none buttons=0 modifiers=0",
                "mousePressed button=left buttons=1 clickCount=1 modifiers=0",
                "mouseReleased button=left buttons=0 clickCount=1 modifiers=0",
                "input released",
            ],
            "the delay is cut short, never a release"
        );
        assert_eq!(
            devices(&tab),
            InputDevices {
                mouse: MouseState {
                    x: 1.0,
                    y: 1.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            "nothing left held"
        );
    }

    #[test]
    fn tests_that_device_commands_decode_with_playwrights_defaults() {
        assert_eq!(
            decode("mouseDown", json!({})),
            Ok(Ok(DeviceCall::MouseDown {
                button: MouseButton::Left,
                click_count: 1
            }))
        );
        assert_eq!(
            decode("mouseUp", json!({ "button": "middle", "clickCount": 3 })),
            Ok(Ok(DeviceCall::MouseUp {
                button: MouseButton::Middle,
                click_count: 3
            }))
        );
        assert_eq!(
            decode("mouseMove", json!({ "x": 1, "y": 2 })),
            Ok(Ok(DeviceCall::MouseMove {
                x: 1.0,
                y: 2.0,
                steps: 1
            }))
        );
        assert_eq!(
            decode("mouseMove", json!({ "x": 1, "y": 2, "steps": -3 })),
            Ok(Ok(DeviceCall::MouseMove {
                x: 1.0,
                y: 2.0,
                steps: 0
            })),
            "fewer than one step moves without an event"
        );
        assert_eq!(
            decode("mouseClick", json!({ "x": 1, "y": 2, "delay": 0 })),
            Ok(Ok(DeviceCall::MouseClick {
                x: 1.0,
                y: 2.0,
                click: LEFT_CLICK
            })),
            "a zero delay is Playwright's pipelined click"
        );
        assert_eq!(
            decode("keyboardPress", json!({ "key": "a", "delay": -5 })),
            Ok(Ok(DeviceCall::KeyboardPress {
                key: "a".to_string(),
                delay: Some(Duration::ZERO),
            }))
        );
        assert_eq!(
            decode("mouseClick", json!({ "x": 1, "y": 2, "clickCount": 1001 })),
            Ok(Err(PlaywrightError::error(
                "Neo does not support a clickCount above 1000"
            )))
        );
        assert!(matches!(
            decode("bringToFront", json!({})),
            Err(PageCall::BringToFront(_))
        ));
    }
}
