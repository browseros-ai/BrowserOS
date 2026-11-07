//! Every request of one tab, as CDP's Network domain reports them to this tab's session and to
//! the out-of-process frame sessions the run attached, and every value Playwright derives from
//! them: `server/chromium/crNetworkManager.ts` (`_onRequest`, `_createResponse`,
//! `_handleRequestRedirect`, `_onLoadingFinished`, `_onLoadingFailed`,
//! `ResponseExtraInfoTracker`), `server/frames.ts` (`FrameManager.requestStarted`,
//! `requestReceivedResponse`, `reportRequestFinished`, `requestFailed`) and `server/network.ts`
//! (`Request`, `Response`, the header and size formulas) at 26137face.
//!
//! ```text
//!  page.rs watcher ── Network.* event ──▶ NetworkLog::apply ──▶ Happenings (one task, arrival order)
//!                                                              └▶ network_events.rs: request,
//!                                                                 response, requestFinished/Failed
//!  navigate.rs goto ── navigation(loaderId) ──▶ the final hop's response, once known
//!  dispatch.rs / bridge.rs ── HopId ──▶ initializers, raw headers, sizes, timing, failure
//! ```
//!
//! Every request with a frame is recorded, except a `data:` URL (Playwright drops those): the
//! frame's navigation requests (`requestId` equal to its `loaderId`, type `Document`), which
//! `goto` and the navigation waits answer, and every other request, which the run's `request`
//! events, `waitForRequest`/`waitForResponse` and `page.requests()` hand out. Each request is a
//! chain of hops, one per redirect, sharing the request id. A request without a frame (a worker's,
//! a service worker's) is dropped, as Playwright drops a page's request it finds no frame for.
//! Recording starts with the session's `Network.enable` (`PageSession::enable_network`):
//! Chromium replays nothing from before it.
//!
//! Playwright resolves most of these values through promises that the next event settles; here
//! each is an `Option` that the matching event fills once (the first value wins, as a resolved
//! promise keeps its first value), and every change bumps a version that waiters watch. Nothing
//! here sends CDP.
//!
//! Neo-only:
//! - A session keeps at most [`MAX_CHAINS`] navigation chains and [`MAX_REQUESTS`] other
//!   requests' chains, oldest dropped first (a request still loading only once none has ended);
//!   an object from a dropped chain answers [`LogError::Dropped`].
//! - A watcher that lagged may have lost any event, so a navigation still unanswered when a lag
//!   hit or first seen mid-redirect, and any value still pending after a lag, answer
//!   [`LogError::Lost`] instead of a guess or a wait that could never end.
//! - `serde_json` sorts object keys, so a header object Chromium sends unsorted (a request's
//!   provisional headers) becomes an array in byte order; Chromium sends response headers and
//!   every raw (extra info) header object sorted already.

use std::collections::{HashSet, VecDeque};
use std::ops::RangeInclusive;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use browseros_cdp::SessionId;
use neo_runtime_spec::serve::{
    FrameChannel, NameValue, RemoteAddr, RequestChannel, RequestInitializer, RequestSizes,
    ResourceTiming, ResponseChannel, ResponseInitializer, SecurityDetails,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::watch;
use url::Url;

/// How many navigation chains one session keeps. A run navigates a tab far fewer times; the
/// cap only bounds a script that loops on `goto`, whose records live in Rust outside the realm.
pub(crate) const MAX_CHAINS: usize = 256;

/// Deviation: `server/network.ts` `Request.frame`: `return this._frame;`
/// How many other requests' chains (subresources, a frame's non-navigation fetches) one session
/// keeps. Playwright's objects live as long as their page; Neo's records live in Rust outside the
/// realm's memory limit for the run, so a busy page (a news site makes hundreds of requests per
/// load) is bounded here, the oldest finished request dropped first.
pub(crate) const MAX_REQUESTS: usize = 1000;

/// Playwright: `server/page.ts` `Page.addNetworkRequest`: `ensureArrayLimit(this._networkRequests, 100);`
const MAX_PAGE_REQUESTS: usize = 100;

/// Extra info that arrives before its request: the two kinds travel on different channels in
/// Chromium (`ResponseExtraInfoTracker`'s comment). Kept for the most recent request ids only:
/// a request's own events come within a few events of it, and requests this log drops (`data:`,
/// workers') leave theirs behind.
const MAX_ORPHANS: usize = 256;

/// Statuses whose body Playwright refuses (`internalBody`).
const REDIRECT_STATUSES: RangeInclusive<i64> = 300..=399;

/// Playwright: `server/chromium/crNetworkManager.ts` `kRefetchSafeResourceTypes`: `'font', 'image', 'manifest', 'media', 'script', 'stylesheet', 'texttrack',`
const REFETCH_SAFE_TYPES: [&str; 7] = [
    "font",
    "image",
    "manifest",
    "media",
    "script",
    "stylesheet",
    "texttrack",
];

/// One hop of one request chain: the address of a `Request` and of its `Response`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct HopId {
    /// The request's id; a navigation's is its loader id.
    pub(crate) request: String,
    /// 0 for the first request, then one per redirect.
    pub(crate) hop: usize,
}

/// Which object of a hop a guid names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObjectKind {
    Request,
    Response,
}

impl HopId {
    /// `request@<targetId>:<requestId>:<hop>`, so the dispatcher finds the page from the guid.
    pub(crate) fn request_guid(&self, target: &str) -> String {
        format!("request@{target}:{}:{}", self.request, self.hop)
    }

    /// `response@<targetId>:<requestId>:<hop>`.
    pub(crate) fn response_guid(&self, target: &str) -> String {
        format!("response@{target}:{}:{}", self.request, self.hop)
    }

    /// The kind, target id and hop a guid names, or `None` when it is not one of these guids.
    pub(crate) fn parse(guid: &str) -> Option<(ObjectKind, &str, Self)> {
        let (kind, rest) = if let Some(rest) = guid.strip_prefix("request@") {
            (ObjectKind::Request, rest)
        } else {
            (ObjectKind::Response, guid.strip_prefix("response@")?)
        };
        let mut parts = rest.splitn(3, ':');
        let target = parts.next().filter(|part| !part.is_empty())?;
        let request = parts.next().filter(|part| !part.is_empty())?;
        let hop = parts.next()?.parse().ok()?;
        Some((
            kind,
            target,
            Self {
                request: request.to_string(),
                hop,
            },
        ))
    }
}

/// Why the log cannot answer for an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum LogError {
    #[error(
        "Neo no longer holds this {0}: a tab keeps its last {MAX_CHAINS} navigations and its last \
         {MAX_REQUESTS} other requests"
    )]
    Dropped(&'static str),
    #[error(
        "Neo lost this navigation's network events (its event queue overflowed), so its response \
         is unknown"
    )]
    Lost,
    #[error("Response body is unavailable for redirect responses")]
    RedirectBody,
    /// A value Playwright cannot compute either, where its server throws.
    #[error("{0}")]
    Invalid(&'static str),
    #[error("neo-runtime: the network log cannot be read")]
    Poisoned,
}

/// What `Request.response` answers once the hop settled (Playwright's `_waitForResponsePromise`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Settled {
    Response(ResponseChannel),
    /// It failed before any response: `null`.
    NoResponse,
}

/// `_responseHeadersSizePromise`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeadersSize {
    Pending,
    /// Playwright's `null`: computed from the raw headers when asked.
    Unknown,
    Known(i64),
}

/// The source of `_encodedBodySizePromise`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum BodySize {
    Pending,
    /// Playwright's `null` (redirected or failed): `content-length` stands in.
    Unknown,
    /// `loadingFinished.encodedDataLength`, from which the headers' size is subtracted.
    Loaded(f64),
}

/// What `goto` learns about the committed document's navigation request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Lookup {
    /// No navigation request: `about:blank`, a `data:` URL, or a document without one.
    NoRequest,
    /// The request failed before its response: Playwright's `response()` is `null`.
    Failed,
    /// The final hop's response is known.
    Response(HopId),
    /// The final hop has neither a response nor a failure yet.
    Pending,
}

/// A request and every hop before it, as the SDK builds them: Playwright's `__create__`
/// messages for each hop's `Request` and `Response`, oldest first.
#[derive(Debug, Serialize)]
pub(crate) struct CreatedChain {
    pub(crate) requests: Vec<Created<RequestInitializer>>,
    /// One per request; `None` for a hop without a response yet.
    pub(crate) responses: Vec<Option<Created<ResponseInitializer>>>,
}

/// One object's guid and initializer.
#[derive(Debug, Serialize)]
pub(crate) struct Created<I> {
    pub(crate) guid: String,
    pub(crate) initializer: I,
}

/// `request.timing()` as Playwright's client holds it: the response's timing, then
/// `responseEnd` from its `requestFinished` or `requestFailed` event.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientTiming {
    pub(crate) start_time: f64,
    pub(crate) domain_lookup_start: f64,
    pub(crate) domain_lookup_end: f64,
    pub(crate) connect_start: f64,
    pub(crate) secure_connection_start: f64,
    pub(crate) connect_end: f64,
    pub(crate) request_start: f64,
    pub(crate) response_start: f64,
    pub(crate) response_end: f64,
}

/// A response body to read: the request id for `Network.getResponseBody`, on the session that
/// reported the request's end, and what Playwright's refetch of an empty body needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BodyRead {
    pub(crate) request_id: String,
    /// `None` for the tab's own session.
    pub(crate) session: Option<SessionId>,
    /// Set when an empty body may be refetched (`createResponseBodyCallback`): a `GET` of a
    /// static resource type, or a prefetch.
    pub(crate) refetch: Option<Refetch>,
}

/// What `Network.loadNetworkResource` needs to load a body again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Refetch {
    pub(crate) url: String,
    pub(crate) frame: String,
    /// The response's `content-length`: a body is refetched only when it says there is one.
    pub(crate) content_length: Option<String>,
}

/// What one event did that Playwright's server reports as a context event
/// (`FrameManager.requestStarted`, `requestReceivedResponse`, `reportRequestFinished`,
/// `requestFailed`), in the order it reports them. Favicons never appear.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Happening {
    Request(HopId),
    Response(HopId),
    /// The hop finished loading, redirected, or was a subresource 204 Chromium aborted.
    Finished(HopId),
    Failed(HopId),
}

/// The session's record, shared by its watcher (the only writer) and every call and getter.
pub(crate) struct NetworkLog {
    main_frame: String,
    log: Mutex<Log>,
    /// Bumped after every change, so a waiter rechecks; never read for its value.
    changes: watch::Sender<u64>,
    /// The out-of-process frame sessions whose Network events are this tab's, each enabled by
    /// `PageSession::enable_network`.
    children: Mutex<HashSet<SessionId>>,
    /// Set once the tab's `Network.enable` answered: frames attached later turn theirs on.
    enabled: AtomicBool,
}

#[derive(Default)]
struct Log {
    chains: VecDeque<Chain>,
    orphans: VecDeque<Orphan>,
    /// How many times the watcher lagged since the session started.
    lags: u64,
    /// `page.requests()`: every hop's request in the order it started, favicons left out,
    /// Playwright's last 100 or so.
    started: VecDeque<HopId>,
}

/// Extra info for a request id no chain holds yet.
struct Orphan {
    request: String,
    extra: Extra,
}

enum Extra {
    Request(Vec<NameValue>),
    Response(ResponseExtra),
    ServedFromCache,
}

/// `Network.responseReceivedExtraInfo`'s raw headers and their text's length.
struct ResponseExtra {
    headers: Vec<NameValue>,
    headers_size: i64,
}

/// One request id: its hops and `ResponseExtraInfoTracker`'s `RequestInfo`.
struct Chain {
    request: String,
    /// `requestId` is `loaderId` and the type is `Document`: Playwright's `isNavigationRequest`.
    navigation: bool,
    /// `None` for the tab's own session: the out-of-process frame session that reported the
    /// request's last event (`_maybeUpdateRequestSession`), where its body is read.
    session: Option<SessionId>,
    hops: Vec<Hop>,
    /// `loadingFinished` or `loadingFailed` arrived: Playwright deletes the request then, and
    /// later request events for its id start nothing here. Extra info still pairs up.
    ended: bool,
    request_extra: Vec<Option<Vec<NameValue>>>,
    response_extra: Vec<Option<ResponseExtra>>,
    /// Hops whose response expects extra info, in arrival order (the tracker's `responses`):
    /// the i-th one pairs with the i-th request and response extra info.
    expecting: Vec<usize>,
    served_from_cache: bool,
    /// What a watcher lag since the chain began may have cost it.
    lag: Lag,
}

/// A watcher lag that may have lost one of a chain's events.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Lag {
    #[default]
    None,
    /// The watcher lagged after the chain's answer: a value still pending may never arrive.
    AfterAnswer,
    /// The chain may miss events before its answer: the watcher lagged while its last hop had
    /// neither a response nor a failure, or the chain began with a redirect continuation, so a
    /// lag lost its first hops. Even an answer that arrives later may pair with the wrong hop.
    BeforeAnswer,
}

/// One request of a chain: Playwright's server `Request`, plus what its client learned from
/// events.
struct Hop {
    url: String,
    /// `requestWillBeSent.frameId`.
    frame: String,
    /// `toResourceType(event.type)`.
    resource_type: &'static str,
    /// Playwright's `_isFavicon`: the URL ends in `/favicon.ico`, or an earlier hop's did.
    favicon: bool,
    method: String,
    headers: Vec<NameValue>,
    post_data: Option<Vec<u8>>,
    /// `requestWillBeSent.timestamp`, monotonic seconds.
    timestamp: f64,
    /// `requestWillBeSent.wallTime`, seconds since the epoch.
    wall_time: f64,
    /// `_rawRequestHeadersPromise`: extra info, or the provisional headers.
    raw_request_headers: Option<Vec<NameValue>>,
    response: Option<HopResponse>,
    /// `_responseEndTiming`: set by `_requestFinished`, else Playwright's -1.
    response_end: f64,
    /// The server's `Response._finishedPromise`: redirected, finished or failed after a response.
    done: bool,
    /// The client's `requestFinished` event: redirected or loading finished.
    finished: bool,
    /// The client's `requestFailed` event: the failure text.
    failure: Option<String>,
}

/// One hop's response: Playwright's server `Response` and its promises.
struct HopResponse {
    status: i64,
    status_text: String,
    headers: Vec<NameValue>,
    timing: ResourceTiming,
    from_service_worker: bool,
    protocol: Option<String>,
    server_addr: Option<RemoteAddr>,
    security_details: SecurityDetails,
    /// `_rawResponseHeadersPromise`.
    raw_headers: Option<Vec<NameValue>>,
    headers_size: HeadersSize,
    body_size: BodySize,
}

impl NetworkLog {
    /// `main_frame` is the tab's target id, which Chromium gives its main frame.
    pub(crate) fn new(main_frame: String) -> Self {
        let (changes, _) = watch::channel(0);
        Self {
            main_frame,
            log: Mutex::new(Log::default()),
            changes,
            children: Mutex::default(),
            enabled: AtomicBool::new(false),
        }
    }

    /// The tab's `Network.enable` answered.
    pub(crate) fn mark_enabled(&self) {
        self.enabled.store(true, Ordering::Release);
    }

    /// Whether the tab's requests are being recorded.
    pub(crate) fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Applies one event of the tab's session (`session` `None`) or of an out-of-process frame
    /// session ([`Self::add_child`]), and answers what Playwright's server would report for it.
    /// Any method outside `Network.*` is ignored.
    pub(crate) fn apply(
        &self,
        session: Option<&SessionId>,
        method: &str,
        params: &Value,
    ) -> Vec<Happening> {
        let Ok(mut log) = self.log.lock() else {
            return Vec::new();
        };
        let mut happenings = Vec::new();
        let changed = log.apply(session, method, params, &mut happenings);
        drop(log);
        if changed {
            self.changes
                .send_modify(|version| *version = version.wrapping_add(1));
        }
        happenings
    }

    /// From now on `session`'s Network events are this tab's: an out-of-process frame session
    /// whose `Network.enable` the page sent.
    pub(crate) fn add_child(&self, session: &SessionId) {
        if let Ok(mut children) = self.children.lock() {
            children.insert(session.clone());
        }
    }

    /// Whether `session` is an out-of-process frame session this log records.
    pub(crate) fn has_child(&self, session: &SessionId) -> bool {
        self.children
            .lock()
            .is_ok_and(|children| children.contains(session))
    }

    /// `frame`'s guid as the SDK names frames: the main frame's, or a child's under the tab.
    pub(crate) fn frame_guid(&self, frame: &str) -> String {
        if frame == self.main_frame {
            format!("frame@{}", self.main_frame)
        } else {
            format!("frame@{}/{frame}", self.main_frame)
        }
    }

    /// `page.requests()`: Playwright's recent requests of the page, oldest first.
    pub(crate) fn requests(&self) -> Result<Vec<RequestChannel>, LogError> {
        let target = self.main_frame.as_str();
        self.read(|log| {
            log.started
                .iter()
                .filter(|id| log.chain(&id.request).is_some())
                .map(|id| RequestChannel {
                    guid: id.request_guid(target),
                })
                .collect()
        })
    }

    /// The frame a request was made in, while the log holds it.
    pub(crate) fn frame_of(&self, request: &str) -> Option<String> {
        self.read(|log| {
            log.chain(request)
                .and_then(|chain| chain.hops.last())
                .map(|hop| hop.frame.clone())
        })
        .ok()
        .flatten()
    }

    /// The hop a redirect made of `id`, once it started: `request.redirectedTo()`.
    pub(crate) fn redirected_to(&self, id: &HopId) -> Result<Option<HopId>, LogError> {
        self.with_chain(id, "request", |chain| {
            Ok((id.hop + 1 < chain.hops.len()).then(|| HopId {
                request: id.request.clone(),
                hop: id.hop + 1,
            }))
        })
    }

    /// Whether `id` has its response yet: `request.existingResponse()`.
    pub(crate) fn has_response(&self, id: &HopId) -> Result<bool, LogError> {
        self.with_hop(id, "request", |hop| Ok(hop.response.is_some()))
    }

    /// One hop's `Request` record, as a `request` event carries it.
    pub(crate) fn created_request(
        &self,
        id: &HopId,
    ) -> Result<Created<RequestInitializer>, LogError> {
        let target = self.main_frame.clone();
        self.with_chain(id, "request", |chain| {
            chain
                .request_record(&target, id.hop, |frame| self.frame_guid(frame))
                .ok_or(LogError::Dropped("request"))
        })
    }

    /// One hop's `Response` record, as a `response` event carries it, or `None` before it.
    pub(crate) fn created_response(
        &self,
        id: &HopId,
    ) -> Result<Option<Created<ResponseInitializer>>, LogError> {
        let target = self.main_frame.clone();
        self.with_chain(id, "response", |chain| {
            Ok(chain.response_record(&target, id.hop))
        })
    }

    /// What `networkidle` needs of a hop that started: its frame, and whether it is an event
    /// stream, which never holds a frame's idleness.
    pub(crate) fn idle_facts(&self, id: &HopId) -> Option<(String, bool)> {
        self.with_hop(id, "request", |hop| {
            Ok((hop.frame.clone(), hop.resource_type == "eventsource"))
        })
        .ok()
    }

    /// `request._responseEndTiming`, which `requestFinished` and `requestFailed` events carry.
    pub(crate) fn response_end(&self, id: &HopId) -> Result<f64, LogError> {
        self.with_hop(id, "request", |hop| Ok(hop.response_end))
    }

    /// The watcher lagged: any event may be missing from here on, so every value still pending
    /// may never arrive.
    pub(crate) fn lagged(&self) {
        if let Ok(mut log) = self.log.lock() {
            log.lags += 1;
            for chain in &mut log.chains {
                if chain.lag != Lag::BeforeAnswer {
                    chain.lag = if chain.lookup() == Lookup::Pending {
                        Lag::BeforeAnswer
                    } else {
                        Lag::AfterAnswer
                    };
                }
            }
        }
        self.changes
            .send_modify(|version| *version = version.wrapping_add(1));
    }

    /// The lags counted so far; `goto` takes this before it navigates.
    pub(crate) fn lags(&self) -> u64 {
        self.log.lock().map_or(0, |log| log.lags)
    }

    /// The committed document's navigation request, as Playwright's goto reads it
    /// (`newDocument.request._finalRequest().response()`). A lag since `lags_before` turns an
    /// unanswered lookup into [`LogError::Lost`].
    pub(crate) fn lookup(&self, document: &str, lags_before: u64) -> Result<Lookup, LogError> {
        self.read(|log| {
            let chain = log.chain(document);
            let lookup = chain.map_or(Lookup::NoRequest, Chain::lookup);
            let lagged = log.lags > lags_before;
            match lookup {
                Lookup::NoRequest | Lookup::Pending if lagged => Err(LogError::Lost),
                _ if chain.is_some_and(|chain| chain.lag == Lag::BeforeAnswer) => {
                    Err(LogError::Lost)
                }
                lookup => Ok(lookup),
            }
        })?
    }

    /// Waits until `ready` answers, rechecking after every change. Callers race it against the
    /// page's closure and the run's end.
    pub(crate) async fn wait<T>(
        &self,
        mut ready: impl FnMut(&Self) -> Result<Option<T>, LogError>,
    ) -> Result<T, LogError> {
        let mut changes = self.changes.subscribe();
        loop {
            if let Some(value) = ready(self)? {
                return Ok(value);
            }
            if changes.changed().await.is_err() {
                return Err(LogError::Poisoned);
            }
        }
    }

    /// The `Request` and `Response` of every hop up to `id`, for the SDK to build.
    pub(crate) fn chain(&self, id: &HopId) -> Result<CreatedChain, LogError> {
        let target = self.main_frame.as_str();
        self.with_chain(id, "request", |chain| {
            let mut requests = Vec::new();
            let mut responses = Vec::new();
            for index in 0..=id.hop.min(chain.hops.len().saturating_sub(1)) {
                if let Some(request) =
                    chain.request_record(target, index, |frame| self.frame_guid(frame))
                {
                    requests.push(request);
                    responses.push(chain.response_record(target, index));
                }
            }
            Ok(CreatedChain {
                requests,
                responses,
            })
        })
    }

    /// `request.timing()` now.
    pub(crate) fn timing(&self, id: &HopId) -> Result<ClientTiming, LogError> {
        self.with_hop(id, "request", |hop| Ok(hop.client_timing()))
    }

    /// `request.failure()` now.
    pub(crate) fn failure(&self, id: &HopId) -> Result<Option<String>, LogError> {
        self.with_hop(id, "request", |hop| Ok(hop.failure.clone()))
    }

    /// Whether the client's `requestFinished` arrived, which resolves `response.finished()`.
    pub(crate) fn finished(&self, id: &HopId) -> Result<bool, LogError> {
        self.with_pending(id, "response", |hop| Ok(hop.finished.then_some(true)))
            .map(|finished| finished.unwrap_or(false))
    }

    /// `Request.response` once the hop settled: a response arrived, or it failed without one.
    pub(crate) fn response(&self, id: &HopId) -> Result<Option<Settled>, LogError> {
        let target = self.main_frame.clone();
        self.with_pending(id, "request", |hop| {
            Ok(if hop.response.is_some() {
                Some(Settled::Response(ResponseChannel {
                    guid: id.response_guid(&target),
                }))
            } else if hop.failure.is_some() {
                Some(Settled::NoResponse)
            } else {
                None
            })
        })
    }

    /// `Request.rawRequestHeaders` once known.
    pub(crate) fn raw_request_headers(
        &self,
        id: &HopId,
    ) -> Result<Option<Vec<NameValue>>, LogError> {
        self.with_pending(id, "request", |hop| Ok(hop.raw_request_headers.clone()))
    }

    /// `Response.rawResponseHeaders` once known.
    pub(crate) fn raw_response_headers(
        &self,
        id: &HopId,
    ) -> Result<Option<Vec<NameValue>>, LogError> {
        self.with_pending(id, "response", |hop| {
            Ok(hop
                .response
                .as_ref()
                .and_then(|response| response.raw_headers.clone()))
        })
    }

    /// `Response.securityDetails`: always an object for Chromium, `{}` without TLS.
    pub(crate) fn security_details(&self, id: &HopId) -> Result<SecurityDetails, LogError> {
        self.with_response(id, |response| Ok(response.security_details.clone()))
    }

    /// `Response.serverAddr`.
    pub(crate) fn server_addr(&self, id: &HopId) -> Result<Option<RemoteAddr>, LogError> {
        self.with_response(id, |response| Ok(response.server_addr.clone()))
    }

    /// `Response.httpVersion` (`internalHttpVersion`).
    pub(crate) fn http_version(&self, id: &HopId) -> Result<String, LogError> {
        self.with_response(id, |response| {
            Ok(match response.protocol.as_deref() {
                None | Some("" | "http/1.1") => "HTTP/1.1".to_string(),
                Some("h2") => "HTTP/2.0".to_string(),
                Some(other) => other.to_string(),
            })
        })
    }

    /// `Response.sizes` (`internalSizes`) once every size it awaits is known.
    pub(crate) fn sizes(&self, id: &HopId) -> Result<Option<RequestSizes>, LogError> {
        self.with_pending(id, "response", Hop::sizes)
    }

    /// What `Response.body` reads, once the response is done (`internalBody` awaits its
    /// `_finishedPromise`); a redirect's body is refused as Playwright refuses it.
    pub(crate) fn body(&self, id: &HopId) -> Result<Option<BodyRead>, LogError> {
        self.with_chain(id, "response", |chain| {
            let hop = chain
                .hops
                .get(id.hop)
                .ok_or(LogError::Dropped("response"))?;
            let read = (|| {
                let Some(response) = &hop.response else {
                    return Ok(None);
                };
                if !hop.done {
                    return Ok(None);
                }
                if REDIRECT_STATUSES.contains(&response.status) {
                    return Err(LogError::RedirectBody);
                }
                Ok(Some(BodyRead {
                    request_id: id.request.clone(),
                    session: chain.session.clone(),
                    refetch: hop.refetch(response),
                }))
            })()?;
            match read {
                None if chain.lag != Lag::None => Err(LogError::Lost),
                read => Ok(read),
            }
        })
    }

    fn read<T>(&self, read: impl FnOnce(&Log) -> T) -> Result<T, LogError> {
        self.log
            .lock()
            .map(|log| read(&log))
            .map_err(|_| LogError::Poisoned)
    }

    fn with_chain<T>(
        &self,
        id: &HopId,
        what: &'static str,
        read: impl FnOnce(&Chain) -> Result<T, LogError>,
    ) -> Result<T, LogError> {
        self.read(|log| {
            log.chain(&id.request)
                .filter(|chain| id.hop < chain.hops.len())
                .map_or(Err(LogError::Dropped(what)), read)
        })?
    }

    fn with_hop<T>(
        &self,
        id: &HopId,
        what: &'static str,
        read: impl FnOnce(&Hop) -> Result<T, LogError>,
    ) -> Result<T, LogError> {
        self.with_chain(id, what, |chain| {
            chain
                .hops
                .get(id.hop)
                .map_or(Err(LogError::Dropped(what)), read)
        })
    }

    /// A hop's value that may still be pending: `None` waits for the next event, unless the
    /// watcher lagged since the chain began, which may have lost that event for good.
    fn with_pending<T>(
        &self,
        id: &HopId,
        what: &'static str,
        read: impl FnOnce(&Hop) -> Result<Option<T>, LogError>,
    ) -> Result<Option<T>, LogError> {
        self.with_chain(id, what, |chain| {
            let hop = chain.hops.get(id.hop).ok_or(LogError::Dropped(what))?;
            match read(hop)? {
                None if chain.lag != Lag::None => Err(LogError::Lost),
                value => Ok(value),
            }
        })
    }

    fn with_response<T>(
        &self,
        id: &HopId,
        read: impl FnOnce(&HopResponse) -> Result<T, LogError>,
    ) -> Result<T, LogError> {
        self.with_hop(id, "response", |hop| {
            hop.response
                .as_ref()
                .map_or(Err(LogError::Dropped("response")), read)
        })
    }
}

impl Log {
    fn chain(&self, request: &str) -> Option<&Chain> {
        // From the newest: a request's events come soon after it starts.
        self.chains
            .iter()
            .rev()
            .find(|chain| chain.request == request)
    }

    fn chain_mut(&mut self, request: &str) -> Option<&mut Chain> {
        self.chains
            .iter_mut()
            .rev()
            .find(|chain| chain.request == request)
    }

    /// Applies one event of `session` (`None`: the tab's own); whether anything changed. What
    /// Playwright's server reports for it goes to `happenings`.
    fn apply(
        &mut self,
        session: Option<&SessionId>,
        method: &str,
        params: &Value,
        happenings: &mut Vec<Happening>,
    ) -> bool {
        match method {
            "Network.requestWillBeSent" => {
                decode(params).is_some_and(|event: RequestWillBeSent| {
                    self.request_will_be_sent(session, &event, happenings)
                })
            }
            "Network.responseReceived" => decode(params)
                .is_some_and(|event: ResponseReceived| self.response_received(&event, happenings)),
            "Network.requestWillBeSentExtraInfo" => {
                decode(params).is_some_and(|event: RequestExtraInfo| {
                    let headers = headers_to_array(&event.headers, Some('\n'));
                    self.extra(event.request_id, Extra::Request(headers))
                })
            }
            "Network.responseReceivedExtraInfo" => {
                decode(params).is_some_and(|event: ResponseExtraInfo| {
                    let extra = ResponseExtra {
                        headers: headers_to_array(&event.headers, Some('\n')),
                        headers_size: event.headers_text.map_or(0, |text| utf16_len(&text)),
                    };
                    self.extra(event.request_id, Extra::Response(extra))
                })
            }
            "Network.requestServedFromCache" => decode(params).is_some_and(|event: RequestId| {
                self.extra(event.request_id, Extra::ServedFromCache)
            }),
            "Network.loadingFinished" => decode(params).is_some_and(|event: LoadingFinished| {
                self.loading_finished(session, &event, happenings)
            }),
            "Network.loadingFailed" => decode(params).is_some_and(|event: LoadingFailed| {
                self.loading_failed(session, event, happenings)
            }),
            _ => false,
        }
    }

    /// `_onRequestWillBeSent` and `_onRequest`, then `FrameManager.requestStarted`: a request
    /// starts a chain, and a redirect ends its chain's last hop (`_handleRequestRedirect`, which
    /// reports that hop's response and its finish) before the next hop starts.
    fn request_will_be_sent(
        &mut self,
        session: Option<&SessionId>,
        event: &RequestWillBeSent,
        happenings: &mut Vec<Happening>,
    ) -> bool {
        // `_onRequest` drops `data:` URLs, and a page's request it finds no frame for.
        if event.request.url.starts_with("data:") {
            return false;
        }
        let Some(frame) = event.frame_id.as_deref().filter(|frame| !frame.is_empty()) else {
            return false;
        };
        let navigation =
            event.request_id == event.loader_id && event.kind.as_deref() == Some("Document");
        let continued = self
            .chain_mut(&event.request_id)
            .filter(|chain| !chain.ended)
            .map(|chain| {
                let favicon = chain.hops.last().is_some_and(|hop| hop.favicon);
                if let Some(redirect) = &event.redirect_response
                    && let Some(index) =
                        chain.redirect(redirect, event.timestamp, event.redirect_has_extra_info)
                    && !favicon
                {
                    happenings.push(Happening::Response(chain.hop_id(index)));
                    happenings.push(Happening::Finished(chain.hop_id(index)));
                }
                let hop = Hop::new(event, frame, favicon);
                let started = (!hop.favicon).then(|| chain.hop_id(chain.hops.len()));
                chain.hops.push(hop);
                started
            });
        let started = match continued {
            Some(started) => started,
            None => {
                let mut chain = Chain::new(event.request_id.clone(), navigation, session.cloned());
                if event.redirect_response.is_some() {
                    // A redirect with no chain to continue: the request's start was never seen.
                    chain.lag = Lag::BeforeAnswer;
                }
                let hop = Hop::new(event, frame, false);
                let started = (!hop.favicon).then(|| chain.hop_id(0));
                chain.hops.push(hop);
                for orphan in self.take_orphans(&chain.request) {
                    chain.extra(orphan);
                }
                self.chains
                    .retain(|existing| existing.request != chain.request);
                self.chains.push_back(chain);
                self.evict(navigation);
                started
            }
        };
        if let Some(id) = started {
            self.record_start(id.clone());
            happenings.push(Happening::Request(id));
        }
        true
    }

    /// Drops the oldest chain past its kind's cap: navigations past [`MAX_CHAINS`], oldest
    /// first, as ever; other requests past [`MAX_REQUESTS`], the oldest that has ended first, so
    /// a request still loading keeps its record while finished ones can go.
    fn evict(&mut self, navigation: bool) {
        let (cap, ended_first) = if navigation {
            (MAX_CHAINS, false)
        } else {
            (MAX_REQUESTS, true)
        };
        let count = self
            .chains
            .iter()
            .filter(|chain| chain.navigation == navigation)
            .count();
        if count <= cap {
            return;
        }
        let index = ended_first
            .then(|| {
                self.chains
                    .iter()
                    .position(|chain| chain.navigation == navigation && chain.ended)
            })
            .flatten()
            .or_else(|| {
                self.chains
                    .iter()
                    .position(|chain| chain.navigation == navigation)
            });
        if let Some(index) = index {
            self.chains.remove(index);
        }
    }

    /// `Page.addNetworkRequest` with `ensureArrayLimit`: past the limit, the oldest tenth goes.
    fn record_start(&mut self, id: HopId) {
        self.started.push_back(id);
        if self.started.len() > MAX_PAGE_REQUESTS {
            // Playwright: `server/page.ts` `ensureArrayLimit`: `return array.splice(0, limit / 10);`
            self.started.drain(..MAX_PAGE_REQUESTS / 10);
        }
    }

    /// `_onResponseReceived` and `FrameManager.requestReceivedResponse`: the response of the
    /// request's latest hop.
    fn response_received(
        &mut self,
        event: &ResponseReceived,
        happenings: &mut Vec<Happening>,
    ) -> bool {
        let Some(chain) = self
            .chain_mut(&event.request_id)
            .filter(|chain| !chain.ended)
        else {
            return false;
        };
        let Some(index) = chain.hops.len().checked_sub(1) else {
            return false;
        };
        chain.respond(index, &event.response, event.has_extra_info);
        if chain.hops.get(index).is_some_and(|hop| !hop.favicon) {
            happenings.push(Happening::Response(chain.hop_id(index)));
        }
        true
    }

    /// `_onLoadingFinished` and `FrameManager.reportRequestFinished`.
    fn loading_finished(
        &mut self,
        session: Option<&SessionId>,
        event: &LoadingFinished,
        happenings: &mut Vec<Happening>,
    ) -> bool {
        let Some(chain) = self
            .chain_mut(&event.request_id)
            .filter(|chain| !chain.ended)
        else {
            return false;
        };
        chain.ended = true;
        chain.follow(session);
        let Some(index) = chain.hops.len().checked_sub(1) else {
            return true;
        };
        let id = chain.hop_id(index);
        let Some(hop) = chain.hops.get_mut(index) else {
            return true;
        };
        if let Some(response) = &mut hop.response {
            response.body_size = BodySize::Loaded(event.encoded_data_length);
            let end = seconds_to_roundish_millis(event.timestamp - hop.timestamp);
            hop.request_finished(end);
        }
        hop.finished = true;
        if !hop.favicon {
            happenings.push(Happening::Finished(id));
        }
        true
    }

    /// `_onLoadingFailed`, then `FrameManager.requestFailed`. Chromium aborts a 204 response: a
    /// subresource's then finishes (`reportRequestFinished`), and a navigation's fails as every
    /// other failure does, since a 204 navigation never commits.
    fn loading_failed(
        &mut self,
        session: Option<&SessionId>,
        event: LoadingFailed,
        happenings: &mut Vec<Happening>,
    ) -> bool {
        let Some(chain) = self
            .chain_mut(&event.request_id)
            .filter(|chain| !chain.ended)
        else {
            return false;
        };
        chain.ended = true;
        chain.follow(session);
        let navigation = chain.navigation;
        let Some(index) = chain.hops.len().checked_sub(1) else {
            return true;
        };
        let id = chain.hop_id(index);
        let Some(hop) = chain.hops.get_mut(index) else {
            return true;
        };
        if let Some(response) = &mut hop.response {
            response.body_size = BodySize::Unknown;
            let end = seconds_to_roundish_millis(event.timestamp - hop.timestamp);
            hop.request_finished(end);
        } else {
            hop.raw_request_headers
                .get_or_insert_with(|| hop.headers.clone());
        }
        if !navigation
            && hop
                .response
                .as_ref()
                .is_some_and(|response| response.status == 204)
        {
            hop.finished = true;
            if !hop.favicon {
                happenings.push(Happening::Finished(id));
            }
            return true;
        }
        hop.failure = Some(
            event
                .error_text
                .filter(|text| !text.is_empty())
                .or(event.blocked_reason)
                .unwrap_or_default(),
        );
        if !hop.favicon {
            happenings.push(Happening::Failed(id));
        }
        true
    }

    /// Extra info for `request`: its chain's, or an orphan until the chain begins.
    fn extra(&mut self, request: String, extra: Extra) -> bool {
        if let Some(chain) = self.chain_mut(&request) {
            chain.extra(extra);
            return true;
        }
        self.orphans.push_back(Orphan { request, extra });
        while self.orphans.len() > MAX_ORPHANS {
            self.orphans.pop_front();
        }
        false
    }

    fn take_orphans(&mut self, request: &str) -> Vec<Extra> {
        let (taken, kept) = self
            .orphans
            .drain(..)
            .partition(|orphan| orphan.request == request);
        self.orphans = kept;
        taken
            .into_iter()
            .map(|orphan: Orphan| orphan.extra)
            .collect()
    }
}

impl Chain {
    fn new(request: String, navigation: bool, session: Option<SessionId>) -> Self {
        Self {
            request,
            navigation,
            session,
            hops: Vec::new(),
            ended: false,
            request_extra: Vec::new(),
            response_extra: Vec::new(),
            expecting: Vec::new(),
            served_from_cache: false,
            lag: Lag::None,
        }
    }

    fn lookup(&self) -> Lookup {
        let Some((index, hop)) = self.hops.iter().enumerate().next_back() else {
            return Lookup::NoRequest;
        };
        if hop.response.is_some() {
            Lookup::Response(HopId {
                request: self.request.clone(),
                hop: index,
            })
        } else if hop.failure.is_some() {
            Lookup::Failed
        } else {
            Lookup::Pending
        }
    }

    fn hop_id(&self, hop: usize) -> HopId {
        HopId {
            request: self.request.clone(),
            hop,
        }
    }

    /// `_maybeUpdateRequestSession`: a navigation request that started in its parent's session
    /// and ends in the out-of-process frame's moves there, where its body is read.
    fn follow(&mut self, session: Option<&SessionId>) {
        if let Some(session) = session
            && self.navigation
        {
            self.session = Some(session.clone());
        }
    }

    /// The `__create__` record of the hop at `index`'s `Request` (`RequestDispatcher`'s
    /// initializer), its frame named by `frame_guid`.
    fn request_record(
        &self,
        target: &str,
        index: usize,
        frame_guid: impl Fn(&str) -> String,
    ) -> Option<Created<RequestInitializer>> {
        let hop = self.hops.get(index)?;
        let redirected_from = index.checked_sub(1).map(|previous| RequestChannel {
            guid: self.hop_id(previous).request_guid(target),
        });
        Some(Created {
            guid: self.hop_id(index).request_guid(target),
            initializer: RequestInitializer {
                frame: Some(FrameChannel {
                    guid: frame_guid(&hop.frame),
                }),
                service_worker: None,
                url: hop.url.clone(),
                resource_type: hop.resource_type.to_string(),
                method: hop.method.clone(),
                post_data: hop.post_data.as_ref().map(|bytes| STANDARD.encode(bytes)),
                headers: hop.headers.clone(),
                is_navigation_request: self.navigation,
                redirected_from,
            },
        })
    }

    /// The hop at `index`'s `Response` record, once it has one.
    fn response_record(&self, target: &str, index: usize) -> Option<Created<ResponseInitializer>> {
        let hop = self.hops.get(index)?;
        let at = self.hop_id(index);
        hop.response.as_ref().map(|response| Created {
            guid: at.response_guid(target),
            initializer: ResponseInitializer {
                request: RequestChannel {
                    guid: at.request_guid(target),
                },
                url: hop.url.clone(),
                status: response.status,
                status_text: response.status_text.clone(),
                headers: response.headers.clone(),
                timing: response.timing.clone(),
                from_service_worker: response.from_service_worker,
            },
        })
    }

    /// `_handleRequestRedirect`: the redirect response finishes the previous hop, whose index
    /// this answers.
    fn redirect(
        &mut self,
        payload: &ResponsePayload,
        timestamp: f64,
        has_extra_info: bool,
    ) -> Option<usize> {
        let index = self.hops.len().checked_sub(1)?;
        self.respond(index, payload, has_extra_info);
        let hop = self.hops.get_mut(index)?;
        if let Some(response) = &mut hop.response {
            response.body_size = BodySize::Unknown;
        }
        let end = (timestamp - hop.timestamp) * 1000.0;
        hop.request_finished(end);
        hop.finished = true;
        // `_requestFinished` starts an unmeasured request at its end, and a redirect's response
        // reaches the client only after it (`_handleRequestRedirect`), so its snapshot has that.
        let end = hop.response_end;
        if let Some(response) = &mut hop.response
            && is_unset(response.timing.request_start)
        {
            response.timing.request_start = end;
        }
        Some(index)
    }

    /// `_createResponse` and the tracker's `processResponse` for the hop at `index`.
    fn respond(&mut self, index: usize, payload: &ResponsePayload, has_extra_info: bool) {
        let served_from_cache = self.served_from_cache;
        let Some(hop) = self.hops.get_mut(index) else {
            return;
        };
        let timing = match &payload.timing {
            Some(timing) if !served_from_cache => ResourceTiming {
                start_time: (timing.request_time - hop.timestamp + hop.wall_time) * 1000.0,
                domain_lookup_start: timing.dns_start,
                domain_lookup_end: timing.dns_end,
                connect_start: timing.connect_start,
                secure_connection_start: timing.ssl_start,
                connect_end: timing.connect_end,
                request_start: timing.send_start,
                response_start: timing.receive_headers_end,
            },
            _ => ResourceTiming {
                start_time: hop.wall_time * 1000.0,
                domain_lookup_start: -1.0,
                domain_lookup_end: -1.0,
                connect_start: -1.0,
                secure_connection_start: -1.0,
                connect_end: -1.0,
                request_start: -1.0,
                response_start: -1.0,
            },
        };
        let headers = headers_to_array(&payload.headers, None);
        let server_addr = payload
            .remote_ip_address
            .as_ref()
            .filter(|address| !address.is_empty())
            .zip(payload.remote_port)
            .map(|(address, port)| RemoteAddr {
                ip_address: address.clone(),
                port,
            });
        let security = payload.security_details.as_ref();
        let mut response = HopResponse {
            status: payload.status,
            status_text: payload.status_text.clone(),
            headers,
            timing,
            from_service_worker: payload.from_service_worker,
            protocol: payload.protocol.clone(),
            server_addr,
            security_details: SecurityDetails {
                protocol: security.and_then(|details| details.protocol.clone()),
                subject_name: security.and_then(|details| details.subject_name.clone()),
                issuer: security.and_then(|details| details.issuer.clone()),
                valid_from: security.and_then(|details| details.valid_from),
                valid_to: security.and_then(|details| details.valid_to),
            },
            raw_headers: None,
            headers_size: HeadersSize::Pending,
            body_size: BodySize::Pending,
        };
        // Cached responses carry an erroneous `hasExtraInfo` (crbug.com/1340398): the
        // provisional headers stand in for the raw ones.
        if !has_extra_info || served_from_cache {
            hop.raw_request_headers
                .get_or_insert_with(|| hop.headers.clone());
            response.headers_size = HeadersSize::Unknown;
            response.raw_headers = Some(response.headers.clone());
            hop.response = Some(response);
            return;
        }
        hop.response = Some(response);
        self.expecting.push(index);
        self.patch(self.expecting.len() - 1);
    }

    fn extra(&mut self, extra: Extra) {
        match extra {
            Extra::Request(headers) => {
                self.request_extra.push(Some(headers));
                self.patch(self.request_extra.len() - 1);
            }
            Extra::Response(extra) => {
                self.response_extra.push(Some(extra));
                self.patch(self.response_extra.len() - 1);
            }
            Extra::ServedFromCache => self.served_from_cache = true,
        }
    }

    /// The tracker's `_patchHeaders`: pairs the i-th response expecting extra info with the
    /// i-th request and response extra info, each used once.
    fn patch(&mut self, index: usize) {
        let Some(&hop_index) = self.expecting.get(index) else {
            return;
        };
        let Some(hop) = self.hops.get_mut(hop_index) else {
            return;
        };
        if let Some(headers) = self.request_extra.get_mut(index).and_then(Option::take) {
            hop.raw_request_headers.get_or_insert(headers);
        }
        if let Some(extra) = self.response_extra.get_mut(index).and_then(Option::take)
            && let Some(response) = &mut hop.response
        {
            if response.headers_size == HeadersSize::Pending {
                response.headers_size = HeadersSize::Known(extra.headers_size);
            }
            response.raw_headers.get_or_insert(extra.headers);
        }
    }
}

impl Hop {
    /// The hop `event` starts in `frame`; `favicon` carries an earlier hop's `_isFavicon`.
    fn new(event: &RequestWillBeSent, frame: &str, favicon: bool) -> Self {
        let entries = event
            .request
            .post_data_entries
            .as_deref()
            .unwrap_or_default();
        let bytes: Vec<Vec<u8>> = entries
            .iter()
            .filter_map(|entry| entry.bytes.as_deref())
            .filter(|bytes| !bytes.is_empty())
            .filter_map(|bytes| STANDARD.decode(bytes).ok())
            .collect();
        Self {
            url: strip_fragment(&event.request.url).to_string(),
            frame: frame.to_string(),
            resource_type: resource_type(event.kind.as_deref().unwrap_or("Other")),
            favicon: favicon || event.request.url.ends_with("/favicon.ico"),
            method: event.request.method.clone(),
            headers: headers_to_array(&event.request.headers, None),
            post_data: (!bytes.is_empty()).then(|| bytes.concat()),
            timestamp: event.timestamp,
            wall_time: event.wall_time,
            raw_request_headers: None,
            response: None,
            response_end: -1.0,
            done: false,
            finished: false,
            failure: None,
        }
    }

    /// What `createResponseBodyCallback` needs to load an empty body again: only a `GET` of a
    /// static resource type, or a prefetch, is refetched.
    fn refetch(&self, response: &HopResponse) -> Option<Refetch> {
        if self.method != "GET" {
            return None;
        }
        let headers = self.raw_request_headers.as_ref().unwrap_or(&self.headers);
        let prefetch = headers.iter().any(|header| {
            header.name.eq_ignore_ascii_case("sec-purpose") && header.value.starts_with("prefetch")
        });
        if !REFETCH_SAFE_TYPES.contains(&self.resource_type) && !prefetch {
            return None;
        }
        Some(Refetch {
            url: self.url.clone(),
            frame: self.frame.clone(),
            content_length: response
                .headers
                .iter()
                .find(|header| header.name.eq_ignore_ascii_case("content-length"))
                .map(|header| header.value.clone()),
        })
    }

    /// `Response._requestFinished`.
    fn request_finished(&mut self, response_end: f64) {
        let response_start = self
            .response
            .as_ref()
            .map_or(-1.0, |response| response.timing.response_start);
        self.response_end = response_end.max(response_start);
        self.done = true;
    }

    /// The client's `_timing`: its defaults, the response's timing, then `_setResponseEndTiming`
    /// once a `requestFinished` or `requestFailed` event arrived.
    fn client_timing(&self) -> ClientTiming {
        let mut timing = self.response.as_ref().map_or(
            ClientTiming {
                start_time: 0.0,
                domain_lookup_start: -1.0,
                domain_lookup_end: -1.0,
                connect_start: -1.0,
                secure_connection_start: -1.0,
                connect_end: -1.0,
                request_start: -1.0,
                response_start: -1.0,
                response_end: -1.0,
            },
            |response| {
                let timing = &response.timing;
                ClientTiming {
                    start_time: timing.start_time,
                    domain_lookup_start: timing.domain_lookup_start,
                    domain_lookup_end: timing.domain_lookup_end,
                    connect_start: timing.connect_start,
                    secure_connection_start: timing.secure_connection_start,
                    connect_end: timing.connect_end,
                    request_start: timing.request_start,
                    response_start: timing.response_start,
                    response_end: -1.0,
                }
            },
        );
        if self.finished || self.failure.is_some() {
            timing.response_end = self.response_end;
            if is_unset(timing.response_start) {
                timing.response_start = self.response_end;
            }
        }
        timing
    }

    /// `internalSizes`, once the raw request headers, the header size and the body size are
    /// known (`None` while one is pending); a hop without a response has none (the client throws
    /// first). A size Playwright cannot compute is an error, as its server throws there.
    fn sizes(&self) -> Result<Option<RequestSizes>, LogError> {
        let (Some(response), Some(raw_request_headers)) =
            (self.response.as_ref(), self.raw_request_headers.as_ref())
        else {
            return Ok(None);
        };
        let request_headers_size =
            request_headers_size(raw_request_headers, &self.url, &self.method)?;
        let response_headers_size = match response.headers_size {
            HeadersSize::Pending => return Ok(None),
            HeadersSize::Known(size) => size,
            HeadersSize::Unknown => {
                let Some(raw) = response.raw_headers.as_ref() else {
                    return Ok(None);
                };
                response_headers_size(raw, &response.status_text)
            }
        };
        let response_body_size = match response.body_size {
            BodySize::Pending => return Ok(None),
            BodySize::Loaded(length) => js_int(length - js_number(response_headers_size))?,
            BodySize::Unknown => {
                let Some(raw) = response.raw_headers.as_ref() else {
                    return Ok(None);
                };
                // `contentLength ? +contentLength : 0`: an empty value is 0, any other is
                // JavaScript's `Number` of it.
                match raw
                    .iter()
                    .find(|header| header.name.eq_ignore_ascii_case("content-length"))
                {
                    Some(header) if !header.value.is_empty() => {
                        js_int(js_string_to_number(&header.value))?
                    }
                    _ => 0,
                }
            }
        };
        let request_body_size = self
            .post_data
            .as_ref()
            .map_or(0, |bytes| i64::try_from(bytes.len()).unwrap_or(i64::MAX));
        Ok(Some(RequestSizes {
            request_body_size,
            request_headers_size,
            response_body_size,
            response_headers_size,
        }))
    }
}

/// `toResourceType` (`server/chromium/crNetworkManager.ts`): CDP's resource type as Playwright's.
fn resource_type(kind: &str) -> &'static str {
    match kind {
        "Document" => "document",
        "Stylesheet" => "stylesheet",
        "Image" => "image",
        "Media" => "media",
        "Font" => "font",
        "Script" => "script",
        "TextTrack" => "texttrack",
        "XHR" => "xhr",
        "Fetch" => "fetch",
        "EventSource" => "eventsource",
        "WebSocket" => "websocket",
        "Manifest" => "manifest",
        "Ping" => "ping",
        "CSPViolationReport" => "cspreport",
        _ => "other",
    }
}

/// `requestHeadersSize` (`server/network.ts`): the request line plus every header line, in
/// JavaScript string lengths. A URL that does not parse is an error, where `new URL` throws; that
/// is a `TypeError` in Playwright, a plain `Error` here, since Chromium's canonical navigation URLs
/// always parse.
fn request_headers_size(headers: &[NameValue], url: &str, method: &str) -> Result<i64, LogError> {
    let path = Url::parse(url)
        .map_err(|_| LogError::Invalid("Invalid URL"))?
        .path()
        .to_string();
    Ok(4 + utf16_len(method) + utf16_len(&path) + 8 + headers_size(headers))
}

/// `responseHeadersSize` (`server/network.ts`).
fn response_headers_size(headers: &[NameValue], status_text: &str) -> i64 {
    4 + 8 + 3 + utf16_len(status_text) + headers_size(headers) + 2
}

fn headers_size(headers: &[NameValue]) -> i64 {
    headers
        .iter()
        .map(|header| utf16_len(&header.name) + utf16_len(&header.value) + 4)
        .sum()
}

/// Playwright's `=== -1`: a timing it never measured.
#[expect(
    clippy::float_cmp,
    reason = "-1 is an exact sentinel, never the result of arithmetic"
)]
fn is_unset(timing: f64) -> bool {
    timing == -1.0
}

/// A JavaScript string's `length`.
fn utf16_len(text: &str) -> i64 {
    i64::try_from(text.encode_utf16().count()).unwrap_or(i64::MAX)
}

/// An `i64` as the JavaScript number Playwright computes with.
#[expect(
    clippy::cast_precision_loss,
    reason = "header sizes stay far below 2^53, where JavaScript numbers are exact"
)]
fn js_number(value: i64) -> f64 {
    value as f64
}

/// A size Playwright's `tInt` validator accepts: an integral, finite number; any other rejects.
#[expect(
    clippy::cast_possible_truncation,
    reason = "checked integral and within i64 first"
)]
fn js_int(value: f64) -> Result<i64, LogError> {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 9.2e18 {
        Ok(value as i64)
    } else {
        Err(LogError::Invalid("sizes: expected integer, got float"))
    }
}

/// JavaScript's `Number(text)` (ECMAScript `StringToNumber`): trimmed of JavaScript white space,
/// empty is 0, `0x`/`0o`/`0b` integers, `Infinity`, and decimal literals; anything else is NaN.
pub(crate) fn js_string_to_number(text: &str) -> f64 {
    let text = text.trim_matches(is_js_whitespace);
    if text.is_empty() {
        return 0.0;
    }
    let radix = |digits: &str, radix: u32| {
        if digits.is_empty() {
            return f64::NAN;
        }
        digits
            .chars()
            .try_fold(0.0_f64, |value, digit| {
                digit
                    .to_digit(radix)
                    .map(|digit| value * f64::from(radix) + f64::from(digit))
            })
            .unwrap_or(f64::NAN)
    };
    let lower = text.get(..2).map(str::to_ascii_lowercase);
    match lower.as_deref() {
        Some("0x") => return radix(&text[2..], 16),
        Some("0o") => return radix(&text[2..], 8),
        Some("0b") => return radix(&text[2..], 2),
        _ => {}
    }
    match text {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    // Rust's parser also takes `inf`, `nan` and the like; JavaScript's decimal literal does not.
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

/// ECMAScript's `StrWhiteSpaceChar`: white space and line terminators.
fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\u{b}'
            | '\u{c}'
            | ' '
            | '\u{a0}'
            | '\u{feff}'
            | '\n'
            | '\r'
            | '\u{2028}'
            | '\u{2029}'
    ) || (c != '\u{85}' && c.is_whitespace() && !c.is_ascii_control())
}

/// `helper.secondsToRoundishMillis`: `((seconds * 1000000) | 0) / 1000`, whose `| 0` is
/// JavaScript's `ToInt32`.
fn seconds_to_roundish_millis(seconds: f64) -> f64 {
    f64::from(to_int32(seconds * 1_000_000.0)) / 1000.0
}

/// ECMAScript `ToInt32`: truncate, then wrap modulo 2^32 into the signed range.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "the value is integral and in [0, 2^32) before the cast, then reinterpreted"
)]
fn to_int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let wrapped = value.trunc().rem_euclid(4_294_967_296.0);
    (wrapped as u32) as i32
}

/// `stripFragmentFromUrl`.
fn strip_fragment(url: &str) -> &str {
    url.split_once('#').map_or(url, |(before, _)| before)
}

/// `headersObjectToArray`: one entry per name, or, with a separator, one per value split on it
/// and trimmed (`set-cookie` splits on the same separator here, as no caller passes another).
fn headers_to_array(headers: &Map<String, Value>, separator: Option<char>) -> Vec<NameValue> {
    let mut array = Vec::new();
    for (name, value) in headers {
        let Some(value) = value.as_str() else {
            continue;
        };
        match separator {
            Some(separator) => array.extend(value.split(separator).map(|value| NameValue {
                name: name.clone(),
                value: value.trim().to_string(),
            })),
            None => array.push(NameValue {
                name: name.clone(),
                value: value.to_string(),
            }),
        }
    }
    array
}

fn decode<T: for<'de> Deserialize<'de>>(params: &Value) -> Option<T> {
    T::deserialize(params).ok()
}

// Event shapes are local serde structs: browseros-cdp's typed surface lacks the Network domain,
// and adding it would change a shared crate that core also compiles.

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestWillBeSent {
    request_id: String,
    loader_id: String,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    frame_id: Option<String>,
    request: RequestPayload,
    timestamp: f64,
    wall_time: f64,
    #[serde(default)]
    redirect_response: Option<ResponsePayload>,
    #[serde(default)]
    redirect_has_extra_info: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestPayload {
    url: String,
    method: String,
    #[serde(default)]
    headers: Map<String, Value>,
    #[serde(default)]
    post_data_entries: Option<Vec<PostDataEntry>>,
}

#[derive(Deserialize)]
struct PostDataEntry {
    #[serde(default)]
    bytes: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseReceived {
    request_id: String,
    response: ResponsePayload,
    #[serde(default)]
    has_extra_info: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponsePayload {
    status: i64,
    #[serde(default)]
    status_text: String,
    #[serde(default)]
    headers: Map<String, Value>,
    #[serde(default)]
    timing: Option<TimingPayload>,
    #[serde(default)]
    from_service_worker: bool,
    #[serde(default)]
    protocol: Option<String>,
    #[serde(rename = "remoteIPAddress", default)]
    remote_ip_address: Option<String>,
    #[serde(default)]
    remote_port: Option<i64>,
    #[serde(default)]
    security_details: Option<SecurityPayload>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimingPayload {
    request_time: f64,
    dns_start: f64,
    dns_end: f64,
    connect_start: f64,
    ssl_start: f64,
    connect_end: f64,
    send_start: f64,
    receive_headers_end: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecurityPayload {
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    subject_name: Option<String>,
    #[serde(default)]
    issuer: Option<String>,
    #[serde(default)]
    valid_from: Option<f64>,
    #[serde(default)]
    valid_to: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestExtraInfo {
    request_id: String,
    #[serde(default)]
    headers: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseExtraInfo {
    request_id: String,
    #[serde(default)]
    headers: Map<String, Value>,
    #[serde(default)]
    headers_text: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestId {
    request_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoadingFinished {
    request_id: String,
    timestamp: f64,
    encoded_data_length: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoadingFailed {
    request_id: String,
    timestamp: f64,
    #[serde(default)]
    error_text: Option<String>,
    #[serde(default)]
    blocked_reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use serde_json::json;
    use tokio::task;

    use super::*;

    /// `tests/network-goldens.mjs`'s recording: each goto's CDP events and Playwright's answer.
    const GOLDENS: &str = include_str!("../tests/fixtures/network-goldens.json");

    struct Golden {
        name: String,
        main_frame: String,
        events: Vec<(String, Value)>,
        playwright: Value,
    }

    fn goldens() -> Vec<Golden> {
        let file: Value = serde_json::from_str(GOLDENS).expect("goldens parse");
        file["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .map(|case| Golden {
                name: case["name"].as_str().expect("name").to_string(),
                main_frame: case["mainFrame"].as_str().expect("main frame").to_string(),
                events: case["events"]
                    .as_array()
                    .expect("events")
                    .iter()
                    .map(|event| {
                        (
                            event["method"].as_str().expect("method").to_string(),
                            event["params"].clone(),
                        )
                    })
                    .collect(),
                playwright: case["playwright"].clone(),
            })
            .collect()
    }

    fn golden(name: &str) -> Golden {
        goldens()
            .into_iter()
            .find(|golden| golden.name == name)
            .expect("a recorded case")
    }

    fn replay(main_frame: &str, events: &[(String, Value)]) -> NetworkLog {
        let log = NetworkLog::new(main_frame.to_string());
        for (method, params) in events {
            log.apply(None, method, params);
        }
        log
    }

    /// The loader of the case's last main-frame commit: the document goto waited for.
    fn committed(golden: &Golden) -> Option<String> {
        golden
            .events
            .iter()
            .filter(|(method, params)| {
                method == "Page.frameNavigated" && params["frame"]["id"] == golden.main_frame
            })
            .filter_map(|(_, params)| params["frame"]["loaderId"].as_str())
            .next_back()
            .map(str::to_string)
    }

    /// Playwright's client `RawHeaders(headers).headers()`: lower-cased names, repeated values
    /// joined with `, ` (`\n` for `set-cookie`).
    fn headers_object(headers: &[NameValue]) -> Value {
        let mut joined: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for header in headers {
            joined
                .entry(header.name.to_lowercase())
                .or_default()
                .push(header.value.clone());
        }
        Value::Object(
            joined
                .into_iter()
                .map(|(name, values)| {
                    let separator = if name == "set-cookie" { "\n" } else { ", " };
                    let value = Value::String(values.join(separator));
                    (name, value)
                })
                .collect(),
        )
    }

    fn array(headers: &[NameValue]) -> Value {
        serde_json::to_value(headers).expect("headers encode")
    }

    /// Every number as an `f64`: JavaScript has one number type, so `-1` and `-1.0` are equal.
    fn numbers(value: Value) -> Value {
        match value {
            Value::Number(number) => json!(number.as_f64().expect("a finite number")),
            Value::Array(items) => Value::Array(items.into_iter().map(numbers).collect()),
            Value::Object(fields) => {
                Value::Object(fields.into_iter().map(|(k, v)| (k, numbers(v))).collect())
            }
            other => other,
        }
    }

    /// `startTime` adds the session's own `wallTime`, which Chromium stamps per CDP session (the
    /// recording's differs from Playwright's by microseconds); every other field comes from
    /// values both sessions share. `serde_json` (without `float_roundtrip`) parses a recorded
    /// double up to one ulp off, so fields compare within 1e-9 ms.
    fn assert_timing(actual: &ClientTiming, expected: &Value, what: &str) {
        let actual = numbers(serde_json::to_value(actual).expect("timing encodes"));
        let expected = numbers(expected.clone());
        let (Some(actual), Some(expected)) = (actual.as_object(), expected.as_object()) else {
            panic!("{what}: timing objects");
        };
        assert_eq!(
            actual.keys().collect::<Vec<_>>(),
            expected.keys().collect::<Vec<_>>(),
            "{what}: fields"
        );
        for (field, ours) in actual {
            let ours = ours.as_f64().expect("a number");
            let theirs = expected[field].as_f64().expect("a number");
            let tolerance = if field == "startTime" { 1.0 } else { 1e-9 };
            assert!(
                (ours - theirs).abs() < tolerance,
                "{what}: {field} {ours} vs Playwright's {theirs}"
            );
        }
    }

    #[test]
    fn tests_that_each_recorded_navigation_rebuilds_playwrights_requests_and_responses() {
        for name in [
            "ok",
            "redirects",
            "not-found",
            "json",
            "latin1",
            "commit-before-body",
        ] {
            let golden = golden(name);
            let log = replay(&golden.main_frame, &golden.events);
            let document = committed(&golden).expect("a commit");
            let Lookup::Response(id) = log.lookup(&document, 0).expect("lookup") else {
                panic!("{name}: the navigation has a response");
            };
            let hops = golden.playwright["hops"].as_array().expect("hops");
            assert_eq!(id.hop + 1, hops.len(), "{name}: one hop per redirect");
            let chain = log.chain(&id).expect("chain");
            for (index, expected) in hops.iter().enumerate() {
                let hop = HopId {
                    request: document.clone(),
                    hop: index,
                };
                let what = format!("{name} hop {index}");
                // The last hop is compared once it finished: its timing after the settle delay.
                let timing = if index + 1 < hops.len() {
                    &expected["request"]["timing"]
                } else {
                    &golden.playwright["timingAfterSettle"]
                };
                check_request(&log, &chain, &hop, &expected["request"], timing, &what);
                check_response(&log, &chain, &hop, &expected["response"], &what);
            }
        }
    }

    /// One hop's `Request` against what Playwright's answered for it.
    fn check_request(
        log: &NetworkLog,
        chain: &CreatedChain,
        hop: &HopId,
        want: &Value,
        timing: &Value,
        what: &str,
    ) {
        let request = &chain.requests[hop.hop].initializer;
        assert_eq!(json!(request.url), want["url"], "{what}");
        assert_eq!(json!(request.method), want["method"], "{what}");
        assert_eq!(json!(request.resource_type), want["resourceType"], "{what}");
        assert_eq!(
            json!(request.is_navigation_request),
            want["isNavigationRequest"],
            "{what}"
        );
        assert_eq!(headers_object(&request.headers), want["headers"], "{what}");
        assert_eq!(
            array(&log.raw_request_headers(hop).expect("hop").expect("raw")),
            want["headersArray"],
            "{what}: raw request headers"
        );
        assert_eq!(request.post_data, None, "{what}");
        assert_eq!(
            json!(log.failure(hop).expect("hop")),
            want["failure"],
            "{what}"
        );
        assert_eq!(
            numbers(
                serde_json::to_value(log.sizes(hop).expect("hop").expect("sizes"))
                    .expect("sizes encode")
            ),
            numbers(want["sizes"].clone()),
            "{what}: sizes"
        );
        assert_timing(&log.timing(hop).expect("hop"), timing, what);
    }

    /// One hop's `Response` against what Playwright's answered for it.
    fn check_response(
        log: &NetworkLog,
        chain: &CreatedChain,
        hop: &HopId,
        want: &Value,
        what: &str,
    ) {
        let response = &chain.responses[hop.hop]
            .as_ref()
            .expect("a response per hop")
            .initializer;
        assert_eq!(json!(response.url), want["url"], "{what}");
        assert_eq!(json!(response.status), want["status"], "{what}");
        assert_eq!(json!(response.status_text), want["statusText"], "{what}");
        assert_eq!(
            json!(response.status == 0 || (200..=299).contains(&response.status)),
            want["ok"],
            "{what}"
        );
        assert_eq!(headers_object(&response.headers), want["headers"], "{what}");
        assert_eq!(
            array(&log.raw_response_headers(hop).expect("hop").expect("raw")),
            want["headersArray"],
            "{what}: raw response headers"
        );
        assert_eq!(
            json!(response.from_service_worker),
            want["fromServiceWorker"],
            "{what}"
        );
        assert_eq!(
            json!(log.server_addr(hop).expect("hop")),
            want["serverAddr"],
            "{what}"
        );
        assert_eq!(
            json!(log.security_details(hop).expect("hop")),
            want["securityDetails"],
            "{what}"
        );
        assert_eq!(
            json!(log.http_version(hop).expect("hop")),
            want["httpVersion"],
            "{what}"
        );
        let body = log.body(hop).map(|read| read.map(|read| read.request_id));
        if REDIRECT_STATUSES.contains(&response.status) {
            assert_eq!(
                body,
                Err(LogError::RedirectBody),
                "{what}: {}",
                want["body"]
            );
        } else {
            assert_eq!(
                body,
                Ok(Some(hop.request.clone())),
                "{what}: {}",
                want["body"]
            );
        }
        assert!(log.finished(hop).expect("hop"), "{what}: finished");
    }

    #[test]
    fn tests_that_navigations_without_a_recorded_request_have_no_response() {
        for name in ["blank", "data"] {
            let golden = golden(name);
            assert_eq!(golden.playwright, Value::Null, "{name}: Playwright's null");
            let log = replay(&golden.main_frame, &golden.events);
            let document = committed(&golden).expect("a commit");
            assert_eq!(log.lookup(&document, 0), Ok(Lookup::NoRequest), "{name}");
        }
    }

    /// A 204 navigation: Chromium reports its response, then aborts the request. The response
    /// came first, so Playwright's `request.response()` resolves with it, and the later failure
    /// is the request's `failure()`; goto itself throws on `Page.navigate`'s `errorText`.
    #[test]
    fn tests_that_a_response_then_a_failure_keeps_the_response_and_records_the_failure() {
        let golden = golden("no-content");
        assert_eq!(
            golden.playwright["error"], "Error",
            "Playwright's goto throws"
        );
        let log = replay(&golden.main_frame, &golden.events);
        let (_, request) = golden
            .events
            .iter()
            .find(|(method, _)| method == "Network.requestWillBeSent")
            .expect("the request");
        let id = request["requestId"].as_str().expect("id").to_string();
        let hop = HopId {
            request: id.clone(),
            hop: 0,
        };
        assert_eq!(log.lookup(&id, 0), Ok(Lookup::Response(hop.clone())));
        assert_eq!(log.failure(&hop), Ok(Some("net::ERR_ABORTED".to_string())));
        assert!(
            !log.finished(&hop).expect("hop"),
            "a failure never finishes"
        );
        let timing = log.timing(&hop).expect("hop");
        assert!(timing.response_end >= timing.response_start, "{timing:?}");
        assert_eq!(
            log.sizes(&hop)
                .expect("hop")
                .map(|sizes| sizes.response_body_size),
            Some(0),
            "no encoded size after a failure, and no content-length"
        );
    }

    #[test]
    fn tests_that_a_failure_before_any_response_answers_no_response() {
        let log = replay(
            "MAIN",
            &[
                navigation("N", "MAIN", "Document", "https://a.test/"),
                event(
                    "Network.loadingFailed",
                    json!({"requestId": "N", "timestamp": 10.5, "errorText": "",
                           "blockedReason": "inspector"}),
                ),
            ],
        );
        let hop = HopId {
            request: "N".to_string(),
            hop: 0,
        };
        assert_eq!(log.lookup("N", 0), Ok(Lookup::Failed));
        assert_eq!(
            log.response(&hop),
            Ok(Some(Settled::NoResponse)),
            "Request.response is null"
        );
        assert_eq!(log.failure(&hop), Ok(Some("inspector".to_string())));
        assert_eq!(
            log.raw_request_headers(&hop)
                .expect("hop")
                .map(|headers| headers.len()),
            Some(1),
            "the provisional headers stand in"
        );
        let timing = log.timing(&hop).expect("hop");
        assert_eq!(
            (
                timing.start_time,
                timing.response_start,
                timing.response_end
            ),
            (0.0, -1.0, -1.0),
            "a request that failed before its response keeps the client's defaults"
        );
    }

    /// Moves the first event matching `moved` to just before the first matching `before`.
    fn reorder(
        events: &[(String, Value)],
        moved: &str,
        before: impl Fn(&str) -> bool,
    ) -> Vec<(String, Value)> {
        let mut events = events.to_vec();
        let from = events
            .iter()
            .position(|(method, _)| method == moved)
            .expect("the moved event");
        let event = events.remove(from);
        let to = events
            .iter()
            .position(|(method, _)| before(method))
            .expect("the anchor event");
        events.insert(to, event);
        events
    }

    #[test]
    fn tests_that_extra_info_before_its_request_still_gives_the_raw_headers() {
        let golden = golden("ok");
        let events = reorder(
            &golden.events,
            "Network.requestWillBeSentExtraInfo",
            |method| method == "Network.requestWillBeSent",
        );
        let events = reorder(&events, "Network.responseReceivedExtraInfo", |method| {
            method == "Network.requestWillBeSent"
        });
        let log = replay(&golden.main_frame, &events);
        let document = committed(&golden).expect("a commit");
        let hop = HopId {
            request: document,
            hop: 0,
        };
        let want = &golden.playwright["hops"][0];
        assert_eq!(
            array(&log.raw_request_headers(&hop).expect("hop").expect("raw")),
            want["request"]["headersArray"]
        );
        assert_eq!(
            array(&log.raw_response_headers(&hop).expect("hop").expect("raw")),
            want["response"]["headersArray"]
        );
    }

    #[test]
    fn tests_that_raw_headers_and_sizes_wait_for_extra_info_that_arrives_last() {
        let golden = golden("ok");
        let (late, early): (Vec<_>, Vec<_>) = golden
            .events
            .iter()
            .cloned()
            .partition(|(method, _)| method == "Network.responseReceivedExtraInfo");
        let log = replay(&golden.main_frame, &early);
        let hop = HopId {
            request: committed(&golden).expect("a commit"),
            hop: 0,
        };
        assert_eq!(log.raw_response_headers(&hop), Ok(None), "still pending");
        assert_eq!(log.sizes(&hop), Ok(None), "sizes await the header size");
        for (method, params) in &late {
            log.apply(None, method, params);
        }
        let want = &golden.playwright["hops"][0];
        assert_eq!(
            array(&log.raw_response_headers(&hop).expect("hop").expect("raw")),
            want["response"]["headersArray"]
        );
        assert_eq!(
            serde_json::to_value(log.sizes(&hop).expect("hop").expect("sizes")).expect("encode"),
            want["request"]["sizes"]
        );
    }

    #[test]
    fn tests_that_the_body_waits_until_the_response_is_done() {
        let golden = golden("commit-before-body");
        let finished = golden
            .events
            .iter()
            .position(|(method, _)| method == "Network.loadingFinished")
            .expect("loading finished");
        let log = replay(&golden.main_frame, &golden.events[..finished]);
        let hop = HopId {
            request: committed(&golden).expect("a commit"),
            hop: 0,
        };
        assert_eq!(log.body(&hop), Ok(None), "the body is still loading");
        assert!(!log.finished(&hop).expect("hop"));
        assert!(is_unset(log.timing(&hop).expect("hop").response_end));
        let (method, params) = &golden.events[finished];
        log.apply(None, method, params);
        assert_eq!(
            log.body(&hop),
            Ok(Some(BodyRead {
                request_id: hop.request.clone(),
                session: None,
                refetch: None,
            }))
        );
        assert!(log.finished(&hop).expect("hop"));
    }

    fn event(method: &str, params: Value) -> (String, Value) {
        (method.to_string(), params)
    }

    fn navigation(id: &str, frame: &str, kind: &str, url: &str) -> (String, Value) {
        event(
            "Network.requestWillBeSent",
            json!({
                "requestId": id, "loaderId": id, "type": kind, "frameId": frame,
                "request": {"url": url, "method": "GET", "headers": {"A": "1"}},
                "timestamp": 10.0, "wallTime": 1000.0,
            }),
        )
    }

    fn response(id: &str, has_extra_info: bool) -> (String, Value) {
        event(
            "Network.responseReceived",
            json!({
                "requestId": id, "type": "Document", "hasExtraInfo": has_extra_info,
                "response": {
                    "url": "https://a.test/", "status": 200, "statusText": "OK",
                    "headers": {"B": "2"}, "protocol": "h2",
                    "timing": {
                        "requestTime": 10.5, "dnsStart": 1.0, "dnsEnd": 2.0, "connectStart": 3.0,
                        "sslStart": 4.0, "connectEnd": 5.0, "sendStart": 6.0,
                        "receiveHeadersEnd": 7.0,
                    },
                },
            }),
        )
    }

    #[test]
    fn tests_that_every_framed_request_is_recorded_but_data_urls_and_frameless_ones() {
        let log = replay(
            "MAIN",
            &[
                navigation("CHILD", "OTHER", "Document", "https://a.test/frame"),
                navigation("SUB", "MAIN", "Script", "https://a.test/a.js"),
                event(
                    "Network.requestWillBeSent",
                    json!({
                        "requestId": "1.2", "loaderId": "MAIN-LOADER", "type": "Document",
                        "frameId": "MAIN", "request": {"url": "https://a.test/", "method": "GET"},
                        "timestamp": 1.0, "wallTime": 1.0,
                    }),
                ),
                navigation("DATA", "MAIN", "Document", "data:text/html,x"),
                event(
                    "Network.requestWillBeSent",
                    json!({
                        "requestId": "W.1", "loaderId": "W.1", "type": "Script",
                        "request": {"url": "https://a.test/worker.js", "method": "GET"},
                        "timestamp": 1.0, "wallTime": 1.0,
                    }),
                ),
            ],
        );
        for id in ["CHILD", "SUB", "1.2"] {
            assert_eq!(log.lookup(id, 0), Ok(Lookup::Pending), "{id} is recorded");
        }
        for id in ["MAIN-LOADER", "DATA", "W.1"] {
            assert_eq!(
                log.lookup(id, 0),
                Ok(Lookup::NoRequest),
                "{id} is not recorded"
            );
        }
        let created = |id: &str| {
            log.created_request(&HopId {
                request: id.to_string(),
                hop: 0,
            })
            .expect("a record")
            .initializer
        };
        let child = created("CHILD");
        assert!(
            child.is_navigation_request,
            "requestId is loaderId: navigation"
        );
        assert_eq!(child.frame.expect("a frame").guid, "frame@MAIN/OTHER");
        let script = created("SUB");
        assert_eq!(script.resource_type, "script");
        assert_eq!(script.frame.expect("a frame").guid, "frame@MAIN");
        let fetch = created("1.2");
        assert!(
            !fetch.is_navigation_request,
            "a Document whose requestId is not its loaderId is no navigation"
        );
        assert_eq!(
            log.requests()
                .expect("requests")
                .into_iter()
                .map(|request| request.guid)
                .collect::<Vec<_>>(),
            [
                "request@MAIN:CHILD:0",
                "request@MAIN:SUB:0",
                "request@MAIN:1.2:0"
            ],
            "page.requests() in start order"
        );
    }

    #[test]
    fn tests_that_a_cached_response_uses_provisional_headers_and_wall_time_timing() {
        let log = replay(
            "MAIN",
            &[
                navigation("N", "MAIN", "Document", "https://a.test/#top"),
                event("Network.requestServedFromCache", json!({"requestId": "N"})),
                response("N", true),
                event(
                    "Network.loadingFinished",
                    json!({"requestId": "N", "timestamp": 10.25, "encodedDataLength": 0}),
                ),
            ],
        );
        let hop = HopId {
            request: "N".to_string(),
            hop: 0,
        };
        let chain = log.chain(&hop).expect("chain");
        assert_eq!(chain.requests[0].initializer.url, "https://a.test/");
        assert_eq!(
            log.raw_request_headers(&hop).expect("hop"),
            Some(vec![NameValue {
                name: "A".to_string(),
                value: "1".to_string()
            }])
        );
        assert_eq!(
            log.raw_response_headers(&hop).expect("hop"),
            Some(vec![NameValue {
                name: "B".to_string(),
                value: "2".to_string()
            }])
        );
        let timing = log.timing(&hop).expect("hop");
        assert_eq!(
            timing,
            ClientTiming {
                start_time: 1_000_000.0,
                domain_lookup_start: -1.0,
                domain_lookup_end: -1.0,
                connect_start: -1.0,
                secure_connection_start: -1.0,
                connect_end: -1.0,
                request_start: -1.0,
                response_start: 250.0,
                response_end: 250.0,
            },
            "cache: wall-time start, responseStart from the end"
        );
        assert_eq!(log.http_version(&hop), Ok("HTTP/2.0".to_string()));
        assert_eq!(log.server_addr(&hop), Ok(None));
    }

    #[test]
    fn tests_that_timing_follows_the_response_payload_when_not_cached() {
        let log = replay(
            "MAIN",
            &[
                navigation("N", "MAIN", "Document", "https://a.test/"),
                response("N", false),
            ],
        );
        let hop = HopId {
            request: "N".to_string(),
            hop: 0,
        };
        let timing = log.timing(&hop).expect("hop");
        assert_eq!(
            timing.start_time.to_bits(),
            ((10.5_f64 - 10.0 + 1000.0) * 1000.0).to_bits(),
            "Playwright's (requestTime - timestamp + wallTime) * 1000"
        );
        assert_eq!(
            (
                timing.request_start,
                timing.response_start,
                timing.response_end
            ),
            (6.0, 7.0, -1.0),
            "no responseEnd before the request finished"
        );
        assert_eq!(log.sizes(&hop), Ok(None), "sizes wait for the body");
    }

    #[test]
    fn tests_that_a_session_keeps_its_latest_chains_only() {
        let events: Vec<_> = (0..=MAX_CHAINS)
            .map(|index| navigation(&format!("N{index}"), "MAIN", "Document", "https://a.test/"))
            .collect();
        let log = replay("MAIN", &events);
        let first = HopId {
            request: "N0".to_string(),
            hop: 0,
        };
        assert_eq!(log.timing(&first), Err(LogError::Dropped("request")));
        assert_eq!(log.lookup("N0", 0), Ok(Lookup::NoRequest));
        assert_eq!(
            log.lookup(&format!("N{MAX_CHAINS}"), 0),
            Ok(Lookup::Pending),
            "the newest is kept"
        );
    }

    #[test]
    fn tests_that_a_lag_turns_an_unanswered_lookup_into_lost() {
        let log = replay(
            "MAIN",
            &[navigation("N", "MAIN", "Document", "https://a.test/")],
        );
        let before = log.lags();
        log.lagged();
        assert_eq!(log.lookup("N", before), Err(LogError::Lost), "pending");
        assert_eq!(
            log.lookup("GONE", before),
            Err(LogError::Lost),
            "maybe never seen"
        );
        log.apply(
            None,
            response("N", false).0.as_str(),
            &response("N", false).1,
        );
        assert_eq!(
            log.lookup("N", before),
            Err(LogError::Lost),
            "a response after the lag may belong to a hop the lag lost"
        );
        assert_eq!(log.lookup("GONE", log.lags()), Ok(Lookup::NoRequest));
        let answered = replay(
            "MAIN",
            &[navigation("A", "MAIN", "Document", "https://a.test/")],
        );
        let (method, params) = response("A", false);
        answered.apply(None, &method, &params);
        answered.lagged();
        assert!(
            matches!(answered.lookup("A", 0), Ok(Lookup::Response(_))),
            "an answer that came before the lag stands"
        );
    }

    /// A lag that drops a navigation's first request leaves a redirect continuation to start its
    /// chain, with the hops before it gone: its answer is lost, not a Response whose request has
    /// no `redirectedFrom`.
    #[test]
    fn tests_that_a_chain_continued_from_an_unseen_redirect_is_lost() {
        let log = NetworkLog::new("MAIN".to_string());
        let before = log.lags();
        log.lagged();
        let (method, params) = event(
            "Network.requestWillBeSent",
            json!({
                "requestId": "N", "loaderId": "N", "type": "Document", "frameId": "MAIN",
                "request": {"url": "https://a.test/next", "method": "GET", "headers": {}},
                "timestamp": 10.25, "wallTime": 1000.25,
                "redirectResponse": {"url": "https://a.test/", "status": 302, "statusText": "Found",
                                     "headers": {"Location": "/next"}},
            }),
        );
        log.apply(None, &method, &params);
        let (method, params) = response("N", false);
        log.apply(None, &method, &params);
        assert_eq!(log.lookup("N", before), Err(LogError::Lost));
        assert_eq!(
            log.lookup("N", log.lags()),
            Err(LogError::Lost),
            "whenever the lag fell"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_waiter_wakes_on_the_event_it_waits_for() {
        let log = Arc::new(replay(
            "MAIN",
            &[navigation("N", "MAIN", "Document", "https://a.test/")],
        ));
        let waiting = {
            let log = Arc::clone(&log);
            tokio::spawn(async move {
                log.wait(|log| {
                    log.lookup("N", 0).map(|lookup| match lookup {
                        Lookup::Pending => None,
                        lookup => Some(lookup),
                    })
                })
                .await
            })
        };
        task::yield_now().await;
        assert!(!waiting.is_finished(), "nothing to answer yet");
        let (method, params) = response("N", false);
        log.apply(None, &method, &params);
        assert!(matches!(
            waiting.await.expect("waiter"),
            Ok(Lookup::Response(_))
        ));
    }

    /// A redirect response without network timing (an internal or cached redirect). Playwright
    /// creates its client object after `_requestFinished`, which starts an unmeasured request at
    /// its end, so the redirect's `requestStart` reads its `responseEnd`.
    #[test]
    fn tests_that_an_unmeasured_redirect_starts_its_request_at_its_end() {
        let redirect = event(
            "Network.requestWillBeSent",
            json!({
                "requestId": "N", "loaderId": "N", "type": "Document", "frameId": "MAIN",
                "request": {"url": "https://a.test/next", "method": "GET", "headers": {}},
                "timestamp": 10.25, "wallTime": 1000.25,
                "redirectResponse": {"url": "https://a.test/", "status": 307,
                                     "statusText": "Internal Redirect",
                                     "headers": {"Location": "/next"}},
            }),
        );
        let log = replay(
            "MAIN",
            &[
                navigation("N", "MAIN", "Document", "https://a.test/"),
                redirect,
            ],
        );
        let hop = HopId {
            request: "N".to_string(),
            hop: 0,
        };
        let chain = log.chain(&hop).expect("chain");
        let initializer = &chain.responses[0].as_ref().expect("a response").initializer;
        assert_eq!(
            (
                initializer.timing.request_start,
                initializer.timing.response_start
            ),
            (250.0, -1.0),
            "the initializer carries the server's rewritten requestStart"
        );
        let timing = log.timing(&hop).expect("hop");
        assert_eq!(
            (
                timing.request_start,
                timing.response_start,
                timing.response_end
            ),
            (250.0, 250.0, 250.0)
        );
    }

    /// Answers from Node 22's `Number(text)`; `None` is NaN.
    #[test]
    fn tests_that_strings_convert_to_numbers_as_javascript_does() {
        let cases: [(&str, Option<f64>); 34] = [
            ("", Some(0.0)),
            (" ", Some(0.0)),
            ("\t\n 12 \r", Some(12.0)),
            ("0x1F", Some(31.0)),
            ("0X1f", Some(31.0)),
            ("0o17", Some(15.0)),
            ("0B101", Some(5.0)),
            ("-0x10", None),
            ("+0x10", None),
            ("1e3", Some(1000.0)),
            ("1E-2", Some(0.01)),
            (".5", Some(0.5)),
            ("5.", Some(5.0)),
            ("+.5", Some(0.5)),
            ("-7", Some(-7.0)),
            ("Infinity", Some(f64::INFINITY)),
            ("-Infinity", Some(f64::NEG_INFINITY)),
            ("+Infinity", Some(f64::INFINITY)),
            ("infinity", None),
            ("inf", None),
            ("NaN", None),
            ("1_000", None),
            ("12px", None),
            ("0x", None),
            ("1e", None),
            ("--1", None),
            ("\u{a0}12\u{2028}", Some(12.0)),
            ("\u{85}12", None),
            ("\u{feff}3", Some(3.0)),
            ("\u{661}", None),
            ("1e5000", Some(f64::INFINITY)),
            ("0x1g", None),
            ("00012", Some(12.0)),
            ("1.2.3", None),
        ];
        for (text, number) in cases {
            let got = js_string_to_number(text);
            match number {
                Some(number) => assert!(got.total_cmp(&number).is_eq(), "{text:?}: {got}"),
                None => assert!(got.is_nan(), "{text:?}: {got}"),
            }
        }
    }

    fn content_length_redirect(content_length: &str) -> NetworkLog {
        let redirect = event(
            "Network.requestWillBeSent",
            json!({
                "requestId": "N", "loaderId": "N", "type": "Document", "frameId": "MAIN",
                "request": {"url": "https://a.test/next", "method": "GET", "headers": {}},
                "timestamp": 10.25, "wallTime": 1000.25,
                "redirectResponse": {"url": "https://a.test/", "status": 302, "statusText": "Found",
                                     "headers": {"Content-Length": content_length}},
            }),
        );
        replay(
            "MAIN",
            &[
                navigation("N", "MAIN", "Document", "https://a.test/"),
                redirect,
            ],
        )
    }

    /// A redirect's body size falls back to `+contentLength` (JavaScript's `Number`), and the
    /// result must be an integer, as Playwright's result validator requires; a size that cannot be
    /// one rejects instead of waiting forever.
    #[test]
    fn tests_that_sizes_read_content_length_as_javascript_and_reject_non_integers() {
        let hop = HopId {
            request: "N".to_string(),
            hop: 0,
        };
        for (content_length, size) in [("0x10", 16), (" 7 ", 7), (" ", 0), ("", 0), ("1e2", 100)] {
            let log = content_length_redirect(content_length);
            assert_eq!(
                log.sizes(&hop)
                    .map(|sizes| sizes.map(|sizes| sizes.response_body_size)),
                Ok(Some(size)),
                "{content_length:?}"
            );
        }
        for content_length in ["abc", "1.5", "0x", "Infinity", "1_000"] {
            let log = content_length_redirect(content_length);
            assert!(
                matches!(log.sizes(&hop), Err(LogError::Invalid(_))),
                "{content_length:?}: {:?}",
                log.sizes(&hop)
            );
        }
    }

    /// After a lag the watcher may have lost the event a value waits for: a value already known
    /// stands, and a pending one is lost rather than waited for forever.
    #[test]
    fn tests_that_a_lag_loses_values_still_pending() {
        let golden = golden("ok");
        let (late, early): (Vec<_>, Vec<_>) =
            golden.events.iter().cloned().partition(|(method, _)| {
                method == "Network.responseReceivedExtraInfo" || method == "Network.loadingFinished"
            });
        let log = replay(&golden.main_frame, &early);
        let document = committed(&golden).expect("a commit");
        let hop = HopId {
            request: document.clone(),
            hop: 0,
        };
        assert_eq!(
            log.raw_response_headers(&hop),
            Ok(None),
            "pending before the lag"
        );
        log.lagged();
        assert!(
            log.raw_request_headers(&hop).expect("known").is_some(),
            "known values stand"
        );
        assert_eq!(log.raw_response_headers(&hop), Err(LogError::Lost));
        assert_eq!(log.sizes(&hop), Err(LogError::Lost));
        assert_eq!(log.body(&hop), Err(LogError::Lost));
        assert_eq!(log.finished(&hop), Err(LogError::Lost));
        assert!(
            matches!(log.lookup(&document, 0), Ok(Lookup::Response(_))),
            "answered before the lag"
        );
        for (method, params) in &late {
            log.apply(None, method, params);
        }
        assert!(
            log.raw_response_headers(&hop).expect("arrived").is_some(),
            "late events count"
        );
    }

    #[test]
    fn tests_that_roundish_millis_truncate_like_javascripts_int32() {
        assert_eq!(
            seconds_to_roundish_millis(0.002_103_9).to_bits(),
            2.103_f64.to_bits()
        );
        assert_eq!(
            seconds_to_roundish_millis(-0.000_000_5).to_bits(),
            0.0_f64.to_bits()
        );
        assert_eq!(
            to_int32(2_147_483_648.0),
            -2_147_483_648,
            "wraps past i32::MAX"
        );
        assert_eq!(to_int32(4_294_967_297.0), 1);
        assert_eq!(to_int32(f64::NAN), 0);
        assert_eq!(to_int32(-1.9), -1, "truncates toward zero");
    }

    #[test]
    fn tests_that_guids_name_their_page_and_hop() {
        let id = HopId {
            request: "AB12".to_string(),
            hop: 2,
        };
        let guid = id.response_guid("T1");
        assert_eq!(guid, "response@T1:AB12:2");
        assert_eq!(
            HopId::parse(&guid),
            Some((ObjectKind::Response, "T1", id.clone()))
        );
        assert_eq!(
            HopId::parse(&id.request_guid("T1")),
            Some((ObjectKind::Request, "T1", id))
        );
        for bad in [
            "page@T1",
            "response@T1:AB12",
            "request@:AB12:0",
            "response@T1:AB12:x",
        ] {
            assert_eq!(HopId::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn tests_that_headers_split_on_their_separator_and_trim() {
        let headers = json!({"Set-Cookie": "a=1\n b=2", "X": "y"});
        let headers = headers.as_object().expect("object");
        assert_eq!(
            array(&headers_to_array(headers, Some('\n'))),
            json!([
                {"name": "Set-Cookie", "value": "a=1"},
                {"name": "Set-Cookie", "value": "b=2"},
                {"name": "X", "value": "y"},
            ])
        );
        assert_eq!(
            array(&headers_to_array(headers, None)),
            json!([
                {"name": "Set-Cookie", "value": "a=1\n b=2"},
                {"name": "X", "value": "y"},
            ])
        );
    }
}
