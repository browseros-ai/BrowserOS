//! Neo: fixed action labels and admission metadata for every served browser command.
//!
//! Generated commands are covered by exhaustive matches. Neo commands carry their action
//! beside their metadata; routing cannot accept an unlabelled `MethodMeta`. Each served
//! command also fixes how a hook's replacement result is checked against it.

#![deny(clippy::wildcard_enum_match_arm)]

use std::ops::Deref;

use neo_runtime_spec::serve;
use neo_runtime_spec::{CallDecodeError, Command, MethodMeta};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Number, Value};

use crate::browser_representations::bindings;
use crate::browser_representations::clock;
use crate::browser_representations::init_scripts;
use crate::browser_representations::routes;
use crate::method_router;
use crate::playwright_methods::capture;
use crate::playwright_methods::element_ops;
use crate::playwright_methods::evaluate;
use crate::playwright_methods::frame_element;

/// A policy family shared by related action kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionGroup {
    Navigate,
    Pointer,
    Select,
    Type,
    Focus,
    Files,
    PageScript,
    Network,
    Storage,
    Permissions,
    Tabs,
    Dialog,
    Emulation,
    Capture,
    Read,
    Wait,
    Control,
}

impl ActionGroup {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Navigate => "navigate",
            Self::Pointer => "pointer",
            Self::Select => "select",
            Self::Type => "type",
            Self::Focus => "focus",
            Self::Files => "files",
            Self::PageScript => "pageScript",
            Self::Network => "network",
            Self::Storage => "storage",
            Self::Permissions => "permissions",
            Self::Tabs => "tabs",
            Self::Dialog => "dialog",
            Self::Emulation => "emulation",
            Self::Capture => "capture",
            Self::Read => "read",
            Self::Wait => "wait",
            Self::Control => "control",
        }
    }
}

impl Serialize for ActionGroup {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// One action across all its receiver forms, with a fixed policy group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Goto,
    Reload,
    GoBack,
    GoForward,
    Click,
    Dblclick,
    Hover,
    Tap,
    Check,
    Uncheck,
    DragAndDrop,
    Drop,
    Mouse,
    Touch,
    SelectOption,
    Fill,
    Press,
    Type,
    Keyboard,
    Focus,
    Blur,
    SelectText,
    ScrollIntoView,
    SetInputFiles,
    SaveAs,
    Cancel,
    Delete,
    Evaluate,
    WaitForFunction,
    AddScriptTag,
    AddStyleTag,
    AddInitScript,
    ExposeBinding,
    DispatchEvent,
    SetContent,
    Clock,
    Route,
    Abort,
    Continue,
    Fulfill,
    Fetch,
    SetExtraHTTPHeaders,
    SetOffline,
    SetHTTPCredentials,
    AddCookies,
    Cookies,
    StorageState,
    SetStorageState,
    WebStorage,
    GrantPermissions,
    ClearPermissions,
    SetGeolocation,
    NewPage,
    NewContext,
    Close,
    BringToFront,
    RunBeforeUnload,
    Accept,
    Dismiss,
    SetViewportSize,
    EmulateMedia,
    Screenshot,
    Pdf,
    Title,
    Content,
    TextContent,
    InnerText,
    InnerHTML,
    GetAttribute,
    InputValue,
    IsChecked,
    IsVisible,
    IsHidden,
    IsEnabled,
    IsDisabled,
    IsEditable,
    Count,
    AriaSnapshot,
    ResolveSelector,
    BoundingBox,
    FrameElement,
    QuerySelector,
    QuerySelectorAll,
    JsonValue,
    GetProperty,
    GetPropertyList,
    ConsoleMessages,
    PageErrors,
    Requests,
    Response,
    RawRequestHeaders,
    Body,
    SecurityDetails,
    ServerAddr,
    RawResponseHeaders,
    HttpVersion,
    Sizes,
    Path,
    Failure,
    WaitForSelector,
    WaitForTimeout,
    WaitForURL,
    WaitForLoadState,
    WaitForNavigation,
    Pause,
    Debugger,
    SetTestIdAttributeName,
    LocatorHandler,
    Highlight,
    HideHighlight,
    ClearConsoleMessages,
    ClearPageErrors,
    RequestGC,
    Dispose,
    BindingCall,
}

impl ActionKind {
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "the complete kind vocabulary stays exhaustive in one match"
    )]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Goto => "goto",
            Self::Reload => "reload",
            Self::GoBack => "goBack",
            Self::GoForward => "goForward",
            Self::Click => "click",
            Self::Dblclick => "dblclick",
            Self::Hover => "hover",
            Self::Tap => "tap",
            Self::Check => "check",
            Self::Uncheck => "uncheck",
            Self::DragAndDrop => "dragAndDrop",
            Self::Drop => "drop",
            Self::Mouse => "mouse",
            Self::Touch => "touch",
            Self::SelectOption => "selectOption",
            Self::Fill => "fill",
            Self::Press => "press",
            Self::Type => "type",
            Self::Keyboard => "keyboard",
            Self::Focus => "focus",
            Self::Blur => "blur",
            Self::SelectText => "selectText",
            Self::ScrollIntoView => "scrollIntoView",
            Self::SetInputFiles => "setInputFiles",
            Self::SaveAs => "saveAs",
            Self::Cancel => "cancel",
            Self::Delete => "delete",
            Self::Evaluate => "evaluate",
            Self::WaitForFunction => "waitForFunction",
            Self::AddScriptTag => "addScriptTag",
            Self::AddStyleTag => "addStyleTag",
            Self::AddInitScript => "addInitScript",
            Self::ExposeBinding => "exposeBinding",
            Self::DispatchEvent => "dispatchEvent",
            Self::SetContent => "setContent",
            Self::Clock => "clock",
            Self::Route => "route",
            Self::Abort => "abort",
            Self::Continue => "continue",
            Self::Fulfill => "fulfill",
            Self::Fetch => "fetch",
            Self::SetExtraHTTPHeaders => "setExtraHTTPHeaders",
            Self::SetOffline => "setOffline",
            Self::SetHTTPCredentials => "setHTTPCredentials",
            Self::AddCookies => "addCookies",
            Self::Cookies => "cookies",
            Self::StorageState => "storageState",
            Self::SetStorageState => "setStorageState",
            Self::WebStorage => "webStorage",
            Self::GrantPermissions => "grantPermissions",
            Self::ClearPermissions => "clearPermissions",
            Self::SetGeolocation => "setGeolocation",
            Self::NewPage => "newPage",
            Self::NewContext => "newContext",
            Self::Close => "close",
            Self::BringToFront => "bringToFront",
            Self::RunBeforeUnload => "runBeforeUnload",
            Self::Accept => "accept",
            Self::Dismiss => "dismiss",
            Self::SetViewportSize => "setViewportSize",
            Self::EmulateMedia => "emulateMedia",
            Self::Screenshot => "screenshot",
            Self::Pdf => "pdf",
            Self::Title => "title",
            Self::Content => "content",
            Self::TextContent => "textContent",
            Self::InnerText => "innerText",
            Self::InnerHTML => "innerHTML",
            Self::GetAttribute => "getAttribute",
            Self::InputValue => "inputValue",
            Self::IsChecked => "isChecked",
            Self::IsVisible => "isVisible",
            Self::IsHidden => "isHidden",
            Self::IsEnabled => "isEnabled",
            Self::IsDisabled => "isDisabled",
            Self::IsEditable => "isEditable",
            Self::Count => "count",
            Self::AriaSnapshot => "ariaSnapshot",
            Self::ResolveSelector => "resolveSelector",
            Self::BoundingBox => "boundingBox",
            Self::FrameElement => "frameElement",
            Self::QuerySelector => "querySelector",
            Self::QuerySelectorAll => "querySelectorAll",
            Self::JsonValue => "jsonValue",
            Self::GetProperty => "getProperty",
            Self::GetPropertyList => "getPropertyList",
            Self::ConsoleMessages => "consoleMessages",
            Self::PageErrors => "pageErrors",
            Self::Requests => "requests",
            Self::Response => "response",
            Self::RawRequestHeaders => "rawRequestHeaders",
            Self::Body => "body",
            Self::SecurityDetails => "securityDetails",
            Self::ServerAddr => "serverAddr",
            Self::RawResponseHeaders => "rawResponseHeaders",
            Self::HttpVersion => "httpVersion",
            Self::Sizes => "sizes",
            Self::Path => "path",
            Self::Failure => "failure",
            Self::WaitForSelector => "waitForSelector",
            Self::WaitForTimeout => "waitForTimeout",
            Self::WaitForURL => "waitForURL",
            Self::WaitForLoadState => "waitForLoadState",
            Self::WaitForNavigation => "waitForNavigation",
            Self::Pause => "pause",
            Self::Debugger => "debugger",
            Self::SetTestIdAttributeName => "setTestIdAttributeName",
            Self::LocatorHandler => "locatorHandler",
            Self::Highlight => "highlight",
            Self::HideHighlight => "hideHighlight",
            Self::ClearConsoleMessages => "clearConsoleMessages",
            Self::ClearPageErrors => "clearPageErrors",
            Self::RequestGC => "requestGC",
            Self::Dispose => "dispose",
            Self::BindingCall => "bindingCall",
        }
    }

    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "the complete kind vocabulary stays exhaustive in one match"
    )]
    pub const fn group(self) -> ActionGroup {
        match self {
            Self::Goto | Self::Reload | Self::GoBack | Self::GoForward => ActionGroup::Navigate,
            Self::Click
            | Self::Dblclick
            | Self::Hover
            | Self::Tap
            | Self::Check
            | Self::Uncheck
            | Self::DragAndDrop
            | Self::Drop
            | Self::Mouse
            | Self::Touch => ActionGroup::Pointer,
            Self::SelectOption => ActionGroup::Select,
            Self::Fill | Self::Press | Self::Type | Self::Keyboard => ActionGroup::Type,
            Self::Focus | Self::Blur | Self::SelectText | Self::ScrollIntoView => {
                ActionGroup::Focus
            }
            Self::SetInputFiles | Self::SaveAs | Self::Cancel | Self::Delete => ActionGroup::Files,
            Self::Evaluate
            | Self::WaitForFunction
            | Self::AddScriptTag
            | Self::AddStyleTag
            | Self::AddInitScript
            | Self::ExposeBinding
            | Self::DispatchEvent
            | Self::SetContent
            | Self::Clock => ActionGroup::PageScript,
            Self::Route
            | Self::Abort
            | Self::Continue
            | Self::Fulfill
            | Self::Fetch
            | Self::SetExtraHTTPHeaders
            | Self::SetOffline
            | Self::SetHTTPCredentials => ActionGroup::Network,
            Self::AddCookies
            | Self::Cookies
            | Self::StorageState
            | Self::SetStorageState
            | Self::WebStorage => ActionGroup::Storage,
            Self::GrantPermissions | Self::ClearPermissions | Self::SetGeolocation => {
                ActionGroup::Permissions
            }
            Self::NewPage
            | Self::NewContext
            | Self::Close
            | Self::BringToFront
            | Self::RunBeforeUnload => ActionGroup::Tabs,
            Self::Accept | Self::Dismiss => ActionGroup::Dialog,
            Self::SetViewportSize | Self::EmulateMedia => ActionGroup::Emulation,
            Self::Screenshot | Self::Pdf => ActionGroup::Capture,
            Self::Title
            | Self::Content
            | Self::TextContent
            | Self::InnerText
            | Self::InnerHTML
            | Self::GetAttribute
            | Self::InputValue
            | Self::IsChecked
            | Self::IsVisible
            | Self::IsHidden
            | Self::IsEnabled
            | Self::IsDisabled
            | Self::IsEditable
            | Self::Count
            | Self::AriaSnapshot
            | Self::ResolveSelector
            | Self::BoundingBox
            | Self::FrameElement
            | Self::QuerySelector
            | Self::QuerySelectorAll
            | Self::JsonValue
            | Self::GetProperty
            | Self::GetPropertyList
            | Self::ConsoleMessages
            | Self::PageErrors
            | Self::Requests
            | Self::Response
            | Self::RawRequestHeaders
            | Self::Body
            | Self::SecurityDetails
            | Self::ServerAddr
            | Self::RawResponseHeaders
            | Self::HttpVersion
            | Self::Sizes
            | Self::Path
            | Self::Failure => ActionGroup::Read,
            Self::WaitForSelector
            | Self::WaitForTimeout
            | Self::WaitForURL
            | Self::WaitForLoadState
            | Self::WaitForNavigation => ActionGroup::Wait,
            Self::Pause
            | Self::Debugger
            | Self::SetTestIdAttributeName
            | Self::LocatorHandler
            | Self::Highlight
            | Self::HideHighlight
            | Self::ClearConsoleMessages
            | Self::ClearPageErrors
            | Self::RequestGC
            | Self::Dispose
            | Self::BindingCall => ActionGroup::Control,
        }
    }
}

impl Serialize for ActionKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// A command's kind and whether it targets an element: its params require a selector (or
/// `source`/`target` selectors) that this call resolves, or its receiver is an `ElementHandle`.
/// `registerLocatorHandler` requires a selector but only stores it, so it does not target one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Action {
    kind: ActionKind,
    element: bool,
}

impl Action {
    const fn new(kind: ActionKind, element: bool) -> Self {
        Self { kind, element }
    }

    #[must_use]
    pub const fn kind(self) -> ActionKind {
        self.kind
    }

    #[must_use]
    pub const fn group(self) -> ActionGroup {
        self.kind.group()
    }

    #[must_use]
    pub const fn targets_element(self) -> bool {
        self.element
    }

    /// Whether this is an element action in design.md's sense: input (pointer, typing, select,
    /// focus or files) aimed at an element the call resolves. Its hooks fire once the element is
    /// found (F4); every other action's hooks fire before the call starts (F3), reads and page
    /// scripts that name a selector included.
    #[must_use]
    pub const fn element_action(self) -> bool {
        self.element
            && matches!(
                self.kind.group(),
                ActionGroup::Pointer
                    | ActionGroup::Type
                    | ActionGroup::Select
                    | ActionGroup::Focus
                    | ActionGroup::Files
            )
    }
}

/// Why a hook's replacement result does not fit its command (the detail goes to the hook trace).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResultShapeError(pub(crate) String);

/// What a hook's replacement result is checked against, fixed per served command.
#[derive(Clone, Copy)]
enum ResultShape {
    /// A generated command that replies a `result`: the replacement must decode as its
    /// `Result` type ([`generated_result`]).
    Generated(fn(&Value) -> Result<Value, String>),
    /// A generated command whose reply has no `result` (`Command::VOID_RESULT`).
    Void,
    /// A Neo command, which has no generated result type ([`like_actual`]).
    Neo,
}

impl ResultShape {
    /// Generated command `P`'s shape. Requiring `Deserialize` here makes every generated row
    /// prove its result type can be decoded, or the table does not compile.
    const fn generated<P>() -> Self
    where
        P: Command,
        P::Result: DeserializeOwned,
    {
        if P::VOID_RESULT {
            Self::Void
        } else {
            Self::Generated(generated_result::<P>)
        }
    }
}

/// Decodes `replaced` as generated command `P`'s result and encodes it again. A key the type
/// does not define, or `null` for an absent optional field, would vanish in that round trip,
/// so the replacement is refused rather than sent as a different value than the hook returned.
fn generated_result<P>(replaced: &Value) -> Result<Value, String>
where
    P: Command,
    P::Result: DeserializeOwned,
{
    let decoded = P::Result::deserialize(replaced)
        .map_err(|error| format!("not its result type: {error}"))?;
    let encoded =
        serde_json::to_value(decoded).map_err(|error| format!("cannot encode: {error}"))?;
    if same_json(&encoded, replaced) {
        Ok(encoded)
    } else {
        Err("a key its result type does not define, or null for an absent one".to_owned())
    }
}

/// Whether `a` and `b` are the same JSON. Numbers compare by value: JavaScript has one number
/// type, so a hook's whole number arrives as `1` (`JSON.stringify`) where a float field
/// encodes `1.0`. Recursion is bounded by the depth of the hook's JSON.
fn same_json(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => same_number(a, b),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_json(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_some_and(|b| same_json(a, b)))
        }
        _ => a == b,
    }
}

/// Integers compare exactly; an integer and a float compare as floats.
fn same_number(a: &Number, b: &Number) -> bool {
    if let (Some(a), Some(b)) = (a.as_i64(), b.as_i64()) {
        return a == b;
    }
    if let (Some(a), Some(b)) = (a.as_u64(), b.as_u64()) {
        return a == b;
    }
    a.as_f64() == b.as_f64()
}

/// A Neo command's check, with `actual` as the reference: the same JSON kind and, for an
/// object, the same keys with values of the same kinds. Values below the top level (such as
/// an evaluate's serialized value) are free.
fn like_actual(replaced: Value, actual: &Value) -> Result<Value, String> {
    if kind(&replaced) != kind(actual) {
        return Err(format!(
            "{} where the result is {}",
            kind(&replaced),
            kind(actual)
        ));
    }
    if let (Value::Object(fields), Value::Object(expected)) = (&replaced, actual) {
        let keys = |map: &Map<String, Value>| {
            map.keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let other_keys = || {
            format!(
                "keys [{}] where the result has [{}]",
                keys(fields),
                keys(expected)
            )
        };
        if fields.len() != expected.len() {
            return Err(other_keys());
        }
        for (key, value) in expected {
            let Some(field) = fields.get(key) else {
                return Err(other_keys());
            };
            if kind(field) != kind(value) {
                return Err(format!(
                    "`{key}` is {} where the result's is {}",
                    kind(field),
                    kind(value)
                ));
            }
        }
    }
    Ok(replaced)
}

/// A JSON value's kind, as the hook trace names it.
fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Metadata for a command outside the generated spec. An action is mandatory at declaration.
pub(crate) struct NeoMethod {
    meta: MethodMeta,
    action: Action,
}

impl NeoMethod {
    pub(crate) const fn new(meta: MethodMeta, kind: ActionKind, element: bool) -> Self {
        Self {
            meta,
            action: Action::new(kind, element),
        }
    }
}

impl Deref for NeoMethod {
    type Target = MethodMeta;

    fn deref(&self) -> &Self::Target {
        &self.meta
    }
}

/// The router's admission token: metadata that cannot exist without its action and the shape
/// a hook's replacement result must have.
#[derive(Clone, Copy)]
pub(crate) struct ServedMethod {
    meta: &'static MethodMeta,
    action: Action,
    result: ResultShape,
}

impl ServedMethod {
    pub(crate) const fn meta(self) -> &'static MethodMeta {
        self.meta
    }

    pub(crate) const fn action(self) -> Action {
        self.action
    }

    /// The reply `result` to send instead of `actual`, the one the action produced (`None`
    /// when the reply has no `result`), if the hook's `replaced` has this command's result
    /// shape. design.md, Error handling: "Replaced results must have the method's result
    /// shape; otherwise the call fails."
    ///
    /// A generated command's replacement must decode as its `Result` type and encode back to
    /// the same JSON (numbers compared by value); the encoded value is sent. A Neo command's
    /// replacement must keep `actual`'s top-level shape and is sent as it is. A command that
    /// replies no `result` has nothing to replace, so this never returns `Ok(None)`.
    ///
    /// # Errors
    ///
    /// When the replacement does not have the shape; the message is for the hook trace.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the F3 hook call path calls it; until then only tests do"
        )
    )]
    pub(crate) fn replacement_result(
        self,
        replaced: Value,
        actual: Option<&Value>,
    ) -> Result<Option<Value>, ResultShapeError> {
        let checked = match (self.result, actual) {
            (ResultShape::Void, _) | (_, None) => Err("replies no result to replace".to_owned()),
            (ResultShape::Generated(check), Some(_)) => check(&replaced),
            (ResultShape::Neo, Some(actual)) => like_actual(replaced, actual),
        };
        checked.map(Some).map_err(|reason| {
            ResultShapeError(format!(
                "{}.{}: {reason}",
                self.meta.interface, self.meta.method
            ))
        })
    }

    pub(crate) fn decode_error(error: &CallDecodeError) -> Option<Self> {
        match error {
            CallDecodeError::InvalidParams { meta, .. } => classify(meta),
            CallDecodeError::UnknownMethod { .. } => None,
        }
    }
}

impl Deref for ServedMethod {
    type Target = MethodMeta;

    fn deref(&self) -> &Self::Target {
        self.meta
    }
}

impl From<&'static NeoMethod> for ServedMethod {
    fn from(method: &'static NeoMethod) -> Self {
        Self {
            meta: &method.meta,
            action: method.action,
            result: ResultShape::Neo,
        }
    }
}

/// Only generated, sealed `Command` types can supply metadata for a spec routing extension
/// (such as `selectOption` with handles). Implementations come from the exhaustive table.
pub(crate) trait SpecCommand: Command {
    /// Neo: typed metadata for the command's row in the exhaustive action table.
    const METHOD: ServedMethod;
}

macro_rules! spec_actions {
    ($table:ident, $call:ident { $($variant:ident ($params:ident) => $kind:ident $(, $element:ident)?;)* }) => {
        impl From<&serve::$call> for ServedMethod {
            fn from(call: &serve::$call) -> Self {
                match call {
                    $(serve::$call::$variant(_) => <serve::$params as SpecCommand>::METHOD,)*
                }
            }
        }

        $(impl SpecCommand for serve::$params {
            /// Neo: this generated command's mandatory action classification and result shape.
            const METHOD: ServedMethod = ServedMethod {
                meta: Self::META,
                action: Action::new(ActionKind::$kind, spec_actions!(@element $($element)?)),
                result: ResultShape::generated::<Self>(),
            };
        })*

        /// Neo: the labelled commands in one generated call enum, shared with the classifier.
        const $table: &[ServedMethod] = &[$(<serve::$params as SpecCommand>::METHOD,)*];
    };
    (@element element) => { true };
    (@element) => { false };
}

spec_actions!(BROWSER_CONTEXT_ACTIONS, BrowserContextCall {
    NewPage(BrowserContextNewPageParams) => NewPage;
    Close(BrowserContextCloseParams) => Close;
    Pause(BrowserContextPauseParams) => Pause;
    SetTestIdAttributeName(BrowserContextSetTestIdAttributeNameParams) => SetTestIdAttributeName;
    AddCookies(BrowserContextAddCookiesParams) => AddCookies;
    Cookies(BrowserContextCookiesParams) => Cookies;
    StorageState(BrowserContextStorageStateParams) => StorageState;
    SetStorageState(BrowserContextSetStorageStateParams) => SetStorageState;
    SetExtraHTTPHeaders(BrowserContextSetExtraHTTPHeadersParams) => SetExtraHTTPHeaders;
    SetGeolocation(BrowserContextSetGeolocationParams) => SetGeolocation;
    SetOffline(BrowserContextSetOfflineParams) => SetOffline;
    SetHTTPCredentials(BrowserContextSetHTTPCredentialsParams) => SetHTTPCredentials;
    GrantPermissions(BrowserContextGrantPermissionsParams) => GrantPermissions;
    ClearPermissions(BrowserContextClearPermissionsParams) => ClearPermissions;
    SetNetworkInterceptionPatterns(BrowserContextSetNetworkInterceptionPatternsParams) => Route;
});

spec_actions!(BROWSER_ACTIONS, BrowserCall {
    NewContext(BrowserNewContextParams) => NewContext;
    Close(BrowserCloseParams) => Close;
});

spec_actions!(DEBUGGER_ACTIONS, DebuggerCall {
    RequestPause(DebuggerRequestPauseParams) => Debugger;
    Resume(DebuggerResumeParams) => Debugger;
    Next(DebuggerNextParams) => Debugger;
    RunTo(DebuggerRunToParams) => Debugger;
});

spec_actions!(PAGE_ACTIONS, PageCall {
    Close(PageCloseParams) => Close;
    RunBeforeUnload(PageRunBeforeUnloadParams) => RunBeforeUnload;
    BringToFront(PageBringToFrontParams) => BringToFront;
    SetViewportSize(PageSetViewportSizeParams) => SetViewportSize;
    EmulateMedia(PageEmulateMediaParams) => EmulateMedia;
    RequestGC(PageRequestGCParams) => RequestGC;
    SetExtraHTTPHeaders(PageSetExtraHTTPHeadersParams) => SetExtraHTTPHeaders;
    HideHighlight(PageHideHighlightParams) => HideHighlight;
    KeyboardDown(PageKeyboardDownParams) => Keyboard;
    KeyboardUp(PageKeyboardUpParams) => Keyboard;
    KeyboardInsertText(PageKeyboardInsertTextParams) => Keyboard;
    KeyboardType(PageKeyboardTypeParams) => Keyboard;
    KeyboardPress(PageKeyboardPressParams) => Keyboard;
    MouseMove(PageMouseMoveParams) => Mouse;
    MouseDown(PageMouseDownParams) => Mouse;
    MouseUp(PageMouseUpParams) => Mouse;
    MouseClick(PageMouseClickParams) => Mouse;
    MouseWheel(PageMouseWheelParams) => Mouse;
    TouchscreenTap(PageTouchscreenTapParams) => Touch;
    Reload(PageReloadParams) => Reload;
    GoBack(PageGoBackParams) => GoBack;
    GoForward(PageGoForwardParams) => GoForward;
    Screenshot(PageScreenshotParams) => Screenshot;
    Pdf(PagePdfParams) => Pdf;
    ConsoleMessages(PageConsoleMessagesParams) => ConsoleMessages;
    PageErrors(PagePageErrorsParams) => PageErrors;
    ClearConsoleMessages(PageClearConsoleMessagesParams) => ClearConsoleMessages;
    ClearPageErrors(PageClearPageErrorsParams) => ClearPageErrors;
    Requests(PageRequestsParams) => Requests;
    // Stores its selector for later; no element is resolved by this call.
    RegisterLocatorHandler(PageRegisterLocatorHandlerParams) => LocatorHandler;
    ResolveLocatorHandlerNoReply(PageResolveLocatorHandlerNoReplyParams) => LocatorHandler;
    UnregisterLocatorHandler(PageUnregisterLocatorHandlerParams) => LocatorHandler;
    SetNetworkInterceptionPatterns(PageSetNetworkInterceptionPatternsParams) => Route;
    WebStorageItems(PageWebStorageItemsParams) => WebStorage;
    WebStorageGetItem(PageWebStorageGetItemParams) => WebStorage;
    WebStorageSetItem(PageWebStorageSetItemParams) => WebStorage;
    WebStorageRemoveItem(PageWebStorageRemoveItemParams) => WebStorage;
    WebStorageClear(PageWebStorageClearParams) => WebStorage;
});

spec_actions!(ROUTE_ACTIONS, RouteCall {
    Abort(RouteAbortParams) => Abort;
    Continue(RouteContinueParams) => Continue;
    Fulfill(RouteFulfillParams) => Fulfill;
});

spec_actions!(DIALOG_ACTIONS, DialogCall {
    Accept(DialogAcceptParams) => Accept;
    Dismiss(DialogDismissParams) => Dismiss;
});

spec_actions!(ARTIFACT_ACTIONS, ArtifactCall {
    PathAfterFinished(ArtifactPathAfterFinishedParams) => Path;
    SaveAs(ArtifactSaveAsParams) => SaveAs;
    Failure(ArtifactFailureParams) => Failure;
    Cancel(ArtifactCancelParams) => Cancel;
    Delete(ArtifactDeleteParams) => Delete;
});

spec_actions!(REQUEST_ACTIONS, RequestCall {
    Response(RequestResponseParams) => Response;
    RawRequestHeaders(RequestRawRequestHeadersParams) => RawRequestHeaders;
});

spec_actions!(RESPONSE_ACTIONS, ResponseCall {
    Body(ResponseBodyParams) => Body;
    SecurityDetails(ResponseSecurityDetailsParams) => SecurityDetails;
    ServerAddr(ResponseServerAddrParams) => ServerAddr;
    RawResponseHeaders(ResponseRawResponseHeadersParams) => RawResponseHeaders;
    HttpVersion(ResponseHttpVersionParams) => HttpVersion;
    Sizes(ResponseSizesParams) => Sizes;
});

spec_actions!(FRAME_ACTIONS, FrameCall {
    Goto(FrameGotoParams) => Goto;
    Title(FrameTitleParams) => Title;
    TextContent(FrameTextContentParams) => TextContent, element;
    AriaSnapshot(FrameAriaSnapshotParams) => AriaSnapshot;
    Click(FrameClickParams) => Click, element;
    Dblclick(FrameDblclickParams) => Dblclick, element;
    Hover(FrameHoverParams) => Hover, element;
    Tap(FrameTapParams) => Tap, element;
    Fill(FrameFillParams) => Fill, element;
    Press(FramePressParams) => Press, element;
    Type(FrameTypeParams) => Type, element;
    DragAndDrop(FrameDragAndDropParams) => DragAndDrop, element;
    InputValue(FrameInputValueParams) => InputValue, element;
    IsChecked(FrameIsCheckedParams) => IsChecked, element;
    IsVisible(FrameIsVisibleParams) => IsVisible, element;
    Check(FrameCheckParams) => Check, element;
    Uncheck(FrameUncheckParams) => Uncheck, element;
    SelectOption(FrameSelectOptionParams) => SelectOption, element;
    SetInputFiles(FrameSetInputFilesParams) => SetInputFiles, element;
    Drop(FrameDropParams) => Drop, element;
    WaitForSelector(FrameWaitForSelectorParams) => WaitForSelector, element;
    InnerText(FrameInnerTextParams) => InnerText, element;
    GetAttribute(FrameGetAttributeParams) => GetAttribute, element;
    QueryCount(FrameQueryCountParams) => Count, element;
    IsEnabled(FrameIsEnabledParams) => IsEnabled, element;
    IsDisabled(FrameIsDisabledParams) => IsDisabled, element;
    IsEditable(FrameIsEditableParams) => IsEditable, element;
    IsHidden(FrameIsHiddenParams) => IsHidden, element;
    InnerHTML(FrameInnerHTMLParams) => InnerHTML, element;
    Content(FrameContentParams) => Content;
    SetContent(FrameSetContentParams) => SetContent;
    WaitForTimeout(FrameWaitForTimeoutParams) => WaitForTimeout;
    ResolveSelector(FrameResolveSelectorParams) => ResolveSelector, element;
    AriaSnapshotJSON(FrameAriaSnapshotJSONParams) => AriaSnapshot;
    Focus(FrameFocusParams) => Focus, element;
    Blur(FrameBlurParams) => Blur, element;
    DispatchEvent(FrameDispatchEventParams) => DispatchEvent, element;
    Highlight(FrameHighlightParams) => Highlight, element;
    HideHighlight(FrameHideHighlightParams) => HideHighlight, element;
});

/// Neo: commands implemented outside the spec; their declarations require an action.
const NEO_METHODS: &[&NeoMethod] = &[
    &method_router::WAIT_FOR_URL_META,
    &method_router::WAIT_FOR_LOAD_STATE_META,
    &method_router::WAIT_FOR_NAVIGATION_META,
    &method_router::NEXT_NAVIGATION_META,
    &method_router::FINISH_NAVIGATION_META,
    &method_router::RELEASE_NAVIGATION_META,
    &init_scripts::PAGE_ADD_INIT_SCRIPT_META,
    &init_scripts::CONTEXT_ADD_INIT_SCRIPT_META,
    &init_scripts::DISPOSABLE_DISPOSE_META,
    &bindings::PAGE_EXPOSE_BINDING_META,
    &bindings::CONTEXT_EXPOSE_BINDING_META,
    &bindings::BINDING_CALL_RESOLVE_META,
    &bindings::BINDING_CALL_REJECT_META,
    &routes::FETCH_META,
    &clock::CLOCK_INSTALL_META,
    &clock::CLOCK_FAST_FORWARD_META,
    &clock::CLOCK_PAUSE_AT_META,
    &clock::CLOCK_RESUME_META,
    &clock::CLOCK_RUN_FOR_META,
    &clock::CLOCK_SET_FIXED_TIME_META,
    &clock::CLOCK_SET_SYSTEM_TIME_META,
    &evaluate::FRAME_EVALUATE_META,
    &evaluate::FRAME_EVALUATE_HANDLE_META,
    &evaluate::HANDLE_EVALUATE_META,
    &evaluate::HANDLE_EVALUATE_HANDLE_META,
    &evaluate::HANDLE_JSON_VALUE_META,
    &evaluate::HANDLE_DISPOSE_META,
    &evaluate::FRAME_ADD_SCRIPT_TAG_META,
    &evaluate::FRAME_ADD_STYLE_TAG_META,
    &evaluate::HANDLE_DISPATCH_EVENT_META,
    &evaluate::FRAME_QUERY_META,
    &evaluate::FRAME_QUERY_ALL_META,
    &evaluate::FRAME_EVAL_ON_SELECTOR_META,
    &evaluate::FRAME_EVAL_ON_SELECTOR_ALL_META,
    &evaluate::FRAME_WAIT_FOR_FUNCTION_META,
    &evaluate::ELEMENT_QUERY_META,
    &evaluate::ELEMENT_QUERY_ALL_META,
    &evaluate::ELEMENT_EVAL_ON_SELECTOR_META,
    &evaluate::ELEMENT_EVAL_ON_SELECTOR_ALL_META,
    &evaluate::HANDLE_GET_PROPERTY_META,
    &evaluate::HANDLE_GET_PROPERTY_LIST_META,
    &element_ops::SCROLL_INTO_VIEW_META,
    &element_ops::SELECT_TEXT_META,
    &element_ops::BOUNDING_BOX_META,
    &capture::NEO_SCREENSHOT_META,
    &capture::ELEMENT_HANDLE_SCREENSHOT_META,
    &frame_element::FRAME_ELEMENT_META,
];

/// Every served command, including commands whose params failed to decode.
pub(crate) fn served() -> impl Iterator<Item = ServedMethod> {
    [
        BROWSER_CONTEXT_ACTIONS,
        BROWSER_ACTIONS,
        DEBUGGER_ACTIONS,
        PAGE_ACTIONS,
        ROUTE_ACTIONS,
        DIALOG_ACTIONS,
        ARTIFACT_ACTIONS,
        REQUEST_ACTIONS,
        RESPONSE_ACTIONS,
        FRAME_ACTIONS,
    ]
    .into_iter()
    .flatten()
    .copied()
    .chain(NEO_METHODS.iter().copied().map(ServedMethod::from))
}

/// Unknown commands have no label; argument values never participate in classification.
pub(crate) fn classify(meta: &MethodMeta) -> Option<ServedMethod> {
    served().find(|known| known.interface == meta.interface && known.method == meta.method)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use neo_runtime_spec::serve;
    use serde_json::{Value, json};

    use super::{ResultShape, ServedMethod, SpecCommand, classify, served};
    use crate::playwright_methods::evaluate;

    /// Neo: the independent literal contract for all 163 served command labels.
    const EXPECTED: &[(&str, &str, &str, &str, bool)] = &[
        ("Artifact", "cancel", "cancel", "files", false),
        ("Artifact", "delete", "delete", "files", false),
        ("Artifact", "failure", "failure", "read", false),
        ("Artifact", "pathAfterFinished", "path", "read", false),
        ("Artifact", "saveAs", "saveAs", "files", false),
        ("BindingCall", "reject", "bindingCall", "control", false),
        ("BindingCall", "resolve", "bindingCall", "control", false),
        ("Browser", "close", "close", "tabs", false),
        ("Browser", "newContext", "newContext", "tabs", false),
        (
            "BrowserContext",
            "addCookies",
            "addCookies",
            "storage",
            false,
        ),
        (
            "BrowserContext",
            "addInitScript",
            "addInitScript",
            "pageScript",
            false,
        ),
        (
            "BrowserContext",
            "clearPermissions",
            "clearPermissions",
            "permissions",
            false,
        ),
        (
            "BrowserContext",
            "clockFastForward",
            "clock",
            "pageScript",
            false,
        ),
        (
            "BrowserContext",
            "clockInstall",
            "clock",
            "pageScript",
            false,
        ),
        (
            "BrowserContext",
            "clockPauseAt",
            "clock",
            "pageScript",
            false,
        ),
        (
            "BrowserContext",
            "clockResume",
            "clock",
            "pageScript",
            false,
        ),
        (
            "BrowserContext",
            "clockRunFor",
            "clock",
            "pageScript",
            false,
        ),
        (
            "BrowserContext",
            "clockSetFixedTime",
            "clock",
            "pageScript",
            false,
        ),
        (
            "BrowserContext",
            "clockSetSystemTime",
            "clock",
            "pageScript",
            false,
        ),
        ("BrowserContext", "close", "close", "tabs", false),
        ("BrowserContext", "cookies", "cookies", "storage", false),
        (
            "BrowserContext",
            "exposeBinding",
            "exposeBinding",
            "pageScript",
            false,
        ),
        (
            "BrowserContext",
            "grantPermissions",
            "grantPermissions",
            "permissions",
            false,
        ),
        ("BrowserContext", "newPage", "newPage", "tabs", false),
        ("BrowserContext", "pause", "pause", "control", false),
        (
            "BrowserContext",
            "setExtraHTTPHeaders",
            "setExtraHTTPHeaders",
            "network",
            false,
        ),
        (
            "BrowserContext",
            "setGeolocation",
            "setGeolocation",
            "permissions",
            false,
        ),
        (
            "BrowserContext",
            "setHTTPCredentials",
            "setHTTPCredentials",
            "network",
            false,
        ),
        (
            "BrowserContext",
            "setNetworkInterceptionPatterns",
            "route",
            "network",
            false,
        ),
        (
            "BrowserContext",
            "setOffline",
            "setOffline",
            "network",
            false,
        ),
        (
            "BrowserContext",
            "setStorageState",
            "setStorageState",
            "storage",
            false,
        ),
        (
            "BrowserContext",
            "setTestIdAttributeName",
            "setTestIdAttributeName",
            "control",
            false,
        ),
        (
            "BrowserContext",
            "storageState",
            "storageState",
            "storage",
            false,
        ),
        ("Debugger", "next", "debugger", "control", false),
        ("Debugger", "requestPause", "debugger", "control", false),
        ("Debugger", "resume", "debugger", "control", false),
        ("Debugger", "runTo", "debugger", "control", false),
        ("Dialog", "accept", "accept", "dialog", false),
        ("Dialog", "dismiss", "dismiss", "dialog", false),
        ("Disposable", "dispose", "dispose", "control", false),
        (
            "ElementHandle",
            "dispatchEvent",
            "dispatchEvent",
            "pageScript",
            true,
        ),
        (
            "ElementHandle",
            "evalOnSelector",
            "evaluate",
            "pageScript",
            true,
        ),
        (
            "ElementHandle",
            "evalOnSelectorAll",
            "evaluate",
            "pageScript",
            true,
        ),
        (
            "ElementHandle",
            "querySelector",
            "querySelector",
            "read",
            true,
        ),
        (
            "ElementHandle",
            "querySelectorAll",
            "querySelectorAll",
            "read",
            true,
        ),
        ("ElementHandle", "screenshot", "screenshot", "capture", true),
        ("Frame", "addScriptTag", "addScriptTag", "pageScript", false),
        ("Frame", "addStyleTag", "addStyleTag", "pageScript", false),
        ("Frame", "ariaSnapshot", "ariaSnapshot", "read", false),
        ("Frame", "ariaSnapshotJSON", "ariaSnapshot", "read", false),
        ("Frame", "blur", "blur", "focus", true),
        ("Frame", "check", "check", "pointer", true),
        ("Frame", "click", "click", "pointer", true),
        ("Frame", "content", "content", "read", false),
        ("Frame", "dblclick", "dblclick", "pointer", true),
        (
            "Frame",
            "dispatchEvent",
            "dispatchEvent",
            "pageScript",
            true,
        ),
        ("Frame", "dragAndDrop", "dragAndDrop", "pointer", true),
        ("Frame", "drop", "drop", "pointer", true),
        ("Frame", "evalOnSelector", "evaluate", "pageScript", true),
        ("Frame", "evalOnSelectorAll", "evaluate", "pageScript", true),
        (
            "Frame",
            "evaluateExpression",
            "evaluate",
            "pageScript",
            false,
        ),
        (
            "Frame",
            "evaluateExpressionHandle",
            "evaluate",
            "pageScript",
            false,
        ),
        ("Frame", "fill", "fill", "type", true),
        ("Frame", "focus", "focus", "focus", true),
        ("Frame", "frameElement", "frameElement", "read", false),
        ("Frame", "getAttribute", "getAttribute", "read", true),
        ("Frame", "goto", "goto", "navigate", false),
        ("Frame", "hideHighlight", "hideHighlight", "control", true),
        ("Frame", "highlight", "highlight", "control", true),
        ("Frame", "hover", "hover", "pointer", true),
        ("Frame", "innerHTML", "innerHTML", "read", true),
        ("Frame", "innerText", "innerText", "read", true),
        ("Frame", "inputValue", "inputValue", "read", true),
        ("Frame", "isChecked", "isChecked", "read", true),
        ("Frame", "isDisabled", "isDisabled", "read", true),
        ("Frame", "isEditable", "isEditable", "read", true),
        ("Frame", "isEnabled", "isEnabled", "read", true),
        ("Frame", "isHidden", "isHidden", "read", true),
        ("Frame", "isVisible", "isVisible", "read", true),
        ("Frame", "neo:boundingBox", "boundingBox", "read", true),
        (
            "Frame",
            "neo:finishNavigation",
            "waitForNavigation",
            "wait",
            false,
        ),
        (
            "Frame",
            "neo:nextNavigation",
            "waitForNavigation",
            "wait",
            false,
        ),
        (
            "Frame",
            "neo:releaseNavigation",
            "waitForNavigation",
            "wait",
            false,
        ),
        ("Frame", "neo:screenshot", "screenshot", "capture", true),
        (
            "Frame",
            "neo:scrollIntoViewIfNeeded",
            "scrollIntoView",
            "focus",
            true,
        ),
        ("Frame", "neo:selectText", "selectText", "focus", true),
        (
            "Frame",
            "neo:waitForLoadState",
            "waitForLoadState",
            "wait",
            false,
        ),
        (
            "Frame",
            "neo:waitForNavigation",
            "waitForNavigation",
            "wait",
            false,
        ),
        ("Frame", "neo:waitForURL", "waitForURL", "wait", false),
        ("Frame", "press", "press", "type", true),
        ("Frame", "queryCount", "count", "read", true),
        ("Frame", "querySelector", "querySelector", "read", true),
        (
            "Frame",
            "querySelectorAll",
            "querySelectorAll",
            "read",
            true,
        ),
        ("Frame", "resolveSelector", "resolveSelector", "read", true),
        ("Frame", "selectOption", "selectOption", "select", true),
        ("Frame", "setContent", "setContent", "pageScript", false),
        ("Frame", "setInputFiles", "setInputFiles", "files", true),
        ("Frame", "tap", "tap", "pointer", true),
        ("Frame", "textContent", "textContent", "read", true),
        ("Frame", "title", "title", "read", false),
        ("Frame", "type", "type", "type", true),
        ("Frame", "uncheck", "uncheck", "pointer", true),
        (
            "Frame",
            "waitForFunction",
            "waitForFunction",
            "pageScript",
            false,
        ),
        ("Frame", "waitForSelector", "waitForSelector", "wait", true),
        ("Frame", "waitForTimeout", "waitForTimeout", "wait", false),
        ("JSHandle", "dispose", "dispose", "control", false),
        (
            "JSHandle",
            "evaluateExpression",
            "evaluate",
            "pageScript",
            false,
        ),
        (
            "JSHandle",
            "evaluateExpressionHandle",
            "evaluate",
            "pageScript",
            false,
        ),
        ("JSHandle", "getProperty", "getProperty", "read", false),
        (
            "JSHandle",
            "getPropertyList",
            "getPropertyList",
            "read",
            false,
        ),
        ("JSHandle", "jsonValue", "jsonValue", "read", false),
        (
            "Page",
            "addInitScript",
            "addInitScript",
            "pageScript",
            false,
        ),
        ("Page", "bringToFront", "bringToFront", "tabs", false),
        (
            "Page",
            "clearConsoleMessages",
            "clearConsoleMessages",
            "control",
            false,
        ),
        (
            "Page",
            "clearPageErrors",
            "clearPageErrors",
            "control",
            false,
        ),
        ("Page", "close", "close", "tabs", false),
        ("Page", "consoleMessages", "consoleMessages", "read", false),
        ("Page", "emulateMedia", "emulateMedia", "emulation", false),
        (
            "Page",
            "exposeBinding",
            "exposeBinding",
            "pageScript",
            false,
        ),
        ("Page", "goBack", "goBack", "navigate", false),
        ("Page", "goForward", "goForward", "navigate", false),
        ("Page", "hideHighlight", "hideHighlight", "control", false),
        ("Page", "keyboardDown", "keyboard", "type", false),
        ("Page", "keyboardInsertText", "keyboard", "type", false),
        ("Page", "keyboardPress", "keyboard", "type", false),
        ("Page", "keyboardType", "keyboard", "type", false),
        ("Page", "keyboardUp", "keyboard", "type", false),
        ("Page", "mouseClick", "mouse", "pointer", false),
        ("Page", "mouseDown", "mouse", "pointer", false),
        ("Page", "mouseMove", "mouse", "pointer", false),
        ("Page", "mouseUp", "mouse", "pointer", false),
        ("Page", "mouseWheel", "mouse", "pointer", false),
        ("Page", "pageErrors", "pageErrors", "read", false),
        ("Page", "pdf", "pdf", "capture", false),
        (
            "Page",
            "registerLocatorHandler",
            "locatorHandler",
            "control",
            false,
        ),
        ("Page", "reload", "reload", "navigate", false),
        ("Page", "requestGC", "requestGC", "control", false),
        ("Page", "requests", "requests", "read", false),
        (
            "Page",
            "resolveLocatorHandlerNoReply",
            "locatorHandler",
            "control",
            false,
        ),
        ("Page", "runBeforeUnload", "runBeforeUnload", "tabs", false),
        ("Page", "screenshot", "screenshot", "capture", false),
        (
            "Page",
            "setExtraHTTPHeaders",
            "setExtraHTTPHeaders",
            "network",
            false,
        ),
        (
            "Page",
            "setNetworkInterceptionPatterns",
            "route",
            "network",
            false,
        ),
        (
            "Page",
            "setViewportSize",
            "setViewportSize",
            "emulation",
            false,
        ),
        ("Page", "touchscreenTap", "touch", "pointer", false),
        (
            "Page",
            "unregisterLocatorHandler",
            "locatorHandler",
            "control",
            false,
        ),
        ("Page", "webStorageClear", "webStorage", "storage", false),
        ("Page", "webStorageGetItem", "webStorage", "storage", false),
        ("Page", "webStorageItems", "webStorage", "storage", false),
        (
            "Page",
            "webStorageRemoveItem",
            "webStorage",
            "storage",
            false,
        ),
        ("Page", "webStorageSetItem", "webStorage", "storage", false),
        (
            "Request",
            "rawRequestHeaders",
            "rawRequestHeaders",
            "read",
            false,
        ),
        ("Request", "response", "response", "read", false),
        ("Response", "body", "body", "read", false),
        ("Response", "httpVersion", "httpVersion", "read", false),
        (
            "Response",
            "rawResponseHeaders",
            "rawResponseHeaders",
            "read",
            false,
        ),
        (
            "Response",
            "securityDetails",
            "securityDetails",
            "read",
            false,
        ),
        ("Response", "serverAddr", "serverAddr", "read", false),
        ("Response", "sizes", "sizes", "read", false),
        ("Route", "abort", "abort", "network", false),
        ("Route", "continue", "continue", "network", false),
        ("Route", "fulfill", "fulfill", "network", false),
        ("Route", "neo:fetch", "fetch", "network", false),
    ];

    #[test]
    fn tests_that_every_served_command_has_its_contract_label() {
        let mut actual = Vec::new();
        let mut commands = BTreeSet::new();
        for method in served() {
            let (meta, action) = (method.meta(), method.action());
            assert!(
                commands.insert((meta.interface, meta.method)),
                "duplicate served command: {meta:?}"
            );
            assert_eq!(
                classify(meta).map(ServedMethod::action),
                Some(action),
                "classification: {meta:?}"
            );
            actual.push((
                meta.interface,
                meta.method,
                action.kind().as_str(),
                action.group().as_str(),
                action.targets_element(),
            ));
        }
        actual.sort_unstable();
        assert_eq!(actual.len(), 163, "all spec and Neo commands");
        assert_eq!(actual, EXPECTED, "every label matches the contract");

        let mut interface = "";
        let mut count = 0;
        for line in include_str!("../../neo-runtime-spec/spec/serve.yml").lines() {
            if let Some(method) = line.strip_prefix("    - ") {
                assert!(
                    commands.contains(&(interface, method)),
                    "missing {interface}.{method}"
                );
                count += 1;
            } else if line.starts_with("  ")
                && !line.starts_with("   ")
                && !line.trim_start().starts_with('#')
            {
                interface = line.trim().split(':').next().unwrap_or_default();
            }
        }
        assert_eq!(count, 116, "all spec commands checked");
    }

    /// A served `Frame` command, labelled as the router labels a decoded call.
    fn frame(method: &str, params: Value) -> ServedMethod {
        ServedMethod::from(&serve::FrameCall::decode(method, Some(params)).unwrap())
    }

    #[test]
    fn tests_that_a_generated_result_decodes_as_its_type_with_no_other_key() {
        let title = frame("title", json!({}));
        let actual = json!({ "value": "Old" });
        assert_eq!(
            title.replacement_result(json!({ "value": "x" }), Some(&actual)),
            Ok(Some(json!({ "value": "x" }))),
            "a string replaces the title"
        );
        for refused in [
            json!({ "value": 1 }),
            json!({ "value": "x", "extra": 1 }),
            json!({}),
            json!("x"),
        ] {
            assert!(
                title
                    .replacement_result(refused.clone(), Some(&actual))
                    .is_err(),
                "refused: {refused}"
            );
        }
    }

    #[test]
    fn tests_that_an_optional_result_field_may_be_absent_but_never_null() {
        let goto = frame("goto", json!({ "url": "https://a.test/" }));
        let actual = json!({ "response": { "guid": "response@1" } });
        for kept in [json!({}), json!({ "response": { "guid": "response@2" } })] {
            assert_eq!(
                goto.replacement_result(kept.clone(), Some(&actual)),
                Ok(Some(kept.clone())),
                "accepted: {kept}"
            );
        }
        for refused in [
            json!({ "response": null }),
            json!({ "response": "response@2" }),
            json!({ "response": { "guid": "response@2", "url": "https://a.test/" } }),
        ] {
            assert!(
                goto.replacement_result(refused.clone(), Some(&actual))
                    .is_err(),
                "refused: {refused}"
            );
        }
    }

    #[test]
    fn tests_that_a_command_with_no_result_has_nothing_to_replace() {
        let click = frame("click", json!({ "selector": "#go" }));
        assert!(
            click.replacement_result(json!({}), None).is_err(),
            "a void command"
        );
        assert!(
            click
                .replacement_result(json!({}), Some(&json!({})))
                .is_err(),
            "a void command, whatever result it is handed"
        );
        let title = frame("title", json!({}));
        assert!(
            title
                .replacement_result(json!({ "value": "x" }), None)
                .is_err(),
            "a command that produced no result"
        );
    }

    #[test]
    fn tests_that_result_numbers_compare_by_value() {
        let cookies = <serve::BrowserContextCookiesParams as SpecCommand>::METHOD;
        let cookie = json!({
            "name": "a",
            "value": "b",
            "domain": "a.test",
            "path": "/",
            "expires": -1,
            "httpOnly": false,
            "secure": false,
            "sameSite": "Lax",
        });
        let replaced = cookies
            .replacement_result(
                json!({ "cookies": [cookie] }),
                Some(&json!({ "cookies": [] })),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            replaced["cookies"][0]["expires"].as_f64(),
            Some(-1.0),
            "a whole number fills a float field"
        );
        let count = frame("queryCount", json!({ "selector": "li" }));
        let actual = json!({ "value": 3 });
        assert_eq!(
            count.replacement_result(json!({ "value": 2 }), Some(&actual)),
            Ok(Some(json!({ "value": 2 }))),
            "an integer field"
        );
        assert!(
            count
                .replacement_result(json!({ "value": 2.5 }), Some(&actual))
                .is_err(),
            "a fraction in an integer field"
        );
    }

    #[test]
    fn tests_that_a_neo_result_keeps_the_actual_top_level_shape() {
        let evaluate_expression = ServedMethod::from(&evaluate::FRAME_EVALUATE_META);
        let actual = json!({ "value": { "s": "old" } });
        let nested = json!({ "value": { "o": [{ "k": "a", "v": { "n": 1 } }] } });
        assert_eq!(
            evaluate_expression.replacement_result(nested.clone(), Some(&actual)),
            Ok(Some(nested)),
            "values below the top level are free"
        );
        for refused in [
            json!({ "handle": { "guid": "handle@1" } }),
            json!({ "value": { "s": "x" }, "extra": 1 }),
            json!({}),
            json!({ "value": "x" }),
            json!(["x"]),
        ] {
            assert!(
                evaluate_expression
                    .replacement_result(refused.clone(), Some(&actual))
                    .is_err(),
                "refused: {refused}"
            );
        }
        assert!(
            evaluate_expression
                .replacement_result(actual, None)
                .is_err(),
            "no result to compare with"
        );
    }

    #[test]
    fn tests_that_a_decode_error_keeps_its_commands_result_shape() {
        let error = serve::FrameCall::decode("goto", Some(json!({}))).unwrap_err();
        let goto = ServedMethod::decode_error(&error).unwrap();
        let actual = json!({});
        assert_eq!(
            goto.replacement_result(json!({}), Some(&actual)),
            Ok(Some(json!({}))),
            "goto's result"
        );
        assert!(
            goto.replacement_result(json!({ "response": null }), Some(&actual))
                .is_err(),
            "checked as goto's result"
        );
    }

    #[test]
    fn tests_that_every_served_command_checks_replacement_results() {
        let reported = json!({ "value": true });
        let mut shapes = BTreeMap::new();
        for method in served() {
            let shape = match method.result {
                ResultShape::Generated(_) => "generated",
                ResultShape::Void => "void",
                ResultShape::Neo => "neo",
            };
            *shapes.entry(shape).or_insert(0) += 1;
            let name = format!("{}.{}", method.interface, method.method);
            assert!(
                method.replacement_result(reported.clone(), None).is_err(),
                "{name}: no result to replace"
            );
            assert!(
                method
                    .replacement_result(json!({ "neo:unknown": true }), Some(&reported))
                    .is_err(),
                "{name}: a key its result does not have"
            );
            if shape == "neo" {
                assert_eq!(
                    method.replacement_result(reported.clone(), Some(&reported)),
                    Ok(Some(reported.clone())),
                    "{name}: its own result"
                );
            }
        }
        assert_eq!(
            shapes,
            BTreeMap::from([("generated", 45), ("neo", 47), ("void", 71)]),
            "every spec command decodes or has no result; every Neo command compares"
        );
    }
}
