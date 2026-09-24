//! Playwright selector resolution shared by actions, waits, and assertions.
//! P1 supplies the isolated-world engine behind these stable interfaces.

mod frames;
mod world;

use crate::{
    CoreError, FrameId, PageId, ProtocolSession, frames::FrameRegistry, pages::PageManager,
};
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Owns locator resolution for one browser session and its frame registry.
pub struct LocatorEngine {
    _pages: Arc<PageManager>,
    _frames: Arc<FrameRegistry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementState {
    Visible,
    Hidden,
    Enabled,
    Disabled,
    Editable,
    Checked,
    Unchecked,
    Stable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strictness {
    Strict,
    First,
}

/// A resolved node retains its frame session so input can address OOPIFs.
pub struct Resolved {
    pub session: ProtocolSession,
    pub backend_node_id: i64,
    pub object_id: String,
    pub frame_id: Option<FrameId>,
}

/// Shared wall-clock and cancellation budget for a locator operation.
pub struct Deadline {
    pub at: tokio::time::Instant,
    pub cancel: CancellationToken,
}

pub struct ExpectOutcome {
    pub matches: bool,
    pub received: Option<Value>,
    pub timed_out: bool,
    pub log: Vec<String>,
}

impl LocatorEngine {
    #[must_use]
    pub fn new(pages: Arc<PageManager>, frames: Arc<FrameRegistry>) -> Self {
        Self {
            _pages: pages,
            _frames: frames,
        }
    }

    /// Resolves a Playwright selector under the caller's strictness and wait budget.
    pub async fn resolve(
        &self,
        _page: PageId,
        _selector: &str,
        _strict: Strictness,
        _wait: Option<ElementState>,
        _dl: &Deadline,
    ) -> Result<Resolved, CoreError> {
        Err(CoreError::Message(
            "not implemented yet: locator.resolve".to_string(),
        ))
    }
    pub async fn resolve_all(
        &self,
        _page: PageId,
        _selector: &str,
    ) -> Result<Vec<Resolved>, CoreError> {
        Err(CoreError::Message(
            "not implemented yet: locator.resolve_all".to_string(),
        ))
    }
    pub async fn count(&self, _page: PageId, _selector: &str) -> Result<usize, CoreError> {
        Err(CoreError::Message(
            "not implemented yet: locator.count".to_string(),
        ))
    }
    pub async fn wait_for_states(
        &self,
        _r: &Resolved,
        _states: &[ElementState],
        _dl: &Deadline,
    ) -> Result<(), CoreError> {
        Err(CoreError::Message(
            "not implemented yet: locator.wait_for_states".to_string(),
        ))
    }
    /// Evaluates an injected `to.*` expression with Playwright matcher options.
    pub async fn expect(
        &self,
        _page: PageId,
        _selector: Option<&str>,
        _expression: &str,
        _options: Value,
        _dl: &Deadline,
    ) -> Result<ExpectOutcome, CoreError> {
        Err(CoreError::Message(
            "not implemented yet: locator.expect".to_string(),
        ))
    }
    /// Runs a page function on the resolved element in its main world.
    pub async fn call_on(
        &self,
        _r: &Resolved,
        _fn_source: &str,
        _arg: Value,
    ) -> Result<Value, CoreError> {
        Err(CoreError::Message(
            "not implemented yet: locator.call_on".to_string(),
        ))
    }
    /// Returns tag, type, and autocomplete for redaction and error reporting.
    pub async fn describe(&self, _r: &Resolved) -> Result<Value, CoreError> {
        Err(CoreError::Message(
            "not implemented yet: locator.describe".to_string(),
        ))
    }
}
