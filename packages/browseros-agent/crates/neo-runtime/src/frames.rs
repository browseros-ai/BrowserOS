//! Per-tab frame documents and the CDP hosts that own their utility worlds. Cached contexts
//! are discarded after a disconnected evaluation; a retry discovers the current document.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use browseros_cdp::{CdpError, CdpEvent, SessionId};
use neo_runtime_spec::InjectedCall;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::{CancellationToken, DropGuard};
use tokio_util::task::TaskTracker;

use crate::click::PageError;
use crate::page::Session;
use crate::run::PageId;
use crate::world::{self, ObjectGroup, World};

/// The document from which a call starts resolving selectors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FrameScope {
    Main,
}

type Contexts = Mutex<HashMap<String, Arc<FrameContext>>>;

/// One frame's document. Handles and cleanup retain this context, never a later document.
pub(crate) struct FrameContext {
    pub(crate) id: String,
    pub(crate) host: Arc<Host>,
    pub(crate) world: Arc<World>,
    contexts: Weak<Contexts>,
}

impl FrameContext {
    pub(crate) async fn call<C: InjectedCall>(
        &self,
        group: Option<&ObjectGroup>,
        params: &C,
    ) -> Result<C::Result, PageError> {
        let result = self.world.call(group, params).await;
        self.forget_if_disconnected(&result);
        result
    }

    pub(crate) async fn call_retaining_reply<C: InjectedCall>(
        &self,
        group: Option<&ObjectGroup>,
        params: &C,
    ) -> Result<(C::Result, bool), PageError> {
        let result = self.world.call_retaining_reply(group, params).await;
        self.forget_if_disconnected(&result);
        result
    }

    fn forget_if_disconnected<T>(&self, result: &Result<T, PageError>) {
        if matches!(result, Err(PageError::NotConnected))
            && let Some(contexts) = self.contexts.upgrade()
            && let Ok(mut contexts) = contexts.lock()
            && contexts
                .get(&self.id)
                .is_some_and(|frame| Arc::ptr_eq(&frame.world, &self.world))
        {
            contexts.remove(&self.id);
        }
    }
}

/// A CDP session retained by every context and cleanup that uses it.
pub(crate) struct Host {
    session: Arc<Session>,
    target_id: Option<String>,
    parent: Option<Arc<Host>>,
    gone: CancellationToken,
    _watch: Option<DropGuard>,
    contexts: Weak<Contexts>,
}

impl Host {
    pub(crate) async fn send(&self, method: &str, params: Value) -> Result<Value, CdpError> {
        let result = tokio::select! {
            biased;
            () = self.gone.cancelled() => Err(CdpError::SessionGone),
            result = self.session.cdp.send(method, params, Some(&self.session.id)) => result,
        };
        if result
            .as_ref()
            .is_err_and(|error| self.classify(error) == PageError::NotConnected)
            && let Some(contexts) = self.contexts.upgrade()
            && let Ok(mut contexts) = contexts.lock()
        {
            contexts.retain(|_, frame| frame.host.session_id() != self.session_id());
        }
        result
    }

    pub(crate) fn is_child(&self) -> bool {
        self.target_id.is_some()
    }

    pub(crate) fn session_id(&self) -> &SessionId {
        &self.session.id
    }

    pub(crate) fn classify(&self, error: &CdpError) -> PageError {
        world::classify_host(error, self.is_child())
    }
}

/// The main context stays on the tab session; local child contexts share their parent's host.
pub(crate) struct Frames {
    main: Arc<FrameContext>,
    contexts: Arc<Contexts>,
    tasks: TaskTracker,
}

impl Frames {
    pub(crate) fn new(
        session: Arc<Session>,
        id: String,
        world: Arc<World>,
        tasks: TaskTracker,
    ) -> Self {
        let contexts = Arc::new(Mutex::new(HashMap::new()));
        let main = Arc::new(FrameContext {
            id,
            host: Arc::new(Host {
                session,
                target_id: None,
                parent: None,
                gone: CancellationToken::new(),
                _watch: None,
                contexts: Arc::downgrade(&contexts),
            }),
            world,
            contexts: Arc::downgrade(&contexts),
        });
        Self {
            main,
            contexts,
            tasks,
        }
    }

    pub(crate) fn main(&self) -> Arc<FrameContext> {
        Arc::clone(&self.main)
    }

    pub(crate) async fn context(
        &self,
        frame: &str,
        parent: Option<&Arc<Host>>,
    ) -> Result<Arc<FrameContext>, PageError> {
        if frame == self.main.id {
            return Ok(self.main());
        }
        if let Some(context) = self.contexts.lock().map_err(|_| cache_error())?.get(frame) {
            return Ok(Arc::clone(context));
        }
        let parent = parent.unwrap_or(&self.main.host);
        // getTargetInfo alone can miss newly inserted OOPIFs. Listing targets discovers
        // their current DevTools hosts before attempting the lazy attach.
        let targets = self
            .main
            .host
            .session
            .cdp
            .send(
                "Target.getTargets",
                json!({"filter":[{"type":"iframe"}]}),
                None,
            )
            .await
            .map_err(|error| world::classify(&error))?;
        let targets = targets["targetInfos"].as_array().ok_or_else(|| {
            PageError::Contract("Target.getTargets returned no targetInfos".to_string())
        })?;
        let remote = targets
            .iter()
            .any(|target| target["targetId"] == frame && target["type"] == "iframe");
        let host = if remote {
            self.attach(frame, parent).await?
        } else {
            Arc::clone(parent)
        };
        let context = Arc::new(FrameContext {
            id: frame.to_string(),
            world: Arc::new(World::on_host(
                host.session.cdp.clone(),
                host.session_id().clone(),
                frame.to_string(),
                host.is_child(),
                host.gone.clone(),
            )),
            host,
            contexts: Arc::downgrade(&self.contexts),
        });
        let mut contexts = self.contexts.lock().map_err(|_| cache_error())?;
        Ok(Arc::clone(
            contexts.entry(frame.to_string()).or_insert(context),
        ))
    }

    async fn attach(&self, frame: &str, parent: &Arc<Host>) -> Result<Arc<Host>, PageError> {
        let cdp = self.main.host.session.cdp.clone();
        // Subscribe before attaching: even a detach delivered before the attach reply must
        // cancel this host. The tracked task reads a late reply and drops unclaimed sessions.
        let events = cdp.events();
        let session = self
            .tasks
            .spawn(Session::attach(
                cdp,
                PageId::new(frame.to_string()),
                Handle::current(),
            ))
            .await
            .map_err(|error| PageError::Contract(format!("attaching to a child frame: {error}")))?
            .map_err(|error| world::classify_host(&error, true))?;
        let gone = CancellationToken::new();
        let stop = CancellationToken::new();
        let watch = stop.clone().drop_guard();
        tokio::spawn(watch_host(
            events,
            session.id.clone(),
            gone.clone(),
            stop,
            Arc::downgrade(&self.contexts),
        ));
        Ok(Arc::new(Host {
            session: Arc::new(session),
            target_id: Some(frame.to_string()),
            parent: Some(Arc::clone(parent)),
            gone,
            _watch: Some(watch),
            contexts: Arc::downgrade(&self.contexts),
        }))
    }

    /// Quads are local to a process's root. Crossing each remote boundary adds the owner's
    /// content origin; local child quads already include their local ancestors' offsets.
    pub(crate) async fn offset(&self, host: &Arc<Host>) -> Result<Option<(f64, f64)>, PageError> {
        if !host.is_child() {
            return Ok(None);
        }
        let mut host = Arc::clone(host);
        let mut offset = (0.0, 0.0);
        while let Some(frame) = &host.target_id {
            let parent = host
                .parent
                .as_ref()
                .ok_or_else(|| PageError::Contract("A child host has no parent".to_string()))?;
            let owner = parent
                .send("DOM.getFrameOwner", json!({"frameId": frame}))
                .await
                .map_err(|error| parent.classify(&error))?;
            let backend = owner["backendNodeId"].as_i64().ok_or_else(|| {
                PageError::Contract("DOM.getFrameOwner returned no backendNodeId".to_string())
            })?;
            let model = parent
                .send("DOM.getBoxModel", json!({"backendNodeId":backend}))
                .await
                .map_err(|error| parent.classify(&error))?;
            let x = model["model"]["content"][0].as_f64().ok_or_else(|| {
                PageError::Contract("Frame content box has no x origin".to_string())
            })?;
            let y = model["model"]["content"][1].as_f64().ok_or_else(|| {
                PageError::Contract("Frame content box has no y origin".to_string())
            })?;
            offset.0 += x;
            offset.1 += y;
            host = Arc::clone(parent);
        }
        if host.session_id() != self.main.host.session_id() {
            return Err(PageError::NotConnected);
        }
        Ok(Some(offset))
    }

    pub(crate) fn forget(&self, stale: &Arc<FrameContext>) {
        if let Ok(mut contexts) = self.contexts.lock()
            && contexts
                .get(&stale.id)
                .is_some_and(|frame| Arc::ptr_eq(frame, stale))
        {
            contexts.remove(&stale.id);
        }
    }

    pub(crate) fn committed(&self, frame: &str) {
        if let Ok(mut contexts) = self.contexts.lock()
            && let Some(context) = contexts.remove(frame)
        {
            context.world.document_committed();
        }
    }
}

/// Chromium never answers some requests after removing an OOPIF. Fail them on detach instead
/// of waiting for CDP's request timeout; lost events conservatively discard this host too.
async fn watch_host(
    mut events: broadcast::Receiver<CdpEvent>,
    session: SessionId,
    gone: CancellationToken,
    stop: CancellationToken,
    contexts: Weak<Contexts>,
) {
    loop {
        let event = tokio::select! { biased; () = stop.cancelled() => return, event = events.recv() => event };
        match event {
            Ok(event)
                if event.method == "Target.detachedFromTarget"
                    && event.params["sessionId"].as_str() == Some(session.as_str()) =>
            {
                break;
            }
            Ok(_) => {}
            Err(RecvError::Closed | RecvError::Lagged(_)) => break,
        }
    }
    gone.cancel();
    if let Some(contexts) = contexts.upgrade()
        && let Ok(mut contexts) = contexts.lock()
    {
        contexts.retain(|_, context| {
            if context.host.session_id() != &session {
                return true;
            }
            context.world.document_committed();
            false
        });
    }
}

fn cache_error() -> PageError {
    PageError::Contract("neo-runtime: frame contexts cannot be accessed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_browser::{FakeBrowser, PAGE, Replies};
    use crate::run::PageId;

    #[tokio::test]
    async fn a_child_detach_cancels_its_host() {
        let (events, receiver) = broadcast::channel(16);
        let gone = CancellationToken::new();
        let watch = watch_host(
            receiver,
            SessionId::from("child-session"),
            gone.clone(),
            CancellationToken::new(),
            Weak::new(),
        );
        events
            .send(CdpEvent {
                method: "Target.detachedFromTarget".to_string(),
                params: json!({"sessionId":"child-session"}),
                session_id: None,
            })
            .unwrap();
        watch.await;
        assert!(gone.is_cancelled());
    }

    #[tokio::test]
    async fn stale_contexts_are_forgotten_without_evicting_their_replacement() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let cdp = browser.cdp().await;
        let session = Arc::new(
            Session::attach(
                cdp.clone(),
                PageId::new(PAGE.to_string()),
                Handle::current(),
            )
            .await
            .unwrap(),
        );
        let world = Arc::new(World::new(cdp, session.id.clone(), PAGE.to_string()));
        let frames = Frames::new(session, PAGE.to_string(), world, TaskTracker::new());
        let stale = frames
            .context("local-child", Some(&frames.main.host))
            .await
            .unwrap();
        frames.forget(&stale);
        let replacement = frames
            .context("local-child", Some(&frames.main.host))
            .await
            .unwrap();
        assert!(!Arc::ptr_eq(&stale, &replacement));
        frames.forget(&stale);
        assert!(Arc::ptr_eq(
            &replacement,
            &frames
                .context("local-child", Some(&frames.main.host))
                .await
                .unwrap()
        ));
        frames.committed("local-child");
        assert!(!Arc::ptr_eq(
            &replacement,
            &frames
                .context("local-child", Some(&frames.main.host))
                .await
                .unwrap()
        ));
    }
}
