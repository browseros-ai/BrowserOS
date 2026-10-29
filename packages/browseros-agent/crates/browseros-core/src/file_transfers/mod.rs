//! File selection and download capture behind one scoped browser interface. BrowserSession
//! owns chooser state across calls; this handle retains the caller's immutable BrowserView.
//! The browser keeps download naming/history, while all local-file checks happen beside it.
mod state;
pub(crate) use state::TransferState;

use crate::{BrowserView, CoreError, FrameId, PageId, PageJs, ProtocolSession, Ref, SessionId};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
const CHOOSER_TIMEOUT: Duration = Duration::from_secs(10);

/// Recoverable file-input selection, scoped internally to one document/session request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChooser {
    pub(crate) epoch: u64,
    pub(crate) request: u64,
    pub(crate) session: SessionId,
    pub(crate) frame: FrameId,
    pub(crate) backend_node_id: Option<i64>,
    pub multiple: bool,
}

/// A completed file at the path chosen and recorded by the browser.
#[derive(Debug)]
pub struct DownloadedFile {
    pub path: PathBuf,
    pub filename: String,
    pub bytes: u64,
}

/// The file-transfer seam used by MCP and embedded scripts. Chooser node/session handles
/// remain internal: callers can target an input/opener ref or finish a pending chooser.
pub struct FileTransfers {
    pub(crate) view: BrowserView,
    pub(crate) state: Arc<TransferState>,
}

impl FileTransfers {
    /// Observe without changing Chrome's destination or collision handling. Enabling events
    /// is connection/context setup, serialized across callers rather than toggled per click.
    async fn enable_downloads(&self) -> Result<(), CoreError> {
        let scope = self.view.scope()?;
        let key = (
            scope.map_or(self.state.epoch(), |scope| scope.connection_epoch),
            scope
                .and_then(|scope| scope.context_id.as_ref())
                .map(|id| id.as_str().to_owned()),
        );
        let mut contexts = self.state.download_contexts.lock().await;
        if !contexts.contains(&key) {
            self.view
                .cdp(
                    "Browser.setDownloadBehavior",
                    json!({"behavior":"default","eventsEnabled":true}),
                    None,
                )
                .await?;
            contexts.retain(|(epoch, _)| *epoch == key.0);
            contexts.insert(key);
        }
        Ok(())
    }

    pub async fn download(
        &self,
        page: PageId,
        reference: &Ref,
    ) -> Result<DownloadedFile, CoreError> {
        let lock = self.state.operation(page.clone());
        let _operation = lock.lock_owned().await;
        let session = self.view.page_session(page.clone()).await?;
        self.enable_downloads().await?;
        // Subscribe before dispatch: tiny/blob downloads can finish before the click ack.
        let mut events = self.view.cdp_events();
        let input = self.view.input(page.clone()).await?;
        let capture = async {
            let mut guid = None::<String>;
            loop {
                let event = events.recv().await.map_err(|error| {
                    CoreError::Message(format!("Download event stream lost: {error}"))
                })?;
                match event.method.as_str() {
                    "Browser.downloadWillBegin" if guid.is_none() => {
                        if let Some(frame) = event.params["frameId"].as_str() {
                            // A click may create the iframe that initiates its download. Prove
                            // live ownership at the event, rather than freezing the frame tree
                            // before the click or accepting another page's browser-wide event.
                            match self
                                .view
                                .frame_session(page.clone(), FrameId(frame.into()))
                                .await
                            {
                                Ok(_) => guid = event.params["guid"].as_str().map(str::to_owned),
                                Err(error @ CoreError::Target(_)) => return Err(error),
                                Err(_) => {}
                            }
                        }
                    }
                    "Browser.downloadProgress"
                        if guid.as_deref().is_some_and(|id| event.params["guid"] == id) =>
                    {
                        match event.params["state"].as_str() {
                            Some("canceled") => {
                                return Err(CoreError::Message("Download was canceled".into()));
                            }
                            Some("completed") => {
                                let path = event.params["filePath"].as_str().filter(|path| !path.is_empty())
                                    .ok_or_else(|| CoreError::Message("Download completed but the browser did not report the saved path".into()))?;
                                return downloaded_file(PathBuf::from(path)).await;
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        };
        let transfer = async {
            tokio::pin!(capture);
            // The renderer may wait on a download while acknowledging input; let either
            // completion path progress without making event observation depend on that ack.
            tokio::select! {
                result = input.click(reference, Default::default()) => { result?; capture.await }
                result = &mut capture => result,
            }
        };
        let result = tokio::time::timeout(DOWNLOAD_TIMEOUT, transfer).await
            .map_err(|_| CoreError::Message("Download timed out after 60000ms; check for a Save As dialog or a blocked download".into()))??;
        if self.view.page(page.clone()).await?.target_id != session.target_id {
            return Err(crate::TargetError::PageTargetChanged(page).into());
        }
        Ok(result)
    }

    /// Pending selection is recoverable across calls, but only while its input's frame and
    /// session still belong to this live page. Never return a foreign profile's chooser.
    pub async fn pending_chooser(&self, page: PageId) -> Result<Option<FileChooser>, CoreError> {
        if !self.state.has_pending() {
            return Ok(None);
        }
        let root = self.view.page_session(page.clone()).await?;
        if let Some(error) = self.state.setup_error(&root.session_id) {
            return Err(CoreError::Message(format!(
                "File chooser interception failed: {error}"
            )));
        }
        for chooser in self.state.choosers() {
            match self
                .view
                .frame_session(page.clone(), chooser.frame.clone())
                .await
            {
                Ok(session)
                    if session.session_id() == Some(&chooser.session)
                        || chooser.session == root.session_id =>
                {
                    return Ok(Some(chooser));
                }
                Err(error @ CoreError::Target(_)) => return Err(error),
                _ => {}
            }
        }
        Ok(None)
    }

    /// Input acknowledgements can precede the UI-thread chooser notification. Give that
    /// handoff a small bounded observation window; ordinary pending-state reads never wait.
    pub async fn chooser_after_action(
        &self,
        page: PageId,
    ) -> Result<Option<FileChooser>, CoreError> {
        if !self.state.has_interception() && !self.state.has_pending() {
            return Ok(None);
        }
        let mut changes = self.state.changes();
        let deadline = tokio::time::sleep(Duration::from_millis(100));
        tokio::pin!(deadline);
        loop {
            if let Some(chooser) = self.pending_chooser(page.clone()).await? {
                return Ok(Some(chooser));
            }
            tokio::select! {
                () = &mut deadline => return Ok(None),
                result = changes.changed() => if result.is_err() { return Ok(None); },
            }
        }
    }

    pub async fn upload(
        &self,
        page: PageId,
        reference: Option<&Ref>,
        files: &[String],
    ) -> Result<(), CoreError> {
        let lock = self.state.operation(page.clone());
        let _operation = lock.lock_owned().await;
        // Admission precedes even local-path validation: a stale/foreign view must not turn
        // this interface into a filesystem existence probe through its error messages.
        self.view.page(page.clone()).await?;
        // Fail before clicking or consuming a chooser, so fixing a bad path is retryable.
        for file in files {
            let path = PathBuf::from(file);
            let metadata = tokio::fs::metadata(&path)
                .await
                .map_err(|error| CoreError::Message(format!("Cannot upload {file}: {error}")))?;
            if !metadata.is_file() {
                return Err(CoreError::Message(format!(
                    "Upload path is not a file: {file}"
                )));
            }
            tokio::fs::File::open(&path).await.map_err(|error| {
                CoreError::Message(format!("Cannot read upload file {file}: {error}"))
            })?;
        }
        if let Some(reference) = reference {
            if self.pending_chooser(page.clone()).await?.is_some() {
                return Err(CoreError::Message(
                    "A file chooser is already pending; omit ref to finish it".into(),
                ));
            }
            let resolved = self
                .view
                .observe(page.clone())
                .await?
                .resolve_ref(reference)
                .await?;
            let node = resolved
                .session
                .send_value(
                    "DOM.describeNode",
                    json!({"backendNodeId":resolved.backend_node_id}),
                )
                .await?;
            if is_file_input(&node["node"]) {
                let multiple = has_attribute(&node["node"], "multiple");
                return set_files(&resolved.session, resolved.backend_node_id, multiple, files)
                    .await;
            }
            let session_id = resolved.session.session_id().cloned();
            // Arm restoration before setup's first await: cancellation can arrive while
            // Chrome is acknowledging interception, before setup returns an installed flag.
            let mut lease = InterceptionLease {
                state: self.state.clone(),
                operation: Some(_operation),
                session: session_id.clone().filter(|id| !self.state.intercepts(id)),
            };
            match self.state.intercept(&resolved.session).await {
                Ok(installed) => lease.session = installed.then_some(session_id).flatten(),
                Err(error) => {
                    if let Err(cleanup) = lease.restore().await {
                        tracing::warn!(%cleanup, "file chooser setup failed and cleanup remains pending");
                    }
                    return Err(error);
                }
            }
            let mut changes = self.state.changes();
            let wait = async {
                loop {
                    if let Some(chooser) = self.pending_chooser(page.clone()).await? {
                        return Ok::<_, CoreError>(chooser);
                    }
                    changes.changed().await.map_err(|_| {
                        CoreError::Message("File chooser event stream closed".into())
                    })?;
                }
            };
            let input = self.view.input(page.clone()).await?;
            let select = async {
                tokio::pin!(wait);
                let chooser = tokio::select! {
                    result = input.click(reference, Default::default()) => { result?; wait.await? }
                    result = &mut wait => result?,
                };
                self.select_chooser(page.clone(), &chooser, files).await
            };
            let result = tokio::time::timeout(CHOOSER_TIMEOUT, select)
                .await
                .map_err(|_| {
                    CoreError::Message(
                        "Upload control did not open a file chooser within 10000ms".into(),
                    )
                })?;
            lease.restore().await?;
            result
        } else {
            let chooser = self.pending_chooser(page.clone()).await?.ok_or_else(|| {
                CoreError::Message(
                    "No file chooser is pending; provide a file input or upload-button ref".into(),
                )
            })?;
            self.select_chooser(page, &chooser, files).await
        }
    }

    async fn select_chooser(
        &self,
        page: PageId,
        chooser: &FileChooser,
        files: &[String],
    ) -> Result<(), CoreError> {
        let Some(node) = chooser.backend_node_id else {
            self.state.clear(chooser);
            return Err(CoreError::Message("This picker does not expose a file input; automated selection is currently unsupported".into()));
        };
        if files.len() > 1 && !chooser.multiple {
            return Err(CoreError::Message(
                "This file input accepts only one file".into(),
            ));
        }
        let session = self
            .view
            .frame_session(page.clone(), chooser.frame.clone())
            .await?;
        // Recheck after async admission: navigation or another selector must not replace the
        // request while a local file or a profile lookup is being awaited.
        if !self.state.choosers().contains(chooser) {
            return Err(CoreError::Message(
                "The file chooser is stale; open it again".into(),
            ));
        }
        // A page can remove or replace its hidden input without navigating. Do not leave a
        // dead request permanently blocking later opener refs; cardinality errors above
        // retain the request, while a missing/replaced native node requires opening it again.
        let node_info = session
            .send_value("DOM.describeNode", json!({"backendNodeId":node}))
            .await;
        match node_info {
            Ok(info) if is_file_input(&info["node"]) => {}
            Ok(_) => {
                self.state.clear(chooser);
                return Err(CoreError::Message(
                    "The chooser input changed; open the chooser again".into(),
                ));
            }
            Err(error) => {
                self.state.clear(chooser);
                return Err(error);
            }
        }
        let connected = PageJs::isolated(&session)
            .call::<bool>(node, "function() { return this.isConnected; }", &[])
            .await;
        match connected {
            Ok(true) => {}
            Ok(false) => {
                self.state.clear(chooser);
                return Err(CoreError::Message(
                    "The chooser input was removed; open the chooser again".into(),
                ));
            }
            Err(error) => {
                self.state.clear(chooser);
                return Err(error);
            }
        }
        if let Err(error) = set_files(&session, node, chooser.multiple, files).await {
            self.state.clear(chooser);
            return Err(error);
        }
        self.view.page(page).await?;
        self.state.clear(chooser);
        Ok(())
    }
}

/// Holds a borrowed page's override and operation lock until restoration, even on cancellation.
struct InterceptionLease {
    state: Arc<TransferState>,
    session: Option<SessionId>,
    operation: Option<tokio::sync::OwnedMutexGuard<()>>,
}
impl InterceptionLease {
    async fn restore(&mut self) -> Result<(), CoreError> {
        if let Some(session) = self.session.as_ref() {
            self.state.restore(session).await?;
            self.session = None;
        }
        self.operation.take();
        Ok(())
    }
}
impl Drop for InterceptionLease {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            let state = self.state.clone();
            let operation = self.operation.take();
            // Keep the page lock until restoration finishes: otherwise an old cancellation
            // could disable interception underneath the next upload on this borrowed page.
            // Cancellation drops the upload future. Restore a borrowed tab even then,
            // using the retained epoch and session identity rather than a new capability.
            tokio::spawn(async move {
                if let Err(error) = state.restore(&session).await {
                    tracing::warn!(%error, "could not restore file chooser interception");
                }
                drop(operation);
            });
        }
    }
}

async fn set_files(
    session: &ProtocolSession,
    node: i64,
    multiple: bool,
    files: &[String],
) -> Result<(), CoreError> {
    if files.len() > 1 && !multiple {
        return Err(CoreError::Message(
            "This file input accepts only one file".into(),
        ));
    }
    session
        .send_value(
            "DOM.setFileInputFiles",
            json!({"backendNodeId":node,"files":files}),
        )
        .await?;
    // Chrome can treat an empty native file list as cancelling the chooser and preserve
    // its previous selection. Our explicit files=[] contract clears it; use the isolated
    // world so page-installed setters cannot observe or replace this internal operation.
    if files.is_empty()
        && !PageJs::isolated(session)
            .call::<bool>(
                node,
                "function() { if (this.files.length) { this.value = ''; this.dispatchEvent(new Event('input', { bubbles: true, composed: true })); this.dispatchEvent(new Event('change', { bubbles: true })); } return this.files.length === 0; }",
                &[],
            )
            .await?
    {
        return Err(CoreError::Message("Could not clear the file selection".into()));
    }
    Ok(())
}
fn has_attribute(node: &Value, name: &str) -> bool {
    node["attributes"]
        .as_array()
        .is_some_and(|attributes| attributes.chunks(2).any(|pair| pair[0] == name))
}
fn is_file_input(node: &Value) -> bool {
    node["nodeName"]
        .as_str()
        .is_some_and(|name| name.eq_ignore_ascii_case("input"))
        && node["attributes"].as_array().is_some_and(|attributes| {
            attributes.chunks(2).any(|pair| {
                pair.first().is_some_and(|name| name == "type")
                    && pair
                        .get(1)
                        .and_then(Value::as_str)
                        .is_some_and(|kind| kind.eq_ignore_ascii_case("file"))
            })
        })
}
async fn downloaded_file(path: PathBuf) -> Result<DownloadedFile, CoreError> {
    if !path.is_absolute() {
        return Err(CoreError::Message(
            "Browser reported a non-absolute download path".into(),
        ));
    }
    for _ in 0..20 {
        if let Ok(metadata) = tokio::fs::metadata(&path).await
            && metadata.is_file()
        {
            let filename = path
                .file_name()
                .ok_or_else(|| CoreError::Message("Download has no filename".into()))?
                .to_string_lossy()
                .into_owned();
            return Ok(DownloadedFile {
                path,
                filename,
                bytes: metadata.len(),
            });
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(CoreError::Message(format!(
        "Download completed but no file exists at {}",
        path.display()
    )))
}
