mod fill;
pub mod geometry;
pub mod keyboard;
pub mod mouse;

use crate::{
    CoreError, CoveredElementTarget, PageId, ProtocolSession, Ref, observer::Observer,
    pages::PageManager, snapshot::RefEntry,
};
use geometry::{
    call_on_element, click_blocker_at_point, focus_element_js, get_element_center, js_click,
    scroll_into_view,
};
use mouse::{MouseButton, dispatch_click, dispatch_drag, dispatch_hover, dispatch_scroll};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

pub use keyboard::{
    KeyInfo, clear_field, get_key_info, modifier_bitmask, normalize_key, press_combo, type_text,
};
pub use mouse::MouseButton as PublicMouseButton;

#[derive(Debug, Clone, Default)]
pub struct ClickOptions {
    pub button: Option<MouseButton>,
    pub click_count: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DragResult {
    pub from: Point,
    pub to: Point,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

pub struct Input {
    observer: Arc<Observer>,
    pages: Arc<PageManager>,
    page_id: PageId,
}

#[derive(Debug, Clone)]
struct InputTarget {
    backend_node_id: i64,
    error_target: CoveredElementTarget,
}

impl InputTarget {
    fn from_ref_entry(entry: &RefEntry) -> Self {
        Self {
            backend_node_id: entry.backend_node_id,
            error_target: CoveredElementTarget {
                ref_id: Some(entry.ref_id.clone()),
                role: Some(entry.role.clone()),
                name: Some(entry.name.clone()),
                backend_node_id: Some(entry.backend_node_id),
            },
        }
    }

    fn from_backend_node(backend_node_id: i64) -> Self {
        Self {
            backend_node_id,
            error_target: CoveredElementTarget {
                backend_node_id: Some(backend_node_id),
                ..CoveredElementTarget::default()
            },
        }
    }
}

impl Input {
    // Locator input keeps the resolved frame session rather than reconstructing a snapshot ref.
    pub async fn check_backend_node(
        &self,
        session: &ProtocolSession,
        backend_node_id: i64,
        checked: bool,
    ) -> Result<bool, CoreError> {
        let read_checked = || async {
            call_on_element(
                session,
                backend_node_id,
                "function(){return this.isConnected ? this.checked : null}",
                None,
            )
            .await?
            .as_bool()
            .ok_or_else(|| CoreError::Message("Element has no readable checked state.".to_string()))
        };
        let current = read_checked().await?;
        if current == checked {
            return Ok(current);
        }
        self.click_node(
            session,
            InputTarget::from_backend_node(backend_node_id),
            ClickOptions::default(),
        )
        .await?;
        // A canceled click or a controlled input can undo the toggle. Report
        // the state after handlers run, never infer success from dispatch alone.
        let current = read_checked().await?;
        if current != checked {
            return Err(CoreError::Message(
                "Click did not set the requested checked state.".to_string(),
            ));
        }
        Ok(current)
    }

    pub async fn upload_backend_node(
        &self,
        session: &ProtocolSession,
        backend_node_id: i64,
        paths: &[PathBuf],
    ) -> Result<(), CoreError> {
        let mut files = Vec::with_capacity(paths.len());
        for path in paths {
            if !path.is_absolute() {
                return Err(CoreError::Message(
                    "Upload paths must be absolute.".to_string(),
                ));
            }
            let metadata = tokio::fs::metadata(path).await.map_err(|err| {
                CoreError::Message(format!(
                    "Could not access upload file {}: {err}",
                    path.display()
                ))
            })?;
            if !metadata.is_file() {
                return Err(CoreError::Message(format!(
                    "Upload path is not a file: {}",
                    path.display()
                )));
            }
            files.push(path.to_str().ok_or_else(|| {
                CoreError::Message("Upload paths must be valid UTF-8.".to_string())
            })?);
        }
        let _: Value = session
            .send(
                "DOM.setFileInputFiles",
                json!({ "backendNodeId": backend_node_id, "files": files }),
            )
            .await?;
        Ok(())
    }

    pub async fn focus_backend_node(
        &self,
        session: &ProtocolSession,
        backend_node_id: i64,
    ) -> Result<(), CoreError> {
        let _: Value = session
            .send("DOM.focus", json!({ "backendNodeId": backend_node_id }))
            .await?;
        Ok(())
    }

    #[must_use]
    pub fn new(observer: Arc<Observer>, pages: Arc<PageManager>, page_id: PageId) -> Self {
        Self {
            observer,
            pages,
            page_id,
        }
    }

    pub async fn click(
        &self,
        ref_id: &Ref,
        opts: ClickOptions,
    ) -> Result<Option<Point>, CoreError> {
        let resolved = self.observer.resolve_ref(ref_id).await?;
        self.click_node(
            &resolved.session,
            InputTarget::from_ref_entry(&resolved.entry),
            opts,
        )
        .await
    }

    pub async fn click_backend_node(
        &self,
        backend_node_id: i64,
        opts: ClickOptions,
    ) -> Result<Option<Point>, CoreError> {
        self.with_page_session_retry(|session| {
            let opts = opts.clone();
            async move {
                self.click_node(
                    &session,
                    InputTarget::from_backend_node(backend_node_id),
                    opts,
                )
                .await
            }
        })
        .await
    }

    pub async fn click_at(&self, x: f64, y: f64, opts: ClickOptions) -> Result<(), CoreError> {
        self.with_page_session_retry(|session| {
            let button = opts.button.unwrap_or(MouseButton::Left);
            let click_count = opts.click_count.unwrap_or(1);
            async move { dispatch_click(&session, x, y, button, click_count, 0).await }
        })
        .await
    }

    pub async fn hover_at(&self, x: f64, y: f64) -> Result<(), CoreError> {
        self.with_page_session_retry(|session| async move { dispatch_hover(&session, x, y).await })
            .await
    }

    pub async fn type_at(&self, x: f64, y: f64, text: &str, clear: bool) -> Result<(), CoreError> {
        self.with_page_session_retry(|session| {
            let text = text.to_string();
            async move {
                dispatch_click(&session, x, y, MouseButton::Left, 1, 0).await?;
                if clear {
                    clear_field(&session).await?;
                }
                type_text(&session, &text).await
            }
        })
        .await
    }

    pub async fn drag_at(&self, from: Point, to: Point) -> Result<(), CoreError> {
        self.with_page_session_retry(
            |session| async move { dispatch_drag(&session, from, to).await },
        )
        .await
    }

    async fn click_node(
        &self,
        session: &ProtocolSession,
        target: InputTarget,
        opts: ClickOptions,
    ) -> Result<Option<Point>, CoreError> {
        scroll_into_view(session, target.backend_node_id).await;
        match get_element_center(session, target.backend_node_id).await {
            Ok(point) => {
                self.check_click_point(session, &target, point).await?;
                dispatch_click(
                    session,
                    point.x,
                    point.y,
                    opts.button.unwrap_or(MouseButton::Left),
                    opts.click_count.unwrap_or(1),
                    0,
                )
                .await?;
                Ok(Some(point))
            }
            Err(_err) => {
                js_click(session, target.backend_node_id).await?;
                Ok(None)
            }
        }
    }

    pub async fn hover(&self, ref_id: &Ref) -> Result<Point, CoreError> {
        let resolved = self.observer.resolve_ref(ref_id).await?;
        self.hover_node(
            &resolved.session,
            InputTarget::from_ref_entry(&resolved.entry),
        )
        .await
    }

    pub async fn hover_backend_node(&self, backend_node_id: i64) -> Result<Point, CoreError> {
        self.with_page_session_retry(|session| async move {
            self.hover_node(&session, InputTarget::from_backend_node(backend_node_id))
                .await
        })
        .await
    }

    async fn hover_node(
        &self,
        session: &ProtocolSession,
        target: InputTarget,
    ) -> Result<Point, CoreError> {
        scroll_into_view(session, target.backend_node_id).await;
        let point = get_element_center(session, target.backend_node_id).await?;
        self.check_click_point(session, &target, point).await?;
        dispatch_hover(session, point.x, point.y).await?;
        Ok(point)
    }

    pub async fn focus(&self, ref_id: &Ref) -> Result<(), CoreError> {
        let resolved = self.observer.resolve_ref(ref_id).await?;
        scroll_into_view(&resolved.session, resolved.backend_node_id).await;
        focus_element_js(&resolved.session, resolved.backend_node_id).await
    }

    pub async fn type_text(&self, text: &str) -> Result<(), CoreError> {
        self.with_page_session_retry(|session| {
            let text = text.to_string();
            async move { type_text(&session, &text).await }
        })
        .await
    }

    pub async fn press(&self, key: &str) -> Result<(), CoreError> {
        self.with_page_session_retry(|session| {
            let key = key.to_string();
            async move { press_combo(&session, &key).await }
        })
        .await
    }

    pub async fn select_option(
        &self,
        ref_id: &Ref,
        value: &str,
    ) -> Result<Option<String>, CoreError> {
        let resolved = self.observer.resolve_ref(ref_id).await?;
        self.select_backend_node_with_session(&resolved.session, resolved.backend_node_id, value)
            .await
    }

    pub async fn select_backend_node(
        &self,
        backend_node_id: i64,
        value: &str,
    ) -> Result<Option<String>, CoreError> {
        self.with_page_session_retry(|session| {
            let value = value.to_string();
            async move {
                self.select_backend_node_with_session(&session, backend_node_id, &value)
                    .await
            }
        })
        .await
    }

    async fn select_backend_node_with_session(
        &self,
        session: &ProtocolSession,
        backend_node_id: i64,
        value: &str,
    ) -> Result<Option<String>, CoreError> {
        scroll_into_view(session, backend_node_id).await;
        let selected = call_on_element(
            session,
            backend_node_id,
            SELECT_OPTION_FN,
            Some(vec![json!(value)]),
        )
        .await?;
        Ok(selected.as_str().map(ToString::to_string))
    }

    pub async fn check(&self, ref_id: &Ref) -> Result<bool, CoreError> {
        let resolved = self.observer.resolve_ref(ref_id).await?;
        let checked = call_on_element(
            &resolved.session,
            resolved.backend_node_id,
            "function(){return this.checked}",
            None,
        )
        .await?;
        if checked.as_bool() != Some(true) {
            let _ = self
                .click_node(
                    &resolved.session,
                    InputTarget::from_ref_entry(&resolved.entry),
                    ClickOptions::default(),
                )
                .await?;
        }
        Ok(true)
    }

    pub async fn uncheck(&self, ref_id: &Ref) -> Result<bool, CoreError> {
        let resolved = self.observer.resolve_ref(ref_id).await?;
        let checked = call_on_element(
            &resolved.session,
            resolved.backend_node_id,
            "function(){return this.checked}",
            None,
        )
        .await?;
        if checked.as_bool() == Some(true) {
            let _ = self
                .click_node(
                    &resolved.session,
                    InputTarget::from_ref_entry(&resolved.entry),
                    ClickOptions::default(),
                )
                .await?;
        }
        Ok(false)
    }

    pub async fn upload_file_by_ref(
        &self,
        ref_id: &Ref,
        files: Vec<String>,
    ) -> Result<(), CoreError> {
        let resolved = self.observer.resolve_ref(ref_id).await?;
        let _: Value = resolved
            .session
            .send(
                "DOM.setFileInputFiles",
                json!({ "files": files, "backendNodeId": resolved.backend_node_id }),
            )
            .await?;
        Ok(())
    }

    pub async fn handle_dialog(
        &self,
        accept: bool,
        prompt_text: Option<&str>,
    ) -> Result<(), CoreError> {
        self.with_page_session_retry(|session| async move {
            let mut params = serde_json::Map::new();
            params.insert("accept".to_string(), Value::Bool(accept));
            if let Some(prompt_text) = prompt_text {
                params.insert(
                    "promptText".to_string(),
                    Value::String(prompt_text.to_string()),
                );
            }
            let _: Value = session
                .send("Page.handleJavaScriptDialog", Value::Object(params))
                .await?;
            Ok(())
        })
        .await
    }

    pub async fn drag(&self, source_ref: &Ref, target_ref: &Ref) -> Result<DragResult, CoreError> {
        let source = self.observer.resolve_ref(source_ref).await?;
        let target = self.observer.resolve_ref(target_ref).await?;
        if !source.session.same_session(&target.session) {
            return Err(CoreError::CrossFrameDrag);
        }
        scroll_into_view(&source.session, source.backend_node_id).await;
        scroll_into_view(&target.session, target.backend_node_id).await;
        let from = get_element_center(&source.session, source.backend_node_id).await?;
        let to = get_element_center(&target.session, target.backend_node_id).await?;
        dispatch_drag(&source.session, from, to).await?;
        Ok(DragResult { from, to })
    }

    pub async fn scroll(
        &self,
        direction: ScrollDirection,
        amount: i64,
        ref_id: Option<&Ref>,
    ) -> Result<(), CoreError> {
        let pixels = amount * 120;
        let (delta_x, delta_y) = match direction {
            ScrollDirection::Left => (-pixels, 0),
            ScrollDirection::Right => (pixels, 0),
            ScrollDirection::Up => (0, -pixels),
            ScrollDirection::Down => (0, pixels),
        };
        if delta_x == 0 && delta_y == 0 {
            return Ok(());
        }
        if let Some(ref_id) = ref_id {
            let resolved = self.observer.resolve_ref(ref_id).await?;
            let point = get_element_center(&resolved.session, resolved.backend_node_id).await?;
            dispatch_scroll(
                &resolved.session,
                point.x,
                point.y,
                delta_x as f64,
                delta_y as f64,
            )
            .await?;
            return Ok(());
        }
        let session = self.page_session().await?;
        let _: Value = session
            .send(
                "Runtime.evaluate",
                json!({
                    "expression": format!("window.scrollBy({delta_x}, {delta_y})"),
                    "returnByValue": true
                }),
            )
            .await?;
        Ok(())
    }

    async fn page_session(&self) -> Result<ProtocolSession, CoreError> {
        Ok(self.pages.get_session(self.page_id.clone()).await?.session)
    }

    async fn check_click_point(
        &self,
        session: &ProtocolSession,
        target: &InputTarget,
        point: Point,
    ) -> Result<(), CoreError> {
        match click_blocker_at_point(session, target.backend_node_id, point).await {
            Ok(Some(blocker)) => Err(CoreError::ElementCovered {
                target: target.error_target.clone(),
                blocker,
            }),
            Ok(None) | Err(_) => Ok(()),
        }
    }

    async fn with_page_session_retry<F, Fut, T>(&self, action: F) -> Result<T, CoreError>
    where
        F: Fn(ProtocolSession) -> Fut,
        Fut: std::future::Future<Output = Result<T, CoreError>>,
    {
        let mut attempted = false;
        loop {
            let result = action(self.page_session().await?).await;
            match result {
                Ok(value) => return Ok(value),
                Err(err) if !attempted && err.is_retryable_session_loss() => {
                    attempted = true;
                    let _ = self.pages.refresh(self.page_id.clone()).await;
                }
                Err(err) => return Err(err),
            }
        }
    }
}

const SELECT_OPTION_FN: &str = "function(val){\
  for(var i=0;i<this.options.length;i++){\
    if(this.options[i].value===val||this.options[i].textContent.trim()===val){\
      this.selectedIndex=i;\
      this.dispatchEvent(new Event('change',{bubbles:true}));\
      return this.options[i].textContent.trim();\
    }\
  }\
  return null;\
}";

#[cfg(test)]
mod backend_node_tests {
    use super::*;
    use crate::{
        SessionId,
        frames::FrameRegistry,
        pages::PageManagerHooks,
        test_support::{TestCall, TestConnection},
    };

    pub(super) struct BackendHarness {
        pub input: Input,
        pub session: ProtocolSession,
        connection: Arc<TestConnection>,
        setup_calls: usize,
        methods: Vec<&'static str>,
    }

    impl BackendHarness {
        pub async fn new(responses: Vec<(&'static str, Value)>) -> Result<Self, CoreError> {
            let methods = responses.iter().map(|(method, _)| *method).collect();
            let mut setup = vec![
                (
                    "Browser.getTabs",
                    json!({ "tabs": [{
                    "tabId": 101, "targetId": "target-1", "url": "https://example.com/",
                    "title": "Test", "isActive": true, "isLoading": false,
                    "loadProgress": 1, "isPinned": false, "isHidden": false, "windowId": 1
                }] }),
                ),
                (
                    "Target.attachToTarget",
                    json!({ "sessionId": "page-session" }),
                ),
                ("Page.enable", json!({})),
                ("DOM.enable", json!({})),
                ("Runtime.enable", json!({})),
                ("Accessibility.enable", json!({})),
                ("Runtime.runIfWaitingForDebugger", json!({})),
            ];
            setup.extend(responses);
            let connection = TestConnection::new(setup);
            let pages = Arc::new(PageManager::new(
                connection.clone(),
                PageManagerHooks::default(),
            ));
            pages.get_session(PageId(1)).await?;
            let observer = Arc::new(Observer::new(
                pages.clone(),
                FrameRegistry::new(connection.clone()),
                PageId(1),
            ));
            Ok(Self {
                input: Input::new(observer, pages, PageId(1)),
                session: ProtocolSession::for_session(
                    connection.clone(),
                    SessionId::from("frame-session"),
                ),
                setup_calls: connection.calls()?.len(),
                connection,
                methods,
            })
        }

        pub fn assert_sequence(&self) -> Result<Vec<TestCall>, CoreError> {
            let calls = self
                .connection
                .calls()?
                .into_iter()
                .skip(self.setup_calls)
                .collect::<Vec<_>>();
            assert_eq!(
                calls
                    .iter()
                    .map(|call| call.method.as_str())
                    .collect::<Vec<_>>(),
                self.methods
            );
            for call in &calls {
                if call.method == "DOM.resolveNode" {
                    assert_eq!(call.params, json!({ "backendNodeId": 42 }));
                }
            }
            Ok(calls)
        }
    }

    pub(super) fn element_result(value: Value) -> Vec<(&'static str, Value)> {
        vec![
            (
                "DOM.resolveNode",
                json!({ "object": { "objectId": "element" } }),
            ),
            (
                "Runtime.callFunctionOn",
                json!({ "result": { "value": value } }),
            ),
        ]
    }

    pub(super) fn click_responses(blocker: Value) -> Vec<(&'static str, Value)> {
        let mut responses = vec![
            ("DOM.scrollIntoViewIfNeeded", json!({})),
            (
                "DOM.getContentQuads",
                json!({ "quads": [[0, 0, 100, 0, 100, 50, 0, 50]] }),
            ),
        ];
        responses.extend(element_result(blocker.clone()));
        if blocker.is_null() {
            responses.extend((0..3).map(|_| ("Input.dispatchMouseEvent", json!({}))));
        }
        responses
    }

    #[tokio::test]
    async fn check_already_desired_does_not_click() -> Result<(), CoreError> {
        for checked in [true, false] {
            let h = BackendHarness::new(element_result(json!(checked))).await?;
            assert_eq!(
                h.input.check_backend_node(&h.session, 42, checked).await?,
                checked
            );
            h.assert_sequence()?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn check_clicks_then_verifies_both_states() -> Result<(), CoreError> {
        for checked in [true, false] {
            let mut responses = element_result(json!(!checked));
            responses.extend(click_responses(Value::Null));
            responses.extend(element_result(json!(checked)));
            let h = BackendHarness::new(responses).await?;
            assert_eq!(
                h.input.check_backend_node(&h.session, 42, checked).await?,
                checked
            );
            let calls = h.assert_sequence()?;
            let events = calls
                .iter()
                .filter(|call| call.method == "Input.dispatchMouseEvent")
                .collect::<Vec<_>>();
            assert_eq!(
                events
                    .iter()
                    .map(|call| call.params["type"].as_str())
                    .collect::<Vec<_>>(),
                [
                    Some("mouseMoved"),
                    Some("mousePressed"),
                    Some("mouseReleased")
                ]
            );
            assert_eq!(events[1].params["x"], 50.0);
            assert_eq!(events[1].params["y"], 25.0);
            assert_eq!(events[1].params["button"], "left");
        }
        Ok(())
    }

    #[tokio::test]
    async fn check_rejects_unchanged_or_unreadable_state() -> Result<(), CoreError> {
        for final_state in [json!(false), Value::Null] {
            let mut responses = element_result(json!(false));
            responses.extend(click_responses(Value::Null));
            responses.extend(element_result(final_state));
            let h = BackendHarness::new(responses).await?;
            assert!(
                h.input
                    .check_backend_node(&h.session, 42, true)
                    .await
                    .is_err()
            );
            h.assert_sequence()?;
        }
        let h = BackendHarness::new(element_result(Value::Null)).await?;
        assert!(
            h.input
                .check_backend_node(&h.session, 42, false)
                .await
                .is_err()
        );
        h.assert_sequence()?;
        Ok(())
    }

    #[tokio::test]
    async fn check_preserves_cover_error_without_clicking() -> Result<(), CoreError> {
        let mut responses = element_result(json!(false));
        responses.extend(click_responses(json!("div#overlay")));
        let h = BackendHarness::new(responses).await?;
        assert!(
            matches!(h.input.check_backend_node(&h.session, 42, true).await,
            Err(CoreError::ElementCovered { target, blocker })
                if target.backend_node_id == Some(42) && blocker == "div#overlay")
        );
        h.assert_sequence()?;
        Ok(())
    }

    #[tokio::test]
    async fn upload_sends_validated_absolute_paths_and_allows_clearing() -> Result<(), CoreError> {
        let paths = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")];
        for files in [paths, Vec::new()] {
            let h = BackendHarness::new(vec![("DOM.setFileInputFiles", json!({}))]).await?;
            h.input.upload_backend_node(&h.session, 42, &files).await?;
            let calls = h.assert_sequence()?;
            assert_eq!(
                calls[0].params,
                json!({ "backendNodeId": 42, "files": files })
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn upload_validates_entire_batch_before_sending() -> Result<(), CoreError> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for invalid in [
            PathBuf::from("Cargo.toml"),
            root.join(uuid::Uuid::new_v4().to_string()),
            root.clone(),
        ] {
            let h = BackendHarness::new(Vec::new()).await?;
            assert!(
                h.input
                    .upload_backend_node(&h.session, 42, &[root.join("Cargo.toml"), invalid])
                    .await
                    .is_err()
            );
            h.assert_sequence()?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn focus_uses_backend_node_id() -> Result<(), CoreError> {
        let h = BackendHarness::new(vec![("DOM.focus", json!({}))]).await?;
        h.input.focus_backend_node(&h.session, 42).await?;
        let calls = h.assert_sequence()?;
        assert_eq!(calls[0].params, json!({ "backendNodeId": 42 }));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{ClickOptions, Input, ScrollDirection};
    use crate::{
        BrowserSession, BrowserSessionHooks, CoreError, Ref,
        connection::CdpConnection,
        snapshot::{AxNode, AxValue},
    };
    use browseros_cdp::{CdpError, CdpEvent};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};
    use tokio::sync::broadcast;

    #[derive(Clone, Copy)]
    enum HitTestResponse {
        Blocked(&'static str),
        Clear,
        Error,
    }

    struct HarnessState {
        hit_test: HitTestResponse,
        hit_test_calls: usize,
        mouse_events: usize,
        page_scrolls: usize,
        select_calls: usize,
        select_result: Option<&'static str>,
        select_semantics_preserved: bool,
    }

    struct HarnessConnection {
        state: Mutex<HarnessState>,
        target_role: &'static str,
        target_name: &'static str,
    }

    impl HarnessConnection {
        fn mouse_events(&self) -> usize {
            match self.state.lock() {
                Ok(state) => state.mouse_events,
                Err(_err) => 0,
            }
        }

        fn page_scrolls(&self) -> usize {
            match self.state.lock() {
                Ok(state) => state.page_scrolls,
                Err(_err) => 0,
            }
        }

        fn select_calls(&self) -> usize {
            match self.state.lock() {
                Ok(state) => state.select_calls,
                Err(_err) => 0,
            }
        }

        fn hit_test_calls(&self) -> usize {
            match self.state.lock() {
                Ok(state) => state.hit_test_calls,
                Err(_err) => 0,
            }
        }

        fn select_semantics_preserved(&self) -> bool {
            match self.state.lock() {
                Ok(state) => state.select_semantics_preserved,
                Err(_err) => false,
            }
        }
    }

    impl CdpConnection for HarnessConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            _session: Option<&'a crate::SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                match method {
                    "Browser.getTabs" => Ok(json!({ "tabs": [tab_value()] })),
                    "Browser.getTabInfo" => Ok(json!({ "tab": tab_value() })),
                    "Target.attachToTarget" => Ok(json!({ "sessionId": "session-1" })),
                    "Page.enable"
                    | "DOM.enable"
                    | "Runtime.enable"
                    | "Accessibility.enable"
                    | "Runtime.runIfWaitingForDebugger"
                    | "Target.setAutoAttach"
                    | "Runtime.releaseObject"
                    | "DOM.scrollIntoViewIfNeeded" => Ok(json!({})),
                    "Page.getFrameTree" => Ok(json!({
                        "frameTree": {
                            "frame": {
                                "id": "main",
                                "loaderId": "loader-1",
                                "url": "https://example.com/"
                            }
                        }
                    })),
                    "Accessibility.getFullAXTree" => Ok(json!({
                        "nodes": ax_tree(self.target_role, self.target_name)
                    })),
                    "Runtime.evaluate" => {
                        if params
                            .get("expression")
                            .and_then(Value::as_str)
                            .is_some_and(|expression| expression.contains("window.scrollBy"))
                            && let Ok(mut state) = self.state.lock()
                        {
                            state.page_scrolls += 1;
                        }
                        Ok(json!({ "result": { "value": [] } }))
                    }
                    "DOM.resolveNode" => Ok(json!({ "object": { "objectId": "target-object" } })),
                    "DOM.getContentQuads" => Ok(json!({
                        "quads": [[0.0, 0.0, 100.0, 0.0, 100.0, 50.0, 0.0, 50.0]]
                    })),
                    "Runtime.callFunctionOn" => self.call_function(params),
                    "Input.dispatchMouseEvent" => {
                        if let Ok(mut state) = self.state.lock() {
                            state.mouse_events += 1;
                        }
                        Ok(json!({}))
                    }
                    _ => Ok(json!({})),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a crate::SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_string()) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            let (_tx, rx) = broadcast::channel(1);
            rx
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    impl HarnessConnection {
        fn call_function(&self, params: Value) -> Result<Value, CdpError> {
            let function = params
                .get("functionDeclaration")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if function.contains("elementFromPoint") {
                let response = match self.state.lock() {
                    Ok(mut state) => {
                        state.hit_test_calls += 1;
                        state.hit_test
                    }
                    Err(_err) => HitTestResponse::Error,
                };
                return match response {
                    HitTestResponse::Blocked(blocker) => {
                        Ok(json!({ "result": { "value": blocker } }))
                    }
                    HitTestResponse::Clear => Ok(json!({ "result": { "value": null } })),
                    HitTestResponse::Error => Err(CdpError::Protocol {
                        code: -32000,
                        message: "execution context unavailable".to_string(),
                    }),
                };
            }
            if function.contains("return this.checked") {
                return Ok(json!({ "result": { "value": false } }));
            }
            if function.contains("this.options")
                && let Ok(mut state) = self.state.lock()
            {
                state.select_calls += 1;
                state.select_semantics_preserved = function.contains(
                    "this.options[i].value===val||this.options[i].textContent.trim()===val",
                ) && function.contains("this.selectedIndex=i")
                    && function.contains("this.dispatchEvent(new Event('change',{bubbles:true}))");
                return Ok(json!({ "result": { "value": state.select_result } }));
            }
            Ok(json!({ "result": { "value": null } }))
        }
    }

    async fn input_harness(
        hit_test: HitTestResponse,
    ) -> Result<(Arc<HarnessConnection>, Input, Ref), CoreError> {
        input_harness_for_target(hit_test, "button", "Submit", Some("Choice")).await
    }

    async fn select_input_harness(
        hit_test: HitTestResponse,
        select_result: Option<&'static str>,
    ) -> Result<(Arc<HarnessConnection>, Input, Ref), CoreError> {
        input_harness_for_target(hit_test, "combobox", "Sort by:", select_result).await
    }

    async fn input_harness_for_target(
        hit_test: HitTestResponse,
        target_role: &'static str,
        target_name: &'static str,
        select_result: Option<&'static str>,
    ) -> Result<(Arc<HarnessConnection>, Input, Ref), CoreError> {
        let connection = Arc::new(HarnessConnection {
            state: Mutex::new(HarnessState {
                hit_test,
                hit_test_calls: 0,
                mouse_events: 0,
                page_scrolls: 0,
                select_calls: 0,
                select_result,
                select_semantics_preserved: false,
            }),
            target_role,
            target_name,
        });
        let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
        let pages = session.pages.list().await?;
        let page_id = match pages.first() {
            Some(page) => page.page_id.clone(),
            None => return Err(CoreError::Message("missing test page".to_string())),
        };
        let observer = session.observe(page_id.clone()).await;
        let _snapshot = observer.snapshot().await?;
        let input = session.input(page_id).await;
        Ok((connection, input, Ref("e1".to_string())))
    }

    fn tab_value() -> Value {
        json!({
            "tabId": 101,
            "targetId": "target-1",
            "url": "https://example.com/",
            "title": "Test",
            "isActive": true,
            "isLoading": false,
            "loadProgress": 1,
            "isPinned": false,
            "isHidden": false,
            "windowId": 1
        })
    }

    fn ax_tree(target_role: &str, target_name: &str) -> Vec<AxNode> {
        vec![
            AxNode {
                node_id: "root".to_string(),
                role: Some(AxValue::role("RootWebArea")),
                child_ids: Some(vec!["target".to_string()]),
                ..AxNode::default()
            },
            AxNode {
                node_id: "target".to_string(),
                role: Some(AxValue::role(target_role)),
                name: Some(AxValue::string(target_name)),
                backend_dom_node_id: Some(10),
                ..AxNode::default()
            },
        ]
    }

    #[tokio::test]
    async fn click_reports_covered_ref_and_skips_dispatch() -> Result<(), CoreError> {
        let (connection, input, ref_id) =
            input_harness(HitTestResponse::Blocked("div#consent-banner")).await?;

        let result = input.click(&ref_id, ClickOptions::default()).await;

        let err = match result {
            Err(err) => err,
            Ok(other) => {
                return Err(CoreError::Message(format!(
                    "expected ElementCovered, got {other:?}"
                )));
            }
        };
        let message = err.to_string();
        match err {
            CoreError::ElementCovered { target, blocker } => {
                assert_eq!(target.ref_id, Some(Ref("e1".to_string())));
                assert_eq!(target.role.as_deref(), Some("button"));
                assert_eq!(target.name.as_deref(), Some("Submit"));
                assert_eq!(blocker, "div#consent-banner");
            }
            other => {
                return Err(CoreError::Message(format!(
                    "expected ElementCovered, got {other:?}"
                )));
            }
        }
        assert_eq!(connection.mouse_events(), 0);
        assert_eq!(
            message,
            "Element e1 (button \"Submit\") is covered by <div#consent-banner> at its click point; the click would hit that element instead. Dismiss or interact with the covering element first (often a dialog, banner, or sticky header)."
        );
        Ok(())
    }

    #[tokio::test]
    async fn click_proceeds_when_hit_test_fails() -> Result<(), CoreError> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Error).await?;

        let point = input.click(&ref_id, ClickOptions::default()).await?;

        assert_eq!(point.map(|point| (point.x, point.y)), Some((50.0, 25.0)));
        assert_eq!(connection.mouse_events(), 3);
        Ok(())
    }

    #[tokio::test]
    async fn hover_reports_covered_ref_before_mouse_move() -> Result<(), CoreError> {
        let (connection, input, ref_id) =
            input_harness(HitTestResponse::Blocked("div.toast")).await?;

        let result = input.hover(&ref_id).await;

        assert!(matches!(result, Err(CoreError::ElementCovered { .. })));
        assert_eq!(connection.mouse_events(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn semantic_select_ignores_pointer_occlusion_but_click_stays_blocked()
    -> Result<(), CoreError> {
        let (connection, input, ref_id) = select_input_harness(
            HitTestResponse::Blocked("span.a-dropdown-prompt"),
            Some("Price: Low to High"),
        )
        .await?;

        let selected = input.select_option(&ref_id, "price-asc-rank").await?;

        assert_eq!(selected.as_deref(), Some("Price: Low to High"));
        assert_eq!(connection.select_calls(), 1);
        assert_eq!(connection.hit_test_calls(), 0);
        assert_eq!(connection.mouse_events(), 0);
        assert!(connection.select_semantics_preserved());

        let click_result = input.click(&ref_id, ClickOptions::default()).await;
        match click_result {
            Err(CoreError::ElementCovered { target, blocker }) => {
                assert_eq!(target.ref_id, Some(Ref("e1".to_string())));
                assert_eq!(target.role.as_deref(), Some("combobox"));
                assert_eq!(target.name.as_deref(), Some("Sort by:"));
                assert_eq!(blocker, "span.a-dropdown-prompt");
            }
            other => {
                return Err(CoreError::Message(format!(
                    "expected ElementCovered, got {other:?}"
                )));
            }
        }
        assert_eq!(connection.hit_test_calls(), 1);
        assert_eq!(connection.mouse_events(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn semantic_select_returns_none_for_missing_option() -> Result<(), CoreError> {
        let (connection, input, ref_id) =
            select_input_harness(HitTestResponse::Clear, None).await?;

        let selected = input.select_option(&ref_id, "missing").await?;

        assert_eq!(selected, None);
        assert_eq!(connection.select_calls(), 1);
        assert_eq!(connection.hit_test_calls(), 0);
        assert!(connection.select_semantics_preserved());
        Ok(())
    }

    #[tokio::test]
    async fn page_scroll_uses_runtime_instead_of_mouse_wheel() -> Result<(), CoreError> {
        let (connection, input, _ref_id) = input_harness(HitTestResponse::Clear).await?;

        input.scroll(ScrollDirection::Down, 3, None).await?;

        assert_eq!(connection.page_scrolls(), 1);
        assert_eq!(connection.mouse_events(), 0);
        Ok(())
    }
}
