//! Run traces: how a host learns what each run did, through one door with a list of listeners.
//! A host installs [`TraceSink`]s on the [`Runtime`](crate::Runtime), such as a trace store and
//! a live feed. Every run asks each sink, before anything can fail, whether it listens, and the
//! sinks that do get the run's [`RunEvent`]s. Nothing here stores anything: persistence is one
//! sink among others, in its own crate.
//!
//! ```text
//!  Runtime::run ── RunId::mint, first ──▶ TraceSink::run(&RunStarted) ──▶ Some(RunSink) | None
//!                                          each installed sink, in order
//!  the run's facts ──────────────────────▶ RunSink::emit(&RunEvent) ──▶ the sink's own queue
//!  (calls, getters, pages, its end)        synchronous, on whatever thread the fact happens
//! ```
//!
//! Emit an event where its fact starts (a call's trace entry, the page registry, the run's
//! end), never in an operation's handler, so an operation added later is traced with no tracing
//! code. Emitting is synchronous and never awaited: the door is a native call on the realm's
//! thread, and the last holder of a call's state may emit from `Drop`, outside tokio. So a sink
//! only queues; it can neither slow a run nor fail it.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Serialize, Serializer};
use serde_json::Value;
use ulid::{Generator, Ulid};

use crate::quickjs_bridge::GetterField;
use crate::run::{CallLocation, CallRecord, Delivery, PageId, RunError, RunRequest, RunStatus};

/// The version of this module's event vocabulary, for sinks that persist or forward events: a
/// change that a reader of earlier events must know about bumps it.
pub const TRACE_EVENT_VERSION: u16 = 1;

/// The most bytes a host label keeps: a [`SessionKey`], and [`RunOrigin`]'s client and id
/// values. Every sink queues them and a store keeps them with each run, so a host's labels must
/// not grow without bound.
const MAX_LABEL_BYTES: usize = 256;

/// The most host ids one run keeps ([`RunOrigin::ids`]).
const MAX_RUN_IDS: usize = 16;

/// The longest [`IdKey`].
const MAX_ID_KEY_BYTES: usize = 64;

/// The last millisecond a ULID's 48-bit clock holds, in the year 10889.
const MAX_ULID_MS: u64 = (1 << Ulid::TIME_BITS) - 1;

/// The process's one source of run ids. Monotonic: an id minted in the same millisecond as the
/// one before it, or after the clock stepped back, is that id plus one, so mint order is sort
/// order across every `Runtime` in the process.
static RUN_IDS: Mutex<Generator> = Mutex::new(Generator::new());

/// A listener on every run of a [`Runtime`](crate::Runtime), installed with
/// [`Runtime::with_sinks`](crate::Runtime::with_sinks). Playwright's `InstrumentationListener`,
/// scoped to one run as Playwright scopes a listener to one browser context.
pub trait TraceSink: Send + Sync + 'static {
    /// Called first thing in `Runtime::run`, on the caller's task, before anything can fail.
    /// Returns the run's own sink, which may hold per-run state, or `None` to skip the run. Must
    /// return at once, as [`RunSink::emit`] must.
    fn run(&self, started: &RunStarted) -> Option<Arc<dyn RunSink>>;

    /// A tab's event that belongs to no run. Ignored by default.
    fn tab(&self, _event: &TabEvent) {}
}

/// One run's listener, from [`TraceSink::run`].
///
/// `emit` is called synchronously wherever a fact of the run happens: on the realm's thread
/// inside the script's native call, on the run's call and writer tasks, and from `Drop` impls,
/// which may run on any thread, outside tokio. So it must return at once: no blocking, no
/// `await`, no I/O and no panic. It cannot fail the run, so it returns nothing; a sink that
/// cannot keep an event counts it. The runtime drops the sink right after it receives
/// [`RunEvent::RunSettled`].
pub trait RunSink: Send + Sync + 'static {
    fn emit(&self, event: &RunEvent);
}

/// A run's id: a ULID minted first thing in `Runtime::run`, before anything can fail, so every
/// outcome and every event of the run carries it, early failures included. Ids come from one
/// process-wide monotonic generator, so they sort by when their runs started, within one
/// millisecond too, and so does their text: a store can use the id as its run order, its cursor
/// and its retention cut ([`RunId::floor`]). Its text, also when serialized, is the ULID's 26
/// characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(Ulid);

impl RunId {
    /// A fresh id. Should the generator fail (2^80 ids in one millisecond, or a panic while it
    /// was locked), the id is still unique, only no longer ordered within its millisecond.
    #[expect(
        clippy::unwrap_or_default,
        reason = "Ulid's default is the nil id; `Ulid::new` mints a fresh one"
    )]
    pub(crate) fn mint() -> Self {
        let next = RUN_IDS.lock().ok().and_then(|mut ids| ids.generate().ok());
        Self(next.unwrap_or_else(Ulid::new))
    }

    /// The smallest id of the millisecond `unix_ms`: runs minted at or after it sort at or
    /// after this id, earlier ones before it, so "started before T" is `id < RunId::floor(T)`.
    /// A millisecond past the ULID clock's end (year 10889) reads as that end.
    #[must_use]
    pub fn floor(unix_ms: u64) -> Self {
        Self(Ulid::from_parts(unix_ms.min(MAX_ULID_MS), 0))
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for RunId {
    type Err = InvalidRunId;

    /// Reads an id's 26 characters. A first character past `7` is refused: 26 characters hold
    /// 130 bits, and decoding would silently drop the top two, naming some other run.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match Ulid::from_string(text) {
            Ok(id) if text.starts_with(|first: char| first <= '7') => Ok(Self(id)),
            _ => Err(InvalidRunId),
        }
    }
}

impl Serialize for RunId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Text that is not a [`RunId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a run id is 26 Crockford base32 characters, the first one 0 to 7")]
pub struct InvalidRunId;

/// A run's session, as the host labels it from its own request metadata: the runs a store shows
/// as one timeline. Never a CDP session id. At most 256 bytes: a longer label is cut on a
/// character boundary.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct SessionKey(Arc<str>);

impl SessionKey {
    #[must_use]
    pub fn new(label: impl AsRef<str>) -> Self {
        Self(Arc::from(cut(label.as_ref(), MAX_LABEL_BYTES)))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The name of one of the host's own ids for a run, such as `zuzu.task` or `user`: 1 to 64 bytes
/// of `a-z`, `0-9`, `.`, `_` and `-`. A host adds a kind of id as a new key, with no change to
/// neo-runtime or to a store's schema.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct IdKey(Arc<str>);

impl IdKey {
    /// # Errors
    ///
    /// [`InvalidIdKey`] when `key` breaks [`IdKey`]'s rule.
    pub fn new(key: &str) -> Result<Self, InvalidIdKey> {
        let allowed = |byte: u8| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-');
        if (1..=MAX_ID_KEY_BYTES).contains(&key.len()) && key.bytes().all(allowed) {
            Ok(Self(Arc::from(key)))
        } else {
            Err(InvalidIdKey)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A name that is not an [`IdKey`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("an id key is 1 to 64 bytes of a-z, 0-9, '.', '_' and '-'")]
pub struct InvalidIdKey;

/// Who started a run, in the host's words: labels from the host's own request metadata, never
/// from the model. They reach every sink in [`RunStarted`], so a store can find runs by them.
///
/// `Runtime::run` reads it once, into [`RunStarted`], and caps it there: the first 16 ids in key
/// order, and `client` and each id's value cut to 256 bytes on a character boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunOrigin {
    pub session: Option<SessionKey>,
    /// The host's client, such as the MCP client's name.
    pub client: Option<String>,
    /// The host's own ids for the run, such as `zuzu.task` → `t_123`.
    pub ids: BTreeMap<IdKey, String>,
}

/// A run as traces refer to it: its id and its session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RunRef {
    pub id: RunId,
    pub session: Option<SessionKey>,
}

/// One call of one run, as text `<run>@<no>` (Playwright's `callId` shape). `no` is the call's
/// [`CallRecord::call_id`]: its arrival order in the run, from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallId {
    pub run: RunId,
    pub no: u64,
}

impl fmt::Display for CallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.run, self.no)
    }
}

impl FromStr for CallId {
    type Err = InvalidCallId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (run, no) = text.split_once('@').ok_or(InvalidCallId)?;
        Ok(Self {
            run: run.parse().map_err(|_| InvalidCallId)?,
            no: no.parse().map_err(|_| InvalidCallId)?,
        })
    }
}

impl Serialize for CallId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Text that is not a [`CallId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a call id is <run id>@<call number>")]
pub struct InvalidCallId;

/// A page as traces name it: its CDP target, and its BrowserOS tab id once Neo has read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRef {
    pub target: PageId,
    pub tab_id: Option<i64>,
}

/// What a run is, handed to every [`TraceSink::run`] before anything can fail.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStarted {
    pub run: RunRef,
    pub client: Option<String>,
    pub ids: BTreeMap<IdKey, String>,
    /// The tab the run starts from.
    pub page: PageId,
    /// The run's budget, [`RunRequest::timeout`].
    pub timeout_ms: u64,
    /// The script's size in bytes; the script itself comes with `RunEnded`.
    pub code_bytes: u64,
    /// Wall clock at the run's start, in Unix milliseconds. Later events count `offset_us` from
    /// the start on a monotonic clock.
    pub started_at_ms: u64,
}

impl RunStarted {
    /// The run `request` asks for, under `run`, with [`RunOrigin`]'s caps applied.
    pub(crate) fn new(run: RunId, request: &RunRequest) -> Self {
        let RunOrigin {
            session,
            client,
            ids,
        } = &request.origin;
        Self {
            run: RunRef {
                id: run,
                session: session.clone(),
            },
            client: client
                .as_deref()
                .map(|client| cut(client, MAX_LABEL_BYTES).to_string()),
            ids: ids
                .iter()
                .take(MAX_RUN_IDS)
                .map(|(key, value)| (key.clone(), cut(value, MAX_LABEL_BYTES).to_string()))
                .collect(),
            page: request.page.clone(),
            timeout_ms: u64::try_from(request.timeout.as_millis()).unwrap_or(u64::MAX),
            code_bytes: u64::try_from(request.code.len()).unwrap_or(u64::MAX),
            started_at_ms: unix_ms(SystemTime::now()),
        }
    }
}

/// Everything a run reports after [`RunStarted`], one variant per kind of fact.
///
/// Deliberately not `#[non_exhaustive]`: a new variant breaks the build of every consumer in the
/// workspace until it decides what to do with it. Serialized with a `type` tag and camelCase
/// fields, so a sink can write JSON lines as they come. Heavy payloads sit behind `Arc`, so a
/// sink that queues an event clones refcounts, never a log.
///
/// A sink may rely on this order within one run: [`TraceSink::run`] first; per call,
/// `BeforeCall` before its other events, exactly one `AfterCall`, and at most one
/// `DeliverySettled` after it; exactly one `RunEnded`; `RunSettled` last. Only for a dropped
/// future or a call past the outcome's caps can an `AfterCall` follow `RunEnded`. Across calls
/// there is no order.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RunEvent {
    /// A call reached the door, before it runs.
    BeforeCall {
        run: RunId,
        /// The call's number in the run, [`CallRecord::call_id`].
        call: u64,
        /// The call that made this one, for a host method's nested calls.
        parent: Option<u64>,
        /// The object the call named, such as `frame@<targetId>`.
        object: String,
        page: Option<PageRef>,
        method: String,
        title: Option<String>,
        selector: Option<String>,
        location: Option<CallLocation>,
        started_at_ms: u64,
        /// Since the run started, on a monotonic clock, as every event's `offset_us`.
        offset_us: u64,
    },
    /// The call is about to write its first input event; from here its delivery may be
    /// `Unknown`.
    InputSent {
        run: RunId,
        call: u64,
        offset_us: u64,
    },
    /// A call ended, however it ended: replied, refused at the door, or cut off by the run.
    AfterCall {
        run: RunId,
        call: u64,
        parent: Option<u64>,
        page: Option<PageRef>,
        /// The call's record, built the way `RunOutcome::calls` builds them.
        record: Arc<CallRecord>,
        /// The record's params were cut to fit.
        params_truncated: bool,
        offset_us: u64,
    },
    /// An input call's delivery moved after its `AfterCall`, such as an acknowledgement that
    /// arrived late: its final delivery.
    DeliverySettled {
        run: RunId,
        call: u64,
        delivery: Delivery,
    },
    /// The run got a page: its starting page, one it listed, or one it created.
    PageOpened {
        run: RunId,
        page: PageRef,
        created: bool,
        /// The call that created the page.
        by_call: Option<u64>,
        offset_us: u64,
    },
    /// The run closed a page.
    PageClosed {
        run: RunId,
        page: PageId,
        offset_us: u64,
    },
    /// A synchronous getter answered. Getters have their own numbers (`no`), so call numbers
    /// stay dense.
    Getter {
        run: RunId,
        no: u64,
        /// The object the getter read, such as `page@<targetId>`.
        object: String,
        field: GetterField,
        page: Option<PageRef>,
        result: GetterResult,
        offset_us: u64,
        /// How long the getter blocked the script.
        duration_us: u64,
    },
    /// The run ended, also when its future was dropped.
    RunEnded {
        run: RunId,
        status: RunStatus,
        cause: EndCause,
        error: Option<RunError>,
        value: Option<Arc<Value>>,
        value_truncated: bool,
        logs: Arc<[ConsoleLine]>,
        script: Option<Arc<str>>,
        calls_made: u64,
        /// Messages the door could not read as a call, so they have no call number.
        unreadable_calls: u64,
        duration_us: u64,
    },
    /// Nothing of the run is left that could emit.
    RunSettled { run: RunId },
}

/// How a run ended, finer than [`RunStatus`]: the end a trace shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EndCause {
    /// The script returned.
    Returned,
    /// The script threw.
    Threw,
    /// The run's deadline passed.
    Deadline,
    /// The host cancelled the run.
    Cancelled,
    /// The host dropped `Runtime::run`'s future, so no outcome was returned.
    Dropped,
    /// The starting tab was closed or could not be opened, so no script ran.
    TabUnavailable,
    /// The host cannot run scripts, such as on tokio's current-thread runtime, so no script ran.
    Unsupported,
}

/// One `console` line the script wrote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleLine {
    pub offset_us: u64,
    pub text: String,
}

/// How a getter answered.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GetterResult {
    /// The getter's value, summarised for the trace.
    Ok { summary: Value },
    /// The error the script saw, by its JS `name`.
    Error { name: String, message: String },
}

/// A tab's event that belongs to no run, such as a tab's console between runs. No tab-scoped
/// source is traced yet, so it has no variants.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum TabEvent {}

/// `at` in Unix milliseconds; `0` before the epoch.
pub(crate) fn unix_ms(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH).map_or(0, |since| {
        u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
    })
}

/// `text`'s longest prefix of at most `max` bytes that ends on a character boundary.
fn cut(text: &str, max: usize) -> &str {
    &text[..text.floor_char_boundary(max)]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn run_ids_minted_back_to_back_increase_and_read_back_from_their_text() {
        let ids: Vec<RunId> = (0..10_000).map(|_| RunId::mint()).collect();
        for pair in ids.windows(2) {
            let (first, next) = (pair[0], pair[1]);
            assert!(first < next, "minted in order: {first} then {next}");
            assert!(
                first.to_string() < next.to_string(),
                "their text sorts as they do: {first} then {next}"
            );
        }
        for id in &ids {
            let text = id.to_string();
            assert_eq!(text.len(), 26, "a ULID's 26 characters: {text}");
            assert_eq!(text.parse(), Ok(*id), "{text} reads back");
            assert_eq!(
                serde_json::to_value(id).unwrap(),
                json!(text),
                "serialized as its text"
            );
        }
        for bad in [
            "",
            "01JC8Z3K",
            "80000000000000000000000000",
            "0000000000000000000000000U",
        ] {
            assert_eq!(
                bad.parse::<RunId>(),
                Err(InvalidRunId),
                "{bad:?} names no run"
            );
        }
    }

    #[test]
    fn floor_splits_runs_at_its_millisecond() {
        let before = unix_ms(SystemTime::now());
        let id = RunId::mint();
        let after = unix_ms(SystemTime::now());
        assert!(
            RunId::floor(before) <= id,
            "{id} was minted at or after {before}"
        );
        assert!(
            id < RunId::floor(after + 1),
            "{id} was minted before {}",
            after + 1
        );
        assert!(
            id < RunId::floor(1 << 48),
            "a millisecond past the ULID clock's end sorts after every run"
        );
    }

    #[test]
    fn call_ids_read_back_from_their_text() {
        let call = CallId {
            run: RunId::mint(),
            no: 7,
        };
        let text = call.to_string();
        assert_eq!(text, format!("{}@7", call.run), "written <run>@<no>");
        assert_eq!(text.parse(), Ok(call), "{text} reads back");
        assert_eq!(
            serde_json::to_value(call).unwrap(),
            json!(text),
            "serialized as its text"
        );
        let run = call.run.to_string();
        let bad = [
            String::new(),
            "@7".to_string(),
            format!("{run}@"),
            format!("{run}@seven"),
            format!("{run}7"),
            format!("{}@7", &run[1..]),
        ];
        for bad in bad {
            assert_eq!(
                bad.parse::<CallId>(),
                Err(InvalidCallId),
                "{bad:?} is not a call id"
            );
        }
    }

    #[test]
    fn id_keys_are_short_lowercase_names() {
        let longest = "k".repeat(64);
        for key in ["zuzu.task", "user", "trace-id", "agent_7", "0", &longest] {
            assert_eq!(
                IdKey::new(key).map(|key| key.as_str().to_string()),
                Ok(key.to_string()),
                "{key:?} is a key"
            );
        }
        let too_long = "k".repeat(65);
        for key in [
            "",
            &too_long,
            "Zuzu.task",
            "zuzu task",
            "zuzu/task",
            "tâche",
        ] {
            assert_eq!(IdKey::new(key), Err(InvalidIdKey), "{key:?} is not a key");
        }
    }

    #[test]
    fn session_keys_keep_their_first_256_bytes_on_a_character_boundary() {
        assert_eq!(
            SessionKey::new("task-42").as_str(),
            "task-42",
            "a short label is kept whole"
        );
        let ascii = "s".repeat(300);
        assert_eq!(
            SessionKey::new(&ascii).as_str(),
            &ascii[..256],
            "a long label keeps its first 256 bytes"
        );
        // `é` takes two bytes, so here it would end at byte 257.
        let label = format!("{}é", "s".repeat(255));
        assert_eq!(
            SessionKey::new(&label).as_str(),
            &label[..255],
            "the character crossing byte 256 is dropped whole"
        );
    }

    #[test]
    fn run_events_serialize_with_a_type_tag_and_camel_case_fields() {
        let run = RunId::mint();
        let settled = serde_json::to_value(RunEvent::RunSettled { run }).unwrap();
        assert_eq!(settled["type"], "runSettled", "{settled}");
        assert_eq!(settled["run"], json!(run.to_string()), "{settled}");
        let sent = serde_json::to_value(RunEvent::InputSent {
            run,
            call: 3,
            offset_us: 9,
        })
        .unwrap();
        assert_eq!(sent["type"], "inputSent", "{sent}");
        assert_eq!(sent["offsetUs"], 9, "{sent}");
    }
}
