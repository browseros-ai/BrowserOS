//! A prepared Playwright resize: admission and locking happen before the caller's last
//! permission check. Only `apply` writes CDP; it publishes configuration after acknowledgement.

use super::device::Fixed;
use super::{ViewportRequest, Viewports, device};
use crate::{BrowserView, CoreError, PageId, SessionId};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OwnedRwLockWriteGuard;

/// Holds one tab between resize preparation and the caller's final permission check.
/// On failure or cancellation, its previous configuration stays authoritative and a bounded
/// cleanup restores CDP while retaining the lock, before another resize or fit can write.
#[must_use = "a prepared resize changes nothing until apply is called"]
pub struct ViewportChange {
    view: BrowserView,
    viewports: Arc<Viewports>,
    page: PageId,
    session: SessionId,
    request: ViewportRequest,
    fixed: Fixed,
    commands: Vec<(&'static str, Value)>,
    restore: Vec<(&'static str, Value)>,
    previous_default: bool,
    held: Option<OwnedRwLockWriteGuard<()>>,
    /// Armed only after the caller authorizes a send, and disarmed after publication.
    restore_generation: Option<u64>,
}

impl Viewports {
    pub(crate) async fn prepare_change(
        self: &Arc<Self>,
        view: &BrowserView,
        page: PageId,
        request: ViewportRequest,
    ) -> Result<ViewportChange, CoreError> {
        if !request.playwright {
            return Err(CoreError::Message(
                "a Playwright viewport request is required".to_owned(),
            ));
        }
        self.forget_closed(view).await;
        view.page(page.clone()).await?;
        let held = self.lock_page(view, &page).await;
        let admitted = view.page_session(page.clone()).await?;
        self.repair_pending_locked(view, &page, &admitted).await?;
        let previous = self.tab(&page);
        let previous_default = previous.is_none();
        let restore = match previous {
            Some(previous) => {
                let version = if matches!(previous.fixed, Fixed::Playwright { .. }) {
                    String::new()
                } else {
                    self.chrome_version(view).await?
                };
                let mut commands = previous.fixed.commands(&version);
                commands.extend(previous.fixed.pin());
                commands
            }
            None => device::clear_commands(),
        };
        let fixed = request
            .fixed()
            .ok_or_else(|| CoreError::Message("a fixed viewport is required".to_owned()))?;
        Ok(ViewportChange {
            view: view.clone(),
            viewports: Arc::clone(self),
            page,
            session: admitted.session_id.clone(),
            request,
            fixed,
            commands: fixed.commands(""),
            restore,
            previous_default,
            held: Some(held),
            restore_generation: None,
        })
    }
}

impl ViewportChange {
    /// Sends the prepared metrics override. The caller must authorize this action immediately
    /// before awaiting it; preparation has already completed every potentially blocking read.
    /// No renderer script is needed to acknowledge a Playwright viewport change.
    pub async fn apply(mut self) -> Result<(), CoreError> {
        self.restore_generation = Some(self.viewports.invalidate(&self.page));
        if self.previous_default {
            self.viewports
                .pending_defaults
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(self.page.clone());
        }
        for (method, params) in std::mem::take(&mut self.commands) {
            self.view
                .send_at_capture(method, params, Some(&self.session))
                .await?;
        }
        // No await between committing the requested size and disarming rollback. Failed
        // calls keep the previous host state even though Playwright's client keeps its try.
        let generation = self.viewports.begin(&self.page, self.request, self.fixed);
        self.viewports
            .applied(&self.page, &self.session, generation);
        self.restore_generation = None;
        Ok(())
    }
}

impl Drop for ViewportChange {
    fn drop(&mut self) {
        let Some(generation) = self.restore_generation else {
            return;
        };
        let view = self.view.clone();
        let viewports = Arc::clone(&self.viewports);
        let page = self.page.clone();
        let session = self.session.clone();
        let commands = std::mem::take(&mut self.restore);
        let held = self.held.take();
        // A run can be cancelled after CDP accepted a message. Keep ownership of the tab in
        // this cleanup task so a later owner fit cannot be overwritten by the old rollback.
        tokio::spawn(async move {
            let _held = held;
            let restore = async {
                for (method, params) in commands {
                    view.send_at_capture(method, params, Some(&session)).await?;
                }
                Ok::<(), CoreError>(())
            };
            match tokio::time::timeout(Duration::from_secs(5), restore).await {
                Ok(Ok(())) => {
                    viewports.reset_complete(&page);
                    viewports.applied(&page, &session, generation);
                }
                Ok(Err(error)) => tracing::debug!(%error, "viewport rollback remains pending"),
                Err(_) => tracing::debug!(
                    "viewport rollback timed out; the previous mode remains pending"
                ),
            }
        });
    }
}
