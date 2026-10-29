//! Page holds: which tabs show the working overlay, in which phase, and what a take over does.
//!
//! A tab is held from neo's first page-changing action on it until neo has been quiet for
//! [`HOLD_GRACE`], the session ends, or the person takes over. Holds are keyed by page, not by
//! ownership, so neo acting on the person's own tab shows the overlay too: that is exactly the
//! tab where the two of them collide.
//!
//! ```text
//! begin_action -> Working --end_action--> Between --grace--> (gone)
//!                    |                       |
//!                    +------ take over ------+--> Person --hand back--> Between
//! session ends: Working/Between -> Finished (the page fades it), Person -> gone
//! ```
//!
//! The overlay itself keeps no server state alive: a beat every [`BEAT_EVERY`] keeps it drawn,
//! re-draws it after the page navigates, and drains the events it queued in case a binding call
//! was lost. A server that stops beating leaves pages that clear themselves.

pub mod labels;

use crate::{
    analytics::{AnalyticsSink, events},
    ids::{DispatchId, SessionId},
    services::{
        agent_settings::AgentSettingsStore,
        browser::BrowserService,
        help::{HelpOpenParams, HelpOrigin, HelpRegistry},
        sessions::Sessions,
    },
};
use browseros_core::{
    BrowserSession, CoreError, PageId, ProtocolSession, SessionId as CdpSessionId,
    overlay::{self, OverlayEvent, OverlayReply, OverlayState, Phase, Rect, Via},
};
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// How long a held page stays held after neo's last action. Agent turns between actions take a
/// few seconds; a hold that ended on every turn would flicker.
pub const HOLD_GRACE: Duration = Duration::from_secs(20);
/// How often a held page is beaten. Also bounds how late a lost take over is noticed and how long
/// a navigated page goes without its overlay.
pub const BEAT_EVERY: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_secs(1);
/// A take over the agent never answered is dropped after the same cap as a help request.
const PERSON_MAX: Duration = Duration::from_secs(20 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HoldPhase {
    Working,
    Between,
    Person,
}

struct Hold {
    owner: SessionId,
    phase: HoldPhase,
    label: String,
    since: Instant,
    beat_at: Instant,
    /// Calls acting on the page right now, interrupted when the person takes over.
    in_flight: HashSet<DispatchId>,
    /// The next draw must clear the page's local "you have control" state.
    resume: bool,
}

impl Hold {
    fn new(owner: &SessionId) -> Self {
        let now = Instant::now();
        Self {
            owner: owner.clone(),
            phase: HoldPhase::Working,
            label: String::new(),
            since: now,
            beat_at: now,
            in_flight: HashSet::new(),
            resume: false,
        }
    }

    fn state(&self) -> OverlayState {
        let (phase, label) = match self.phase {
            HoldPhase::Working => (Phase::Working, self.label.clone()),
            HoldPhase::Between => (Phase::Between, labels::BETWEEN.to_string()),
            HoldPhase::Person => (Phase::Person, String::new()),
        };
        OverlayState {
            phase,
            label,
            rect: None,
            resume: self.resume,
        }
    }
}

/// What a person sees in the agent's reply when it acts on a page they took over.
#[must_use]
pub fn person_control_message() -> String {
    CoreError::PersonControl.to_string()
}

/// What an action the person interrupted returns.
pub const INTERRUPTED_MESSAGE: &str = "Interrupted: the person took over this page. Do not act on it: call await_human_help to wait until they hand it back, or work in another tab.";

pub struct PageHoldService {
    /// The server's rollout switch (`[pages] working_overlay`).
    switch: bool,
    agent_settings: Arc<AgentSettingsStore>,
    help: Arc<HelpRegistry>,
    sessions: Arc<Sessions>,
    browser: Arc<BrowserService>,
    analytics: Arc<dyn AnalyticsSink>,
    holds: Mutex<HashMap<PageId, Hold>>,
    interrupted: Mutex<HashSet<DispatchId>>,
    /// CDP sessions the binding was added to, and their page, for routing binding calls.
    bindings: Mutex<HashMap<CdpSessionId, PageId>>,
    listening: Mutex<Weak<BrowserSession>>,
    started: Mutex<HashSet<(SessionId, PageId)>>,
}

impl PageHoldService {
    #[must_use]
    pub fn new(
        switch: bool,
        agent_settings: Arc<AgentSettingsStore>,
        help: Arc<HelpRegistry>,
        sessions: Arc<Sessions>,
        browser: Arc<BrowserService>,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> Arc<Self> {
        Arc::new(Self {
            switch,
            agent_settings,
            help,
            sessions,
            browser,
            analytics,
            holds: Mutex::new(HashMap::new()),
            interrupted: Mutex::new(HashSet::new()),
            bindings: Mutex::new(HashMap::new()),
            listening: Mutex::new(Weak::new()),
            started: Mutex::new(HashSet::new()),
        })
    }

    #[must_use]
    pub fn enabled(&self) -> bool {
        self.switch && self.agent_settings.working_overlay_enabled()
    }

    /// Runs the grace, beat and expiry timers until `shutdown`.
    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) {
        if !self.switch {
            return;
        }
        let service = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(TICK);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => return,
                    _ = interval.tick() => {}
                }
                let Some(service) = service.upgrade() else {
                    return;
                };
                service.tick().await;
            }
        });
    }

    /// The refusal for a page-changing call on a page the person has taken over.
    pub async fn person_message(&self, page: &PageId) -> Option<String> {
        let holds = self.holds.lock().await;
        holds
            .get(page)
            .filter(|hold| hold.phase == HoldPhase::Person)
            .map(|_| person_control_message())
    }

    /// True once for a call that was cancelled because the person took over its page.
    pub async fn take_interrupted(&self, dispatch: &DispatchId) -> bool {
        self.interrupted.lock().await.remove(dispatch)
    }

    /// Draws the overlay for an action neo is about to take on `page`. Refuses when the person
    /// has taken the page over, including a take over the page made locally a moment ago.
    pub async fn begin_action(
        self: &Arc<Self>,
        browser: &Arc<BrowserSession>,
        page: PageId,
        owner: &SessionId,
        dispatch: &DispatchId,
        label: String,
        rect: Option<Rect>,
    ) -> Result<(), String> {
        if !self.enabled() {
            return Ok(());
        }
        if let Some(refusal) = self.person_message(&page).await {
            return Err(refusal);
        }
        let Ok(page_session) = browser.pages.get_session(page.clone()).await else {
            return Ok(());
        };
        self.ensure_binding(
            browser,
            &page_session.session_id,
            &page_session.session,
            &page,
        )
        .await;
        let resume = {
            let mut holds = self.holds.lock().await;
            if holds
                .get(&page)
                .is_some_and(|hold| hold.phase == HoldPhase::Person)
            {
                return Err(person_control_message());
            }
            let hold = holds
                .entry(page.clone())
                .or_insert_with(|| Hold::new(owner));
            hold.owner = owner.clone();
            hold.phase = HoldPhase::Working;
            hold.label.clone_from(&label);
            hold.since = Instant::now();
            hold.beat_at = Instant::now();
            hold.in_flight.insert(dispatch.clone());
            std::mem::take(&mut hold.resume)
        };
        self.report_started(owner, &page).await;
        let reply = overlay::show(
            &page_session.session,
            &OverlayState {
                phase: Phase::Working,
                label,
                rect,
                resume,
            },
        )
        .await;
        self.handle_reply(&page, reply).await;
        let mut holds = self.holds.lock().await;
        match holds.get_mut(&page) {
            Some(hold) if hold.phase == HoldPhase::Person => {
                hold.in_flight.remove(dispatch);
                drop(holds);
                self.interrupted.lock().await.remove(dispatch);
                Err(person_control_message())
            }
            _ => Ok(()),
        }
    }

    /// Moves a page to "between actions" once no call is acting on it.
    pub async fn end_action(
        &self,
        browser: &Arc<BrowserSession>,
        page: PageId,
        dispatch: &DispatchId,
    ) {
        let state = {
            let mut holds = self.holds.lock().await;
            let Some(hold) = holds.get_mut(&page) else {
                return;
            };
            hold.in_flight.remove(dispatch);
            if hold.phase != HoldPhase::Working || !hold.in_flight.is_empty() {
                return;
            }
            hold.phase = HoldPhase::Between;
            hold.since = Instant::now();
            hold.beat_at = Instant::now();
            hold.state()
        };
        let Ok(page_session) = browser.pages.get_session(page.clone()).await else {
            return;
        };
        let reply = overlay::show(&page_session.session, &state).await;
        self.handle_reply(&page, reply).await;
    }

    /// Takes the overlay off a page whose agent asked for human help: the extension's takeover
    /// bar owns the page until the request resolves, and the next action draws it again.
    pub async fn quiet(&self, page: &PageId) {
        if self.holds.lock().await.remove(page).is_none() {
            return;
        }
        if let Some(session) = self.page_session(page).await {
            overlay::dispose(&session).await;
        }
    }

    /// Ends every hold of a finished session: "Finished" where neo was working, nothing where
    /// the person already has the page.
    pub async fn finish_session(&self, owner: &SessionId) {
        let finished: Vec<(PageId, HoldPhase)> = {
            let mut holds = self.holds.lock().await;
            let pages: Vec<PageId> = holds
                .iter()
                .filter(|(_, hold)| &hold.owner == owner)
                .map(|(page, _)| page.clone())
                .collect();
            pages
                .into_iter()
                .filter_map(|page| holds.remove(&page).map(|hold| (page, hold.phase)))
                .collect()
        };
        self.started
            .lock()
            .await
            .retain(|(session, _)| session != owner);
        for (page, phase) in finished {
            let Some(session) = self.page_session(&page).await else {
                continue;
            };
            if phase == HoldPhase::Person {
                overlay::dispose(&session).await;
                continue;
            }
            let _ = overlay::show(
                &session,
                &OverlayState {
                    phase: Phase::Finished,
                    label: String::new(),
                    rect: None,
                    resume: true,
                },
            )
            .await;
        }
    }

    async fn tick(self: &Arc<Self>) {
        if !self.enabled() {
            let pages: Vec<PageId> = self
                .holds
                .lock()
                .await
                .drain()
                .map(|(page, _)| page)
                .collect();
            for page in pages {
                if let Some(session) = self.page_session(&page).await {
                    overlay::dispose(&session).await;
                }
            }
            return;
        }
        let now = Instant::now();
        let mut expired = Vec::new();
        let mut due = Vec::new();
        {
            let mut holds = self.holds.lock().await;
            holds.retain(|page, hold| {
                let over = match hold.phase {
                    HoldPhase::Between => hold.since.elapsed() >= HOLD_GRACE,
                    HoldPhase::Person => hold.since.elapsed() >= PERSON_MAX,
                    HoldPhase::Working => false,
                };
                if over {
                    expired.push(page.clone());
                    return false;
                }
                if now.duration_since(hold.beat_at) >= BEAT_EVERY {
                    hold.beat_at = now;
                    due.push((page.clone(), hold.state()));
                }
                true
            });
        }
        for page in expired {
            if let Some(session) = self.page_session(&page).await {
                overlay::dispose(&session).await;
            }
        }
        for (page, state) in due {
            let Some(session) = self.page_session(&page).await else {
                // The tab is gone.
                self.holds.lock().await.remove(&page);
                continue;
            };
            let mut reply = overlay::beat(&session).await;
            if reply.status == "absent" {
                // The page navigated to a new document, which has no overlay yet.
                let events = std::mem::take(&mut reply.events);
                reply = overlay::show(&session, &state).await;
                reply.events.splice(0..0, events);
            }
            self.handle_reply(&page, reply).await;
        }
    }

    async fn handle_reply(&self, page: &PageId, reply: OverlayReply) {
        let local_take_over = reply.status == "person";
        let mut took_over = false;
        for event in reply.events {
            match event {
                OverlayEvent::TakeOver { via } => {
                    took_over = true;
                    self.take_over(page, via).await;
                }
                event => self.handle_event(page, event).await,
            }
        }
        // The page took over locally but its event never reached us (a lost binding call whose
        // queue entry was drained by an earlier reply that failed): believe the page.
        if local_take_over && !took_over {
            self.take_over(page, Via::Button).await;
        }
    }

    pub(crate) async fn handle_event(&self, page: &PageId, event: OverlayEvent) {
        match event {
            OverlayEvent::TakeOver { via } => self.take_over(page, via).await,
            OverlayEvent::HandBack => self.hand_back(page).await,
            OverlayEvent::ReachIn => {
                let held = self
                    .holds
                    .lock()
                    .await
                    .get(page)
                    .map(|hold| (hold.owner.clone(), hold.phase));
                if let Some((owner, phase)) = held {
                    let phase = if phase == HoldPhase::Between {
                        "between"
                    } else {
                        "working"
                    };
                    self.capture(events::PAGE_REACH_IN, &owner, json!({ "phase": phase }))
                        .await;
                }
            }
            OverlayEvent::LetFinish => {}
        }
    }

    async fn take_over(&self, page: &PageId, via: Via) {
        let (owner, in_flight) = {
            let mut holds = self.holds.lock().await;
            let Some(hold) = holds.get_mut(page) else {
                return;
            };
            if hold.phase == HoldPhase::Person {
                return;
            }
            hold.phase = HoldPhase::Person;
            hold.since = Instant::now();
            hold.resume = false;
            (hold.owner.clone(), std::mem::take(&mut hold.in_flight))
        };
        if !in_flight.is_empty() {
            self.interrupted
                .lock()
                .await
                .extend(in_flight.iter().cloned());
            if let Some(session) = self.sessions.lookup(&owner).await {
                for dispatch in &in_flight {
                    session.interrupt_dispatch(dispatch).await;
                }
            }
        }
        let agent_waiting = self
            .help
            .get(&owner)
            .await
            .is_some_and(|entry| entry.is_open() && entry.origin() == HelpOrigin::Agent);
        if !agent_waiting {
            let info = match self.browser.session().await {
                Some(browser) => browser.pages.get_info(page.clone()).await,
                None => None,
            };
            self.help
                .open(
                    &owner,
                    HelpOpenParams {
                        request_id: format!("takeover-{}", ulid::Ulid::new()),
                        reason: "You took over this page".to_string(),
                        details: None,
                        resume_hint: None,
                        kind: Some(claw_api::models::HelpRequestKind::Other),
                        browser_tab_id: info.as_ref().map(|info| info.tab_id.0),
                        url: info.as_ref().map(|info| info.url.clone()),
                        title: info.as_ref().map(|info| info.title.clone()),
                        origin: HelpOrigin::Person,
                    },
                )
                .await;
        }
        let via = match via {
            Via::Button => "button",
            Via::Esc => "esc",
        };
        self.capture(events::PAGE_TAKEN_OVER, &owner, json!({ "via": via }))
            .await;
    }

    async fn hand_back(&self, page: &PageId) {
        let (owner, held_for, state) = {
            let mut holds = self.holds.lock().await;
            let Some(hold) = holds.get_mut(page) else {
                return;
            };
            if hold.phase != HoldPhase::Person {
                return;
            }
            let held_for = hold.since.elapsed();
            hold.phase = HoldPhase::Between;
            hold.since = Instant::now();
            hold.beat_at = Instant::now();
            hold.resume = true;
            (hold.owner.clone(), held_for, hold.state())
        };
        if let Some(entry) = self.help.get(&owner).await
            && entry.is_open()
            && entry.origin() == HelpOrigin::Person
        {
            self.help.resolve(&owner, None).await;
        }
        if let Some(session) = self.page_session(page).await {
            overlay::show(&session, &state).await;
            if let Some(hold) = self.holds.lock().await.get_mut(page) {
                hold.resume = false;
            }
        }
        // Whole seconds: enough to tell a glance from a real use of the page.
        self.capture(
            events::PAGE_HANDED_BACK,
            &owner,
            json!({ "held_ms": held_for.as_secs() * 1000 }),
        )
        .await;
    }

    async fn report_started(&self, owner: &SessionId, page: &PageId) {
        let first = self
            .started
            .lock()
            .await
            .insert((owner.clone(), page.clone()));
        if first {
            self.capture(events::PAGE_HOLD_STARTED, owner, json!({}))
                .await;
        }
    }

    async fn capture(
        &self,
        event: events::EventDefinition,
        owner: &SessionId,
        mut properties: serde_json::Value,
    ) {
        let Some(session) = self.sessions.lookup(owner).await else {
            return;
        };
        properties["client_name"] = json!(session.client_name());
        self.analytics.capture(event, properties);
    }

    async fn page_session(&self, page: &PageId) -> Option<ProtocolSession> {
        let browser = self.browser.session().await?;
        browser
            .pages
            .get_session(page.clone())
            .await
            .ok()
            .map(|page| page.session)
    }

    /// Exposes the overlay's binding on this CDP session once, and listens for its calls on the
    /// browser connection the session belongs to.
    async fn ensure_binding(
        self: &Arc<Self>,
        browser: &Arc<BrowserSession>,
        session_id: &CdpSessionId,
        session: &ProtocolSession,
        page: &PageId,
    ) {
        let added = self
            .bindings
            .lock()
            .await
            .insert(session_id.clone(), page.clone())
            .is_none();
        if added && let Err(error) = overlay::install_binding(session).await {
            tracing::debug!(%error, "working overlay binding unavailable; beats carry its events");
        }
        let mut listening = self.listening.lock().await;
        if listening
            .upgrade()
            .is_some_and(|current| Arc::ptr_eq(&current, browser))
        {
            return;
        }
        *listening = Arc::downgrade(browser);
        drop(listening);
        let mut events = browser.cdp_events();
        let service = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                let event = match events.recv().await {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                };
                if event.method != "Runtime.bindingCalled"
                    || event.params["name"].as_str() != Some(overlay::BINDING)
                {
                    continue;
                }
                let Some(service) = service.upgrade() else {
                    return;
                };
                let Some(session_id) = event.session_id.as_ref() else {
                    continue;
                };
                let Some(page) = service.bindings.lock().await.get(session_id).cloned() else {
                    continue;
                };
                let Some(parsed) = event.params["payload"]
                    .as_str()
                    .and_then(OverlayEvent::parse)
                else {
                    continue;
                };
                service.handle_event(&page, parsed).await;
            }
        });
    }
}

#[cfg(test)]
impl PageHoldService {
    /// Puts a page in a phase as if neo had acted on it, without a browser.
    pub(crate) async fn seed(
        &self,
        page: PageId,
        owner: &SessionId,
        phase: HoldPhase,
        in_flight: &[DispatchId],
    ) {
        let mut hold = Hold::new(owner);
        hold.phase = phase;
        hold.in_flight = in_flight.iter().cloned().collect();
        self.holds.lock().await.insert(page, hold);
    }

    pub(crate) async fn phase(&self, page: &PageId) -> Option<HoldPhase> {
        self.holds.lock().await.get(page).map(|hold| hold.phase)
    }
}
