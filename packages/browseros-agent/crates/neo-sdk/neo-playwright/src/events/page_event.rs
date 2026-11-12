//! The raw facts the target watcher records, as CDP reported them: target ids, never `PageId`s,
//! because which pages a run may see is its own view's decision, made when it reads.
//!
//! Console messages and page errors are turned into Playwright's text here, as upstream does on
//! its server (`renderPreview`, `exceptionToError`), so each reader gets the same string.

use browseros_core::{FrameId, SessionId, TabId, TargetId};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};
use ts_rs::TS;
use url::Url;

use crate::events::PageEventKind;
use crate::events::frame_lifecycle::{EventFrame, FrameLifecycle};
use crate::events::navigation_event::NavigationEvent;
use crate::world::BackendNodeId;

/// One recorded event.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PageEvent {
    /// Internal facts for navigation waiters; never delivered as public page events.
    Navigation(NavigationEvent),
    /// Dedicated worker lifetime, retained independently of page listeners.
    Worker {
        target: TargetId,
        worker: super::worker_table::WorkerInfo,
        closed: bool,
    },
    /// Public frame state at the transition, retained after the frame disappears.
    FrameLifecycle(FrameLifecycle),
    /// A main document fired a load event; OOPIF loads never become page loads.
    Load {
        target: TargetId,
        kind: PageEventKind,
        frame: EventFrame,
    },
    /// Actual frame removal, excluding renderer swaps.
    FrameDetached(FrameId),
    /// A stable frame moved to a renderer whose Page domain may still be disabled.
    FrameSwapped(FrameId),
    /// A new tab, recorded once it has a URL (1 to 30 ms after `Target.targetCreated`,
    /// measured) or after 1 s with whatever it has.
    Opened(OpenedPage),
    /// A tab that was reported opened, or that existed when the watcher started, closed.
    Closed(TargetId),
    /// The renderer of such a tab crashed. The tab stays open until it is closed.
    Crashed(TargetId),
    /// A JavaScript dialog opened.
    Dialog(DialogOpened),
    /// The same dialog closed, by a script, the browser, or automatic handling.
    DialogClosed(DialogOpened),
    /// A console message, recorded only while some run listens for it on that page.
    Console(ConsoleMessage),
    /// An uncaught exception, recorded only while some run listens for it on that page.
    PageError(PageError),
    /// A download started.
    Download(DownloadStarted),
    /// A native chooser intercepted by core before an agent-created tab is exposed.
    FileChooser(FileChooserOpened),
}

impl PageEvent {
    /// Whether this is an event of `kind` for the page whose target is `target`.
    pub(crate) fn concerns(&self, kind: PageEventKind, target: &TargetId) -> bool {
        match (kind, self) {
            (PageEventKind::WorkerState, Self::Worker { target: page, .. })
            | (
                PageEventKind::Worker,
                Self::Worker {
                    target: page,
                    closed: false,
                    ..
                },
            )
            | (
                PageEventKind::WorkerClosed,
                Self::Worker {
                    target: page,
                    closed: true,
                    ..
                },
            )
            | (PageEventKind::Close, Self::Closed(page))
            | (PageEventKind::Crash, Self::Crashed(page)) => page == target,
            (kind, Self::FrameLifecycle(frame)) => frame.target == *target && frame.kind == kind,
            (
                kind,
                Self::Load {
                    target: page,
                    kind: load,
                    ..
                },
            ) => page == target && *load == kind,
            (PageEventKind::Popup, Self::Opened(opened)) => opened.opener.as_ref() == Some(target),
            (PageEventKind::Dialog, Self::Dialog(dialog))
            | (PageEventKind::DialogClosed, Self::DialogClosed(dialog)) => dialog.target == *target,
            (PageEventKind::Console, Self::Console(message)) => message.target == *target,
            (PageEventKind::PageError, Self::PageError(error)) => error.target == *target,
            (PageEventKind::Download, Self::Download(download)) => {
                download.page.as_ref() == Some(target)
            }
            (PageEventKind::FileChooser, Self::FileChooser(chooser)) => chooser.target == *target,
            _ => false,
        }
    }
}

/// The chooser's originating session and node. Readers adopt it in its frame's current
/// document, and drop it if that frame disappeared, as Playwright's Chromium delegate does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileChooserOpened {
    pub(crate) target: TargetId,
    pub(crate) session: SessionId,
    pub(crate) frame: FrameId,
    pub(crate) node: BackendNodeId,
}

/// One call of upstream's CDP binding `__playwright__binding__` (`Runtime.bindingCalled`):
/// the stub of a binding a run exposed posted `{ name, seq, serializedArgs }` as JSON. The run
/// that exposed `name` answers it in `context`, the execution context that made the call. Not a
/// log record: the watcher hands it to each such run's own queue (`EventWriter::deliver_binding`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BindingCalled {
    pub(crate) target: TargetId,
    /// The execution context of the call, on the tab's page session: where the answer goes.
    pub(crate) context: i64,
    /// The frame of that context, when the watcher saw the context made; the main frame else.
    pub(crate) frame: Option<FrameId>,
    /// The page's JSON text, as the stub wrote it. The prelude parses it.
    pub(crate) payload: String,
}

/// A tab that a page, the owner or `context.newPage` opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenedPage {
    pub(crate) target: TargetId,
    /// `openerId`: set for every popup kind, `noopener` ones included (measured), as upstream
    /// reads it.
    pub(crate) opener: Option<TargetId>,
    pub(crate) url: String,
    pub(crate) window: WindowKind,
    /// BrowserOS's native tab id, from the target's `tabId` (present from `targetCreated` on,
    /// measured). The run's tab listing names live tabs only, so this is how a popup that
    /// closed before a read got to it (`window.open(..)` then `close()` in one task) still
    /// reaches the script as a page, already closed, as upstream emits it.
    pub(crate) tab: Option<TabId>,
}

/// Where a new page opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowKind {
    /// A tab in a normal window, which Neo drives.
    Tab,
    /// A separate popup window (`window.open(url, name, 'width=..')`), which Neo cannot drive
    /// (D9). Measured on BrowserOS 0.52.1.1, headless, with hosted mode on and off alike.
    Window,
}

/// A JavaScript dialog, as `Page.javascriptDialogOpening` reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DialogOpened {
    /// The dialog's handle for `dialog.accept` and `dialog.dismiss`.
    pub(crate) id: DialogId,
    pub(crate) target: TargetId,
    pub(crate) kind: DialogKind,
    pub(crate) message: String,
    pub(crate) default_value: String,
}

/// Neo's number for one dialog, unique per `Playwright`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub(crate) struct DialogId(pub(crate) u64);

/// Playwright's dialog types, as `dialog.type()` names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename = "DialogType")]
pub(crate) enum DialogKind {
    Alert,
    Confirm,
    Prompt,
    BeforeUnload,
}

impl DialogKind {
    /// The kind CDP names `name`; an unknown name is treated as a confirm.
    pub(crate) fn parse(name: &str) -> Self {
        match name {
            "alert" => Self::Alert,
            "prompt" => Self::Prompt,
            "beforeunload" => Self::BeforeUnload,
            _ => Self::Confirm,
        }
    }

    /// Playwright's `dialog.type()`.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::Confirm => "confirm",
            Self::Prompt => "prompt",
            Self::BeforeUnload => "beforeunload",
        }
    }

    /// How Neo closes a dialog nobody answers: upstream's `Dialog._close`, which accepts a
    /// `beforeunload` (so the page can leave) and dismisses every other kind.
    pub(crate) const fn accept_when_unanswered(self) -> bool {
        matches!(self, Self::BeforeUnload)
    }
}

/// A console message, in Playwright's terms.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ConsoleMessage {
    pub(crate) worker: Option<super::worker_table::WorkerInfo>,
    pub(crate) target: TargetId,
    /// `log`, `warning`, `error`, `info`, `debug`, ... as CDP and Playwright both name them.
    pub(crate) kind: String,
    /// The arguments' previews joined with spaces, as upstream's `text()`.
    pub(crate) text: String,
    /// The call's arguments, for `args()`; none for a browser log entry, as upstream's.
    pub(crate) args: Vec<ConsoleArg>,
    pub(crate) location: Location,
    /// When the page called the console, in milliseconds since the Unix epoch, as CDP reports
    /// it and upstream's `timestamp()` returns it.
    pub(crate) timestamp: f64,
}

/// One argument of a console call, as `message.args()` hands it out. Neo keeps no page object
/// past the event (no handles, `scope.json`), so an argument is the value the call's own report
/// carried, a snapshot read without calling into the page.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ConsoleArg {
    /// The value in Playwright's serialized format (`page/serialized_value.rs`) when the report
    /// holds all of it: a primitive, or an object or array whose V8 preview lists every member
    /// as a primitive. V8 previews one level deep, at most 5 properties of an object and 100
    /// items of an array, and abbreviates long strings; `None` for anything it cut, which
    /// `jsonValue()` refuses rather than guess. One known gap: a preview does not say which own
    /// properties are enumerable, so a property made non-enumerable with `defineProperty` is
    /// included where upstream's `Object.keys` leaves it out.
    pub(crate) value: Option<Value>,
    /// What the argument prints as (upstream's `renderPreview`): its part of `text()`.
    pub(crate) preview: String,
}

/// Where a message or error came from: the first stack frame, 0-based as CDP reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "ConsoleLocation")]
pub(crate) struct Location {
    pub(crate) url: String,
    pub(crate) line_number: u64,
    pub(crate) column_number: u64,
}

/// An uncaught exception, split as upstream's `exceptionToError` splits it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PageError {
    pub(crate) target: TargetId,
    pub(crate) name: String,
    pub(crate) message: String,
    pub(crate) stack: String,
}

/// A download that started in a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DownloadStarted {
    /// The page it is reported on: the frame's tab, or the opener of a tab that opened only to
    /// download (upstream's rule). `None` when the frame belongs to no tab the watcher knows.
    pub(crate) page: Option<TargetId>,
    pub(crate) frame: FrameId,
    pub(crate) guid: String,
    pub(crate) url: String,
    pub(crate) suggested_filename: String,
}

/// CDP's `Runtime.RemoteObject`, reduced to what a preview needs.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteObject {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default)]
    value: Option<Value>,
    #[serde(default)]
    unserializable_value: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    preview: Option<ObjectPreview>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObjectPreview {
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default)]
    description: Option<String>,
    /// Whether V8 left members out.
    #[serde(default)]
    overflow: bool,
    #[serde(default)]
    properties: Vec<PropertyPreview>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PropertyPreview {
    name: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    subtype: Option<String>,
    /// The member as text; V8 abbreviates a long string with a `…` in its middle.
    #[serde(default)]
    value: Option<String>,
    // No `valuePreview`: V8 nests one only in `console.table`'s preview, which lists every
    // column of every row (50 rows of 400 columns report as 20,000 properties, 1 MB, measured).
    // Read, it made each such call a value that size on its way to the script.
}

/// The most items of an array V8 lists in a preview. Upstream reads every index of an array, so a
/// longer one is never read from its preview, however sparse, and its length alone cannot make
/// Neo build a large value. No preview with more members than this is read either.
const ARRAY_PREVIEW_ITEMS: usize = 100;

/// `Log.entryAdded`, reduced to what upstream's `_onLogEntryAdded` reads.
#[derive(Debug, Deserialize)]
pub(crate) struct LogEntryParams {
    entry: LogEntry,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogEntry {
    /// `network`, `security`, `violation`, `worker`, `other`, ...
    #[serde(default)]
    source: String,
    /// `verbose`, `info`, `warning` or `error`, which upstream passes on as the message's type.
    #[serde(default)]
    level: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    line_number: Option<u64>,
    /// Milliseconds since the Unix epoch, as the console's own timestamps.
    #[serde(default)]
    timestamp: f64,
    #[serde(default)]
    args: Vec<HeldObject>,
}

/// A remote object a log entry holds, by id.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeldObject {
    #[serde(default)]
    object_id: Option<String>,
}

impl LogEntryParams {
    /// Whether the page logged it: upstream drops what a worker logged, which reaches it as the
    /// worker's own console.
    pub(crate) fn logged_by_page(&self) -> bool {
        self.entry.source != "worker"
    }

    /// Whether it is the failure of the browser's own request for the document's default
    /// favicon. Upstream's headless Chromium never asks for one (upstream's `favicon.spec.ts`:
    /// "headless browsers, except firefox, do not request favicons"), so no upstream listener
    /// sees it; BrowserOS asks even headless, as an ordinary image load whose `404` logs exactly
    /// like the page's own (measured). Only the URL tells them apart, so a page script's own
    /// failed load of `/favicon.ico` reads as the browser's too.
    pub(crate) fn browser_favicon(&self) -> bool {
        self.entry.source == "network"
            && self
                .entry
                .url
                .as_deref()
                .and_then(|url| Url::parse(url).ok())
                .is_some_and(|url| url.path() == "/favicon.ico")
    }

    /// When the browser logged it.
    pub(crate) fn timestamp(&self) -> f64 {
        self.entry.timestamp
    }

    /// The remote objects it holds, which upstream releases.
    pub(crate) fn argument_objects(&self) -> Vec<String> {
        self.entry
            .args
            .iter()
            .filter_map(|held| held.object_id.clone())
            .collect()
    }
}

/// `Runtime.consoleAPICalled`, reduced to what a message needs.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConsoleParams {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    args: Vec<RemoteObject>,
    #[serde(default)]
    stack_trace: Option<StackTrace>,
    #[serde(default)]
    timestamp: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StackTrace {
    #[serde(default)]
    call_frames: Vec<CallFrame>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CallFrame {
    #[serde(default)]
    function_name: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    line_number: u64,
    #[serde(default)]
    column_number: u64,
}

/// `Runtime.exceptionThrown`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExceptionParams {
    exception_details: ExceptionDetails,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExceptionDetails {
    #[serde(default)]
    text: String,
    #[serde(default)]
    exception: Option<RemoteObject>,
    #[serde(default)]
    stack_trace: Option<StackTrace>,
}

impl ConsoleMessage {
    /// The message `params` describes, on the page whose target is `target`.
    pub(crate) fn from_params(target: TargetId, params: ConsoleParams) -> Self {
        let args: Vec<ConsoleArg> = params
            .args
            .iter()
            .map(|arg| ConsoleArg {
                value: arg.serialized(),
                preview: arg.preview(),
            })
            .collect();
        let text: Vec<&str> = args.iter().map(|arg| arg.preview.as_str()).collect();
        let location = params
            .stack_trace
            .and_then(|stack| stack.call_frames.into_iter().next())
            .map(|frame| Location {
                url: frame.url,
                line_number: frame.line_number,
                column_number: frame.column_number,
            })
            .unwrap_or_default();
        Self {
            worker: None,
            target,
            kind: params.kind,
            text: text.join(" "),
            args,
            location,
            timestamp: params.timestamp,
        }
    }

    /// Upstream's `_onLogEntryAdded`: the entry's level as the type and its text, no arguments,
    /// and the entry's URL and line, column 0.
    pub(crate) fn from_log_entry(target: TargetId, params: LogEntryParams) -> Self {
        let entry = params.entry;
        Self {
            worker: None,
            target,
            kind: entry.level,
            text: entry.text,
            args: Vec::new(),
            location: Location {
                url: entry.url.unwrap_or_default(),
                line_number: entry.line_number.unwrap_or(0),
                column_number: 0,
            },
            timestamp: entry.timestamp,
        }
    }
}

impl PageError {
    /// Upstream's `exceptionToError`: the message with its stack, split at the first `    at`
    /// line, and `Name: message` split at the first colon.
    pub(crate) fn from_params(target: TargetId, params: ExceptionParams) -> Self {
        let details = params.exception_details;
        let with_stack = if let Some(exception) = &details.exception {
            exception.thrown_text()
        } else {
            let frames: Vec<String> = details
                .stack_trace
                .iter()
                .flat_map(|stack| &stack.call_frames)
                .map(|frame| {
                    let name = if frame.function_name.is_empty() {
                        "<anonymous>"
                    } else {
                        &frame.function_name
                    };
                    format!(
                        "\n    at {name} ({}:{}:{})",
                        frame.url, frame.line_number, frame.column_number
                    )
                })
                .collect();
            format!("{}{}", details.text, frames.concat())
        };
        let lines: Vec<&str> = with_stack.lines().collect();
        let first_frame = lines.iter().position(|line| line.starts_with("    at"));
        let (with_name, stack) = match first_frame {
            Some(at) => (
                lines
                    .iter()
                    .take(at)
                    .copied()
                    .collect::<Vec<_>>()
                    .join("\n"),
                with_stack.clone(),
            ),
            None => (with_stack.clone(), String::new()),
        };
        let (name, message) = match with_name.split_once(':') {
            Some((name, rest)) => (
                name.to_owned(),
                rest.strip_prefix(' ').unwrap_or(rest).to_owned(),
            ),
            None => (String::new(), with_name),
        };
        let name_override = details
            .exception
            .as_ref()
            .and_then(|exception| exception.preview.as_ref())
            .and_then(|preview| {
                preview
                    .properties
                    .iter()
                    .find(|property| property.name == "name")
            })
            .map(|property| property.value.clone().unwrap_or_else(|| "Error".to_owned()));
        Self {
            target,
            name: name_override.unwrap_or(name),
            message,
            stack,
        }
    }
}

impl RemoteObject {
    /// Upstream's `getExceptionMessage` for a thrown value: `description || String(value)`. CDP
    /// sends a thrown `null` as `"value": null`, which reads as no value here, and a thrown
    /// `undefined` with no value at all, so both are told apart by their type, as `String` would
    /// print them.
    fn thrown_text(&self) -> String {
        if let Some(description) = self.description.as_ref().filter(|text| !text.is_empty()) {
            return description.clone();
        }
        match &self.value {
            Some(value) => js_string(value),
            None if self.subtype.as_deref() == Some("null") => "null".to_owned(),
            None => "undefined".to_owned(),
        }
    }

    /// Upstream's `renderPreview`: what one console argument prints as.
    fn preview(&self) -> String {
        if self.kind == "undefined" {
            return "undefined".to_owned();
        }
        if let Some(value) = &self.value {
            return js_string(value);
        }
        if let Some(value) = &self.unserializable_value {
            return value.clone();
        }
        if let Some(preview) = &self.preview {
            if self.description.as_deref() == Some("Object") {
                let tokens: Vec<String> = preview
                    .properties
                    .iter()
                    .map(|property| {
                        format!(
                            "{}: {}",
                            property.name,
                            property.value.as_deref().unwrap_or("undefined")
                        )
                    })
                    .collect();
                return format!("{{{}}}", tokens.join(", "));
            }
            if self.subtype.as_deref() == Some("array") {
                return sparse_array(&preview.properties);
            }
        }
        self.description.clone().unwrap_or_default()
    }
}

impl RemoteObject {
    /// The argument as upstream's `serializeAsCallArgument` writes it for `jsonValue()`, read
    /// from the report alone ([`ConsoleArg::value`]). Each argument numbers its objects from 1,
    /// as each upstream handle's own serialization does.
    fn serialized(&self) -> Option<Value> {
        match self.kind.as_str() {
            // Upstream writes a function as nothing, which reads back as `undefined` too.
            "undefined" | "symbol" | "function" => Some(json!({ "v": "undefined" })),
            "string" | "boolean" => self.value.clone(),
            "number" => match (&self.value, &self.unserializable_value) {
                (Some(value), _) => Some(value.clone()),
                (None, Some(special)) => special_number(special),
                (None, None) => None,
            },
            "bigint" => bigint(self.unserializable_value.as_deref()?),
            "object" if self.subtype.as_deref() == Some("null") => Some(json!({ "v": "null" })),
            "object" => self.preview.as_ref()?.serialized(&mut 0),
            _ => None,
        }
    }
}

impl ObjectPreview {
    /// An array or a plain object whose preview lists every member, as upstream serializes it:
    /// `{a, id}` over every index (a hole reads as `undefined`), `{o: [{k, v}], id}` over the
    /// own keys. `ids` is the last object number used.
    fn serialized(&self, ids: &mut u64) -> Option<Value> {
        if self.overflow || self.properties.len() > ARRAY_PREVIEW_ITEMS {
            return None;
        }
        match self.subtype.as_deref() {
            Some("array") => {
                let length: usize = self
                    .description
                    .as_deref()?
                    .strip_prefix("Array(")?
                    .strip_suffix(')')?
                    .parse()
                    .ok()?;
                if length > ARRAY_PREVIEW_ITEMS {
                    return None;
                }
                *ids += 1;
                let id = *ids;
                let mut items = vec![json!({ "v": "undefined" }); length];
                for property in &self.properties {
                    // Upstream reads indices only; other keys of an array are not its value.
                    if let Ok(index) = property.name.parse::<usize>() {
                        *items.get_mut(index)? = property.serialized()?;
                    }
                }
                Some(json!({ "a": items, "id": id }))
            }
            // A class instance with no own key may serialize as its `toJSON()` upstream, which
            // a preview cannot tell; a plain object cannot.
            None if !self.properties.is_empty()
                || self.description.as_deref() == Some("Object") =>
            {
                *ids += 1;
                let id = *ids;
                let members = self
                    .properties
                    .iter()
                    .map(|property| {
                        // Upstream writes an own `toJSON` function as an empty object.
                        let value = if property.name == "toJSON" && property.kind == "function" {
                            json!({ "o": [], "id": 0 })
                        } else {
                            property.serialized()?
                        };
                        Some(json!({ "k": property.name, "v": value }))
                    })
                    .collect::<Option<Vec<Value>>>()?;
                Some(json!({ "o": members, "id": id }))
            }
            _ => None,
        }
    }
}

impl PropertyPreview {
    fn serialized(&self) -> Option<Value> {
        let text = self.value.as_deref();
        match self.kind.as_str() {
            "undefined" | "symbol" | "function" => Some(json!({ "v": "undefined" })),
            // V8 cuts a long string in the middle; a string that holds `…` may be cut.
            "string" => text.filter(|text| !text.contains('…')).map(Value::from),
            "boolean" => match text? {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                _ => None,
            },
            "number" => {
                let text = text?;
                special_number(text).or_else(|| {
                    let number: f64 = text
                        .parse()
                        .ok()
                        .filter(|number: &f64| number.is_finite())?;
                    text.parse::<i64>()
                        .map(Value::from)
                        .ok()
                        .or_else(|| Number::from_f64(number).map(Value::Number))
                })
            }
            "bigint" => bigint(text?),
            "object" if self.subtype.as_deref() == Some("null") => Some(json!({ "v": "null" })),
            // A nested object or array: a preview lists only its description. A getter
            // (`accessor`) runs page code to read.
            _ => None,
        }
    }
}

/// The numbers JSON cannot carry, as CDP names them and Playwright's format writes them.
fn special_number(text: &str) -> Option<Value> {
    matches!(text, "NaN" | "Infinity" | "-Infinity" | "-0").then(|| json!({ "v": text }))
}

/// A `BigInt` as CDP prints it (`123n`), in Playwright's format.
fn bigint(text: &str) -> Option<Value> {
    let digits = text.strip_suffix('n')?;
    Some(json!({ "bi": digits }))
}

/// JavaScript's `String(value)` for a JSON value.
fn js_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".to_owned(),
        Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

/// Upstream's `sparseArrayToString` over an array preview's index properties.
fn sparse_array(properties: &[PropertyPreview]) -> String {
    let mut entries: Vec<(u64, &str)> = properties
        .iter()
        .filter_map(|property| {
            let index = property.name.parse::<u64>().ok()?;
            Some((index, property.value.as_deref().unwrap_or("undefined")))
        })
        .collect();
    entries.sort_by_key(|(index, _)| *index);
    let mut tokens = Vec::new();
    let mut next = 0;
    for (index, value) in entries {
        match index.saturating_sub(next) {
            0 => {}
            1 => tokens.push("<empty>".to_owned()),
            empty => tokens.push(format!("<{empty} empty items>")),
        }
        tokens.push(value.to_owned());
        next = index + 1;
    }
    format!("[{}]", tokens.join(", "))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_core::TargetId;
    use serde_json::json;

    use super::{
        ConsoleMessage, ConsoleParams, DialogKind, ExceptionParams, LogEntryParams, PageError,
    };

    fn target() -> TargetId {
        TargetId("T".to_owned())
    }

    #[test]
    fn tests_that_console_text_joins_argument_previews_as_upstream() -> Result<(), Box<dyn Error>> {
        let params: ConsoleParams = serde_json::from_value(json!({
            "type": "log",
            "args": [
                { "type": "string", "value": "clicked" },
                { "type": "number", "value": 7 },
                { "type": "object", "description": "Object",
                  "preview": { "properties": [{ "name": "a", "value": "1" }] } },
                { "type": "object", "subtype": "array", "description": "Array(3)",
                  "preview": { "properties": [{ "name": "0", "value": "1" },
                                              { "name": "2", "value": "3" }] } },
                { "type": "undefined" },
                { "type": "number", "unserializableValue": "NaN" },
            ],
            "stackTrace": { "callFrames": [{ "functionName": "", "url": "https://a.test/",
                                             "lineNumber": 4, "columnNumber": 9 }] },
            "timestamp": 1_700_000_000_123.5,
        }))?;
        let message = ConsoleMessage::from_params(target(), params);
        assert_eq!(
            message.text,
            "clicked 7 {a: 1} [1, <empty>, 3] undefined NaN"
        );
        assert_eq!(message.kind, "log");
        assert_eq!(message.location.url, "https://a.test/");
        assert_eq!(
            (message.location.line_number, message.location.column_number),
            (4, 9)
        );
        assert!(
            (message.timestamp - 1_700_000_000_123.5).abs() < f64::EPSILON,
            "CDP's call time, unchanged"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_page_error_splits_like_upstreams_exception_to_error()
    -> Result<(), Box<dyn Error>> {
        let thrown: ExceptionParams = serde_json::from_value(json!({ "exceptionDetails": {
            "text": "Uncaught",
            "exception": { "type": "object", "subtype": "error",
                           "description": "TypeError: x is undefined\n    at f (https://a.test/:1:5)" },
        } }))?;
        let error = PageError::from_params(target(), thrown);
        assert_eq!(
            (error.name.as_str(), error.message.as_str()),
            ("TypeError", "x is undefined")
        );
        assert_eq!(
            error.stack,
            "TypeError: x is undefined\n    at f (https://a.test/:1:5)"
        );

        let rejected: ExceptionParams = serde_json::from_value(json!({ "exceptionDetails": {
            "text": "Uncaught (in promise)",
            "exception": { "type": "string", "value": "plain" },
        } }))?;
        let error = PageError::from_params(target(), rejected);
        assert_eq!(
            (
                error.name.as_str(),
                error.message.as_str(),
                error.stack.as_str()
            ),
            ("", "plain", "")
        );
        Ok(())
    }

    /// ER-006: a thrown value that is not an error reads as `String(value)`, as upstream's
    /// `getExceptionMessage` gives it, `null` and `undefined` included.
    #[test]
    fn tests_that_odd_thrown_values_read_as_upstreams_string_of_them()
    -> Result<(), serde_json::Error> {
        for (exception, message) in [
            (
                json!({ "type": "object", "subtype": "null", "value": null }),
                "null",
            ),
            (json!({ "type": "undefined" }), "undefined"),
            (
                json!({ "type": "number", "value": 0, "description": "0" }),
                "0",
            ),
            (json!({ "type": "string", "value": "" }), ""),
        ] {
            let thrown: ExceptionParams = serde_json::from_value(json!({ "exceptionDetails": {
                "text": "Uncaught", "exception": exception,
            } }))?;
            let error = PageError::from_params(target(), thrown);
            assert_eq!((error.name.as_str(), error.message.as_str()), ("", message));
        }
        Ok(())
    }

    /// A dialog's `type` on the wire is the name `dialog.type()` answers, for every kind.
    #[test]
    fn tests_that_a_dialog_kind_serializes_as_its_playwright_name() -> Result<(), Box<dyn Error>> {
        for kind in [
            DialogKind::Alert,
            DialogKind::Confirm,
            DialogKind::Prompt,
            DialogKind::BeforeUnload,
        ] {
            assert_eq!(serde_json::to_value(kind)?, json!(kind.as_str()));
        }
        Ok(())
    }

    /// Upstream's `_onLogEntryAdded`: the level as the type, the text, the entry's URL and line
    /// with column 0, and nothing from a worker.
    #[test]
    fn tests_that_a_log_entry_reads_as_upstreams_console_message() -> Result<(), serde_json::Error>
    {
        let blocked: LogEntryParams = serde_json::from_value(json!({ "entry": {
            "source": "network", "level": "error",
            "text": "Access to fetch at 'http://b.test/' from origin 'http://a.test' has been blocked by CORS policy",
            "url": "http://b.test/", "lineNumber": 4, "timestamp": 1_700_000_000_123.5,
            "args": [{ "type": "object", "objectId": "held" }],
        } }))?;
        assert!(blocked.logged_by_page());
        assert_eq!(blocked.argument_objects(), ["held"]);
        let message = ConsoleMessage::from_log_entry(target(), blocked);
        assert_eq!(
            (message.kind.as_str(), message.location.url.as_str()),
            ("error", "http://b.test/")
        );
        assert!(message.text.contains("CORS policy"));
        assert_eq!(
            (message.location.line_number, message.location.column_number),
            (4, 0)
        );
        let worker: LogEntryParams = serde_json::from_value(json!({ "entry": {
            "source": "worker", "level": "info", "text": "w", "timestamp": 1.0,
        } }))?;
        assert!(!worker.logged_by_page());
        let favicon: LogEntryParams = serde_json::from_value(json!({ "entry": {
            "source": "network", "level": "error",
            "text": "Failed to load resource: the server responded with a status of 404 (Not Found)",
            "url": "http://127.0.0.1:8907/favicon.ico", "timestamp": 1.0,
        } }))?;
        assert!(
            favicon.browser_favicon(),
            "the browser's own favicon request"
        );
        let missing: LogEntryParams = serde_json::from_value(json!({ "entry": {
            "source": "network", "level": "error",
            "text": "Failed to load resource: the server responded with a status of 404 (Not Found)",
            "url": "http://127.0.0.1:8907/missing.png", "timestamp": 1.0,
        } }))?;
        assert!(!missing.browser_favicon(), "a page's own failed load");
        Ok(())
    }

    /// `args()` values in Playwright's format, read from the console call's report alone:
    /// primitives and flat objects and arrays whole, with each argument numbering its objects
    /// from 1; anything V8's preview cut (a nested object, overflow, an abbreviated string, a
    /// getter) has no value, nor has `console.table`'s preview, which nests every row.
    #[test]
    fn tests_that_console_arguments_keep_only_values_their_preview_holds_whole()
    -> Result<(), serde_json::Error> {
        let params: ConsoleParams = serde_json::from_value(json!({
            "type": "log", "timestamp": 1.0,
            "args": [
                { "type": "string", "value": "hello" },
                { "type": "number", "value": 5 },
                { "type": "number", "unserializableValue": "-0", "description": "-0" },
                { "type": "bigint", "unserializableValue": "12n", "description": "12n" },
                { "type": "undefined" },
                { "type": "object", "subtype": "null", "value": null },
                { "type": "object", "className": "Object", "description": "Object",
                  "preview": { "type": "object", "description": "Object", "overflow": false,
                    "properties": [
                      { "name": "foo", "type": "string", "value": "bar" },
                      { "name": "n", "type": "number", "value": "NaN" },
                    ] } },
                { "type": "object", "subtype": "array", "description": "Array(2)",
                  "preview": { "type": "object", "subtype": "array", "description": "Array(2)",
                    "overflow": false,
                    "properties": [{ "name": "1", "type": "boolean", "value": "true" }] } },
                // `console.table`'s preview nests every row's columns: no value is read from it.
                { "type": "object", "subtype": "array", "description": "Array(1)",
                  "preview": { "type": "object", "subtype": "array", "description": "Array(1)",
                    "overflow": false,
                    "properties": [{ "name": "0", "type": "object", "value": "Object",
                      "valuePreview": { "type": "object", "description": "Object",
                        "overflow": false,
                        "properties": [{ "name": "k0", "type": "string", "value": "v" }] } }] } },
                { "type": "object", "description": "Object",
                  "preview": { "type": "object", "description": "Object", "overflow": false,
                    "properties": [{ "name": "deep", "type": "object", "value": "Object" }] } },
                { "type": "object", "description": "Object",
                  "preview": { "type": "object", "description": "Object", "overflow": true,
                    "properties": [] } },
                { "type": "object", "description": "Object",
                  "preview": { "type": "object", "description": "Object", "overflow": false,
                    "properties": [{ "name": "long", "type": "string", "value": "aaaa…aaaa" }] } },
                { "type": "object", "description": "Object",
                  "preview": { "type": "object", "description": "Object", "overflow": false,
                    "properties": [{ "name": "get", "type": "accessor" }] } },
                { "type": "object", "description": "Object",
                  "preview": { "type": "object", "description": "Object", "overflow": false,
                    "properties": [{ "name": "toJSON", "type": "function", "value": "" },
                                   { "name": "f", "type": "function", "value": "" }] } },
            ],
        }))?;
        let message = ConsoleMessage::from_params(target(), params);
        let values: Vec<Option<serde_json::Value>> =
            message.args.into_iter().map(|arg| arg.value).collect();
        assert_eq!(
            values,
            [
                Some(json!("hello")),
                Some(json!(5)),
                Some(json!({ "v": "-0" })),
                Some(json!({ "bi": "12" })),
                Some(json!({ "v": "undefined" })),
                Some(json!({ "v": "null" })),
                Some(json!({ "o": [
                    { "k": "foo", "v": "bar" },
                    { "k": "n", "v": { "v": "NaN" } },
                ], "id": 1 })),
                Some(json!({ "a": [{ "v": "undefined" }, true], "id": 1 })),
                None,
                None,
                None,
                None,
                None,
                Some(json!({ "o": [
                    { "k": "toJSON", "v": { "o": [], "id": 0 } },
                    { "k": "f", "v": { "v": "undefined" } },
                ], "id": 1 })),
            ]
        );
        Ok(())
    }
}
