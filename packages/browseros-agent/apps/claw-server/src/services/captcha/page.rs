use std::future::Future;

use browseros_core::{BrowserView, PageId};
use serde_json::Value;

use super::types::{DocumentId, FrameRef, PageError, Rect};

pub trait CaptchaPage: Send + Sync {
    fn eval_main(&self, expression: &str) -> impl Future<Output = Result<Value, PageError>> + Send;
    fn frames(&self) -> impl Future<Output = Result<Vec<FrameRef>, PageError>> + Send;
    fn eval_in_frame(
        &self,
        frame: &FrameRef,
        expression: &str,
    ) -> impl Future<Output = Result<Value, PageError>> + Send;
    fn click_at(&self, x: f64, y: f64) -> impl Future<Output = Result<(), PageError>> + Send;
    fn screenshot_clip(&self, rect: Rect)
    -> impl Future<Output = Result<String, PageError>> + Send;
    fn document(&self) -> impl Future<Output = Result<DocumentId, PageError>> + Send;
    /// Viewport rect (top-level CSS px) of the first visible element matching a CSS selector
    /// inside `frame`, searching through closed shadow roots (CDP DOM.getDocument pierce).
    /// None when nothing visible matches.
    fn query_rect_in_frame(
        &self,
        frame: &FrameRef,
        selector: &str,
    ) -> impl Future<Output = Result<Option<Rect>, PageError>> + Send;
}

pub struct LivePage {
    pub view: BrowserView,
    pub page: PageId,
}

impl LivePage {
    pub fn new(view: BrowserView, page: PageId) -> Self {
        Self { view, page }
    }
}

impl CaptchaPage for LivePage {
    async fn eval_main(&self, _expression: &str) -> Result<Value, PageError> {
        Err(not_implemented())
    }
    async fn frames(&self) -> Result<Vec<FrameRef>, PageError> {
        Err(not_implemented())
    }
    async fn eval_in_frame(
        &self,
        _frame: &FrameRef,
        _expression: &str,
    ) -> Result<Value, PageError> {
        Err(not_implemented())
    }
    async fn click_at(&self, _x: f64, _y: f64) -> Result<(), PageError> {
        Err(not_implemented())
    }
    async fn screenshot_clip(&self, _rect: Rect) -> Result<String, PageError> {
        Err(not_implemented())
    }
    async fn document(&self) -> Result<DocumentId, PageError> {
        Err(not_implemented())
    }
    async fn query_rect_in_frame(
        &self,
        _frame: &FrameRef,
        _selector: &str,
    ) -> Result<Option<Rect>, PageError> {
        Err(not_implemented())
    }
}

fn not_implemented() -> PageError {
    PageError::Browser("not implemented".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use browseros_cdp::{CdpError, CdpEvent};
    use browseros_core::{BrowserSession, CdpConnection, SessionId};
    use futures_util::future::BoxFuture;
    use std::sync::Arc;
    use tokio::sync::broadcast;

    struct Disconnected;

    impl CdpConnection for Disconnected {
        fn send<'a>(
            &'a self,
            _method: &'a str,
            _params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async { Err(CdpError::NotConnected) })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Err(CdpError::NotConnected) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            broadcast::channel(1).1
        }

        fn is_connected(&self) -> bool {
            false
        }

        fn connection_epoch(&self) -> u64 {
            0
        }
    }

    #[tokio::test]
    async fn tests_that_shadow_root_query_returns_the_stub_error() {
        let session = BrowserSession::new(Arc::new(Disconnected), Default::default());
        let view = BrowserView::unbound(session, None);
        let page = LivePage::new(view, PageId(42));
        let frame = FrameRef {
            frame_id: "widget".into(),
            url: "https://example.test/widget".into(),
            rect: None,
        };
        assert_eq!(
            page.query_rect_in_frame(&frame, "input[type=checkbox]")
                .await,
            Err(PageError::Browser("not implemented".into()))
        );
    }
}
