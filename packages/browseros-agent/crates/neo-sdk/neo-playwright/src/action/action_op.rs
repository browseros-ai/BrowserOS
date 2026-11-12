//! The action ops and their registration metadata.
//!
//! Each action handler owns its input and reply shape. The shared runners still own
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
    /// Describes every action from the same exhaustive route rows used at runtime.
    pub(crate) fn wire(self) -> OpWire {
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

/// `locator.setInputFiles`'s one field for upstream's `localPaths`, `localDirectory` and
/// `payloads` (a difference the owner names as needed), with why it exists.
const UPLOAD_FILES: (&str, &str) = (
    "files",
    "the prelude has no filesystem, so it cannot stat a path to choose localPaths \
                     or localDirectory as upstream's client does: either the script's path \
                     strings, which Rust resolves and stats on Neo's host under the HostFiles \
                     capability (files, or one directory), or the script's in-memory payloads",
);

/// `locator.drop`'s one field for upstream's `localPaths` and `payloads`, with why it exists.
const DROP_FILES: (&str, &str) = (
    "files",
    "the prelude has no filesystem: either the script's paths, each with the MIME \
                     type the prelude's vendored table gives it, which Rust reads into payloads on \
                     Neo's host under the HostFiles capability (a directory refused), or the \
                     script's in-memory payloads",
);

/// Why `fill` (and `clear`) behaves differently from upstream's on purpose.
const FILL_BEHAVES: &str = "a deliberate deviation, by decision (the owner's standing rule that \
     Neo never acts on an element other than the one asked for; lead, 2026-11-12): fill types \
     only into its target. A field the page replaces as it takes the focus is found again, and a \
     focus that ends elsewhere fails not_focused, where upstream types into the box typed into \
     before or the one that took the focus. A failure names the value by its length \
     (call_log::entered_text), where upstream quotes it";

/// Why `pressSequentially` (and `type`) behaves differently from upstream's on purpose.
const TYPE_BEHAVES: &str = "a deliberate deviation, by decision (the owner's standing rule that \
     Neo never acts on an element other than the one asked for; lead, 2026-11-12): the keys type \
     only into the target. A box the page replaces as it takes the focus is found again, and a \
     focus that ends elsewhere, or a target that cannot take it, fails not_focused, where \
     upstream types into nothing or into whatever has the focus. The call log names the text by \
     its length (call_log::entered_text)";

/// The same exhaustive rows drive runtime decoding, generated senders and upstream checks.
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
                .adds(&[DROP_FILES])
                .omits(&[
                    (
                        "localPaths",
                        "upstream's client sends local paths after resolving and stat'ing them; \
                         Neo's host does that from files.paths and reads the files into payloads",
                    ),
                    (
                        "payloads",
                        "carried as files.payloads, or built on Neo's host from files.paths",
                    ),
                    ("streams", "no client file streams in a stateless run"),
                ]),
            drop,
        ),
        ActionOp::Fill => r.op(wire::command("ElementHandle.fill").behaves(FILL_BEHAVES), fill),
        ActionOp::Press => r.op(wire::command("ElementHandle.press"), press),
        ActionOp::PressSequentially => r.op(
            wire::command("ElementHandle.type").behaves(TYPE_BEHAVES),
            type_text,
        ),
        ActionOp::SelectOption => r.op(
            wire::command("ElementHandle.selectOption").retypes(&[(
                "elements",
                "Neo element brands replace upstream channel handles",
            )]),
            select_option,
        ),
        ActionOp::SelectText => r.op(wire::command("ElementHandle.selectText"), select_text),
        ActionOp::Focus => r.op(wire::command("ElementHandle.focus"), focus),
        ActionOp::Blur => r.op(wire::command("Frame.blur"), blur),
        ActionOp::SetInputFiles => r.op(
            wire::command("ElementHandle.setInputFiles")
                .adds(&[UPLOAD_FILES])
                .omits(&[
                    (
                        "directoryStream",
                        "a remote client's upload of a directory it read; Neo's host reads the \
                         directory named in files.paths itself",
                    ),
                    (
                        "localDirectory",
                        "chosen by upstream's client when a path stats as a directory; Neo's host \
                         makes that choice from files.paths",
                    ),
                    (
                        "localPaths",
                        "chosen by upstream's client after it stats each path; Neo's host makes \
                         that choice from files.paths",
                    ),
                    ("payloads", "carried as files.payloads, the script's in-memory files"),
                    ("streams", "no client file streams in a stateless run"),
                ]),
            set_input_files,
        ),
        ActionOp::DispatchEvent => r.op(
            wire::command("ElementHandle.dispatchEvent").retypes(&[(
                "eventInit",
                "JSON init keeps dispatch in Neo's isolated world instead of the page's world",
            )]),
            dispatch_event,
        ),
        ActionOp::ScrollIntoViewIfNeeded => r.op(
            wire::command("ElementHandle.scrollIntoViewIfNeeded"),
            scroll_into_view,
        ),
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

/// `locator.fill(value)` and `clear()`: the redacted text and its action options.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
struct FillInput {
    target: Target,
    strict: Strictness,
    #[ts(type = "string")]
    value: FillValue,
    #[serde(default, deserialize_with = "optional_bool")]
    #[ts(optional)]
    force: Option<bool>,
}
options!(FillInput, flat, positional["value"]);

/// `locator.press(key, options)`: delay is in milliseconds; noWaitAfter controls whether
/// the key runner waits for the navigations requested by its input.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PressInput {
    target: Target,
    strict: Strictness,
    key: Typed,
    #[ts(optional)]
    delay: Option<f64>,
    #[serde(default, deserialize_with = "optional_bool")]
    #[ts(optional)]
    no_wait_after: Option<bool>,
}
options!(PressInput, flat, positional["key"]);

/// `locator.pressSequentially(text, options)` and `type`; neither waits for navigation.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
struct TypeInput {
    target: Target,
    strict: Strictness,
    text: Typed,
    #[ts(optional)]
    delay: Option<f64>,
}
options!(TypeInput, flat, positional["text"]);

/// `locator.selectOption(values)`. `options` contains selected values, not a script option
/// bag: force stays flat on the wire. `elements` are the `<option>` handles among the values.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
struct SelectOptionInput {
    target: Target,
    strict: Strictness,
    #[serde(default, deserialize_with = "optional_selection_list")]
    #[ts(
        optional,
        type = "Array<import('../../../probe/protocol').SelectOption>"
    )]
    options: Option<Vec<SelectOption>>,
    #[serde(default, deserialize_with = "optional_selection_list")]
    #[ts(optional)]
    elements: Option<Vec<Target>>,
    #[serde(default, deserialize_with = "optional_bool")]
    #[ts(optional)]
    force: Option<bool>,
}
options!(SelectOptionInput);

/// Absent selection lists mean empty lists, as upstream defaults them. A present null must
/// still fail decoding rather than silently clearing a selection at the page-effect boundary.
fn optional_selection_list<'de, D, T>(deserializer: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Vec::deserialize(deserializer).map(Some)
}

/// `locator.selectText()`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
struct SelectTextInput {
    target: Target,
    strict: Strictness,
    #[serde(default, deserialize_with = "optional_bool")]
    #[ts(optional)]
    force: Option<bool>,
}
options!(SelectTextInput);

/// `locator.focus`, `blur` and `scrollIntoViewIfNeeded`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "ActionTargetInput")]
struct TargetInput {
    target: Target,
    strict: Strictness,
}
options!(TargetInput);

/// `locator.dispatchEvent(type, eventInit)`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DispatchEventInput {
    target: Target,
    strict: Strictness,
    r#type: String,
    /// JSON in v1; an absent or `null` init is `{}`, as upstream's default.
    #[serde(default)]
    #[ts(type = "unknown")]
    event_init: Value,
}
options!(DispatchEventInput);

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

/// Form edits keep their retry/input-lock runner; only selectOption exposes its selected values.
async fn fill(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: FillInput,
) -> Result<Nothing, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
    let _lease = lease(scope, &page).await?;
    FormRunner::new(
        scope,
        &page,
        &input.target,
        input.strict,
        input.force.unwrap_or(false),
    )
    .run(&FormEdit::Fill(&input.value))
    .await?;
    Ok(())
}

async fn select_option(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: SelectOptionInput,
) -> Result<Vec<String>, PlaywrightError> {
    if input.elements.is_some_and(|elements| !elements.is_empty()) {
        return Err(scope.fail(
            ErrorCode::Unsupported,
            "Neo selects options by value, label or index, not by element handle; pass \
             the option's value or label",
        ));
    }
    let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
    let _lease = lease(scope, &page).await?;
    let options = input.options.unwrap_or_default();
    FormRunner::new(
        scope,
        &page,
        &input.target,
        input.strict,
        input.force.unwrap_or(false),
    )
    .run(&FormEdit::SelectOptions(&options))
    .await
}

async fn select_text(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: SelectTextInput,
) -> Result<Nothing, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
    FormRunner::new(
        scope,
        &page,
        &input.target,
        input.strict,
        input.force.unwrap_or(false),
    )
    .run(&FormEdit::SelectText)
    .await?;
    Ok(())
}

async fn press(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PressInput,
) -> Result<Nothing, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
    let _lease = lease(scope, &page).await?;
    let keys = Keys::Press {
        key: &input.key.0,
        delay: millis(input.delay),
        wait_after: !input.no_wait_after.unwrap_or(false),
    };
    KeyRunner::new(pw, scope, &page, &input.target, input.strict)
        .run(&keys)
        .await?;
    Ok(())
}

async fn type_text(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: TypeInput,
) -> Result<Nothing, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
    let _lease = lease(scope, &page).await?;
    let keys = Keys::Type {
        text: &input.text.0,
        delay: millis(input.delay),
    };
    KeyRunner::new(pw, scope, &page, &input.target, input.strict)
        .run(&keys)
        .await?;
    Ok(())
}

async fn focus(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: TargetInput,
) -> Result<Nothing, PlaywrightError> {
    edit_target(
        pw,
        scope,
        input,
        Edit::Focus {
            reset_selection: false,
        },
    )
    .await
}

async fn blur(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: TargetInput,
) -> Result<Nothing, PlaywrightError> {
    edit_target(pw, scope, input, Edit::Blur).await
}

/// Focus and blur keep their existing read-admission path, without acquiring an input lease.
async fn edit_target(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: TargetInput,
    edit: Edit,
) -> Result<Nothing, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
    ElementRunner::new(scope, &page, &input.target, input.strict)
        .edit(edit)
        .await
}

async fn scroll_into_view(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: TargetInput,
) -> Result<Nothing, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
    ElementRunner::new(scope, &page, &input.target, input.strict)
        .scroll_into_view()
        .await
}

async fn dispatch_event(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: DispatchEventInput,
) -> Result<Nothing, PlaywrightError> {
    // eventInit stays JSON for the isolated-world probe. Normalize the old
    // null/absent default before admission, and reject primitives before any page effect.
    let init = match input.event_init {
        Value::Object(init) => init,
        Value::Null => Map::new(),
        _ => return Err(scope.fail(ErrorCode::InvalidInput, "eventInit: expected an object")),
    };
    let page = OpPage::admit_handle(pw, scope, input.target.page()).await?;
    let _lease = lease(scope, &page).await?;
    ElementRunner::new(scope, &page, &input.target, input.strict)
        .edit(Edit::DispatchEvent {
            r#type: input.r#type,
            init,
        })
        .await
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

#[cfg(test)]
#[path = "form_wire_tests.rs"]
mod form_tests;

#[cfg(test)]
#[path = "wire_reply_tests.rs"]
mod wire_reply;
