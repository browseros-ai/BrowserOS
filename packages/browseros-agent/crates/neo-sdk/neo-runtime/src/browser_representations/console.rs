//! Playwright: `server/chromium/crExecutionContext.ts` `createHandle`
//! Playwright: `server/javascript.ts` `JSHandle`
//! Playwright: `server/console.ts` `ConsoleMessage`
//! Playwright: `server/chromium/crPage.ts` `FrameSession`
//! Playwright: `server/chromium/crProtocolHelper.ts` `exceptionToError`
//! Playwright: `utils/stackTrace.ts` `splitErrorMessage`
//! Playwright: `server/errors.ts` `serializeError`
//! Playwright: `server/page.ts` `Page`
//!
//! The server half of Playwright's console messages and page errors for Neo: this tab's CDP
//! `Runtime.consoleAPICalled`, `Log.entryAdded` and `Runtime.exceptionThrown` payloads become the
//! protocol's console message (`{type, text, args, location, timestamp}`) and serialized error,
//! and [`History`] keeps one page session's messages and errors for `Page.consoleMessages` and
//! `Page.pageErrors`.
//!
//! ```text
//!  page_events.rs ── CDP payload ──▶ from_console_api | from_log_entry | from_exception
//!        │                                  (pure: one payload in, one record out)
//!        └── push_* | mark_navigation ──▶ History ──▶ messages | errors | clear_*
//! ```
//!
//! Nothing here sends CDP or locks: the caller releases a Log entry's argument objects
//! ([`log_entry_object_ids`]) and keeps the [`History`] behind its own mutex.

use std::cmp::Ordering;

use neo_playwright_protocol::serve::{
    PageConsoleMessagesResultMessagesItem, PageConsoleMessagesResultMessagesItemLocation,
    SerializedError, SerializedErrorError,
};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::progress;

/// Playwright: `server/page.ts` `Page.addConsoleMessage`: `ensureArrayLimit(this._consoleMessages, 200); // Avoid unbounded memory growth.`
const CONSOLE_MESSAGES_LIMIT: usize = 200;

/// Playwright: `server/page.ts` `Page.addPageError`: `ensureArrayLimit(this._pageErrors, 200); // Avoid unbounded memory growth.`
const PAGE_ERRORS_LIMIT: usize = 200;

/// Deviation: `server/page.ts` `Page.addConsoleMessage`: `ensureArrayLimit(this._consoleMessages, 200); // Avoid unbounded memory growth.`
/// Playwright bounds the count only. Neo keeps the history in the server for the run, outside the
/// realm's memory limit, so each list also keeps at most this many bytes of text (a message's,
/// an error's message and stack), dropping its oldest entries first; the newest always stays.
const HISTORY_BYTES_LIMIT: usize = 16 * 1024 * 1024;

/// A console message as the protocol carries it, in `Page.consoleMessages` and the context's
/// `console` event: `{type, text, args, location, timestamp}`.
pub(crate) type ConsoleRecord = PageConsoleMessagesResultMessagesItem;

/// Where a console message or page error was raised: CDP's 0-based line and column.
pub(crate) type ConsoleLocation = PageConsoleMessagesResultMessagesItemLocation;

/// CDP's `Runtime.RemoteObject`, as far as a console argument's text or an exception's message
/// reads it.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteObject {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) subtype: Option<String>,
    /// `Some(Value::Null)` when CDP sent `"value": null`, `None` when it sent no `value`.
    #[serde(default, deserialize_with = "present")]
    pub(crate) value: Option<Value>,
    #[serde(default)]
    pub(crate) unserializable_value: Option<String>,
    #[serde(default)]
    pub(crate) description: Option<String>,
    #[serde(default)]
    pub(crate) object_id: Option<String>,
    #[serde(default)]
    pub(crate) preview: Option<ObjectPreview>,
}

/// CDP's `Runtime.ObjectPreview`.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ObjectPreview {
    pub(crate) properties: Vec<PropertyPreview>,
}

/// CDP's `Runtime.PropertyPreview`; an accessor has no `value`.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct PropertyPreview {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) value: Option<String>,
}

/// CDP's `Runtime.StackTrace`, innermost frame first.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StackTrace {
    pub(crate) call_frames: Vec<CallFrame>,
}

/// CDP's `Runtime.CallFrame`; its line and column are 0-based.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CallFrame {
    pub(crate) function_name: String,
    pub(crate) url: String,
    pub(crate) line_number: i64,
    pub(crate) column_number: i64,
}

/// CDP's `Runtime.consoleAPICalled` params.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConsoleApiCalled {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) args: Vec<RemoteObject>,
    pub(crate) execution_context_id: i64,
    /// Epoch milliseconds (`Runtime.Timestamp`).
    pub(crate) timestamp: f64,
    #[serde(default)]
    pub(crate) stack_trace: Option<StackTrace>,
}

/// CDP's `Runtime.exceptionThrown` params.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExceptionThrown {
    /// Epoch milliseconds (`Runtime.Timestamp`).
    pub(crate) timestamp: f64,
    pub(crate) exception_details: ExceptionDetails,
}

/// CDP's `Runtime.ExceptionDetails`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExceptionDetails {
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) exception: Option<RemoteObject>,
    #[serde(default)]
    pub(crate) stack_trace: Option<StackTrace>,
}

/// CDP's `Log.entryAdded` params.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct LogEntryAdded {
    pub(crate) entry: LogEntry,
}

/// CDP's `Log.LogEntry`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LogEntry {
    pub(crate) source: String,
    pub(crate) level: String,
    pub(crate) text: String,
    /// Epoch milliseconds (`Runtime.Timestamp`).
    pub(crate) timestamp: f64,
    #[serde(default)]
    pub(crate) url: Option<String>,
    #[serde(default)]
    pub(crate) line_number: Option<i64>,
    #[serde(default)]
    pub(crate) args: Option<Vec<RemoteObject>>,
}

/// One page error as Playwright's `Page` keeps it. The `pageError` event carries the location as
/// `{url, line, column}`.
///
/// Playwright: `server/page.ts` `PageError`
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PageErrorRecord {
    pub(crate) error: SerializedError,
    pub(crate) location: ConsoleLocation,
}

/// The text of one console argument: the preview its handle has when the message is built.
///
/// Playwright: `server/chromium/crExecutionContext.ts` `createHandle`: `if (remoteObject.subtype === 'node') {`
/// Playwright: `server/javascript.ts` `JSHandle.constructor`: `this._preview = this._objectId ? preview ||`
pub(crate) fn arg_preview(object: &RemoteObject) -> String {
    let has_object = object.object_id.as_deref().is_some_and(|id| !id.is_empty());
    if object.subtype.as_deref() == Some("node") {
        // Deviation: `server/dom.ts` `ElementHandle._initializePreview`: `this._setPreview(await utility.evaluate((injected, e) => 'JSHandle@' + injected.previewNode(e), this));`
        // Playwright later swaps a node's preview for its markup (`JSHandle@<div id="a">hi</div>`),
        // which a message's text shows when first read after the swap: a history read with no
        // console listener (a listener, or a `debug` message, reads the text at once). Neo never
        // swaps, so the text is always `JSHandle@node`, what a listener sees.
        return if has_object {
            "JSHandle@node"
        } else {
            "undefined"
        }
        .to_string();
    }
    if !has_object {
        return value_text(object);
    }
    let kind = object
        .subtype
        .as_deref()
        .filter(|subtype| !subtype.is_empty())
        .unwrap_or(&object.kind);
    render_preview(object)
        .filter(|preview| !preview.is_empty())
        .unwrap_or_else(|| format!("JSHandle@{kind}"))
}

/// A message's text: its arguments' previews.
///
/// Playwright: `server/console.ts` `ConsoleMessage.text`: `this._text = this._args.map(arg => arg.preview()).join(' ');`
pub(crate) fn console_text(args: &[RemoteObject]) -> String {
    args.iter().map(arg_preview).collect::<Vec<_>>().join(" ")
}

/// Playwright: `server/chromium/crProtocolHelper.ts` `stackTraceToLocation`
pub(crate) fn location(stack: Option<&StackTrace>) -> ConsoleLocation {
    match stack.and_then(|stack| stack.call_frames.first()) {
        Some(frame) => ConsoleLocation {
            url: frame.url.clone(),
            line_number: frame.line_number,
            column_number: frame.column_number,
        },
        None => ConsoleLocation {
            url: String::new(),
            line_number: 0,
            column_number: 0,
        },
    }
}

/// A `console.*` call as a message, or `None` for one CDP replays from a removed context.
///
/// Playwright: `server/chromium/crPage.ts` `FrameSession._onConsoleAPI`: `if (event.executionContextId === 0) {`
pub(crate) fn from_console_api(event: &ConsoleApiCalled) -> Option<ConsoleRecord> {
    if event.execution_context_id == 0 {
        return None;
    }
    // Deviation: `server/chromium/crPage.ts` `FrameSession._onConsoleAPI`: `if (!context)`
    // Playwright also drops a message from a context it holds no record of (none created in a
    // frame it knows, or one already destroyed); Neo keeps no table of contexts and keeps it.
    Some(ConsoleRecord {
        r#type: event.kind.clone(),
        text: console_text(&event.args),
        // Deviation: `server/chromium/crPage.ts` `FrameSession._onConsoleAPI`: `const values = event.args.map(arg => createHandle(context, arg));`
        // Neo serves no JSHandle objects, so a message carries no arguments; its text still
        // renders each one.
        args: Vec::new(),
        location: location(event.stack_trace.as_ref()),
        timestamp: event.timestamp,
    })
}

/// A browser-reported entry (network, violation, intervention) as a message, or `None` for a
/// worker's.
///
/// Playwright: `server/chromium/crPage.ts` `FrameSession._onLogEntryAdded`: `this._page.addConsoleMessage(null, level, [], location, text, event.entry.timestamp);`
pub(crate) fn from_log_entry(entry: &LogEntry) -> Option<ConsoleRecord> {
    if entry.source == "worker" {
        return None;
    }
    Some(ConsoleRecord {
        r#type: entry.level.clone(),
        text: entry.text.clone(),
        args: Vec::new(),
        location: ConsoleLocation {
            url: entry.url.clone().unwrap_or_default(),
            line_number: entry.line_number.unwrap_or(0),
            column_number: 0,
        },
        timestamp: entry.timestamp,
    })
}

/// The objects a Log entry's arguments hold in the page, which the caller releases with
/// `Runtime.releaseObject` as the entry arrives, a worker's entry too. An argument without one
/// is skipped: Playwright's release of it fails, and it ignores the failure.
///
/// Playwright: `server/chromium/crPage.ts` `FrameSession._onLogEntryAdded`: `args.map(arg => releaseObject(this._client, arg.objectId!));`
pub(crate) fn log_entry_object_ids(entry: &LogEntry) -> Vec<String> {
    entry
        .args
        .iter()
        .flatten()
        .filter_map(|arg| arg.object_id.clone())
        .collect()
}

/// Playwright: `server/chromium/crPage.ts` `FrameSession._handleException`: `this._page.addPageError(exceptionToError(exceptionDetails), stackTraceToLocation(exceptionDetails.stackTrace));`
pub(crate) fn from_exception(event: &ExceptionThrown) -> PageErrorRecord {
    let details = &event.exception_details;
    PageErrorRecord {
        error: exception_to_error(details),
        location: location(details.stack_trace.as_ref()),
    }
}

/// An exception as the `Error` Playwright builds from it, serialized. That `Error` has no `code`,
/// so `serializeError` sends its message unchanged and no code.
///
/// Playwright: `server/chromium/crProtocolHelper.ts` `exceptionToError`: `const firstStackTraceLine = lines.findIndex(line => line.startsWith('    at'));`
/// Playwright: `server/errors.ts` `serializeError`: `return { error: { message: systemErrorMessage(e), stack: e.stack, name: e.name, code: systemErrorCode(e) } };`
pub(crate) fn exception_to_error(details: &ExceptionDetails) -> SerializedError {
    let message_with_stack = exception_message(details);
    let first_stack_line = message_with_stack
        .split('\n')
        .position(|line| line.starts_with("    at"));
    let (message_with_name, stack) = match first_stack_line {
        None => (message_with_stack, String::new()),
        Some(index) => (
            message_with_stack
                .split('\n')
                .take(index)
                .collect::<Vec<_>>()
                .join("\n"),
            message_with_stack,
        ),
    };
    let (name, message) = split_error_message(&message_with_name);
    let name_override = details
        .exception
        .as_ref()
        .and_then(|exception| exception.preview.as_ref())
        .and_then(|preview| {
            preview
                .properties
                .iter()
                .find(|property| property.name == "name")
        });
    let name = match name_override {
        Some(property) => property
            .value
            .clone()
            .unwrap_or_else(|| "Error".to_string()),
        None => name,
    };
    SerializedError {
        error: Some(SerializedErrorError {
            message,
            name,
            stack: Some(stack),
            code: None,
        }),
        value: None,
    }
}

/// One page session's console messages and page errors, kept as Playwright's `Page` keeps them
/// for `Page.consoleMessages` and `Page.pageErrors`. Plain data: the caller locks it.
#[derive(Debug, Default)]
pub(crate) struct History {
    messages: Vec<Entry<ConsoleRecord>>,
    errors: Vec<Entry<PageErrorRecord>>,
}

/// A kept message or error and Playwright's navigation mark on it.
#[derive(Debug)]
struct Entry<T> {
    value: T,
    /// The main frame committed a new document after this entry. The mark stays with its entry,
    /// so dropping the oldest entries never moves it onto another.
    marked: bool,
}

/// Playwright: `server/page.ts` `Page`
impl History {
    /// Playwright: `server/page.ts` `Page.addConsoleMessage`: `this._consoleMessages.push(message);`
    pub(crate) fn push_message(&mut self, message: ConsoleRecord) {
        self.messages.push(Entry {
            value: message,
            marked: false,
        });
        ensure_array_limit(&mut self.messages, CONSOLE_MESSAGES_LIMIT);
        ensure_bytes_limit(&mut self.messages, |message| message.text.len());
    }

    /// Playwright: `server/page.ts` `Page.addPageError`: `this._pageErrors.push(pageError);`
    pub(crate) fn push_error(&mut self, error: PageErrorRecord) {
        self.errors.push(Entry {
            value: error,
            marked: false,
        });
        ensure_array_limit(&mut self.errors, PAGE_ERRORS_LIMIT);
        ensure_bytes_limit(&mut self.errors, |error| {
            error.error.error.as_ref().map_or(0, |error| {
                error.message.len() + error.stack.as_ref().map_or(0, String::len)
            })
        });
    }

    /// The main frame committed a new document: marks the last message and the last error.
    ///
    /// Playwright: `server/page.ts` `Page.frameNavigatedToNewDocument`: `(this._consoleMessages[this._consoleMessages.length - 1] as any)[navigationMarkSymbol] = true;`
    pub(crate) fn mark_navigation(&mut self) {
        if let Some(last) = self.messages.last_mut() {
            last.marked = true;
        }
        if let Some(last) = self.errors.last_mut() {
            last.marked = true;
        }
    }

    /// Every message when `all` (the `all` filter), else those after the last marked one, or
    /// every message when none is marked (`since-navigation`, or no filter).
    ///
    /// Playwright: `server/page.ts` `Page.consoleMessages`: `return marked === -1 ? this._consoleMessages : this._consoleMessages.slice(marked + 1);`
    pub(crate) fn messages(&self, all: bool) -> Vec<ConsoleRecord> {
        since_navigation(&self.messages, all)
            .iter()
            .map(|entry| entry.value.clone())
            .collect()
    }

    /// The errors [`History::messages`] would pick, serialized.
    ///
    /// Playwright: `server/page.ts` `Page.pageErrors`: `return (marked === -1 ? this._pageErrors : this._pageErrors.slice(marked + 1)).map(e => e.error);`
    pub(crate) fn errors(&self, all: bool) -> Vec<SerializedError> {
        since_navigation(&self.errors, all)
            .iter()
            .map(|entry| entry.value.error.clone())
            .collect()
    }

    /// Playwright: `server/page.ts` `Page.clearConsoleMessages`: `this._consoleMessages.length = 0;`
    pub(crate) fn clear_messages(&mut self) {
        self.messages.clear();
    }

    /// Playwright: `server/page.ts` `Page.clearPageErrors`: `this._pageErrors.length = 0;`
    pub(crate) fn clear_errors(&mut self) {
        self.errors.clear();
    }
}

/// Playwright: `server/page.ts` `ensureArrayLimit`: `return array.splice(0, limit / 10);`
fn ensure_array_limit<T>(array: &mut Vec<T>, limit: usize) {
    if array.len() > limit {
        array.drain(..limit / 10);
    }
}

/// Deviation: `server/page.ts` `ensureArrayLimit`: `return array.splice(0, limit / 10);`
/// Playwright trims a list by count only; Neo also drops its oldest entries until the list's text
/// fits [`HISTORY_BYTES_LIMIT`], keeping the newest entry whatever its size.
fn ensure_bytes_limit<T>(entries: &mut Vec<Entry<T>>, bytes: impl Fn(&T) -> usize) {
    let mut total: usize = entries.iter().map(|entry| bytes(&entry.value)).sum();
    let mut dropped = 0;
    for entry in entries.iter().take(entries.len().saturating_sub(1)) {
        if total <= HISTORY_BYTES_LIMIT {
            break;
        }
        total -= bytes(&entry.value);
        dropped += 1;
    }
    entries.drain(..dropped);
}

/// Playwright: `server/page.ts` `Page.consoleMessages`: `const marked = this._consoleMessages.findLastIndex(m => (m as any)[navigationMarkSymbol]);`
fn since_navigation<T>(entries: &[Entry<T>], all: bool) -> &[Entry<T>] {
    if all {
        return entries;
    }
    match entries.iter().rposition(|entry| entry.marked) {
        Some(marked) => entries.get(marked + 1..).unwrap_or_default(),
        None => entries,
    }
}

/// The text of a handle without an object: JS's `String()` of the value it holds.
///
/// Playwright: `server/chromium/crExecutionContext.ts` `potentiallyUnserializableValue`: `return unserializableValue ? js.parseUnserializableValue(unserializableValue) : value;`
fn value_text(object: &RemoteObject) -> String {
    match object
        .unserializable_value
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        Some(text) => parse_unserializable_value(text)
            .map_or_else(|| "undefined".to_string(), progress::js_number),
        None => js_string(object.value.as_ref()),
    }
}

/// `None` is JS's `undefined`: the function knows no bigint, so a bigint argument (`12n`) prints
/// `undefined`.
///
/// Playwright: `server/javascript.ts` `parseUnserializableValue`
fn parse_unserializable_value(text: &str) -> Option<f64> {
    match text {
        "NaN" => Some(f64::NAN),
        "Infinity" => Some(f64::INFINITY),
        "-Infinity" => Some(f64::NEG_INFINITY),
        "-0" => Some(-0.0),
        _ => None,
    }
}

/// Playwright: `server/chromium/crExecutionContext.ts` `renderPreview`
fn render_preview(object: &RemoteObject) -> Option<String> {
    if object.kind == "undefined" {
        return Some("undefined".to_string());
    }
    if let Some(value) = &object.value {
        return Some(js_value_string(value));
    }
    if let Some(text) = object
        .unserializable_value
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_string());
    }
    if object.description.as_deref() == Some("Object")
        && let Some(preview) = &object.preview
    {
        let tokens: Vec<String> = preview
            .properties
            .iter()
            .map(|property| {
                let value = property.value.as_deref().unwrap_or("undefined");
                format!("{}: {value}", property.name)
            })
            .collect();
        return Some(format!("{{{}}}", tokens.join(", ")));
    }
    if object.subtype.as_deref() == Some("array")
        && let Some(preview) = &object.preview
    {
        return Some(sparse_array_to_string(&preview.properties));
    }
    object.description.clone()
}

/// Playwright: `server/javascript.ts` `sparseArrayToString`
fn sparse_array_to_string(entries: &[PropertyPreview]) -> String {
    let mut array_entries: Vec<(f64, &str)> = entries
        .iter()
        .filter_map(|entry| {
            let index = js_string_to_number(&entry.name);
            let value = entry.value.as_deref().unwrap_or("undefined");
            (!index.is_nan() && index >= 0.0).then_some((index, value))
        })
        .collect();
    array_entries.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
    let mut last_index = -1.0;
    let mut tokens = Vec::new();
    for (index, value) in array_entries {
        let empty_items: f64 = index - last_index - 1.0;
        match empty_items.partial_cmp(&1.0) {
            Some(Ordering::Equal) => tokens.push("empty".to_string()),
            Some(Ordering::Greater) => {
                tokens.push(format!("empty x {}", progress::js_number(empty_items)));
            }
            _ => {}
        }
        tokens.push(value.to_string());
        last_index = index;
    }
    format!("[{}]", tokens.join(", "))
}

/// Playwright: `server/chromium/crProtocolHelper.ts` `getExceptionMessage`
fn exception_message(details: &ExceptionDetails) -> String {
    if let Some(exception) = &details.exception {
        return match exception
            .description
            .as_deref()
            .filter(|description| !description.is_empty())
        {
            Some(description) => description.to_string(),
            None => js_string(exception.value.as_ref()),
        };
    }
    let mut message = details.text.clone();
    let Some(stack) = &details.stack_trace else {
        return message;
    };
    for frame in &stack.call_frames {
        // CDP's lines and columns are 0-based, a V8 stack's 1-based.
        let location = format!(
            "{}:{}:{}",
            frame.url,
            frame.line_number.saturating_add(1),
            frame.column_number.saturating_add(1)
        );
        let function_name = if frame.function_name.is_empty() {
            "<anonymous>"
        } else {
            &frame.function_name
        };
        message.push_str("\n    at ");
        message.push_str(function_name);
        message.push_str(" (");
        message.push_str(&location);
        message.push(')');
    }
    message
}

/// The name before the first `:` and the message past `: `, or no name and the whole message.
///
/// Playwright: `utils/stackTrace.ts` `splitErrorMessage`
fn split_error_message(message: &str) -> (String, String) {
    let Some((name, after)) = message.split_once(':') else {
        return (String::new(), message.to_string());
    };
    // Past the `:` and the one UTF-16 unit after it; a `:` that ends the text keeps it whole.
    let mut rest = after.chars();
    let message = match rest.next() {
        None => message.to_string(),
        // Deviation: `utils/stackTrace.ts` `splitErrorMessage`: `message.substring(separationIdx + 2)`
        // Playwright cuts a character outside the BMP in half and keeps its low surrogate, which
        // a Rust string cannot hold; Neo writes U+FFFD there, one UTF-16 unit as in JS.
        Some(skipped) if skipped.len_utf16() == 2 => format!("\u{fffd}{}", rest.as_str()),
        Some(_) => rest.as_str().to_string(),
    };
    (name.to_string(), message)
}

/// Playwright: `server/chromium/crExecutionContext.ts` `renderPreview`: `if ('value' in object)`
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// Platform: ECMAScript `ToString`, which JS's `String(value)` applies: `undefined` for a value
/// CDP did not send.
fn js_string(value: Option<&Value>) -> String {
    value.map_or_else(|| "undefined".to_string(), js_value_string)
}

/// Platform: ECMAScript `ToString` of a JSON value: an array joins its items' strings with `,`
/// (a `null` item as nothing), and an object is `[object Object]`.
fn js_value_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number
            .as_f64()
            .map_or_else(|| number.to_string(), progress::js_number),
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                item => js_value_string(item),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

/// Platform: ECMAScript `StringToNumber`, which JS's unary `+` applies to a string: trimmed of
/// JS white space, empty is 0, `0x`/`0o`/`0b` integers, `Infinity`, and decimal literals; any
/// other text is NaN.
fn js_string_to_number(text: &str) -> f64 {
    let text = text.trim_matches(progress::is_js_whitespace);
    if text.is_empty() {
        return 0.0;
    }
    let radix = match text.get(..2) {
        Some("0x" | "0X") => 16,
        Some("0o" | "0O") => 8,
        Some("0b" | "0B") => 2,
        _ => 10,
    };
    if radix != 10 {
        let digits = text.get(2..).unwrap_or_default();
        if digits.is_empty() {
            return f64::NAN;
        }
        return digits
            .chars()
            .try_fold(0.0_f64, |value, digit| {
                digit
                    .to_digit(radix)
                    .map(|digit| value * f64::from(radix) + f64::from(digit))
            })
            .unwrap_or(f64::NAN);
    }
    match text {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    // Rust's parser also takes `inf`, `nan` and the like; JS's decimal literal does not.
    let decimal = text
        .strip_prefix(['+', '-'])
        .unwrap_or(text)
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'E' | b'+' | b'-'));
    if decimal {
        text.parse().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn object(payload: &Value) -> RemoteObject {
        serde_json::from_value(payload.clone()).expect("a CDP RemoteObject")
    }

    fn details(payload: &Value) -> ExceptionDetails {
        serde_json::from_value(payload.clone()).expect("a CDP ExceptionDetails")
    }

    fn error(message: &str, name: &str, stack: &str) -> SerializedError {
        SerializedError {
            error: Some(SerializedErrorError {
                message: message.to_string(),
                name: name.to_string(),
                stack: Some(stack.to_string()),
                code: None,
            }),
            value: None,
        }
    }

    /// A CDP `Runtime.CallFrame`: 0-based line and column.
    fn frame(function_name: &str, url: &str, line: i64, column: i64) -> Value {
        json!({"functionName": function_name, "scriptId": "9", "url": url,
            "lineNumber": line, "columnNumber": column})
    }

    fn nowhere() -> ConsoleLocation {
        ConsoleLocation {
            url: String::new(),
            line_number: 0,
            column_number: 0,
        }
    }

    fn message(text: &str) -> ConsoleRecord {
        ConsoleRecord {
            r#type: "log".to_string(),
            text: text.to_string(),
            args: Vec::new(),
            location: nowhere(),
            timestamp: 0.0,
        }
    }

    fn page_error(text: &str) -> PageErrorRecord {
        PageErrorRecord {
            error: error(text, "Error", ""),
            location: nowhere(),
        }
    }

    fn texts(messages: &[ConsoleRecord]) -> Vec<&str> {
        messages
            .iter()
            .map(|message| message.text.as_str())
            .collect()
    }

    fn error_messages(errors: &[SerializedError]) -> Vec<&str> {
        errors
            .iter()
            .filter_map(|error| error.error.as_ref())
            .map(|error| error.message.as_str())
            .collect()
    }

    /// Each argument as V8 reports it for `console.log(<value>)`.
    #[test]
    fn tests_that_primitive_arguments_print_as_javascripts_string_does() {
        let cases = [
            (json!({"type": "string", "value": "hello"}), "hello"),
            (
                json!({"type": "number", "value": 5, "description": "5"}),
                "5",
            ),
            (
                json!({"type": "number", "value": 0.1, "description": "0.1"}),
                "0.1",
            ),
            (
                json!({"type": "number", "unserializableValue": "-0", "description": "-0"}),
                "0",
            ),
            (
                json!({"type": "number", "unserializableValue": "NaN", "description": "NaN"}),
                "NaN",
            ),
            (
                json!({"type": "number", "unserializableValue": "-Infinity",
                    "description": "-Infinity"}),
                "-Infinity",
            ),
            // Playwright's `parseUnserializableValue` knows no bigint: the handle holds undefined.
            (
                json!({"type": "bigint", "unserializableValue": "12n", "description": "12n"}),
                "undefined",
            ),
            (json!({"type": "undefined"}), "undefined"),
            (
                json!({"type": "object", "subtype": "null", "value": null}),
                "null",
            ),
            (json!({"type": "boolean", "value": false}), "false"),
        ];
        for (payload, text) in cases {
            assert_eq!(arg_preview(&object(&payload)), text, "{payload}");
        }
    }

    #[test]
    fn tests_that_a_null_value_is_told_apart_from_no_value() {
        let null = json!({"type": "object", "subtype": "null", "value": null});
        assert_eq!(object(&null).value, Some(Value::Null));
        assert_eq!(object(&json!({"type": "undefined"})).value, None);
        let handle = json!({"type": "object", "objectId": "1", "value": null});
        assert_eq!(arg_preview(&object(&handle)), "null", "`'value' in object`");
    }

    #[test]
    fn tests_that_objects_and_arrays_render_their_previews() {
        // console.log({a: 1, b: 'str', get c() {}}): an accessor previews without a value.
        let plain = json!({
            "type": "object", "className": "Object", "description": "Object", "objectId": "1",
            "preview": {"type": "object", "description": "Object", "overflow": false,
                "properties": [
                    {"name": "a", "type": "number", "value": "1"},
                    {"name": "b", "type": "string", "value": "str"},
                    {"name": "c", "type": "accessor"},
                ]},
        });
        assert_eq!(arg_preview(&object(&plain)), "{a: 1, b: str, c: undefined}");
        let array = |properties: Value| {
            object(&json!({
                "type": "object", "subtype": "array", "className": "Array",
                "description": "Array(6)", "objectId": "2",
                "preview": {"type": "object", "subtype": "array", "description": "Array(6)",
                    "overflow": false, "properties": properties},
            }))
        };
        let holes = array(json!([
            {"name": "0", "type": "number", "value": "1"},
            {"name": "2", "type": "number", "value": "3"},
        ]));
        assert_eq!(arg_preview(&holes), "[1, empty, 3]");
        // [, , 'x', , , {}] plus `.extra`: a name that is no number is skipped, `+'05'` is 5.
        let sparse = array(json!([
            {"name": "extra", "type": "number", "value": "9"},
            {"name": "05", "type": "object", "value": "Object"},
            {"name": "2", "type": "string", "value": "x"},
        ]));
        assert_eq!(arg_preview(&sparse), "[empty x 2, x, empty x 2, Object]");
    }

    #[test]
    fn tests_that_handles_without_a_preview_fall_back_as_playwright_does() {
        let cases = [
            (
                json!({"type": "function", "className": "Function",
                    "description": "function f() {}", "objectId": "3"}),
                "function f() {}",
            ),
            (
                json!({"type": "object", "className": "Window", "description": "Window",
                    "objectId": "4", "preview": {"type": "object", "description": "Window",
                        "overflow": true, "properties": []}}),
                "Window",
            ),
            (
                json!({"type": "object", "subtype": "node", "className": "HTMLDivElement",
                    "description": "div#a", "objectId": "5"}),
                "JSHandle@node",
            ),
            (
                json!({"type": "object", "objectId": "6"}),
                "JSHandle@object",
            ),
            (
                json!({"type": "object", "subtype": "array", "objectId": "7"}),
                "JSHandle@array",
            ),
        ];
        for (payload, text) in cases {
            assert_eq!(arg_preview(&object(&payload)), text, "{payload}");
        }
    }

    #[test]
    fn tests_that_a_console_api_call_becomes_a_protocol_message() {
        let event: ConsoleApiCalled = serde_json::from_value(json!({
            "type": "warning",
            "args": [
                {"type": "string", "value": "hello"},
                {"type": "number", "value": 5, "description": "5"},
                {"type": "object", "className": "Object", "description": "Object", "objectId": "1",
                    "preview": {"type": "object", "description": "Object", "overflow": false,
                        "properties": [{"name": "foo", "type": "string", "value": "bar"}]}},
            ],
            "executionContextId": 3,
            "timestamp": 1_700_000_000_123.5,
            "stackTrace": {"callFrames": [
                frame("", "http://x/a.js", 6, 12),
                frame("outer", "http://x/b.js", 1, 0),
            ]},
        }))
        .expect("a consoleAPICalled payload");
        let record = from_console_api(&event).expect("a message");
        assert_eq!(
            serde_json::to_value(&record).expect("a record serializes"),
            json!({
                "type": "warning",
                "text": "hello 5 {foo: bar}",
                "args": [],
                "location": {"url": "http://x/a.js", "lineNumber": 6, "columnNumber": 12},
                "timestamp": 1_700_000_000_123.5,
            })
        );
    }

    #[test]
    fn tests_that_a_message_replayed_from_a_removed_context_is_dropped() {
        let event = |context: i64| -> ConsoleApiCalled {
            serde_json::from_value(json!({
                "type": "log", "args": [], "executionContextId": context, "timestamp": 1.0,
            }))
            .expect("a consoleAPICalled payload")
        };
        assert_eq!(from_console_api(&event(0)), None);
        let record = from_console_api(&event(2)).expect("a message");
        assert_eq!((record.text.as_str(), record.location), ("", nowhere()));
    }

    #[test]
    fn tests_that_log_entries_become_messages_except_a_workers() {
        let added: LogEntryAdded = serde_json::from_value(json!({"entry": {
            "source": "network", "level": "error",
            "text": "Failed to load resource: net::ERR_FAILED",
            "timestamp": 1_700_000_000_000.0, "url": "http://x/missing.png",
            "networkRequestId": "7",
            "args": [{"type": "object", "objectId": "o1"}, {"type": "string", "value": "s"}],
        }}))
        .expect("an entryAdded payload");
        let record = from_log_entry(&added.entry).expect("a message");
        assert_eq!(
            serde_json::to_value(&record).expect("a record serializes"),
            json!({
                "type": "error",
                "text": "Failed to load resource: net::ERR_FAILED",
                "args": [],
                "location": {"url": "http://x/missing.png", "lineNumber": 0, "columnNumber": 0},
                "timestamp": 1_700_000_000_000.0,
            })
        );
        assert_eq!(log_entry_object_ids(&added.entry), ["o1"]);
        let violation = LogEntry {
            url: None,
            line_number: Some(4),
            ..added.entry.clone()
        };
        let location = from_log_entry(&violation).map(|record| record.location);
        assert_eq!(
            location,
            Some(ConsoleLocation {
                line_number: 4,
                ..nowhere()
            })
        );
        let worker = LogEntry {
            source: "worker".to_string(),
            ..added.entry
        };
        assert_eq!(from_log_entry(&worker), None);
        assert_eq!(
            log_entry_object_ids(&worker),
            ["o1"],
            "released all the same"
        );
    }

    #[test]
    fn tests_that_an_error_object_splits_into_name_message_and_stack() {
        let stack = "Error: boom\n    at foo (http://x/a.js:3:9)";
        let thrown: ExceptionThrown = serde_json::from_value(json!({
            "timestamp": 1.0,
            "exceptionDetails": {
                "exceptionId": 1, "text": "Uncaught", "lineNumber": 2, "columnNumber": 8,
                "scriptId": "5", "url": "http://x/a.js",
                "stackTrace": {"callFrames": [frame("foo", "http://x/a.js", 2, 8)]},
                "exception": {"type": "object", "subtype": "error", "className": "Error",
                    "description": stack, "objectId": "9",
                    "preview": {"type": "object", "subtype": "error", "description": stack,
                        "overflow": false, "properties": [
                            {"name": "stack", "type": "string", "value": stack},
                            {"name": "message", "type": "string", "value": "boom"},
                        ]}},
                "executionContextId": 3,
            },
        }))
        .expect("an exceptionThrown payload");
        let record = from_exception(&thrown);
        assert_eq!(
            serde_json::to_value(&record.error).expect("an error serializes"),
            json!({"error": {"message": "boom", "name": "Error", "stack": stack}})
        );
        assert_eq!(
            record.location,
            ConsoleLocation {
                url: "http://x/a.js".to_string(),
                line_number: 2,
                column_number: 8,
            }
        );
    }

    #[test]
    fn tests_that_other_exceptions_follow_playwrights_message_rules() {
        // throw 'odd': no description, so `String(value)`; no `:` and no stack line.
        let odd =
            details(&json!({"text": "Uncaught", "exception": {"type": "string", "value": "odd"}}));
        assert_eq!(exception_to_error(&odd), error("odd", "", ""));
        // No exception object: the text and each frame, 1-based, `<anonymous>` for no name.
        let syntax = details(&json!({
            "text": "Uncaught SyntaxError: Unexpected token ')'",
            "stackTrace": {"callFrames": [
                frame("", "http://x/b.js", 0, 4),
                frame("load", "http://x/c.js", 9, 0),
            ]},
        }));
        assert_eq!(
            exception_to_error(&syntax),
            error(
                "Unexpected token ')'",
                "Uncaught SyntaxError",
                concat!(
                    "Uncaught SyntaxError: Unexpected token ')'\n",
                    "    at <anonymous> (http://x/b.js:1:5)\n",
                    "    at load (http://x/c.js:10:1)",
                ),
            )
        );
        // An own `name` property beats the stack's name; one previewed without a value is Error.
        let named = |name: Value| {
            details(&json!({"text": "Uncaught", "exception": {
                "type": "object", "subtype": "error", "className": "MyError", "objectId": "1",
                "description": "Error: bad\n    at f (http://x/a.js:1:1)",
                "preview": {"type": "object", "subtype": "error", "overflow": false,
                    "properties": [{"name": "message", "type": "string", "value": "bad"}, name]},
            }}))
        };
        let stack = "Error: bad\n    at f (http://x/a.js:1:1)";
        let custom = named(json!({"name": "name", "type": "string", "value": "MyError"}));
        assert_eq!(exception_to_error(&custom), error("bad", "MyError", stack));
        let accessor = named(json!({"name": "name", "type": "accessor"}));
        assert_eq!(exception_to_error(&accessor), error("bad", "Error", stack));
    }

    #[test]
    fn tests_that_an_error_message_splits_as_javascripts_substring_does() {
        let cases = [
            ("Error: boom", "Error", "boom"),
            ("x: y: z", "x", "y: z"),
            ("no colon", "", "no colon"),
            ("trailing:", "trailing", "trailing:"),
            ("a:b", "a", ""),
            ("Error:\u{e9}done", "Error", "done"),
            ("Error:\u{1f600}done", "Error", "\u{fffd}done"),
        ];
        for (text, name, message) in cases {
            assert_eq!(
                split_error_message(text),
                (name.to_string(), message.to_string()),
                "{text:?}"
            );
        }
    }

    #[test]
    fn tests_that_history_keeps_two_hundred_then_drops_the_oldest_twenty() {
        let mut history = History::default();
        for index in 0..=200 {
            history.push_message(message(&index.to_string()));
            history.push_error(page_error(&index.to_string()));
        }
        let messages = history.messages(true);
        let kept = texts(&messages);
        assert_eq!(
            (kept.len(), kept.first(), kept.last()),
            (181, Some(&"20"), Some(&"200"))
        );
        let errors = history.errors(true);
        let kept = error_messages(&errors);
        assert_eq!(
            (kept.len(), kept.first(), kept.last()),
            (181, Some(&"20"), Some(&"200"))
        );
    }

    #[test]
    fn tests_that_history_keeps_at_most_its_bytes_and_always_the_newest() {
        let mut history = History::default();
        let mebibyte = "x".repeat(1 << 20);
        for index in 0..20 {
            history.push_message(message(&format!("{index}{mebibyte}")));
        }
        let kept = history.messages(true);
        assert_eq!(kept.len(), 15, "16 MiB holds 15 of these");
        assert!(kept[14].text.starts_with("19"), "the newest stays");
        history.push_message(message(&"y".repeat(20 << 20)));
        assert_eq!(
            history.messages(true).len(),
            1,
            "a huge newest entry stays alone"
        );
    }

    #[test]
    fn tests_that_since_navigation_reads_after_the_last_marked_entry() {
        let mut history = History::default();
        history.mark_navigation();
        history.push_message(message("a"));
        history.push_error(page_error("a"));
        assert_eq!(
            texts(&history.messages(false)),
            ["a"],
            "nothing marked: all"
        );
        history.mark_navigation();
        history.push_message(message("b"));
        assert_eq!(texts(&history.messages(false)), ["b"]);
        assert_eq!(texts(&history.messages(true)), ["a", "b"]);
        assert!(
            history.errors(false).is_empty(),
            "no error since the navigation"
        );
        assert_eq!(error_messages(&history.errors(true)), ["a"]);
        history.mark_navigation();
        assert!(
            history.messages(false).is_empty(),
            "none since the latest one"
        );
    }

    #[test]
    fn tests_that_a_navigation_mark_travels_with_its_entry_when_the_oldest_are_dropped() {
        let after_mark_at = |marked: usize| {
            let mut history = History::default();
            for index in 0..=200 {
                history.push_message(message(&index.to_string()));
                if index == marked {
                    history.mark_navigation();
                }
            }
            history.messages(false)
        };
        let kept = after_mark_at(29);
        assert_eq!((kept.len(), texts(&kept).first()), (171, Some(&"30")));
        let kept = after_mark_at(9);
        assert_eq!(
            (kept.len(), texts(&kept).first()),
            (181, Some(&"20")),
            "the marked entry was dropped, so nothing is marked"
        );
    }

    #[test]
    fn tests_that_clearing_one_list_keeps_the_other() {
        let mut history = History::default();
        history.push_message(message("a"));
        history.push_error(page_error("a"));
        history.clear_messages();
        assert!(history.messages(true).is_empty());
        assert_eq!(error_messages(&history.errors(true)), ["a"]);
        history.clear_errors();
        assert!(history.errors(true).is_empty());
    }
}
