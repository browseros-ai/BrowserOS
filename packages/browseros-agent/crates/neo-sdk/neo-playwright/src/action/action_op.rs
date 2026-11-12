//! The action ops and their registration metadata.
//!
//! Each pointer handler owns its input and reply shape. The shared runners still own
//! actionability, retrying, focus leases and navigation waits; typing the wire does not move
//! those checks or grant an op any capability.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::sync::Arc;
use ts_rs::TS;

use crate::action::actionability::PointerKind;
use crate::action::element_action::ElementRunner;
use crate::action::focus_lease::lease;
use crate::action::form_action::{FormEdit, FormRunner};
use crate::action::input_files::{DropFiles, drop_payloads, set_input_files};
use crate::action::key_action::{KeyRunner, Keys};
use crate::action::pointer_action::{
    DragPhases, DropData, Perform, PointerAction, PointerRunner, WaitAfter,
};
use crate::action::requested_navigations::NavigationCut;
use crate::input::{ClickOptions as MouseClickOptions, MouseButton, Typed, millis};
use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{Capability, OpSpec, TARGET};
use crate::page::OpPage;
use crate::probe::{DropEntry, Edit, FillValue, Point, SelectOption};
use crate::target::{Strictness, Target};
use crate::wire::{self, Describe, Nothing, OpWire, Route, Run, options};
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
        Drop,
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
    /// Typed actions describe their handlers; the rest retain raw decoding and hand-written
    /// TypeScript contracts until they move onto typed rows.
    pub(crate) fn wire(self) -> Option<OpWire> {
        route(self, Describe)
    }

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
            ),
            Self::Press => OpSpec::write(
                "locator.press",
                TARGET,
                "focus an element, then press a key or chord",
            ),
            Self::PressSequentially => OpSpec::write(
                "locator.pressSequentially",
                TARGET,
                "focus an element, then type text key by key; also type()",
            ),
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
            Self::Drop => OpSpec::write(
                "locator.drop",
                TARGET,
                "drop external files and MIME data onto an element",
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

/// The same exhaustive rows drive runtime decoding and wire generation. The transitional
/// rows keep undescribed actions on their existing decoder and hand-written contract.
fn route<'a, R: Route<'a>>(op: ActionOp, r: R) -> R::Out {
    match op {
        ActionOp::Click => r.op(
            wire::command("ElementHandle.click").result(
                "the requested-navigation event cut the prelude publishes before click resolves",
            ),
            click,
        ),
        ActionOp::Dblclick => r.op(wire::command("ElementHandle.dblclick"), dblclick),
        ActionOp::Hover => r.op(wire::command("ElementHandle.hover"), hover),
        ActionOp::Check => r.op(wire::command("ElementHandle.check"), check),
        ActionOp::Uncheck => r.op(wire::command("ElementHandle.uncheck"), uncheck),
        ActionOp::DragTo => r.op(
            wire::command("Frame.dragAndDrop")
                .adds(&[("to", "the destination retains its locator frame; same-page admission covers both endpoints")])
                .omits(&[
                    (
                        "source",
                        "the source selector is carried by Neo's target brand",
                    ),
                    (
                        "target",
                        "the destination selector is carried by Neo's to brand",
                    ),
                ]),
            drag_to,
        ),
        ActionOp::Drop => r.op(
            wire::command("Frame.drop")
                .adds(&[(
                    "files",
                    "Neo resolves paths or prepared payloads on its host",
                )])
                .omits(&[
                    ("localPaths", "folded into Neo's files paths variant"),
                    ("payloads", "folded into Neo's files payloads variant"),
                    ("streams", "no client file streams in a stateless run"),
                ]),
            drop,
        ),
        op @ (ActionOp::Fill
        | ActionOp::Press
        | ActionOp::PressSequentially
        | ActionOp::SelectOption
        | ActionOp::SelectText
        | ActionOp::Focus
        | ActionOp::Blur
        | ActionOp::SetInputFiles
        | ActionOp::DispatchEvent
        | ActionOp::ScrollIntoViewIfNeeded) => {
            r.untyped(move |pw, scope, args| run(pw, scope, op, args))
        }
    }
}

/// Optional wire flags distinguish omission from an explicit null, which must fail before an
/// action runs. Field defaults handle omission; only a boolean reaches this deserializer.
fn optional_bool<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<bool>, D::Error> {
    bool::deserialize(deserializer).map(Some)
}

/// Each pointer input carries only its command's flat fields. The runners own actionability,
/// focus and navigation; these types own validation before any browser input is sent.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(optional_fields)]
struct ClickInput {
    target: Target,
    strict: Strictness,
    button: Option<WireButton>,
    click_count: Option<u32>,
    /// Milliseconds, after each press and between clicks.
    delay: Option<f64>,
    #[serde(default, deserialize_with = "optional_bool")]
    force: Option<bool>,
    modifiers: Option<Vec<WireModifier>>,
    #[serde(default, deserialize_with = "optional_bool")]
    no_wait_after: Option<bool>,
    #[ts(optional, type = "import('../../../probe/protocol').Point")]
    position: Option<Point>,
    scroll: Option<ScrollOption>,
    steps: Option<u32>,
    #[serde(default, deserialize_with = "optional_bool")]
    trial: Option<bool>,
}
options!(ClickInput, flat);

/// The double-click wire input; upstream fixes the click count at two.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(optional_fields)]
struct DblclickInput {
    target: Target,
    strict: Strictness,
    button: Option<WireButton>,
    delay: Option<f64>,
    #[serde(default, deserialize_with = "optional_bool")]
    force: Option<bool>,
    modifiers: Option<Vec<WireModifier>>,
    #[ts(optional, type = "import('../../../probe/protocol').Point")]
    position: Option<Point>,
    scroll: Option<ScrollOption>,
    steps: Option<u32>,
    #[serde(default, deserialize_with = "optional_bool")]
    trial: Option<bool>,
}
options!(DblclickInput, flat);

/// A hover carries move options only, while the runner keeps actionability and focus ownership.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(optional_fields)]
struct HoverInput {
    target: Target,
    strict: Strictness,
    #[serde(default, deserialize_with = "optional_bool")]
    force: Option<bool>,
    modifiers: Option<Vec<WireModifier>>,
    #[ts(optional, type = "import('../../../probe/protocol').Point")]
    position: Option<Point>,
    scroll: Option<ScrollOption>,
    #[serde(default, deserialize_with = "optional_bool")]
    trial: Option<bool>,
}
options!(HoverInput, flat);

/// A check uses the default physical click, then verifies the element became checked.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(optional_fields)]
struct CheckInput {
    target: Target,
    strict: Strictness,
    #[serde(default, deserialize_with = "optional_bool")]
    force: Option<bool>,
    #[ts(optional, type = "import('../../../probe/protocol').Point")]
    position: Option<Point>,
    scroll: Option<ScrollOption>,
    #[serde(default, deserialize_with = "optional_bool")]
    trial: Option<bool>,
}
options!(CheckInput, flat);

/// An uncheck has its own contract even though it shares check's pointer runner.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(optional_fields)]
struct UncheckInput {
    target: Target,
    strict: Strictness,
    #[serde(default, deserialize_with = "optional_bool")]
    force: Option<bool>,
    #[ts(optional, type = "import('../../../probe/protocol').Point")]
    position: Option<Point>,
    scroll: Option<ScrollOption>,
    #[serde(default, deserialize_with = "optional_bool")]
    trial: Option<bool>,
}
options!(UncheckInput, flat);

/// The requested-navigation event cut that the prelude publishes before click resolves.
#[derive(Debug, Serialize, TS)]
struct Clicked {
    navigation: NavigationCut,
}

/// `locator.dragTo` and `dragAndDrop`: the second target resolves independently in its
/// locator's frame, while both endpoints must stay under the same page admission.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(optional_fields)]
struct DragInput {
    target: Target,
    strict: Strictness,
    to: Target,
    #[serde(default, deserialize_with = "optional_bool")]
    force: Option<bool>,
    scroll: Option<ScrollOption>,
    #[ts(optional, type = "import('../../../probe/protocol').Point")]
    source_position: Option<Point>,
    #[ts(optional, type = "import('../../../probe/protocol').Point")]
    target_position: Option<Point>,
    steps: Option<u32>,
    #[serde(default, deserialize_with = "optional_bool")]
    trial: Option<bool>,
}
options!(DragInput, flat);

/// External drops already carry flat fields, not an options bag. Keep their file/MIME
/// preparation before target admission and preserve the caller's MIME entry order.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
struct DropInput {
    target: Target,
    strict: Strictness,
    #[serde(default)]
    #[ts(optional)]
    files: Option<DropFiles>,
    // The probe names this shape inside its Edit union rather than exporting a second alias.
    #[serde(default, deserialize_with = "optional_drop_data")]
    #[ts(
        optional,
        type = "Extract<import('../../../probe/protocol').Edit, { kind: 'drop' }>['data']"
    )]
    data: Option<Vec<DropEntry>>,
    #[serde(default)]
    #[ts(optional, type = "import('../../../probe/protocol').Point")]
    position: Option<Point>,
}
options!(DropInput);

/// Omission means a files-only drop. A present value must still be an array: `null` must not
/// silently become an empty MIME list at the decode boundary.
fn optional_drop_data<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<DropEntry>>, D::Error> {
    Vec::deserialize(deserializer).map(Some)
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

#[derive(Clone, Copy, Debug, Deserialize, TS)]
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
#[derive(Clone, Copy, Debug, Deserialize, TS)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, TS)]
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
    route(op, Run { pw, scope, args }).await
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
        // These variants are always intercepted by typed route rows above. Keep this
        // boundary explicit while the remaining ten variants still share the old runner.
        ActionOp::Click
        | ActionOp::Dblclick
        | ActionOp::Hover
        | ActionOp::Check
        | ActionOp::Uncheck
        | ActionOp::DragTo
        | ActionOp::Drop => {
            return Err(scope.fail(ErrorCode::Internal, "a typed action bypassed its route"));
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

/// Click alone returns a navigation cut. Keeping this result typed ties the publication the
/// prelude reads to the same runner that waited for the requested navigations.
async fn click(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ClickInput,
) -> Result<Option<Clicked>, PlaywrightError> {
    let action = pointer_action(&input);
    let navigation = run_pointer(pw, scope, &input.target, input.strict, &action).await?;
    Ok(navigation.map(|navigation| Clicked { navigation }))
}

async fn dblclick(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: DblclickInput,
) -> Result<Nothing, PlaywrightError> {
    let action = PointerAction {
        kind: PointerKind::Dblclick,
        perform: Perform::Click(MouseClickOptions {
            button: input.button.map_or(MouseButton::Left, WireButton::button),
            click_count: 2,
            delay: millis(input.delay),
            steps: input.steps.unwrap_or(1),
        }),
        wait_after: WaitAfter::Verdict,
        force: input.force.unwrap_or(false),
        trial: input.trial.unwrap_or(false),
        position: input.position,
        modifiers: input
            .modifiers
            .as_ref()
            .map(|names| names.iter().map(|name| name.name()).collect()),
        scroll: input.scroll != Some(ScrollOption::None),
    };
    run_pointer(pw, scope, &input.target, input.strict, &action).await?;
    Ok(())
}

async fn hover(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: HoverInput,
) -> Result<Nothing, PlaywrightError> {
    let action = PointerAction {
        kind: PointerKind::Hover,
        perform: Perform::Move,
        wait_after: WaitAfter::Verdict,
        force: input.force.unwrap_or(false),
        trial: input.trial.unwrap_or(false),
        position: input.position,
        modifiers: input
            .modifiers
            .as_ref()
            .map(|names| names.iter().map(|name| name.name()).collect()),
        scroll: input.scroll != Some(ScrollOption::None),
    };
    run_pointer(pw, scope, &input.target, input.strict, &action).await?;
    Ok(())
}

async fn check(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: CheckInput,
) -> Result<Nothing, PlaywrightError> {
    let action = checked_action(input.force, input.trial, input.position, input.scroll);
    set_checked(pw, scope, &input.target, input.strict, true, &action).await
}

async fn uncheck(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: UncheckInput,
) -> Result<Nothing, PlaywrightError> {
    let action = checked_action(input.force, input.trial, input.position, input.scroll);
    set_checked(pw, scope, &input.target, input.strict, false, &action).await
}

/// The focus lease covers the full retrying pointer run; no input is sent before admission.
async fn run_pointer(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &Target,
    strict: Strictness,
    action: &PointerAction,
) -> Result<Option<NavigationCut>, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, target.page()).await?;
    let _lease = lease(scope, &page).await?;
    let mut runner = PointerRunner::new(pw, scope, &page, target, strict);
    runner.run(action).await?;
    Ok(runner.navigation_cut())
}

/// Check/uncheck use the same pointer runner but verify the checked state after its click.
async fn set_checked(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &Target,
    strict: Strictness,
    checked: bool,
    action: &PointerAction,
) -> Result<Nothing, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, target.page()).await?;
    let _lease = lease(scope, &page).await?;
    PointerRunner::new(pw, scope, &page, target, strict)
        .set_checked(checked, action)
        .await
}

async fn drag_to(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: DragInput,
) -> Result<Nothing, PlaywrightError> {
    // The host admits target.page only. The destination must carry that same page brand so
    // the single admission also covers the drag's second half.
    if input.to.page() != input.target.page() {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            "the drop target is on another page than the dragged element",
        ));
    }
    let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
    let _lease = lease(scope, &page).await?;
    PointerRunner::new(pw, scope, &page, &input.target, input.strict)
        .drag_to(&input.to, &drag_phases(&input))
        .await
}

async fn drop(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: DropInput,
) -> Result<Nothing, PlaywrightError> {
    scope.require(Capability::HostFiles)?;
    let payloads = match input.files {
        Some(files) => scope
            .within(drop_payloads(files))
            .await
            .map_err(|_expired| scope.timed_out(ErrorCode::WaitTimeout))??,
        None => Vec::new(),
    };
    let data = input.data.unwrap_or_default();
    if payloads.is_empty() && data.is_empty() {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            "At least one of \"files\" or \"data\" must be provided.",
        ));
    }
    let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
    let _lease = lease(scope, &page).await?;
    let action = PointerAction {
        kind: PointerKind::Drop,
        perform: Perform::Drop(Arc::new(DropData { payloads, data })),
        wait_after: WaitAfter::Verdict,
        force: false,
        trial: false,
        position: input.position,
        modifiers: None,
        scroll: true,
    };
    PointerRunner::new(pw, scope, &page, &input.target, input.strict)
        .run(&action)
        .await
}

/// Only click waits for requested navigations; other pointer inputs build verdict-only
/// actions without admitting click-only fields.
fn pointer_action(input: &ClickInput) -> PointerAction {
    PointerAction {
        kind: PointerKind::Click,
        perform: Perform::Click(MouseClickOptions {
            button: input.button.map_or(MouseButton::Left, WireButton::button),
            click_count: input.click_count.unwrap_or(1),
            delay: millis(input.delay),
            steps: input.steps.unwrap_or(1),
        }),
        wait_after: if input.no_wait_after.unwrap_or(false) {
            WaitAfter::Nothing
        } else {
            WaitAfter::Navigations
        },
        force: input.force.unwrap_or(false),
        trial: input.trial.unwrap_or(false),
        position: input.position,
        modifiers: input
            .modifiers
            .as_ref()
            .map(|names| names.iter().map(|name| name.name()).collect()),
        scroll: input.scroll != Some(ScrollOption::None),
    }
}

/// Checking clicks with upstream's default button/count and never waits for navigation.
fn checked_action(
    force: Option<bool>,
    trial: Option<bool>,
    position: Option<Point>,
    scroll: Option<ScrollOption>,
) -> PointerAction {
    PointerAction {
        kind: PointerKind::Click,
        perform: Perform::Click(MouseClickOptions::default()),
        wait_after: WaitAfter::Verdict,
        force: force.unwrap_or(false),
        trial: trial.unwrap_or(false),
        position,
        modifiers: None,
        scroll: scroll != Some(ScrollOption::None),
    }
}

/// `dragTo`'s two halves: `move and down` on the source, `move and up` on the target.
fn drag_phases(input: &DragInput) -> DragPhases {
    let half = |kind, perform, position| PointerAction {
        kind,
        perform,
        wait_after: WaitAfter::Verdict,
        force: input.force.unwrap_or(false),
        trial: input.trial.unwrap_or(false),
        position,
        modifiers: None,
        scroll: input.scroll != Some(ScrollOption::None),
    };
    DragPhases {
        down: half(
            PointerKind::DragSource,
            Perform::MoveAndDown,
            input.source_position,
        ),
        up: half(
            PointerKind::DragTarget,
            Perform::MoveAndUp {
                steps: input.steps.unwrap_or(1),
            },
            input.target_position,
        ),
    }
}

#[cfg(test)]
#[path = "pointer_wire_tests.rs"]
mod tests;
