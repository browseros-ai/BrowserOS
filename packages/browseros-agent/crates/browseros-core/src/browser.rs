use crate::{
    BrowserView, CoreError, NativeCapabilities, PageId, ProtocolSession, TabId, TargetBinding,
    connection::CdpConnection,
    pages::PageInfo,
    screenshot::ScreenshotFormat,
    session::{BrowserSession, BrowserSessionHooks},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

/// Rust facade with host-local binding. Each operation captures one view; reusable protocol
/// handles returned from it retain that capture rather than reading a later facade binding.
pub struct Browser {
    core: Arc<BrowserSession>,
    binding: tokio::sync::RwLock<Option<TargetBinding>>,
}

impl Browser {
    /// A facade whose native identity is unknown; gated native features stay unused.
    #[must_use]
    pub fn new(connection: Arc<dyn CdpConnection>) -> Self {
        Self::new_with_native(connection, NativeCapabilities::default())
    }

    #[must_use]
    pub fn new_with_native(connection: Arc<dyn CdpConnection>, native: NativeCapabilities) -> Self {
        Self {
            core: BrowserSession::new_with_native(
                connection,
                BrowserSessionHooks::default(),
                native,
            ),
            binding: tokio::sync::RwLock::new(None),
        }
    }

    pub async fn set_target(&self, binding: Option<TargetBinding>) {
        *self.binding.write().await = binding;
    }

    pub async fn view(&self) -> BrowserView {
        let binding = self.binding.read().await.clone();
        BrowserView::capture(self.core.clone(), binding, None).await
    }

    /// Infrastructure only: tools and task execution must use view() to capture targeting.
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
        let view = self.view().await;
        // Only a genuinely unbound view (older native) takes the global path; a failed
        // default capture must surface its error, never widen to another profile's window.
        if view.scope()?.is_none() {
            return self
                .core
                .pages
                .get_active_session_for_window(window_id)
                .await;
        }
        let result = view
            .cdp(
                "Browser.getActiveTab",
                json!({ "windowId": window_id.0 }),
                None,
            )
            .await?;
        let target_id = result["tab"]["targetId"]
            .as_str()
            .ok_or_else(|| CoreError::Message(format!("No active tab in window {window_id}")))?;
        let page = view
            .list_pages()
            .await?
            .pages
            .into_iter()
            .find(|page| page.target_id.as_str() == target_id)
            .ok_or_else(|| CoreError::Message(format!("No active tab in window {window_id}")))?;
        view.page_session(page.page_id).await
    }

    pub async fn get_page_session(
        &self,
        page_id: PageId,
    ) -> Result<crate::pages::PageSession, CoreError> {
        self.view().await.page_session(page_id).await
    }

    pub async fn list_pages(&self) -> Result<Vec<PageInfo>, CoreError> {
        Ok(self.view().await.list_pages().await?.pages)
    }

    pub async fn new_page(
        &self,
        url: &str,
        opts: crate::pages::NewPageOptions,
    ) -> Result<PageId, CoreError> {
        let view = self.view().await;
        let mut opts = opts;
        if view.scope()?.is_none() && opts.window_id.is_none() {
            opts.window_id = Self::resolve_visible_window_id(&view).await?;
        }
        Ok(view.new_page(url, opts).await?.page_id)
    }

    pub async fn close_page(&self, page_id: PageId) -> Result<(), CoreError> {
        self.view().await.close_page(page_id).await
    }

    pub async fn resolve_tab_ids(
        &self,
        tab_ids: &[TabId],
    ) -> Result<HashMap<TabId, PageId>, CoreError> {
        self.view().await.resolve_tab_ids(tab_ids).await
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
        let dpr_result = session
            .send::<_, EvaluateResult>(
                "Runtime.evaluate",
                json!({"expression":"window.devicePixelRatio","returnByValue":true}),
            )
            .await;
        // DPR is advisory for an unbound transport failure, but a target admission error must
        // invalidate the whole capture instead of publishing bytes from an expired capability.
        let dpr = match dpr_result {
            Err(error @ CoreError::Target(_)) => return Err(error),
            result => result
                .ok()
                .and_then(|result| result.result.value.and_then(|value| value.as_f64()))
                .unwrap_or(1.0),
        };
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
        Ok(self.view().await.page_session(page_id).await?.session)
    }

    async fn resolve_visible_window_id(
        view: &BrowserView,
    ) -> Result<Option<crate::WindowId>, CoreError> {
        let windows = view.list_windows().await?;
        let active = windows
            .iter()
            .find(|window| window.is_visible && window.is_active)
            .or_else(|| windows.iter().find(|window| window.is_visible));
        if let Some(active) = active {
            return Ok(Some(crate::WindowId(active.window_id)));
        }
        let created = view.create_window().await?;
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
