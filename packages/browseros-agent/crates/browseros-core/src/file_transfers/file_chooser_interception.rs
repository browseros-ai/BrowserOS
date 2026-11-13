//! Shared ownership of a borrowed tab's native chooser interception. Neo subscriptions and
//! core uploads use the same leases, so only the last owner restores the native picker.

use std::fmt;
use std::sync::Arc;
use tokio::runtime::Handle;

use super::TransferState;
use crate::{CoreError, SessionId};

/// Keeps file chooser interception active for a page and its remote descendants. The last
/// lease restores a borrowed tab; interception already owned by an agent tab stays enabled.
pub struct FileChooserInterception {
    state: Arc<TransferState>,
    epoch: u64,
    session: Option<SessionId>,
}

impl fmt::Debug for FileChooserInterception {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileChooserInterception")
            .finish_non_exhaustive()
    }
}

impl FileChooserInterception {
    pub(super) fn new(state: Arc<TransferState>, epoch: u64, session: SessionId) -> Self {
        Self {
            state,
            epoch,
            session: Some(session),
        }
    }

    /// Releases this owner's interest and awaits restoration when it was the last owner.
    pub async fn release(&mut self) -> Result<(), CoreError> {
        if let Some(session) = self.session.as_ref() {
            self.state.release_interception(self.epoch, session).await?;
            self.session = None;
        }
        Ok(())
    }
}

impl Drop for FileChooserInterception {
    fn drop(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        let state = Arc::clone(&self.state);
        let epoch = self.epoch;
        // Cancellation cannot skip cleanup. Acquisition and release share the state's lease
        // lock, so this old release cannot disable a newer owner's interception.
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        drop(runtime.spawn(async move {
            if let Err(error) = state.release_interception(epoch, &session).await {
                tracing::warn!(%error, "could not restore file chooser interception");
            }
        }));
    }
}
