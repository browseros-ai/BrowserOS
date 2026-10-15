//! Routes notifications by sessionId, never by whichever prompt ran most recently.
//! Registering a turn and publishing its terminal event share the route lock;
//! replay and idle transcript updates cannot leak into another turn. Control
//! notifications additionally refresh shared snapshots even without an active turn.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tokio::sync::mpsc;

use crate::{
    AcpError,
    process::lock,
    schema::{
        SessionConfigOption, SessionId, SessionModeState, SessionNotification, SessionUpdate,
    },
    session::{SessionSnapshot, TurnEvent, TurnState},
};

/// Registry shared by notification dispatch and public session operations. The
/// lock covers routing decisions only; neither SDK IO nor policy futures await it.
#[derive(Default)]
pub(crate) struct Routes(Mutex<HashMap<SessionId, Route>>);

#[derive(Default)]
struct Route {
    loading: bool,
    closed: bool,
    active: Option<Arc<TurnState>>,
    snapshot: Arc<Mutex<SessionSnapshot>>,
}

impl Routes {
    pub(crate) fn attach(
        &self,
        id: &SessionId,
        modes: Option<SessionModeState>,
        config: Vec<SessionConfigOption>,
    ) -> Arc<Mutex<SessionSnapshot>> {
        let mut routes = lock(&self.0);
        let route = routes.entry(id.clone()).or_default();
        *lock(&route.snapshot) = SessionSnapshot { modes, config };
        route.snapshot.clone()
    }

    pub(crate) fn loading(self: &Arc<Self>, id: &SessionId) -> Result<Loading, AcpError> {
        let mut routes = lock(&self.0);
        let route = routes.entry(id.clone()).or_default();
        if route.loading || route.active.is_some() {
            return Err(AcpError::TurnInProgress);
        }
        route.loading = true;
        Ok(Loading {
            routes: self.clone(),
            id: id.clone(),
            complete: false,
        })
    }

    pub(crate) fn start(
        &self,
        id: &SessionId,
        tx: mpsc::UnboundedSender<TurnEvent>,
    ) -> Result<Arc<TurnState>, AcpError> {
        let mut routes = lock(&self.0);
        let route = routes.entry(id.clone()).or_default();
        if route.closed {
            return Err(AcpError::Closed);
        }
        if route.loading || route.active.is_some() {
            return Err(AcpError::TurnInProgress);
        }
        let active = Arc::new(TurnState::new(tx));
        route.active = Some(active.clone());
        Ok(active)
    }

    pub(crate) fn update(&self, notification: SessionNotification) {
        let routes = lock(&self.0);
        if let Some(route) = routes.get(&notification.session_id)
            && !route.loading
            && !route.closed
        {
            // Snapshots contain only controls. Idle transcript/replay data has
            // no consumer and is never retained for the next prompt.
            let mut snapshot = lock(&route.snapshot);
            match &notification.update {
                SessionUpdate::CurrentModeUpdate(update) => {
                    // A notification can introduce mode state even when the
                    // attachment did not include an available-mode catalog.
                    let modes = snapshot.modes.get_or_insert_with(|| {
                        SessionModeState::new(update.current_mode_id.clone(), Vec::new())
                    });
                    modes.current_mode_id = update.current_mode_id.clone();
                }
                SessionUpdate::ConfigOptionUpdate(update) => {
                    snapshot.config = update.config_options.clone()
                }
                _ => {}
            }
            if let Some(active) = &route.active {
                let _ = active.tx.send(TurnEvent::Update(notification.update));
            }
        }
    }

    pub(crate) fn permission_turn(&self, id: &SessionId) -> Option<Arc<TurnState>> {
        lock(&self.0).get(id).and_then(|route| route.active.clone())
    }

    pub(crate) fn finish(&self, id: &SessionId, active: &Arc<TurnState>, event: TurnEvent) {
        let mut routes = lock(&self.0);
        if let Some(route) = routes.get_mut(id)
            && route
                .active
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, active))
        {
            active.finish();
            let _ = active.tx.send(event);
            route.active = None;
        }
    }

    pub(crate) fn close(&self, id: &SessionId) -> Result<(), AcpError> {
        let mut routes = lock(&self.0);
        let route = routes.entry(id.clone()).or_default();
        if route.active.is_some() || route.loading {
            return Err(AcpError::TurnInProgress);
        }
        route.closed = true;
        Ok(())
    }
}

/// RAII also clears replay suppression if the caller drops a load future.
/// Connection::control retires the process on a deadline, before this guard drops.
pub(crate) struct Loading {
    routes: Arc<Routes>,
    id: SessionId,
    complete: bool,
}
impl Loading {
    pub(crate) fn complete(&mut self) {
        // Reopen a closed alias only after the agent accepted reattachment;
        // an RPC refusal must not revive handles to a closed session.
        if let Some(route) = lock(&self.routes.0).get_mut(&self.id) {
            route.closed = false;
            route.loading = false;
        }
        // Completion happens at the response's wire position. Another alias
        // may begin a new load before this guard drops; never clear its flag.
        self.complete = true;
    }
}
impl Drop for Loading {
    fn drop(&mut self) {
        if !self.complete
            && let Some(route) = lock(&self.routes.0).get_mut(&self.id)
        {
            route.loading = false;
        }
    }
}
