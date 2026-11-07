//! Playwright: `server/network.ts` `Route`
//! Playwright: `server/chromium/crNetworkManager.ts` `RouteImpl`
//! Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._onRequestPaused`
//! Playwright: `server/dispatchers/pageDispatcher.ts` `PageDispatcher`
//!
//! One tab's routed requests for one run. While the run has a route on the page or on its
//! context (`setNetworkInterceptionPatterns`), every session of the tab pauses every request
//! (`Fetch.enable` for `*`, as Playwright's server does, which matches URLs itself). The tab's
//! watcher hands each `Fetch.requestPaused` here: a request no pattern names goes on at once;
//! one a pattern names gets a guid (`route@<targetId>:<n>`) and becomes the run's `route` event,
//! on the page when a page pattern names it, else on the context, after its `request` event.
//! The script's `Route.abort`, `Route.continue` and `Route.fulfill` answer it once, with
//! `Fetch.failRequest`, `Fetch.continueRequest` or `Fetch.fulfillRequest` on the session that
//! paused it.
//!
//! ```text
//!  Fetch.requestPaused ─▶ Routes::paused ─┬─ no pattern, favicon, preflight, redirect ─▶ answered here
//!                                         ├─ before its requestWillBeSent ─▶ parked by networkId
//!                                         └─ named ─▶ `route` event ─▶ handlers ─▶ Routes::serve
//! ```
//!
//! A pause the run was handed and never answered goes on when the run ends
//! ([`Routes::left_open`]), as Playwright continues a disconnected client's paused requests.

use std::collections::{HashMap, HashSet, VecDeque};
use std::mem;
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use browseros_cdp::{CdpError, SessionId};
use neo_runtime_spec::serve::{NameValue, RouteCall, RouteContinueParams, RouteFulfillParams};
use serde_json::{Map, Value, json};

use tracing::debug;

use crate::click::PageError;
use crate::network::{HopId, NetworkLog, PauseTarget};
use crate::page::PageSession;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::tabs::Tabs;
use crate::url_match::{self, SerializedUrlMatch, UrlMatcher};
use crate::world;

/// Neo: the `route@` guid prefix; the rest is `<targetId>:<n>`, minted by the run.
const ROUTE_PREFIX: &str = "route@";

/// Playwright: `server/network.ts` `Route._startHandling`: `assert(!this._handled, 'Route is already handled!');`
const ROUTE_HANDLED: &str = "Route is already handled!";

/// Playwright: `server/network.ts` `Route.continue`: `throw new Error('New URL must have same protocol as overridden URL');`
const NEW_PROTOCOL: &str = "New URL must have same protocol as overridden URL";

/// Playwright: `server/chromium/crNetworkManager.ts` `RouteImpl.abort`: `assert(errorReason, 'Unknown error code: ' + errorCode);`
const UNKNOWN_ERROR_CODE: &str = "Unknown error code: ";

/// Playwright: `server/network.ts` `Route.abort`: `async abort(errorCode: string = 'failed') {`
const DEFAULT_ERROR_CODE: &str = "failed";

/// Playwright: `server/network.ts` `Route.fulfill`: `assert(buffer, 'Fetch response has been disposed');`
const FETCH_DISPOSED: &str = "Fetch response has been disposed";

/// Deviation: `server/network.ts` `Route.continue`: `const newUrl = new URL(overrides.url);`
/// Playwright's `new URL` throws a `TypeError`; Neo parses with Rust's WHATWG parser and
/// answers that error's message as a plain error.
const INVALID_URL: &str = "Invalid URL";

/// Playwright: `server/chromium/crNetworkManager.ts` `catchDisallowedErrors`: `if (isProtocolError(e) && e.message.includes('Invalid http status code or phrase'))`
const BAD_STATUS: &str = "Invalid http status code or phrase";

/// Playwright: `server/chromium/crNetworkManager.ts` `catchDisallowedErrors`: `if (isProtocolError(e) && e.message.includes('Unsafe header'))`
const UNSAFE_HEADER: &str = "Unsafe header";

/// Neo: pauses kept while they wait for their `requestWillBeSent`; past this the oldest goes on
/// untouched, so a pause whose request never reaches the log cannot hold the page.
const MAX_PARKED: usize = 256;

/// Neo: a poisoned lock (a panic elsewhere while holding it), which no script can cause.
const ROUTES_UNREADABLE: &str = "neo-runtime: the tab's routes cannot be read";

/// Playwright: `server/chromium/crNetworkManager.ts` `errorReasons`
#[rustfmt::skip]
const ERROR_REASONS: &[(&str, &str)] = &[
    ("aborted", "Aborted"),
    ("accessdenied", "AccessDenied"),
    ("addressunreachable", "AddressUnreachable"),
    ("blockedbyclient", "BlockedByClient"),
    ("blockedbyresponse", "BlockedByResponse"),
    ("connectionaborted", "ConnectionAborted"),
    ("connectionclosed", "ConnectionClosed"),
    ("connectionfailed", "ConnectionFailed"),
    ("connectionrefused", "ConnectionRefused"),
    ("connectionreset", "ConnectionReset"),
    ("internetdisconnected", "InternetDisconnected"),
    ("namenotresolved", "NameNotResolved"),
    ("timedout", "TimedOut"),
    ("failed", "Failed"),
];

/// Playwright: `server/network.ts` `FORBIDDEN_HEADER_NAMES`
#[rustfmt::skip]
const FORBIDDEN_HEADER_NAMES: &[&str] = &[
    "accept-charset",
    "accept-encoding",
    "access-control-request-headers",
    "access-control-request-method",
    "connection",
    "content-length",
    "cookie",
    "date",
    "dnt",
    "expect",
    "host",
    "keep-alive",
    "origin",
    "referer",
    "set-cookie",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "via",
];

/// Playwright: `server/network.ts` `FORBIDDEN_METHODS`: `const FORBIDDEN_METHODS = new Set(['CONNECT', 'TRACE', 'TRACK']);`
const FORBIDDEN_METHODS: &[&str] = &["CONNECT", "TRACE", "TRACK"];

/// Playwright: `server/network.ts` `STATUS_TEXTS`
#[rustfmt::skip]
const STATUS_TEXTS: &[(i64, &str)] = &[
    (100, "Continue"),
    (101, "Switching Protocols"),
    (102, "Processing"),
    (103, "Early Hints"),
    (200, "OK"),
    (201, "Created"),
    (202, "Accepted"),
    (203, "Non-Authoritative Information"),
    (204, "No Content"),
    (205, "Reset Content"),
    (206, "Partial Content"),
    (207, "Multi-Status"),
    (208, "Already Reported"),
    (226, "IM Used"),
    (300, "Multiple Choices"),
    (301, "Moved Permanently"),
    (302, "Found"),
    (303, "See Other"),
    (304, "Not Modified"),
    (305, "Use Proxy"),
    (306, "Switch Proxy"),
    (307, "Temporary Redirect"),
    (308, "Permanent Redirect"),
    (400, "Bad Request"),
    (401, "Unauthorized"),
    (402, "Payment Required"),
    (403, "Forbidden"),
    (404, "Not Found"),
    (405, "Method Not Allowed"),
    (406, "Not Acceptable"),
    (407, "Proxy Authentication Required"),
    (408, "Request Timeout"),
    (409, "Conflict"),
    (410, "Gone"),
    (411, "Length Required"),
    (412, "Precondition Failed"),
    (413, "Payload Too Large"),
    (414, "URI Too Long"),
    (415, "Unsupported Media Type"),
    (416, "Range Not Satisfiable"),
    (417, "Expectation Failed"),
    (418, "I'm a teapot"),
    (421, "Misdirected Request"),
    (422, "Unprocessable Entity"),
    (423, "Locked"),
    (424, "Failed Dependency"),
    (425, "Too Early"),
    (426, "Upgrade Required"),
    (428, "Precondition Required"),
    (429, "Too Many Requests"),
    (431, "Request Header Fields Too Large"),
    (451, "Unavailable For Legal Reasons"),
    (500, "Internal Server Error"),
    (501, "Not Implemented"),
    (502, "Bad Gateway"),
    (503, "Service Unavailable"),
    (504, "Gateway Timeout"),
    (505, "HTTP Version Not Supported"),
    (506, "Variant Also Negotiates"),
    (507, "Insufficient Storage"),
    (508, "Loop Detected"),
    (510, "Not Extended"),
    (511, "Network Authentication Required"),
];

/// Playwright: `server/network.ts` `statusText`: `return STATUS_TEXTS[String(status)] || 'Unknown';`
const UNKNOWN_STATUS: &str = "Unknown";

/// Neo: the guid of route `index` on tab `target`.
pub(crate) fn guid(target: &str, index: u64) -> String {
    format!("{ROUTE_PREFIX}{target}:{index}")
}

/// Neo: the tab and index a route guid names, or `None` for another guid.
pub(crate) fn parse(guid: &str) -> Option<(&str, u64)> {
    let (target, index) = guid.strip_prefix(ROUTE_PREFIX)?.rsplit_once(':')?;
    if target.is_empty() || target.contains('/') {
        return None;
    }
    Some((target, index.parse().ok()?))
}

/// Playwright: `server/dispatchers/pageDispatcher.ts` `PageDispatcher.setNetworkInterceptionPatterns`: `this._interceptionUrlMatchers = params.patterns.map(deserializeURLMatch);`
///
/// One owner's interception patterns, as its client serialized them; what its `route` event
/// covers.
#[derive(Debug, Default)]
pub(crate) struct Patterns {
    matchers: Vec<UrlMatcher>,
    /// Deviation: `server/dispatchers/pageDispatcher.ts` `PageDispatcher.constructor`: `const matchesSome = this._interceptionUrlMatchers.some(urlMatch => urlMatches(this._page.browserContext._options.baseURL, request.url(), urlMatch));`
    /// A `URLPattern` matcher names every URL here (Neo has no `URLPattern`); the client's
    /// handler checks it exactly, as Playwright's does on a server without one.
    every: bool,
}

/// Neo: Patterns from the client's serialized matchers.
impl Patterns {
    /// One pattern per `(glob, regexSource, regexFlags, urlPattern given)`.
    ///
    /// # Errors
    ///
    /// The matcher's error for a pattern Rust's URL matcher cannot compile.
    pub(crate) fn new(
        items: impl IntoIterator<Item = (Option<String>, Option<String>, Option<String>, bool)>,
    ) -> Result<Self, PlaywrightError> {
        let mut patterns = Self::default();
        for (glob, regex_source, regex_flags, url_pattern) in items {
            if url_pattern {
                patterns.every = true;
                continue;
            }
            patterns.matchers.push(UrlMatcher::new(&SerializedUrlMatch {
                glob,
                regex_source,
                regex_flags,
            })?);
        }
        Ok(patterns)
    }

    pub(crate) fn is_empty(&self) -> bool {
        !self.every && self.matchers.is_empty()
    }

    fn matches(&self, url: &str) -> bool {
        self.every
            || self
                .matchers
                .iter()
                .any(|matcher| matcher.matches(url).unwrap_or(false))
    }
}

/// Neo: the run's context routes, shared by every tab the run has a session on.
#[derive(Default)]
pub(crate) struct ContextRoutes(Mutex<Option<Arc<Patterns>>>);

/// Neo: the context's patterns, set by `BrowserContext.setNetworkInterceptionPatterns`.
impl ContextRoutes {
    /// Replaces the context's patterns; whether it had some before.
    pub(crate) fn set(&self, patterns: Patterns) -> bool {
        let Ok(mut current) = self.0.lock() else {
            return false;
        };
        let had = current.is_some();
        *current = (!patterns.is_empty()).then(|| Arc::new(patterns));
        had
    }

    fn get(&self) -> Option<Arc<Patterns>> {
        self.0.lock().ok()?.clone()
    }

    pub(crate) fn any(&self) -> bool {
        self.get().is_some()
    }
}

/// Which channel's `route` event handed a pause out: Playwright's page dispatcher's, whose
/// handlers come first, or the context's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Channel {
    Page,
    Context,
}

/// What the tab's watcher does with one pause.
#[derive(Debug, PartialEq)]
pub(crate) enum Pause {
    /// Sends this `Fetch` command on the session at once, without waiting for its answer.
    Answer(Answer),
    /// Waits for its `requestWillBeSent` ([`Routes::unpark`]).
    Parked,
    /// Hands it to the run as a `route` event on `channel`; [`Routes::emitted`] once queued, or
    /// [`Routes::release`] when nobody takes it.
    Route {
        index: u64,
        channel: Channel,
        params: Value,
    },
}

/// One `Fetch` command for a pause, on the session that paused it (`None` for the tab's own).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Answer {
    pub(crate) session: Option<SessionId>,
    pub(crate) method: &'static str,
    pub(crate) params: Value,
}

/// A pause handed to the run, until its one answer.
#[derive(Clone, Debug)]
struct Paused {
    session: Option<SessionId>,
    /// `Fetch`'s interception id.
    request_id: String,
    network_id: String,
    channel: Channel,
    /// The request as it paused: Playwright's server request reads these.
    url: String,
    headers: Vec<NameValue>,
    /// Queued as the run's event, so the script owes the answer.
    emitted: bool,
}

/// A pause that came before its `requestWillBeSent`.
#[derive(Clone, Debug)]
struct Parked {
    session: Option<SessionId>,
    params: Value,
}

#[derive(Default)]
struct State {
    next: u64,
    page: Option<Arc<Patterns>>,
    /// The `Fetch` interception of the tab's sessions is on.
    on: bool,
    parked: VecDeque<(String, Parked)>,
    paused: HashMap<u64, Paused>,
    /// The pauses whose one answer was taken, by the script or a release.
    answered: HashSet<u64>,
    /// Playwright: `server/chromium/crNetworkManager.ts` `RouteImpl.continue`: `this._alreadyContinuedParams = {`
    ///
    /// Each continued request's params (without its interception id), by `networkId`: its
    /// redirect hops keep its header overrides, and a restart of it goes on with them again.
    continued: HashMap<String, Map<String, Value>>,
}

/// Neo: one tab's routes in this run (`route@<targetId>:<n>`).
pub(crate) struct Routes {
    target: String,
    context: Arc<ContextRoutes>,
    state: Mutex<State>,
}

/// Neo: the tab's patterns, pauses and answers.
impl Routes {
    pub(crate) fn new(target: &str, context: Arc<ContextRoutes>) -> Self {
        Self {
            target: target.to_string(),
            context,
            state: Mutex::new(State::default()),
        }
    }

    fn with<T>(&self, run: impl FnOnce(&mut State) -> T) -> Result<T, PlaywrightError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| PlaywrightError::error(ROUTES_UNREADABLE))?;
        Ok(run(&mut state))
    }

    /// Replaces the page's patterns. Returns the pauses the page's handlers held and nobody
    /// answered, now released (`Route.removeHandler`), to go on as they are.
    pub(crate) fn set_page(&self, patterns: Patterns) -> Vec<Answer> {
        let patterns = (!patterns.is_empty()).then(|| Arc::new(patterns));
        let emptied = patterns.is_none();
        self.with(|state| {
            state.page = patterns;
            if emptied {
                release(state, Channel::Page)
            } else {
                Vec::new()
            }
        })
        .unwrap_or_default()
    }

    /// The context's patterns emptied: the pauses its handlers held go on.
    pub(crate) fn context_emptied(&self) -> Vec<Answer> {
        self.with(|state| release(state, Channel::Context))
            .unwrap_or_default()
    }

    /// Whether the tab's sessions should start (`Some(true)`) or stop (`Some(false)`) pausing
    /// requests, now that some route of the page or of the context exists or none does; `None`
    /// when that does not change. Records the new state.
    pub(crate) fn wants_interception(&self) -> Option<bool> {
        let context = self.context.any();
        self.with(|state| {
            let wanted = state.page.is_some() || context;
            let changed = wanted != state.on;
            state.on = wanted;
            changed.then_some(wanted)
        })
        .ok()
        .flatten()
    }

    /// The run is over: the interception goes off. Whether it was on.
    fn stop_intercepting(&self) -> bool {
        self.with(|state| mem::take(&mut state.on)).unwrap_or(false)
    }

    /// Whether the interception is on: an OOPIF session attached now pauses its requests too.
    pub(crate) fn intercepting(&self) -> bool {
        self.with(|state| state.on).unwrap_or(false)
    }

    /// Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._onRequestPaused`: `if (!event.networkId) {`
    ///
    /// What the watcher does with `Fetch.requestPaused` from `session` (`None` for the tab's).
    pub(crate) fn paused(
        &self,
        session: Option<&SessionId>,
        params: &Value,
        log: &NetworkLog,
    ) -> Pause {
        let Some(request_id) = params["requestId"].as_str() else {
            return Pause::Parked;
        };
        let Some(network_id) = params["networkId"].as_str().filter(|id| !id.is_empty()) else {
            return Pause::Answer(go_on(session, request_id, Map::new()));
        };
        let redirected = params["redirectedRequestId"]
            .as_str()
            .is_some_and(|id| !id.is_empty());
        let continued = self
            .with(|state| state.continued.get(network_id).cloned())
            .ok()
            .flatten();
        if redirected {
            return Pause::Answer(redirect_hop(
                session,
                request_id,
                params,
                continued.as_ref(),
            ));
        }
        // Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._onRequestPaused`: `if (alreadyContinuedParams && !event.redirectedRequestId) {`
        if let Some(continued) = continued {
            return Pause::Answer(go_on(session, request_id, continued));
        }
        match log.pause_target(network_id) {
            PauseTarget::Unknown => self.park(session, network_id, params),
            PauseTarget::Skipped => Pause::Answer(go_on(session, request_id, Map::new())),
            PauseTarget::Hop { hop, favicon } => {
                self.route_or_answer(session, request_id, network_id, params, log, &hop, favicon)
            }
        }
    }

    /// The parked pause of `network_id`, now that its `requestWillBeSent` arrived.
    pub(crate) fn unpark(&self, network_id: &str, log: &NetworkLog) -> Option<Pause> {
        let parked = self
            .with(|state| {
                let index = state.parked.iter().position(|(id, _)| id == network_id)?;
                state.parked.remove(index).map(|(_, parked)| parked)
            })
            .ok()
            .flatten()?;
        Some(self.paused(parked.session.as_ref(), &parked.params, log))
    }

    fn park(&self, session: Option<&SessionId>, network_id: &str, params: &Value) -> Pause {
        let evicted = self
            .with(|state| {
                state.parked.push_back((
                    network_id.to_string(),
                    Parked {
                        session: session.cloned(),
                        params: params.clone(),
                    },
                ));
                (state.parked.len() > MAX_PARKED)
                    .then(|| state.parked.pop_front())
                    .flatten()
            })
            .ok()
            .flatten();
        match evicted.and_then(|(_, parked)| {
            let request_id = parked.params["requestId"].as_str()?.to_string();
            Some(go_on(parked.session.as_ref(), &request_id, Map::new()))
        }) {
            Some(answer) => Pause::Answer(answer),
            None => Pause::Parked,
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one pause's facts, read once from its event and the log"
    )]
    fn route_or_answer(
        &self,
        session: Option<&SessionId>,
        request_id: &str,
        network_id: &str,
        params: &Value,
        log: &NetworkLog,
        hop: &HopId,
        favicon: bool,
    ) -> Pause {
        // Playwright: `server/frames.ts` `FrameManager.requestStarted`: `route?.abort('aborted').catch(() => {});`
        if favicon {
            return Pause::Answer(Answer {
                session: session.cloned(),
                method: "Fetch.failRequest",
                params: json!({ "requestId": request_id, "errorReason": "Aborted" }),
            });
        }
        let request = &params["request"];
        let url = request["url"].as_str().unwrap_or_default().to_string();
        let headers = headers_array(&request["headers"]);
        if is_preflight(params) {
            return Pause::Answer(preflight(session, request_id, &headers));
        }
        let page = self.with(|state| state.page.clone()).ok().flatten();
        let channel = if page.is_some_and(|patterns| patterns.matches(&url)) {
            Channel::Page
        } else if self
            .context
            .get()
            .is_some_and(|patterns| patterns.matches(&url))
        {
            Channel::Context
        } else {
            return Pause::Answer(go_on(session, request_id, Map::new()));
        };
        let Ok(record) = log.created_request(hop) else {
            return Pause::Answer(go_on(session, request_id, Map::new()));
        };
        // The handler's `allHeaders()` reads the request's raw headers, which Chromium reports
        // only once the request is sent: the pause's headers stand in until then.
        log.paused_headers(hop, headers.clone());
        let index = self
            .with(|state| {
                state.next += 1;
                let index = state.next;
                state.paused.insert(
                    index,
                    Paused {
                        session: session.cloned(),
                        request_id: request_id.to_string(),
                        network_id: network_id.to_string(),
                        channel,
                        url,
                        headers,
                        emitted: false,
                    },
                );
                index
            })
            .unwrap_or_default();
        let route = guid(&self.target, index);
        Pause::Route {
            index,
            channel,
            params: json!({
                "route": { "guid": route, "initializer": { "request": { "guid": record.guid } } },
                "request": record,
                "page": { "guid": format!("page@{}", self.target) },
            }),
        }
    }

    /// The run's event for pause `index` is queued: the script owes its answer.
    pub(crate) fn emitted(&self, index: u64) {
        let _marked = self.with(|state| {
            if let Some(paused) = state.paused.get_mut(&index) {
                paused.emitted = true;
            }
        });
    }

    /// Nobody took pause `index` (no listener, a full queue, the run over): it goes on.
    pub(crate) fn release(&self, index: u64) -> Option<Answer> {
        let paused = self.with(|state| state.paused.remove(&index)).ok()??;
        Some(go_on(
            paused.session.as_ref(),
            &paused.request_id,
            Map::new(),
        ))
    }

    /// Deviation: `server/dispatchers/pageDispatcher.ts` `PageDispatcher._onDispose`: `this._page.removeRequestInterceptor(this._requestInterceptor).catch(() => {});`
    /// As Playwright releases a gone client's paused requests, the run's end continues every
    /// pause it was handed and never answered, and every one still parked; Neo's tab outlives
    /// the run. (A session's detach alone also resumes them on BrowserOS, measured.)
    pub(crate) fn left_open(&self) -> Vec<Answer> {
        self.with(|state| {
            let mut answers: Vec<Answer> = state
                .paused
                .drain()
                .map(|(_, paused)| go_on(paused.session.as_ref(), &paused.request_id, Map::new()))
                .collect();
            answers.extend(state.parked.drain(..).filter_map(|(_, parked)| {
                let request_id = parked.params["requestId"].as_str()?.to_string();
                Some(go_on(parked.session.as_ref(), &request_id, Map::new()))
            }));
            answers
        })
        .unwrap_or_default()
    }

    /// The pauses an OOPIF session that went away held: they cannot be answered any more.
    pub(crate) fn forget_session(&self, session: &SessionId) {
        let _forgotten = self.with(|state| {
            state
                .paused
                .retain(|_, paused| paused.session.as_ref() != Some(session));
            state
                .parked
                .retain(|(_, parked)| parked.session.as_ref() != Some(session));
        });
    }

    /// Playwright: `server/dispatchers/networkDispatchers.ts` `RouteDispatcher._checkNotHandled`: `throw new Error('Route is already handled!');`
    ///
    /// Takes the one answer of pause `index`, or refuses a second one with Playwright's error.
    /// The answer is taken before it is checked, as Playwright's dispatcher does: an answer the
    /// browser cannot carry (a URL changing protocol, an unknown abort code) still uses it, and
    /// the pause stays, for the run's end to release.
    fn take(&self, index: u64) -> Result<Paused, PlaywrightError> {
        self.with(|state| {
            let paused = state.paused.get(&index)?.clone();
            state.answered.insert(index).then_some(paused)
        })?
        .ok_or_else(|| PlaywrightError::error(ROUTE_HANDLED))
    }

    /// The `Fetch` command answering pause `index` as `call` asks. Takes its one answer.
    ///
    /// # Errors
    ///
    /// Playwright's error for a second answer, a URL override that changes the protocol, or an
    /// unknown abort code.
    pub(crate) fn answer(&self, index: u64, call: RouteCall) -> Result<Answer, PlaywrightError> {
        let paused = self.take(index)?;
        let (method, params) = match call {
            RouteCall::Abort(params) => (
                "Fetch.failRequest",
                abort_params(&paused, params.error_code)?,
            ),
            RouteCall::Continue(params) => {
                let continued = continue_params(&paused, params)?;
                let _kept = self.with(|state| {
                    state
                        .continued
                        .insert(paused.network_id.clone(), continued.clone())
                });
                let mut params = continued;
                params.insert("requestId".into(), json!(paused.request_id));
                ("Fetch.continueRequest", Value::Object(params))
            }
            RouteCall::Fulfill(params) => {
                ("Fetch.fulfillRequest", fulfill_params(&paused, params)?)
            }
        };
        // The browser carries it: the pause is no longer the run's to release.
        let _released = self.with(|state| state.paused.remove(&index));
        Ok(Answer {
            session: paused.session,
            method,
            params,
        })
    }
}

/// Neo: `Route.abort`, `Route.continue` and `Route.fulfill` on pause `index` of `page`'s tab.
///
/// # Errors
///
/// Playwright's error for a second answer and for a refused override; the browser's refusal of
/// a status or header the script gave; `TargetClosedError` once the tab is gone.
pub(crate) async fn serve(
    page: &PageSession,
    index: u64,
    call: RouteCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let answer = page.routes.answer(index, call)?;
    let sent = progress
        .race(page.send_to(answer.session.as_ref(), answer.method, answer.params))
        .await?;
    answered(answer.method, sent)?;
    Ok(None)
}

/// Neo: `Page.setNetworkInterceptionPatterns`: the page's patterns, the pauses its handlers
/// held when it has none left, then whether its sessions pause requests.
///
/// # Errors
///
/// The browser's error turning the pausing over; `TargetClosedError` once the tab is gone.
pub(crate) async fn set_page_patterns(
    page: &PageSession,
    patterns: Patterns,
    progress: &Progress,
) -> Result<(), CallError> {
    let released = page.routes.set_page(patterns);
    progress.race(send_all(page, released)).await?;
    turn(page, progress).await
}

/// Neo: `BrowserContext.setNetworkInterceptionPatterns`, for every tab the run has a session
/// on; a tab attached later pauses at its attach (`Tabs::session`).
///
/// # Errors
///
/// As [`set_page_patterns`].
pub(crate) async fn set_context_patterns(
    tabs: &Tabs,
    patterns: Patterns,
    progress: &Progress,
) -> Result<(), CallError> {
    let emptied = patterns.is_empty();
    tabs.routes().set(patterns);
    for (_, page) in tabs.attached() {
        if emptied {
            progress
                .race(send_all(&page, page.routes.context_emptied()))
                .await?;
        }
        turn(&page, progress).await?;
    }
    Ok(())
}

/// Neo: turns the tab's request pausing on or off when its routes changed that.
async fn turn(page: &PageSession, progress: &Progress) -> Result<(), CallError> {
    if let Some(on) = page.routes.wants_interception() {
        progress
            .race(page.intercept(on))
            .await?
            .map_err(|error| cdp_error(&error))?;
    }
    Ok(())
}

/// Neo: at the run's end, every pause it left open goes on and the tab stops pausing
/// requests, before its session detaches ([`Routes::left_open`]).
pub(crate) async fn finish(page: &PageSession) {
    send_all(page, page.routes.left_open()).await;
    if page.routes.stop_intercepting()
        && let Err(error) = page.intercept(false).await
    {
        debug!(%error, "turning a tab's request pausing off at run end failed");
    }
}

/// Neo: sends answers whose outcome nobody waits for (released pauses).
async fn send_all(page: &PageSession, answers: Vec<Answer>) {
    for answer in answers {
        if let Err(error) = page
            .send_to(answer.session.as_ref(), answer.method, answer.params)
            .await
        {
            debug!(%error, "releasing a paused request failed");
        }
    }
}

/// Neo: a CDP error turning the pausing over, as the script sees it.
fn cdp_error(error: &CdpError) -> PlaywrightError {
    if world::classify(error) == PageError::Closed {
        PlaywrightError::target_closed()
    } else {
        PlaywrightError::error(error.to_string())
    }
}

/// Neo: the release of every pause handed out on `channel` and not answered.
fn release(state: &mut State, channel: Channel) -> Vec<Answer> {
    let released: Vec<u64> = state
        .paused
        .iter()
        .filter(|(_, paused)| paused.channel == channel && paused.emitted)
        .map(|(index, _)| *index)
        .collect();
    released
        .into_iter()
        .filter_map(|index| state.paused.remove(&index))
        .map(|paused| go_on(paused.session.as_ref(), &paused.request_id, Map::new()))
        .collect()
}

/// Playwright: `server/chromium/crNetworkManager.ts` `RouteImpl.continue`: `await this._session.send('Fetch.continueRequest', this._alreadyContinuedParams);`
fn go_on(session: Option<&SessionId>, request_id: &str, mut params: Map<String, Value>) -> Answer {
    params.insert("requestId".into(), json!(request_id));
    Answer {
        session: session.cloned(),
        method: "Fetch.continueRequest",
        params: Value::Object(params),
    }
}

/// Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._onRequest`: `headersOverride = removeCookieHeader(network.applyHeadersOverrides(originalHeaders, headersOverride));`
///
/// A redirect hop goes on untouched but for the header overrides its first hop was continued
/// with, which Chromium does not keep across a redirect.
fn redirect_hop(
    session: Option<&SessionId>,
    request_id: &str,
    params: &Value,
    continued: Option<&Map<String, Value>>,
) -> Answer {
    let mut answer = Map::new();
    if let Some(overrides) = continued
        .and_then(|continued| continued.get("headers"))
        .and_then(|headers| serde_json::from_value::<Vec<NameValue>>(headers.clone()).ok())
    {
        let original = headers_array(&params["request"]["headers"]);
        answer.insert(
            "headers".into(),
            json!(remove_cookie_header(apply_headers_overrides(
                &original, overrides
            ))),
        );
    }
    go_on(session, request_id, answer)
}

/// Deviation: `server/chromium/crNetworkManager.ts` `CRNetworkManager._onRequest`: `const isInterceptedOptionsPreflight = !!requestPausedEvent && requestPausedEvent.request.method === 'OPTIONS' && requestWillBeSentEvent.initiator.type === 'preflight';`
/// Neo reads the preflight from the pause's resource type (`Preflight`), which Chromium gives
/// exactly the CORS preflights; the network log keeps no initiator.
fn is_preflight(params: &Value) -> bool {
    params["request"]["method"] == "OPTIONS" && params["resourceType"] == "Preflight"
}

/// Playwright: `server/chromium/crNetworkManager.ts` `CRNetworkManager._onRequest`: `{ name: 'Access-Control-Allow-Credentials', value: 'true' }`
///
/// An intercepted CORS preflight is answered at once, allowing what it asks.
fn preflight(session: Option<&SessionId>, request_id: &str, headers: &[NameValue]) -> Answer {
    let header = |name: &str| {
        headers
            .iter()
            .find(|header| header.name == name)
            .map(|header| header.value.clone())
    };
    let mut response = vec![
        json!({ "name": "Access-Control-Allow-Origin", "value": header("Origin").unwrap_or_else(|| "*".into()) }),
        json!({
            "name": "Access-Control-Allow-Methods",
            "value": header("Access-Control-Request-Method").unwrap_or_else(|| "GET, POST, OPTIONS, DELETE".into()),
        }),
        json!({ "name": "Access-Control-Allow-Credentials", "value": "true" }),
    ];
    if let Some(allowed) = header("Access-Control-Request-Headers") {
        response.push(json!({ "name": "Access-Control-Allow-Headers", "value": allowed }));
    }
    Answer {
        session: session.cloned(),
        method: "Fetch.fulfillRequest",
        params: json!({
            "requestId": request_id,
            "responseCode": 204,
            "responsePhrase": status_text(204),
            "responseHeaders": response,
            "body": "",
        }),
    }
}

/// Playwright: `server/chromium/crNetworkManager.ts` `RouteImpl.abort`: `const errorReason = errorReasons[errorCode];`
fn abort_params(paused: &Paused, error_code: Option<String>) -> Result<Value, PlaywrightError> {
    let code = error_code
        .filter(|code| !code.is_empty())
        .unwrap_or_else(|| DEFAULT_ERROR_CODE.to_string());
    let reason = ERROR_REASONS
        .iter()
        .find(|(name, _)| *name == code)
        .map(|(_, reason)| *reason)
        .ok_or_else(|| PlaywrightError::error(format!("{UNKNOWN_ERROR_CODE}{code}")))?;
    Ok(json!({ "requestId": paused.request_id, "errorReason": reason }))
}

/// Playwright: `server/network.ts` `Route.continue`: `if (oldUrl.protocol !== newUrl.protocol)`
///
/// `Fetch.continueRequest`'s params for the script's overrides, without the interception id.
fn continue_params(
    paused: &Paused,
    params: RouteContinueParams,
) -> Result<Map<String, Value>, PlaywrightError> {
    let mut out = Map::new();
    if let Some(url) = params.url {
        let new = url_match::parse_url(&url).ok_or_else(|| PlaywrightError::error(INVALID_URL))?;
        let old =
            url_match::parse_url(&paused.url).ok_or_else(|| PlaywrightError::error(INVALID_URL))?;
        if old.protocol != new.protocol {
            return Err(PlaywrightError::error(NEW_PROTOCOL));
        }
        out.insert("url".into(), json!(url));
    }
    if let Some(method) = params.method {
        out.insert("method".into(), json!(method));
    }
    if let Some(headers) = params.headers {
        let headers = apply_headers_overrides(&paused.headers, headers);
        out.insert("headers".into(), json!(remove_cookie_header(headers)));
    }
    if let Some(post_data) = params.post_data {
        out.insert("postData".into(), json!(post_data));
    }
    Ok(out)
}

/// Playwright: `server/network.ts` `Route.fulfill`: `status: overrides.status || 200,`
///
/// `Fetch.fulfillRequest`'s params for the script's response.
fn fulfill_params(paused: &Paused, params: RouteFulfillParams) -> Result<Value, PlaywrightError> {
    let status = params.status.filter(|status| *status != 0).unwrap_or(200);
    let mut headers = params.headers.unwrap_or_default();
    maybe_add_cors_headers(paused, &mut headers);
    let body = match params.body {
        Some(body) if params.is_base64.unwrap_or(false) => body,
        Some(body) => STANDARD.encode(body.as_bytes()),
        // Deviation: `server/network.ts` `Route.fulfill`: `if (overrides.fetchResponseUid) {`
        // Neo keeps no `route.fetch` response bodies yet: a fulfill that names one answers
        // Playwright's error for a disposed one.
        None if params.fetch_response_uid.is_some() => {
            return Err(PlaywrightError::error(FETCH_DISPOSED));
        }
        None => String::new(),
    };
    Ok(json!({
        "requestId": paused.request_id,
        "responseCode": status,
        "responsePhrase": status_text(status),
        "responseHeaders": split_set_cookie_header(headers),
        "body": body,
    }))
}

/// Playwright: `server/network.ts` `Route._maybeAddCorsHeaders`: `headers.push({ name: 'access-control-allow-origin', value: origin });`
fn maybe_add_cors_headers(paused: &Paused, headers: &mut Vec<NameValue>) {
    let Some(origin) = paused
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("origin"))
        .map(|header| header.value.clone())
    else {
        return;
    };
    let Some(url) = url_match::parse_url(&paused.url) else {
        return;
    };
    if !url.protocol.starts_with("http") || url.origin == origin.trim() {
        return;
    }
    if headers
        .iter()
        .any(|header| header.name == "access-control-allow-origin")
    {
        return;
    }
    for (name, value) in [
        ("access-control-allow-origin", origin.as_str()),
        ("access-control-allow-credentials", "true"),
        ("vary", "Origin"),
    ] {
        headers.push(NameValue {
            name: name.to_string(),
            value: value.to_string(),
        });
    }
}

/// Playwright: `server/network.ts` `statusText`: `return STATUS_TEXTS[String(status)] || 'Unknown';`
fn status_text(status: i64) -> &'static str {
    STATUS_TEXTS
        .iter()
        .find(|(code, _)| *code == status)
        .map_or(UNKNOWN_STATUS, |(_, text)| text)
}

/// Playwright: `server/network.ts` `isForbiddenHeader`: `if (lowerName.startsWith('sec-'))`
fn is_forbidden_header(name: &str, value: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if FORBIDDEN_HEADER_NAMES.contains(&lower.as_str())
        || lower.starts_with("proxy-")
        || lower.starts_with("sec-")
    {
        return true;
    }
    matches!(
        lower.as_str(),
        "x-http-method" | "x-http-method-override" | "x-method-override"
    ) && FORBIDDEN_METHODS.contains(&value.to_ascii_uppercase().as_str())
}

/// Playwright: `server/network.ts` `applyHeadersOverrides`: `return mergeHeaders([allowedHeaders, forbiddenHeaders]);`
fn apply_headers_overrides(original: &[NameValue], overrides: Vec<NameValue>) -> Vec<NameValue> {
    let forbidden = original
        .iter()
        .filter(|header| is_forbidden_header(&header.name, &header.value))
        .cloned();
    let allowed = overrides
        .into_iter()
        .filter(|header| !is_forbidden_header(&header.name, &header.value));
    merge_headers(allowed.chain(forbidden))
}

/// Playwright: `server/network.ts` `mergeHeaders`: `lowerCaseToValue.set(lower, value);`
fn merge_headers(headers: impl Iterator<Item = NameValue>) -> Vec<NameValue> {
    let mut merged: Vec<NameValue> = Vec::new();
    for header in headers {
        match merged
            .iter_mut()
            .find(|seen| seen.name.eq_ignore_ascii_case(&header.name))
        {
            Some(seen) => *seen = header,
            None => merged.push(header),
        }
    }
    merged
}

/// Playwright: `server/chromium/crNetworkManager.ts` `removeCookieHeader`: `return headers.filter(header => header.name.toLowerCase() !== 'cookie');`
fn remove_cookie_header(headers: Vec<NameValue>) -> Vec<NameValue> {
    headers
        .into_iter()
        .filter(|header| !header.name.eq_ignore_ascii_case("cookie"))
        .collect()
}

/// Playwright: `isomorphic/headers.ts` `splitSetCookieHeader`: `const values = header.value.split('\n');`
fn split_set_cookie_header(headers: Vec<NameValue>) -> Vec<NameValue> {
    let Some(index) = headers
        .iter()
        .position(|header| header.name.eq_ignore_ascii_case("set-cookie"))
    else {
        return headers;
    };
    if !headers[index].value.contains('\n') {
        return headers;
    }
    let mut headers = headers;
    let header = headers.remove(index);
    for (offset, value) in header.value.split('\n').enumerate() {
        headers.insert(
            index + offset,
            NameValue {
                name: header.name.clone(),
                value: value.to_string(),
            },
        );
    }
    headers
}

/// Neo: a CDP `Headers` object as name/value pairs, in its order.
fn headers_array(headers: &Value) -> Vec<NameValue> {
    headers
        .as_object()
        .map(|headers| {
            headers
                .iter()
                .map(|(name, value)| NameValue {
                    name: name.clone(),
                    value: value.as_str().unwrap_or_default().to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Playwright: `server/chromium/crNetworkManager.ts` `catchDisallowedErrors`: `async function catchDisallowedErrors(callback: () => Promise<void>) {`
/// Playwright: `server/protocolError.ts` `ProtocolError.setMessage`: `Protocol error (${this.method}): ${message}`
///
/// The answer's outcome: the browser's refusal of a status or header the script gave, which
/// the script can fix, rejects; any other error (the request already went, the page closed)
/// is swallowed, as Playwright does.
pub(crate) fn answered(method: &str, sent: Result<Value, CdpError>) -> Result<(), CallError> {
    match sent {
        Err(CdpError::Protocol { message, .. })
            if message.contains(BAD_STATUS) || message.contains(UNSAFE_HEADER) =>
        {
            Err(PlaywrightError::error(format!("Protocol error ({method}): {message}")).into())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use neo_runtime_spec::serve::RouteAbortParams;

    use super::*;

    fn paused(url: &str, headers: &[(&str, &str)]) -> Paused {
        Paused {
            session: None,
            request_id: "interception-1".into(),
            network_id: "R1".into(),
            channel: Channel::Page,
            url: url.into(),
            headers: headers
                .iter()
                .map(|(name, value)| NameValue {
                    name: (*name).into(),
                    value: (*value).into(),
                })
                .collect(),
            emitted: true,
        }
    }

    fn name_value(name: &str, value: &str) -> NameValue {
        NameValue {
            name: name.into(),
            value: value.into(),
        }
    }

    #[test]
    fn tests_that_route_guids_round_trip() {
        let route = guid("ABC", 3);
        assert_eq!(route, "route@ABC:3");
        assert_eq!(parse(&route), Some(("ABC", 3)));
        assert_eq!(parse("route@ABC"), None);
        assert_eq!(parse("route@:1"), None);
        assert_eq!(parse("dialog@ABC:1"), None);
    }

    #[test]
    fn tests_that_abort_maps_playwright_codes_and_refuses_others() {
        let paused = paused("https://a.test/x", &[]);
        assert_eq!(
            abort_params(&paused, None).unwrap()["errorReason"],
            "Failed"
        );
        assert_eq!(
            abort_params(&paused, Some("internetdisconnected".into())).unwrap()["errorReason"],
            "InternetDisconnected"
        );
        assert_eq!(
            abort_params(&paused, Some("Failed".into()))
                .unwrap_err()
                .message,
            "Unknown error code: Failed",
            "the codes are lower-case, as Playwright's table"
        );
    }

    #[test]
    fn tests_that_continue_keeps_forbidden_headers_and_never_sends_cookie() {
        let paused = paused(
            "https://a.test/x",
            &[
                ("Referer", "https://a.test/"),
                ("Cookie", "a=1"),
                ("X-Old", "1"),
            ],
        );
        let params = continue_params(
            &paused,
            RouteContinueParams {
                url: None,
                method: Some("POST".into()),
                headers: Some(vec![
                    name_value("x-new", "2"),
                    name_value("referer", "https://evil.test/"),
                    name_value("cookie", "b=2"),
                ]),
                post_data: None,
                is_fallback: false,
            },
        )
        .unwrap();
        assert_eq!(params["method"], "POST");
        assert_eq!(
            params["headers"],
            json!([
                { "name": "x-new", "value": "2" },
                { "name": "Referer", "value": "https://a.test/" },
            ]),
            "the original referer stays, the overrides' replaces nothing; cookie never goes"
        );
    }

    #[test]
    fn tests_that_continue_refuses_a_protocol_change() {
        let paused = paused("https://a.test/x", &[]);
        let error = continue_params(
            &paused,
            RouteContinueParams {
                url: Some("http://a.test/x".into()),
                method: None,
                headers: None,
                post_data: None,
                is_fallback: false,
            },
        )
        .unwrap_err();
        assert_eq!(error.message, NEW_PROTOCOL);
    }

    #[test]
    fn tests_that_fulfill_defaults_and_adds_cors_for_another_origin() {
        let paused = paused("https://api.test/data", &[("Origin", "https://page.test")]);
        let params = fulfill_params(
            &paused,
            RouteFulfillParams {
                status: None,
                headers: Some(vec![name_value("set-cookie", "a=1\nb=2")]),
                body: Some("hé".into()),
                is_base64: Some(false),
                fetch_response_uid: None,
            },
        )
        .unwrap();
        assert_eq!(params["responseCode"], 200);
        assert_eq!(params["responsePhrase"], "OK");
        assert_eq!(params["body"], STANDARD.encode("hé".as_bytes()));
        assert_eq!(
            params["responseHeaders"],
            json!([
                { "name": "set-cookie", "value": "a=1" },
                { "name": "set-cookie", "value": "b=2" },
                { "name": "access-control-allow-origin", "value": "https://page.test" },
                { "name": "access-control-allow-credentials", "value": "true" },
                { "name": "vary", "value": "Origin" },
            ])
        );
        assert_eq!(status_text(299), "Unknown");
    }

    #[test]
    fn tests_that_patterns_match_globs_and_regexps() {
        let patterns = Patterns::new([
            (Some("**/*.json".to_string()), None, None, false),
            (
                None,
                Some("img\\.png$".to_string()),
                Some(String::new()),
                false,
            ),
        ])
        .unwrap();
        assert!(patterns.matches("https://a.test/data.json"));
        assert!(patterns.matches("https://a.test/img.png"));
        assert!(!patterns.matches("https://a.test/page.html"));
        assert!(Patterns::new([]).unwrap().is_empty());
        assert!(
            Patterns::new([(None, None, None, true)])
                .unwrap()
                .matches("https://any.test/")
        );
    }

    #[test]
    fn tests_that_a_pause_takes_one_answer() {
        let routes = Routes::new("T", Arc::default());
        let _released = routes
            .set_page(Patterns::new([(Some("**/*".to_string()), None, None, false)]).unwrap());
        let paused = paused("https://a.test/x", &[]);
        routes
            .with(|state| {
                state.paused.insert(1, paused);
            })
            .unwrap();
        let answer = routes
            .answer(1, RouteCall::Abort(RouteAbortParams { error_code: None }))
            .unwrap();
        assert_eq!(answer.method, "Fetch.failRequest");
        let second = routes
            .answer(1, RouteCall::Abort(RouteAbortParams { error_code: None }))
            .unwrap_err();
        assert_eq!(second.message, ROUTE_HANDLED);
    }

    #[test]
    fn tests_that_a_refused_answer_still_takes_the_routes_one_answer() {
        let routes = Routes::new("T", Arc::default());
        routes
            .with(|state| {
                state.paused.insert(1, paused("https://a.test/x", &[]));
            })
            .unwrap();
        let refused = routes
            .answer(
                1,
                RouteCall::Continue(RouteContinueParams {
                    url: Some("file:///tmp/foo".into()),
                    method: None,
                    headers: None,
                    post_data: None,
                    is_fallback: false,
                }),
            )
            .unwrap_err();
        assert_eq!(refused.message, NEW_PROTOCOL);
        let second = routes
            .answer(1, RouteCall::Abort(RouteAbortParams { error_code: None }))
            .unwrap_err();
        assert_eq!(second.message, ROUTE_HANDLED, "as Playwright's dispatcher");
        assert_eq!(
            routes.left_open().len(),
            1,
            "the pause stays for the run's end to release"
        );
    }

    #[test]
    fn tests_that_emptied_patterns_release_what_their_handlers_held() {
        let routes = Routes::new("T", Arc::default());
        let _set = routes
            .set_page(Patterns::new([(Some("**/*".to_string()), None, None, false)]).unwrap());
        routes
            .with(|state| {
                state.paused.insert(1, paused("https://a.test/x", &[]));
            })
            .unwrap();
        let released = routes.set_page(Patterns::default());
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].method, "Fetch.continueRequest");
        assert_eq!(released[0].params, json!({ "requestId": "interception-1" }));
        assert!(routes.left_open().is_empty(), "released once");
    }

    #[test]
    fn tests_that_a_redirect_hop_keeps_its_first_hops_header_overrides() {
        let continued = json!({ "headers": [{ "name": "x-extra", "value": "1" }] });
        let answer = redirect_hop(
            None,
            "interception-2",
            &json!({ "request": { "headers": { "Referer": "https://a.test/", "Cookie": "a=1" } } }),
            continued.as_object(),
        );
        assert_eq!(
            answer.params,
            json!({
                "requestId": "interception-2",
                "headers": [
                    { "name": "x-extra", "value": "1" },
                    { "name": "Referer", "value": "https://a.test/" },
                ],
            })
        );
    }
}
