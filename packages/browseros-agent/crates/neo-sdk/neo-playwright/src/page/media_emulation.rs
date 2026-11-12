//! Page-scoped media overrides. Each call merges only supplied fields, then sends a complete
//! CDP snapshot to every renderer of the admitted page. The page owns this state across runs;
//! session attachment replays it into new OOPIFs without changing another tab's preferences.

use std::collections::HashSet;
use std::mem;
use std::sync::Arc;

use browseros_cdp::CdpError;
use browseros_core::{CoreError, ProtocolSession};
use serde::Deserialize;
use serde_json::json;
use ts_rs::TS;

use crate::wire::{Nothing, options};
use tokio::runtime::Handle;
use tokio::sync::OwnedMutexGuard;

use crate::op_scope::OpScope;
use crate::page::{OpPage, frame_tree};
use crate::target::PageRef;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// The last committed settings for a tab. None means Neo has never emulated this tab.
#[derive(Debug, Default)]
pub(crate) struct MediaState {
    settings: Option<MediaSettings>,
}

/// CDP values, where the empty string means to use the browser's native preference.
#[derive(Clone, Debug)]
pub(crate) struct MediaSettings {
    media: String,
    color_scheme: String,
    reduced_motion: String,
    forced_colors: String,
    contrast: String,
}

impl Default for MediaSettings {
    fn default() -> Self {
        Self {
            media: String::new(),
            color_scheme: "light".to_owned(),
            reduced_motion: "no-preference".to_owned(),
            forced_colors: "none".to_owned(),
            contrast: "no-preference".to_owned(),
        }
    }
}

/// Nulls are translated to upstream's `no-override` by the client. Omitted fields stay None.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(optional_fields)]
pub(crate) struct MediaInput {
    page: PageRef,
    #[ts(optional, type = "\"screen\" | \"print\" | \"no-override\"")]
    media: Option<String>,
    #[ts(
        optional,
        type = "\"dark\" | \"light\" | \"no-preference\" | \"no-override\""
    )]
    color_scheme: Option<String>,
    #[ts(optional, type = "\"reduce\" | \"no-preference\" | \"no-override\"")]
    reduced_motion: Option<String>,
    #[ts(optional, type = "\"active\" | \"none\" | \"no-override\"")]
    forced_colors: Option<String>,
    #[ts(optional, type = "\"no-preference\" | \"more\" | \"no-override\"")]
    contrast: Option<String>,
}

options!(MediaInput);

impl MediaInput {
    fn validate(&self, scope: &OpScope<'_>) -> Result<(), PlaywrightError> {
        for (field, supplied, allowed) in [
            (
                "media",
                &self.media,
                &["screen", "print", "no-override"][..],
            ),
            (
                "colorScheme",
                &self.color_scheme,
                &["dark", "light", "no-preference", "no-override"][..],
            ),
            (
                "reducedMotion",
                &self.reduced_motion,
                &["reduce", "no-preference", "no-override"][..],
            ),
            (
                "forcedColors",
                &self.forced_colors,
                &["active", "none", "no-override"][..],
            ),
            (
                "contrast",
                &self.contrast,
                &["no-preference", "more", "no-override"][..],
            ),
        ] {
            if let Some(value) = supplied
                && !allowed.contains(&value.as_str())
            {
                return Err(scope.fail(
                    ErrorCode::InvalidInput,
                    &format!("{field}: expected one of ({})", allowed.join("|")),
                ));
            }
        }
        Ok(())
    }
}

/// A page-wide write tolerates a definitively closed child while the page itself stays live.
#[derive(Debug, PartialEq, Eq)]
enum RendererDelivery {
    Applied,
    ClosedChild,
}

impl MediaSettings {
    fn patched(mut self, input: MediaInput) -> Self {
        for (supplied, saved) in [
            (input.media, &mut self.media),
            (input.color_scheme, &mut self.color_scheme),
            (input.reduced_motion, &mut self.reduced_motion),
            (input.forced_colors, &mut self.forced_colors),
            (input.contrast, &mut self.contrast),
        ] {
            if let Some(value) = supplied {
                *saved = if value == "no-override" {
                    String::new()
                } else {
                    value
                };
            }
        }
        self
    }

    async fn send_in_page(
        &self,
        root: &ProtocolSession,
        session: &ProtocolSession,
    ) -> Result<RendererDelivery, CoreError> {
        match self.send(session).await {
            Ok(()) => Ok(RendererDelivery::Applied),
            Err(error) if !session.same_session(root) && child_closed(&error) => {
                Ok(RendererDelivery::ClosedChild)
            }
            Err(error) => Err(error),
        }
    }

    /// Send all features together: CDP replaces the feature list, so a partial payload would
    /// silently erase settings made by an earlier call.
    pub(crate) async fn send(&self, session: &ProtocolSession) -> Result<(), CoreError> {
        session
            .send_value(
                "Emulation.setEmulatedMedia",
                json!({
                    "media": self.media,
                    "features": [
                        { "name": "prefers-color-scheme", "value": self.color_scheme },
                        { "name": "prefers-reduced-motion", "value": self.reduced_motion },
                        { "name": "forced-colors", "value": self.forced_colors },
                        { "name": "prefers-contrast", "value": self.contrast },
                    ],
                }),
            )
            .await?;
        Ok(())
    }
}

impl MediaState {
    pub(super) fn is_emulated(&self) -> bool {
        self.settings.is_some()
    }

    /// Session setup and page updates use the same lock: a newly attached renderer must get
    /// the latest complete setting, even if attachment races an emulateMedia call.
    pub(crate) async fn replay(&self, session: &ProtocolSession) -> Result<(), CoreError> {
        if let Some(settings) = &self.settings {
            settings.send(session).await?;
        }
        Ok(())
    }
}

/// Owns a pending setting change and its renderer endpoints. Cancellation rolls back in a
/// spawned cleanup while retaining the per-page lock, so later calls cannot overtake cleanup.
#[must_use = "owns the page lock and restores a cancelled media update"]
struct MediaUpdate {
    state: Option<OwnedMutexGuard<MediaState>>,
    previous: Option<MediaSettings>,
    sessions: Vec<ProtocolSession>,
    sent: bool,
}

impl Drop for MediaUpdate {
    fn drop(&mut self) {
        if !self.sent {
            return;
        }
        let Some(mut state) = self.state.take() else {
            return;
        };
        let previous = self.previous.take();
        let sessions = mem::take(&mut self.sessions);
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        drop(runtime.spawn(async move {
            // Before the first explicit override the browser's native values must be restored,
            // rather than Playwright's default light/no-preference emulation.
            let native = MediaSettings {
                media: String::new(),
                color_scheme: String::new(),
                reduced_motion: String::new(),
                forced_colors: String::new(),
                contrast: String::new(),
            };
            let reset = previous.as_ref().unwrap_or(&native);
            for session in sessions {
                let _restored = reset.send(&session).await;
            }
            state.settings = previous;
        }));
    }
}

/// The run's door supplies cancellation and the delivery record. This leaf rechecks immediately
/// before each renderer write; settings and their cleanup live on the page across run teardown.
pub(crate) async fn emulate(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: MediaInput,
) -> Result<Nothing, PlaywrightError> {
    // Validate before page admission, so bad values never open the lazy default tab.
    input.validate(scope)?;
    let page_ref = input.page.clone();
    let page = OpPage::admit_handle(pw, scope, &page_ref).await?;
    let state = Arc::clone(page.state().media());
    let root = page.root_session();
    let tree = frame_tree::read(pw.browser(), page.core_page(), &root)
        .await
        .map_err(|error| scope.core_error(&error))?;
    pw.browser()
        .frames()
        .enable_initialization(&root.at_connection_epoch())
        .await
        .map_err(|error| scope.core_error(&error))?;
    // Discovery may initialize a previously missed paused renderer. Do that before taking
    // the page lock, which its initializer also needs. Merge the live binding set under the
    // lock afterwards so frames attached during discovery cannot miss this update.
    let guard = state.lock_owned().await;
    let next = guard.settings.clone().unwrap_or_default().patched(input);
    pw.media().remember(&root, Arc::clone(page.state().media()));
    for frame in tree.ids() {
        if let Some(session) = tree.endpoint(frame) {
            pw.media()
                .remember(session, Arc::clone(page.state().media()));
        }
    }
    let mut sessions = vec![root.at_connection_epoch()];
    let mut seen = HashSet::new();
    if let Some(id) = root.session_id() {
        seen.insert(id.clone());
    }
    for session in pw.media().sessions(page.state().media()) {
        if let Some(id) = session.session_id()
            && seen.insert(id.clone())
        {
            // Existing renderers may predate this facade. Arm their nested attachment before
            // sending the preference; only a definitively closed child is harmless here.
            if let Err(error) = pw.browser().frames().enable_initialization(&session).await {
                if child_closed(&error) {
                    pw.media().forget(&session);
                    continue;
                }
                return Err(scope.core_error(&error));
            }
            sessions.push(session);
        }
    }
    let previous = guard.settings.clone();
    let mut update = MediaUpdate {
        state: Some(guard),
        previous,
        sessions,
        sent: false,
    };
    for session in &update.sessions {
        let _sent = scope.send_effect()?;
        update.sent = true;
        match next.send_in_page(&root, session).await {
            Ok(RendererDelivery::Applied) => {}
            Ok(RendererDelivery::ClosedChild) => pw.media().forget(session),
            Err(error) => return Err(scope.core_error(&error)),
        }
    }
    if let Some(mut state) = update.state.take() {
        state.settings = Some(next);
    }
    Ok(())
}

/// Only definitive iframe closure is a no-op for a page-wide broadcast. A root failure,
/// transport loss, timeout or another protocol error still fails and restores the update.
pub(super) fn child_closed(error: &CoreError) -> bool {
    match error {
        CoreError::Cdp(CdpError::SessionGone) => true,
        CoreError::Cdp(CdpError::Protocol { code, message }) => matches!(
            CdpError::from_protocol(*code, message.clone()),
            CdpError::SessionGone
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{MediaSettings, RendererDelivery};
    use browseros_core::testing::{TestConnection, cdp_error};
    use browseros_core::{CdpConnection, ProtocolSession, SessionId};
    use serde_json::json;
    use std::error::Error;
    use std::sync::Arc;

    #[tokio::test]
    async fn page_broadcast_skips_a_closed_child_but_preserves_root_and_other_failures()
    -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([
            ("Emulation.setEmulatedMedia", json!({})),
            (
                "Emulation.setEmulatedMedia",
                cdp_error("No session with given id"),
            ),
            ("Emulation.setEmulatedMedia", json!({})),
            (
                "Emulation.setEmulatedMedia",
                cdp_error("No session with given id"),
            ),
            (
                "Emulation.setEmulatedMedia",
                cdp_error("Unexpected renderer failure"),
            ),
        ]);
        let transport: Arc<dyn CdpConnection> = connection;
        let root = ProtocolSession::for_session(Arc::clone(&transport), SessionId::from("root"));
        let child = ProtocolSession::for_session(transport, SessionId::from("child"));
        let settings = MediaSettings::default();
        assert_eq!(
            settings.send_in_page(&root, &root).await?,
            RendererDelivery::Applied
        );
        assert_eq!(
            settings.send_in_page(&root, &child).await?,
            RendererDelivery::ClosedChild
        );
        assert_eq!(
            settings.send_in_page(&root, &child).await?,
            RendererDelivery::Applied,
            "a detached sibling must not stop updating another live child"
        );
        assert!(
            settings.send_in_page(&root, &root).await.is_err(),
            "a closed root always fails the page operation"
        );
        assert!(
            settings.send_in_page(&root, &child).await.is_err(),
            "other child protocol failures must remain visible"
        );
        Ok(())
    }
}
