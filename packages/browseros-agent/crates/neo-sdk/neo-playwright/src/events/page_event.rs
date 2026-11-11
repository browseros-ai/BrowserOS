//! The raw facts the target watcher records, as CDP reported them: target ids, never `PageId`s,
//! because which pages a run may see is its own view's decision, made when it reads.
//!
//! Console messages and page errors are turned into Playwright's text here, as upstream does on
//! its server (`renderPreview`, `exceptionToError`), so each reader gets the same string.

use browseros_core::{FrameId, SessionId, TargetId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::events::PageEventKind;
use crate::events::navigation_event::NavigationEvent;
use crate::world::BackendNodeId;

/// One recorded event.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PageEvent {
    /// Internal facts for navigation waiters; never delivered as public page events.
    Navigation(NavigationEvent),
    /// Actual frame removal, excluding renderer swaps.
    FrameDetached(FrameId),
    /// A stable frame moved to a renderer whose Page domain may still be disabled.
    FrameSwapped(FrameId),
    /// A new tab, recorded once it has a URL (1 to 30 ms after `Target.targetCreated`,
    /// measured) or after 1 s with whatever it has.
    Opened(OpenedPage),
    /// A tab that was reported opened, or that existed when the watcher started, closed.
    Closed(TargetId),
    /// A JavaScript dialog opened.
    Dialog(DialogOpened),
    /// A console message, recorded only while some run listens for it on that page.
    Console(ConsoleMessage),
    /// An uncaught exception, recorded only while some run listens for it on that page.
    PageError(PageError),
    /// A download started.
    Download(DownloadStarted),
    /// A native chooser intercepted by core before an agent-created tab is exposed.
    FileChooser(FileChooserOpened),
    /// A page called a binding a run exposed, recorded only while some run exposes one on that
    /// page.
    Binding(BindingCalled),
}

impl PageEvent {
    /// Whether this is an event of `kind` for the page whose target is `target`.
    pub(crate) fn concerns(&self, kind: PageEventKind, target: &TargetId) -> bool {
        match (kind, self) {
            (PageEventKind::Popup, Self::Opened(opened)) => opened.opener.as_ref() == Some(target),
            (PageEventKind::Close, Self::Closed(closed)) => closed == target,
            (PageEventKind::Dialog, Self::Dialog(dialog)) => dialog.target == *target,
            (PageEventKind::Console, Self::Console(message)) => message.target == *target,
            (PageEventKind::PageError, Self::PageError(error)) => error.target == *target,
            (PageEventKind::Download, Self::Download(download)) => {
                download.page.as_ref() == Some(target)
            }
            (PageEventKind::FileChooser, Self::FileChooser(chooser)) => chooser.target == *target,
            (PageEventKind::Binding, Self::Binding(call)) => call.target == *target,
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
/// that exposed `name` answers it in `context`, the execution context that made the call.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct DialogId(pub(crate) u64);

/// Playwright's dialog types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub(crate) target: TargetId,
    /// `log`, `warning`, `error`, `info`, `debug`, ... as CDP and Playwright both name them.
    pub(crate) kind: String,
    /// The arguments' previews joined with spaces, as upstream's `text()`.
    pub(crate) text: String,
    pub(crate) location: Location,
    /// When the page called the console, in milliseconds since the Unix epoch, as CDP reports
    /// it and upstream's `timestamp()` returns it.
    pub(crate) timestamp: f64,
}

/// Where a message or error came from: the first stack frame, 0-based as CDP reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
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
struct ObjectPreview {
    #[serde(default)]
    properties: Vec<PropertyPreview>,
}

#[derive(Debug, Deserialize)]
struct PropertyPreview {
    name: String,
    #[serde(default)]
    value: Option<String>,
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
        let text: Vec<String> = params.args.iter().map(RemoteObject::preview).collect();
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
            target,
            kind: params.kind,
            text: text.join(" "),
            location,
            timestamp: params.timestamp,
        }
    }
}

impl PageError {
    /// Upstream's `exceptionToError`: the message with its stack, split at the first `    at`
    /// line, and `Name: message` split at the first colon.
    pub(crate) fn from_params(target: TargetId, params: ExceptionParams) -> Self {
        let details = params.exception_details;
        let with_stack = if let Some(exception) = &details.exception {
            exception
                .description
                .clone()
                .unwrap_or_else(|| exception.value.as_ref().map(js_string).unwrap_or_default())
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

    use super::{ConsoleMessage, ConsoleParams, ExceptionParams, PageError};

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
}
