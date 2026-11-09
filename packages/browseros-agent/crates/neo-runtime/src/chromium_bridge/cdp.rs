//! The one door between neo-runtime and the browser: CDP commands and events cross here, typed.
//!
//! ```text
//!  chromium_bridge/, runtime.rs, method_router.rs
//!    ├─ cdp::Browser::send(command)    browser-wide: `Browser.*`, `Target.*`, …, no session
//!    ├─ cdp::Browser::attach(target)   a `cdp::Session` on a tab, bound to the socket
//!    ├─ cdp::Session::send(command)    on a tab's or a frame's session: `Page.*`, `DOM.*`, …
//!    ├─ cdp::Session::write(&command, &ticket)   input, at the session's socket incarnation
//!    └─ cdp::{Browser, Session}::events()        the client's event ring, as `Raw`
//!         browseros_cdp::CdpClient     transport: one socket, one reader, a FIFO send lock
//! ```
//!
//! A command is a type declared with [`command!`] in `cdp/<domain>.rs`: its `METHOD`, its params
//! as the struct serializes, the `Reply` decoded no stricter than Neo reads it, and `Via`, which
//! picks the one method of the door that may send it. [`Command`] is sealed, so no type outside
//! `cdp/` becomes a command, and no caller writes a method string. `browser_representations/`
//! and `playwright_methods/` still call the client directly; their commands and events are
//! declared here already, so moving them onto the door touches only their own files.
//!
//! Input is written only at the session's socket incarnation and only with the call's delivery
//! record in hand. After a reconnect a write is refused before any byte goes out
//! (`ConnectionEpochChanged`), which is what lets a writer prove `notSent`.
//!
//! An event is a payload declared with [`event!`], holding only the fields Neo reads. Events stay
//! lenient on purpose: one that does not decode reads as `None`, as JavaScript's `undefined`
//! reads in Playwright, and is logged at debug instead of vanishing silently.
//!
//! Neo: Neo's own door. It types every send by its method, as Playwright's `CRSession.send`
//! does; browseros-cdp's generated `pub async fn`s take a concrete client and cannot bind a write
//! to the socket a session belongs to.
#![expect(
    dead_code,
    unused_imports,
    reason = "the door and every declaration land before their callers move onto them"
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

use std::future::Future;

use browseros_cdp::{CdpClient, CdpError, CdpEvent, SessionId};
use serde::Serialize;
use serde::de::{DeserializeOwned, IgnoredAny};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tracing::debug;

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

impl sealed::Sealed for OnBrowser {}
impl Via for OnBrowser {}
impl sealed::Sealed for OnSession {}
impl Via for OnSession {}
impl sealed::Sealed for Write {}
impl Via for Write {}

/// A CDP command neo-runtime sends, declared with [`command!`] in `cdp/<domain>.rs`.
pub(crate) trait Command: Serialize + sealed::Sealed {
    /// `Domain.method`, as the protocol names it.
    const METHOD: &'static str;
    /// The reply, decoded no stricter than Neo reads it: a generated `…Result`, a local struct
    /// of only the fields Neo reads, `serde_json::Value`, or [`Ignored`].
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

/// A CDP event neo-runtime reads, declared with [`event!`] in `cdp/<domain>.rs`. The payload
/// holds only the fields Neo reads, each optional unless Neo cannot act without it.
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

/// The browser end of the door, over the `Runtime`'s one connection. Cheap to clone.
#[derive(Clone)]
pub(crate) struct Browser {
    client: CdpClient,
}

impl Browser {
    pub(crate) fn new(client: CdpClient) -> Self {
        Self { client }
    }

    /// Sends a browser-wide command and decodes its reply. Serializing consumes the command, so
    /// the future holds its JSON across the await, not the command too.
    pub(crate) async fn send<C: Command<Via = OnBrowser>>(
        &self,
        command: C,
    ) -> Result<C::Reply, CdpError> {
        let params = serde_json::to_value(command)?;
        let reply = self.client.send(C::METHOD, params, None).await?;
        serde_json::from_value(reply).map_err(CdpError::from)
    }

    /// Attaches a session to `target` with `Target.attachToTarget {targetId, flatten: true}`.
    /// The socket incarnation is read just before the attach is sent: the session id belongs to
    /// that socket, so input written under it after a reconnect is refused instead of reaching
    /// a session the new socket never had.
    #[expect(
        dead_code,
        reason = "browser_representations' page sessions attach through the door once they move onto it"
    )]
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
    /// A session the door did not attach: one `browser_representations/page.rs` attached itself
    /// (`page::Session::attach`) and hands to the `chromium_bridge` types it builds (the
    /// utility world, the page's event producer, the frames' hosts) as a client and an id, or
    /// one Chromium auto-attached under a session Neo holds (`Target.attachedToTarget`).
    ///
    /// Its socket incarnation is read now, not at the attach, so adopt a session as soon as it
    /// is known: one adopted after a reconnect would bind its writes to the new socket. Those
    /// holders only send and subscribe; input stays on `page::Session` until it attaches
    /// through [`Browser::attach`].
    pub(crate) fn adopt(client: CdpClient, id: SessionId) -> Self {
        let epoch = client.epoch();
        Self {
            browser: Browser::new(client),
            id,
            epoch,
        }
    }

    pub(crate) fn id(&self) -> &SessionId {
        &self.id
    }

    /// The browser end, for the browser-wide commands a page sends (`Target.getTargets`).
    pub(crate) fn browser(&self) -> &Browser {
        &self.browser
    }

    /// Sends a command on this session and decodes its reply ([`Browser::send`]).
    pub(crate) async fn send<C: Command<Via = OnSession>>(
        &self,
        command: C,
    ) -> Result<C::Reply, CdpError> {
        let params = serde_json::to_value(command)?;
        let reply = self
            .browser
            .client
            .send(C::METHOD, params, Some(&self.id))
            .await?;
        serde_json::from_value(reply).map_err(CdpError::from)
    }

    /// Writes `command` to the page at the session's socket incarnation, serialized as its
    /// struct declares (key events keep Playwright's field order), and decodes the answer. It
    /// takes the call's delivery record, so a page write without one does not compile.
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
    #[expect(
        dead_code,
        reason = "page.rs's input writer moves onto the door with browser_representations"
    )]
    pub(crate) async fn write<C: Command<Via = Write>>(
        &self,
        command: &C,
        _delivery: &InputTicket,
    ) -> Result<C::Reply, CdpError> {
        let params = serde_json::to_string(command)?;
        let reply = self
            .browser
            .client
            .send_raw_json_at_epoch(self.epoch, C::METHOD, &params, Some(&self.id))
            .await?;
        serde_json::from_str(&reply).map_err(CdpError::from)
    }

    /// Every event on the client's ring from now on, every session's included
    /// ([`Browser::events`]). Subscribe before the command whose events you need, as a watcher
    /// subscribes before `Page.enable`.
    pub(crate) fn events(&self) -> Events {
        self.browser.events()
    }

    /// `Target.detachFromTarget` for this session, owning all it needs: the session's owner
    /// spawns it on drop, best effort.
    #[expect(
        dead_code,
        reason = "page.rs's session detaches through the door once it attaches through it"
    )]
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
}

/// One event off the ring, not decoded yet.
pub(crate) struct Raw(CdpEvent);

impl Raw {
    /// Whether the event came on `session`. The root session's `Target.detachedFromTarget` does
    /// not: it names the detached session in its params.
    pub(crate) fn on_session(&self, session: &SessionId) -> bool {
        self.0.session_id.as_ref() == Some(session)
    }

    /// The session the event came on; `None` for the browser target's own events.
    pub(crate) fn session(&self) -> Option<&SessionId> {
        self.0.session_id.as_ref()
    }

    /// Whether this is an `E`, whether or not its params decode.
    pub(crate) fn is<E: Event>(&self) -> bool {
        is::<E>(&self.0)
    }

    /// `Some` when this is an `E` and its params decode. A known event that does not decode
    /// reads as `None` and is logged at debug.
    pub(crate) fn decode<E: Event>(&self) -> Option<E> {
        decode(&self.0)
    }
}

/// [`Raw::is`] over an event `browser_representations` read off the ring itself: its watchers
/// still subscribe to the client and hand the `chromium_bridge` consumers a `CdpEvent`. Goes
/// once they read through [`Session::events`].
pub(crate) fn is<E: Event>(event: &CdpEvent) -> bool {
    event.method == E::METHOD
}

/// [`Raw::decode`] over an event `browser_representations` read off the ring itself ([`is`]).
pub(crate) fn decode<E: Event>(event: &CdpEvent) -> Option<E> {
    if !is::<E>(event) {
        return None;
    }
    match E::deserialize(&event.params) {
        Ok(decoded) => Some(decoded),
        Err(error) => {
            debug!(method = E::METHOD, %error, "a CDP event did not decode; ignoring it");
            None
        }
    }
}

#[cfg(test)]
impl Raw {
    /// An event as the client delivers one.
    pub(crate) fn fixture(method: &str, session: Option<&str>, params: serde_json::Value) -> Self {
        Self(CdpEvent {
            method: method.to_string(),
            params,
            session_id: session.map(SessionId::from),
        })
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
}

#[cfg(test)]
mod tests {
    use std::any;
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;

    use serde_json::{Value, json};

    use super::*;

    /// One `command!` or `event!` entry: its method, and the Rust type declared for it.
    pub(crate) struct Declared {
        method: &'static str,
        type_name: &'static str,
    }

    impl Declared {
        pub(crate) fn command<C: Command>() -> Self {
            Self {
                method: C::METHOD,
                type_name: any::type_name::<C>(),
            }
        }

        pub(crate) fn event<E: Event>() -> Self {
            Self {
                method: E::METHOD,
                type_name: any::type_name::<E>(),
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
    fn events_decode_as_leniently_as_neo_reads_them() {
        let commit = Raw::fixture(
            "Page.frameNavigated",
            Some("S"),
            json!({"frame": {"id": "F", "url": "https://a.test/", "mimeType": "text/html"}}),
        );
        let decoded = commit
            .decode::<page::FrameNavigated>()
            .expect("a commit without type, loaderId or urlFragment decodes");
        assert_eq!((decoded.frame.loader_id, decoded.kind), (None, None));
        assert!(
            commit.decode::<page::LifecycleEvent>().is_none(),
            "another event's payload never decodes from it"
        );
        let broken = Raw::fixture("Page.lifecycleEvent", Some("S"), json!({"frameId": "F"}));
        assert!(broken.is::<page::LifecycleEvent>());
        assert!(
            broken.decode::<page::LifecycleEvent>().is_none(),
            "a known event that does not decode reads as None"
        );
    }

    #[tokio::test]
    async fn events_pass_lag_and_close_through() {
        let (ring, mut events) = Ring::new(1);
        for _ in 0..2 {
            ring.send(Raw::fixture(
                "Inspector.targetCrashed",
                Some("S"),
                json!(null),
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
