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
}

fn not_implemented() -> PageError {
    PageError::Browser("not implemented".into())
}
