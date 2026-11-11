//! Page media at the browser's renderer-initialization seam. Core pauses new OOPIFs, invokes
//! this lease before resume, and asks it to rediscover ancestry after event lag. Bindings belong
//! to one CDP epoch; the actual preferences stay on `PageState` across runs and reconnects.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use browseros_core::frames::FrameInitializer;
use browseros_core::{BrowserSession, CoreError, PageId, ProtocolSession, SessionId};
use futures_util::future::BoxFuture;

use crate::page::frame_tree;
use crate::page::media_emulation::{self, MediaState};
use tokio::sync::Mutex as AsyncMutex;

/// One renderer's inherited page state and whether its initial configuration has completed.
#[derive(Clone)]
struct Binding {
    session: ProtocolSession,
    state: Arc<AsyncMutex<MediaState>>,
    ready: Arc<AtomicBool>,
}

#[derive(Clone)]
struct Root {
    page: PageId,
    state: Arc<AsyncMutex<MediaState>>,
}

#[derive(Default)]
struct Bindings {
    sessions: HashMap<SessionId, Binding>,
    roots: HashMap<PageId, Root>,
}

/// Weak browser ownership avoids a cycle through core's initializer registry. Losing the
/// transport epoch discards endpoint identities while retaining `PageState`'s preferences.
struct MediaBindings {
    browser: Weak<BrowserSession>,
    epoch: AtomicU64,
    bindings: Mutex<Bindings>,
}

impl MediaBindings {
    fn current(&self) -> MutexGuard<'_, Bindings> {
        let mut bindings = self.bindings.lock().unwrap_or_else(PoisonError::into_inner);
        let epoch = self
            .browser
            .upgrade()
            .map_or(0, |browser| browser.connection().connection_epoch());
        if self.epoch.swap(epoch, Ordering::Relaxed) != epoch {
            bindings.sessions.clear();
            // Core page identities survive reconnects; retain their preferences for root reattachment.
        }
        bindings
    }

    fn remember(&self, session: &ProtocolSession, state: Arc<AsyncMutex<MediaState>>) -> Binding {
        let mut bindings = self.current();
        let Some(id) = session.session_id() else {
            return Binding {
                session: session.at_connection_epoch(),
                state,
                ready: Arc::new(AtomicBool::new(false)),
            };
        };
        if let Some(binding) = bindings.sessions.get(id)
            && Arc::ptr_eq(&binding.state, &state)
        {
            return binding.clone();
        }
        let binding = Binding {
            session: session.at_connection_epoch(),
            state,
            ready: Arc::new(AtomicBool::new(false)),
        };
        bindings.sessions.insert(id.clone(), binding.clone());
        binding
    }

    async fn ensure(
        &self,
        session: &ProtocolSession,
        state: &Arc<AsyncMutex<MediaState>>,
    ) -> Result<(), CoreError> {
        let session = session.at_connection_epoch();
        let binding = self.remember(&session, Arc::clone(state));
        let guard = Arc::clone(state).lock_owned().await;
        if !binding.ready.load(Ordering::Acquire) {
            if guard.is_emulated() {
                if let Some(browser) = self.browser.upgrade() {
                    browser.frames().enable_initialization(&session).await?;
                }
                guard.replay(&session).await?;
            }
            binding.ready.store(true, Ordering::Release);
        }
        Ok(())
    }
}

/// The facade owns this lease. Core keeps only a weak initializer, so dropping Playwright
/// releases all media bindings without leaving a browser task or a strong ownership cycle.
pub(crate) struct MediaSessions {
    inner: Arc<MediaBindings>,
    _initializer: Arc<dyn FrameInitializer>,
}

struct MediaInitializer {
    inner: Weak<MediaBindings>,
}

impl MediaSessions {
    pub(crate) fn new(browser: &Arc<BrowserSession>) -> Self {
        let inner = Arc::new(MediaBindings {
            browser: Arc::downgrade(browser),
            epoch: AtomicU64::new(browser.connection().connection_epoch()),
            bindings: Mutex::default(),
        });
        let initializer: Arc<dyn FrameInitializer> = Arc::new(MediaInitializer {
            inner: Arc::downgrade(&inner),
        });
        browser.frames().register_initializer(&initializer);
        Self {
            inner,
            _initializer: initializer,
        }
    }

    /// Root identities are retained for proactive recovery of a missed attach, including
    /// between runs when no caller is reading the frame tree.
    pub(crate) async fn ensure_root(
        &self,
        page: &PageId,
        session: &ProtocolSession,
        state: &Arc<AsyncMutex<MediaState>>,
    ) -> Result<(), CoreError> {
        self.inner.current().roots.insert(
            page.clone(),
            Root {
                page: page.clone(),
                state: Arc::clone(state),
            },
        );
        self.ensure(session, state).await
    }

    pub(crate) fn remember(&self, session: &ProtocolSession, state: Arc<AsyncMutex<MediaState>>) {
        self.inner.remember(session, state);
    }

    pub(crate) async fn ensure(
        &self,
        session: &ProtocolSession,
        state: &Arc<AsyncMutex<MediaState>>,
    ) -> Result<(), CoreError> {
        self.inner.ensure(session, state).await
    }

    /// Include renderers initialized while discovery ran. Holding the page lock at this point
    /// orders this snapshot against initialization, so a new child gets this update or replays
    /// the committed value before resume. Epoch pruning excludes endpoints from old sockets.
    pub(crate) fn sessions(&self, state: &Arc<AsyncMutex<MediaState>>) -> Vec<ProtocolSession> {
        self.inner
            .current()
            .sessions
            .values()
            .filter(|binding| Arc::ptr_eq(&binding.state, state))
            .map(|binding| binding.session.clone())
            .collect()
    }

    pub(crate) fn forget(&self, session: &ProtocolSession) {
        if let Some(id) = session.session_id() {
            self.inner.current().sessions.remove(id);
        }
    }
}

impl FrameInitializer for MediaInitializer {
    fn initialize<'a>(
        &'a self,
        session: &'a ProtocolSession,
        parent: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<(), CoreError>> {
        Box::pin(async move {
            let Some(inner) = self.inner.upgrade() else {
                return Ok(());
            };
            let inherited = {
                let bindings = inner.current();
                parent
                    .and_then(|parent| bindings.sessions.get(parent))
                    .cloned()
                    .or_else(|| {
                        session
                            .session_id()
                            .and_then(|id| bindings.sessions.get(id))
                            .cloned()
                    })
            };
            if let Some(binding) = inherited {
                let binding = inner.remember(session, binding.state);
                // Core has already armed recursive attachment. The same page lock serializes
                // initialization with media changes and cancellation restoration before resume.
                let guard = Arc::clone(&binding.state).lock_owned().await;
                guard.replay(session).await?;
                binding.ready.store(true, Ordering::Release);
            }
            Ok(())
        })
    }

    fn page_attached<'a>(
        &'a self,
        session: &'a ProtocolSession,
        page: &'a PageId,
    ) -> BoxFuture<'a, Result<(), CoreError>> {
        Box::pin(async move {
            let Some(inner) = self.inner.upgrade() else {
                return Ok(());
            };
            let saved = inner.current().roots.get(page).cloned();
            if let Some(root) = saved {
                let binding = inner.remember(session, Arc::clone(&root.state));
                let guard = root.state.lock_owned().await;
                guard.replay(session).await?;
                binding.ready.store(true, Ordering::Release);
            }
            Ok(())
        })
    }

    fn recover(&self) -> BoxFuture<'_, Result<(), CoreError>> {
        Box::pin(async move {
            let Some(inner) = self.inner.upgrade() else {
                return Ok(());
            };
            let Some(browser) = inner.browser.upgrade() else {
                return Ok(());
            };
            let roots: Vec<_> = inner.current().roots.values().cloned().collect();
            let mut failure = None;
            for root in roots {
                let session = match browser.pages.get_session(root.page.clone()).await {
                    Ok(attached) => attached.session.at_connection_epoch(),
                    Err(error) => {
                        failure = Some(error);
                        continue;
                    }
                };
                match frame_tree::read(&browser, &root.page, &session).await {
                    Ok(tree) => {
                        for frame in tree.ids() {
                            if let Some(session) = tree.endpoint(frame)
                                && let Err(error) = inner.ensure(session, &root.state).await
                                && !media_emulation::child_closed(&error)
                            {
                                failure = Some(error);
                            }
                        }
                    }
                    Err(error) if media_emulation::child_closed(&error) => {
                        inner.current().roots.remove(&root.page);
                    }
                    Err(error) => {
                        failure = Some(error);
                    }
                }
            }
            failure.map_or(Ok(()), Err)
        })
    }
}
