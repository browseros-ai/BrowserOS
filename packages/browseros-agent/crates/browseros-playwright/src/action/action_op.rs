//! The action ops and their registration metadata.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::action::actionability::PointerKind;
use crate::action::element_action::ElementRunner;
use crate::action::focus_lease::lease;
use crate::action::form_action::{FormEdit, FormRunner};
use crate::action::input_files::set_input_files;
use crate::action::key_action::{KeyRunner, Keys};
use crate::action::pointer::{DragPhases, Perform, PointerAction, Runner, WaitAfter};
use crate::input::{ClickOptions, MouseButton, Typed, millis};
use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{Capability, OpSpec, TARGET};
use crate::page::OpPage;
use crate::probe::{Edit, FillValue, Point, SelectOption};
use crate::target::{Strictness, Target};
use crate::{ErrorCode, Playwright, PlaywrightError};

listed_enum! {
    /// Actions on one element. The locator, page, frame and element-handle forms of a method
    /// are one op, and so are Playwright's client-side aliases: `clear` is `fill('')`,
    /// `setChecked` is `check` or `uncheck`, `type` is `pressSequentially` and `dragAndDrop`
    /// is `dragTo`. The prelude names the method it ran in `call.api`.
    pub(crate) enum ActionOp {
        Click,
        Dblclick,
        Hover,
        Check,
        Uncheck,
        DragTo,
        Fill,
        Press,
        PressSequentially,
        SelectOption,
        SelectText,
        Focus,
        Blur,
        SetInputFiles,
        DispatchEvent,
        ScrollIntoViewIfNeeded,
    }
}

impl ActionOp {
    /// This op's registration metadata.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::Click => OpSpec::write(
                "locator.click",
                TARGET,
                "click once visible, enabled, stable and hit; also page, frame and handle click",
            ),
            Self::Dblclick => OpSpec::write(
                "locator.dblclick",
                TARGET,
                "double-click after the click checks; also the page, frame and handle forms",
            ),
            Self::Hover => OpSpec::write(
                "locator.hover",
                TARGET,
                "move the mouse over an element once visible, stable and hit",
            ),
            Self::Check => OpSpec::write(
                "locator.check",
                TARGET,
                "check a checkbox or radio and verify it changed; also setChecked(true)",
            ),
            Self::Uncheck => OpSpec::write(
                "locator.uncheck",
                TARGET,
                "uncheck a checkbox and verify it changed; also setChecked(false)",
            ),
            Self::DragTo => OpSpec::write(
                "locator.dragTo",
                TARGET,
                "drag one element onto another; also page.dragAndDrop and frame.dragAndDrop",
            ),
            Self::Fill => OpSpec::write(
                "locator.fill",
                TARGET,
                "replace an editable element's value; also clear()",
            )
            .sensitive(&["/value"]),
            Self::Press => OpSpec::write(
                "locator.press",
                TARGET,
                "focus an element, then press a key or chord",
            )
            .sensitive(&["/key"]),
            Self::PressSequentially => OpSpec::write(
                "locator.pressSequentially",
                TARGET,
                "focus an element, then type text key by key; also type()",
            )
            .sensitive(&["/text"]),
            Self::SelectOption => OpSpec::write(
                "locator.selectOption",
                TARGET,
                "select options of a <select> by value, label or index",
            ),
            Self::SelectText => {
                OpSpec::write("locator.selectText", TARGET, "select an element's text")
            }
            Self::Focus => OpSpec::write("locator.focus", TARGET, "focus an element"),
            Self::Blur => OpSpec::write("locator.blur", TARGET, "remove focus from an element"),
            Self::SetInputFiles => OpSpec::write(
                "locator.setInputFiles",
                TARGET,
                "set a file input's files from host paths, a host directory or in-memory payloads",
            )
            .using(&[Capability::HostFiles])
            .policed(),
            Self::DispatchEvent => OpSpec::write(
                "locator.dispatchEvent",
                TARGET,
                "dispatch a DOM event on an element",
            ),
            Self::ScrollIntoViewIfNeeded => OpSpec::read(
                "locator.scrollIntoViewIfNeeded",
                TARGET,
                "scroll an element into view once it is stable",
            ),
        }
    }
}

/// `locator.click`, `dblclick`, `hover`, `check` and `uncheck` (wire.ts `PointerOptions`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PointerInput {
    target: Target,
    strict: Strictness,
    options: PointerOptions,
}

/// Playwright 1.64.1's pointer options. Each action reads the ones its protocol method has;
/// the prelude sends only those (`HOVER_KEYS`, `CHECK_KEYS` in options.ts).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PointerOptions {
    button: Option<WireButton>,
    click_count: Option<u32>,
    /// Milliseconds, after each press and between clicks.
    delay: Option<f64>,
    #[serde(default)]
    force: bool,
    modifiers: Option<Vec<WireModifier>>,
    #[serde(default)]
    no_wait_after: bool,
    position: Option<Point>,
    scroll: Option<ScrollOption>,
    steps: Option<u32>,
    #[serde(default)]
    trial: bool,
}

/// `locator.dragTo` and `dragAndDrop` (wire.ts `DragOptions`). `to` resolves on its own, in the
/// frame its locator starts from, as upstream's `frame.dragAndDrop` resolves both selectors.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DragInput {
    target: Target,
    strict: Strictness,
    to: Target,
    options: DragOptions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DragOptions {
    #[serde(default)]
    force: bool,
    /// Accepted, as upstream's client accepts it; 1.64.1's `dragAndDrop` never waits after.
    #[serde(default, rename = "noWaitAfter")]
    _no_wait_after: bool,
    scroll: Option<ScrollOption>,
    source_position: Option<Point>,
    target_position: Option<Point>,
    steps: Option<u32>,
    #[serde(default)]
    trial: bool,
}

/// `locator.fill(value)` and `clear()` (wire.ts `FillOptions`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FillInput {
    target: Target,
    strict: Strictness,
    value: FillValue,
    options: FillOptions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FillOptions {
    #[serde(default)]
    force: bool,
    /// Accepted, as upstream's client accepts it; 1.64.1's fill never waits after.
    #[serde(default, rename = "noWaitAfter")]
    _no_wait_after: bool,
}

/// `locator.press(key, options)`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PressInput {
    target: Target,
    strict: Strictness,
    key: Typed,
    options: KeyOptions,
}

/// `locator.pressSequentially(text, options)` and `type`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TypeInput {
    target: Target,
    strict: Strictness,
    text: Typed,
    options: KeyOptions,
}

/// wire.ts `KeyOptions`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct KeyOptions {
    /// Milliseconds, inside each press and before each inserted character.
    delay: Option<f64>,
    /// `press` waits for the navigations its keys requested unless this is set; `type` never
    /// waits, as upstream.
    #[serde(default)]
    no_wait_after: bool,
}

/// `locator.selectOption(values)`. `elements` are the `<option>` handles among the values.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectOptionInput {
    target: Target,
    strict: Strictness,
    options: Vec<SelectOption>,
    elements: Vec<Target>,
    #[serde(default)]
    force: bool,
}

/// `locator.selectText()`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectTextInput {
    target: Target,
    strict: Strictness,
    #[serde(default)]
    force: bool,
}

/// `locator.focus`, `blur` and `scrollIntoViewIfNeeded`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetInput {
    target: Target,
    strict: Strictness,
}

/// `locator.dispatchEvent(type, eventInit)`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DispatchEventInput {
    target: Target,
    strict: Strictness,
    r#type: String,
    /// JSON in v1; an absent or `null` init is `{}`, as upstream's default.
    #[serde(default)]
    event_init: Value,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WireButton {
    Left,
    Right,
    Middle,
}

impl WireButton {
    fn button(self) -> MouseButton {
        match self {
            Self::Left => MouseButton::Left,
            Self::Right => MouseButton::Right,
            Self::Middle => MouseButton::Middle,
        }
    }
}

/// Upstream's modifier names; `ControlOrMeta` resolves by the browser's platform when sent.
#[derive(Clone, Copy, Debug, Deserialize)]
enum WireModifier {
    Alt,
    Control,
    ControlOrMeta,
    Meta,
    Shift,
}

impl WireModifier {
    fn name(self) -> &'static str {
        match self {
            Self::Alt => "Alt",
            Self::Control => "Control",
            Self::ControlOrMeta => "ControlOrMeta",
            Self::Meta => "Meta",
            Self::Shift => "Shift",
        }
    }
}

/// 1.64.1's `scroll` option.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ScrollOption {
    Auto,
    None,
}

/// Runs one action op. Boxed: an action's retry loops hold a large state machine, and the
/// future every op shares (`Playwright::invoke`) is as large as its largest family's.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: ActionOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    Box::pin(run(pw, scope, op, args)).await
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per op, each decoding its input and handing it to its runner"
)]
async fn run(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: ActionOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    match op {
        ActionOp::Click | ActionOp::Dblclick | ActionOp::Hover => {
            let input: PointerInput = args.decode(scope)?;
            let action = pointer_action(op, &input.options);
            let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
            let _lease = lease(scope, &page).await?;
            Runner::new(pw, scope, &page, &input.target, input.strict)
                .run(&action)
                .await?;
        }
        ActionOp::Check | ActionOp::Uncheck => {
            let input: PointerInput = args.decode(scope)?;
            let action = pointer_action(op, &input.options);
            let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
            let _lease = lease(scope, &page).await?;
            Runner::new(pw, scope, &page, &input.target, input.strict)
                .set_checked(op == ActionOp::Check, &action)
                .await?;
        }
        ActionOp::DragTo => {
            let input: DragInput = args.decode(scope)?;
            // The host's door checks the tab at `target.page` only, so the drop target must
            // carry the same page brand, id and tab, for that one check to cover both halves.
            if input.to.page() != input.target.page() {
                return Err(scope.fail(
                    ErrorCode::InvalidInput,
                    "the drop target is on another page than the dragged element",
                ));
            }
            let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
            let _lease = lease(scope, &page).await?;
            Runner::new(pw, scope, &page, &input.target, input.strict)
                .drag_to(&input.to, &drag_phases(&input.options))
                .await?;
        }
        ActionOp::Focus | ActionOp::Blur => {
            let input: TargetInput = args.decode(scope)?;
            let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
            let edit = if op == ActionOp::Focus {
                Edit::Focus {
                    reset_selection: false,
                }
            } else {
                Edit::Blur
            };
            ElementRunner::new(scope, &page, &input.target, input.strict)
                .edit(edit)
                .await?;
        }
        ActionOp::DispatchEvent => {
            let input: DispatchEventInput = args.decode(scope)?;
            let init = match input.event_init {
                Value::Object(init) => init,
                Value::Null => Map::new(),
                _ => {
                    return Err(
                        scope.fail(ErrorCode::InvalidInput, "eventInit: expected an object")
                    );
                }
            };
            let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
            let _lease = lease(scope, &page).await?;
            ElementRunner::new(scope, &page, &input.target, input.strict)
                .edit(Edit::DispatchEvent {
                    r#type: input.r#type,
                    init,
                })
                .await?;
        }
        ActionOp::ScrollIntoViewIfNeeded => {
            let input: TargetInput = args.decode(scope)?;
            let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
            ElementRunner::new(scope, &page, &input.target, input.strict)
                .scroll_into_view()
                .await?;
        }
        ActionOp::Fill => {
            let input: FillInput = args.decode(scope)?;
            let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
            let _lease = lease(scope, &page).await?;
            let edit = FormEdit::Fill(&input.value);
            FormRunner::new(
                scope,
                &page,
                &input.target,
                input.strict,
                input.options.force,
            )
            .run(&edit)
            .await?;
        }
        ActionOp::SelectOption => {
            let input: SelectOptionInput = args.decode(scope)?;
            if !input.elements.is_empty() {
                return Err(scope.fail(
                    ErrorCode::Unsupported,
                    "Neo selects options by value, label or index, not by element handle; pass \
                     the option's value or label",
                ));
            }
            let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
            let _lease = lease(scope, &page).await?;
            let edit = FormEdit::SelectOptions(&input.options);
            return FormRunner::new(scope, &page, &input.target, input.strict, input.force)
                .run(&edit)
                .await;
        }
        ActionOp::SelectText => {
            let input: SelectTextInput = args.decode(scope)?;
            let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
            FormRunner::new(scope, &page, &input.target, input.strict, input.force)
                .run(&FormEdit::SelectText)
                .await?;
        }
        ActionOp::Press => {
            let input: PressInput = args.decode(scope)?;
            let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
            let _lease = lease(scope, &page).await?;
            let keys = Keys::Press {
                key: &input.key.0,
                delay: millis(input.options.delay),
                wait_after: !input.options.no_wait_after,
            };
            KeyRunner::new(pw, scope, &page, &input.target, input.strict)
                .run(&keys)
                .await?;
        }
        ActionOp::PressSequentially => {
            let input: TypeInput = args.decode(scope)?;
            let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
            let _lease = lease(scope, &page).await?;
            let keys = Keys::Type {
                text: &input.text.0,
                delay: millis(input.options.delay),
            };
            KeyRunner::new(pw, scope, &page, &input.target, input.strict)
                .run(&keys)
                .await?;
        }
        ActionOp::SetInputFiles => set_input_files(pw, scope, args.decode(scope)?).await?,
    }
    Ok(Value::Null)
}

/// Upstream's click, dblclick, hover and the click inside check, from their options.
fn pointer_action(op: ActionOp, options: &PointerOptions) -> PointerAction {
    let click = ClickOptions {
        button: options.button.map_or(MouseButton::Left, WireButton::button),
        click_count: options.click_count.unwrap_or(1),
        delay: millis(options.delay),
        steps: options.steps.unwrap_or(1),
    };
    let (kind, perform, wait_after) = match op {
        ActionOp::Dblclick => (
            PointerKind::Dblclick,
            Perform::Click(ClickOptions {
                click_count: 2,
                ..click
            }),
            WaitAfter::Verdict,
        ),
        ActionOp::Hover => (PointerKind::Hover, Perform::Move, WaitAfter::Verdict),
        // check and uncheck click with upstream's defaults and wait for the verdict only.
        ActionOp::Check | ActionOp::Uncheck => (
            PointerKind::Click,
            Perform::Click(ClickOptions::default()),
            WaitAfter::Verdict,
        ),
        _ => (
            PointerKind::Click,
            Perform::Click(click),
            if options.no_wait_after {
                WaitAfter::Nothing
            } else {
                WaitAfter::Navigations
            },
        ),
    };
    PointerAction {
        kind,
        perform,
        wait_after,
        force: options.force,
        trial: options.trial,
        position: options.position,
        modifiers: options
            .modifiers
            .as_ref()
            .map(|names| names.iter().map(|name| name.name()).collect()),
        scroll: options.scroll != Some(ScrollOption::None),
    }
}

/// `dragTo`'s two halves: `move and down` on the source, `move and up` on the target.
fn drag_phases(options: &DragOptions) -> DragPhases {
    let half = |kind, perform, position| PointerAction {
        kind,
        perform,
        wait_after: WaitAfter::Verdict,
        force: options.force,
        trial: options.trial,
        position,
        modifiers: None,
        scroll: options.scroll != Some(ScrollOption::None),
    };
    DragPhases {
        down: half(
            PointerKind::DragSource,
            Perform::MoveAndDown,
            options.source_position,
        ),
        up: half(
            PointerKind::DragTarget,
            Perform::MoveAndUp {
                steps: options.steps.unwrap_or(1),
            },
            options.target_position,
        ),
    }
}
