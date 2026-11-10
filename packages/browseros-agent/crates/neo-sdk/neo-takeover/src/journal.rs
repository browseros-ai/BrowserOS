//! The control journal: while the owner has control, each value they type into a password, card
//! or one-time-code field is caught at its `input` event and reaches the known secrets list, so
//! a submit or a navigation cannot take it away (design D6, synthesis item 6, sdk-privacy D4). It
//! replaces zuzu's one-second poll, which lost a value when the page navigated within that second.
//!
//! sdk-privacy's seal carries the journal: in the sealed world, a capture-phase listener sends
//! `{kind, value, trusted}` through the `JOURNAL_BINDING` binding when a sensitive field gets
//! input, `trusted` saying whether a real key, paste or drop entered it, the owner's or an agent's
//! typed over CDP. This module arms it during control and feeds the secret set, counting the
//! owner's presses apart:
//!
//! ```text
//!  control display ─ each key-down and paste it sends on to Xvnc ─▶ Journal's press count
//!  Journal::arm(tabs) ─ subscribe first ─▶ ArmedJournal: its listener, and the listener's worker
//!    the listener's credits: the presses counted from now on, banked up to MAX_OWNER_CREDITS
//!    each tab: its page's session, then each OOPIF's (sealed_frames' walk), once per session:
//!      Page.enable, Runtime.enable
//!      Runtime.addBinding {JOURNAL_BINDING, executionContextName: SEALED_WORLD}
//!      Page.addScriptToEvaluateOnNewDocument {seal_source(), SEALED_WORLD, runImmediately}
//!    then sealed_frames' seal: closed shadow roots watched, any frame the script missed sealed
//!  listener, from its own binding-call channel:
//!            Runtime.bindingCalled on an armed session ─▶ JournalEntry::parse_with_trust
//!              ─▶ Pending: an owner entry if trusted and a credit is left, which it spends
//!              ─ a second with no entry, disarm or drop ─▶ SecretSet: the owner's values, and
//!                the rest as page values
//!  listener, from the connection's broadcast:
//!            Target.attachedToTarget, an iframe ─▶ worker: armed the same way, if its parent is
//!            a lag ─▶ worker: every armed tab swept
//!  disarm, or drop ─▶ the listener takes what was already sent, registers what it holds, waits
//!                     for the worker's queued jobs, then removes its scripts
//! ```
//!
//! **Binding calls come on a channel of the journal's own, never the broadcast.** browseros-cdp
//! keeps a method that any reader takes with `events_targeted` off the connection's shared
//! broadcast, and sdk-playwright takes `Runtime.bindingCalled` that way for as long as the
//! process runs. A journal that read binding calls from the broadcast would get none, register
//! nothing, and let what the owner typed reach the agent raw. browseros-cdp hands every targeted
//! subscriber each call, so this one reads them beside sdk-playwright's. The channel is
//! unbounded: a listener that falls behind delays an entry but never loses one.
//!
//! **The listener never waits on the browser.** Frame attaches and detaches still come on the
//! broadcast, which every reader shares and which drops the oldest events a reader has not read
//! once it is 4096 behind. A listener that armed frames itself would stop reading whenever a
//! browser call stalled, and miss the frames that attached meanwhile. So it takes each entry into
//! `Pending` as it comes and hands browser work to a worker task, which runs it one job at a
//! time, in the order the listener read the events. A lag can still happen if the listener goes
//! unscheduled that long. It loses no entry, so what is held keeps its quiet period; it can lose
//! an iframe's attach, so the worker sweeps every armed tab.
//!
//! **The two channels keep the browser's order where it matters.** The connection's reader hands
//! over each binding call before it broadcasts any later event, so the listener takes the calls
//! already queued before it handles a broadcast event. A call sent just before its frame
//! detached is journaled while that session still counts as armed.
//!
//! **Who arms it** is the display's and the flow's business (decision 31): a takeover arms its
//! tab at start, and every control display arms every open tab and calls
//! [`ArmedJournal::arm_tab`] for each tab that opens while it is open. Over the one screen the
//! owner can switch to any tab and type there. An agent typing over CDP meanwhile reaches the
//! journal too, but no control display sent its keys, so what it types registers as a page value,
//! which is masked but never kept past the set's cap.
//!
//! **Both domains on every armed session (measured on Chromium 155).** Without `Page.enable` on a
//! session, its new-document script never runs in the next document; without `Runtime.enable`, a
//! world made later lacks the binding, so the seal there never journals. Core enables both on a
//! page's session but neither on an OOPIF's, and enabling a domain twice changes nothing.
//!
//! **A frame that appears during control is armed as it attaches.** A page's new-document script
//! reaches only the frames its own process hosts. A cross-site iframe that loads later, such as a
//! payment form, is a new target; core auto-attaches it under its parent's session, and the
//! worker arms it when that parent is armed. The worker looks the parent up when it reaches the
//! frame, not when the listener reads the event, so a frame nested in one that attached just
//! before it finds that parent claimed.
//!
//! **A field's final value is registered, not each keystroke (decision 32).** Each `input` event
//! sends the field's whole value, so registering each one made every prefix a 24-hour secret: a
//! password that starts `https` registered `http`, which then masked every URL in the agent's
//! results for a day. The listener keeps what it is sent in `Pending`, one latest value per
//! field, and registers those with the set's full TTL once a second passes with no entry, and at
//! disarm or drop. The wait is safe: the tab is private by the owner's first input, so nothing
//! captures its pixels; the sealed world masks the field in every read; and a takeover's tab is
//! locked to the agent.
//!
//! **An entry is the owner's only when the seal trusts it and one of the owner's presses pays for
//! it (sdk-traces decisions 53 and 54).** A full set never drops or refuses an owner value, and
//! lets owner values pass its cap, because people type at human speed. So both must hold, and
//! any other entry registers as a page value:
//! - The seal reports the entry trusted. A page's script can enter values into its own fields at
//!   any rate: with `input` events it dispatches itself, which are untrusted, or with
//!   `document.execCommand`, whose `input` events the browser dispatches as trusted ones. Neither
//!   comes with a trusted `keydown`, `paste` or `drop`, so the seal reports an entry trusted only
//!   when one of those reached the field since its last trusted entry: at most one entry per key,
//!   paste or drop.
//! - The listener has a credit, and the entry spends it. Each key-down and each paste that a
//!   control display sends on to Xvnc is one of the owner's presses, counted outside the browser
//!   (displays.rs), and gives every armed journal one credit. An agent typing over CDP makes
//!   trusted entries too, but no display sent its keys, so they find no credit. Each listener
//!   keeps its own account from the moment its journal is armed, so a takeover's journal and its
//!   display's, armed on one tab, never spend each other's credits. Presses that type nothing,
//!   such as arrows and modifiers, bank only up to `MAX_OWNER_CREDITS`.
//!
//! So the owner values one journal makes never outnumber the owner's presses since it was armed.
//! A page value is masked like the rest, but a flood of them pushes out only the page's earlier
//! values and never grows the set past its cap. A value entered both ways is the owner's.
//!
//! Chromium dispatches a trusted `keydown` before each field it autofills (measured on Chromium
//! 148) and before each key an IME takes (keyCode 229), so the seal trusts both. An IME's keys
//! are presses. An autofill or a drop the owner makes with the mouse comes with no press, so it is
//! the owner's only while earlier presses left a credit banked. Text inserted with no key, paste
//! or drop, such as an emoji panel's, is a page value.
//!
//! A deliberate page can still make what the owner types a page value, which a flood can then
//! push out (measured on Chromium 156). An edit from its own `keydown` or `beforeinput` listener
//! spends the owner's grant before the keystroke lands, and stopping `keydown` at window capture,
//! before the seal's listener, makes no grant. That was open before the grant too: stopping
//! `input` the same way keeps the value from the journal, so only the release or close sweep
//! finds it, as a page value. Credits do not close it either. Within one keystroke, the page's
//! edit and the owner's character are both trusted browser input, and nothing in the page or in
//! the journal tells them apart, so the edit can spend that key's credit (decision 54).
//!
//! The binding stays at disarm (see [`ArmedJournal::disarm`]). Values are never logged: not a
//! value, not a length, not a payload.

use crate::limits::{JOURNAL_QUIET, MAX_JOURNAL_PENDING, MAX_OWNER_CREDITS};
use crate::sealed_frames::{TabFrames, step};
use crate::{BrowserTabs, PartError, sweep};
use browseros_cdp::CdpEvent;
use browseros_core::sensitive_fields::{JOURNAL_BINDING, JournalEntry, SEALED_WORLD, seal_source};
use browseros_core::{CoreError, ProtocolSession, SessionId, TabId};
use neo_trace::{Registration, SecretSet, SecretSource, SecretValue};
use serde_json::json;
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use zeroize::Zeroizing;

/// The control journal, ready to arm. The service holds one for the display and the takeover
/// flow; cheap to clone.
#[derive(Clone)]
pub struct Journal {
    tabs: BrowserTabs,
    secrets: Arc<SecretSet>,
    /// The owner's presses (sdk-traces decision 54): every key-down and every paste a control
    /// display has sent on to Xvnc. Neo counts them outside the browser, so no page and no CDP
    /// client can add to it. Every clone shares the one count, and it only grows. Each armed
    /// journal's listener reads it for its own [`Credits`].
    presses: Arc<AtomicU64>,
}

impl Journal {
    /// A journal that reaches the browser through `tabs` and registers into `secrets`, the
    /// process-wide set every result and record is masked with.
    #[must_use]
    pub fn new(tabs: BrowserTabs, secrets: Arc<SecretSet>) -> Self {
        Self {
            tabs,
            secrets,
            presses: Arc::default(),
        }
    }

    /// Counts one of the owner's presses: a control display is sending a key-down or a paste on
    /// to Xvnc. Only the display's proxy calls it, for input it forwards as the owner's.
    pub(crate) fn owner_pressed(&self) {
        self.presses.fetch_add(1, Ordering::SeqCst);
    }

    /// How many presses the owner's control displays have sent on so far.
    pub(crate) fn owner_presses(&self) -> u64 {
        self.presses.load(Ordering::SeqCst)
    }

    /// Arms the journal on `tabs` until the returned [`ArmedJournal`] is disarmed or dropped. A
    /// tab that cannot be armed is logged and skipped; the final sweep still reads it. Its credits
    /// are the owner's presses from now on.
    pub async fn arm(&self, tabs: &[TabId]) -> ArmedJournal {
        let armed = Arc::new(Armed {
            tabs: self.tabs.clone(),
            secrets: Arc::clone(&self.secrets),
            sessions: Mutex::default(),
        });
        // Read here, not when the listener first runs, so no press the owner makes from now on
        // is missed.
        let credits = Credits::new(Arc::clone(&self.presses));
        // Subscribed before any session is armed, so no entry is sent where no one listens.
        let bindings = self.tabs.binding_calls();
        let events = self.tabs.events();
        let (stop, stopped) = oneshot::channel();
        let listener = tokio::spawn(listen(
            Arc::clone(&armed),
            bindings,
            events,
            stopped,
            credits,
        ));
        for &tab in tabs {
            if let Err(error) = armed.arm_tab(tab).await {
                tracing::warn!(tab = tab.0, %error, "the journal could not arm a tab");
            }
        }
        ArmedJournal {
            armed,
            stop,
            listener,
        }
    }
}

/// The journal, armed. Dropping it disarms in the background, in the order [`Self::disarm`]
/// waits for. It registers nothing synchronously when dropped: the listener may not have read the
/// owner's last keystrokes yet, and registering the value they extend would register a prefix.
pub struct ArmedJournal {
    armed: Arc<Armed>,
    /// Dropped to stop the listener, which then registers what it holds and removes the scripts:
    /// dropping the whole `ArmedJournal` disarms with no `Drop` of its own.
    stop: oneshot::Sender<()>,
    listener: JoinHandle<()>,
}

impl ArmedJournal {
    /// Arms one more tab, such as one that opened while a control display is open. Arming a tab
    /// again is harmless: each session is armed once.
    ///
    /// # Errors
    /// `TabNotFound` when no open tab has this id; `Browser` when its page cannot be armed. An
    /// out-of-process frame that cannot be armed is logged and skipped.
    pub async fn arm_tab(&self, tab: TabId) -> Result<(), PartError> {
        self.armed.arm_tab(tab).await
    }

    /// Stops the journal: registers what it holds, with the entries the browser had already
    /// sent, lets the browser work already queued finish, then removes the new-document scripts
    /// it added, and returns when that is done.
    ///
    /// The binding stays. Only the sealed world can call it, so neither the page nor the agent
    /// can reach it; sdk-playwright will add it for good; and another armed journal on the same
    /// tab, such as a takeover's under its display's, still needs it.
    pub async fn disarm(self) {
        let Self { stop, listener, .. } = self;
        drop(stop);
        if let Err(error) = listener.await {
            tracing::warn!(%error, "the journal's listener did not stop cleanly");
        }
    }
}

/// What the armed journal, its listener and the listener's worker share: the sessions it armed.
struct Armed {
    tabs: BrowserTabs,
    secrets: Arc<SecretSet>,
    sessions: Mutex<HashMap<SessionId, ArmedSession>>,
}

/// Browser work the listener hands its worker.
enum Job {
    /// An iframe target attached: armed if the session it attached under is.
    Frame(CdpEvent),
    /// The listener fell behind the broadcast: every armed tab is swept.
    Sweep,
}

/// One armed session: the tab it serves, and the new-document script it got.
struct ArmedSession {
    tab: TabId,
    /// `None` while it is being armed.
    script: Option<String>,
}

impl Armed {
    /// Never held across an await.
    fn sessions(&self) -> MutexGuard<'_, HashMap<SessionId, ArmedSession>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn arm_tab(&self, tab: TabId) -> Result<(), PartError> {
        let (page, session) = self.tabs.page_session(tab).await?;
        // The page first, so an OOPIF that attaches during the walk below already has an armed
        // parent for the listener to find.
        self.arm_session(&session, tab)
            .await
            .map_err(PartError::Browser)?;
        for target in TabFrames::walk(&self.tabs, &page, session).await?.targets {
            if let Err(error) = self.arm_session(&target.session, tab).await {
                tracing::warn!(tab = tab.0, %error, "the journal skipped a frame target it could not arm");
            }
            target.seal().await;
        }
        Ok(())
    }

    /// Arms one session, exactly once per `ArmedJournal`: the first caller claims it under the
    /// lock before any CDP call, so the walk, a repeated `arm_tab` and the attach listener racing
    /// on one session add one new-document script, and disarm removes exactly the scripts this
    /// journal added. A failed arming gives the claim back, so a later `arm_tab` tries again.
    async fn arm_session(&self, session: &ProtocolSession, tab: TabId) -> Result<(), CoreError> {
        let Some(id) = session.session_id().cloned() else {
            return Ok(());
        };
        {
            let mut sessions = self.sessions();
            if sessions.contains_key(&id) {
                return Ok(());
            }
            sessions.insert(id.clone(), ArmedSession { tab, script: None });
        }
        match add_journal(session).await {
            Ok(script) => {
                if let Some(armed) = self.sessions().get_mut(&id) {
                    armed.script = Some(script);
                }
                Ok(())
            }
            Err(error) => {
                self.sessions().remove(&id);
                Err(error)
            }
        }
    }

    /// Takes one broadcast event without waiting on the browser: a session that went away, or a
    /// job for the worker.
    fn on_event(&self, event: CdpEvent, jobs: &mpsc::UnboundedSender<Job>) {
        match event.method.as_str() {
            "Target.attachedToTarget" if event.params["targetInfo"]["type"] == "iframe" => {
                // Refused only once the worker is gone, which leaves nothing to arm the frame.
                let _ = jobs.send(Job::Frame(event));
            }
            "Target.detachedFromTarget" => {
                if let Some(gone) = event.params["sessionId"].as_str() {
                    self.sessions().remove(&SessionId::from(gone));
                }
            }
            _ => {}
        }
    }

    /// Takes one entry from an armed session's binding into `pending`. Another binding, or the
    /// same one on a session this journal did not arm, such as one sdk-playwright set up, is not
    /// this journal's to read.
    fn journal(&self, event: &CdpEvent, pending: &mut Pending) {
        let armed = event
            .session_id
            .as_ref()
            .is_some_and(|session| self.sessions().contains_key(session));
        if !armed || event.params["name"] != JOURNAL_BINDING {
            return;
        }
        if let Some((entry, trusted)) = event.params["payload"]
            .as_str()
            .and_then(JournalEntry::parse_with_trust)
        {
            pending.note(entry.value, trusted);
        }
    }

    /// Takes every binding call already queued, without waiting for more.
    fn journal_queued(
        &self,
        bindings: &mut mpsc::UnboundedReceiver<CdpEvent>,
        pending: &mut Pending,
    ) {
        while let Ok(call) = bindings.try_recv() {
            self.journal(&call, pending);
        }
    }

    /// Arms an iframe target that attached under a session this journal armed.
    async fn frame_attached(&self, event: &CdpEvent) {
        let parent = event
            .session_id
            .as_ref()
            .and_then(|parent| self.sessions().get(parent).map(|armed| armed.tab));
        let (Some(tab), Some(child)) = (parent, event.params["sessionId"].as_str()) else {
            return;
        };
        let session = self.tabs.session(SessionId::from(child));
        if let Err(error) = self.arm_session(&session, tab).await {
            tracing::warn!(tab = tab.0, %error, "the journal could not arm a frame that attached during control");
        }
    }

    /// The broadcast dropped events the listener had not read, perhaps the attach of an iframe
    /// that is now unarmed, so every armed tab is swept: what every frame's fields hold gets
    /// registered, a frame the journal never armed included.
    async fn sweep_tabs(&self) {
        let tabs = self
            .sessions()
            .values()
            .map(|armed| armed.tab)
            .collect::<BTreeSet<_>>();
        for tab in tabs {
            if let Err(error) = sweep(&self.tabs, &self.secrets, tab).await {
                tracing::warn!(tab = tab.0, %error, "the journal's catch-up sweep failed");
            }
        }
    }

    async fn remove_scripts(&self) {
        let scripts = self
            .sessions()
            .drain()
            .filter_map(|(id, armed)| armed.script.map(|script| (id, script)))
            .collect::<Vec<_>>();
        for (id, script) in scripts {
            let removed = step(
                &self.tabs.session(id),
                "Page.removeScriptToEvaluateOnNewDocument",
                json!({ "identifier": script }),
            )
            .await;
            if let Err(error) = removed {
                tracing::debug!(%error, "a journal script was not removed; its target is likely gone");
            }
        }
    }
}

/// Makes one session's sealed world journal, in its current documents and in each later one.
async fn add_journal(session: &ProtocolSession) -> Result<String, CoreError> {
    step(session, "Page.enable", json!({})).await?;
    step(session, "Runtime.enable", json!({})).await?;
    step(
        session,
        "Runtime.addBinding",
        json!({ "name": JOURNAL_BINDING, "executionContextName": SEALED_WORLD }),
    )
    .await?;
    let added = step(
        session,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": seal_source(), "worldName": SEALED_WORLD, "runImmediately": true }),
    )
    .await?;
    added["identifier"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            CoreError::Message("Page.addScriptToEvaluateOnNewDocument gave no identifier".into())
        })
}

/// The listener: takes entries, and hands browser work to its worker, until stopped; then
/// disarms. Until the stop it awaits nothing but its two channels and the quiet period.
async fn listen(
    armed: Arc<Armed>,
    mut bindings: mpsc::UnboundedReceiver<CdpEvent>,
    mut events: broadcast::Receiver<CdpEvent>,
    mut stop: oneshot::Receiver<()>,
    credits: Credits,
) {
    let (jobs, queued) = mpsc::unbounded_channel();
    let worker = tokio::spawn(work(Arc::clone(&armed), queued));
    let mut pending = Pending::new(Arc::clone(&armed.secrets), credits);
    loop {
        tokio::select! {
            biased;
            _ = &mut stop => break,
            call = bindings.recv() => match call {
                Some(call) => armed.journal(&call, &mut pending),
                // Closed only once the connection is gone, as the broadcast then is too.
                None => break,
            },
            received = events.recv() => {
                // The calls sent before this event are queued by now (see the module doc).
                armed.journal_queued(&mut bindings, &mut pending);
                match received {
                    Ok(event) => armed.on_event(event, &jobs),
                    // No entry was lost, so what is held stays on its quiet period.
                    Err(RecvError::Lagged(missed)) => {
                        tracing::warn!(
                            missed,
                            "the journal fell behind the browser's events; sweeping its tabs"
                        );
                        let _ = jobs.send(Job::Sweep);
                    }
                    Err(RecvError::Closed) => break,
                }
            }
            // After the binding calls, so an entry already sent extends what is held before the
            // quiet period can end on it.
            () = quiet(pending.quiet_until) => pending.register(),
        }
    }
    // What the browser sent before the stop is already here: the owner's last keystrokes before
    // give-back. Register it, with what is held, before the scripts go.
    armed.journal_queued(&mut bindings, &mut pending);
    // Read for the last time. Dropped now, it queues no calls while the scripts are removed.
    drop(bindings);
    pending.register();
    // The worker ends once its queue is empty, so a frame it is arming has its script recorded
    // before the scripts are removed, and none is left behind.
    drop(jobs);
    if let Err(error) = worker.await {
        tracing::warn!(%error, "the journal's worker did not stop cleanly");
    }
    armed.remove_scripts().await;
}

/// The listener's worker: the browser work its events call for, one job at a time, in the order
/// the listener read them. Ends once the listener drops its end and the queue is empty.
async fn work(armed: Arc<Armed>, mut jobs: mpsc::UnboundedReceiver<Job>) {
    while let Some(job) = jobs.recv().await {
        match job {
            Job::Frame(event) => armed.frame_attached(&event).await,
            Job::Sweep => armed.sweep_tabs().await,
        }
    }
}

/// Ends when the held values' quiet period does, and never while nothing is held.
async fn quiet(until: Option<Instant>) {
    match until {
        Some(until) => tokio::time::sleep_until(until).await,
        None => std::future::pending().await,
    }
}

/// What was entered into sensitive fields during control and is not registered yet: the latest
/// value of each field. Only the listener holds it.
///
/// The seal reports a value, not the field it came from, so fields are told apart by how one
/// value follows another. A value that extends a held one, or repeats it, is that field typed
/// further, and the held one goes: it is a prefix. A value one character shorter than the value
/// typed last is a backspace there. Anything else is held beside the rest: another field, or a
/// field retyped. A shorter value is not taken for the same field otherwise, because the first
/// key of the next field can look just like it: a security code that starts like its card
/// number, or a new password that starts like the current one, would erase a value nothing had
/// registered yet.
struct Pending {
    secrets: Arc<SecretSet>,
    /// What lets a trusted entry be the owner's.
    credits: Credits,
    /// The value typed last is last.
    values: Vec<Held>,
    /// When `values` is registered, unless another entry comes first.
    quiet_until: Option<Instant>,
    /// Page values the set refused since the last registration, logged with the next.
    refused: usize,
}

/// One field's latest value, held until it is registered.
struct Held {
    /// Zeroed when dropped.
    value: Zeroizing<String>,
    /// Whether the value is the owner's: an owner entry made it, or it repeats a value one made.
    /// If so it registers as the owner's, else as a page value.
    owner: bool,
}

impl Pending {
    fn new(secrets: Arc<SecretSet>, credits: Credits) -> Self {
        Self {
            secrets,
            credits,
            values: Vec::new(),
            quiet_until: None,
            refused: 0,
        }
    }

    /// Takes one entry: the whole value of a field that got input, and whether the seal reported
    /// it trusted. A trusted entry is the owner's if a credit is left, and spends it.
    fn note(&mut self, value: Zeroizing<String>, trusted: bool) {
        let owner = trusted && self.credits.spend();
        // An entry that is not the owner's never replaces one that is, as a backspace here or as
        // a longer value below. A page's script, or an agent's keys, could otherwise push out
        // what the owner typed with an entry one character shorter or longer, and the owner's
        // value would be registered nowhere.
        let backspace = self.values.last().is_some_and(|last| {
            (owner || !last.owner)
                && last
                    .value
                    .strip_prefix(value.as_str())
                    .is_some_and(|removed| removed.chars().count() == 1)
        });
        if backspace {
            self.values.pop();
        }
        // A repeat of a value the owner typed stays the owner's, as the set keeps the higher
        // source of a value registered twice. That ownership only labels the value kept: what
        // this entry may replace is decided by whether the entry itself is the owner's, so a
        // page's replay of one owner value clears none of the owner's others.
        let owned = owner
            || self
                .values
                .iter()
                .any(|held| held.owner && held.value.as_str() == value.as_str());
        self.values.retain(|held| {
            held.value.as_str() != value.as_str()
                && !(value.starts_with(held.value.as_str()) && (owner || !held.owner))
        });
        self.values.push(Held {
            value,
            owner: owned,
        });
        if self.values.len() > MAX_JOURNAL_PENDING {
            let oldest = self.values.remove(0);
            self.refused += usize::from(register_value(&self.secrets, oldest));
        }
        self.quiet_until = Some(Instant::now() + JOURNAL_QUIET);
    }

    /// Registers every held value with the set's full TTL. A value the set already holds keeps
    /// the later of its two expiries, so registering it again never shortens it. Refusals are
    /// counted and logged here, once, so a page that floods a full set floods no log.
    fn register(&mut self) {
        for held in self.values.drain(..) {
            self.refused += usize::from(register_value(&self.secrets, held));
        }
        if self.refused > 0 {
            tracing::error!(
                refused = self.refused,
                "the secret set is full of values it never drops, so page values the journal took \
                 are masked nowhere"
            );
            self.refused = 0;
        }
        self.quiet_until = None;
    }
}

/// The listener registers what it holds before it removes its scripts; this covers a listener
/// whose task is dropped instead, as its runtime shuts down, so what it had read still counts.
impl Drop for Pending {
    fn drop(&mut self) {
        self.register();
    }
}

/// One armed journal's owner credits (sdk-traces decision 54): the owner's presses since it was
/// armed, less the owner entries it made, banked up to [`MAX_OWNER_CREDITS`]. Only its listener
/// holds it, so two journals armed on one tab, such as a takeover's and its display's, each
/// read every entry and spend their own credits, never each other's.
struct Credits {
    /// [`Journal`]'s press count, which every control display adds to.
    presses: Arc<AtomicU64>,
    /// The count when this account last read it.
    seen: u64,
    /// Credits not spent yet.
    banked: u64,
}

impl Credits {
    /// An account with nothing banked: the presses counted so far are not its credits.
    fn new(presses: Arc<AtomicU64>) -> Self {
        let seen = presses.load(Ordering::SeqCst);
        Self {
            presses,
            seen,
            banked: 0,
        }
    }

    /// Banks the presses counted since the last call, up to the cap, then spends one credit.
    /// `false` when none is left. A display counts a press before its bytes reach Xvnc, so the
    /// press behind an entry is always counted by the time the entry arrives here.
    fn spend(&mut self) -> bool {
        let now = self.presses.load(Ordering::SeqCst);
        self.banked = self
            .banked
            .saturating_add(now.saturating_sub(self.seen))
            .min(MAX_OWNER_CREDITS);
        self.seen = now;
        if self.banked == 0 {
            return false;
        }
        self.banked -= 1;
        true
    }
}

/// Registers a held value for the set's full TTL, moving its buffer into the secret, which
/// zeroes it when dropped. Returns whether the set refused it.
///
/// An owner value is one a full set never drops or refuses (sdk-traces decision 53), so only one
/// too short to mask goes unregistered. Any other value is a page value: a page's own script
/// entered it, or no press of the owner's paid for it, as with an agent typing over CDP. A full
/// set pushes out the page value that expires first to keep it, and refuses it when every value
/// it holds is protected. A refused value is masked nowhere, as the sweep's are.
fn register_value(secrets: &SecretSet, mut held: Held) -> bool {
    let source = if held.owner {
        SecretSource::Owner
    } else {
        SecretSource::Page
    };
    let registration = secrets.register_from(
        SecretValue::new(std::mem::take(&mut *held.value)),
        None,
        source,
    );
    registration == Registration::Full
}
