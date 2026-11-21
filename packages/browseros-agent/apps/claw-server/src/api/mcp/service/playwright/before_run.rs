//! Restores the desktop window precondition the SDK's unbound page creation needs.
//! Window placement belongs to the host; the SDK discovers the reopened window itself.

use crate::{AppState, services::neo::EngineState};
use browseros_core::{
    BrowserSession, BrowserView, CoreError, DEFAULT_PROFILE_DIRECTORY, NativeFeature,
    pages::NewPageOptions, windows::WindowInfo,
};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Prepare an agent run without replacing the SDK's own unavailable/error reply.
/// The caller keeps this future inside its cancellation boundary, before forwarding.
pub(super) async fn ensure_window(state: &AppState) -> Option<String> {
    if !matches!(state.neo.state(), EngineState::Ready) {
        return None;
    }
    let browser = state.browser.session().await?;
    ensure_browser_window(browser)
        .await
        .unwrap_or_else(|error| {
            Some(format!(
                "Could not prepare a browser window before run: {error}"
            ))
        })
}

async fn ensure_browser_window(browser: Arc<BrowserSession>) -> Result<Option<String>, CoreError> {
    let profile_targeting = browser.native().supports(NativeFeature::ProfileTargeting);
    if has_window(&browser.windows.list().await?, profile_targeting) {
        return Ok(None);
    }

    // Only recovery is serialized. Re-read after acquiring the lock because another
    // agent run may have opened Default while this run was waiting for it.
    static OPEN_WINDOW: Mutex<()> = Mutex::const_new(());
    let _opening = OPEN_WINDOW.lock().await;
    if has_window(&browser.windows.list().await?, profile_targeting) {
        return Ok(None);
    }
    // SDK runs ignore saved target bindings. Capture Default only on recovery,
    // since capture validates profiles and windows through several CDP requests.
    let view = BrowserView::capture(browser, None, None).await;
    if let Err(error) = view.create_window().await {
        tracing::debug!(%error, "Default window creation failed; using scoped page creation");
        // This reaches the old tools' Default-window helper. Keep its blank tab
        // for the SDK to adopt as its keeper instead of closing it underneath it.
        view.new_page(
            "about:blank",
            NewPageOptions {
                background: Some(true),
                ..Default::default()
            },
        )
        .await?;
    }
    Ok(Some(if profile_targeting {
        "Opened an inactive Default browser window before run.".to_string()
    } else {
        "Opened a browser window before run.".to_string()
    }))
}

fn has_window(windows: &[WindowInfo], profile_targeting: bool) -> bool {
    // Window metadata's browserContextId is a directory label. Only creation
    // uses the live routing handle supplied by the captured Default view.
    windows.iter().any(|window| {
        window.window_type == "normal"
            && (!profile_targeting
                || window.browser_context_id.as_deref() == Some(DEFAULT_PROFILE_DIRECTORY))
    })
}

#[cfg(test)]
mod tests {
    use super::{ensure_browser_window, has_window};
    use browseros_core::{
        BrowserSession, BrowserSessionHooks, NativeCapabilities, testing::TestConnection,
        windows::WindowInfo,
    };
    use serde_json::json;

    #[test]
    fn only_normal_default_windows_satisfy_profile_targeting() {
        let window = |kind: &str, directory: Option<&str>| WindowInfo {
            window_type: kind.to_string(),
            browser_context_id: directory.map(str::to_string),
            ..Default::default()
        };
        assert!(!has_window(&[], true));
        assert!(!has_window(&[window("popup", Some("Default"))], true));
        assert!(!has_window(&[window("normal", Some("Profile 1"))], true));
        assert!(!has_window(&[window("normal", None)], true));
        assert!(has_window(&[window("normal", Some("Default"))], true));
        assert!(has_window(&[window("normal", None)], false));
    }

    #[tokio::test]
    async fn existing_default_window_costs_only_one_inventory_read() -> anyhow::Result<()> {
        let connection = TestConnection::new([(
            "Browser.getWindows",
            json!({"windows": [{
                "windowId": 1, "windowType": "normal", "bounds": {},
                "isActive": false, "isVisible": true, "tabCount": 1,
                "browserContextId": "Default"
            }]}),
        )]);
        let browser = BrowserSession::new_with_native(
            connection.clone(),
            BrowserSessionHooks::default(),
            NativeCapabilities::from_browseros_version(Some("0.51.1.3")),
        );
        assert!(ensure_browser_window(browser).await?.is_none());
        let calls = connection.calls()?;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "Browser.getWindows");
        Ok(())
    }
}
