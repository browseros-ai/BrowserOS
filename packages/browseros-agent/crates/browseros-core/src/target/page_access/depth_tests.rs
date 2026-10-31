//! Native request counts for frame admission through a chain of cross-site OOPIFs
//! root -> f1 -> ... -> fN, whose page trees omit remote frames as Chromium's do.
use super::*;
use crate::{Browser, CdpConnection, NativeCapabilities, TargetBinding, TargetIntent};
use browseros_cdp::{CdpError, CdpEvent};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::sync::broadcast;

type TestResult = Result<(), Box<dyn std::error::Error>>;
/// Each native request a test saw: its method and session.
type Calls = Vec<(String, Option<SessionId>)>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// The parent at this level reports another owner element for its child frame.
    OwnerMoved(usize),
    /// The endpoint at this level is gone.
    SessionGone(usize),
    /// The tab moves to another target once a frame owner is read.
    RetargetOnOwnerRead,
}

struct Chain {
    depth: usize,
    fault: Mutex<Option<Fault>>,
    retargeted: AtomicBool,
    calls: Mutex<Calls>,
    epoch: AtomicU64,
    events: broadcast::Sender<CdpEvent>,
}

fn tab(target: &str) -> Value {
    json!({"tabId":1,"targetId":target,"windowId":11,
        "url":"https://local.test","title":"test","isActive":true,"isLoading":false,
        "loadProgress":1.0,"isPinned":false,"isHidden":false,
        "browserProfileId":"eng","liveBrowserContextId":"live-eng"})
}

impl Chain {
    fn new(depth: usize) -> Arc<Self> {
        Arc::new(Self {
            depth,
            fault: Mutex::new(None),
            retargeted: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
            epoch: AtomicU64::new(1),
            events: broadcast::channel(64).0,
        })
    }

    /// 0 for the page's endpoint, k for fk's.
    fn level(session: Option<&SessionId>) -> usize {
        match session.map(SessionId::as_str) {
            None | Some("session-page") => 0,
            Some(id) => id
                .strip_prefix("session-f")
                .and_then(|n| n.parse().ok())
                .unwrap_or(usize::MAX),
        }
    }

    fn ax_tree(&self, level: usize) -> Value {
        let button = format!("button-{level}");
        let mut children = vec![button.clone()];
        let mut nodes = vec![json!({"nodeId":button,"backendDOMNodeId":10 + level,
            "role":{"type":"internalRole","value":"button"},
            "name":{"type":"computedString","value":format!("Target {level}")}})];
        if level < self.depth {
            let owner = format!("owner-{}", level + 1);
            children.push(owner.clone());
            nodes.push(json!({"nodeId":owner,"backendDOMNodeId":1001 + level,
                "role":{"type":"internalRole","value":"Iframe"},
                "name":{"type":"computedString","value":owner}}));
        }
        nodes.insert(
            0,
            json!({"nodeId":"document","role":{"type":"internalRole","value":"RootWebArea"},"childIds":children}),
        );
        json!({ "nodes": nodes })
    }

    fn reply(
        &self,
        expected: Option<u64>,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        if let Some(expected) = expected {
            let actual = self.connection_epoch();
            if expected != actual {
                return Err(CdpError::ConnectionEpochChanged { expected, actual });
            }
        }
        self.calls
            .lock()
            .map_err(|_| CdpError::SessionGone)?
            .push((method.into(), session.cloned()));
        let level = Self::level(session);
        let fault = *self.fault.lock().map_err(|_| CdpError::SessionGone)?;
        if fault == Some(Fault::SessionGone(level)) {
            return Err(CdpError::SessionGone);
        }
        Ok(match method {
            "Browser.getProfiles" => {
                json!({"profiles":[{"browserProfileId":"eng","name":"eng","directory":"directory-eng","loaded":true,"browserContextId":"live-eng"}]})
            }
            "Browser.getWindows" => {
                json!({"windows":[{"windowId":11,"windowType":"normal","bounds":{},"isActive":true,"isVisible":true,"tabCount":1,"browserProfileId":"eng","liveBrowserContextId":"live-eng"}]})
            }
            "Browser.getTabs" => json!({"tabs":[tab(self.target())]}),
            "Browser.getTabInfo" => json!({"tab":tab(self.target())}),
            "Target.attachToTarget" => {
                json!({"sessionId":format!("session-{}", params["targetId"].as_str().unwrap_or_default())})
            }
            "Page.getFrameTree" => match level {
                0 => json!({"frameTree":{"frame":{"id":"root"}}}),
                level => json!({"frameTree":{"frame":{"id":format!("f{level}")}}}),
            },
            "DOM.getFrameOwner" => {
                if fault == Some(Fault::RetargetOnOwnerRead) {
                    self.retargeted.store(true, Ordering::SeqCst);
                }
                let child: usize = params["frameId"]
                    .as_str()
                    .and_then(|id| id.strip_prefix('f'))
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                if child != level + 1 || child > self.depth {
                    return Err(CdpError::Protocol {
                        code: -32000,
                        message: "Frame with the given id does not belong to the target.".into(),
                    });
                }
                let moved = fault == Some(Fault::OwnerMoved(level));
                json!({"backendNodeId": if moved { 9999 } else { 1000 + child }})
            }
            "DOM.describeNode" => match params["backendNodeId"].as_i64() {
                Some(backend) if backend > 1000 => {
                    json!({"node":{"frameId":format!("f{}", backend - 1000)}})
                }
                _ => json!({"node":{}}),
            },
            "Accessibility.getFullAXTree" => self.ax_tree(level),
            "DOM.resolveNode" => json!({"object":{"objectId":"node"}}),
            "DOM.getContentQuads" => {
                json!({"quads":[[9.0, 9.0, 11.0, 9.0, 11.0, 11.0, 9.0, 11.0]]})
            }
            "Runtime.evaluate" => json!({"result":{"value":session.map(SessionId::as_str)}}),
            _ => json!({}),
        })
    }

    fn target(&self) -> &'static str {
        if self.retargeted.load(Ordering::SeqCst) {
            "page-moved"
        } else {
            "page"
        }
    }

    fn take(&self) -> Result<Calls, Box<dyn std::error::Error>> {
        Ok(std::mem::take(
            &mut *self.calls.lock().map_err(|_| "poisoned")?,
        ))
    }
}

impl CdpConnection for Chain {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { self.reply(None, method, params, session) })
    }
    fn send_at_epoch<'a>(
        &'a self,
        epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { self.reply(Some(epoch), method, params, session) })
    }
    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            Ok(serde_json::to_string(&self.reply(
                None,
                method,
                serde_json::from_str(params)?,
                session,
            )?)?)
        })
    }
    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn connection_epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }
}

async fn scoped_chain(depth: usize) -> Result<(Arc<Chain>, BrowserView, crate::PageId), CoreError> {
    let native = Chain::new(depth);
    let browser = Browser::new_with_native(
        native.clone(),
        NativeCapabilities::from_browseros_version(Some("0.51.1.3")),
    );
    browser
        .set_target(Some(TargetBinding {
            intent: TargetIntent {
                browser_profile_id: crate::BrowserProfileId("eng".into()),
                directory: "directory-eng".into(),
                name: "eng".into(),
                window: None,
            },
            revision: 1,
        }))
        .await;
    let view = browser.view().await;
    let page = view
        .list_pages()
        .await?
        .pages
        .into_iter()
        .next()
        .ok_or("missing page")?
        .page_id;
    Ok((native, view, page))
}

fn ref_named(snapshot: &crate::observer::SnapshotResult, name: &str) -> Result<crate::Ref, String> {
    snapshot
        .refs
        .entries_in_order()
        .into_iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.ref_id.clone())
        .ok_or_else(|| format!("no ref for {name:?} in:\n{}", snapshot.text))
}

fn count(calls: &Calls, method: &str, session: &str) -> usize {
    calls
        .iter()
        .filter(|(called, on)| {
            called == method && on.as_ref().map(SessionId::as_str) == Some(session)
        })
        .count()
}

#[tokio::test]
async fn tests_that_frame_admission_grows_linearly_with_oopif_depth() -> TestResult {
    let (mut calls, mut clicks) = (Vec::new(), Vec::new());
    for depth in 1..=5 {
        let (native, view, page) = scoped_chain(depth).await?;
        let observer = view.observe(page.clone()).await?;
        let snapshot = observer.snapshot().await?;
        for level in 0..=depth {
            ref_named(&snapshot, &format!("Target {level}"))?;
        }
        let deepest = ref_named(&snapshot, &format!("Target {depth}"))?;
        let resolved = observer.resolve_ref(&deepest).await?;
        native.take()?;
        let value = resolved
            .session
            .send_value("Runtime.evaluate", json!({"expression":"1"}))
            .await?;
        assert_eq!(value["result"]["value"], format!("session-f{depth}"));
        let call = native.take()?;
        // Before and after the call, each hop's owner is asked once, through its parent.
        assert_eq!(
            call.iter()
                .filter(|(method, _)| method == "DOM.getFrameOwner")
                .count(),
            2 * depth
        );
        calls.push(call.len());
        let input = view.input(page).await?;
        native.take()?;
        input
            .click(&deepest, crate::input::ClickOptions::default())
            .await?;
        clicks.push(native.take()?.len());
    }
    // Linear: each extra hop adds the same requests, where re-admitting every parent from
    // inside its child's check doubled them.
    for (label, counts) in [("call", &calls), ("click", &clicks)] {
        let steps = counts
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect::<Vec<_>>();
        assert!(
            steps.windows(2).all(|pair| pair[0] == pair[1]),
            "{label}: {counts:?}"
        );
    }
    assert_eq!(calls[1] - calls[0], 2, "{calls:?}");
    Ok(())
}

#[tokio::test]
async fn tests_that_an_intermediate_ancestor_refusal_stops_the_deepest_frame() -> TestResult {
    let (native, view, page) = scoped_chain(3).await?;
    let observer = view.observe(page).await?;
    let deepest = ref_named(&observer.snapshot().await?, "Target 3")?;
    let resolved = observer.resolve_ref(&deepest).await?;
    // f1 no longer reports f2's owner element.
    *native.fault.lock().map_err(|_| "poisoned")? = Some(Fault::OwnerMoved(1));
    native.take()?;
    let result = resolved
        .session
        .send_value("Runtime.evaluate", json!({"expression":"1"}))
        .await;
    assert!(
        matches!(&result, Err(CoreError::Target(TargetError::FrameNotInPage { frame, .. })) if frame.as_str() == "f2"),
        "{result:?}"
    );
    let calls = native.take()?;
    // The check stops at f2: f3's owner is never asked and nothing reaches f3.
    assert_eq!(
        count(&calls, "DOM.getFrameOwner", "session-f2"),
        0,
        "{calls:?}"
    );
    assert!(
        calls
            .iter()
            .all(|(_, on)| on.as_ref().map(SessionId::as_str) != Some("session-f3"))
    );

    // A transport failure on the way keeps its own error type.
    *native.fault.lock().map_err(|_| "poisoned")? = Some(Fault::SessionGone(1));
    let result = resolved
        .session
        .send_value("Runtime.evaluate", json!({"expression":"1"}))
        .await;
    assert!(
        matches!(result, Err(CoreError::Cdp(CdpError::SessionGone))),
        "{result:?}"
    );
    assert!(
        !native
            .take()?
            .iter()
            .any(|(method, _)| method == "Runtime.evaluate")
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_a_page_change_during_owner_reads_stops_the_call() -> TestResult {
    let (native, view, page) = scoped_chain(2).await?;
    let observer = view.observe(page).await?;
    let deepest = ref_named(&observer.snapshot().await?, "Target 2")?;
    let resolved = observer.resolve_ref(&deepest).await?;
    // The page is checked first, then each owner is read; the tab moves meanwhile.
    *native.fault.lock().map_err(|_| "poisoned")? = Some(Fault::RetargetOnOwnerRead);
    native.take()?;
    let result = resolved
        .session
        .send_value("Runtime.evaluate", json!({"expression":"1"}))
        .await;
    assert!(matches!(result, Err(CoreError::Target(_))), "{result:?}");
    assert!(
        !native
            .take()?
            .iter()
            .any(|(method, _)| method == "Runtime.evaluate")
    );
    Ok(())
}
