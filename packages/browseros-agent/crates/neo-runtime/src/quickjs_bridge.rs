//! `__neo_runtime`, the only door from a run's JS into Rust. neo-quickjs-injected's
//! `runtime-bridge.ts` calls `get` for every synchronous Playwright getter and `call` for every
//! browser call, and the realm's `console` calls `log`.
//!
//! ```text
//!  page.url()                                    JS, on the run's thread
//!    └─ __neo_runtime.get('{"guid":"page@<id>","field":"url"}')
//!         └─ Getters::get: parse → (object, field) → getters::url   ← blocks this thread
//!              └─ CDP request ──▶ BrowserOS ──▶ reply, read by browseros-cdp's reader task
//!         ◀─ '"https://…"'                       JSON text of the value
//!
//!  locator.click()                               JS, on the run's thread
//!    └─ __neo_runtime.callWithEvents('{"id":1,"guid":"frame@<id>","method":"click",…}')
//!         └─ Dispatcher::call: starts the call on its own tokio task, returns at once
//!         ◀─ Promise of {reply: '{"id":1}' | '{"id":1,"error":{…},"log":[…]}', events: W}
//!    (`call` is the same without `events`: the reply text alone, for white-box tests)
//!
//!  controller.abort()                            JS, a call's `abort` listener (connection.ts)
//!    └─ __neo_runtime.abort(1, 'reason')         sync: aborts message 1's call (`abort.rs`)
//!
//!  page.on('console', f)                         JS, when a listener count leaves 0
//!    └─ __neo_runtime.subscribe('{"guid":"page@<id>","event":"console","enabled":true}')
//!  runtime-bridge.ts, before a reply's caller continues, and on its idle wake
//!    └─ __neo_runtime.drain(W) ◀─ '[{"seq":1,"guid":"context@default","method":"console",…}]'
//!    └─ __neo_runtime.nextEvents() ◀─ Promise that resolves once an event is queued
//! ```
//!
//! A call's promise only waits for the reply its task sends; this file does no CDP for it. So a
//! call keeps going while the script spins or a getter blocks, and dropping the realm drops only
//! the wait, never the call. The reply carries the event watermark W (`events.rs`): the SDK runs
//! the listeners of every event up to W before the call's caller continues, Playwright's order.
//!
//! A run with hooks has a second realm (`hooks.rs`) with its own `__neo_runtime` of the same
//! shape ([`install_hooks`]), every closure bound to that realm's state: its own getters, its
//! `log` writing the hook trace, its own dispatcher (origin hook, own abort registry), and no
//! events: `subscribe` does nothing, `drain` is always empty, `nextEvents` never resolves. It never
//! touches the agent's `EventHub` or `Logs`. The realm boundary is the trust boundary: each
//! bridge holds exactly one dispatcher, so which bridge a call came through, not anything in its
//! message, decides whose call it is.

use std::future;
use std::future::Future;
use std::mem;
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rand::RngCore;
use rquickjs::function::{Async, Func};
use rquickjs::{Ctx, Exception, Object};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::task;

use crate::browser_representations::network::{HopId, LogError, ObjectKind as NetworkObject};
use crate::browser_representations::tabs::Tabs;
use crate::chromium_bridge::page_events;
use crate::deadline::{Deadline, Stop};
use crate::debugger;
use crate::events::{self, CONTEXT_GUID};
use crate::method_router::Dispatcher;
use crate::playwright_methods::browser_methods::{self, BROWSER_GUID};
use crate::playwright_methods::evaluate::Handles;
use crate::playwright_methods::getters::{self, FrameField};
use crate::progress::PlaywrightError;
use crate::run::{PageId, RunError};
use crate::url_match::{self, SerializedUrlMatch, UrlPart};

/// The guid the realm gives a run's page, in the `<type>@<id>` form Playwright's protocol uses.
pub(crate) fn page_guid(page: &PageId) -> String {
    format!("page@{}", page.as_str())
}

/// The guid the realm gives a run's main frame. Chromium gives a page's main frame its target's
/// id (measured in `tests/sync_getters.rs`), so the frame is `frame@<targetId>`.
pub(crate) fn frame_guid(page: &PageId) -> String {
    format!("frame@{}", page.as_str())
}

/// The guid the realm gives the frame `frame` of a run's page: its main frame's, or
/// `frame@<targetId>/<frameId>` for a child (`getters.rs` names frames so).
pub(crate) fn frame_guid_of(page: &PageId, frame: &str) -> String {
    if frame == page.as_str() {
        frame_guid(page)
    } else {
        format!("frame@{}/{frame}", page.as_str())
    }
}

/// Installs `__neo_runtime` as a global of the agent's realm `ctx`.
pub(crate) fn install<'js>(
    ctx: &Ctx<'js>,
    getters: Getters,
    logs: Logs,
    dispatcher: Dispatcher,
) -> rquickjs::Result<()> {
    let tabs = getters.tabs.clone();
    let bridge = Object::new(ctx.clone())?;
    install_calls(
        &bridge,
        getters,
        logs.clone(),
        dispatcher,
        Watermark::Events,
    )?;
    let subscriptions = tabs.clone();
    bridge.set(
        "subscribe",
        Func::from(move |ctx: Ctx<'js>, request: String| {
            subscribe(&subscriptions, &request)
                .map_err(|message| throw_named(&ctx, "Error", &message))
        }),
    )?;
    let hub = tabs.events().clone();
    bridge.set(
        "drain",
        // Synchronous: the SDK runs the drained events' listeners in the same JS job, before the
        // reply that asked for them reaches its caller. `until` < 0 takes every queued event.
        Func::from(move |until: i64| {
            let text = hub.drain(u64::try_from(until).ok());
            for note in hub.take_notes() {
                logs.push(note);
            }
            text
        }),
    )?;
    let hub = tabs.events().clone();
    bridge.set(
        "nextEvents",
        // The idle wake: the SDK keeps one pending while it listens to anything. It resolves once
        // an undelivered event exists and never keeps the run alive (the realm ends with the
        // script, dropping it).
        Func::from(Async(move || {
            let hub = hub.clone();
            async move {
                hub.next().await;
                Ok::<(), rquickjs::Error>(())
            }
        })),
    )?;
    ctx.globals().set("__neo_runtime", bridge)
}

/// Installs `__neo_runtime` as a global of a run's hook realm `ctx` (`hooks.rs`): the agent's
/// shape, with `getters` of its own, `log` appending to the hook `trace`, and `dispatcher` the
/// run's hook dispatcher (`Dispatcher::for_hooks`). Hook listeners never fire (a documented
/// limit): `subscribe` does nothing, `drain` answers `[]` and `nextEvents` never resolves, so the
/// hook realm never takes, delays or reorders an event of the agent's.
pub(crate) fn install_hooks(
    ctx: &Ctx<'_>,
    getters: Getters,
    trace: Logs,
    dispatcher: Dispatcher,
) -> rquickjs::Result<()> {
    let bridge = Object::new(ctx.clone())?;
    install_calls(&bridge, getters, trace, dispatcher, Watermark::None)?;
    bridge.set("subscribe", Func::from(|_request: String| {}))?;
    bridge.set("drain", Func::from(|_until: i64| "[]".to_string()))?;
    bridge.set(
        "nextEvents",
        // Never resolves; dropped with the hook realm.
        Func::from(Async(future::pending::<rquickjs::Result<()>>)),
    )?;
    ctx.globals().set("__neo_runtime", bridge)
}

/// Whether a call's answer carries the run's event watermark (the agent's realm) or always 0
/// (the hook realm, which delivers no events).
#[derive(Clone, Copy)]
enum Watermark {
    Events,
    None,
}

/// The members both realms' bridges share: `get` on `getters`, `log` into `logs`, and `abort`,
/// `call` and `callWithEvents` on `dispatcher`, the one dispatcher this bridge ever holds.
fn install_calls<'js>(
    bridge: &Object<'js>,
    getters: Getters,
    logs: Logs,
    dispatcher: Dispatcher,
    watermark: Watermark,
) -> rquickjs::Result<()> {
    bridge.set(
        "get",
        Func::from(move |ctx: Ctx<'js>, request: String| getters.get(&ctx, &request)),
    )?;
    bridge.set("log", Func::from(move |line: String| logs.push(line)))?;
    let aborts = dispatcher.clone();
    bridge.set(
        "abort",
        // Synchronous, inside the signal's `abort` listener: the call stops at its next race or
        // check and replies `AbortError`. An id that is not a call in flight is ignored.
        Func::from(move |id: i64, reason: String| {
            if let Ok(id) = u64::try_from(id) {
                aborts.abort(id, reason);
            }
        }),
    )?;
    let calls = dispatcher.clone();
    bridge.set(
        "call",
        // The reply text alone: what the realm's tests drive Rust with directly. The SDK uses
        // `callWithEvents`, so its listeners run before the reply's caller continues.
        Func::from(Async(move |ctx: Ctx<'js>, message: String| {
            let reply = calls.call(&message);
            async move {
                reply
                    .await
                    .map(|(reply, _)| reply)
                    .map_err(|error| throw_named(&ctx, "Error", &error.to_string()))
            }
        })),
    )?;
    bridge.set(
        "callWithEvents",
        // rquickjs makes the promise now and polls this future only while the script awaits.
        // That is enough because the future only waits for the reply: `Dispatcher::call` has
        // already started the call on a tokio task. An `Err` rejects the promise with a named
        // `Error`; only an unreadable envelope (an SDK bug) or a run ending under the call does.
        Func::from(Async(move |ctx: Ctx<'js>, message: String| {
            let reply = dispatcher.call(&message);
            async move {
                let (reply, events) = reply
                    .await
                    .map_err(|error| throw_named(&ctx, "Error", &error.to_string()))?;
                let events = match watermark {
                    Watermark::Events => events,
                    Watermark::None => 0,
                };
                let answer = Object::new(ctx.clone())?;
                answer.set("reply", reply)?;
                answer.set("events", i64::try_from(events).unwrap_or(i64::MAX))?;
                Ok::<_, rquickjs::Error>(answer)
            }
        })),
    )?;
    Ok(())
}

/// `__neo_runtime.subscribe`: the script's listening to `event` on a page or the context turned
/// on or off. On, it switches on what the event's producer needs in each page it covers, and
/// that page's next call waits for it (`EventHub::ready`); off, a page whose console nobody
/// listens to any more frees the objects its console events hold.
fn subscribe(tabs: &Tabs, request: &str) -> Result<(), String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Subscription {
        guid: String,
        event: String,
        enabled: bool,
    }
    let Subscription {
        guid,
        event,
        enabled,
    } = serde_json::from_str(request).map_err(|error| format!("bad subscribe request: {error}"))?;
    let needs = events::needs(&guid, &event)
        .ok_or_else(|| format!("Neo does not produce {event:?} events on {guid:?}"))?;
    // A closed context listens to nothing more, and must not attach to its still-open tabs.
    if tabs.events().subscriptions_ended() {
        return Ok(());
    }
    let pages = if guid.starts_with(debugger::DEBUGGER_PREFIX) {
        // A debugger's event comes from no page.
        Vec::new()
    } else if guid == CONTEXT_GUID {
        // The run's own context's pages: a created context's page is not one of them.
        tabs.attached()
            .into_iter()
            .map(|(entry, _)| entry)
            .filter(|entry| entry.context.get().is_none())
            .collect()
    } else if let Some(context) = tabs.created().find(&guid) {
        // (platform) A context the run created: its own pages, and none of the run's.
        tabs.attached()
            .into_iter()
            .map(|(entry, _)| entry)
            .filter(|entry| {
                entry
                    .context
                    .get()
                    .is_some_and(|owner| Arc::ptr_eq(owner, &context))
            })
            .collect()
    } else {
        let (entry, _) = tabs
            .resolve(&guid)
            .ok_or_else(|| format!("unknown guid {guid:?}"))?;
        vec![entry]
    };
    tabs.events().subscribe(&guid, &event, enabled);
    if enabled && needs.discovery {
        tabs.discover();
    }
    if enabled && needs.downloads {
        tabs.watch_downloads();
    }
    if !needs.of_page() {
        return Ok(());
    }
    if event == "filechooser" {
        // A page's own event: `pages` is that page alone.
        for entry in pages {
            page_events::intercept_file_chooser(tabs, entry);
        }
        return Ok(());
    }
    for entry in pages {
        if enabled {
            page_events::enable(tabs, entry, needs);
        } else if needs.runtime
            && let Some(session) = entry.attached_session()
            && session.events.console_unwanted()
        {
            session.events.release_console_objects();
        }
    }
    Ok(())
}

/// Platform: Web Crypto's `getRandomValues` throws `QuotaExceededError` for an array of more than
/// 65,536 bytes (W3C Web Cryptography API, `Crypto.getRandomValues`), and Node's follows it; one
/// `randomValues` getter never serves more.
const MAX_RANDOM_BYTES: usize = 65_536;

/// The most `console` text and lines a run keeps. Kept lines live in Rust, outside the realm's
/// memory limit, so without a cap one script could hold any amount of the server's memory. A
/// run's hook trace and its hooks' notes keep as much, for the same reason.
const MAX_LOG_BYTES: usize = 1024 * 1024;
const MAX_LOG_LINES: usize = 1000;

/// Neo: the most lines of its own a run's hook trace keeps (why a hook failed), apart from the
/// hook's own lines, so a hook that fills its trace cannot hide them.
const MAX_FAILURE_LINES: usize = 100;
/// Neo: the most bytes of those lines.
const MAX_FAILURE_BYTES: usize = 64 * 1024;

/// Lines the script wrote with `console`. Shared because the realm owns the JS `log` function
/// while the run reads the lines back, also after a deadline dropped the realm mid-script. Also
/// a run's hook trace, Neo's own lines on why a hook failed, and the hooks' notes, each a
/// [`LogKind`] of its own with its own caps and overflow note (`hooks.rs`).
///
/// Keeps the first lines up to its kind's line and byte caps ([`MAX_LOG_LINES`] and
/// [`MAX_LOG_BYTES`] for the script's `console`). The first line past either cap ends keeping: it
/// and every later line are only counted, so the kept lines are always the start of what was
/// written, and [`Logs::take`] ends them with a note saying how many were dropped. Logging itself
/// never fails, so the cap never throws into the script.
#[derive(Debug, Clone)]
pub(crate) struct Logs(Arc<Mutex<LogLines>>);

/// What a [`Logs`] holds, which sets its caps and names it in its overflow note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogKind {
    /// The script's `console` lines.
    Console,
    /// A run's hook trace: the hook realm's `ctx.trace` and `console` lines.
    HookTrace,
    /// Neo's own lines in the hook trace, on why a hook failed.
    HookFailures,
    /// The notes a run's hooks left for the model.
    HookNotes,
}

#[derive(Debug)]
struct LogLines {
    kind: LogKind,
    kept: Vec<String>,
    kept_bytes: usize,
    /// Set by the first line that did not fit.
    overflow: Option<Overflow>,
}

#[derive(Debug)]
struct Overflow {
    limit: LogLimit,
    dropped: usize,
}

#[derive(Debug, Clone, Copy)]
enum LogLimit {
    Lines,
    Bytes,
}

impl Default for Logs {
    /// The script's `console` lines.
    fn default() -> Self {
        Self::of(LogKind::Console)
    }
}

impl Logs {
    /// An empty list of `kind`.
    pub(crate) fn of(kind: LogKind) -> Self {
        Self(Arc::new(Mutex::new(LogLines::new(kind))))
    }

    pub(crate) fn push(&self, line: String) {
        if let Ok(mut lines) = self.0.lock() {
            lines.push(line);
        }
    }

    pub(crate) fn take(&self) -> Vec<String> {
        let Ok(mut lines) = self.0.lock() else {
            return Vec::new();
        };
        let kind = lines.kind;
        let LogLines {
            mut kept, overflow, ..
        } = mem::replace(&mut *lines, LogLines::new(kind));
        kept.extend(overflow.map(|overflow| overflow.note(kind)));
        kept
    }
}

impl LogKind {
    /// The most lines and bytes a list of this kind keeps.
    fn caps(self) -> (usize, usize) {
        match self {
            Self::Console | Self::HookTrace | Self::HookNotes => (MAX_LOG_LINES, MAX_LOG_BYTES),
            Self::HookFailures => (MAX_FAILURE_LINES, MAX_FAILURE_BYTES),
        }
    }

    /// What the overflow note calls one of its lines.
    fn noun(self) -> &'static str {
        match self {
            Self::Console => "console",
            Self::HookTrace => "hook trace",
            Self::HookFailures => "hook failure",
            Self::HookNotes => "hook note",
        }
    }
}

impl LogLines {
    fn new(kind: LogKind) -> Self {
        Self {
            kind,
            kept: Vec::new(),
            kept_bytes: 0,
            overflow: None,
        }
    }

    fn push(&mut self, line: String) {
        if let Some(overflow) = &mut self.overflow {
            overflow.dropped += 1;
            return;
        }
        if let Some(limit) = self.limit_passed_by(&line) {
            self.overflow = Some(Overflow { limit, dropped: 1 });
        } else {
            self.kept_bytes += line.len();
            self.kept.push(line);
        }
    }

    fn limit_passed_by(&self, line: &str) -> Option<LogLimit> {
        let (max_lines, max_bytes) = self.kind.caps();
        if self.kept.len() >= max_lines {
            Some(LogLimit::Lines)
        } else if self.kept_bytes + line.len() > max_bytes {
            Some(LogLimit::Bytes)
        } else {
            None
        }
    }
}

impl Overflow {
    fn note(&self, kind: LogKind) -> String {
        let lines = if self.dropped == 1 { "line" } else { "lines" };
        let (max_lines, max_bytes) = kind.caps();
        let noun = kind.noun();
        let limit = match self.limit {
            LogLimit::Lines => format!("{max_lines} lines"),
            LogLimit::Bytes if max_bytes >= 1 << 20 => {
                format!("{} MiB of {noun} text", max_bytes >> 20)
            }
            LogLimit::Bytes => format!("{} KiB of {noun} text", max_bytes >> 10),
        };
        format!(
            "neo-runtime dropped the last {} {noun} {lines}: a run keeps at most {limit}",
            self.dropped
        )
    }
}

/// Everything `__neo_runtime.get` needs to answer for one run.
pub(crate) struct Getters {
    tabs: Tabs,
    deadline: Deadline,
    tokio: Handle,
    /// The run's handles, which `ConsoleMessage.args()` adds its message's arguments to.
    handles: Arc<Handles>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum GetterField {
    Url,
    IsClosed,
    Pages,
    Initializer,
    /// On a request or response: its hop and every hop before, for the SDK to build.
    Chain,
    /// On a request: the guid of the request a redirect made of it, or `null`.
    RedirectedTo,
    /// On a request: the guid of its response once it has one, or `null`.
    ExistingResponse,
    /// On a request: `timing()`, which Playwright's client updates from events.
    Timing,
    /// On a request: `failure()`'s text, set by Playwright's `requestFailed` event.
    Failure,
    /// On a response: `"finished"`, `"pending"` or `"closed"`, which `finished()` waits on.
    Finished,
    Frames,
    Name,
    ParentFrame,
    ChildFrames,
    IsDetached,
    /// On the browser: `version()`, as Playwright reports it.
    Version,
    /// On the browser: whether the run's CDP connection to it is up.
    IsConnected,
    /// On a debugger: the call it holds, `null` while none.
    PausedDetails,
    /// On a context: the guid of its debugger.
    Debugger,
    /// On a page: `opener()`'s record, `null` when none is open in the run's context.
    Opener,
    /// On a console message (`console@<targetId>:<n>`): `args()`'s handle records.
    Args,
    /// On a file chooser (`filechooser@<targetId>:<n>`): `element()`'s handle record.
    Element,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectRequest {
    guid: String,
    field: GetterField,
}

#[derive(Deserialize)]
enum MatchField {
    #[serde(rename = "matchesURL")]
    MatchesUrl,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MatchRequest {
    #[serde(rename = "field")]
    _field: MatchField,
    url: String,
    matcher: SerializedUrlMatch,
}

#[derive(Deserialize)]
enum DevicesField {
    #[serde(rename = "devices")]
    Devices,
}

/// `playwright.devices`: Playwright's device descriptors, the same for every run.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DevicesRequest {
    #[serde(rename = "field")]
    _field: DevicesField,
}

#[derive(Deserialize)]
enum ParseField {
    #[serde(rename = "parseURL")]
    ParseUrl,
}

/// The realm's `new URL(url, base)` (and a URL predicate's argument): its parts, or `null` where
/// it would throw.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParseRequest {
    #[serde(rename = "field")]
    _field: ParseField,
    url: String,
    #[serde(default)]
    base: Option<String>,
}

#[derive(Deserialize)]
enum SetUrlField {
    #[serde(rename = "setURL")]
    SetUrl,
}

/// One setter of the realm's `URL`: the parts of `href` once `part` is set to `value`, or `null`
/// where the setter throws.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetUrlRequest {
    #[serde(rename = "field")]
    _field: SetUrlField,
    href: String,
    part: UrlPart,
    value: String,
}

#[derive(Deserialize)]
enum RandomField {
    #[serde(rename = "randomValues")]
    RandomValues,
}

/// The realm's `crypto`: `length` random bytes, as base64.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RandomRequest {
    #[serde(rename = "field")]
    _field: RandomField,
    length: usize,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum GetRequest {
    Object(ObjectRequest),
    Match(MatchRequest),
    Pure(PureRead),
}

/// A getter that reads no browser state, answered on the spot: platform pieces QuickJS lacks.
#[derive(Deserialize)]
#[serde(untagged)]
enum PureRead {
    Devices(DevicesRequest),
    Parse(ParseRequest),
    SetUrl(SetUrlRequest),
    Random(RandomRequest),
}

impl PureRead {
    fn answer(self) -> Result<Value, GetError> {
        match self {
            Self::Devices(_) => {
                serde_json::from_str(browser_methods::DEVICE_DESCRIPTORS).map_err(|error| {
                    GetError::Page(PlaywrightError::error(format!(
                        "neo-runtime: the device descriptors are unreadable: {error}"
                    )))
                })
            }
            Self::Parse(request) => {
                let parsed = url_match::parse_url_against(&request.url, request.base.as_deref());
                serde_json::to_value(parsed)
                    .map_err(|error| GetError::BadRequest(format!("encoding a URL: {error}")))
            }
            Self::SetUrl(request) => {
                let set = url_match::set_url_part(&request.href, request.part, &request.value);
                serde_json::to_value(set)
                    .map_err(|error| GetError::BadRequest(format!("encoding a URL: {error}")))
            }
            Self::Random(request) => random_values(request.length),
        }
    }
}

/// Why a getter threw. Each maps to the JS error the script sees.
#[derive(Debug)]
enum GetError {
    BadRequest(String),
    Page(PlaywrightError),
    Stopped(Stop),
}

impl Getters {
    pub(crate) fn new(
        tabs: Tabs,
        deadline: Deadline,
        tokio: Handle,
        handles: Arc<Handles>,
    ) -> Self {
        Self {
            tabs,
            deadline,
            tokio,
            handles,
        }
    }

    fn get(&self, ctx: &Ctx<'_>, request: &str) -> rquickjs::Result<String> {
        match self.answer(request) {
            Ok(value) => Ok(value.to_string()),
            Err(error) => Err(self.throw(ctx, error)),
        }
    }

    fn answer(&self, request: &str) -> Result<Value, GetError> {
        let request: GetRequest = serde_json::from_str(request)
            .map_err(|error| GetError::BadRequest(format!("bad getter request: {error}")))?;
        let request = match request {
            GetRequest::Object(request) => request,
            GetRequest::Match(request) => {
                return self.wait(async {
                    getters::matches_url(&request.url, &request.matcher).map(|value| json!(value))
                });
            }
            GetRequest::Pure(read) => return read.answer(),
        };
        if request.guid == "context@default" && matches!(request.field, GetterField::Pages) {
            return self.wait(async {
                serde_json::to_value(self.tabs.pages().await?)
                    .map_err(|error| PlaywrightError::error(error.to_string()))
            });
        }
        if let Some(answer) = self.platform(&request) {
            return answer;
        }
        if let Some(answer) = self.handed_out(&request) {
            return answer;
        }
        if let Some((kind, _, hop)) = HopId::parse(&request.guid) {
            return self.network(&request.guid, kind, &hop, request.field);
        }
        let (page, scope) = self
            .tabs
            .resolve(&request.guid)
            .ok_or_else(|| GetError::BadRequest(format!("unknown guid {:?}", request.guid)))?;
        if request.guid.starts_with("frame@") {
            let field = match request.field {
                GetterField::Url => FrameField::Url,
                GetterField::Name => FrameField::Name,
                GetterField::ParentFrame => FrameField::ParentFrame,
                GetterField::ChildFrames => FrameField::ChildFrames,
                GetterField::IsDetached => FrameField::IsDetached,
                _ => return Err(GetError::BadRequest("not a frame getter".to_string())),
            };
            return self.wait(async { getters::frame(&self.tabs, &page, &scope, field).await });
        }
        if !request.guid.starts_with("page@") {
            return Err(GetError::BadRequest(format!(
                "unknown guid {:?}",
                request.guid
            )));
        }
        match request.field {
            GetterField::Url => self.wait(self.tabs.url(&page)).map(|url| json!(url)),
            GetterField::IsClosed => self
                .wait(self.tabs.is_closed(&page))
                .map(|closed| json!(closed)),
            GetterField::Initializer => self.wait(async {
                serde_json::to_value(self.tabs.initializer(&page).await?)
                    .map_err(|error| PlaywrightError::error(error.to_string()))
            }),
            GetterField::Opener => self.wait(async {
                serde_json::to_value(self.tabs.opener(&page).await?)
                    .map_err(|error| PlaywrightError::error(error.to_string()))
            }),
            GetterField::Pages => Err(GetError::BadRequest(
                "pages is a context getter".to_string(),
            )),
            GetterField::Chain
            | GetterField::RedirectedTo
            | GetterField::ExistingResponse
            | GetterField::Timing
            | GetterField::Failure
            | GetterField::Finished => Err(GetError::BadRequest(format!(
                "{:?} is a request or response getter",
                request.field
            ))),
            GetterField::Frames => self.wait(async { getters::frames(&self.tabs, &page).await }),
            GetterField::Name
            | GetterField::ParentFrame
            | GetterField::ChildFrames
            | GetterField::IsDetached
            | GetterField::Version
            | GetterField::IsConnected
            | GetterField::PausedDetails
            | GetterField::Debugger
            | GetterField::Args
            | GetterField::Element => Err(GetError::BadRequest("not a page getter".to_string())),
        }
    }

    /// The getters of the run's browser, its debuggers and the contexts it created
    /// (`browser_methods.rs`, `debugger.rs`), or `None` for any other object.
    fn platform(&self, request: &ObjectRequest) -> Option<Result<Value, GetError>> {
        if let Some(context) = request.guid.strip_prefix(debugger::DEBUGGER_PREFIX) {
            return Some(match request.field {
                GetterField::PausedDetails => Ok(self.tabs.debuggers().paused_details(context)),
                _ => Err(GetError::BadRequest("not a debugger getter".to_string())),
            });
        }
        if matches!(request.field, GetterField::Debugger) {
            let context = if request.guid == "context@default" {
                Some(debugger::DEFAULT_CONTEXT.to_string())
            } else {
                self.tabs
                    .created()
                    .find(&request.guid)
                    .map(|context| context.id.clone())
            };
            return Some(
                context
                    .map(|context| json!(debugger::guid(&context)))
                    .ok_or_else(|| {
                        GetError::BadRequest(format!("unknown guid {:?}", request.guid))
                    }),
            );
        }
        if request.guid == BROWSER_GUID {
            return Some(match request.field {
                GetterField::Version => self.wait(browser_methods::version(&self.tabs)),
                GetterField::IsConnected => Ok(json!(self.tabs.cdp().is_connected())),
                _ => Err(GetError::BadRequest("not a browser getter".to_string())),
            });
        }
        let context = self.tabs.created().find(&request.guid)?;
        if !matches!(request.field, GetterField::Pages) {
            return Some(Err(GetError::BadRequest(
                "not a context getter".to_string(),
            )));
        }
        Some(self.wait(async {
            serde_json::to_value(browser_methods::context_pages(&self.tabs, &context).await?)
                .map_err(|error| PlaywrightError::error(error.to_string()))
        }))
    }

    /// What a run's event handed out by id, made at its first read: a console message's `args`
    /// (`console@<targetId>:<n>`) or a file chooser's `element` (`filechooser@<targetId>:<n>`).
    /// `None` for any other guid.
    fn handed_out(&self, request: &ObjectRequest) -> Option<Result<Value, GetError>> {
        let (kind, rest) = request.guid.split_once('@')?;
        let wanted = match (kind, request.field) {
            ("console", GetterField::Args) | ("filechooser", GetterField::Element) => true,
            ("console" | "filechooser", _) => false,
            _ => return None,
        };
        if !wanted {
            return Some(Err(GetError::BadRequest(format!(
                "no {:?} on {:?}",
                request.field, request.guid
            ))));
        }
        let Some((target, id)) = rest.rsplit_once(':').and_then(|(target, id)| {
            Some((PageId::new(target.to_string()), id.parse::<u64>().ok()?))
        }) else {
            return Some(Err(GetError::BadRequest(format!(
                "unknown guid {:?}",
                request.guid
            ))));
        };
        Some(self.wait(async {
            if kind == "console" {
                self.tabs.console_args(&target, id, &self.handles).await
            } else {
                self.tabs
                    .file_chooser_element(&target, id, &self.handles)
                    .await
            }
        }))
    }

    /// A hop's live client state, read from the network log of the session that recorded it.
    /// Nothing here waits: every value is already in Rust.
    fn network(
        &self,
        guid: &str,
        kind: NetworkObject,
        hop: &HopId,
        field: GetterField,
    ) -> Result<Value, GetError> {
        let unknown = || GetError::BadRequest(format!("unknown guid {guid:?}"));
        let (page, _) = self.tabs.resolve(guid).ok_or_else(unknown)?;
        // A hop exists only after a goto on this session, which stays attached for the run.
        let session = page.attached_session().ok_or_else(unknown)?;
        let log = &session.network;
        let target = page.id.as_str();
        let value = match (kind, field) {
            (_, GetterField::Chain) => log
                .chain(hop)
                .and_then(|chain| serde_json::to_value(chain).map_err(|_| LogError::Poisoned)),
            (NetworkObject::Request, GetterField::RedirectedTo) => log
                .redirected_to(hop)
                .map(|next| json!(next.map(|next| next.request_guid(target)))),
            (NetworkObject::Request, GetterField::ExistingResponse) => log
                .has_response(hop)
                .map(|has| json!(has.then(|| hop.response_guid(target)))),
            (NetworkObject::Request, GetterField::Timing) => log
                .timing(hop)
                .and_then(|timing| serde_json::to_value(timing).map_err(|_| LogError::Poisoned)),
            (NetworkObject::Request, GetterField::Failure) => {
                log.failure(hop).map(|text| json!(text))
            }
            (NetworkObject::Response, GetterField::Finished) => log.finished(hop).map(|finished| {
                json!(if finished {
                    "finished"
                } else if page.closed.is_cancelled() {
                    "closed"
                } else {
                    "pending"
                })
            }),
            (_, field) => return Err(GetError::BadRequest(format!("no {field:?} on {guid:?}"))),
        };
        value.map_err(|error| GetError::Page(PlaywrightError::error(error.to_string())))
    }

    /// Waits on the current thread for one CDP reply, so a JS getter can return a plain
    /// value. This is safe because a run's JS is single-threaded: while the getter waits the
    /// script has nothing else it could do. `block_in_place` hands this tokio worker's other
    /// tasks to another thread first, so other runs and browseros-cdp's reader task, which
    /// delivers the reply, keep going. That needs tokio's multi-threaded runtime, which
    /// `Runtime::run` checks before a realm exists. The wait ends early at the run's deadline
    /// or on cancellation.
    fn wait<T>(
        &self,
        read: impl Future<Output = Result<T, PlaywrightError>>,
    ) -> Result<T, GetError> {
        task::block_in_place(|| {
            self.tokio.block_on(async {
                tokio::select! {
                    stop = self.deadline.reached() => Err(GetError::Stopped(stop)),
                    result = read => result.map_err(GetError::Page),
                }
            })
        })
    }

    fn throw(&self, ctx: &Ctx<'_>, error: GetError) -> rquickjs::Error {
        let error = match error {
            GetError::BadRequest(message) => RunError::new("Error", message),
            GetError::Page(error) => RunError::new(error.name.as_str(), error.message),
            GetError::Stopped(Stop::Timeout) => RunError::timeout(self.deadline.timeout()),
            GetError::Stopped(Stop::Cancelled) => {
                RunError::new("Error", "Run cancelled".to_string())
            }
        };
        throw_named(ctx, &error.name, &error.message)
    }
}

/// `length` bytes from the thread's CSPRNG (rand's `ThreadRng`: `ChaCha`, seeded and reseeded from
/// the OS), as standard base64, for the realm's `crypto.getRandomValues` and `randomUUID`.
/// neo-quickjs-injected refuses a longer request with the Web Crypto `QuotaExceededError` before it
/// asks.
fn random_values(length: usize) -> Result<Value, GetError> {
    if length > MAX_RANDOM_BYTES {
        return Err(GetError::BadRequest(format!(
            "neo-runtime: {length} random bytes asked for, at most {MAX_RANDOM_BYTES} served"
        )));
    }
    let mut bytes = vec![0; length];
    rand::rng().fill_bytes(&mut bytes);
    Ok(json!(STANDARD.encode(bytes)))
}

/// Throws a JS `Error` whose `name` is `name`, the way Playwright's own error classes read.
fn throw_named(ctx: &Ctx<'_>, name: &str, message: &str) -> rquickjs::Error {
    let exception = match Exception::from_message(ctx.clone(), message) {
        Ok(exception) => exception,
        Err(error) => return error,
    };
    if let Err(error) = exception.as_object().set("name", name) {
        return error;
    }
    exception.throw()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde_json::Value;

    use super::{GetRequest, MAX_RANDOM_BYTES, PureRead, random_values};
    use crate::playwright_methods::getters;
    use crate::url_match::UrlPart;

    #[test]
    fn tests_that_matches_url_uses_the_wait_for_url_wire_shape_without_a_guid() {
        for wire in [
            r#"{"field":"matchesURL","url":"https://a.test/","matcher":{"glob":"https://A.test/"}}"#,
            r#"{"field":"matchesURL","url":"https://a.test/","matcher":{"regexSource":"A\\.test","regexFlags":"i"}}"#,
        ] {
            let GetRequest::Match(request) = serde_json::from_str(wire).unwrap() else {
                panic!("expected the pure URL matcher getter");
            };
            assert!(getters::matches_url(&request.url, &request.matcher).unwrap());
        }
    }

    #[test]
    fn tests_that_the_url_getters_read_their_wire_shapes() {
        let wire = r#"{"field":"parseURL","url":"/x","base":"https://h.test/p/"}"#;
        let GetRequest::Pure(PureRead::Parse(request)) = serde_json::from_str(wire).unwrap() else {
            panic!("expected the URL parse getter");
        };
        assert_eq!(
            (request.url.as_str(), request.base.as_deref()),
            ("/x", Some("https://h.test/p/"))
        );
        let GetRequest::Pure(PureRead::Parse(request)) =
            serde_json::from_str(r#"{"field":"parseURL","url":"https://h.test/"}"#).unwrap()
        else {
            panic!("expected the URL parse getter without a base");
        };
        assert_eq!(request.base, None);
        let wire = r#"{"field":"setURL","href":"https://h.test/","part":"pathname","value":"/a"}"#;
        let GetRequest::Pure(PureRead::SetUrl(request)) = serde_json::from_str(wire).unwrap()
        else {
            panic!("expected the URL setter getter");
        };
        assert_eq!(request.part, UrlPart::Pathname);
        assert!(
            serde_json::from_str::<GetRequest>(
                r#"{"field":"setURL","href":"https://h.test/","part":"origin","value":"x"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn tests_that_random_values_are_fresh_and_bounded() {
        let draw = |length| match random_values(length) {
            Ok(Value::String(text)) => STANDARD.decode(text).unwrap(),
            other => panic!("expected base64 text, got {other:?}"),
        };
        assert_eq!(draw(0).len(), 0);
        assert_eq!(draw(MAX_RANDOM_BYTES).len(), MAX_RANDOM_BYTES);
        // 2^-256 odds of a false failure.
        assert_ne!(draw(32), draw(32));
        assert!(random_values(MAX_RANDOM_BYTES + 1).is_err());
        let wire = r#"{"field":"randomValues","length":16}"#;
        assert!(matches!(
            serde_json::from_str(wire).unwrap(),
            GetRequest::Pure(PureRead::Random(request)) if request.length == 16
        ));
    }

    #[test]
    fn tests_that_frame_getter_requests_preserve_the_opaque_guid() {
        for field in ["url", "name", "parentFrame", "childFrames", "isDetached"] {
            let wire = json!({"guid": "frame@tab/child", "field": field});
            let GetRequest::Object(request) = serde_json::from_value(wire).unwrap() else {
                panic!("expected a frame getter");
            };
            assert_eq!(request.guid, "frame@tab/child");
        }
        assert!(serde_json::from_str::<GetRequest>(r#"{"field":"matchesURL","url":"https://a.test/","matcher":{"glob":"**"},"guid":"frame@tab"}"#).is_err());
    }
}
