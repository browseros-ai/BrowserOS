//! Per-tab frame documents and the CDP hosts that own their utility worlds. Cached contexts
//! are discarded after a disconnected evaluation; a retry discovers the current document.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use browseros_cdp::{CdpError, SessionId};
use neo_runtime_spec::InjectedCall;
use serde_json::Value;

use crate::click::PageError;
use crate::page::Session;
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
    child: bool,
}

impl Host {
    pub(crate) async fn send(&self, method: &str, params: Value) -> Result<Value, CdpError> {
        self.session
            .cdp
            .send(method, params, Some(&self.session.id))
            .await
    }

    pub(crate) fn session_id(&self) -> &SessionId {
        &self.session.id
    }

    pub(crate) fn classify(&self, error: &CdpError) -> PageError {
        if self.child && matches!(error, CdpError::SessionGone) {
            PageError::NotConnected
        } else {
            world::classify(error)
        }
    }
}

/// The main context stays on the tab session; local child contexts share their parent's host.
pub(crate) struct Frames {
    main: Arc<FrameContext>,
    contexts: Arc<Contexts>,
}

impl Frames {
    pub(crate) fn new(session: Arc<Session>, id: String, world: Arc<World>) -> Self {
        let contexts = Arc::new(Mutex::new(HashMap::new()));
        let main = Arc::new(FrameContext {
            id,
            host: Arc::new(Host {
                session,
                child: false,
            }),
            world,
            contexts: Arc::downgrade(&contexts),
        });
        Self { main, contexts }
    }

    pub(crate) fn main(&self) -> Arc<FrameContext> {
        Arc::clone(&self.main)
    }

    #[expect(
        clippy::unused_async,
        reason = "Frame host discovery becomes asynchronous for OOPIFs"
    )]
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
        let host = Arc::clone(parent.unwrap_or(&self.main.host));
        let context = Arc::new(FrameContext {
            id: frame.to_string(),
            world: Arc::new(World::new(
                host.session.cdp.clone(),
                host.session_id().clone(),
                frame.to_string(),
            )),
            host,
            contexts: Arc::downgrade(&self.contexts),
        });
        let mut contexts = self.contexts.lock().map_err(|_| cache_error())?;
        Ok(Arc::clone(
            contexts.entry(frame.to_string()).or_insert(context),
        ))
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

fn cache_error() -> PageError {
    PageError::Contract("neo-runtime: frame contexts cannot be accessed".to_string())
}

#[cfg(test)]
mod tests {
    use tokio::runtime::Handle;

    use super::*;
    use crate::fake_browser::{FakeBrowser, PAGE, Replies};
    use crate::run::PageId;

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
        let frames = Frames::new(session, PAGE.to_string(), world);
        let stale = frames.context("local-child", None).await.unwrap();
        frames.forget(&stale);
        let replacement = frames.context("local-child", None).await.unwrap();
        assert!(!Arc::ptr_eq(&stale, &replacement));
        frames.forget(&stale);
        assert!(Arc::ptr_eq(
            &replacement,
            &frames.context("local-child", None).await.unwrap()
        ));
        frames.committed("local-child");
        assert!(!Arc::ptr_eq(
            &replacement,
            &frames.context("local-child", None).await.unwrap()
        ));
    }
}
