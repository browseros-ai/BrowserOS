//! Bound event streams expose only admitted page endpoints and download GUIDs whose initiating
//! frame belongs to the captured scope. A relay exits as soon as its last subscriber drops.
use super::*;
use std::collections::HashSet;

impl BrowserView {
    pub(super) fn scoped_events(&self) -> broadcast::Receiver<CdpEvent> {
        // Only a genuinely unbound view (older native, host infrastructure) gets the global
        // stream. A capture that failed, including an unusable default, relays nothing.
        if matches!(self.capture, Ok(None)) {
            return self.session.cdp_events();
        }
        let (sender, receiver) = broadcast::channel(128);
        let mut source = self.session.cdp_events();
        let view = self.clone();
        tokio::spawn(async move {
            let mut downloads = HashSet::new();
            loop {
                let event = tokio::select! {
                    _ = sender.closed() => break,
                    event = source.recv() => match event {
                        Ok(event) => event,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    },
                };
                let admitted = tokio::select! {
                    _ = sender.closed() => break,
                    admitted = view.admit_event(&event,&mut downloads) => admitted,
                };
                match admitted {
                    Ok(true) => {
                        if sender.send(event).is_err() {
                            break;
                        }
                    }
                    Ok(false) => {}
                    Err(_) => break,
                }
            }
        });
        receiver
    }
    async fn admit_event(
        &self,
        event: &CdpEvent,
        downloads: &mut HashSet<String>,
    ) -> Result<bool, CoreError> {
        self.validate_live().await?;
        if let Some(session) = &event.session_id {
            let Some(page) = self
                .session
                .pages
                .page_for_session(&SessionId::from(session.as_str()))
                .await
            else {
                return Ok(false);
            };
            return Ok(self.page(page.page_id).await.is_ok());
        }
        match event.method.as_str() {
            "Browser.downloadWillBegin" => {
                let Some(frame) = event.params["frameId"].as_str() else {
                    return Ok(false);
                };
                for page in self.list_pages().await?.pages {
                    let session = self.page_session(page.page_id).await?;
                    let tree = session
                        .session
                        .send_value("Page.getFrameTree", json!({}))
                        .await?;
                    if contains_frame(&tree["frameTree"], frame)
                        && let Some(guid) = event.params["guid"].as_str()
                    {
                        downloads.insert(guid.into());
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            "Browser.downloadProgress" => Ok(event.params["guid"]
                .as_str()
                .is_some_and(|guid| downloads.contains(guid))),
            _ => Ok(false),
        }
    }
}
