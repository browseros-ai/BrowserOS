use crate::{PageId, SessionId, connection::CdpConnection};
use browseros_cdp::CdpEvent;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
};
use tokio::sync::broadcast;
use tracing::{debug, warn};

const CONSOLE_RING_CAPACITY: usize = 50;
const ALERT_NOTE_CAPACITY: usize = 10;
const SUMMARY_TEXT_MAX_CHARS: usize = 240;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageDialog {
    pub kind: String,
    pub message: String,
    pub default_prompt: Option<String>,
    pub url: Option<String>,
}

impl PageDialog {
    #[must_use]
    pub fn is_alert(&self) -> bool {
        self.kind == "alert"
    }

    #[must_use]
    pub fn line(&self, page_id: &PageId) -> String {
        let mut line = format!(
            "[page {} dialog open] {}: {}",
            page_id.0,
            self.kind,
            quote(&self.message)
        );
        if self.kind == "prompt"
            && let Some(default_prompt) = self
                .default_prompt
                .as_deref()
                .filter(|value| !value.is_empty())
        {
            line.push_str(&format!(" (default: {})", quote(default_prompt)));
        }
        line.push_str(
            " - use act kind=\"dialog_accept\" or \"dialog_dismiss\" before other actions on this page.",
        );
        line
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleLevel {
    Warning,
    Error,
    Exception,
}

impl ConsoleLevel {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Exception => "exception",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleEntry {
    pub sequence: u64,
    pub level: ConsoleLevel,
    pub text: String,
    pub source: Option<String>,
    pub line: Option<u64>,
    pub column: Option<u64>,
}

impl ConsoleEntry {
    #[must_use]
    pub fn line(&self) -> String {
        let mut line = format!("{}: {}", self.level.as_str(), compact_text(&self.text));
        if let Some(location) = self.location() {
            line.push_str(&format!(" ({location})"));
        }
        line
    }

    #[must_use]
    pub fn summary_text(&self) -> String {
        compact_text(&clamp_chars(&self.text, SUMMARY_TEXT_MAX_CHARS))
    }

    #[must_use]
    pub fn is_warning(&self) -> bool {
        self.level == ConsoleLevel::Warning
    }

    fn location(&self) -> Option<String> {
        let source = self.source.as_deref().filter(|source| !source.is_empty())?;
        Some(match self.line {
            Some(line) => format!("{source}:{line}"),
            None => source.to_string(),
        })
    }
}

#[derive(Default)]
struct PageSignalState {
    pending_dialog: Option<PageDialog>,
    alert_notes: VecDeque<String>,
    console_entries: VecDeque<ConsoleEntry>,
    next_console_sequence: u64,
}

#[derive(Default)]
struct SignalsState {
    sessions: HashMap<SessionId, PageId>,
    pages: HashMap<PageId, PageSignalState>,
}

pub struct PageSignals {
    cdp: Arc<dyn CdpConnection>,
    state: Mutex<SignalsState>,
}

impl PageSignals {
    #[must_use]
    pub fn new(cdp: Arc<dyn CdpConnection>) -> Arc<Self> {
        let signals = Arc::new(Self {
            cdp,
            state: Mutex::new(SignalsState::default()),
        });
        Self::spawn_event_listener(signals.clone());
        signals
    }

    pub fn attach_page(&self, page_id: PageId, session_id: SessionId) {
        let mut state = self.state();
        state
            .pages
            .entry(page_id.clone())
            .or_default()
            .pending_dialog = None;
        state.sessions.insert(session_id, page_id);
    }

    pub fn detach_session(&self, session_id: &SessionId) {
        let mut state = self.state();
        let Some(page_id) = state.sessions.remove(session_id) else {
            return;
        };
        if let Some(page) = state.pages.get_mut(&page_id) {
            page.pending_dialog = None;
        }
    }

    pub fn forget_page(&self, page_id: &PageId) {
        let mut state = self.state();
        state.sessions.retain(|_, existing| existing != page_id);
        state.pages.remove(page_id);
    }

    #[must_use]
    pub fn pending_dialog(&self, page_id: &PageId) -> Option<PageDialog> {
        self.state()
            .pages
            .get(page_id)
            .and_then(|page| page.pending_dialog.clone())
    }

    #[must_use]
    pub fn pending_dialog_line(&self, page_id: &PageId) -> Option<String> {
        self.pending_dialog(page_id)
            .map(|dialog| dialog.line(page_id))
    }

    pub fn clear_dialog(&self, page_id: &PageId) {
        if let Some(page) = self.state().pages.get_mut(page_id) {
            page.pending_dialog = None;
        }
    }

    #[must_use]
    pub fn console_mark(&self, page_id: &PageId) -> u64 {
        self.state()
            .pages
            .get(page_id)
            .map(|page| page.next_console_sequence)
            .unwrap_or(0)
    }

    #[must_use]
    pub fn console_entries(&self, page_id: &PageId) -> Vec<ConsoleEntry> {
        self.state()
            .pages
            .get(page_id)
            .map(|page| page.console_entries.iter().cloned().collect())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn console_entries_since(&self, page_id: &PageId, sequence: u64) -> Vec<ConsoleEntry> {
        self.state()
            .pages
            .get(page_id)
            .map(|page| {
                page.console_entries
                    .iter()
                    .filter(|entry| entry.sequence >= sequence)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn take_alert_note_lines(&self, page_id: &PageId) -> Vec<String> {
        let mut state = self.state();
        let Some(page) = state.pages.get_mut(page_id) else {
            return Vec::new();
        };
        page.alert_notes
            .drain(..)
            .map(|message| {
                format!(
                    "[page {} dismissed an alert: {}]",
                    page_id.0,
                    quote(&message)
                )
            })
            .collect()
    }

    fn spawn_event_listener(signals: Arc<Self>) {
        let mut events = signals.cdp.events();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => signals.handle_event(event).await,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("browser page signal listener lagged by {skipped} CDP event(s)");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    async fn handle_event(&self, event: CdpEvent) {
        match event.method.as_str() {
            "Page.javascriptDialogOpening" => self.handle_dialog_opening(event).await,
            "Page.javascriptDialogClosed" => self.handle_dialog_closed(event),
            "Runtime.consoleAPICalled" => self.handle_console_api_called(event),
            "Runtime.exceptionThrown" => self.handle_exception_thrown(event),
            _ => {}
        }
    }

    async fn handle_dialog_opening(&self, event: CdpEvent) {
        let Some((page_id, session_id)) = self.page_for_event(&event) else {
            return;
        };
        let dialog = dialog_from_params(&event.params);
        if dialog.is_alert() {
            self.record_alert_note(&page_id, dialog.message.clone());
            self.accept_alert(session_id).await;
            return;
        }
        let mut state = self.state();
        state.pages.entry(page_id).or_default().pending_dialog = Some(dialog);
    }

    fn handle_dialog_closed(&self, event: CdpEvent) {
        let Some((page_id, _session_id)) = self.page_for_event(&event) else {
            return;
        };
        self.clear_dialog(&page_id);
    }

    fn handle_console_api_called(&self, event: CdpEvent) {
        let Some((page_id, _session_id)) = self.page_for_event(&event) else {
            return;
        };
        let Some(entry) = console_entry_from_params(&event.params) else {
            return;
        };
        self.push_console_entry(&page_id, entry);
    }

    fn handle_exception_thrown(&self, event: CdpEvent) {
        let Some((page_id, _session_id)) = self.page_for_event(&event) else {
            return;
        };
        let entry = exception_entry_from_params(&event.params);
        self.push_console_entry(&page_id, entry);
    }

    fn page_for_event(&self, event: &CdpEvent) -> Option<(PageId, SessionId)> {
        let session_id = event.session_id.clone()?;
        let page_id = self.state().sessions.get(&session_id).cloned()?;
        Some((page_id, session_id))
    }

    async fn accept_alert(&self, session_id: SessionId) {
        let result = self
            .cdp
            .send(
                "Page.handleJavaScriptDialog",
                json!({ "accept": true }),
                Some(&session_id),
            )
            .await;
        if let Err(err) = result {
            debug!("failed to auto-accept JavaScript alert: {err}");
        }
    }

    fn record_alert_note(&self, page_id: &PageId, message: String) {
        let mut state = self.state();
        let page = state.pages.entry(page_id.clone()).or_default();
        page.pending_dialog = None;
        if page.alert_notes.len() == ALERT_NOTE_CAPACITY {
            page.alert_notes.pop_front();
        }
        page.alert_notes.push_back(message);
    }

    fn push_console_entry(&self, page_id: &PageId, mut entry: ConsoleEntry) {
        let mut state = self.state();
        let page = state.pages.entry(page_id.clone()).or_default();
        entry.sequence = page.next_console_sequence;
        page.next_console_sequence = page.next_console_sequence.saturating_add(1);
        if page.console_entries.len() == CONSOLE_RING_CAPACITY {
            page.console_entries.pop_front();
        }
        page.console_entries.push_back(entry);
    }

    fn state(&self) -> MutexGuard<'_, SignalsState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn dialog_from_params(params: &Value) -> PageDialog {
    PageDialog {
        kind: params
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("dialog")
            .to_string(),
        message: params
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        default_prompt: params
            .get("defaultPrompt")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        url: params
            .get("url")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    }
}

fn console_entry_from_params(params: &Value) -> Option<ConsoleEntry> {
    let level = match params.get("type").and_then(Value::as_str) {
        Some("warning") => ConsoleLevel::Warning,
        Some("error") => ConsoleLevel::Error,
        _ => return None,
    };
    let text = params
        .get("args")
        .and_then(Value::as_array)
        .map(|args| {
            args.iter()
                .map(remote_object_text)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            params
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        });
    let (source, line, column) = stack_location(params.get("stackTrace"));
    Some(ConsoleEntry {
        sequence: 0,
        level,
        text,
        source,
        line,
        column,
    })
}

fn exception_entry_from_params(params: &Value) -> ConsoleEntry {
    let details = params.get("exceptionDetails").unwrap_or(params);
    let text = details
        .get("exception")
        .and_then(|exception| {
            exception
                .get("description")
                .or_else(|| exception.get("value"))
                .map(remote_object_text)
        })
        .filter(|value| !value.is_empty())
        .or_else(|| {
            details
                .get("text")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| "Uncaught exception".to_string());
    let (stack_source, stack_line, stack_column) = stack_location(details.get("stackTrace"));
    ConsoleEntry {
        sequence: 0,
        level: ConsoleLevel::Exception,
        text,
        source: stack_source.or_else(|| {
            details
                .get("url")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        }),
        line: stack_line.or_else(|| one_based(details.get("lineNumber"))),
        column: stack_column.or_else(|| one_based(details.get("columnNumber"))),
    }
}

fn remote_object_text(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    if value.is_number() || value.is_boolean() {
        return value.to_string();
    }
    if let Some(text) = value.get("value") {
        return remote_object_text(text);
    }
    for key in ["description", "unserializableValue", "type"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            return text.to_string();
        }
    }
    if value.is_null() {
        String::new()
    } else {
        value.to_string()
    }
}

fn stack_location(stack_trace: Option<&Value>) -> (Option<String>, Option<u64>, Option<u64>) {
    let Some(frame) = stack_trace
        .and_then(|stack| stack.get("callFrames"))
        .and_then(Value::as_array)
        .and_then(|frames| frames.first())
    else {
        return (None, None, None);
    };
    (
        frame
            .get("url")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        one_based(frame.get("lineNumber")),
        one_based(frame.get("columnNumber")),
    )
}

fn one_based(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64).map(|value| value + 1)
}

fn quote(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_err| format!("\"{value}\""))
}

fn compact_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clamp_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::PageSignals;
    use crate::{CdpConnection, PageId, SessionId};
    use browseros_cdp::{CdpError, CdpEvent};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::sync::broadcast;

    #[derive(Debug, Clone)]
    struct SentCall {
        method: String,
        params: Value,
        session: Option<SessionId>,
    }

    struct TestConnection {
        sender: broadcast::Sender<CdpEvent>,
        sent: Mutex<Vec<SentCall>>,
    }

    impl TestConnection {
        fn new() -> Arc<Self> {
            let (sender, _receiver) = broadcast::channel(128);
            Arc::new(Self {
                sender,
                sent: Mutex::new(Vec::new()),
            })
        }

        fn emit(&self, method: &str, params: Value, session_id: Option<&str>) {
            let _ = self.sender.send(CdpEvent {
                method: method.to_string(),
                params,
                session_id: session_id.map(SessionId::from),
            });
        }

        fn sent_calls(&self) -> Vec<SentCall> {
            self.sent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    }

    impl CdpConnection for TestConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                self.sent
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(SentCall {
                        method: method.to_string(),
                        params,
                        session: session.cloned(),
                    });
                Ok(json!({}))
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async move {
                Err(CdpError::Protocol {
                    code: -1,
                    message: format!("unexpected raw call: {method}"),
                })
            })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            self.sender.subscribe()
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    fn harness() -> (Arc<PageSignals>, Arc<TestConnection>, PageId) {
        let connection = TestConnection::new();
        let signals = PageSignals::new(connection.clone());
        let page_id = PageId(7);
        signals.attach_page(page_id.clone(), SessionId::from("session-7"));
        (signals, connection, page_id)
    }

    async fn eventually(mut condition: impl FnMut() -> bool) {
        for _attempt in 0..50 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(condition());
    }

    #[tokio::test]
    async fn tracks_pending_dialog_and_clears_on_close() {
        let (signals, connection, page_id) = harness();
        connection.emit(
            "Page.javascriptDialogOpening",
            json!({
                "type": "confirm",
                "message": "Leave site?",
                "url": "https://example.test/"
            }),
            Some("session-7"),
        );

        eventually(|| signals.pending_dialog(&page_id).is_some()).await;
        let line = signals.pending_dialog_line(&page_id).unwrap_or_default();
        assert!(line.contains("[page 7 dialog open] confirm: \"Leave site?\""));
        assert!(line.contains("act kind=\"dialog_accept\""));

        connection.emit("Page.javascriptDialogClosed", json!({}), Some("session-7"));
        eventually(|| signals.pending_dialog(&page_id).is_none()).await;
    }

    #[tokio::test]
    async fn auto_accepts_alert_and_records_next_response_note() {
        let (signals, connection, page_id) = harness();
        connection.emit(
            "Page.javascriptDialogOpening",
            json!({
                "type": "alert",
                "message": "Saved",
                "url": "https://example.test/"
            }),
            Some("session-7"),
        );

        eventually(|| {
            connection.sent_calls().iter().any(|call| {
                call.method == "Page.handleJavaScriptDialog"
                    && call.params.get("accept").and_then(Value::as_bool) == Some(true)
                    && call.session.as_ref() == Some(&SessionId::from("session-7"))
            })
        })
        .await;
        assert!(signals.pending_dialog(&page_id).is_none());
        assert_eq!(
            signals.take_alert_note_lines(&page_id),
            vec!["[page 7 dismissed an alert: \"Saved\"]"]
        );
        assert!(signals.take_alert_note_lines(&page_id).is_empty());
    }

    #[tokio::test]
    async fn captures_console_ring_with_exceptions_newest_last() {
        let (signals, connection, page_id) = harness();
        for index in 0..55 {
            connection.emit(
                "Runtime.consoleAPICalled",
                json!({
                    "type": "error",
                    "args": [{ "type": "string", "value": format!("boom {index}") }],
                    "stackTrace": {
                        "callFrames": [{
                            "url": "https://example.test/app.js",
                            "lineNumber": index,
                            "columnNumber": 0
                        }]
                    }
                }),
                Some("session-7"),
            );
        }
        connection.emit(
            "Runtime.exceptionThrown",
            json!({
                "exceptionDetails": {
                    "text": "Uncaught",
                    "url": "https://example.test/app.js",
                    "lineNumber": 99,
                    "columnNumber": 2,
                    "exception": {
                        "description": "TypeError: x is undefined"
                    }
                }
            }),
            Some("session-7"),
        );

        eventually(|| signals.console_entries(&page_id).len() == 50).await;
        let entries = signals.console_entries(&page_id);
        assert_eq!(entries.len(), 50);
        assert_eq!(entries.first().map(|entry| entry.sequence), Some(6));
        assert_eq!(entries.last().map(|entry| entry.sequence), Some(55));
        assert_eq!(
            entries.last().map(super::ConsoleEntry::line),
            Some(
                "exception: TypeError: x is undefined (https://example.test/app.js:100)"
                    .to_string()
            )
        );
    }
}
