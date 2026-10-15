//! Routes notifications by sessionId, never by whichever prompt ran most recently.
//! Registering a turn and publishing its terminal event share the route lock;
//! replay and idle updates have no consumer and cannot leak into another turn.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tokio::sync::mpsc;

use crate::{
    AcpError,
    process::lock,
    schema::{SessionId, SessionNotification},
    session::{TurnEvent, TurnState},
};

#[derive(Default)]
pub(crate) struct Routes(Mutex<HashMap<SessionId, Route>>);

#[derive(Default)]
struct Route {
    loading: bool,
    closed: bool,
    active: Option<Arc<TurnState>>,
}

impl Routes {
    pub(crate) fn loading(self: &Arc<Self>, id: &SessionId) -> Result<Loading, AcpError> {
        let mut routes = lock(&self.0);
        let route = routes.entry(id.clone()).or_default();
        if route.loading || route.active.is_some() {
            return Err(AcpError::TurnInProgress);
        }
        route.loading = true;
        route.closed = false;
        Ok(Loading {
            routes: self.clone(),
            id: id.clone(),
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
            && let Some(active) = &route.active
        {
            let _ = active.tx.send(TurnEvent::Update(notification.update));
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
}
impl Drop for Loading {
    fn drop(&mut self) {
        if let Some(route) = lock(&self.routes.0).get_mut(&self.id) {
            route.loading = false;
        }
    }
}
