//! The one door between neo-runtime and the browser: CDP commands and events cross here, typed.
//!
//! ```text
//!  every caller in neo-runtime
//!    ├─ cdp::Browser::send(command)    browser-wide: `Browser.*`, `Target.*`, …, no session
//!    ├─ cdp::Browser::attach(target)   a `cdp::Session` on a tab, bound to the socket
//!    ├─ cdp::Session::send(command)    on a tab's or a frame's session: `Page.*`, `DOM.*`, …
//!    ├─ cdp::Session::write(&input, &ticket)   input, at the session's socket incarnation
//!    ├─ cdp::Session::release(&release)        a run end's releases, at that incarnation
//!    └─ cdp::{Browser, Session}::events()      the client's event ring, as `Raw`
//!         browseros_cdp::CdpClient     transport: one socket, one reader, a FIFO send lock
//! ```
//!
//! A command is declared with [`command!`] in `cdp/<domain>.rs`, on browseros-cdp's params type
//! for its method under the method's name (`browseros_cdp::page::NavigateParams` as
//! `page::Navigate`): its `METHOD`, its `Reply` (the generated `…Result`, `serde_json::Value`
//! where several readers walk the JSON, or [`Ignored`]), and `Via`, which picks the one method
//! of the door that may send it. [`Command`] is sealed, so no type outside `cdp/` becomes a
//! command, and no caller writes a method string: an error that names a method reads the
//! command's `METHOD`.
//!
//! Input is written only at the session's socket incarnation and only with the call's delivery
//! record in hand. After a reconnect a write is refused before any byte goes out
//! (`ConnectionEpochChanged`), which is what lets a writer prove `notSent`.
//!
//! An event is declared with [`event!`], on browseros-cdp's generated `…Event` type. Events and
//! replies decode with every field the protocol requires, as Chromium's builders always send
//! them; an event that does not decode reads as `None` and is logged as a warning, since a
//! dropped `frameNavigated` is a protocol skew worth seeing ([`Raw::decode`]). The types come
//! from browseros-cdp's pinned `protocol/protocol.json`. The door's tests check every
//! declaration against it, and `tests/protocol_e2e.rs` checks it against the browser's own
//! protocol.
//!
//! A domain file keeps a local type only where its doc says why: a command that takes no params
//! (browseros-cdp generates no type for one), a second declaration of a method, a write whose
//! field order or `Debug` output matters, a payload the generated type would lose
//! (`Network.Headers`, which browseros-cdp generates as an empty struct), or one Neo already
//! reads through its own type (the console's remote objects, which keep `"value": null` apart
//! from no value).
//!
//! Neo: Neo's own door. It types every send by its method, as Playwright's `CRSession.send`
//! does; browseros-cdp's generated `pub async fn`s take a concrete client and cannot bind a write
//! to the socket a session belongs to.
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the CDP door owns browseros-cdp's client, raw sends and event decoding"
)]

pub(crate) mod browser;
pub(crate) mod dom;
pub(crate) mod dom_storage;
pub(crate) mod emulation;
pub(crate) mod fetch;
pub(crate) mod heap_profiler;
pub(crate) mod input;
pub(crate) mod inspector;
pub(crate) mod io;
pub(crate) mod log;
pub(crate) mod network;
pub(crate) mod page;
pub(crate) mod runtime;
pub(crate) mod security;
pub(crate) mod storage;
pub(crate) mod target;

use std::fmt;
use std::future::Future;
use std::marker::PhantomData;

use browseros_cdp::{CdpClient, CdpError, CdpEvent, SessionId};
use serde::Serialize;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tracing::warn;

use crate::progress::InputTicket;

/// A reply Neo never reads. It decodes from any JSON, so ignoring a reply cannot fail.
pub(crate) type Ignored = IgnoredAny;

/// Private to `cdp`: only `command!` and `event!`, expanded under `cdp/`, can name it.
mod sealed {
    pub trait Sealed {}
}

/// Where a command goes, and so which method of the door compiles for it. Sealed.
pub(crate) trait Via: sealed::Sealed {}

/// Sent with no session, to the browser target: the `Browser` and `Target` domains, and the
/// browser-wide commands of `Storage`.
pub(crate) enum OnBrowser {}

/// Sent on a tab's or an out-of-process frame's session.
pub(crate) enum OnSession {}

/// A write to the page whose delivery a call records: the `Input.*` commands that act on the
/// page. Written on the tab's session at its socket incarnation.
pub(crate) enum Write {}

/// A release of input a run left held: a key's `keyUp`, a button's `mouseReleased`, a drag's
/// `dragCancel`. A call writes one under its delivery record like any write; a run's end writes
/// one with no record at all ([`Session::release`]), since no call wrote what it releases.
pub(crate) enum Release {}

impl sealed::Sealed for OnBrowser {}
impl Via for OnBrowser {}
impl sealed::Sealed for OnSession {}
impl Via for OnSession {}
impl sealed::Sealed for Write {}
impl Via for Write {}
impl sealed::Sealed for Release {}
impl Via for Release {}

/// A CDP command neo-runtime sends, declared with [`command!`] in `cdp/<domain>.rs`.
pub(crate) trait Command: Serialize + sealed::Sealed {
    /// `Domain.method`, as the protocol names it.
    const METHOD: &'static str;
    /// The reply: the generated `…Result`, `serde_json::Value` where several readers walk the
    /// JSON, or [`Ignored`].
    type Reply: DeserializeOwned;
    /// [`OnBrowser`], [`OnSession`] or [`Write`]: which method of the door sends it.
    type Via: Via;
}

/// Declares a domain file's commands, one entry per command in method order:
/// `command! { Type => "Domain.method", Via, Reply; … }`. The expansion implements the private
/// `sealed` trait, so it compiles only under `cdp/`, and lists the entries for the door's tests,
/// so a domain file holds one `command!`.
macro_rules! command {
    ($($command:ty => $method:literal, $via:ty, $reply:ty;)+) => {
        $(
            impl $crate::chromium_bridge::cdp::sealed::Sealed for $command {}

            impl $crate::chromium_bridge::cdp::Command for $command {
                const METHOD: &'static str = $method;
                type Reply = $reply;
                type Via = $via;
            }
        )+

        #[cfg(test)]
        pub(super) fn commands() -> Vec<$crate::chromium_bridge::cdp::tests::Declared> {
            vec![$($crate::chromium_bridge::cdp::tests::Declared::command::<$command>()),+]
        }
    };
}
pub(crate) use command;

/// A CDP event neo-runtime reads, declared with [`event!`] in `cdp/<domain>.rs`: the generated
/// `…Event`, every field the protocol requires present, or a local type whose doc says why.
pub(crate) trait Event: DeserializeOwned + sealed::Sealed {
    /// `Domain.event`, as the protocol names it.
    const METHOD: &'static str;
}

/// Declares a domain file's events, one entry per event in method order:
/// `event! { Type => "Domain.event"; … }`. Like `command!`, it compiles only under `cdp/` and
/// lists its entries for the door's tests.
macro_rules! event {
    ($($event:ty => $method:literal;)+) => {
        $(
            impl $crate::chromium_bridge::cdp::sealed::Sealed for $event {}

            impl $crate::chromium_bridge::cdp::Event for $event {
                const METHOD: &'static str = $method;
            }
        )+

        #[cfg(test)]
        pub(super) fn events() -> Vec<$crate::chromium_bridge::cdp::tests::Declared> {
            vec![$($crate::chromium_bridge::cdp::tests::Declared::event::<$event>()),+]
        }
    };
}
pub(crate) use event;

/// A command serialized when its send is made: its JSON, or why it would not serialize, typed
/// by the command so the reply still decodes as declared. Only [`Serialized::new`] makes one.
///
/// A send's future holds this, not the command. A generated params struct runs to a few hundred
/// bytes, and a command held by a future is held twice there (an `async fn`'s arguments live on
/// both as its captures and as its locals), then again by each future that awaits it, up call
/// paths already near clippy's `large_futures` bound (`method_router.rs` `serve_call`).
pub(crate) struct Serialized<C> {
    params: Result<Value, serde_json::Error>,
    command: PhantomData<fn() -> C>,
}

impl<C: Command> Serialized<C> {
    pub(crate) fn new(command: C) -> Self {
        Self {
            params: serde_json::to_value(command),
            command: PhantomData,
        }
    }
}

/// One page write as the input writer plans it: a [`Write`] command's method and its params,
/// serialized now as the command's struct declares, so key and touch events keep their bytes.
/// Only [`Input::new`] makes one and only [`Session::write`] sends one. Planned before anything
/// is written, it is read to meet a drag and has the held modifiers and buttons patched in after
/// a stop ([`Input::with_number`]); nothing else changes it. Its `Debug` leaves the params out:
/// typed text may be a secret.
#[derive(Clone, PartialEq)]
pub(crate) struct Input {
    method: &'static str,
    params: String,
}

impl Input {
    /// `command`, serialized now. Serializing these plain structs cannot fail; if it ever did,
    /// Chromium would refuse the empty params, and the write counts as failed.
    pub(crate) fn new<C: Command<Via = Write>>(command: &C) -> Self {
        Self {
            method: C::METHOD,
            params: serde_json::to_string(command).unwrap_or_default(),
        }
    }

    pub(crate) fn method(&self) -> &'static str {
        self.method
    }

    /// The params as they go out, for the writer to read a planned event's fields.
    pub(crate) fn params(&self) -> &str {
        &self.params
    }

    /// This write with the number its `name` field holds replaced by `value`, in place, so the
    /// params keep their field order; unchanged when it has no such field.
    #[must_use]
    pub(crate) fn with_number(&self, name: &str, value: i64) -> Self {
        let key = format!("\"{name}\":");
        let Some(start) = self.params.find(&key).map(|at| at + key.len()) else {
            return self.clone();
        };
        let end = self.params[start..]
            .find(|character: char| !(character.is_ascii_digit() || character == '-'))
            .map_or(self.params.len(), |length| start + length);
        Self {
            method: self.method,
            params: format!("{}{value}{}", &self.params[..start], &self.params[end..]),
        }
    }
}

/// A planned [`Release`]: its method and params, serialized now as [`Input`] serializes a write.
/// Only [`ReleaseInput::new`] makes one, from a `Release` command, whose constructor writes only
/// that release, so [`Session::release`], the one write that takes no delivery record, can write
/// nothing else.
#[derive(Clone, PartialEq)]
pub(crate) struct ReleaseInput(Input);

impl ReleaseInput {
    /// `command`, serialized now ([`Input::new`]).
    pub(crate) fn new<C: Command<Via = Release>>(command: &C) -> Self {
        Self(Input {
            method: C::METHOD,
            params: serde_json::to_string(command).unwrap_or_default(),
        })
    }

    /// The release as a write, for a call that writes it under its own delivery record.
    pub(crate) fn input(&self) -> &Input {
        &self.0
    }

    /// [`Input::with_number`]: the held modifiers and buttons patched in, still this release.
    #[must_use]
    pub(crate) fn with_number(&self, name: &str, value: i64) -> Self {
        Self(self.0.with_number(name, value))
    }
}

impl fmt::Debug for ReleaseInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ReleaseInput")
            .field(&self.0)
            .finish()
    }
}

impl fmt::Debug for Input {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Input")
            .field("method", &self.method)
            .finish_non_exhaustive()
    }
}

/// The browser end of the door, over the `Runtime`'s one connection. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Browser {
    client: CdpClient,
}

impl Browser {
    pub(crate) fn new(client: CdpClient) -> Self {
        Self { client }
    }

    /// Whether the connection is up now: Playwright's `browser.isConnected()`.
    pub(crate) fn is_connected(&self) -> bool {
        self.client.is_connected()
    }

    /// A session the door did not attach: one Chromium auto-attached under a session Neo holds
    /// (`Target.attachedToTarget`), or a tab's or frame's session handed around by its id.
    ///
    /// Its socket incarnation is read now, not at the attach, so adopt a session as soon as it
    /// is known: one adopted after a reconnect would bind its writes to the new socket. Only a
    /// session attached through [`Self::attach`] writes input.
    pub(crate) fn adopt(&self, id: SessionId) -> Session {
        Session {
            browser: self.clone(),
            epoch: self.client.epoch(),
            id,
        }
    }

    /// Sends a browser-wide command and decodes its reply. The command is serialized now, when
    /// the send is made, and the future holds only its JSON ([`Serialized`]).
    pub(crate) fn send<C: Command<Via = OnBrowser>>(
        &self,
        command: C,
    ) -> impl Future<Output = Result<C::Reply, CdpError>> {
        self.send_serialized(Serialized::new(command))
    }

    async fn send_serialized<C: Command<Via = OnBrowser>>(
        &self,
        command: Serialized<C>,
    ) -> Result<C::Reply, CdpError> {
        let reply = self.client.send(C::METHOD, command.params?, None).await?;
        serde_json::from_value(reply).map_err(CdpError::from)
    }

    /// Attaches a session to `target` with `Target.attachToTarget {targetId, flatten: true}`.
    /// The socket incarnation is read just before the attach is sent: the session id belongs to
    /// that socket, so input written under it after a reconnect is refused instead of reaching
    /// a session the new socket never had.
    pub(crate) async fn attach(&self, target: &str) -> Result<Session, CdpError> {
        let epoch = self.client.epoch();
        let attached = self
            .send(target::AttachToTarget {
                target_id: target.to_string(),
                flatten: Some(true),
            })
            .await?;
        Ok(Session {
            browser: self.clone(),
            id: SessionId::from(attached.session_id),
            epoch,
        })
    }

    /// Every event on the client's ring from now on: one 4096-slot broadcast shared by the
    /// whole connection, the browser target's own events (`Target.targetCreated`) and every
    /// session's.
    pub(crate) fn events(&self) -> Events {
        Events(self.client.events())
    }
}

/// One run's session on one tab or out-of-process frame: its id and the socket incarnation it
/// belongs to. Cheap to clone, and it does not own the attachment: the session's owner detaches.
#[derive(Clone)]
pub(crate) struct Session {
    browser: Browser,
    id: SessionId,
    /// browseros-cdp's socket incarnation when the attach was sent ([`Browser::attach`]).
    epoch: u64,
}

impl Session {
    pub(crate) fn id(&self) -> &SessionId {
        &self.id
    }

    /// The browser end, for the browser-wide commands a page sends (`Target.getTargets`).
    pub(crate) fn browser(&self) -> &Browser {
        &self.browser
    }

    /// Sends a command on this session and decodes its reply. The command is serialized now, when
    /// the send is made ([`Serialized`]).
    pub(crate) fn send<C: Command<Via = OnSession>>(
        &self,
        command: C,
    ) -> impl Future<Output = Result<C::Reply, CdpError>> {
        self.send_serialized(Serialized::new(command))
    }

    /// [`Self::send`] of a command serialized earlier, for a caller that races the send against
    /// something else: it serializes before its own future, then makes this one inside it, so
    /// neither holds the command.
    pub(crate) async fn send_serialized<C: Command<Via = OnSession>>(
        &self,
        command: Serialized<C>,
    ) -> Result<C::Reply, CdpError> {
        let reply = self
            .browser
            .client
            .send(C::METHOD, command.params?, Some(&self.id))
            .await?;
        serde_json::from_value(reply).map_err(CdpError::from)
    }

    /// Writes `input` to the page at the session's socket incarnation, its params as planned
    /// (key events keep Playwright's field order). It takes the call's delivery record, so a
    /// page write without one does not compile.
    ///
    /// What each outcome proves, for that record:
    /// - `Err(ConnectionEpochChanged)`: refused before any byte went out, because the socket
    ///   was replaced. It is the only proof of `notSent`.
    /// - `Ok`: the browser answered.
    /// - any other error, and a future dropped before the answer: `unknown`, never retried.
    ///
    /// Nothing awaits before the client's write, so a `join_all` over a click's events writes
    /// them in order behind browseros-cdp's FIFO send lock. A writer that must give up its input
    /// turn once the frame is written, before the answer (navigation, as neo-nav-history plans
    /// it), needs this split in two: a `dispatch` that resolves on the write, and the pending
    /// answer. That split needs browseros-cdp to expose the write stage of its send.
    pub(crate) async fn write(
        &self,
        input: &Input,
        _delivery: &InputTicket,
    ) -> Result<(), CdpError> {
        self.browser
            .client
            .send_raw_json_at_epoch(self.epoch, input.method, &input.params, Some(&self.id))
            .await
            .map(drop)
    }

    /// Writes a run end's release at the session's socket incarnation, with no delivery record:
    /// no call wrote what it releases, so none has a delivery to record it in. Only a
    /// [`ReleaseInput`] goes out so, which only a [`Release`] command makes, so every other page
    /// write still needs its call's record. A refused release (`ConnectionEpochChanged`) went
    /// nowhere, as a refused write did ([`Self::write`]).
    pub(crate) async fn release(&self, release: &ReleaseInput) -> Result<(), CdpError> {
        let input = &release.0;
        self.browser
            .client
            .send_raw_json_at_epoch(self.epoch, input.method, &input.params, Some(&self.id))
            .await
            .map(drop)
    }

    /// Every event on the client's ring from now on, every session's included
    /// ([`Browser::events`]). Subscribe before the command whose events you need, as a watcher
    /// subscribes before `Page.enable`.
    pub(crate) fn events(&self) -> Events {
        self.browser.events()
    }

    /// `Target.detachFromTarget` for this session, owning all it needs: the session's owner
    /// spawns it on drop, best effort.
    pub(crate) fn detach(&self) -> impl Future<Output = Result<(), CdpError>> + Send + 'static {
        let browser = self.browser.clone();
        let session_id = self.id.as_str().to_string();
        async move {
            browser
                .send(target::DetachFromTarget {
                    session_id: Some(session_id),
                    target_id: None,
                })
                .await
                .map(drop)
        }
    }
}

/// A subscription to the client's event ring. `Lagged` and `Closed` pass through unchanged.
pub(crate) struct Events(broadcast::Receiver<CdpEvent>);

impl Events {
    /// The next event; cancel-safe, as the ring's own `recv` is.
    pub(crate) async fn recv(&mut self) -> Result<Raw, RecvError> {
        self.0.recv().await.map(Raw)
    }

    pub(crate) fn try_recv(&mut self) -> Result<Raw, TryRecvError> {
        self.0.try_recv().map(Raw)
    }

    /// A subscription of its own from the ring's next event on, as `broadcast::Receiver`'s.
    pub(crate) fn resubscribe(&self) -> Self {
        Self(self.0.resubscribe())
    }
}

/// One event off the ring, not decoded yet.
pub(crate) struct Raw(CdpEvent);

impl Raw {
    /// Whether the event came on `session`. The root session's `Target.detachedFromTarget` does
    /// not: it names the detached session in its params.
    pub(crate) fn on_session(&self, session: &SessionId) -> bool {
        self.0.session_id.as_ref() == Some(session)
    }

    /// Whether the event is one of `domain`'s (`Network`), whether or not Neo declares it.
    pub(crate) fn in_domain(&self, domain: &str) -> bool {
        self.0
            .method
            .strip_prefix(domain)
            .is_some_and(|rest| rest.starts_with('.'))
    }

    /// The session the event came on; `None` for the browser target's own events.
    pub(crate) fn session(&self) -> Option<&SessionId> {
        self.0.session_id.as_ref()
    }

    /// Whether this is an `E`, whether or not its params decode.
    pub(crate) fn is<E: Event>(&self) -> bool {
        self.0.method == E::METHOD
    }

    /// `Some` when this is an `E` and its params decode. A known event that does not decode
    /// reads as `None` and is logged as a warning: a protocol skew to see, not to drop quietly.
    pub(crate) fn decode<E: Event>(&self) -> Option<E> {
        if !self.is::<E>() {
            return None;
        }
        match E::deserialize(&self.0.params) {
            Ok(decoded) => Some(decoded),
            Err(error) => {
                warn!(method = E::METHOD, %error, "a CDP event did not decode; ignoring it");
                None
            }
        }
    }
}

#[cfg(test)]
impl Raw {
    /// An event as the client delivers one. A declared event's params must decode as each type
    /// declared for its method, or this panics: a fixture the runtime would drop as malformed
    /// ([`Raw::decode`]) would let a test pass on an event nothing read.
    pub(crate) fn fixture(method: &str, session: Option<&str>, params: serde_json::Value) -> Self {
        tests::check_fixture(method, &params);
        Self::malformed(method, session, params)
    }

    /// [`Raw::fixture`] unchecked, for a test of what a malformed event does.
    pub(crate) fn malformed(
        method: &str,
        session: Option<&str>,
        params: serde_json::Value,
    ) -> Self {
        Self(CdpEvent {
            method: method.to_string(),
            params,
            session_id: session.map(SessionId::from),
        })
    }
}

#[cfg(test)]
impl Browser {
    /// Drops the connection, as a browser that went away does: for the tests of what a call
    /// does once its browser is gone.
    pub(crate) async fn disconnect(&self) {
        self.client.disconnect().await;
    }
}

/// A ring that tests feed events into, in place of the client's.
#[cfg(test)]
pub(crate) struct Ring(broadcast::Sender<CdpEvent>);

#[cfg(test)]
impl Ring {
    pub(crate) fn new(capacity: usize) -> (Self, Events) {
        let (sender, receiver) = broadcast::channel(capacity);
        (Self(sender), Events(receiver))
    }

    pub(crate) fn send(&self, event: Raw) -> Result<usize, broadcast::error::SendError<CdpEvent>> {
        self.0.send(event.0)
    }

    /// Another subscription to this ring, from its next event on.
    pub(crate) fn subscribe(&self) -> Events {
        Events(self.0.subscribe())
    }
}

#[cfg(test)]
mod tests {
    use std::any;
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;

    use serde_json::{Value, json};

    use super::*;

    /// An event type's decode of a payload, as [`Raw::decode`] makes it.
    type Decodes = fn(&Value) -> Result<(), serde_json::Error>;

    /// One `command!` or `event!` entry: its method, and the Rust type declared for it.
    pub(crate) struct Declared {
        method: &'static str,
        type_name: &'static str,
        /// An event's decode of a payload; `None` for a command.
        decodes: Option<Decodes>,
    }

    impl Declared {
        pub(crate) fn command<C: Command>() -> Self {
            Self {
                method: C::METHOD,
                type_name: any::type_name::<C>(),
                decodes: None,
            }
        }

        pub(crate) fn event<E: Event>() -> Self {
            Self {
                method: E::METHOD,
                type_name: any::type_name::<E>(),
                decodes: Some(|params| E::deserialize(params).map(drop)),
            }
        }
    }

    /// Panics unless `params` decode as every event type declared for `method` ([`Raw::fixture`]).
    pub(super) fn check_fixture(method: &str, params: &Value) {
        for (_, _, events) in domains() {
            for declared in events.iter().filter(|declared| declared.method == method) {
                if let Some(Err(error)) = declared.decodes.map(|decodes| decodes(params)) {
                    panic!(
                        "a {method} fixture does not decode as {}: {error}",
                        declared.type_name
                    );
                }
            }
        }
    }

    /// Each domain file under `cdp/`, by name, with its commands and its events.
    fn domains() -> Vec<(&'static str, Vec<Declared>, Vec<Declared>)> {
        vec![
            ("browser", browser::commands(), browser::events()),
            ("dom", dom::commands(), Vec::new()),
            ("dom_storage", dom_storage::commands(), Vec::new()),
            ("emulation", emulation::commands(), Vec::new()),
            ("fetch", fetch::commands(), fetch::events()),
            ("heap_profiler", heap_profiler::commands(), Vec::new()),
            ("input", input::commands(), input::events()),
            ("inspector", Vec::new(), inspector::events()),
            ("io", io::commands(), Vec::new()),
            ("log", log::commands(), log::events()),
            ("network", network::commands(), network::events()),
            ("page", page::commands(), page::events()),
            ("runtime", runtime::commands(), runtime::events()),
            ("security", security::commands(), Vec::new()),
            ("storage", storage::commands(), Vec::new()),
            ("target", target::commands(), target::events()),
        ]
    }

    /// A protocol domain's name as browseros-cdp names its generated module
    /// (`build.rs` `to_snake`): `DOMStorage` is `dom_storage`, `IO` is `io`.
    fn module_name(domain: &str) -> String {
        let chars: Vec<char> = domain.chars().collect();
        let mut out = String::new();
        for (index, ch) in chars.iter().enumerate() {
            if ch.is_ascii_uppercase() {
                let lower_before = index > 0 && chars[index - 1].is_ascii_lowercase();
                let lower_after = chars.get(index + 1).is_some_and(char::is_ascii_lowercase);
                if index > 0 && (lower_before || lower_after) {
                    out.push('_');
                }
                out.push(ch.to_ascii_lowercase());
            } else {
                out.push(*ch);
            }
        }
        out
    }

    /// A file of this crate's checkout, read at run time.
    fn crate_file(path: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    }

    #[test]
    fn module_names_follow_browseros_cdps_generated_modules() {
        let names: Vec<String> = ["DOM", "DOMStorage", "HeapProfiler", "IO", "Page"]
            .into_iter()
            .map(module_name)
            .collect();
        assert_eq!(
            names,
            ["dom", "dom_storage", "heap_profiler", "io", "page"],
            "domain names snake-case as browseros-cdp's build.rs does"
        );
    }

    #[test]
    fn every_domain_file_is_checked() {
        let files: BTreeSet<String> =
            fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/chromium_bridge/cdp"))
                .expect("src/chromium_bridge/cdp")
                .map(|entry| {
                    let path = entry.expect("a directory entry").path();
                    path.file_stem()
                        .expect("a file name")
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
        let checked: BTreeSet<String> = domains()
            .into_iter()
            .map(|(file, _, _)| file.to_string())
            .collect();
        assert_eq!(
            files, checked,
            "every cdp/<domain>.rs is listed in `domains`"
        );
    }

    #[test]
    fn every_declaration_is_a_method_of_its_domain_in_the_protocol() {
        let protocol: Value =
            serde_json::from_str(&crate_file("../browseros-cdp/protocol/protocol.json"))
                .expect("browseros-cdp's protocol.json parses");
        let names = |domain: &Value, kind: &str| -> BTreeSet<String> {
            domain[kind]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| item["name"].as_str())
                .map(str::to_string)
                .collect()
        };
        let known: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = protocol["domains"]
            .as_array()
            .expect("protocol domains")
            .iter()
            .map(|domain| {
                let name = domain["domain"].as_str().unwrap_or_default().to_string();
                (name, (names(domain, "commands"), names(domain, "events")))
            })
            .collect();
        for (file, commands, events) in domains() {
            let declared = commands.iter().map(|command| (command, "commands"));
            for (declaration, kind) in declared.chain(events.iter().map(|event| (event, "events")))
            {
                let (domain, name) = declaration.method.split_once('.').expect("Domain.method");
                assert_eq!(
                    module_name(domain),
                    file,
                    "{} is declared in the file of its domain",
                    declaration.method
                );
                let (protocol_commands, protocol_events) = known
                    .get(domain)
                    .unwrap_or_else(|| panic!("{domain} is a domain of the protocol"));
                let listed = if kind == "commands" {
                    protocol_commands
                } else {
                    protocol_events
                };
                assert!(
                    listed.contains(name),
                    "{} is not among the protocol's {kind}",
                    declaration.method
                );
            }
        }
    }

    #[test]
    fn generated_params_are_their_methods_own() {
        for (file, commands, _) in domains() {
            for command in commands {
                if !command.type_name.starts_with("browseros_cdp::") {
                    continue;
                }
                let (_, name) = command.method.split_once('.').expect("Domain.method");
                let mut letters = name.chars();
                let first = letters.next().expect("a method name").to_ascii_uppercase();
                let expected = format!(
                    "browseros_cdp::generated::{file}::{first}{}Params",
                    letters.as_str()
                );
                assert_eq!(
                    command.type_name, expected,
                    "{} is sent with its own generated params",
                    command.method
                );
            }
        }
    }

    #[test]
    fn declarations_are_in_method_order() {
        for (file, commands, events) in domains() {
            for declared in [commands, events] {
                let methods: Vec<&str> = declared.iter().map(|declared| declared.method).collect();
                assert!(
                    methods.is_sorted(),
                    "cdp/{file}.rs declares in method order: {methods:?}"
                );
            }
        }
    }

    #[test]
    fn the_client_ban_names_every_way_to_send_and_subscribe() {
        // Read the transport's current API at run time: a new raw entry point must extend
        // the ban instead of silently bypassing this module's typed commands and events.
        let client = crate_file("../browseros-cdp/src/client.rs");
        let banned = crate_file("clippy.toml");
        let ways: Vec<&str> = client
            .lines()
            .map(str::trim_start)
            .filter_map(|line| {
                line.strip_prefix("pub async fn ")
                    .or_else(|| line.strip_prefix("pub fn "))
            })
            .filter_map(|signature| signature.split(['(', '<']).next())
            .filter(|name| {
                ["send", "dispatch", "events", "on_event", "subscribe"]
                    .iter()
                    .any(|way| name.starts_with(way))
            })
            .collect();
        assert!(
            ways.contains(&"send") && ways.contains(&"events"),
            "client.rs's API was read: {ways:?}"
        );
        for way in ways {
            assert!(
                banned.contains(&format!("\"browseros_cdp::CdpClient::{way}\"")),
                "clippy.toml denies CdpClient::{way} outside chromium_bridge/cdp.rs"
            );
        }
        assert!(
            banned.contains("\"browseros_cdp::CdpEvent\""),
            "clippy.toml denies raw CdpEvents outside chromium_bridge/cdp.rs"
        );
        assert!(
            banned.contains("\"browseros_cdp::CdpClient\""),
            "clippy.toml denies client holders outside chromium_bridge/cdp.rs"
        );
    }

    #[test]
    fn events_decode_with_what_the_pinned_protocol_requires() {
        let frame = json!({
            "id": "F", "loaderId": "L", "url": "https://a.test/", "domainAndRegistry": "a.test",
            "securityOrigin": "https://a.test", "mimeType": "text/html",
            "secureContextType": "Secure", "crossOriginIsolatedContextType": "NotIsolated",
            "gatedAPIFeatures": [],
        });
        let commit = Raw::fixture(
            "Page.frameNavigated",
            Some("S"),
            json!({ "frame": frame, "type": "Navigation" }),
        );
        let decoded = commit
            .decode::<page::FrameNavigated>()
            .expect("a commit with every field the protocol requires decodes");
        assert_eq!(
            (decoded.frame.id.as_str(), decoded.frame.parent_id),
            ("F", None)
        );
        assert!(
            commit.decode::<page::LifecycleEvent>().is_none(),
            "another event's payload never decodes from it"
        );
        let partial = Raw::malformed(
            "Page.frameNavigated",
            Some("S"),
            json!({ "frame": { "id": "F", "url": "https://a.test/" }, "type": "Navigation" }),
        );
        assert!(partial.is::<page::FrameNavigated>());
        assert!(
            partial.decode::<page::FrameNavigated>().is_none(),
            "a known event missing a required field reads as None, logged"
        );
    }

    #[tokio::test]
    async fn events_pass_lag_and_close_through() {
        let (ring, mut events) = Ring::new(1);
        for _ in 0..2 {
            ring.send(Raw::fixture(
                "Inspector.targetCrashed",
                Some("S"),
                json!({}),
            ))
            .expect("subscribed");
        }
        assert!(matches!(events.try_recv(), Err(TryRecvError::Lagged(1))));
        assert!(
            events
                .try_recv()
                .is_ok_and(|event| event.is::<inspector::TargetCrashed>())
        );
        drop(ring);
        assert!(matches!(events.recv().await, Err(RecvError::Closed)));
    }
}
