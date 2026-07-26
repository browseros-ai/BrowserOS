use crate::{
    CoreError, PageId, ProtocolSession, TabId,
    connection::CdpConnection,
    pages::PageInfo,
    screenshot::ScreenshotFormat,
    session::{BrowserSession, BrowserSessionHooks},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

pub struct Browser {
    core: Arc<BrowserSession>,
}

impl Browser {
    #[must_use]
    pub fn new(connection: Arc<dyn CdpConnection>) -> Self {
        Self {
            core: BrowserSession::new(connection, BrowserSessionHooks::default()),
        }
    }

    #[must_use]
    pub fn session(&self) -> Arc<BrowserSession> {
        self.core.clone()
    }

    #[must_use]
    pub fn is_cdp_connected(&self) -> bool {
        self.core.is_connected()
    }

    pub async fn get_active_page_for_window(
        &self,
        window_id: crate::WindowId,
    ) -> Result<crate::pages::PageSession, CoreError> {
        self.core
            .pages
            .get_active_session_for_window(window_id)
            .await
    }

    pub async fn get_page_session(
        &self,
        page_id: PageId,
    ) -> Result<crate::pages::PageSession, CoreError> {
        self.core.pages.get_session(page_id).await
    }

    pub async fn list_pages(&self) -> Result<Vec<PageInfo>, CoreError> {
        self.core.pages.list().await
    }

    pub async fn new_page(
        &self,
        url: &str,
        opts: crate::pages::NewPageOptions,
    ) -> Result<PageId, CoreError> {
        let mut opts = opts;
        if opts.window_id.is_none() {
            opts.window_id = self.resolve_visible_window_id().await?;
        }
        self.core.pages.new_page(url, opts).await
    }

    pub async fn close_page(&self, page_id: PageId) -> Result<(), CoreError> {
        self.core.pages.close(page_id).await
    }

    pub async fn resolve_tab_ids(
        &self,
        tab_ids: &[TabId],
    ) -> Result<HashMap<TabId, PageId>, CoreError> {
        self.core.pages.resolve_tab_ids(tab_ids).await
    }

    pub async fn screenshot(
        &self,
        page_id: PageId,
        format: ScreenshotFormat,
        quality: Option<i64>,
        full_page: bool,
    ) -> Result<BrowserScreenshot, CoreError> {
        let session = self.resolve_session(page_id).await?;
        let mut params = serde_json::Map::new();
        params.insert(
            "format".to_string(),
            Value::String(format.as_str().to_string()),
        );
        params.insert("captureBeyondViewport".to_string(), Value::Bool(full_page));
        if let Some(quality) = quality {
            params.insert("quality".to_string(), Value::from(quality));
        }
        let screenshot = session
            .send::<_, CaptureScreenshotResult>("Page.captureScreenshot", Value::Object(params))
            .await?;
        let dpr = session
            .send::<_, EvaluateResult>(
                "Runtime.evaluate",
                json!({ "expression": "window.devicePixelRatio", "returnByValue": true }),
            )
            .await
            .ok()
            .and_then(|result| result.result.value.and_then(|value| value.as_f64()))
            .unwrap_or(1.0);
        Ok(BrowserScreenshot {
            data: screenshot.data,
            mime_type: format!("image/{}", format.as_str()),
            device_pixel_ratio: dpr,
        })
    }

    pub async fn evaluate(
        &self,
        page_id: PageId,
        expression: &str,
    ) -> Result<EvaluateOutput, CoreError> {
        let session = self.resolve_session(page_id).await?;
        let result: EvaluateResult = session
            .send(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
            )
            .await?;
        if let Some(exception) = result.exception_details {
            return Ok(EvaluateOutput {
                value: None,
                error: Some(
                    exception
                        .exception
                        .and_then(|exception| exception.description)
                        .unwrap_or(exception.text),
                ),
                description: None,
            });
        }
        Ok(EvaluateOutput {
            value: result.result.value,
            error: None,
            description: result.result.description,
        })
    }

    async fn resolve_session(&self, page_id: PageId) -> Result<ProtocolSession, CoreError> {
        Ok(self.core.pages.get_session(page_id).await?.session)
    }

    async fn resolve_visible_window_id(&self) -> Result<Option<crate::WindowId>, CoreError> {
        let windows = self.core.windows.list().await?;
        let active = windows
            .iter()
            .find(|window| window.is_visible && window.is_active)
            .or_else(|| windows.iter().find(|window| window.is_visible));
        if let Some(active) = active {
            return Ok(Some(crate::WindowId(active.window_id)));
        }
        let created = self.core.windows.create().await?;
        Ok(Some(crate::WindowId(created.window_id)))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrowserScreenshot {
    pub data: String,
    pub mime_type: String,
    pub device_pixel_ratio: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvaluateOutput {
    pub value: Option<Value>,
    pub error: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CaptureScreenshotResult {
    data: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvaluateResult {
    result: RemoteObject,
    exception_details: Option<ExceptionDetails>,
}

#[derive(Debug, Deserialize)]
struct RemoteObject {
    value: Option<Value>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExceptionDetails {
    text: String,
    exception: Option<RemoteObject>,
}
