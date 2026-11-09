//! Per-tab frame documents and the CDP hosts that own their utility worlds. Cached contexts
//! are discarded after a disconnected evaluation; a retry discovers the current document.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use browseros_cdp::{CdpError, CdpEvent, SessionId};
use neo_runtime_spec::{Element, Handle, InjectedCall};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::{CancellationToken, DropGuard};
use tokio_util::task::TaskTracker;

use crate::browser_representations::page::Session;
use crate::chromium_bridge::main_world::MainWorld;
use crate::chromium_bridge::playwright_injected_methods::{self, ObjectGroup, World};
use crate::playwright_methods::click::PageError;
use crate::run::PageId;

/// The document from which a call starts resolving selectors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FrameScope {
    Main,
    Frame(String),
}

/// A live frame record, in pre-order when returned by `Frames::tree`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct FrameInfo {
    pub(crate) id: String,
    #[serde(default, rename = "parentId")]
    pub(crate) parent: Option<String>,
    #[serde(default)]
    pub(crate) name: String,
    pub(crate) url: String,
}

type Contexts = Mutex<HashMap<String, Arc<FrameContext>>>;

/// One frame's document. Handles and cleanup retain this context, never a later document.
pub(crate) struct FrameContext {
    pub(crate) id: String,
    pub(crate) host: Arc<Host>,
    pub(crate) world: Arc<World>,
    /// The same document's main world, for page evaluation (`main_world.rs`).
    pub(crate) main: MainWorld,
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

    pub(crate) async fn element(
        &self,
        group: &ObjectGroup,
        backend: i64,
    ) -> Result<Handle<Element>, PageError> {
        let result = self.world.element(group, backend).await;
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

    /// Cancelled when this host's session detaches: an OOPIF that was removed or moved back
    /// into its parent's process. Never cancelled for the tab's own host.
    pub(crate) fn gone(&self) -> &CancellationToken {
        &self.gone
    }

    pub(crate) fn session_id(&self) -> &SessionId {
        &self.session.id
    }

    /// Every CDP event the host's connection receives from now on; [`Host::session_id`] picks
    /// this host's.
    pub(crate) fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.session.cdp.events()
    }

    /// The child target this host attached, an out-of-process frame's id; `None` for the tab.
    pub(crate) fn target_id(&self) -> Option<&str> {
        self.target_id.as_deref()
    }

    pub(crate) fn classify(&self, error: &CdpError) -> PageError {
        playwright_injected_methods::classify_host(error, self.is_child())
    }
}

/// What the page does with an out-of-process frame's session once it is attached, before the
/// call that attached it goes on (`page_events.rs` `PageEvents::child_attached`).
pub(crate) type ChildHostHook =
    Arc<dyn Fn(Arc<Host>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// The main context stays on the tab session; local child contexts share their parent's host.
pub(crate) struct Frames {
    main: Arc<FrameContext>,
    contexts: Arc<Contexts>,
    tasks: TaskTracker,
    /// Set once by the page session: the run's console producers on each new OOPIF session.
    child_hook: OnceLock<ChildHostHook>,
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
            main: MainWorld::default(),
            contexts: Arc::downgrade(&contexts),
        });
        Self {
            main,
            contexts,
            tasks,
            child_hook: OnceLock::new(),
        }
    }

    /// Sets what every out-of-process frame session this page attaches from now on runs first.
    pub(crate) fn on_child_host(&self, hook: ChildHostHook) {
        let _first = self.child_hook.set(hook);
    }

    /// The out-of-process frame sessions this page has attached and still holds, one per session.
    pub(crate) fn child_hosts(&self) -> Vec<Arc<Host>> {
        let Ok(contexts) = self.contexts.lock() else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        contexts
            .values()
            .filter(|context| context.host.is_child() && !context.host.gone.is_cancelled())
            .filter(|context| seen.insert(context.host.session_id().clone()))
            .map(|context| Arc::clone(&context.host))
            .collect()
    }

    pub(crate) fn main(&self) -> Arc<FrameContext> {
        Arc::clone(&self.main)
    }

    /// `frame`'s context as the last [`Self::tree`] or hop left it, without CDP traffic.
    pub(crate) fn cached(&self, frame: &str) -> Option<Arc<FrameContext>> {
        if frame == self.main.id {
            return Some(self.main());
        }
        self.contexts.lock().ok()?.get(frame).cloned()
    }

    pub(crate) async fn context(
        &self,
        frame: &str,
        parent: Option<&Arc<Host>>,
    ) -> Result<Arc<FrameContext>, PageError> {
        if frame == self.main.id {
            return Ok(self.main());
        }
        if parent.is_none() {
            let tree = Box::pin(self.tree()).await?;
            if !tree.iter().any(|info| info.id == frame) {
                return Err(PageError::Threw("Frame was detached".to_string()));
            }
            return self
                .contexts
                .lock()
                .map_err(|_| cache_error())?
                .get(frame)
                .cloned()
                .ok_or(PageError::NotConnected);
        }
        if let Some(context) = self.contexts.lock().map_err(|_| cache_error())?.get(frame)
            && !context.host.gone.is_cancelled()
        {
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
            .map_err(|error| playwright_injected_methods::classify(&error))?;
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
        self.remember(frame, host)
    }

    fn remember(&self, frame: &str, host: Arc<Host>) -> Result<Arc<FrameContext>, PageError> {
        if frame == self.main.id {
            return Ok(self.main());
        }
        let mut contexts = self.contexts.lock().map_err(|_| cache_error())?;
        if let Some(context) = contexts.get(frame)
            && context.host.session_id() == host.session_id()
            && !context.host.gone.is_cancelled()
        {
            return Ok(Arc::clone(context));
        }
        let context = Arc::new(FrameContext {
            id: frame.to_string(),
            world: Arc::new(
                World::on_host(
                    host.session.cdp.clone(),
                    host.session_id().clone(),
                    frame.to_string(),
                    host.is_child(),
                    host.gone.clone(),
                )
                .with_test_id(self.main.world.test_id()),
            ),
            host,
            main: MainWorld::default(),
            contexts: Arc::downgrade(&self.contexts),
        });
        contexts.insert(frame.to_string(), Arc::clone(&context));
        Ok(context)
    }

    /// Chromium omits remote subtrees from Page.getFrameTree. Join each reachable iframe
    /// target to its parent's frame, then read that target's local subtree on its own host.
    /// A child disappearing during enumeration does not invalidate unrelated frame records.
    pub(crate) async fn tree(&self) -> Result<Vec<FrameInfo>, PageError> {
        let (frames, changed_host) = self.read_tree().await?;
        if changed_host {
            // The old target can disappear as the same frame moves to another process.
            // Re-read both sources before treating the missing subtree as detached.
            self.read_tree().await.map(|(frames, _)| frames)
        } else {
            Ok(frames)
        }
    }

    /// Neo: Reads fresh host trees and reports a child lost during enumeration.
    async fn read_tree(&self) -> Result<(Vec<FrameInfo>, bool), PageError> {
        let mut changed_host = false;
        let local = self.main.host.send("Page.getFrameTree", json!({}));
        let targets = self.main.host.session.cdp.send(
            "Target.getTargets",
            json!({"filter":[{"type":"iframe"}]}),
            None,
        );
        let (local, targets) = tokio::try_join!(local, targets)
            .map_err(|error| playwright_injected_methods::classify(&error))?;
        let mut frames = local_tree(&local)?;
        let targets = targets["targetInfos"].as_array().ok_or_else(|| {
            PageError::Contract("Target.getTargets returned no targetInfos".to_string())
        })?;
        let mut pending: Vec<Value> = targets
            .iter()
            .filter(|target| target["type"] == "iframe")
            .cloned()
            .collect();
        let mut hosts = HashMap::new();
        for frame in &frames {
            self.remember(&frame.id, Arc::clone(&self.main.host))?;
            hosts.insert(frame.id.clone(), Arc::clone(&self.main.host));
        }
        while let Some(index) = pending.iter().position(|target| {
            target_parent(target).is_some_and(|parent| hosts.contains_key(parent))
        }) {
            let target = pending.remove(index);
            let id = target["targetId"].as_str().ok_or_else(|| {
                PageError::Contract("An iframe target has no targetId".to_string())
            })?;
            let parent_id = target_parent(&target).ok_or(PageError::NotConnected)?;
            let parent = hosts
                .get(parent_id)
                .cloned()
                .ok_or(PageError::NotConnected)?;
            let cached = self
                .contexts
                .lock()
                .map_err(|_| cache_error())?
                .get(id)
                .cloned();
            if let Some(cached) = cached
                && (cached.host.target_id.as_deref() != Some(id)
                    || cached
                        .host
                        .parent
                        .as_ref()
                        .is_none_or(|host| host.session_id() != parent.session_id()))
            {
                self.forget(&cached);
            }
            let context = match self.context(id, Some(&parent)).await {
                Ok(context) => context,
                Err(PageError::NotConnected) => {
                    changed_host = true;
                    continue;
                }
                Err(error) => return Err(error),
            };
            let local = match context
                .host
                .send("Page.getFrameTree", json!({}))
                .await
                .map_err(|error| context.host.classify(&error))
            {
                Ok(local) => local,
                Err(PageError::NotConnected) => {
                    changed_host = true;
                    continue;
                }
                Err(error) => return Err(error),
            };
            for mut frame in local_tree(&local)? {
                if frame.id == id && frame.parent.is_none() {
                    frame.parent = Some(parent_id.to_string());
                }
                self.remember(&frame.id, Arc::clone(&context.host))?;
                hosts.insert(frame.id.clone(), Arc::clone(&context.host));
                if let Some(existing) = frames.iter_mut().find(|existing| existing.id == frame.id) {
                    *existing = frame;
                } else {
                    frames.push(frame);
                }
            }
        }
        Ok((preorder(&frames, &self.main.id), changed_host))
    }

    /// Parent worlds and child ids from main to leaf, for direct scopes' owner visibility
    /// and geometry. `tree` also refreshes the host of every context used below.
    pub(crate) async fn owners(
        &self,
        frame: &str,
    ) -> Result<Vec<(Arc<FrameContext>, String)>, PageError> {
        let tree = self.tree().await?;
        if !tree.iter().any(|info| info.id == frame) {
            return Err(PageError::Threw("Frame was detached".to_string()));
        }
        let mut child = frame;
        let mut path = Vec::new();
        let mut seen = HashSet::new();
        while child != self.main.id {
            if !seen.insert(child) {
                return Err(PageError::Contract(
                    "Frame tree contains a cycle".to_string(),
                ));
            }
            let parent = tree
                .iter()
                .find(|info| info.id == child)
                .and_then(|info| info.parent.as_deref())
                .ok_or(PageError::NotConnected)?;
            let context = if parent == self.main.id {
                self.main()
            } else {
                self.contexts
                    .lock()
                    .map_err(|_| cache_error())?
                    .get(parent)
                    .cloned()
                    .ok_or(PageError::NotConnected)?
            };
            path.push((context, child.to_string()));
            child = parent;
        }
        path.reverse();
        Ok(path)
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
                TokioHandle::current(),
            ))
            .await
            .map_err(|error| PageError::Contract(format!("attaching to a child frame: {error}")))?
            .map_err(|error| playwright_injected_methods::classify_host(&error, true))?;
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
        let host = Arc::new(Host {
            session: Arc::new(session),
            target_id: Some(frame.to_string()),
            parent: Some(Arc::clone(parent)),
            gone,
            _watch: Some(watch),
            contexts: Arc::downgrade(&self.contexts),
        });
        if let Some(hook) = self.child_hook.get() {
            hook(Arc::clone(&host)).await;
        }
        Ok(host)
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

    /// Playwright: `server/chromium/crPage.ts` `FrameSession._framePosition`: `const box = await element.boundingBox(nullProgress);`
    ///
    /// Where a box measured in `host`'s coordinates sits in the main frame's, as Playwright's
    /// `getBoundingBox` places it: crossing each remote boundary adds the owner iframe's
    /// border-box origin (the corner of its border quad), not where the frame's viewport starts
    /// (its content box, [`Frames::offset`], which input aims by). `(0, 0)` for the main
    /// process's frames; `None` when an owner on the way has no box, as Playwright's `null`.
    pub(crate) async fn border_offset(
        &self,
        host: &Arc<Host>,
    ) -> Result<Option<(f64, f64)>, PageError> {
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
            // Playwright's `_sendMayFail`: an owner with no box gives no position.
            let Ok(model) = parent
                .send("DOM.getBoxModel", json!({"backendNodeId": backend}))
                .await
            else {
                return Ok(None);
            };
            let Some((x, y)) = quad_origin(&model["model"]["border"]) else {
                return Ok(None);
            };
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

/// Playwright: `server/chromium/crPage.ts` `FrameSession._getBoundingBox`: `const x = Math.min(quad[0], quad[2], quad[4], quad[6]);`
///
/// The top-left corner of a quad: the smallest x and y of its four points.
fn quad_origin(quad: &Value) -> Option<(f64, f64)> {
    let points: Vec<f64> = quad
        .as_array()?
        .iter()
        .map(Value::as_f64)
        .collect::<Option<_>>()?;
    if points.len() != 8 {
        return None;
    }
    let x = [points[0], points[2], points[4], points[6]]
        .into_iter()
        .fold(f64::INFINITY, f64::min);
    let y = [points[1], points[3], points[5], points[7]]
        .into_iter()
        .fold(f64::INFINITY, f64::min);
    Some((x, y))
}

fn target_parent(target: &Value) -> Option<&str> {
    target["parentFrameId"]
        .as_str()
        .or_else(|| target["parentId"].as_str())
}

/// The recorded Page.getFrameTree payload, including local descendants and URL fragments.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalTree {
    frame: FrameInfo,
    #[serde(default)]
    child_frames: Vec<LocalTree>,
}

pub(crate) fn local_tree(reply: &Value) -> Result<Vec<FrameInfo>, PageError> {
    fn flatten(tree: LocalTree, frames: &mut Vec<FrameInfo>) {
        frames.push(tree.frame);
        for child in tree.child_frames {
            flatten(child, frames);
        }
    }
    let mut value = reply["frameTree"].clone();
    append_fragments(&mut value);
    let tree: LocalTree = serde_json::from_value(value)
        .map_err(|error| PageError::Contract(format!("Page.getFrameTree answered: {error}")))?;
    let mut frames = Vec::new();
    flatten(tree, &mut frames);
    Ok(frames)
}

fn append_fragments(tree: &mut Value) {
    if let (Some(url), Some(fragment)) = (
        tree["frame"]["url"].as_str(),
        tree["frame"]["urlFragment"].as_str(),
    ) {
        tree["frame"]["url"] = Value::String(format!("{url}{fragment}"));
    }
    if let Some(children) = tree.get_mut("childFrames").and_then(Value::as_array_mut) {
        for child in children {
            append_fragments(child);
        }
    }
}

fn preorder(frames: &[FrameInfo], main: &str) -> Vec<FrameInfo> {
    let by_id: HashMap<&str, &FrameInfo> = frames
        .iter()
        .map(|frame| (frame.id.as_str(), frame))
        .collect();
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for frame in frames {
        if let Some(parent) = &frame.parent {
            children.entry(parent).or_default().push(&frame.id);
        }
    }
    let mut ordered = Vec::new();
    let mut stack = vec![main];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(info) = by_id.get(id) {
            ordered.push((*info).clone());
        }
        if let Some(children) = children.get(id) {
            stack.extend(children.iter().rev().copied());
        }
    }
    ordered
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
    async fn tree_joins_recorded_local_and_remote_subtrees_in_preorder() {
        use crate::fake_browser::FrameReplies;

        let browser = FakeBrowser::start(Replies::default()).await;
        let main = json!({"frameTree":{"frame":{"id":PAGE,"url":"https://main.test/","urlFragment":"#top"},"childFrames":[{"frame":{"id":"local","parentId":PAGE,"name":"same","url":"https://main.test/leaf"}}]}});
        let remote = json!({"frameTree":{"frame":{"id":"remote","name":"cross","url":"https://other.test/"},"childFrames":[{"frame":{"id":"remote-local","parentId":"remote","name":"local-in-cross","url":"https://other.test/leaf"}}]}});
        let nested = json!({"frameTree":{"frame":{"id":"nested","parentId":"remote-local","name":"deep","url":"https://main.test/leaf","urlFragment":"#deep"}}});
        browser.page.lock().unwrap().frames = Some(FrameReplies {
            targets: vec![
                json!({"type":"iframe","targetId":"nested","parentId":"remote-local"}),
                json!({"type":"iframe","targetId":"foreign","parentFrameId":"another-tab"}),
                json!({"type":"iframe","targetId":"remote","parentFrameId":PAGE}),
            ],
            trees: [
                (PAGE.to_string(), main),
                ("remote".to_string(), remote),
                ("nested".to_string(), nested),
            ]
            .into(),
        });
        let cdp = browser.cdp().await;
        let session = Arc::new(
            Session::attach(
                cdp.clone(),
                PageId::new(PAGE.to_string()),
                TokioHandle::current(),
            )
            .await
            .unwrap(),
        );
        let world = Arc::new(World::new(cdp, session.id.clone(), PAGE.to_string()));
        let frames = Frames::new(session, PAGE.to_string(), world, TaskTracker::new());
        let tree = frames.tree().await.unwrap();
        assert_eq!(
            tree.iter()
                .map(|frame| frame.id.as_str())
                .collect::<Vec<_>>(),
            [PAGE, "local", "remote", "remote-local", "nested"]
        );
        assert_eq!(tree[0].url, "https://main.test/#top");
        assert_eq!(tree[2].parent.as_deref(), Some(PAGE));
        assert_eq!(tree[3].name, "local-in-cross");
        assert_eq!(tree[4].url, "https://main.test/leaf#deep");
        assert!(
            frames
                .context("nested", None)
                .await
                .unwrap()
                .host
                .is_child()
        );
        assert!(matches!(
            frames.context("foreign", None).await,
            Err(PageError::Threw(_))
        ));
    }

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
                TokioHandle::current(),
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
