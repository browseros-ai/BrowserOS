use crate::{CoreError, ProtocolSession, WindowId, connection::CdpConnection, timeouts};
use browseros_cdp::browser;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::time::sleep;

pub type WindowInfo = browser::WindowInfo;

#[derive(Debug, Clone, PartialEq)]
pub struct SetWindowVisibilityResult {
    pub window: WindowInfo,
    pub replaced: bool,
    pub previous_window_id: WindowId,
    pub new_window_id: WindowId,
}

pub struct WindowManager {
    cdp: Arc<dyn CdpConnection>,
}

impl WindowManager {
    #[must_use]
    pub fn new(cdp: Arc<dyn CdpConnection>) -> Self {
        Self { cdp }
    }

    pub async fn list(&self) -> Result<Vec<WindowInfo>, CoreError> {
        self.ensure_connected().await?;
        let root = ProtocolSession::root(self.cdp.clone());
        let result: browser::GetWindowsResult = root.send("Browser.getWindows", json!({})).await?;
        Ok(result.windows)
    }

    pub async fn create(&self, hidden: bool) -> Result<WindowInfo, CoreError> {
        self.ensure_connected().await?;
        let root = ProtocolSession::root(self.cdp.clone());
        let result: browser::CreateWindowResult = root
            .send("Browser.createWindow", json!({ "hidden": hidden }))
            .await?;
        Ok(result.window)
    }

    pub async fn close(&self, window_id: WindowId) -> Result<(), CoreError> {
        self.ensure_connected().await?;
        let root = ProtocolSession::root(self.cdp.clone());
        let _: Value = root
            .send("Browser.closeWindow", json!({ "windowId": window_id.0 }))
            .await?;
        Ok(())
    }

    pub async fn activate(&self, window_id: WindowId) -> Result<(), CoreError> {
        self.ensure_connected().await?;
        let root = ProtocolSession::root(self.cdp.clone());
        let _: Value = root
            .send("Browser.activateWindow", json!({ "windowId": window_id.0 }))
            .await?;
        Ok(())
    }

    pub async fn set_visibility(
        &self,
        window_id: WindowId,
        visible: bool,
        activate: Option<bool>,
    ) -> Result<SetWindowVisibilityResult, CoreError> {
        self.ensure_connected().await?;
        let root = ProtocolSession::root(self.cdp.clone());
        let result: browser::SetWindowVisibilityResult = root
            .send(
                "Browser.setWindowVisibility",
                set_visibility_params(window_id, visible, activate),
            )
            .await?;
        let new_window_id = WindowId(result.window.window_id);
        Ok(SetWindowVisibilityResult {
            window: result.window,
            replaced: result.replaced,
            previous_window_id: WindowId(result.previous_window_id),
            new_window_id,
        })
    }

    async fn ensure_connected(&self) -> Result<(), CoreError> {
        if self.cdp.is_connected() {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + timeouts::WAIT_FOR_CONNECTION_TIMEOUT;
        while !self.cdp.is_connected() && tokio::time::Instant::now() < deadline {
            sleep(timeouts::WAIT_FOR_CONNECTION_POLL).await;
        }
        if self.cdp.is_connected() {
            Ok(())
        } else {
            Err(CoreError::Cdp(browseros_cdp::CdpError::NotConnected))
        }
    }
}

fn set_visibility_params(window_id: WindowId, visible: bool, activate: Option<bool>) -> Value {
    let mut params = serde_json::Map::new();
    params.insert("windowId".to_string(), Value::from(window_id.0));
    params.insert("visible".to_string(), Value::Bool(visible));
    if let Some(activate) = activate {
        params.insert("activate".to_string(), Value::Bool(activate));
    }
    Value::Object(params)
}

#[cfg(test)]
mod tests {
    use super::set_visibility_params;
    use crate::WindowId;

    #[test]
    fn set_visibility_omits_absent_activate() {
        let params = set_visibility_params(WindowId(7), false, None);

        assert_eq!(params.get("activate"), None);
        assert_eq!(
            params.get("windowId").and_then(|value| value.as_i64()),
            Some(7)
        );
        assert_eq!(
            params.get("visible").and_then(|value| value.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn set_visibility_sends_explicit_activate() {
        let params = set_visibility_params(WindowId(7), true, Some(true));

        assert_eq!(
            params.get("activate").and_then(|value| value.as_bool()),
            Some(true)
        );
    }
}
