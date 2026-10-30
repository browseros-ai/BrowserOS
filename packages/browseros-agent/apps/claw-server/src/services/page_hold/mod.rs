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
//!                    +------ take over ------+--> (gone)
//! session ends: Working/Between -> Finished (the page fades it)
//! ```
//!
//! A take over removes the overlay and ends the hold. The calls acting on the page are
//! interrupted, and every other session on it is refused its next page-changing call there once,
//! so neo stops and checks with the user instead of taking the page straight back.
//!
//! The overlay itself keeps no server state alive: a beat every [`BEAT_EVERY`] keeps it drawn,
//! re-draws it after the page navigates, and drains the events it queued in case a binding call
//! was lost. A server that stops beating leaves pages that clear themselves.

pub mod labels;

use crate::{
    analytics::{AnalyticsSink, events},
    ids::{DispatchId, SessionId},
    services::{agent_settings::AgentSettingsStore, browser::BrowserService, sessions::Sessions},
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
/// A take over notice the agent never ran into is dropped after this.
const NOTICE_MAX: Duration = Duration::from_secs(20 * 60);
/// A call whose end never arrived (its future was dropped) stops holding the page after this.
const WORKING_MAX: Duration = Duration::from_secs(2 * 60);
/// The page reports a reach-in at most every 10s, through the binding and again in its queue.
const REACH_IN_DEDUPE: Duration = Duration::from_secs(9);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HoldPhase {
    Working,
    Between,
}

struct Hold {
    owner: SessionId,
    phase: HoldPhase,
    label: String,
    since: Instant,
    beat_at: Instant,
    /// Calls acting on the page right now and the session each belongs to, interrupted when the
    /// person takes over.
    in_flight: HashMap<DispatchId, SessionId>,
    /// The next draw must clear a take over the page may still keep locally. True for a new hold:
    /// the server ends a hold when it hears a take over, so a new one comes after it.
    resume: bool,
    reach_in_at: Option<Instant>,
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
            in_flight: HashMap::new(),
            resume: true,
            reach_in_at: None,
        }
    }

    fn state(&self) -> OverlayState {
        let (phase, label) = match self.phase {
            HoldPhase::Working => (Phase::Working, self.label.clone()),
            HoldPhase::Between => (Phase::Between, labels::BETWEEN.to_string()),
        };
        OverlayState {
            phase,
            label,
            rect: None,
            resume: self.resume,
        }
    }
}

/// What the agent is told when the person took over a page it was working on.
#[must_use]
pub fn person_control_message() -> String {
    CoreError::PersonControl.to_string()
}

/// What an action the person interrupted returns.
pub const INTERRUPTED_MESSAGE: &str = "Interrupted: the person took over this page and is using it. Stop and check with the user before you act on it again.";

pub struct PageHoldService {
    /// The server's rollout switch (`[pages] working_overlay`).
    switch: bool,
    agent_settings: Arc<AgentSettingsStore>,
    sessions: Arc<Sessions>,
    browser: Arc<BrowserService>,
    analytics: Arc<dyn AnalyticsSink>,
    holds: Mutex<HashMap<PageId, Hold>>,
    interrupted: Mutex<HashSet<DispatchId>>,
    /// Sessions that were working on a page the person took over and have not been told yet:
    /// their next page-changing call there is refused once.
    notices: Mutex<HashMap<(PageId, SessionId), Instant>>,
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
        sessions: Arc<Sessions>,
        browser: Arc<BrowserService>,
        analytics: Arc<dyn AnalyticsSink>,
    ) -> Arc<Self> {
        Arc::new(Self {
            switch,
            agent_settings,
            sessions,
            browser,
            analytics,
            holds: Mutex::new(HashMap::new()),
            interrupted: Mutex::new(HashSet::new()),
            notices: Mutex::new(HashMap::new()),
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

    /// The refusal for `session`'s first page-changing call on a page the person took over from
    /// it. Later calls go ahead: by then the agent has stopped and the user has had their say.
    pub async fn take_notice(&self, page: &PageId, session: &SessionId) -> Option<String> {
        self.notices
            .lock()
            .await
            .remove(&(page.clone(), session.clone()))
            .filter(|at| at.elapsed() < NOTICE_MAX)
            .map(|_| person_control_message())
    }

    /// True once for a call that was cancelled because the person took over its page.
    pub async fn take_interrupted(&self, dispatch: &DispatchId) -> bool {
        self.interrupted.lock().await.remove(dispatch)
    }

    /// Draws the overlay for an action neo is about to take on `page`. Refuses the first action
    /// after the person took the page over, including a take over the page made locally a moment
    /// ago.
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
        if let Some(refusal) = self.take_notice(&page, owner).await {
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
            let hold = holds
                .entry(page.clone())
                .or_insert_with(|| Hold::new(owner));
            hold.owner = owner.clone();
            hold.phase = HoldPhase::Working;
            hold.label.clone_from(&label);
            hold.since = Instant::now();
            hold.beat_at = Instant::now();
            hold.in_flight.insert(dispatch.clone(), owner.clone());
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
        // The page answered with a take over the server had not heard yet, which interrupted
        // this very call.
        if self.interrupted.lock().await.remove(dispatch) {
            return Err(person_control_message());
        }
        Ok(())
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

    /// Lets go of every page a finished call was still acting on, however it ended. A `run`
    /// script that was cancelled or timed out never reaches its own per-primitive end.
    pub async fn end_dispatch(&self, browser: &Arc<BrowserSession>, dispatch: &DispatchId) {
        for (page, state) in self.release_dispatch(dispatch).await {
            if let Ok(page_session) = browser.pages.get_session(page.clone()).await {
                let reply = overlay::show(&page_session.session, &state).await;
                self.handle_reply(&page, reply).await;
            }
        }
    }

    async fn release_dispatch(&self, dispatch: &DispatchId) -> Vec<(PageId, OverlayState)> {
        {
            let mut holds = self.holds.lock().await;
            holds
                .iter_mut()
                .filter_map(|(page, hold)| {
                    hold.in_flight.remove(dispatch)?;
                    if hold.phase != HoldPhase::Working || !hold.in_flight.is_empty() {
                        return None;
                    }
                    hold.phase = HoldPhase::Between;
                    hold.since = Instant::now();
                    hold.beat_at = Instant::now();
                    Some((page.clone(), hold.state()))
                })
                .collect()
        }
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

    /// Ends every hold of a finished session with "Finished", unless another session is still
    /// acting on the page.
    pub async fn finish_session(&self, owner: &SessionId) {
        let finished: Vec<PageId> = {
            let mut holds = self.holds.lock().await;
            let mut finished = Vec::new();
            holds.retain(|page, hold| {
                hold.in_flight.retain(|_, session| session != owner);
                if &hold.owner != owner {
                    return true;
                }
                if let Some(other) = hold.in_flight.values().next() {
                    hold.owner = other.clone();
                    return true;
                }
                finished.push(page.clone());
                false
            });
            finished
        };
        self.notices
            .lock()
            .await
            .retain(|(_, session), _| session != owner);
        self.started
            .lock()
            .await
            .retain(|(session, _)| session != owner);
        for page in finished {
            let Some(session) = self.page_session(&page).await else {
                continue;
            };
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
            let drained: Vec<PageId> = self
                .holds
                .lock()
                .await
                .drain()
                .map(|(page, _)| page)
                .collect();
            self.notices.lock().await.clear();
            for page in drained {
                if let Some(session) = self.page_session(&page).await {
                    overlay::dispose(&session).await;
                }
            }
            return;
        }
        self.notices
            .lock()
            .await
            .retain(|_, at| at.elapsed() < NOTICE_MAX);
        let now = Instant::now();
        let mut expired = Vec::new();
        let mut due = Vec::new();
        {
            let mut holds = self.holds.lock().await;
            holds.retain(|page, hold| {
                if hold.phase == HoldPhase::Working && hold.since.elapsed() >= WORKING_MAX {
                    hold.in_flight.clear();
                    hold.phase = HoldPhase::Between;
                    hold.since = now;
                }
                if hold.phase == HoldPhase::Between && hold.since.elapsed() >= HOLD_GRACE {
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
            OverlayEvent::ReachIn => {
                let held = {
                    let mut holds = self.holds.lock().await;
                    holds.get_mut(page).and_then(|hold| {
                        let seen = hold
                            .reach_in_at
                            .is_some_and(|at| at.elapsed() < REACH_IN_DEDUPE);
                        if seen {
                            return None;
                        }
                        hold.reach_in_at = Some(Instant::now());
                        Some((hold.owner.clone(), hold.phase))
                    })
                };
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

    /// The page has already removed its overlay. Ends the hold, interrupts the calls acting on the
    /// page, and leaves a notice for the sessions that had none in flight.
    async fn take_over(&self, page: &PageId, via: Via) {
        let Some(hold) = self.holds.lock().await.remove(page) else {
            return;
        };
        // An interrupted call tells its own session.
        if !hold
            .in_flight
            .values()
            .any(|session| session == &hold.owner)
        {
            self.notices
                .lock()
                .await
                .insert((page.clone(), hold.owner.clone()), Instant::now());
        }
        if !hold.in_flight.is_empty() {
            self.interrupted
                .lock()
                .await
                .extend(hold.in_flight.keys().cloned());
            for (dispatch, session) in &hold.in_flight {
                if let Some(session) = self.sessions.lookup(session).await {
                    session.interrupt_dispatch(dispatch).await;
                }
            }
        }
        let via = match via {
            Via::Button => "button",
            Via::Esc => "esc",
        };
        self.capture(events::PAGE_TAKEN_OVER, &hold.owner, json!({ "via": via }))
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
        hold.in_flight = in_flight
            .iter()
            .map(|dispatch| (dispatch.clone(), owner.clone()))
            .collect();
        self.holds.lock().await.insert(page, hold);
    }

    /// Records a call from another session acting on an already held page.
    pub(crate) async fn seed_call(
        &self,
        page: &PageId,
        dispatch: &DispatchId,
        session: &SessionId,
    ) {
        if let Some(hold) = self.holds.lock().await.get_mut(page) {
            hold.in_flight.insert(dispatch.clone(), session.clone());
        }
    }

    pub(crate) async fn release_for_test(&self, dispatch: &DispatchId) -> Vec<PageId> {
        self.release_dispatch(dispatch)
            .await
            .into_iter()
            .map(|(page, _)| page)
            .collect()
    }

    pub(crate) async fn tick_for_test(self: &Arc<Self>) {
        self.tick().await;
    }

    pub(crate) async fn phase(&self, page: &PageId) -> Option<HoldPhase> {
        self.holds.lock().await.get(page).map(|hold| hold.phase)
    }
}
