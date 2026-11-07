//! Neo: one run's event pipe from Rust into the realm. Playwright's server pushes event messages
//! (`{guid, method, params}`) down the same socket as its replies, and its client runs their
//! listeners in that order; Neo has no socket into the realm, so the page watchers put the same
//! messages on one ordered queue here, and the realm takes them on its own thread.
//!
//! ```text
//!  page watcher, discovery ──emit()──▶ EventHub queue (seq 1, 2, 3 …)   subscribed events only
//!  call task: handler done ─▶ barrier(): every watcher applies what preceded the reply
//!                          ─▶ W = seq() ─▶ reply (text, W)
//!  JS: runtime-bridge call() ─▶ drain(W) + dispatch, synchronously ─▶ the caller continues
//!  JS idle (a timer, waitForEvent): nextEvents() resolves ─▶ drain(all) + dispatch ─▶ re-arm
//! ```
//!
//! So every event a watcher applied from CDP traffic that came before a call's last reply has run
//! its listeners before that call's `await` continues, which is the order Playwright's client
//! sees. The barrier works because browseros-cdp's reader broadcasts every event before it
//! completes a later reply: once a call task holds its reply, the events before it are in the
//! watchers' rings, and a watcher acks a flush only after draining its ring.
//!
//! A subscription is a (guid, listener event) pair the SDK turns on when an event's listener
//! count leaves 0. Producers emit only what is subscribed, and a run with no subscription skips
//! the barrier, so it pays nothing. A subscription also switches on what its producer needs
//! (`Runtime.enable` for console messages, the page watcher for dialogs, target discovery for new
//! pages); [`EventHub::ready`] holds a page's next call until that is done, so the call cannot
//! race the enable. Discovery holds every call of the run ([`ALL_PAGES`]): any call may open a
//! tab.
//!
//! The queue lives in Rust, outside the realm's memory limit, so it is capped; past a cap,
//! events are counted and dropped until the script drains, and the run log says so once per
//! such period. Everything here dies with the run: [`EventHub::close`] at the run's end drops
//! the subscriptions, so producers fall back to their no-listener defaults (dialogs answered).

use std::collections::{HashMap, HashSet, VecDeque};
use std::mem;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future;
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio::time;
use tokio_util::sync::CancellationToken;
use tracing::warn;

/// Neo: the guid of the run's one context, where Playwright's server sends console, dialog and
/// page error events, each naming its page.
pub(crate) const CONTEXT_GUID: &str = "context@default";

/// Neo: the prefix of every context's guid, the run's own and each one the run created
/// (`browser_calls.rs`).
const CONTEXT_PREFIX: &str = "context@";

/// Deviation: `client/connection.ts` `Connection.dispatch`: `const object = this._objects.get(guid);`
/// Playwright's client dispatches every event its server sends; Neo's queue lives outside the
/// realm's memory limit, so a run holds at most this many undelivered events and counts the rest.
const MAX_QUEUED_EVENTS: usize = 10_000;

/// Deviation: `client/connection.ts` `Connection.dispatch`: `const object = this._objects.get(guid);`
/// The same bound in bytes of queued JSON, so a few huge payloads cannot pin the server's memory.
const MAX_QUEUED_BYTES: usize = 16 * 1024 * 1024;

/// Neo: how long a call's reply waits for the watchers to apply what preceded it. A watcher only
/// stops reading to answer a flush, so this bound matters only for one stuck behind a slow
/// mutex; past it the reply goes, and events it missed arrive with a later reply or wake.
const BARRIER_WAIT: Duration = Duration::from_millis(250);

/// Neo: the longest a call waits for its page's enables. A renderer frozen by a dialog may hold
/// an enable until the dialog is answered; a call with no timeout of its own (a close, a history
/// read) then goes ahead rather than wait for the run's deadline.
const READY_WAIT: Duration = Duration::from_secs(10);

/// Neo: how long the end of `Runtime.enable`'s replay waits for the watchers. Once, per session;
/// a watcher that applied the replay late would hand its tail to listeners as live messages.
const REPLAY_WAIT: Duration = Duration::from_secs(5);

/// Neo: the listener events Neo produces on a `Page`, under the names Playwright's client gives
/// them (`Events.Page`), and what each needs switched on in the page's session; and the one
/// channel event the SDK handles itself, `locatorHandlerTriggered` (`locator_handlers.rs`).
const PAGE_EVENTS: [(&str, Needs); 21] = [
    ("console", Needs::RUNTIME),
    ("pageerror", Needs::RUNTIME),
    ("dialog", Needs::WATCHER),
    ("dialogclosed", Needs::WATCHER),
    ("close", Needs::WATCHER),
    ("crash", Needs::WATCHER),
    ("load", Needs::WATCHER),
    ("domcontentloaded", Needs::WATCHER),
    ("frameattached", Needs::WATCHER),
    ("framedetached", Needs::WATCHER),
    ("framenavigated", Needs::WATCHER),
    ("popup", Needs::DISCOVERY),
    // The protocol event of an exposed binding (`bindings.rs`), which the SDK listens to while
    // one is exposed; a binding needs `Runtime.enable` on the tab.
    ("bindingCall", Needs::RUNTIME),
    // The watcher reports it once the page intercepts its file chooser (`page_events.rs`
    // `intercept_file_chooser`).
    ("filechooser", Needs::WATCHER),
    ("download", Needs::DOWNLOADS),
    ("request", Needs::NETWORK),
    ("response", Needs::NETWORK),
    ("requestfinished", Needs::NETWORK),
    ("requestfailed", Needs::NETWORK),
    // A paused request of a route (`routes.rs`): Playwright's channel event, which the SDK
    // listens to while the page has routes. Its request's record needs the network log.
    ("route", Needs::NETWORK),
    ("locatorHandlerTriggered", Needs::NOTHING),
];

/// Neo: the listener events Neo produces on a `BrowserContext` (`Events.BrowserContext`), the
/// run's own or one `browser.newContext` created, for every page of it the run has a session on;
/// `page` for every new tab of the context.
const CONTEXT_EVENTS: [(&str, Needs); 17] = [
    ("console", Needs::RUNTIME),
    ("weberror", Needs::RUNTIME),
    ("dialog", Needs::WATCHER),
    ("dialogclosed", Needs::WATCHER),
    ("pageclose", Needs::WATCHER),
    ("pageload", Needs::WATCHER),
    ("frameattached", Needs::WATCHER),
    ("framedetached", Needs::WATCHER),
    ("framenavigated", Needs::WATCHER),
    ("page", Needs::DISCOVERY),
    ("bindingCall", Needs::RUNTIME),
    ("download", Needs::DOWNLOADS),
    ("request", Needs::NETWORK),
    ("response", Needs::NETWORK),
    ("requestfinished", Needs::NETWORK),
    ("requestfailed", Needs::NETWORK),
    ("route", Needs::NETWORK),
];

/// Neo: the listener event of a context's `Debugger` (`Events.Debugger`), which needs nothing
/// switched on: the debugger queues it itself (`debugger.rs`).
const DEBUGGER_EVENTS: [(&str, Needs); 1] = [("pausedstatechanged", Needs::NOTHING)];

/// What a subscription switches on in a page's session before that page's next call runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one flag per producer a subscription may switch on, combined with ||"
)]
pub(crate) struct Needs {
    /// `PageSession::prepare`: the watcher, which is the producer, with `Page.enable`.
    pub(crate) watcher: bool,
    /// `Runtime.enable` and `Log.enable`, which report console messages and page errors.
    pub(crate) runtime: bool,
    /// `Network.enable` (`PageSession::enable_network`), which reports the tab's requests.
    pub(crate) network: bool,
    /// The run's target discovery (`discovery.rs`), which reports new tabs. Not a page's: every
    /// call of the run waits for it ([`ALL_PAGES`]).
    pub(crate) discovery: bool,
    /// The run's download reports (`downloads.rs`). Not a page's either: every call of the run
    /// waits for them ([`ALL_PAGES`]).
    pub(crate) downloads: bool,
}

impl Needs {
    /// Neo: an event with nothing to switch on, which no page's domains produce: an action's own
    /// call's (`locatorHandlerTriggered`) or the debugger's.
    const NOTHING: Self = Self {
        watcher: false,
        runtime: false,
        network: false,
        discovery: false,
        downloads: false,
    };
    /// Neo: an event the page watcher produces from the domains it always enables.
    pub(crate) const WATCHER: Self = Self {
        watcher: true,
        runtime: false,
        network: false,
        discovery: false,
        downloads: false,
    };
    /// Neo: an event only `Runtime.enable` and `Log.enable` report.
    const RUNTIME: Self = Self {
        watcher: true,
        runtime: true,
        network: false,
        discovery: false,
        downloads: false,
    };
    /// Neo: an event only `Network.enable` reports: a request's.
    const NETWORK: Self = Self {
        watcher: true,
        runtime: false,
        network: true,
        discovery: false,
        downloads: false,
    };
    /// Neo: an event the run's target discovery produces, about a page no session watches yet.
    const DISCOVERY: Self = Self {
        watcher: false,
        runtime: false,
        network: false,
        discovery: true,
        downloads: false,
    };
    /// Neo: an event the run's download watcher produces from the browser's reports, for any
    /// tab of the run's context.
    const DOWNLOADS: Self = Self {
        watcher: false,
        runtime: false,
        network: false,
        discovery: false,
        downloads: true,
    };

    /// Whether a page's session has anything to switch on for it.
    pub(crate) fn of_page(self) -> bool {
        self.watcher || self.runtime || self.network
    }
}

/// Neo: the [`EventHub::begin_enable`] key of an enable every call of the run waits for, a
/// page's or the context's: target discovery, which must hold its baseline before any call can
/// open a tab.
pub(crate) const ALL_PAGES: &str = "*";

/// What a listener event on `guid` needs, or `None` when Neo does not produce it there. `guid`
/// is the context's or a page's (`page@<targetId>`).
pub(crate) fn needs(guid: &str, event: &str) -> Option<Needs> {
    let table: &[(&str, Needs)] = if guid.starts_with(CONTEXT_PREFIX) {
        &CONTEXT_EVENTS
    } else if guid.starts_with("page@") {
        &PAGE_EVENTS
    } else if guid.starts_with("debugger@") {
        &DEBUGGER_EVENTS
    } else {
        return None;
    };
    table
        .iter()
        .find(|(name, _)| *name == event)
        .map(|(_, needs)| *needs)
}

/// The run's event queue, subscriptions and flush registry. Cheap to clone; every clone is the
/// same hub. Built with the run's `Tabs`, so the bridge, the dispatcher and every page session
/// reach the same one.
#[derive(Clone, Default)]
pub(crate) struct EventHub(Arc<Inner>);

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    /// Wakes the realm's pending `nextEvents()` when an event is queued.
    arrived: Notify,
    /// Cancelled by [`EventHub::close`] and [`EventHub::end_subscriptions`]: producers that run
    /// on their own (target discovery) end.
    ended: CancellationToken,
}

#[derive(Default)]
struct State {
    /// Set when the run ends: nothing is subscribed, queued or delivered after that.
    closed: bool,
    /// Set when the run's context closes ([`EventHub::end_subscriptions`]): nothing is
    /// subscribed after that, while what is queued still drains.
    subscriptions_ended: bool,
    /// The seq of the newest queued event, 0 before the first.
    seq: u64,
    queue: VecDeque<Queued>,
    bytes: usize,
    /// (guid, listener event) pairs the script listens to.
    subscriptions: HashSet<(String, String)>,
    /// (platform) The context of each page of a context the run created, by page guid; every
    /// other page is the run's own context's.
    owners: HashMap<String, String>,
    /// One flush channel per live watcher; a closed one belongs to a watcher that ended.
    watchers: Vec<Watcher>,
    /// Per page (target id): the enables its subscriptions started, done when `true`.
    enables: HashMap<String, Vec<watch::Receiver<bool>>>,
    /// Events dropped since the queue last filled up, until the script next drains.
    dropped: Option<usize>,
    /// Run-log lines waiting for the bridge to write them.
    notes: Vec<String>,
}

/// Neo: a producer's flush channel, and whether it consumes Runtime.enable's replay.
/// Callback delivery can prepare a cold frame and wait for replay itself, so replay must not
/// wait back on it. Ordinary call barriers still flush both kinds of producer.
struct Watcher {
    sender: mpsc::UnboundedSender<oneshot::Sender<()>>,
    replay: bool,
}

/// One queued event, already in the realm's JSON shape `{seq, guid, method, params}`.
struct Queued {
    seq: u64,
    text: String,
}

/// What [`EventHub::emit`] did with an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Emitted {
    Queued,
    /// The queue is full; the event is counted and lost. A producer that owes an answer (a
    /// dialog) gives the default one.
    Dropped,
    /// The run is over.
    Closed,
}

/// A watcher's end of the flush registry ([`EventHub::watcher`]). Each message is a flush: the
/// watcher drains every event already in its ring, then answers on the enclosed sender.
pub(crate) struct Flushes(mpsc::UnboundedReceiver<oneshot::Sender<()>>);

impl Flushes {
    pub(crate) async fn recv(&mut self) -> Option<oneshot::Sender<()>> {
        self.0.recv().await
    }
}

/// An enable in progress for one page ([`EventHub::begin_enable`]). Dropping it, done or failed,
/// releases the page's calls: a failed enable must not hold them, and they fail on their own if
/// the page has gone.
pub(crate) struct Enabling(watch::Sender<bool>);

impl Drop for Enabling {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

impl EventHub {
    /// Turns the script's listening to `event` on `guid` on or off. Takes effect for the next
    /// event a producer applies; what it needs switched on is the caller's ([`needs`]).
    pub(crate) fn subscribe(&self, guid: &str, event: &str, enabled: bool) {
        self.with(|state| {
            if state.closed || state.subscriptions_ended {
                return;
            }
            let key = (guid.to_string(), event.to_string());
            if enabled {
                state.subscriptions.insert(key);
            } else {
                state.subscriptions.remove(&key);
            }
        });
    }

    /// Whether the script listens to `event` on `guid`.
    pub(crate) fn subscribed(&self, guid: &str, event: &str) -> bool {
        self.with(|state| listens(state, guid, event))
            .unwrap_or(false)
    }

    /// (platform) Records that the page `page_guid` belongs to the context `context_guid` the run
    /// created: its context-level events answer to that context's listeners, never the run's own.
    pub(crate) fn set_owner(&self, page_guid: &str, context_guid: &str) {
        self.with(|state| {
            state
                .owners
                .insert(page_guid.to_string(), context_guid.to_string())
        });
    }

    /// Whether a page event reaches any listener: the page's own `page_event` or its context's
    /// `context_event` (Playwright's `BrowserContextDispatcher._shouldDispatchEvent`), the
    /// context that owns the page.
    pub(crate) fn wants(&self, page_guid: &str, page_event: &str, context_event: &str) -> bool {
        self.with(|state| {
            let context = state
                .owners
                .get(page_guid)
                .map_or(CONTEXT_GUID, String::as_str);
            listens(state, page_guid, page_event) || listens(state, context, context_event)
        })
        .unwrap_or(false)
    }

    /// What the listeners of the context `context_guid` need on every page of it with a
    /// session, if it listens at all.
    pub(crate) fn context_needs(&self, context_guid: &str) -> Option<Needs> {
        self.with(|state| {
            state
                .subscriptions
                .iter()
                .filter(|(guid, _)| guid == context_guid)
                .filter_map(|(guid, event)| needs(guid, event))
                .reduce(|left, right| Needs {
                    watcher: left.watcher || right.watcher,
                    runtime: left.runtime || right.runtime,
                    network: left.network || right.network,
                    discovery: left.discovery || right.discovery,
                    downloads: left.downloads || right.downloads,
                })
        })
        .flatten()
    }

    fn any_subscription(&self) -> bool {
        self.with(|state| !state.subscriptions.is_empty())
            .unwrap_or(false)
    }

    /// Queues one Playwright event message for the realm, unless the run is over or the queue is
    /// full. Producers check the subscription first ([`Self::wants`]).
    pub(crate) fn emit(&self, guid: &str, method: &str, params: &Value) -> Emitted {
        let emitted = self
            .with(|state| {
                if state.closed {
                    return Emitted::Closed;
                }
                let seq = state.seq + 1;
                let text = json!({ "seq": seq, "guid": guid, "method": method, "params": params })
                    .to_string();
                if state.queue.len() >= MAX_QUEUED_EVENTS
                    || state.bytes.saturating_add(text.len()) > MAX_QUEUED_BYTES
                {
                    *state.dropped.get_or_insert(0) += 1;
                    return Emitted::Dropped;
                }
                state.seq = seq;
                state.bytes += text.len();
                state.queue.push_back(Queued { seq, text });
                Emitted::Queued
            })
            .unwrap_or(Emitted::Closed);
        if emitted == Emitted::Queued {
            self.0.arrived.notify_waiters();
        }
        emitted
    }

    /// The seq of the newest queued event: a reply's watermark W.
    pub(crate) fn seq(&self) -> u64 {
        self.with(|state| state.seq).unwrap_or(0)
    }

    /// Takes every queued event with seq ≤ `until` (all of them for `None`), in seq order, as the
    /// JSON array text the realm parses. Ends a drop period with its run-log note.
    pub(crate) fn drain(&self, until: Option<u64>) -> String {
        self.with(|state| {
            let mut text = String::from("[");
            while let Some(event) = state.queue.front() {
                if until.is_some_and(|until| event.seq > until) {
                    break;
                }
                let Some(event) = state.queue.pop_front() else {
                    break;
                };
                state.bytes -= event.text.len();
                if text.len() > 1 {
                    text.push(',');
                }
                text.push_str(&event.text);
            }
            text.push(']');
            if let Some(dropped) = state.dropped.take() {
                state.notes.push(drop_note(dropped));
            }
            text
        })
        .unwrap_or_else(|| "[]".to_string())
    }

    /// Resolves once an undelivered event is queued: at once if one is. The realm's idle wake;
    /// dropping it (the run ended) is fine.
    pub(crate) async fn next(&self) {
        loop {
            let arrived = self.0.arrived.notified();
            tokio::pin!(arrived);
            arrived.as_mut().enable();
            if self.with(|state| !state.queue.is_empty()).unwrap_or(false) {
                return;
            }
            arrived.await;
        }
    }

    /// Registers a watcher's flush channel. The watcher selects [`Flushes::recv`] after its CDP
    /// events under `biased;` and drains its ring before answering.
    pub(crate) fn watcher(&self) -> Flushes {
        self.add_watcher(true)
    }

    /// Neo: callback delivery participates in call ordering, but not Runtime.enable replay.
    /// Preparing a callback's cold frame can itself wait for that replay to finish.
    pub(crate) fn callback_watcher(&self) -> Flushes {
        self.add_watcher(false)
    }

    fn add_watcher(&self, replay: bool) -> Flushes {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.with(|state| state.watchers.push(Watcher { sender, replay }));
        Flushes(receiver)
    }

    /// The ordering barrier a call runs before it replies: when anything is subscribed, every
    /// live watcher applies the events that preceded the call's last CDP reply. Free otherwise.
    pub(crate) async fn barrier(&self) {
        if self.any_subscription() {
            self.flush(BARRIER_WAIT, false).await;
        }
    }

    /// Flushes runtime-event producers through the end of `Runtime.enable`'s replay, whatever
    /// is subscribed. Callback delivery is excluded because it can be waiting for this replay.
    pub(crate) async fn replay_barrier(&self) {
        self.flush(REPLAY_WAIT, true).await;
    }

    async fn flush(&self, bound: Duration, replay_only: bool) {
        let acks = self
            .with(|state| {
                state.watchers.retain(|watcher| !watcher.sender.is_closed());
                state
                    .watchers
                    .iter()
                    .filter(|watcher| !replay_only || watcher.replay)
                    .filter_map(|watcher| {
                        let (ack, acked) = oneshot::channel();
                        watcher.sender.send(ack).ok().map(|()| acked)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if acks.is_empty() {
            return;
        }
        if time::timeout(bound, future::join_all(acks)).await.is_err() {
            warn!(
                ?bound,
                "a page watcher did not apply its events within the barrier"
            );
        }
    }

    /// Records an enable starting on `page` (a target id); [`Self::ready`] waits for it.
    pub(crate) fn begin_enable(&self, page: &str) -> Enabling {
        let (done, receiver) = watch::channel(false);
        self.with(|state| {
            let pending = state.enables.entry(page.to_string()).or_default();
            pending.retain(|enable| !*enable.borrow());
            pending.push(receiver);
        });
        Enabling(done)
    }

    /// Whether an enable started on `page` is still running. A call checks this before it waits
    /// in [`Self::ready`], so a call with nothing to wait for gets no extra cancellation point
    /// before its handler starts.
    pub(crate) fn enabling(&self, page: &str) -> bool {
        self.with(|state| {
            [page, ALL_PAGES].iter().any(|key| {
                state
                    .enables
                    .get(*key)
                    .is_some_and(|pending| pending.iter().any(|enable| !*enable.borrow()))
            })
        })
        .unwrap_or(false)
    }

    /// Resolves once every enable started on `page` (or on [`ALL_PAGES`]) so far is done, or after
    /// [`READY_WAIT`]. The
    /// caller races it with its call's progress, so a call's own timeout still ends it first.
    pub(crate) async fn ready(&self, page: &str) {
        if time::timeout(READY_WAIT, self.enabled(page)).await.is_err() {
            warn!(page, "a call went ahead of its page's unfinished enable");
        }
    }

    async fn enabled(&self, page: &str) {
        let pending = self
            .with(|state| {
                let mut pending = Vec::new();
                for key in [page, ALL_PAGES] {
                    if let Some(enables) = state.enables.get_mut(key) {
                        enables.retain(|enable| !*enable.borrow());
                        pending.extend(enables.iter().cloned());
                    }
                }
                pending
            })
            .unwrap_or_default();
        for mut enable in pending {
            let _done = enable.wait_for(|done| *done).await;
        }
    }

    /// The run's context closed: drops every subscription, so producers give their
    /// no-listener defaults from now on and target discovery ends, and refuses later ones.
    /// Events already queued still drain, in order, before the close's reply continues.
    pub(crate) fn end_subscriptions(&self) {
        self.with(|state| {
            state.subscriptions_ended = true;
            state.subscriptions.clear();
            state.enables.clear();
        });
        self.0.ended.cancel();
    }

    /// Whether the run's context closed ([`Self::end_subscriptions`]) or the run ended.
    pub(crate) fn subscriptions_ended(&self) -> bool {
        self.with(|state| state.closed || state.subscriptions_ended)
            .unwrap_or(true)
    }

    /// Ends the run's events: drops every subscription and queued event, so producers give
    /// their no-listener defaults from now on, and refuses further emits.
    pub(crate) fn close(&self) {
        self.with(|state| {
            state.closed = true;
            state.subscriptions.clear();
            state.queue.clear();
            state.bytes = 0;
            state.enables.clear();
            if let Some(dropped) = state.dropped.take() {
                state.notes.push(drop_note(dropped));
            }
        });
        self.0.ended.cancel();
        self.0.arrived.notify_waiters();
    }

    /// Cancelled once the run's events end ([`Self::close`]) or its context closes
    /// ([`Self::end_subscriptions`]).
    pub(crate) fn ended(&self) -> CancellationToken {
        self.0.ended.clone()
    }

    /// Adds a line the run's log gets with the next drain: a producer that could not start.
    pub(crate) fn note(&self, line: String) {
        self.with(|state| state.notes.push(line));
    }

    /// Run-log lines the hub owes the script's log, oldest first.
    pub(crate) fn take_notes(&self) -> Vec<String> {
        self.with(|state| mem::take(&mut state.notes))
            .unwrap_or_default()
    }

    /// Runs `f` on the state. A poisoned lock (a panic elsewhere while holding it) reads as a
    /// closed hub: nothing more is delivered.
    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> Option<T> {
        self.0.state.lock().ok().map(|mut state| f(&mut state))
    }
}

/// Whether the script listens to `event` on `guid`, under the hub's lock.
fn listens(state: &State, guid: &str, event: &str) -> bool {
    state
        .subscriptions
        .iter()
        .any(|(listening, name)| listening == guid && name == event)
}

fn drop_note(dropped: usize) -> String {
    let events = if dropped == 1 { "event" } else { "events" };
    format!(
        "neo-runtime dropped {dropped} {events}: a run holds at most {MAX_QUEUED_EVENTS} \
         undelivered events or {} MiB of them until the script takes them",
        MAX_QUEUED_BYTES >> 20
    )
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use tokio::task;

    use super::*;

    fn drained(hub: &EventHub, until: Option<u64>) -> Vec<Value> {
        serde_json::from_str::<Vec<Value>>(&hub.drain(until)).unwrap()
    }

    #[test]
    fn tests_that_events_drain_in_seq_order_up_to_a_watermark() {
        let hub = EventHub::default();
        for text in ["one", "two", "three"] {
            assert_eq!(
                hub.emit(CONTEXT_GUID, "console", &json!({ "text": text })),
                Emitted::Queued
            );
        }
        assert_eq!(hub.seq(), 3);
        let first = drained(&hub, Some(2));
        assert_eq!(
            first
                .iter()
                .map(|event| event["params"]["text"].clone())
                .collect::<Vec<_>>(),
            [json!("one"), json!("two")]
        );
        assert_eq!(first[0]["seq"], 1);
        assert_eq!(first[0]["guid"], CONTEXT_GUID);
        assert_eq!(first[0]["method"], "console");
        let rest = drained(&hub, None);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0]["seq"], 3);
        assert!(drained(&hub, None).is_empty());
    }

    #[test]
    fn tests_that_a_full_queue_counts_drops_and_notes_them_once_per_period() {
        let hub = EventHub::default();
        for _ in 0..MAX_QUEUED_EVENTS {
            assert_eq!(hub.emit("page@T", "close", &json!({})), Emitted::Queued);
        }
        assert_eq!(hub.emit("page@T", "close", &json!({})), Emitted::Dropped);
        assert_eq!(hub.emit("page@T", "close", &json!({})), Emitted::Dropped);
        assert!(
            hub.take_notes().is_empty(),
            "the period lasts until a drain"
        );
        assert_eq!(drained(&hub, None).len(), MAX_QUEUED_EVENTS);
        let notes = hub.take_notes();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("dropped 2 events"), "{notes:?}");
        assert_eq!(hub.emit("page@T", "close", &json!({})), Emitted::Queued);
        assert!(hub.take_notes().is_empty());
    }

    #[test]
    fn tests_that_a_big_payload_is_bounded_by_bytes() {
        let hub = EventHub::default();
        let big = "x".repeat(MAX_QUEUED_BYTES);
        assert_eq!(
            hub.emit(CONTEXT_GUID, "console", &json!({ "text": big })),
            Emitted::Dropped
        );
        assert_eq!(hub.seq(), 0, "a dropped event takes no seq");
    }

    #[test]
    fn tests_that_a_created_contexts_page_answers_to_its_own_contexts_listeners() {
        let hub = EventHub::default();
        hub.set_owner("page@C", "context@C1");
        // Every context-level event subscribes on a created context, as on the run's own.
        for (name, wanted) in CONTEXT_EVENTS {
            assert_eq!(needs("context@C1", name), Some(wanted), "{name}");
        }
        hub.subscribe(CONTEXT_GUID, "console", true);
        assert!(hub.wants("page@T", "console", "console"));
        assert!(
            !hub.wants("page@C", "console", "console"),
            "the run's own context hears none of a created context's pages"
        );
        hub.subscribe("context@C1", "console", true);
        assert!(hub.wants("page@C", "console", "console"));
        assert_eq!(hub.context_needs("context@C1"), Some(Needs::RUNTIME));
        hub.subscribe(CONTEXT_GUID, "console", false);
        assert!(!hub.wants("page@T", "console", "console"));
        assert!(hub.wants("page@C", "console", "console"));
    }

    #[test]
    fn tests_that_subscriptions_route_page_events_to_the_page_or_the_context() {
        let hub = EventHub::default();
        assert!(!hub.wants("page@T", "console", "console"));
        hub.subscribe(CONTEXT_GUID, "weberror", true);
        assert!(hub.wants("page@T", "pageerror", "weberror"));
        assert!(!hub.wants("page@T", "console", "console"));
        hub.subscribe("page@T", "console", true);
        assert!(hub.wants("page@T", "console", "console"));
        assert!(!hub.wants("page@U", "console", "console"));
        assert_eq!(hub.context_needs(CONTEXT_GUID), Some(Needs::RUNTIME));
        hub.subscribe("page@T", "console", false);
        assert!(!hub.wants("page@T", "console", "console"));
    }

    #[test]
    fn tests_that_needs_names_only_the_events_neo_produces() {
        assert_eq!(needs("page@T", "console"), Some(Needs::RUNTIME));
        assert_eq!(needs("page@T", "dialog"), Some(Needs::WATCHER));
        assert_eq!(needs(CONTEXT_GUID, "weberror"), Some(Needs::RUNTIME));
        assert_eq!(needs(CONTEXT_GUID, "pageerror"), None);
        assert_eq!(needs("page@T", "request"), Some(Needs::NETWORK));
        assert_eq!(needs(CONTEXT_GUID, "requestfailed"), Some(Needs::NETWORK));
        assert_eq!(needs("frame@T", "console"), None);
        assert_eq!(needs("page@T", "popup"), Some(Needs::DISCOVERY));
        assert_eq!(needs(CONTEXT_GUID, "page"), Some(Needs::DISCOVERY));
        assert_eq!(needs(CONTEXT_GUID, "pageclose"), Some(Needs::WATCHER));
        assert_eq!(needs("page@T", "framenavigated"), Some(Needs::WATCHER));
        assert_eq!(needs(CONTEXT_GUID, "domcontentloaded"), None);
        assert!(
            !Needs::DISCOVERY.of_page(),
            "discovery switches nothing on in a page"
        );
    }

    #[test]
    fn tests_that_an_enable_for_all_pages_holds_every_page_and_the_context() {
        let hub = EventHub::default();
        let enabling = hub.begin_enable(ALL_PAGES);
        assert!(hub.enabling("T"), "a page's call waits for it");
        assert!(hub.enabling(ALL_PAGES), "a context call waits for it");
        drop(enabling);
        assert!(!hub.enabling("T") && !hub.enabling(ALL_PAGES));
    }

    #[test]
    fn tests_that_closing_the_hub_ends_its_producers() {
        let hub = EventHub::default();
        let ended = hub.ended();
        assert!(!ended.is_cancelled());
        hub.close();
        assert!(ended.is_cancelled());
    }

    #[test]
    fn tests_that_closing_ends_subscriptions_and_refuses_events() {
        let hub = EventHub::default();
        hub.subscribe("page@T", "dialog", true);
        hub.emit(CONTEXT_GUID, "dialog", &json!({}));
        hub.close();
        assert!(!hub.wants("page@T", "dialog", "dialog"));
        assert_eq!(
            hub.emit(CONTEXT_GUID, "dialog", &json!({})),
            Emitted::Closed
        );
        assert!(drained(&hub, None).is_empty());
        hub.subscribe("page@T", "dialog", true);
        assert!(!hub.subscribed("page@T", "dialog"));
    }

    #[tokio::test]
    async fn tests_that_next_wakes_when_an_event_arrives() {
        let hub = EventHub::default();
        let waiting = tokio::spawn({
            let hub = hub.clone();
            async move { hub.next().await }
        });
        task::yield_now().await;
        assert!(!waiting.is_finished());
        hub.emit(CONTEXT_GUID, "console", &json!({}));
        time::timeout(Duration::from_secs(5), waiting)
            .await
            .unwrap()
            .unwrap();
        // Already queued: resolves at once.
        time::timeout(Duration::from_secs(5), hub.next())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn tests_that_a_flush_waits_for_the_watcher_to_answer() {
        let hub = EventHub::default();
        let mut flushes = hub.watcher();
        hub.subscribe("page@T", "console", true);
        let barrier = tokio::spawn({
            let hub = hub.clone();
            async move { hub.barrier().await }
        });
        let ack = time::timeout(Duration::from_secs(5), flushes.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(!barrier.is_finished(), "the barrier waits for the ack");
        ack.send(()).unwrap();
        time::timeout(Duration::from_secs(5), barrier)
            .await
            .unwrap()
            .unwrap();
        // A watcher that ended is skipped.
        drop(flushes);
        time::timeout(Duration::from_secs(5), hub.barrier())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn tests_that_no_subscription_skips_the_barrier() {
        let hub = EventHub::default();
        let _flushes = hub.watcher();
        // Nobody answers, yet the barrier is free without a subscription.
        time::timeout(Duration::from_millis(100), hub.barrier())
            .await
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_replay_does_not_wait_on_callbacks_but_call_ordering_does() {
        let hub = EventHub::default();
        let mut producer = hub.watcher();
        let mut callbacks = hub.callback_watcher();
        let replay = tokio::spawn({
            let hub = hub.clone();
            async move { hub.replay_barrier().await }
        });
        let ack = producer.recv().await.expect("replay flushes its producer");
        assert!(
            callbacks.0.try_recv().is_err(),
            "replay cannot wait for a callback preparing its frame"
        );
        ack.send(()).expect("replay still waits for the producer");
        replay
            .await
            .expect("replay finishes without a callback acknowledgement");

        hub.subscribe("page@T", "console", true);
        let ordered = tokio::spawn({
            let hub = hub.clone();
            async move { hub.barrier().await }
        });
        let producer_ack = producer.recv().await.expect("call flushes the producer");
        let callback_ack = callbacks.recv().await.expect("call also flushes callbacks");
        producer_ack.send(()).expect("producer acknowledges");
        assert!(!ordered.is_finished(), "call still waits for its callbacks");
        callback_ack.send(()).expect("callback acknowledges");
        ordered
            .await
            .expect("call ordering finishes after both acknowledgements");
    }

    #[tokio::test]
    async fn tests_that_ready_waits_for_the_pages_enables_only() {
        let hub = EventHub::default();
        let enabling = hub.begin_enable("T");
        time::timeout(Duration::from_millis(100), hub.ready("U"))
            .await
            .unwrap();
        let ready = tokio::spawn({
            let hub = hub.clone();
            async move { hub.ready("T").await }
        });
        task::yield_now().await;
        assert!(!ready.is_finished());
        drop(enabling);
        time::timeout(Duration::from_secs(5), ready)
            .await
            .unwrap()
            .unwrap();
    }
}
