use crate::{CoreError, ProtocolSession, WindowId, connection::CdpConnection, timeouts};
use browseros_cdp::browser;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::time::sleep;

pub type WindowInfo = browser::WindowInfo;

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

    pub async fn create(&self) -> Result<WindowInfo, CoreError> {
        self.ensure_connected().await?;
        let root = ProtocolSession::root(self.cdp.clone());
        let result: browser::CreateWindowResult =
            root.send("Browser.createWindow", json!({})).await?;
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

#[cfg(test)]
mod tests {
    use super::WindowManager;
    use crate::test_support::TestConnection;
    use serde_json::json;
    use std::error::Error;

    #[tokio::test]
    async fn create_omits_hidden_option() -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([(
            "Browser.createWindow",
            json!({
                "window": {
                    "windowId": 7,
                    "windowType": "normal",
                    "bounds": {},
                    "isActive": true,
                    "isVisible": true,
                    "tabCount": 1
                }
            }),
        )]);
        let window = WindowManager::new(connection.clone()).create().await?;

        assert_eq!(window.window_id, 7);
        assert_eq!(connection.calls()?[0].params, json!({}));
        Ok(())
    }
}
