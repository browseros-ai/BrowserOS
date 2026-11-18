use crate::{
    CoreError, PageId, ProtocolSession, SessionId, connection::CdpConnection,
    runtime_lease::RuntimeLeases,
};
use browseros_cdp::CdpEvent;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{broadcast, watch};
use tracing::{debug, warn};

const CONSOLE_RING_CAPACITY: usize = 50;
const ALERT_NOTE_CAPACITY: usize = 10;
const SUMMARY_TEXT_MAX_CHARS: usize = 240;
/// Slack between this process's clock and the browser's console timestamps when a
/// [`ConsoleMark`] decides whether an entry was logged after the mark.
const CONSOLE_MARK_SKEW_US: u64 = 50_000;

/// Where an action started in a page's console history: entries pushed after `sequence` and,
/// when the browser stamped them, logged after `logged_after_us` (microseconds since epoch).
///
/// The timestamp matters because a harvest pushes the page's whole stored history, including
/// entries logged long before the action that are only now being seen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConsoleMark {
    pub sequence: u64,
    pub logged_after_us: u64,
}

/// A position in a page's document events, taken before an action so a settle can tell the
/// navigations that action caused from earlier ones.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DocumentMark(u64);

/// Where a page's main-frame document stands, counted from a [`DocumentMark`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentSince {
    /// No cross-document navigation of the main frame started after the mark.
    Unchanged,
    /// One started and its document has not yet committed and reached DOMContentLoaded.
    Loading,
    /// The latest one committed and reached DOMContentLoaded, or stopped loading without a
    /// document, as a download or a 204 does.
    Arrived,
}

/// The latest navigation events of one frame, as positions in its page's event sequence.
///
/// Latest-only per frame rather than a shared history, so no amount of iframe traffic can push
/// the main frame's pending navigation out of memory before its document arrives.
#[derive(Debug, Clone, Copy, Default)]
struct FrameDocument {
    started: Option<u64>,
    /// `Page.frameNavigated`: a new document replaced the old one, before it starts parsing.
    committed: Option<u64>,
    stopped: Option<u64>,
}

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
    /// When the page logged it, in microseconds since epoch, as stamped by the browser.
    pub timestamp_us: Option<u64>,
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
    /// The newest browser timestamp recorded, and the texts recorded at exactly that time.
    /// Every harvest replays the page's whole stored history; anything at or before the
    /// watermark was already recorded by an earlier one.
    console_watermark: Option<(u64, Vec<String>)>,
    /// Frames that live on the page's own session, keyed by frame id. Out-of-process frames
    /// report on sessions of their own and never land here; same-process ones leave on
    /// `Page.frameDetached`.
    frame_documents: HashMap<String, FrameDocument>,
    /// The latest `Page.domContentEventFired`, which Chromium sends only for the root frame.
    content_loaded: Option<u64>,
    next_document_sequence: u64,
}

#[derive(Default)]
struct SignalsState {
    sessions: HashMap<SessionId, PageId>,
    pages: HashMap<PageId, PageSignalState>,
}

type SignalAdmission = Arc<dyn Fn(&PageId) -> bool + Send + Sync>;

/// Shared page events, optionally projected through a view's cached admission predicate.
pub struct PageSignals {
    cdp: Arc<dyn CdpConnection>,
    state: Arc<Mutex<SignalsState>>,
    scope: Option<SignalAdmission>,
    /// Bumped after every recorded document event, on any page. Waiters re-read their own
    /// page's state when it moves; scoped projections share it with the source.
    document_changes: Arc<watch::Sender<u64>>,
    /// Who holds `Runtime` on, per session. Lives here because `harvest_console` toggles the
    /// domain itself and must leave it on under a lease; projections share the source's.
    runtime: Arc<RuntimeLeases>,
}

impl PageSignals {
    #[must_use]
    pub fn new(cdp: Arc<dyn CdpConnection>) -> Arc<Self> {
        let signals = Arc::new(Self {
            cdp,
            state: Arc::new(Mutex::new(SignalsState::default())),
            scope: None,
            document_changes: Arc::new(watch::channel(0).0),
            runtime: RuntimeLeases::new(),
        });
        Self::spawn_event_listener(signals.clone());
        signals
    }

    /// Read-only projection over the global event cache. Task tools can inspect admitted
    /// pages but cannot rewrite session ownership or trigger infrastructure alert handling.
    pub(crate) fn scoped(source: &Arc<Self>, scope: SignalAdmission) -> Arc<Self> {
        Arc::new(Self {
            cdp: source.cdp.clone(),
            state: source.state.clone(),
            scope: Some(scope),
            document_changes: source.document_changes.clone(),
            runtime: source.runtime.clone(),
        })
    }

    /// The connection's `Runtime` leases: take one to keep `Runtime` events flowing on a
    /// session (see [`crate::runtime_lease`]).
    #[must_use]
    pub fn runtime_leases(&self) -> &Arc<RuntimeLeases> {
        &self.runtime
    }
    fn allowed(&self, page: &PageId) -> bool {
        self.scope.as_ref().is_none_or(|scope| scope(page))
    }

    pub fn attach_page(&self, page_id: PageId, session_id: SessionId) {
        if self.scope.is_some() {
            return;
        }
        let mut state = self.state();
        state
            .pages
            .entry(page_id.clone())
            .or_default()
            .pending_dialog = None;
        state.sessions.insert(session_id, page_id);
    }

    pub fn detach_session(&self, session_id: &SessionId) {
        if self.scope.is_some() {
            return;
        }
        let mut state = self.state();
        let Some(page_id) = state.sessions.remove(session_id) else {
            return;
        };
        if let Some(page) = state.pages.get_mut(&page_id) {
            page.pending_dialog = None;
        }
    }

    pub fn forget_page(&self, page_id: &PageId) {
        if self.scope.is_some() {
            return;
        }
        let mut state = self.state();
        state.sessions.retain(|_, existing| existing != page_id);
        state.pages.remove(page_id);
    }

    #[must_use]
    pub fn pending_dialog(&self, page_id: &PageId) -> Option<PageDialog> {
        if !self.allowed(page_id) {
            return None;
        }
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
        if self.scope.is_some() {
            return;
        }
        if let Some(page) = self.state().pages.get_mut(page_id) {
            page.pending_dialog = None;
        }
    }

    #[must_use]
    pub fn console_mark(&self, page_id: &PageId) -> ConsoleMark {
        let logged_after_us = now_us().saturating_sub(CONSOLE_MARK_SKEW_US);
        if !self.allowed(page_id) {
            return ConsoleMark {
                sequence: 0,
                logged_after_us,
            };
        }
        let sequence = self
            .state()
            .pages
            .get(page_id)
            .map(|page| page.next_console_sequence)
            .unwrap_or(0);
        ConsoleMark {
            sequence,
            logged_after_us,
        }
    }

    /// Pulls the page's console warnings, errors and uncaught exceptions into the ring.
    ///
    /// Sessions do not keep `Runtime` enabled: while it is, the page's own console calls
    /// serialize their arguments for the inspector, which pages time to detect DevTools and
    /// then blank or close themselves. Enabling it makes the browser replay the stored
    /// history before it answers; the disable follows the enable immediately on the same
    /// session, in order, so reporting is on only for the brief span between the pair.
    pub async fn harvest_console(
        &self,
        page_id: &PageId,
        session: &ProtocolSession,
    ) -> Result<(), CoreError> {
        let Some(session_id) = session.session_id().cloned() else {
            return Ok(());
        };
        if !self.allowed(page_id) {
            return Ok(());
        }
        // A held lease keeps reporting on, so the listener below already records the page's
        // console live; the pair would switch it off under the lease's holder.
        let Some(_turn) = self.runtime.unleased_turn(&session_id).await else {
            return Ok(());
        };
        let mut events = self.cdp.events();
        // Enable, then disable, as an ordered pair on the same session. send_value re-runs
        // page admission before each call, so issuing the two concurrently let their
        // admissions finish out of order and could put Runtime.disable on the wire before
        // Runtime.enable, leaving reporting on (the live test uses an unguarded session and
        // misses this). CDP keeps a session's messages in order, so an ordered pair always
        // applies enable before disable.
        session.send_value("Runtime.enable", json!({})).await?;
        session.send_value("Runtime.disable", json!({})).await?;
        loop {
            let event = match events.try_recv() {
                Ok(event) => event,
                Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                    warn!("console harvest lagged by {skipped} CDP event(s)");
                    continue;
                }
                Err(_) => break,
            };
            if event.session_id.as_ref() != Some(&session_id) {
                continue;
            }
            match event.method.as_str() {
                "Runtime.consoleAPICalled" => {
                    if let Some(entry) = console_entry_from_params(&event.params) {
                        self.push_console_entry(page_id, entry);
                    }
                }
                "Runtime.exceptionThrown" => {
                    self.push_console_entry(page_id, exception_entry_from_params(&event.params));
                }
                _ => {}
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn console_entries(&self, page_id: &PageId) -> Vec<ConsoleEntry> {
        if !self.allowed(page_id) {
            return Vec::new();
        }
        self.state()
            .pages
            .get(page_id)
            .map(|page| page.console_entries.iter().cloned().collect())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn console_entries_since(&self, page_id: &PageId, mark: ConsoleMark) -> Vec<ConsoleEntry> {
        if !self.allowed(page_id) {
            return Vec::new();
        }
        self.state()
            .pages
            .get(page_id)
            .map(|page| {
                page.console_entries
                    .iter()
                    .filter(|entry| {
                        entry.sequence >= mark.sequence
                            && entry
                                .timestamp_us
                                .is_none_or(|logged| logged >= mark.logged_after_us)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    #[must_use]
    pub fn document_mark(&self, page_id: &PageId) -> DocumentMark {
        if !self.allowed(page_id) {
            return DocumentMark::default();
        }
        DocumentMark(
            self.state()
                .pages
                .get(page_id)
                .map(|page| page.next_document_sequence)
                .unwrap_or(0),
        )
    }

    /// Whether the page's main frame started a new document after `mark`, and whether that
    /// document has arrived.
    ///
    /// `main_frame` is the page's target id: Chromium names a tab's main frame with the same
    /// devtools token as its target. Events of any other frame on the session (same-process
    /// iframes) are ignored, so an ad iframe navigating never reads as the page navigating.
    #[must_use]
    pub fn document_since(
        &self,
        page_id: &PageId,
        main_frame: &str,
        mark: DocumentMark,
    ) -> DocumentSince {
        if !self.allowed(page_id) {
            return DocumentSince::Unchanged;
        }
        let state = self.state();
        let Some(page) = state.pages.get(page_id) else {
            return DocumentSince::Unchanged;
        };
        let Some(main) = page.frame_documents.get(main_frame) else {
            return DocumentSince::Unchanged;
        };
        let Some(started) = main.started.filter(|started| *started >= mark.0) else {
            return DocumentSince::Unchanged;
        };
        let after = |sequence: Option<u64>, floor: u64| sequence.is_some_and(|at| at > floor);
        // The page being left keeps parsing until the new document commits, so a click made
        // while it is still loading can see the old page's own DOMContentLoaded after the
        // start. Only one after the commit belongs to the new document. Stopping needs no
        // commit: the browser counts the pending navigation as loading until it ends.
        let content_arrived = main
            .committed
            .filter(|committed| *committed > started)
            .is_some_and(|committed| after(page.content_loaded, committed));
        if content_arrived || after(main.stopped, started) {
            DocumentSince::Arrived
        } else {
            DocumentSince::Loading
        }
    }

    /// Changes whenever a document event is recorded on any page. Subscribe before reading
    /// [`Self::document_since`] so an event landing between the read and the wait still wakes
    /// the waiter.
    #[must_use]
    pub fn document_changes(&self) -> watch::Receiver<u64> {
        self.document_changes.subscribe()
    }

    pub fn take_alert_note_lines(&self, page_id: &PageId) -> Vec<String> {
        if !self.allowed(page_id) {
            return Vec::new();
        }
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
            "Page.frameStartedNavigating" => {
                // Same-document navigations (pushState, fragments) keep the document, so the
                // DOM quiet check covers them; only a new document is worth waiting for.
                let cross_document = matches!(
                    event.params.get("navigationType").and_then(Value::as_str),
                    Some(
                        "differentDocument"
                            | "historyDifferentDocument"
                            | "reload"
                            | "reloadBypassingCache"
                            | "restore"
                            | "restoreWithPost"
                    )
                );
                if cross_document {
                    self.record_document_event(&event, |page, sequence| {
                        frame_document(page, &event).started = Some(sequence);
                    });
                }
            }
            // Fires only for a new document (same-document changes report
            // `navigatedWithinDocument`), including a back-forward cache restore, which then
            // arrives through `frameStoppedLoading` since it never fires DOMContentLoaded again.
            "Page.frameNavigated" => {
                self.record_document_event(&event, |page, sequence| {
                    frame_document(page, &event).committed = Some(sequence);
                });
            }
            "Page.domContentEventFired" => {
                self.record_document_event(&event, |page, sequence| {
                    page.content_loaded = Some(sequence);
                });
            }
            "Page.frameStoppedLoading" => {
                self.record_document_event(&event, |page, sequence| {
                    frame_document(page, &event).stopped = Some(sequence);
                });
            }
            // A `swap` moves a frame to another process and it keeps navigating; only a
            // removed frame is gone for good.
            "Page.frameDetached"
                if event.params.get("reason").and_then(Value::as_str) == Some("remove") =>
            {
                let Some((page_id, _session_id)) = self.page_for_event(&event) else {
                    return;
                };
                if let (Some(page), Some(frame)) = (
                    self.state().pages.get_mut(&page_id),
                    event.params.get("frameId").and_then(Value::as_str),
                ) {
                    page.frame_documents.remove(frame);
                }
            }
            _ => {}
        }
    }

    /// Stamps the next position in the page's document sequence through `apply`, then wakes
    /// anyone waiting on [`Self::document_changes`].
    fn record_document_event(
        &self,
        event: &CdpEvent,
        apply: impl FnOnce(&mut PageSignalState, u64),
    ) {
        let Some((page_id, _session_id)) = self.page_for_event(event) else {
            return;
        };
        {
            let mut state = self.state();
            let page = state.pages.entry(page_id).or_default();
            let sequence = page.next_document_sequence;
            page.next_document_sequence = sequence.saturating_add(1);
            apply(page, sequence);
        }
        self.document_changes
            .send_modify(|changes| *changes = changes.wrapping_add(1));
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
        if let Some(logged) = entry.timestamp_us {
            // Dedupe replayed history by (level, location, text), not text alone: two
            // entries can share a microsecond timestamp yet differ in level or source (for
            // example a warn and an error with the same message), and keying on text alone
            // would drop the second and lose a distinct error.
            let key = format!(
                "{}\u{1f}{}\u{1f}{}",
                entry.level.as_str(),
                entry.location().unwrap_or_default(),
                entry.text
            );
            match &mut page.console_watermark {
                Some((newest, _)) if logged < *newest => return,
                Some((newest, keys)) if logged == *newest => {
                    if keys.contains(&key) {
                        return;
                    }
                    keys.push(key);
                }
                watermark => *watermark = Some((logged, vec![key])),
            }
        }
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

/// The named frame's entry, created on its first event. An event without a frame id is
/// malformed and lands under the empty id, which no target id ever matches.
fn frame_document<'a>(page: &'a mut PageSignalState, event: &CdpEvent) -> &'a mut FrameDocument {
    // `Page.frameNavigated` carries the whole frame; the other navigation events, its id.
    let frame = event
        .params
        .get("frameId")
        .or_else(|| event.params.pointer("/frame/id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    page.frame_documents.entry(frame.to_string()).or_default()
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
        timestamp_us: browser_timestamp_us(params),
    })
}

/// `Runtime.Timestamp`: milliseconds since epoch, with a fraction.
fn browser_timestamp_us(params: &Value) -> Option<u64> {
    params
        .get("timestamp")
        .and_then(Value::as_f64)
        .filter(|ms| ms.is_finite() && *ms >= 0.0)
        .map(|ms| (ms * 1000.0) as u64)
}

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX))
        .unwrap_or(0)
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
        timestamp_us: browser_timestamp_us(params),
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
    use super::{DocumentSince, PageSignals};
    use crate::{CdpConnection, PageId, ProtocolSession, SessionId};
    use browseros_cdp::{CdpError, CdpEvent, TargetedEvents};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use std::{
        error::Error,
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::sync::{broadcast, mpsc};

    #[derive(Debug, Clone)]
    struct SentCall {
        method: String,
        params: Value,
        session: Option<SessionId>,
    }

    struct TestConnection {
        sender: broadcast::Sender<CdpEvent>,
        targeted: TargetedEvents,
        sent: Mutex<Vec<SentCall>>,
        /// What the browser replays from its console storage when `Runtime.enable` arrives.
        stored_console: Mutex<Vec<(String, Value)>>,
    }

    impl TestConnection {
        fn new() -> Arc<Self> {
            let (sender, _receiver) = broadcast::channel(128);
            Arc::new(Self {
                sender,
                targeted: TargetedEvents::default(),
                sent: Mutex::new(Vec::new()),
                stored_console: Mutex::new(Vec::new()),
            })
        }

        fn store_console(&self, method: &str, params: Value) {
            self.stored_console
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((method.to_string(), params));
        }

        fn emit(&self, method: &str, params: Value, session_id: Option<&str>) {
            let event = CdpEvent {
                method: method.to_string(),
                params,
                session_id: session_id.map(SessionId::from),
            };
            if let Some(event) = self.targeted.deliver(event) {
                let _ = self.sender.send(event);
            }
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
                if method == "Runtime.enable" {
                    let stored = self
                        .stored_console
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone();
                    for (event, params) in stored {
                        self.emit(&event, params, session.map(SessionId::as_str));
                    }
                }
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

        fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
            self.targeted.subscribe(method)
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

    fn navigation(frame: &str, navigation_type: &str) -> Value {
        json!({
            "frameId": frame,
            "url": "https://example.test/next",
            "loaderId": "loader-2",
            "navigationType": navigation_type
        })
    }

    fn committed(frame: &str) -> Value {
        json!({
            "frame": { "id": frame, "loaderId": "loader-2", "url": "https://example.test/next" },
            "type": "Navigation"
        })
    }

    #[tokio::test]
    async fn a_main_frame_navigation_is_loading_until_its_content_loads() {
        let (signals, connection, page_id) = harness();
        let mark = signals.document_mark(&page_id);
        assert_eq!(
            signals.document_since(&page_id, "main", mark),
            DocumentSince::Unchanged
        );

        connection.emit(
            "Page.frameStartedNavigating",
            navigation("main", "differentDocument"),
            Some("session-7"),
        );
        eventually(|| signals.document_since(&page_id, "main", mark) == DocumentSince::Loading)
            .await;

        connection.emit("Page.frameNavigated", committed("main"), Some("session-7"));
        connection.emit(
            "Page.domContentEventFired",
            json!({ "timestamp": 1.0 }),
            Some("session-7"),
        );
        eventually(|| signals.document_since(&page_id, "main", mark) == DocumentSince::Arrived)
            .await;
    }

    #[tokio::test]
    async fn the_old_pages_late_content_loaded_does_not_count_as_the_new_document() {
        let (signals, connection, page_id) = harness();
        let mark = signals.document_mark(&page_id);
        connection.emit(
            "Page.frameStartedNavigating",
            navigation("main", "differentDocument"),
            Some("session-7"),
        );
        // The page being left was still loading and finishes parsing before the new one commits.
        connection.emit(
            "Page.domContentEventFired",
            json!({ "timestamp": 1.0 }),
            Some("session-7"),
        );
        eventually(|| signals.document_mark(&page_id).0 == mark.0 + 2).await;
        assert_eq!(
            signals.document_since(&page_id, "main", mark),
            DocumentSince::Loading
        );

        connection.emit("Page.frameNavigated", committed("main"), Some("session-7"));
        eventually(|| signals.document_mark(&page_id).0 == mark.0 + 3).await;
        assert_eq!(
            signals.document_since(&page_id, "main", mark),
            DocumentSince::Loading
        );

        connection.emit(
            "Page.domContentEventFired",
            json!({ "timestamp": 2.0 }),
            Some("session-7"),
        );
        eventually(|| signals.document_since(&page_id, "main", mark) == DocumentSince::Arrived)
            .await;
    }

    #[tokio::test]
    async fn a_navigation_that_stops_without_a_document_has_arrived() {
        let (signals, connection, page_id) = harness();
        let mark = signals.document_mark(&page_id);
        connection.emit(
            "Page.frameStartedNavigating",
            navigation("main", "differentDocument"),
            Some("session-7"),
        );
        // A download or a 204 stops loading without ever firing DOMContentLoaded.
        connection.emit(
            "Page.frameStoppedLoading",
            json!({ "frameId": "main" }),
            Some("session-7"),
        );
        eventually(|| signals.document_since(&page_id, "main", mark) == DocumentSince::Arrived)
            .await;
    }

    #[tokio::test]
    async fn subframe_same_document_and_earlier_navigations_leave_the_document_unchanged() {
        let (signals, connection, page_id) = harness();
        let before = signals.document_mark(&page_id);
        connection.emit(
            "Page.frameStartedNavigating",
            navigation("main", "differentDocument"),
            Some("session-7"),
        );
        eventually(|| signals.document_mark(&page_id) != before).await;

        let mark = signals.document_mark(&page_id);
        connection.emit(
            "Page.frameStartedNavigating",
            navigation("child", "differentDocument"),
            Some("session-7"),
        );
        connection.emit(
            "Page.frameStartedNavigating",
            navigation("main", "sameDocument"),
            Some("session-7"),
        );
        connection.emit(
            "Page.frameStartedNavigating",
            navigation("main", "differentDocument"),
            Some("session-other"),
        );
        // Only the child's start and stop are recorded; once both are, the listener has handled
        // every event above, so the assertion below cannot run ahead of them.
        connection.emit(
            "Page.frameStoppedLoading",
            json!({ "frameId": "child" }),
            Some("session-7"),
        );
        eventually(|| signals.document_mark(&page_id).0 == mark.0 + 2).await;
        assert_eq!(
            signals.document_since(&page_id, "main", mark),
            DocumentSince::Unchanged
        );
    }

    #[tokio::test]
    async fn iframe_traffic_never_hides_a_pending_main_frame_navigation() {
        let (signals, connection, page_id) = harness();
        let mark = signals.document_mark(&page_id);
        connection.emit(
            "Page.frameStartedNavigating",
            navigation("main", "differentDocument"),
            Some("session-7"),
        );
        // A process swap moves the main frame; its navigation is still pending.
        connection.emit(
            "Page.frameDetached",
            json!({ "frameId": "main", "reason": "swap" }),
            Some("session-7"),
        );
        for index in 0..40 {
            let frame = format!("child-{index}");
            connection.emit(
                "Page.frameStartedNavigating",
                navigation(&frame, "differentDocument"),
                Some("session-7"),
            );
            connection.emit(
                "Page.frameStoppedLoading",
                json!({ "frameId": frame }),
                Some("session-7"),
            );
            connection.emit(
                "Page.frameDetached",
                json!({ "frameId": frame, "reason": "remove" }),
                Some("session-7"),
            );
        }
        eventually(|| signals.document_mark(&page_id).0 == mark.0 + 81).await;
        assert_eq!(
            signals.document_since(&page_id, "main", mark),
            DocumentSince::Loading
        );
    }

    #[tokio::test]
    async fn document_events_wake_a_waiter() {
        let (signals, connection, _page_id) = harness();
        let mut changes = signals.document_changes();
        connection.emit("Page.domContentEventFired", json!({}), Some("session-7"));
        let woke = tokio::time::timeout(Duration::from_secs(1), changes.changed()).await;
        assert!(matches!(woke, Ok(Ok(()))));
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

    #[test]
    fn an_emitted_event_reaches_its_targeted_subscriber_and_not_the_broadcast()
    -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new();
        let mut broadcast = connection.events();
        let mut targeted = connection.events_targeted("Runtime.bindingCalled");
        connection.emit(
            "Runtime.bindingCalled",
            json!({ "name": "probe" }),
            Some("session-7"),
        );
        assert_eq!(targeted.try_recv()?.params["name"], "probe");
        assert!(
            broadcast.try_recv().is_err(),
            "a method with a live targeted subscriber must bypass the broadcast"
        );
        Ok(())
    }

    fn console_error(text: &str, timestamp_ms: f64) -> Value {
        json!({
            "type": "error",
            "args": [{ "type": "string", "value": text }],
            "timestamp": timestamp_ms
        })
    }

    fn uncaught(text: &str, timestamp_ms: f64) -> Value {
        json!({
            "timestamp": timestamp_ms,
            "exceptionDetails": { "text": "Uncaught", "exception": { "description": text } }
        })
    }

    fn texts(entries: &[super::ConsoleEntry]) -> Vec<String> {
        entries.iter().map(|entry| entry.text.clone()).collect()
    }

    #[tokio::test]
    async fn harvest_turns_console_reporting_on_only_for_the_replay()
    -> Result<(), Box<dyn std::error::Error>> {
        let (signals, connection, page_id) = harness();
        connection.store_console(
            "Runtime.consoleAPICalled",
            console_error("save failed", 1.0),
        );
        connection.store_console("Runtime.exceptionThrown", uncaught("TypeError: x", 2.0));
        let session =
            ProtocolSession::for_session(connection.clone(), SessionId::from("session-7"));

        signals.harvest_console(&page_id, &session).await?;

        let runtime: Vec<_> = connection
            .sent_calls()
            .into_iter()
            .filter(|call| call.method.starts_with("Runtime."))
            .map(|call| (call.method, call.session))
            .collect();
        let on_page = Some(SessionId::from("session-7"));
        assert_eq!(
            runtime,
            vec![
                ("Runtime.enable".to_string(), on_page.clone()),
                ("Runtime.disable".to_string(), on_page),
            ]
        );
        // Recorded by the time the harvest returns, without waiting on the listener.
        assert_eq!(
            texts(&signals.console_entries(&page_id)),
            vec!["save failed", "TypeError: x"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn repeated_harvests_record_each_replayed_entry_once()
    -> Result<(), Box<dyn std::error::Error>> {
        let (signals, connection, page_id) = harness();
        let session =
            ProtocolSession::for_session(connection.clone(), SessionId::from("session-7"));
        connection.store_console("Runtime.consoleAPICalled", console_error("first", 1.0));
        connection.store_console(
            "Runtime.consoleAPICalled",
            console_error("same instant", 1.0),
        );
        signals.harvest_console(&page_id, &session).await?;

        connection.store_console("Runtime.consoleAPICalled", console_error("second", 3.0));
        signals.harvest_console(&page_id, &session).await?;
        signals.harvest_console(&page_id, &session).await?;
        // The background listener sees the same replays; give it the chance to double up.
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(
            texts(&signals.console_entries(&page_id)),
            vec!["first", "same instant", "second"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn distinct_levels_sharing_a_timestamp_are_both_kept()
    -> Result<(), Box<dyn std::error::Error>> {
        let (signals, connection, page_id) = harness();
        let session =
            ProtocolSession::for_session(connection.clone(), SessionId::from("session-7"));
        // Same text and microsecond, different level: the warning must not mask the error.
        connection.store_console(
            "Runtime.consoleAPICalled",
            json!({
                "type": "warning",
                "args": [{ "type": "string", "value": "failed" }],
                "timestamp": 1.0
            }),
        );
        connection.store_console("Runtime.consoleAPICalled", console_error("failed", 1.0));
        signals.harvest_console(&page_id, &session).await?;

        let levels: Vec<_> = signals
            .console_entries(&page_id)
            .iter()
            .map(|entry| (entry.level, entry.text.clone()))
            .collect();
        assert_eq!(
            levels,
            vec![
                (super::ConsoleLevel::Warning, "failed".to_string()),
                (super::ConsoleLevel::Error, "failed".to_string()),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_actions_summary_leaves_out_history_logged_before_it()
    -> Result<(), Box<dyn std::error::Error>> {
        let (signals, connection, page_id) = harness();
        let session =
            ProtocolSession::for_session(connection.clone(), SessionId::from("session-7"));
        let before_mark_ms = (super::now_us() / 1000) as f64 - 60_000.0;
        connection.store_console(
            "Runtime.consoleAPICalled",
            console_error("old", before_mark_ms),
        );

        let mark = signals.console_mark(&page_id);
        let after_mark_ms = (super::now_us() / 1000) as f64 + 1.0;
        connection.store_console(
            "Runtime.consoleAPICalled",
            console_error("validation failed", after_mark_ms),
        );
        signals.harvest_console(&page_id, &session).await?;

        assert_eq!(
            texts(&signals.console_entries_since(&page_id, mark)),
            vec!["validation failed"]
        );
        assert_eq!(
            texts(&signals.console_entries(&page_id)),
            vec!["old", "validation failed"]
        );
        Ok(())
    }
}
