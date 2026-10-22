use crate::{
    CoreError, PageId, ProtocolSession,
    page_signals::PageSignals,
    pages::{PageManager, PageSession},
    target::page_access::PageAccess,
    timeouts,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::time::{sleep, timeout};

/// Navigation retains the admitted page incarnation through load waits; it cannot reacquire a
/// different context or page when a task rebinds or the browser reconnects.
pub struct Navigation {
    pages: Arc<PageManager>,
    page_signals: Arc<PageSignals>,
    page_id: PageId,
    access: Option<Arc<PageAccess>>,
}

impl Navigation {
    #[must_use]
    pub fn new(pages: Arc<PageManager>, page_signals: Arc<PageSignals>, page_id: PageId) -> Self {
        Self {
            pages,
            page_signals,
            page_id,
            access: None,
        }
    }

    pub(crate) fn with_access(mut self, access: Arc<PageAccess>) -> Self {
        self.access = Some(access);
        self
    }
    async fn page_session(&self) -> Result<PageSession, CoreError> {
        match &self.access {
            Some(access) => access.session().await,
            None => self.pages.get_session(self.page_id.clone()).await,
        }
    }

    pub async fn goto(&self, url: &str) -> Result<(), CoreError> {
        if let Some(access) = &self.access {
            access.fresh().await?;
        }
        self.throw_if_dialog_open()?;
        let page = self.page_session().await?;
        let _: Value = page
            .session
            .send("Page.navigate", json!({ "url": url }))
            .await?;
        wait_for_load(
            &page.session,
            &self.page_signals,
            &self.page_id,
            self.access.as_deref(),
        )
        .await
    }

    pub async fn reload(&self) -> Result<(), CoreError> {
        if let Some(access) = &self.access {
            access.fresh().await?;
        }
        self.throw_if_dialog_open()?;
        let page = self.page_session().await?;
        let _: Value = page.session.send("Page.reload", json!({})).await?;
        wait_for_load(
            &page.session,
            &self.page_signals,
            &self.page_id,
            self.access.as_deref(),
        )
        .await
    }

    pub async fn back(&self) -> Result<(), CoreError> {
        self.history("back").await
    }

    pub async fn forward(&self) -> Result<(), CoreError> {
        self.history("forward").await
    }

    async fn history(&self, direction: &str) -> Result<(), CoreError> {
        if let Some(access) = &self.access {
            access.fresh().await?;
        }
        self.throw_if_dialog_open()?;
        let page = self.page_session().await?;
        let _: Value = page
            .session
            .send(
                "Runtime.evaluate",
                json!({ "expression": format!("history.{direction}()"), "awaitPromise": true }),
            )
            .await?;
        wait_for_load(
            &page.session,
            &self.page_signals,
            &self.page_id,
            self.access.as_deref(),
        )
        .await
    }

    fn throw_if_dialog_open(&self) -> Result<(), CoreError> {
        if let Some(message) = self.page_signals.pending_dialog_line(&self.page_id) {
            Err(CoreError::Message(message))
        } else {
            Ok(())
        }
    }
}

/// Waits for a navigation to arrive somewhere worth acting on.
///
/// This used to wait for `document.readyState` to reach `complete`, which is
/// the load event and so waits on every ad iframe, tracking pixel and lazy
/// image the destination carries. Measured on this surface, that cost 3077ms
/// and 3410ms on two ordinary navigations whose useful work then took under
/// 10ms. What a caller actually needs is the content having stopped changing,
/// and `complete` remains one of the two ways to satisfy that.
///
/// Everything else here is unchanged on purpose: the dialog check still runs
/// each time round, a target error still propagates rather than being retried,
/// and `fresh()` still admits before each shared read.
async fn wait_for_load(
    session: &ProtocolSession,
    page_signals: &PageSignals,
    page_id: &PageId,
    access: Option<&PageAccess>,
) -> Result<(), CoreError> {
    sleep(timeouts::WAIT_FOR_CONNECTION_POLL).await;
    let deadline = tokio::time::Instant::now() + timeouts::ARRIVAL_READY_BUDGET;
    let mut previous: Option<u64> = None;
    while tokio::time::Instant::now() < deadline {
        // The sleep is an async handoff. Admit before reading the shared dialog cache, which
        // may now belong to a tab dragged out of this capture even before the next CDP send.
        if let Some(access) = access {
            access.fresh().await?;
        }
        if let Some(message) = page_signals.pending_dialog_line(page_id) {
            return Err(CoreError::Message(message));
        }
        let result = timeout(
            timeouts::ACTION_SETTLE_CDP_CALL_TIMEOUT,
            crate::settle::read_readiness(session),
        )
        .await;
        if let Ok(Err(error @ CoreError::Target(_))) = result {
            return Err(error);
        }
        // Navigation can tear down the execution context; errors and timeouts mean not ready yet, so keep polling.
        if let Ok(Ok(reading)) = result {
            if !reading.installed && previous == Some(reading.count) && reading.settled_enough() {
                break;
            }
            previous = Some(reading.count);
        }
        sleep(timeouts::ACTION_SETTLE_DOM_QUIET).await;
    }
    if let Some(access) = access {
        access.fresh().await?;
    }
    Ok(())
}
