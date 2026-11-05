//! `FileTransfers::upload_node` against a fake browser with one tab: what it sets, on which
//! session, and what it refuses before it touches the page.
use super::*;
use crate::{BrowserSession, BrowserSessionHooks, CdpConnection};
use browseros_cdp::{CdpError, CdpEvent, TargetedEvents};
use futures_util::future::BoxFuture;
use std::error::Error;
use std::sync::{Mutex, PoisonError};
use tokio::sync::{broadcast, mpsc};

type TestResult = Result<(), Box<dyn Error>>;

/// Backend nodes of the fake page. Any other id describes as a `<button>`.
const FILE_INPUT: i64 = 7;
const MULTIPLE_FILE_INPUT: i64 = 8;
const BUTTON: i64 = 9;

/// Answers the admission reads for one tab, attaches it as `page-session`, and describes the
/// nodes above. Every call is recorded with the session it was sent on.
struct Native {
    calls: Mutex<Vec<(String, Value, Option<SessionId>)>>,
    events: broadcast::Sender<CdpEvent>,
    targeted: TargetedEvents,
}

impl Native {
    fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(8);
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            events,
            targeted: TargetedEvents::default(),
        })
    }

    fn calls(&self, method: &str) -> Vec<(Value, Option<SessionId>)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(called, ..)| called == method)
            .map(|(_, params, session)| (params.clone(), session.clone()))
            .collect()
    }
}

impl CdpConnection for Native {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        let tab = json!({"tabId":1,"targetId":"target-1","windowId":10,"url":"https://site.test/",
            "title":"t","isActive":true,"isLoading":false,"loadProgress":1.0,"isPinned":false,
            "isHidden":false});
        let input =
            |attributes: Value| json!({"node":{"nodeName":"INPUT","attributes":attributes}});
        let reply = match method {
            "Browser.getTabs" => json!({ "tabs": [tab] }),
            "Browser.getTabInfo" => json!({ "tab": tab }),
            "Target.attachToTarget" => json!({ "sessionId": "page-session" }),
            "DOM.describeNode" => match params["backendNodeId"].as_i64() {
                Some(FILE_INPUT) => input(json!(["type", "file"])),
                Some(MULTIPLE_FILE_INPUT) => input(json!(["type", "file", "multiple", ""])),
                _ => json!({"node":{"nodeName":"BUTTON","attributes":[]}}),
            },
            _ => json!({}),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((method.to_string(), params, session.cloned()));
        Box::pin(async move { Ok(reply) })
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let params = serde_json::from_str(params_json)?;
            let reply = self.send(method, params, session).await?;
            serde_json::to_string(&reply).map_err(CdpError::from)
        })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.targeted.subscribe(method)
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn connection_epoch(&self) -> u64 {
        1
    }
}

/// A local file to upload, removed when the test ends, however it ends.
struct LocalFile(PathBuf);

impl LocalFile {
    fn new() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!("upload-node-{}.txt", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"fixture bytes")?;
        Ok(Self(path))
    }

    fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for LocalFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The fake browser, an unbound view over it, its one tab, and a frame session the node lives
/// in, which is not the tab's own session.
async fn fixture() -> Result<(Arc<Native>, BrowserView, PageId, ProtocolSession), CoreError> {
    let native = Native::new();
    let view = BrowserView::unbound(
        BrowserSession::new(native.clone(), BrowserSessionHooks::default()),
        None,
    );
    let page = view
        .list_pages()
        .await?
        .pages
        .into_iter()
        .next()
        .ok_or_else(|| CoreError::Message("fixture tab missing".into()))?
        .page_id;
    let frame = ProtocolSession::for_session(native.clone(), SessionId::from("frame-session"));
    Ok((native, view, page, frame))
}

#[tokio::test]
async fn tests_that_upload_node_sets_files_on_the_node_in_its_own_session() -> TestResult {
    let (native, view, page, frame) = fixture().await?;
    let file = LocalFile::new()?;

    view.files()
        .upload_node(page, &frame, FILE_INPUT, &[file.path()])
        .await?;

    assert_eq!(
        native.calls("DOM.setFileInputFiles"),
        [(
            json!({ "backendNodeId": FILE_INPUT, "files": [file.path()] }),
            Some(SessionId::from("frame-session"))
        )],
        "a backend node id only resolves on the session it was found in"
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_upload_node_refuses_a_node_that_is_not_a_file_input() -> TestResult {
    let (native, view, page, frame) = fixture().await?;
    let file = LocalFile::new()?;

    let refused = view
        .files()
        .upload_node(page, &frame, BUTTON, &[file.path()])
        .await
        .err()
        .ok_or("a button cannot take files")?;

    assert!(
        refused.to_string().contains("not a file input"),
        "{refused}"
    );
    assert!(
        native.calls("DOM.setFileInputFiles").is_empty(),
        "nothing is set on a node that is not a file input"
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_only_a_multiple_file_input_takes_several_files() -> TestResult {
    let (native, view, page, frame) = fixture().await?;
    let (first, second) = (LocalFile::new()?, LocalFile::new()?);
    let both = [first.path(), second.path()];

    let refused = view
        .files()
        .upload_node(page.clone(), &frame, FILE_INPUT, &both)
        .await
        .err()
        .ok_or("an input without `multiple` takes one file")?;
    assert!(refused.to_string().contains("only one file"), "{refused}");
    assert!(native.calls("DOM.setFileInputFiles").is_empty());

    view.files()
        .upload_node(page, &frame, MULTIPLE_FILE_INPUT, &both)
        .await?;
    assert_eq!(
        native.calls("DOM.setFileInputFiles"),
        [(
            json!({ "backendNodeId": MULTIPLE_FILE_INPUT, "files": both }),
            Some(SessionId::from("frame-session"))
        )],
        "a `multiple` input takes every file"
    );
    Ok(())
}

#[tokio::test]
async fn tests_that_upload_node_admits_the_page_before_it_checks_local_paths() -> TestResult {
    let (native, view, _, frame) = fixture().await?;
    let missing = std::env::temp_dir().join(format!("upload-node-{}.txt", uuid::Uuid::new_v4()));

    let refused = view
        .files()
        .upload_node(
            PageId(99),
            &frame,
            FILE_INPUT,
            &[missing.to_string_lossy().into_owned()],
        )
        .await
        .err()
        .ok_or("an unknown page is refused")?;

    assert!(
        matches!(refused, CoreError::UnknownPage(_)),
        "a foreign view must not learn which local paths exist: {refused}"
    );
    assert!(
        native.calls("DOM.describeNode").is_empty(),
        "nothing reaches the page"
    );
    Ok(())
}
