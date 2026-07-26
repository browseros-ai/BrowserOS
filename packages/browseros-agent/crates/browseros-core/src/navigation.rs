use crate::{
    CoreError, PageId, ProtocolSession, page_signals::PageSignals, pages::PageManager, timeouts,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::time::{sleep, timeout};

pub struct Navigation {
    pages: Arc<PageManager>,
    page_signals: Arc<PageSignals>,
    page_id: PageId,
}

impl Navigation {
    #[must_use]
    pub fn new(pages: Arc<PageManager>, page_signals: Arc<PageSignals>, page_id: PageId) -> Self {
        Self {
            pages,
            page_signals,
            page_id,
        }
    }

    pub async fn goto(&self, url: &str) -> Result<(), CoreError> {
        self.throw_if_dialog_open()?;
        let page = self.pages.get_session(self.page_id.clone()).await?;
        let _: Value = page
            .session
            .send("Page.navigate", json!({ "url": url }))
            .await?;
        wait_for_load(&page.session, &self.page_signals, &self.page_id).await
    }

    pub async fn reload(&self) -> Result<(), CoreError> {
        self.throw_if_dialog_open()?;
        let page = self.pages.get_session(self.page_id.clone()).await?;
        let _: Value = page.session.send("Page.reload", json!({})).await?;
        wait_for_load(&page.session, &self.page_signals, &self.page_id).await
    }

    pub async fn back(&self) -> Result<(), CoreError> {
        self.history("back").await
    }

    pub async fn forward(&self) -> Result<(), CoreError> {
        self.history("forward").await
    }

    async fn history(&self, direction: &str) -> Result<(), CoreError> {
        self.throw_if_dialog_open()?;
        let page = self.pages.get_session(self.page_id.clone()).await?;
        let _: Value = page
            .session
            .send(
                "Runtime.evaluate",
                json!({ "expression": format!("history.{direction}()"), "awaitPromise": true }),
            )
            .await?;
        wait_for_load(&page.session, &self.page_signals, &self.page_id).await
    }

    fn throw_if_dialog_open(&self) -> Result<(), CoreError> {
        if let Some(message) = self.page_signals.pending_dialog_line(&self.page_id) {
            Err(CoreError::Message(message))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Deserialize)]
struct EvaluateResult {
    result: RemoteObject,
}

#[derive(Debug, Deserialize)]
struct RemoteObject {
    value: Option<Value>,
}

async fn wait_for_load(
    session: &ProtocolSession,
    page_signals: &PageSignals,
    page_id: &PageId,
) -> Result<(), CoreError> {
    sleep(timeouts::WAIT_FOR_CONNECTION_POLL).await;
    let deadline = tokio::time::Instant::now() + timeouts::WAIT_FOR_LOAD_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        if let Some(message) = page_signals.pending_dialog_line(page_id) {
            return Err(CoreError::Message(message));
        }
        let result = timeout(
            timeouts::ACTION_SETTLE_CDP_CALL_TIMEOUT,
            session.send::<_, EvaluateResult>(
                "Runtime.evaluate",
                json!({ "expression": "document.readyState", "returnByValue": true }),
            ),
        )
        .await;
        // Navigation can tear down the execution context; errors and timeouts mean not ready yet, so keep polling.
        if let Ok(Ok(result)) = result
            && result.result.value.as_ref().and_then(Value::as_str) == Some("complete")
        {
            return Ok(());
        }
        sleep(timeouts::WAIT_FOR_LOAD_POLL).await;
    }
    Ok(())
}
