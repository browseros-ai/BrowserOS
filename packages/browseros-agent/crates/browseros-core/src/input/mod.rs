mod click;
mod fill;
pub mod geometry;
pub mod keyboard;
pub mod mouse;

use crate::{
    CoreError, CoveredElementTarget, PageId, PageJs, ProtocolSession, Ref,
    observer::Observer,
    pages::PageManager,
    pointer::{Pointer, PointerAction},
    snapshot::RefEntry,
    target::page_access::PageAccess,
    timeouts,
};
use geometry::{click_blocker_at_point, focus_element_js, get_element_center, scroll_into_view};
use mouse::{
    MouseButton, dispatch_click, dispatch_click_with_options, dispatch_drag, dispatch_hover,
    dispatch_scroll,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

/// Delivery state survives cancellation of the click future by a dialog/picker
/// race. Started means the one-shot handoff began; delivery is still uncertain.
/// A dispatched press is trusted when BrowserOS pressed the element natively,
/// and inert when the page had made it inert, usually behind an open dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomFallbackStatus {
    Started,
    Dispatched { trusted: bool, inert: bool },
}

/// `Input::dom_fallback` states: started, or dispatched with flag bits.
const FALLBACK_STARTED: u8 = 1;
const FALLBACK_DISPATCHED: u8 = 2;
const FALLBACK_TRUSTED: u8 = 4;
const FALLBACK_INERT: u8 = 8;

pub use keyboard::{
    KeyInfo, clear_field, get_key_info, insert_text, modifier_bitmask, normalize_key, press_combo,
    type_text,
};
pub use mouse::MouseButton as PublicMouseButton;

#[derive(Debug, Clone, Default)]
pub struct ClickOptions {
    pub button: Option<MouseButton>,
    pub click_count: Option<i64>,
    /// Path start in the target's coordinate space: page coordinates for `click_at`,
    /// frame coordinates for ref clicks inside iframes.
    pub humanlike_from: Option<Point>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DragResult {
    pub from: Point,
    pub to: Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

/// Input uses the observer capability for element refs and the same captured capability for
/// direct coordinates/keyboard commands. Retaining this handle cannot adopt a replacement target.
pub struct Input {
    observer: Arc<Observer>,
    pages: Arc<PageManager>,
    page_id: PageId,
    access: Option<Arc<PageAccess>>,
    pointer: Option<Pointer>,
    /// A zoomed mobile preview (see `crate::viewport`) takes main-frame pointer coordinates in
    /// its scaled on-screen space; everything else here is in CSS pixels.
    pointer_scale: f64,
    /// Observe delivery after a raced future is dropped. This flag publishes no
    /// other data, so relaxed atomic access suffices across executor threads.
    dom_fallback: AtomicU8,
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
    #[must_use]
    pub fn new(observer: Arc<Observer>, pages: Arc<PageManager>, page_id: PageId) -> Self {
        Self {
            observer,
            pages,
            page_id,
            access: None,
            pointer: None,
            pointer_scale: 1.0,
            dom_fallback: AtomicU8::new(0),
        }
    }

    /// Includes the handoff even when a modal interrupts its acknowledgement.
    /// Native clicks leave a previous fallback notice intact within this Input.
    pub fn dom_fallback_status(&self) -> Option<DomFallbackStatus> {
        match self.dom_fallback.load(Ordering::Relaxed) {
            0 => None,
            FALLBACK_STARTED => Some(DomFallbackStatus::Started),
            state => Some(DomFallbackStatus::Dispatched {
                trusted: state & FALLBACK_TRUSTED != 0,
                inert: state & FALLBACK_INERT != 0,
            }),
        }
    }

    pub(crate) fn with_access(mut self, access: Arc<PageAccess>) -> Self {
        self.access = Some(access);
        self
    }

    pub(crate) fn with_pointer(mut self, pointer: Option<Pointer>) -> Self {
        self.pointer = pointer;
        self
    }

    pub(crate) fn with_pointer_scale(mut self, scale: f64) -> Self {
        self.pointer_scale = scale;
        self
    }

    /// Maps a CSS point to where the pointer event must land. Child-frame (OOPIF) sessions
    /// take their own coordinates unscaled; only the page's main session sees the preview
    /// scale.
    async fn dispatch_point(&self, session: &ProtocolSession, point: Point) -> Point {
        if self.pointer_scale == 1.0 {
            return point;
        }
        match self.page_session().await {
            Ok(root) if root.same_session(session) => Point {
                x: point.x * self.pointer_scale,
                y: point.y * self.pointer_scale,
            },
            _ => point,
        }
    }

    async fn dispatch_options(
        &self,
        session: &ProtocolSession,
        opts: ClickOptions,
    ) -> ClickOptions {
        let humanlike_from = match opts.humanlike_from {
            Some(from) => Some(self.dispatch_point(session, from).await),
            None => None,
        };
        ClickOptions {
            humanlike_from,
            ..opts
        }
    }

    /// Recover on one pinned node, preferring native input. `Some(point)` means
    /// native dispatch; `None` means the internal DOM fallback dispatched once.
    /// Either acknowledges delivery only; callers must verify application state.
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

    /// Backend-node callers share the ref click transaction and delivery result.
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
            let opts = opts.clone();
            async move {
                if let Some(pointer) = self.drawing_pointer().await
                    && pointer
                        .move_to(&session, PointerAction::Click, Point { x, y })
                        .await
                {
                    pointer.click_effect(&session, Point { x, y }).await;
                }
                let at = self.dispatch_point(&session, Point { x, y }).await;
                let opts = self.dispatch_options(&session, opts).await;
                dispatch_click_with_options(&session, at.x, at.y, opts, 0).await
            }
        })
        .await
    }

    pub async fn hover_at(&self, x: f64, y: f64) -> Result<(), CoreError> {
        self.with_page_session_retry(|session| async move {
            if let Some(pointer) = self.drawing_pointer().await {
                pointer
                    .move_to(&session, PointerAction::Hover, Point { x, y })
                    .await;
            }
            let at = self.dispatch_point(&session, Point { x, y }).await;
            dispatch_hover(&session, at.x, at.y).await
        })
        .await
    }

    pub async fn type_at(&self, x: f64, y: f64, text: &str, clear: bool) -> Result<(), CoreError> {
        self.with_page_session_retry(|session| {
            let text = text.to_string();
            async move {
                let at = self.dispatch_point(&session, Point { x, y }).await;
                dispatch_click(&session, at.x, at.y, MouseButton::Left, 1, 0).await?;
                if clear {
                    clear_field(&session).await?;
                }
                type_text(&session, &text).await
            }
        })
        .await
    }

    pub async fn drag_at(&self, from: Point, to: Point) -> Result<(), CoreError> {
        self.with_page_session_retry(|session| async move {
            let from = self.dispatch_point(&session, from).await;
            let to = self.dispatch_point(&session, to).await;
            dispatch_drag(&session, from, to).await
        })
        .await
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
        let mut point = get_element_center(session, target.backend_node_id).await?;
        self.check_click_point(session, &target, point).await?;
        if let Some(pointer) = self.drawing_pointer().await
            && pointer.move_to(session, PointerAction::Hover, point).await
        {
            point = get_element_center(session, target.backend_node_id).await?;
            self.check_click_point(session, &target, point).await?;
        }
        let at = self.dispatch_point(session, point).await;
        dispatch_hover(session, at.x, at.y).await?;
        Ok(point)
    }

    pub async fn focus(&self, ref_id: &Ref) -> Result<(), CoreError> {
        let resolved = self.observer.resolve_ref(ref_id).await?;
        scroll_into_view(&resolved.session, resolved.backend_node_id).await;
        if let Some(pointer) = self.drawing_pointer().await
            && let Ok(point) = get_element_center(&resolved.session, resolved.backend_node_id).await
        {
            pointer
                .move_to(&resolved.session, PointerAction::Focus, point)
                .await;
        }
        focus_element_js(&resolved.session, resolved.backend_node_id).await
    }

    pub async fn type_text(&self, text: &str) -> Result<(), CoreError> {
        self.with_page_session_retry(|session| {
            let text = text.to_string();
            async move { type_text(&session, &text).await }
        })
        .await
    }

    pub async fn insert_text(&self, text: &str) -> Result<(), CoreError> {
        self.with_page_session_retry(|session| {
            let text = text.to_string();
            async move { insert_text(&session, &text).await }
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
        PageJs::isolated(session)
            .call(backend_node_id, SELECT_OPTION_FN, &[json!(value)])
            .await
    }

    pub async fn check(&self, ref_id: &Ref) -> Result<bool, CoreError> {
        self.set_checked(ref_id, true).await
    }

    pub async fn uncheck(&self, ref_id: &Ref) -> Result<bool, CoreError> {
        self.set_checked(ref_id, false).await
    }

    /// Clicks the control only when it is not already in the wanted state, then confirms the
    /// click changed it. Native checkboxes and radios report `checked`; custom switches report
    /// `aria-checked`. A control with neither keeps the old contract: check clicks it once,
    /// uncheck leaves it, and nothing can be confirmed.
    async fn set_checked(&self, ref_id: &Ref, checked: bool) -> Result<bool, CoreError> {
        let resolved = self.observer.resolve_ref(ref_id).await?;
        let js = PageJs::isolated(&resolved.session);
        let before: Option<CheckState> = js
            .call(resolved.backend_node_id, CHECK_STATE_FN, &[])
            .await?;
        let Some(before) = before else {
            if checked {
                self.click_node(
                    &resolved.session,
                    InputTarget::from_ref_entry(&resolved.entry),
                    ClickOptions::default(),
                )
                .await?;
            }
            return Ok(checked);
        };
        if before.checked == checked {
            return Ok(checked);
        }
        if before.radio && !checked {
            return Err(CoreError::Message(format!(
                "{ref_id} is a radio button, which cannot be unchecked. Check another option in its group instead."
            )));
        }
        self.click_node(
            &resolved.session,
            InputTarget::from_ref_entry(&resolved.entry),
            ClickOptions::default(),
        )
        .await?;
        // A framework can apply the new state a tick after the click event, so poll briefly
        // instead of reading once.
        let deadline = tokio::time::Instant::now() + timeouts::CHECK_STATE_SETTLE;
        loop {
            // The click has landed. A control the page then replaced, or a page that navigated
            // away, can no longer report its state; the readback shows the page as it is, and
            // an error here would only invite a retry that clicks again and flips it back.
            let Ok(after) = js
                .call::<Option<CheckState>>(resolved.backend_node_id, CHECK_STATE_FN, &[])
                .await
            else {
                return Ok(checked);
            };
            match after {
                Some(after) if after.detached || after.checked == checked => return Ok(checked),
                _ => {}
            }
            if tokio::time::Instant::now() >= deadline {
                let wanted = if checked { "checked" } else { "unchecked" };
                return Err(CoreError::Message(format!(
                    "Clicking {ref_id} did not make it {wanted}. The page may ignore the click or need another control; take a new snapshot."
                )));
            }
            tokio::time::sleep(timeouts::CHECK_STATE_POLL).await;
        }
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
        dispatch_drag(
            &source.session,
            self.dispatch_point(&source.session, from).await,
            self.dispatch_point(&source.session, to).await,
        )
        .await?;
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
            let at = self.dispatch_point(&resolved.session, point).await;
            dispatch_scroll(
                &resolved.session,
                at.x,
                at.y,
                delta_x as f64,
                delta_y as f64,
            )
            .await?;
            return Ok(());
        }
        let session = self.page_session().await?;
        let _: Value = PageJs::isolated(&session)
            .eval(&format!("window.scrollBy({delta_x}, {delta_y})"))
            .await?;
        Ok(())
    }

    /// Best-effort: true when the ref resolves to a password or payment-card field.
    /// Used by the audit layer to redact the value before it is persisted; a read
    /// failure returns false (the explicit `sensitive` flag is the deterministic path).
    /// The read runs once more after a failure, as [`Self::evaluate_sensitive`] does.
    pub async fn ref_field_is_sensitive(&self, ref_id: &Ref) -> bool {
        for _ in 0..SENSITIVE_CHECK_ATTEMPTS {
            let Ok(resolved) = self.observer.resolve_ref(ref_id).await else {
                continue;
            };
            if let Ok(sensitive) = PageJs::isolated(&resolved.session)
                .call::<Option<bool>>(resolved.backend_node_id, &sensitive_field_fn(), &[])
                .await
            {
                return sensitive.unwrap_or(false);
            }
        }
        false
    }

    /// Best-effort: true when the currently focused element is a password or
    /// payment-card field. For `type`, which targets focus. Descends shadow roots
    /// and same-origin iframes to the real focused element; a cross-origin iframe is
    /// opaque, so a secret typed into one relies on the explicit `sensitive` flag.
    pub async fn focused_field_is_sensitive(&self) -> bool {
        self.evaluate_sensitive(
            &FOCUSED_SENSITIVE_JS.replace("__PRED__", SENSITIVE_FIELD_PREDICATE),
        )
        .await
    }

    /// Best-effort: true when the element at the viewport point is a password or
    /// payment-card field. For `type_at`, which targets coordinates. Descends shadow
    /// roots and same-origin iframes (translating the point into the frame); a
    /// cross-origin iframe is opaque and relies on the explicit `sensitive` flag.
    pub async fn point_field_is_sensitive(&self, x: f64, y: f64) -> bool {
        let expression = POINT_SENSITIVE_JS
            .replace("__PRED__", SENSITIVE_FIELD_PREDICATE)
            .replace("__X__", &x.to_string())
            .replace("__Y__", &y.to_string());
        self.evaluate_sensitive(&expression).await
    }

    /// Runs the check once more after any failure. A session can die between the isolated
    /// world's setup and the script, and the action that follows then re-attaches and types a
    /// secret this check reported as not sensitive. Unlike the input retry, a second read
    /// repeats nothing the page sees, so it needs no proof that the first one never landed.
    async fn evaluate_sensitive(&self, expression: &str) -> bool {
        for _ in 0..SENSITIVE_CHECK_ATTEMPTS {
            let Ok(session) = self.page_session().await else {
                continue;
            };
            if let Ok(sensitive) = PageJs::isolated(&session)
                .eval::<Option<bool>>(expression)
                .await
            {
                return sensitive.unwrap_or(false);
            }
        }
        false
    }

    async fn drawing_pointer(&self) -> Option<Pointer> {
        let pointer = self.pointer?;
        let page = match &self.access {
            Some(access) => access.fresh().await.ok()?,
            None => self
                .pages
                .refresh(self.page_id.clone())
                .await
                .ok()
                .flatten()?,
        };
        page.is_active.then_some(pointer)
    }

    async fn page_session(&self) -> Result<ProtocolSession, CoreError> {
        match &self.access {
            Some(access) => Ok(access.session().await?.session),
            None => Ok(self.pages.get_session(self.page_id.clone()).await?.session),
        }
    }

    async fn check_click_point(
        &self,
        session: &ProtocolSession,
        target: &InputTarget,
        point: Point,
    ) -> Result<(), CoreError> {
        match click_blocker_at_point(session, target.backend_node_id, point).await {
            Ok(Some(blocker)) => Err(CoreError::ElementCovered {
                target: Box::new(target.error_target.clone()),
                blocker,
                blocker_ref: None,
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
            // Run again only if the session was gone before anything reached the page. Once a
            // command was answered, part of the action (a keyDown, a mouse press) may have
            // landed, and running it again on a new attachment would repeat it.
            let (session, answered) = self.page_session().await?.witnessed();
            let result = action(session).await;
            match result {
                Ok(value) => return Ok(value),
                Err(err)
                    if !attempted
                        && err.is_retryable_session_loss()
                        && !answered.load(Ordering::SeqCst) =>
                {
                    attempted = true;
                    let _ = self.pages.refresh(self.page_id.clone()).await;
                }
                Err(err) => return Err(err),
            }
        }
    }
}

/// A field is sensitive when it is an `<input type=password>` or carries an
/// autocomplete token for a password, a one-time code, or a payment card. Written
/// as a JS arrow `(el) => bool` so it can run against `this`, `activeElement`, or
/// `elementFromPoint`.
const SENSITIVE_FIELD_PREDICATE: &str = "(el)=>{\
  if(!el)return false;\
  const type=((el.getAttribute&&el.getAttribute('type'))||el.type||'').toLowerCase();\
  if(el.tagName==='INPUT'&&type==='password')return true;\
  const ac=((el.getAttribute&&el.getAttribute('autocomplete'))||'').toLowerCase();\
  return ac.split(/\\s+/).some(t=>t==='current-password'||t==='new-password'||t==='one-time-code'||t.startsWith('cc-'));\
}";

/// The sensitivity predicate wrapped for `Runtime.callFunctionOn` against a node (`this`).
fn sensitive_field_fn() -> String {
    format!("function(){{const pred={SENSITIVE_FIELD_PREDICATE};return pred(this);}}")
}

/// Reads in the audit layer's sensitive-field checks: the first, and one after a failure that
/// may have been a session dying mid-read.
const SENSITIVE_CHECK_ATTEMPTS: usize = 2;

/// Resolves the real focused element, descending shadow roots and same-origin
/// iframes, then applies the predicate. `__PRED__` is replaced with the predicate.
/// The loop is capped so a pathological tree cannot spin.
const FOCUSED_SENSITIVE_JS: &str = "(()=>{const pred=__PRED__;\
  let el=document.activeElement;\
  for(let i=0;i<32&&el;i++){\
    if(el.shadowRoot&&el.shadowRoot.activeElement){el=el.shadowRoot.activeElement;continue;}\
    if(el.tagName==='IFRAME'){let d=null;try{d=el.contentDocument;}catch(e){d=null;}\
      if(d&&d.activeElement){el=d.activeElement;continue;}}\
    break;\
  }\
  return pred(el);})()";

/// Resolves the element at a viewport point, descending shadow roots and
/// same-origin iframes (translating the point into each frame), then applies the
/// predicate. `__PRED__`, `__X__`, `__Y__` are replaced before evaluation.
const POINT_SENSITIVE_JS: &str = "(()=>{const pred=__PRED__;\
  let x=__X__,y=__Y__;\
  let el=document.elementFromPoint(x,y);\
  for(let i=0;i<32&&el;i++){\
    if(el.shadowRoot){const inner=el.shadowRoot.elementFromPoint(x,y);\
      if(inner&&inner!==el){el=inner;continue;}}\
    if(el.tagName==='IFRAME'){let d=null;try{d=el.contentDocument;}catch(e){d=null;}\
      if(d){const r=el.getBoundingClientRect();x=x-r.left;y=y-r.top;\
        const inner=d.elementFromPoint(x,y);if(inner){el=inner;continue;}}}\
    break;\
  }\
  return pred(el);})()";

/// What [`CHECK_STATE_FN`] reads.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
struct CheckState {
    checked: bool,
    radio: bool,
    /// The control is no longer in the document, typically replaced by a re-render.
    #[serde(default)]
    detached: bool,
}

/// Reads a control's checked state, following a label to the control it names. Returns null
/// when the element has no checked state at all.
///
/// A mixed control reads as its underlying state, as Playwright's `getCheckedWithoutMixed`
/// does: a native box's `checked` sits beneath `indeterminate`, and clicking clears the mixed
/// look by toggling that, so treating mixed as unknown and clicking would invert a box that
/// was already where the caller wanted it. An `aria-checked="mixed"` control counts as
/// unchecked.
const CHECK_STATE_FN: &str = "function(){\
const el=this.tagName==='LABEL'&&this.control?this.control:this;\
if(!el.isConnected)return{checked:false,radio:false,detached:true};\
if(el.tagName==='INPUT'&&(el.type==='checkbox'||el.type==='radio'))\
return{checked:el.checked,radio:el.type==='radio'};\
const aria=el.getAttribute&&el.getAttribute('aria-checked');\
if(aria!=='true'&&aria!=='false'&&aria!=='mixed')return null;\
const role=(el.getAttribute('role')||'').trim().split(/\\s+/)[0];\
return{checked:aria==='true',radio:role==='radio'||role==='menuitemradio'};\
}";

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
mod tests {
    use super::{
        ClickOptions, DomFallbackStatus, Input, Point, PublicMouseButton, ScrollDirection,
    };
    use crate::{
        BrowserSession, BrowserSessionHooks, CoreError, Ref,
        connection::CdpConnection,
        pointer::Pointer,
        snapshot::{AxNode, AxValue},
        target::BrowserView,
        test_support::TestCall,
        timeouts,
    };
    use browseros_cdp::{CdpError, CdpEvent};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};
    use tokio::sync::broadcast;
    use tokio::time::{Duration, Instant};

    #[derive(Clone, Copy)]
    enum HitTestResponse {
        Blocked(&'static str),
        Clear,
        Error,
    }

    #[derive(Clone, Copy)]
    enum PointerResponse {
        Arrived,
        Skipped,
        Error,
        Pending,
    }

    /// How the browser answers `DOM.click`.
    #[derive(Clone, Copy)]
    enum NativePress {
        Pressed {
            inert: bool,
        },
        Refused(&'static str),
        /// A BrowserOS build from before `DOM.click`.
        Missing,
        /// Sent, but the acknowledgement never came back.
        Lost,
    }

    struct HarnessState {
        hit_test: HitTestResponse,
        hit_test_calls: usize,
        mouse_events: usize,
        fail_mouse_ack: Option<&'static str>,
        native_press: NativePress,
        fail_dom_ack: bool,
        mouse_calls: Vec<(Value, Instant)>,
        page_scrolls: usize,
        select_calls: usize,
        select_result: Option<&'static str>,
        select_semantics_preserved: bool,
        calls: Vec<TestCall>,
        pointer_response: PointerResponse,
        pointer_moved: bool,
        effect_pending: bool,
        moved_box: bool,
        missing_geometry: bool,
        missing_geometry_after_move: bool,
        hit_test_after_move: Option<HitTestResponse>,
        field_value: String,
        /// What the page's input handler turns typed text into, like a code field grouping digits.
        reformat: Option<fn(&str) -> String>,
        tab_active: bool,
        tab_info_error: bool,
        tab_list_error: bool,
        tab_target: &'static str,
        /// What the check-state read answers: null when the element has no checked state.
        check_state: Value,
        /// The check state a mouse press leaves behind, for controls the click toggles.
        check_state_after_click: Option<Value>,
        /// The held move (counting from 1) that never gets an answer, to cancel a drag mid-way.
        stall_held_move: Option<usize>,
        /// The held move (counting from 1) that the browser rejects, to fail a drag mid-way.
        fail_held_move: Option<usize>,
        held_moves: usize,
        /// The field is a date/time/colour/range input, which fill sets instead of typing.
        value_control: bool,
        /// What the set-value script answers: true, or the reason the value was refused.
        set_value_result: Value,
        set_values: Vec<Value>,
    }

    struct HarnessConnection {
        state: Mutex<HarnessState>,
        target_role: &'static str,
        target_name: &'static str,
    }

    impl HarnessConnection {
        fn mouse_calls(&self) -> Result<Vec<(Value, Instant)>, CoreError> {
            Ok(self
                .state
                .lock()
                .map_err(|_| CoreError::Message("poisoned".into()))?
                .mouse_calls
                .clone())
        }

        fn configure(&self, update: impl FnOnce(&mut HarnessState)) -> Result<(), CoreError> {
            let mut state = self
                .state
                .lock()
                .map_err(|_| CoreError::Message("poisoned test state".into()))?;
            update(&mut state);
            Ok(())
        }

        fn take_calls(&self) -> Result<Vec<TestCall>, CoreError> {
            let mut calls = Vec::new();
            self.configure(|state| calls = std::mem::take(&mut state.calls))?;
            Ok(calls)
        }

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
                self.configure(|state| {
                    state.calls.push(TestCall {
                        method: method.to_string(),
                        params: params.clone(),
                    });
                })
                .map_err(|err| CdpError::Protocol {
                    code: -1,
                    message: err.to_string(),
                })?;
                match method {
                    "Browser.getTabs" | "Browser.getTabInfo" => {
                        let state = self.state.lock().map_err(|_| CdpError::Protocol {
                            code: -1,
                            message: "poisoned test state".into(),
                        })?;
                        if (method == "Browser.getTabs" && state.tab_list_error)
                            || (method == "Browser.getTabInfo" && state.tab_info_error)
                        {
                            return Err(CdpError::Protocol {
                                code: -32000,
                                message: "tab information unavailable".into(),
                            });
                        }
                        let mut tab = tab_value();
                        tab["isActive"] = json!(state.tab_active);
                        tab["targetId"] = json!(state.tab_target);
                        Ok(if method == "Browser.getTabs" {
                            json!({ "tabs": [tab] })
                        } else {
                            json!({ "tab": tab })
                        })
                    }
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
                        let pointer_move = params["expression"]
                            .as_str()
                            .is_some_and(|expression| expression.contains(r#""op":"move""#));
                        if pointer_move {
                            let response = {
                                let mut state =
                                    self.state.lock().map_err(|_| CdpError::Protocol {
                                        code: -1,
                                        message: "poisoned test state".into(),
                                    })?;
                                state.pointer_moved = true;
                                state.pointer_response
                            };
                            return match response {
                                PointerResponse::Arrived => {
                                    Ok(json!({ "result": { "value": { "status": "arrived" } } }))
                                }
                                PointerResponse::Skipped => Ok(
                                    json!({ "result": { "value": { "status": "skipped", "reason": "hidden" } } }),
                                ),
                                PointerResponse::Error => Err(CdpError::Protocol {
                                    code: -32000,
                                    message: "drawing failed".into(),
                                }),
                                PointerResponse::Pending => std::future::pending().await,
                            };
                        }
                        let effect = params["expression"]
                            .as_str()
                            .is_some_and(|expression| expression.contains(".click("));
                        if effect {
                            let pending = self
                                .state
                                .lock()
                                .map_err(|_| CdpError::Protocol {
                                    code: -1,
                                    message: "poisoned test state".into(),
                                })?
                                .effect_pending;
                            if pending {
                                return std::future::pending().await;
                            }
                            return Ok(json!({ "result": { "value": true } }));
                        }
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
                    "DOM.click" => {
                        let press = self
                            .state
                            .lock()
                            .map_err(|_| CdpError::ConnectionLost)?
                            .native_press;
                        match press {
                            NativePress::Pressed { inert } => Ok(json!({ "inert": inert })),
                            NativePress::Refused(code) => Ok(json!({ "refused": code })),
                            NativePress::Missing => Err(CdpError::Protocol {
                                code: -32601,
                                message: "'DOM.click' wasn't found".into(),
                            }),
                            NativePress::Lost => Err(CdpError::Protocol {
                                code: -32000,
                                message: "Inspected target navigated or closed".into(),
                            }),
                        }
                    }
                    "DOM.resolveNode" => Ok(json!({ "object": { "objectId": "target-object" } })),
                    "DOM.getNodeForLocation" => {
                        Ok(json!({ "backendNodeId": 20, "frameId": "main" }))
                    }
                    "DOM.getContentQuads" => {
                        let state = self.state.lock().map_err(|_| CdpError::Protocol {
                            code: -1,
                            message: "poisoned test state".into(),
                        })?;
                        if state.missing_geometry
                            || (state.pointer_moved && state.missing_geometry_after_move)
                        {
                            Ok(json!({ "quads": [] }))
                        } else if state.pointer_moved && state.moved_box {
                            Ok(
                                json!({ "quads": [[200.0, 100.0, 300.0, 100.0, 300.0, 150.0, 200.0, 150.0]] }),
                            )
                        } else {
                            Ok(json!({ "quads": [[0.0, 0.0, 100.0, 0.0, 100.0, 50.0, 0.0, 50.0]] }))
                        }
                    }
                    "Runtime.callFunctionOn" => self.call_function(params),
                    "Input.insertText" => {
                        if let Ok(mut state) = self.state.lock() {
                            let typed = params["text"].as_str().unwrap_or_default();
                            state.field_value = match state.reformat {
                                Some(reformat) => reformat(typed),
                                None => typed.to_string(),
                            };
                        }
                        Ok(json!({}))
                    }
                    "Input.dispatchMouseEvent" => {
                        let (mut stall, mut fail) = (false, false);
                        let mut lost_ack = false;
                        if let Ok(mut state) = self.state.lock() {
                            state.mouse_events += 1;
                            if params["type"] == "mousePressed"
                                && let Some(after) = state.check_state_after_click.take()
                            {
                                state.check_state = after;
                            }
                            if params["type"] == "mouseMoved" && params["buttons"] == 1 {
                                state.held_moves += 1;
                                stall = state.stall_held_move == Some(state.held_moves);
                                fail = state.fail_held_move == Some(state.held_moves);
                            }
                            if state
                                .fail_mouse_ack
                                .is_some_and(|kind| params["type"] == kind)
                            {
                                state.fail_mouse_ack = None;
                                lost_ack = true;
                            }
                            state.mouse_calls.push((params, Instant::now()));
                        }
                        if lost_ack {
                            return Err(CdpError::ConnectionLost);
                        }
                        if stall {
                            std::future::pending::<()>().await;
                        }
                        if fail {
                            return Err(CdpError::Protocol {
                                code: -32000,
                                message: "the held move was rejected".into(),
                            });
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

        fn send_at_epoch<'a>(
            &'a self,
            expected_epoch: u64,
            method: &'a str,
            params: Value,
            session: Option<&'a crate::SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            // This harness has one immutable epoch and cannot reconnect.
            Box::pin(async move {
                if expected_epoch != 1 {
                    return Err(CdpError::ConnectionEpochChanged {
                        expected: expected_epoch,
                        actual: 1,
                    });
                }
                self.send(method, params, session).await
            })
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
            if function.contains("browserosActivateDom") {
                if self.state.lock().is_ok_and(|state| state.fail_dom_ack) {
                    return Err(CdpError::Protocol {
                        code: -32000,
                        message: "Execution context was destroyed".into(),
                    });
                }
                return Ok(json!({"result":{"value":{}}}));
            }
            if function.contains("browserosProbeClick")
                && self.state.lock().is_ok_and(|state| {
                    state.missing_geometry
                        || (state.pointer_moved && state.missing_geometry_after_move)
                })
            {
                // A disappearing target loses its DOM client rects as well as
                // its native quads; the exact destination recheck must refuse it.
                return Ok(
                    json!({ "result": { "value": { "unavailable": "Element has no visible click area." } } }),
                );
            }
            if function.contains("elementFromPoint") {
                let response = match self.state.lock() {
                    Ok(mut state) => {
                        state.hit_test_calls += 1;
                        if state.pointer_moved {
                            state.hit_test_after_move.unwrap_or(state.hit_test)
                        } else {
                            state.hit_test
                        }
                    }
                    Err(_err) => HitTestResponse::Error,
                };
                return match response {
                    HitTestResponse::Blocked(blocker)
                        if function.contains("browserosProbeClick") =>
                    {
                        Ok(
                            json!({ "result": { "value": { "points": [], "blocker": blocker, "blockedPoint": {"x":50.0,"y":25.0} } } }),
                        )
                    }
                    HitTestResponse::Clear if function.contains("browserosProbeClick") => {
                        let moved = self
                            .state
                            .lock()
                            .is_ok_and(|state| state.pointer_moved && state.moved_box);
                        Ok(
                            json!({ "result": { "value": { "points": [{"x":if moved {250.0} else {50.0},"y":if moved {125.0} else {25.0}}], "viewport": {"width":1280.0,"height":720.0} } } }),
                        )
                    }
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
            if function.contains("aria-checked") {
                let state = self.state.lock().map_err(|_| CdpError::Protocol {
                    code: -1,
                    message: "poisoned test state".into(),
                })?;
                return Ok(json!({ "result": { "value": state.check_state } }));
            }
            if function.contains("browserosSetInputValue") {
                let mut state = self.state.lock().map_err(|_| CdpError::Protocol {
                    code: -1,
                    message: "poisoned test state".into(),
                })?;
                state
                    .set_values
                    .push(params["arguments"][0]["value"].clone());
                return Ok(json!({ "result": { "value": state.set_value_result } }));
            }
            if function.contains("browserosPrepareFill")
                && params["arguments"][0]["value"] == "validate"
                && self.state.lock().is_ok_and(|state| state.value_control)
            {
                return Ok(json!({ "result": { "value": "setValue" } }));
            }
            if function.contains("browserosPrepareFill") || function.contains("this.focus()") {
                return Ok(json!({ "result": { "value": true } }));
            }
            if function.contains("typeof this.value === 'string'") {
                let state = self.state.lock().map_err(|_| CdpError::Protocol {
                    code: -1,
                    message: "poisoned test state".into(),
                })?;
                return Ok(json!({ "result": { "value": state.field_value } }));
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
                fail_mouse_ack: None,
                native_press: NativePress::Pressed { inert: false },
                fail_dom_ack: false,
                mouse_calls: Vec::new(),
                page_scrolls: 0,
                select_calls: 0,
                select_result,
                select_semantics_preserved: false,
                calls: Vec::new(),
                pointer_response: PointerResponse::Arrived,
                pointer_moved: false,
                effect_pending: false,
                moved_box: false,
                missing_geometry: false,
                missing_geometry_after_move: false,
                hit_test_after_move: None,
                field_value: String::new(),
                reformat: None,
                tab_active: true,
                tab_info_error: false,
                tab_list_error: false,
                tab_target: "target-1",
                check_state: Value::Null,
                check_state_after_click: None,
                stall_held_move: None,
                fail_held_move: None,
                held_moves: 0,
                value_control: false,
                set_value_result: json!(true),
                set_values: Vec::new(),
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

    #[tokio::test(start_paused = true)]
    async fn tests_that_default_clicks_preserve_the_exact_mouse_events() -> Result<(), CoreError> {
        for kind in 0..3 {
            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            let started = Instant::now();
            match kind {
                0 => input.click_at(50.0, 25.0, ClickOptions::default()).await?,
                1 => assert_eq!(
                    input.click(&ref_id, ClickOptions::default()).await?,
                    Some(Point { x: 50.0, y: 25.0 })
                ),
                _ => assert_eq!(
                    input
                        .click_backend_node(10, ClickOptions::default())
                        .await?,
                    Some(Point { x: 50.0, y: 25.0 })
                ),
            }
            let calls = connection.mouse_calls()?;
            assert_eq!(
                calls
                    .iter()
                    .map(|(params, _)| params.clone())
                    .collect::<Vec<_>>(),
                vec![
                    json!({"type":"mouseMoved","x":50.0,"y":25.0}),
                    json!({"type":"mousePressed","x":50.0,"y":25.0,"button":"left","clickCount":1,"modifiers":0}),
                    json!({"type":"mouseReleased","x":50.0,"y":25.0,"button":"left","clickCount":1,"modifiers":0}),
                ]
            );
            assert_eq!(
                started.elapsed(),
                if kind == 0 {
                    Duration::ZERO
                } else {
                    Duration::from_millis(50)
                }
            );
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_humanlike_clicks_follow_the_path_for_coordinates_and_refs()
    -> Result<(), CoreError> {
        for kind in 0..3 {
            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            let opts = ClickOptions {
                button: Some(PublicMouseButton::Right),
                click_count: Some(2),
                humanlike_from: Some(Point { x: 10.0, y: 5.0 }),
            };
            match kind {
                0 => input.click_at(50.0, 25.0, opts).await?,
                1 => assert_eq!(
                    input.click(&ref_id, opts).await?,
                    Some(Point { x: 50.0, y: 25.0 })
                ),
                _ => assert_eq!(
                    input.click_backend_node(10, opts).await?,
                    Some(Point { x: 50.0, y: 25.0 })
                ),
            }
            let calls = connection.mouse_calls()?;
            assert_eq!(
                calls
                    .iter()
                    .map(|(params, _)| params.clone())
                    .collect::<Vec<_>>(),
                vec![
                    json!({"type":"mouseMoved","x":20.0,"y":10.0}),
                    json!({"type":"mouseMoved","x":30.0,"y":15.0}),
                    json!({"type":"mouseMoved","x":40.0,"y":20.0}),
                    json!({"type":"mouseMoved","x":50.0,"y":25.0}),
                    json!({"type":"mousePressed","x":50.0,"y":25.0,"button":"right","clickCount":2,"modifiers":0}),
                    json!({"type":"mouseReleased","x":50.0,"y":25.0,"button":"right","clickCount":2,"modifiers":0}),
                ]
            );
            for pair in calls[..5].windows(2) {
                assert!(pair[1].1.duration_since(pair[0].1) >= Duration::from_millis(12));
            }
            assert!(calls[5].1.duration_since(calls[4].1) >= Duration::from_millis(90));
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_the_pointer_keeps_the_humanlike_click_path_after_its_effect()
    -> Result<(), Box<dyn std::error::Error>> {
        for kind in 0..3 {
            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            connection.configure(|state| state.moved_box = true)?;
            let input = input.with_pointer(Some(Pointer::default()));
            let opts = ClickOptions {
                button: Some(PublicMouseButton::Right),
                click_count: Some(2),
                humanlike_from: Some(Point { x: 10.0, y: 5.0 }),
            };
            connection.take_calls()?;
            let path = match kind {
                0 => {
                    input.click_at(50.0, 25.0, opts).await?;
                    [(20.0, 10.0), (30.0, 15.0), (40.0, 20.0), (50.0, 25.0)]
                }
                1 => {
                    assert_eq!(
                        input.click(&ref_id, opts).await?,
                        Some(Point { x: 250.0, y: 125.0 })
                    );
                    [(70.0, 35.0), (130.0, 65.0), (190.0, 95.0), (250.0, 125.0)]
                }
                _ => {
                    assert_eq!(
                        input.click_backend_node(10, opts).await?,
                        Some(Point { x: 250.0, y: 125.0 })
                    );
                    [(70.0, 35.0), (130.0, 65.0), (190.0, 95.0), (250.0, 125.0)]
                }
            };
            let (x, y) = path[3];
            let mut expected: Vec<Value> = path
                .iter()
                .map(|(x, y)| json!({"type":"mouseMoved","x":x,"y":y}))
                .collect();
            for event in ["mousePressed", "mouseReleased"] {
                expected.push(
                    json!({"type":event,"x":x,"y":y,"button":"right","clickCount":2,"modifiers":0}),
                );
            }
            let mouse = connection.mouse_calls()?;
            assert_eq!(
                mouse
                    .iter()
                    .map(|(params, _)| params.clone())
                    .collect::<Vec<_>>(),
                expected
            );
            for pair in mouse[..5].windows(2) {
                assert!(pair[1].1.duration_since(pair[0].1) >= Duration::from_millis(12));
            }
            assert!(mouse[5].1.duration_since(mouse[4].1) >= Duration::from_millis(90));
            let calls = connection.take_calls()?;
            let movement = calls
                .iter()
                .position(is_pointer_move)
                .ok_or("missing pointer move")?;
            let effect = calls
                .iter()
                .position(is_pointer_effect)
                .ok_or("missing click effect")?;
            assert!(
                calls[effect].params["expression"]
                    .as_str()
                    .is_some_and(|expression| expression.contains(&format!(".click({x},{y})")))
            );
            let first_mouse = calls
                .iter()
                .position(|call| call.method == "Input.dispatchMouseEvent")
                .ok_or("missing mouse input")?;
            assert!(movement < effect && effect < first_mouse, "{kind}");
            // Ref clicks recheck after hover and before the press; geometry
            // reads may occur between native movement and button input.
            let press = calls
                .iter()
                .position(|call| call.params["type"] == "mousePressed")
                .ok_or("missing press")?;
            assert!(first_mouse < press);
        }
        Ok(())
    }

    #[derive(Clone, Copy, Debug)]
    enum PointerInput {
        Click,
        Hover,
        Focus,
        Fill,
        ClickAt,
        HoverAt,
    }

    const POINTER_INPUTS: [PointerInput; 6] = [
        PointerInput::Click,
        PointerInput::Hover,
        PointerInput::Focus,
        PointerInput::Fill,
        PointerInput::ClickAt,
        PointerInput::HoverAt,
    ];

    impl PointerInput {
        async fn run(self, input: &Input, ref_id: &Ref) -> Result<Option<Point>, CoreError> {
            match self {
                Self::Click => input.click(ref_id, ClickOptions::default()).await,
                Self::Hover => input.hover(ref_id).await.map(Some),
                Self::Focus => input.focus(ref_id).await.map(|()| None),
                Self::Fill => input.fill(ref_id, "s3cr3t-pa55", true).await,
                Self::ClickAt => input
                    .click_at(50.0, 25.0, ClickOptions::default())
                    .await
                    .map(|()| None),
                Self::HoverAt => input.hover_at(50.0, 25.0).await.map(|()| None),
            }
        }

        fn has_effect(self) -> bool {
            matches!(self, Self::Click | Self::Fill | Self::ClickAt)
        }
    }

    #[tokio::test]
    async fn tests_that_disabled_inputs_preserve_the_legacy_protocol()
    -> Result<(), Box<dyn std::error::Error>> {
        for action in POINTER_INPUTS {
            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            connection.take_calls()?;
            action.run(&input, &ref_id).await?;
            let calls = connection.take_calls()?;
            assert!(
                !calls.iter().any(|call| call.method == "Runtime.evaluate"),
                "{action:?}"
            );
            assert!(
                !calls.iter().any(|call| call.method == "Browser.getTabInfo"),
                "{action:?}"
            );
            assert_real_input_once(&calls, action);
        }
        Ok(())
    }

    fn is_pointer_move(call: &TestCall) -> bool {
        call.method == "Runtime.evaluate"
            && call.params["expression"]
                .as_str()
                .is_some_and(|expression| expression.contains(r#""op":"move""#))
    }

    /// The ripple is a short call into the installed pointer; a move carries the whole player,
    /// which also mentions `.click(`.
    fn is_pointer_effect(call: &TestCall) -> bool {
        call.method == "Runtime.evaluate"
            && !is_pointer_move(call)
            && call.params["expression"]
                .as_str()
                .is_some_and(|expression| expression.contains(".click("))
    }

    fn pointer_calls(calls: &[TestCall]) -> impl Iterator<Item = &TestCall> {
        calls
            .iter()
            .filter(|call| call.method == "Runtime.evaluate")
    }

    fn press_count(calls: &[TestCall]) -> usize {
        calls
            .iter()
            .filter(|call| {
                call.method == "Input.dispatchMouseEvent" && call.params["type"] == "mousePressed"
            })
            .count()
    }

    fn assert_real_input_once(calls: &[TestCall], action: PointerInput) {
        let mouse_types: Vec<_> = calls
            .iter()
            .filter(|call| call.method == "Input.dispatchMouseEvent")
            .map(|call| call.params["type"].as_str().unwrap_or_default())
            .collect();
        let expected: &[&str] = match action {
            PointerInput::Click | PointerInput::Fill | PointerInput::ClickAt => {
                &["mouseMoved", "mousePressed", "mouseReleased"]
            }
            PointerInput::Hover | PointerInput::HoverAt => &["mouseMoved"],
            PointerInput::Focus => &[],
        };
        assert_eq!(mouse_types, expected, "{action:?}");
        let focuses = calls
            .iter()
            .filter(|call| {
                call.params["functionDeclaration"]
                    .as_str()
                    .is_some_and(|function| function.contains("this.focus()"))
            })
            .count();
        assert_eq!(
            focuses,
            usize::from(matches!(action, PointerInput::Focus | PointerInput::Fill)),
            "{action:?}"
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.method == "Input.insertText")
                .count(),
            usize::from(matches!(action, PointerInput::Fill)),
            "{action:?}"
        );
    }

    #[tokio::test]
    async fn tests_that_enabled_inputs_move_before_real_input()
    -> Result<(), Box<dyn std::error::Error>> {
        for action in POINTER_INPUTS {
            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            let input = input.with_pointer(Some(Pointer::default()));
            connection.take_calls()?;
            action.run(&input, &ref_id).await?;
            let calls = connection.take_calls()?;
            assert_real_input_once(&calls, action);
            let tab_checks: Vec<_> = calls
                .iter()
                .enumerate()
                .filter(|(_, call)| call.method == "Browser.getTabInfo")
                .collect();
            assert_eq!(tab_checks.len(), 1, "{action:?}");
            assert_eq!(tab_checks[0].1.params, json!({ "tabId": 101 }));
            let movement = calls
                .iter()
                .position(is_pointer_move)
                .ok_or("missing pointer move")?;
            assert!(tab_checks[0].0 < movement, "{action:?}");
            let real_input = calls
                .iter()
                .position(|call| {
                    call.method == "Input.dispatchMouseEvent"
                        || call.params["functionDeclaration"]
                            .as_str()
                            .is_some_and(|function| function.contains("this.focus()"))
                })
                .ok_or("missing real input")?;
            assert!(movement < real_input, "{action:?}");
            let effects: Vec<_> = pointer_calls(&calls)
                .filter(|call| is_pointer_effect(call))
                .collect();
            assert_eq!(
                effects.len(),
                usize::from(action.has_effect()),
                "{action:?}"
            );
            let quads = calls
                .iter()
                .filter(|call| call.method == "DOM.getContentQuads")
                .count();
            assert_eq!(
                quads,
                match action {
                    PointerInput::Focus => 1,
                    PointerInput::ClickAt | PointerInput::HoverAt => 0,
                    PointerInput::Click => 2,
                    _ => 2,
                },
                "{action:?}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_moved_targets_use_the_fresh_center_for_effects_and_input()
    -> Result<(), Box<dyn std::error::Error>> {
        for action in [PointerInput::Click, PointerInput::Hover, PointerInput::Fill] {
            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            connection.configure(|state| state.moved_box = true)?;
            let input = input.with_pointer(Some(Pointer::default()));
            connection.take_calls()?;
            assert_eq!(
                action.run(&input, &ref_id).await?,
                Some(Point { x: 250.0, y: 125.0 })
            );
            let calls = connection.take_calls()?;
            for call in calls
                .iter()
                .filter(|call| call.method == "Input.dispatchMouseEvent")
            {
                assert_eq!(call.params["x"], 250.0);
                assert_eq!(call.params["y"], 125.0);
            }
            let hit_tests: Vec<_> = calls
                .iter()
                .filter(|call| {
                    call.params["functionDeclaration"]
                        .as_str()
                        .is_some_and(|function| function.contains("elementFromPoint"))
                })
                .collect();
            assert!(hit_tests.len() >= 2);
            if !matches!(action, PointerInput::Click) {
                assert_eq!(
                    hit_tests[1].params["arguments"],
                    json!([{ "value": 250.0 }, { "value": 125.0 }])
                );
            }
            if action.has_effect() {
                let effect = pointer_calls(&calls)
                    .find(|call| is_pointer_effect(call))
                    .ok_or("missing click effect")?;
                assert!(
                    effect.params["expression"]
                        .as_str()
                        .is_some_and(|expression| expression.contains(".click(250,125)"))
                );
                let effect_index = calls
                    .iter()
                    .position(|call| std::ptr::eq(call, effect))
                    .ok_or("missing effect call")?;
                let hit_index = calls
                    .iter()
                    .rposition(|call| {
                        call.params["functionDeclaration"]
                            .as_str()
                            .is_some_and(|function| function.contains("elementFromPoint"))
                            && !is_pointer_effect(call)
                    })
                    .ok_or("missing recheck")?;
                let mouse_index = calls
                    .iter()
                    .position(|call| call.method == "Input.dispatchMouseEvent")
                    .ok_or("missing mouse input")?;
                if matches!(action, PointerInput::Click) {
                    let press_index = calls
                        .iter()
                        .position(|call| call.params["type"] == "mousePressed")
                        .ok_or("missing press")?;
                    assert!(effect_index < mouse_index && hit_index < press_index);
                } else {
                    assert!(hit_index < effect_index && effect_index < mouse_index);
                }
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn newly_covered_targets_stop_native_input_and_only_click_can_fall_back()
    -> Result<(), Box<dyn std::error::Error>> {
        for action in [PointerInput::Click, PointerInput::Hover, PointerInput::Fill] {
            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            connection.configure(|state| {
                state.hit_test_after_move = Some(HitTestResponse::Blocked("div#overlay"))
            })?;
            let input = input.with_pointer(Some(Pointer::default()));
            connection.take_calls()?;
            let result = action.run(&input, &ref_id).await;
            if matches!(action, PointerInput::Click) {
                assert_eq!(result?, None);
            } else {
                assert!(
                    result
                        .err()
                        .ok_or("covered action succeeded")?
                        .to_string()
                        .contains("covered by <div#overlay>")
                );
            }
            let calls = connection.take_calls()?;
            assert!(
                !calls
                    .iter()
                    .any(|call| call.method == "Input.dispatchMouseEvent")
            );
            assert_eq!(pointer_calls(&calls).count(), 1);
            assert_eq!(
                native_press_count(&calls),
                usize::from(matches!(action, PointerInput::Click))
            );
            assert_eq!(dom_activation_count(&calls), 0);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_skipped_moves_preserve_legacy_input_without_effects_or_rechecks()
    -> Result<(), Box<dyn std::error::Error>> {
        for action in POINTER_INPUTS {
            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            connection.take_calls()?;
            action.run(&input, &ref_id).await?;
            let legacy = connection.take_calls()?;
            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            connection.configure(|state| state.pointer_response = PointerResponse::Skipped)?;
            let input = input.with_pointer(Some(Pointer::default()));
            connection.take_calls()?;
            action.run(&input, &ref_id).await?;
            let mut calls = connection.take_calls()?;
            assert_eq!(pointer_calls(&calls).count(), 1, "{action:?}");
            assert_eq!(
                calls
                    .iter()
                    .filter(|call| call.method == "Browser.getTabInfo")
                    .count(),
                1,
                "{action:?}"
            );
            calls.retain(|call| {
                call.method != "Runtime.evaluate" && call.method != "Browser.getTabInfo" &&
                    // Focus must measure once before asking the page to draw.
                    !(matches!(action, PointerInput::Focus) && call.method == "DOM.getContentQuads")
            });
            assert_eq!(calls, legacy, "{action:?}");
        }
        Ok(())
    }

    async fn assert_skipped_tab_uses_legacy_input(
        read_error: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for action in POINTER_INPUTS {
            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            connection.take_calls()?;
            action.run(&input, &ref_id).await?;
            let legacy = connection.take_calls()?;

            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            connection.configure(|state| {
                state.tab_active = read_error;
                state.tab_info_error = read_error;
                state.tab_list_error = read_error;
                // A hidden tab must not enter either animation deadline.
                state.pointer_response = PointerResponse::Pending;
                state.effect_pending = true;
                state.moved_box = true;
            })?;
            let input = input.with_pointer(Some(Pointer::default()));
            connection.take_calls()?;
            let started = tokio::time::Instant::now();
            action.run(&input, &ref_id).await?;
            assert_eq!(
                started.elapsed(),
                if matches!(action, PointerInput::Click) {
                    Duration::from_millis(50)
                } else {
                    Duration::ZERO
                },
                "{action:?}"
            );
            let mut calls = connection.take_calls()?;
            assert_eq!(pointer_calls(&calls).count(), 0, "{action:?}");
            assert_eq!(
                calls
                    .iter()
                    .filter(|call| call.method == "Browser.getTabInfo")
                    .count(),
                1,
                "{action:?}"
            );
            // PageManager's existing recovery may list tabs after a failed read.
            assert_eq!(
                calls
                    .iter()
                    .filter(|call| call.method == "Browser.getTabs")
                    .count(),
                usize::from(read_error),
                "{action:?}"
            );
            calls.retain(|call| {
                !matches!(
                    call.method.as_str(),
                    "Browser.getTabInfo" | "Browser.getTabs"
                )
            });
            assert_eq!(calls, legacy, "{action:?}");
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_inactive_tabs_skip_drawing_and_keep_legacy_input()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_skipped_tab_uses_legacy_input(false).await
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_failed_fresh_tab_reads_skip_drawing_and_keep_legacy_input()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_skipped_tab_uses_legacy_input(true).await
    }

    #[tokio::test]
    async fn tests_that_disabled_inputs_never_query_tab_activity()
    -> Result<(), Box<dyn std::error::Error>> {
        for action in POINTER_INPUTS {
            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            connection.configure(|state| {
                state.tab_active = false;
                state.tab_info_error = true;
                state.tab_list_error = true;
            })?;
            connection.take_calls()?;
            action.run(&input, &ref_id).await?;
            let calls = connection.take_calls()?;
            assert!(!calls.iter().any(|call| matches!(
                call.method.as_str(),
                "Browser.getTabInfo" | "Browser.getTabs"
            )));
            assert_eq!(pointer_calls(&calls).count(), 0);
            assert_real_input_once(&calls, action);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_tab_activity_is_refreshed_for_each_action()
    -> Result<(), Box<dyn std::error::Error>> {
        let (connection, input, _ref_id) = input_harness(HitTestResponse::Clear).await?;
        let input = input.with_pointer(Some(Pointer::default()));
        for active in [false, true, false] {
            connection.configure(|state| state.tab_active = active)?;
            connection.take_calls()?;
            input.hover_at(50.0, 25.0).await?;
            let calls = connection.take_calls()?;
            assert_eq!(
                calls
                    .iter()
                    .filter(|call| call.method == "Browser.getTabInfo")
                    .count(),
                1
            );
            assert_eq!(pointer_calls(&calls).count(), usize::from(active));
            assert_real_input_once(&calls, PointerInput::HoverAt);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_guarded_input_checks_activity_through_its_captured_access()
    -> Result<(), Box<dyn std::error::Error>> {
        let (connection, _input, _ref_id) = input_harness(HitTestResponse::Clear).await?;
        let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
        let page = session
            .pages
            .list()
            .await?
            .into_iter()
            .next()
            .ok_or("missing page")?;
        let view = BrowserView::unbound(session, None)
            .with_expected_created_page(&page)?
            .with_pointer(Pointer::default());
        let input = view.input(page.page_id).await?;
        assert!(input.access.is_some());
        for active in [false, true] {
            connection.configure(|state| state.tab_active = active)?;
            connection.take_calls()?;
            assert_eq!(input.drawing_pointer().await.is_some(), active);
            let calls = connection.take_calls()?;
            // The captured view also refreshes its saved page incarnation.
            assert_eq!(calls.len(), 2);
            assert!(calls.iter().all(|call| call.method == "Browser.getTabInfo"));
            assert_eq!(calls[0].params, json!({ "tabId": 101 }));
        }
        connection.configure(|state| state.tab_target = "replacement-target")?;
        assert!(input.drawing_pointer().await.is_none());
        let input = input.with_pointer(None);
        connection.take_calls()?;
        assert!(input.drawing_pointer().await.is_none());
        assert!(connection.take_calls()?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_drawing_errors_recheck_and_send_real_input_once()
    -> Result<(), Box<dyn std::error::Error>> {
        for action in POINTER_INPUTS {
            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            connection.configure(|state| {
                state.pointer_response = PointerResponse::Error;
                state.moved_box = true;
            })?;
            let input = input.with_pointer(Some(Pointer::default()));
            connection.take_calls()?;
            let point = action.run(&input, &ref_id).await?;
            if matches!(
                action,
                PointerInput::Click | PointerInput::Hover | PointerInput::Fill
            ) {
                assert_eq!(point, Some(Point { x: 250.0, y: 125.0 }));
            }
            let calls = connection.take_calls()?;
            assert_eq!(
                press_count(&calls),
                usize::from(action.has_effect()),
                "{action:?}"
            );
            assert_real_input_once(&calls, action);
            assert_eq!(
                pointer_calls(&calls)
                    .filter(|call| is_pointer_move(call))
                    .count(),
                if matches!(action, PointerInput::Click) {
                    2
                } else {
                    1
                }
            );
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_unanswered_moves_finish_at_the_deadline_and_input_runs_once()
    -> Result<(), Box<dyn std::error::Error>> {
        for action in POINTER_INPUTS {
            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            connection.configure(|state| state.pointer_response = PointerResponse::Pending)?;
            let input = input.with_pointer(Some(Pointer::default()));
            connection.take_calls()?;
            let started = tokio::time::Instant::now();
            action.run(&input, &ref_id).await?;
            assert_eq!(
                started.elapsed(),
                timeouts::POINTER_MOVE_DEADLINE
                    + if matches!(action, PointerInput::Click) {
                        Duration::from_millis(50)
                    } else {
                        Duration::ZERO
                    },
                "{action:?}"
            );
            let calls = connection.take_calls()?;
            assert_eq!(
                press_count(&calls),
                usize::from(action.has_effect()),
                "{action:?}"
            );
            assert_real_input_once(&calls, action);
            assert_eq!(
                pointer_calls(&calls)
                    .filter(|call| is_pointer_move(call))
                    .count(),
                1
            );
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_an_unanswered_effect_is_bounded_and_the_click_runs_once()
    -> Result<(), Box<dyn std::error::Error>> {
        let (connection, input, _ref_id) = input_harness(HitTestResponse::Clear).await?;
        connection.configure(|state| state.effect_pending = true)?;
        let input = input.with_pointer(Some(Pointer::default()));
        connection.take_calls()?;
        let started = tokio::time::Instant::now();
        input.click_at(12.5, 17.0, ClickOptions::default()).await?;
        assert_eq!(started.elapsed(), timeouts::POINTER_EFFECT_DEADLINE);
        assert_eq!(press_count(&connection.take_calls()?), 1);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_focus_succeeds_without_drawable_geometry()
    -> Result<(), Box<dyn std::error::Error>> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        connection.configure(|state| state.missing_geometry = true)?;
        let input = input.with_pointer(Some(Pointer::default()));
        connection.take_calls()?;
        input.focus(&ref_id).await?;
        let calls = connection.take_calls()?;
        assert_eq!(pointer_calls(&calls).count(), 0);
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.params["functionDeclaration"]
                    .as_str()
                    .is_some_and(|function| function.contains("this.focus()")))
                .count(),
            1
        );
        Ok(())
    }

    #[tokio::test]
    async fn missing_click_geometry_uses_dom_fallback_before_any_press()
    -> Result<(), Box<dyn std::error::Error>> {
        for after_move in [false, true] {
            let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
            connection.configure(|state| {
                state.missing_geometry = !after_move;
                state.missing_geometry_after_move = after_move;
            })?;
            let input = input.with_pointer(Some(Pointer::default()));
            connection.take_calls()?;
            assert_eq!(input.click(&ref_id, ClickOptions::default()).await?, None);
            assert_eq!(
                input.dom_fallback_status(),
                Some(DomFallbackStatus::Dispatched {
                    trusted: true,
                    inert: false
                })
            );
            let calls = connection.take_calls()?;
            assert_eq!(press_count(&calls), 0);
            assert_eq!(pointer_calls(&calls).count(), usize::from(after_move));
            assert_eq!(native_press_count(&calls), 1);
            assert_eq!(dom_activation_count(&calls), 0);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_fill_values_never_enter_pointer_expressions()
    -> Result<(), Box<dyn std::error::Error>> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        let input = input.with_pointer(Some(Pointer::default()));
        connection.take_calls()?;
        input.fill(&ref_id, "s3cr3t-pa55", true).await?;
        let calls = connection.take_calls()?;
        assert_eq!(pointer_calls(&calls).count(), 2);
        for call in pointer_calls(&calls) {
            assert!(
                !call.params["expression"]
                    .as_str()
                    .ok_or("missing expression")?
                    .contains("s3cr3t-pa55")
            );
        }
        let inserts: Vec<_> = calls
            .iter()
            .filter(|call| call.method == "Input.insertText")
            .collect();
        assert_eq!(inserts.len(), 1);
        assert_eq!(inserts[0].params["text"], "s3cr3t-pa55");
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_fill_leaves_a_value_the_page_reformats_for_the_caller_to_read()
    -> Result<(), Box<dyn std::error::Error>> {
        // A verification-code field that groups digits as they are typed, like Overleaf's.
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        connection.configure(|state| {
            state.reformat = Some(|typed| format!("{} {}", &typed[..3], &typed[3..]));
        })?;
        input.fill(&ref_id, "440212", true).await?;
        let mut shown = String::new();
        connection.configure(|state| shown = state.field_value.clone())?;
        assert_eq!(shown, "440 212");
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_a_zoomed_fill_clicks_the_field_where_the_preview_shows_it()
    -> Result<(), Box<dyn std::error::Error>> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        let input = input.with_pointer_scale(1.5);
        connection.take_calls()?;
        input.fill(&ref_id, "value", true).await?;
        let calls = connection.take_calls()?;
        let pressed = calls
            .iter()
            .find(|call| {
                call.method == "Input.dispatchMouseEvent" && call.params["type"] == "mousePressed"
            })
            .ok_or("no mousePressed")?;
        // The field's centre (50, 25) in CSS px, where a 150 % preview shows it.
        assert_eq!(
            (&pressed.params["x"], &pressed.params["y"]),
            (&json!(75.0), &json!(37.5))
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_browser_views_enable_pointer_input_only_on_the_configured_clone()
    -> Result<(), Box<dyn std::error::Error>> {
        let (connection, _input, _ref_id) = input_harness(HitTestResponse::Clear).await?;
        let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
        let page = session
            .pages
            .list()
            .await?
            .into_iter()
            .next()
            .ok_or("missing page")?
            .page_id;
        let view = BrowserView::unbound(session, None);
        for enabled in [false, true, false] {
            let selected = if enabled {
                view.clone().with_pointer(Pointer::default())
            } else {
                view.clone()
            };
            let input = selected.input(page.clone()).await?;
            connection.take_calls()?;
            input.click_at(50.0, 25.0, ClickOptions::default()).await?;
            let calls = connection.take_calls()?;
            assert_eq!(pointer_calls(&calls).count(), if enabled { 2 } else { 0 });
            assert_eq!(press_count(&calls), 1);
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_other_input_operations_do_not_draw_a_pointer()
    -> Result<(), Box<dyn std::error::Error>> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        let input = input.with_pointer(Some(Pointer::default()));
        connection.take_calls()?;
        input.type_at(50.0, 25.0, "text", true).await?;
        input.type_text("text").await?;
        input.press("Enter").await?;
        input.select_option(&ref_id, "Choice").await?;
        input.select_backend_node(10, "Choice").await?;
        input.drag(&ref_id, &ref_id).await?;
        input
            .drag_at(Point { x: 0.0, y: 0.0 }, Point { x: 1.0, y: 1.0 })
            .await?;
        input
            .scroll(ScrollDirection::Down, 1, Some(&ref_id))
            .await?;
        input
            .upload_file_by_ref(&ref_id, vec!["fixture.txt".into()])
            .await?;
        input.handle_dialog(true, None).await?;
        assert_eq!(pointer_calls(&connection.take_calls()?).count(), 0);
        Ok(())
    }

    fn sent<'a>(calls: &'a [TestCall], method: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
        calls
            .iter()
            .filter(move |call| call.method == method)
            .map(|call| &call.params)
    }

    #[tokio::test]
    async fn insert_text_sends_newlines_as_text_not_enter() -> Result<(), CoreError> {
        let (connection, input, _ref_id) = input_harness(HitTestResponse::Clear).await?;
        connection.take_calls()?;
        input.insert_text("line one\nline two").await?;
        let calls = connection.take_calls()?;
        let inserted: Vec<_> = sent(&calls, "Input.insertText").collect();
        assert_eq!(inserted, [&json!({ "text": "line one\nline two" })]);
        assert_eq!(sent(&calls, "Input.dispatchKeyEvent").count(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn check_clicks_an_unchecked_control_and_confirms_the_new_state() -> Result<(), CoreError>
    {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        connection.configure(|state| {
            state.check_state = json!({ "checked": false, "radio": false });
            state.check_state_after_click = Some(json!({ "checked": true, "radio": false }));
        })?;
        assert!(input.check(&ref_id).await?);
        assert_eq!(press_count(&connection.take_calls()?), 1);
        Ok(())
    }

    #[tokio::test]
    async fn check_leaves_a_switch_that_is_already_on() -> Result<(), CoreError> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        // A role=switch reports aria-checked; reading only `checked` used to click it off.
        connection.configure(|state| {
            state.check_state = json!({ "checked": true, "radio": false });
        })?;
        connection.take_calls()?;
        assert!(input.check(&ref_id).await?);
        assert_eq!(press_count(&connection.take_calls()?), 0);
        Ok(())
    }

    #[tokio::test]
    async fn uncheck_refuses_a_radio_without_clicking() -> Result<(), CoreError> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        connection.configure(|state| {
            state.check_state = json!({ "checked": true, "radio": true });
        })?;
        connection.take_calls()?;
        let error = input
            .uncheck(&ref_id)
            .await
            .err()
            .map(|error| error.to_string());
        assert!(error.is_some_and(|error| error.contains("cannot be unchecked")));
        assert_eq!(press_count(&connection.take_calls()?), 0);
        Ok(())
    }

    #[tokio::test]
    async fn check_reports_a_click_that_did_not_change_the_state() -> Result<(), CoreError> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        connection.configure(|state| {
            state.check_state = json!({ "checked": false, "radio": false });
        })?;
        let error = input
            .check(&ref_id)
            .await
            .err()
            .map(|error| error.to_string());
        assert!(error.is_some_and(|error| error.contains("did not make it checked")));
        Ok(())
    }

    #[tokio::test]
    async fn check_trusts_a_click_after_which_the_page_replaced_the_control()
    -> Result<(), CoreError> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        connection.configure(|state| {
            state.check_state = json!({ "checked": false, "radio": false });
            state.check_state_after_click =
                Some(json!({ "checked": false, "radio": false, "detached": true }));
        })?;
        connection.take_calls()?;
        assert!(input.check(&ref_id).await?);
        assert_eq!(press_count(&connection.take_calls()?), 1);
        Ok(())
    }

    #[tokio::test]
    async fn drag_holds_the_left_button_through_every_move() -> Result<(), CoreError> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        connection.take_calls()?;
        input.drag(&ref_id, &ref_id).await?;
        let calls = connection.take_calls()?;
        let mouse: Vec<&Value> = sent(&calls, "Input.dispatchMouseEvent").collect();
        let held_moves = mouse
            .iter()
            .filter(|params| params["type"] == "mouseMoved" && params["buttons"] == 1)
            .count();
        assert_eq!(held_moves, timeouts::DRAG_STEPS as usize);
        assert!(mouse.iter().all(|params| params["type"] != "mouseMoved"
            || params["buttons"] == 1
            || params["buttons"].is_null()));
        assert_eq!(
            mouse.last().map(|params| &params["type"]),
            Some(&json!("mouseReleased"))
        );
        // Chromium's own drag controller runs an HTML5 drag from these events.
        assert_eq!(sent(&calls, "Input.cancelDragging").count(), 0);
        Ok(())
    }

    /// The calls a drag's cleanup task sends, once it has had a moment to run.
    async fn cleanup_calls(connection: &HarnessConnection) -> Result<Vec<TestCall>, CoreError> {
        tokio::time::sleep(Duration::from_millis(50)).await;
        connection.take_calls()
    }

    fn ends_the_drag(calls: &[TestCall]) -> bool {
        let methods: Vec<&str> = calls
            .iter()
            .filter(|call| {
                call.method == "Input.cancelDragging"
                    || (call.method == "Input.dispatchMouseEvent"
                        && call.params["type"] == "mouseReleased"
                        && call.params["buttons"] == 0)
            })
            .map(|call| call.method.as_str())
            .collect();
        methods == ["Input.cancelDragging", "Input.dispatchMouseEvent"]
    }

    #[tokio::test]
    async fn a_drag_cancelled_mid_gesture_cancels_the_drag_and_releases_the_button()
    -> Result<(), CoreError> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        connection.configure(|state| state.stall_held_move = Some(3))?;
        connection.take_calls()?;
        let drag = tokio::time::timeout(Duration::from_secs(1), input.drag(&ref_id, &ref_id));
        assert!(drag.await.is_err(), "the stalled drag should be cancelled");
        assert!(ends_the_drag(&cleanup_calls(&connection).await?));
        Ok(())
    }

    #[tokio::test]
    async fn a_drag_that_fails_part_way_still_cancels_and_releases() -> Result<(), CoreError> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        connection.configure(|state| state.fail_held_move = Some(3))?;
        connection.take_calls()?;
        assert!(input.drag(&ref_id, &ref_id).await.is_err());
        assert!(ends_the_drag(&cleanup_calls(&connection).await?));
        Ok(())
    }

    #[tokio::test]
    async fn fill_sets_a_date_input_without_clicking_or_typing() -> Result<(), CoreError> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        connection.configure(|state| state.value_control = true)?;
        connection.take_calls()?;
        input.fill(&ref_id, "2026-10-29", true).await?;
        let calls = connection.take_calls()?;
        // A click would open the date picker; typing has no editor to land in.
        assert_eq!(press_count(&calls), 0);
        assert_eq!(sent(&calls, "Input.insertText").count(), 0);
        let set: Vec<Value> = connection
            .state
            .lock()
            .map(|state| state.set_values.clone())
            .unwrap_or_default();
        assert_eq!(set, [json!("2026-10-29")]);
        Ok(())
    }

    #[tokio::test]
    async fn fill_reports_a_value_the_input_cannot_hold() -> Result<(), CoreError> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
        connection.configure(|state| {
            state.value_control = true;
            state.set_value_result =
                json!("\"29/10/2026\" is not a valid date value. Use YYYY-MM-DD.");
        })?;
        let error = input
            .fill(&ref_id, "29/10/2026", true)
            .await
            .err()
            .map(|error| error.to_string());
        assert!(error.is_some_and(|error| error.contains("Use YYYY-MM-DD")));
        Ok(())
    }

    fn dom_activation_count(calls: &[TestCall]) -> usize {
        calls
            .iter()
            .filter(|call| {
                call.params["functionDeclaration"]
                    .as_str()
                    .is_some_and(|script| script.contains("browserosActivateDom"))
            })
            .count()
    }

    fn native_press_count(calls: &[TestCall]) -> usize {
        calls
            .iter()
            .filter(|call| call.method == "DOM.click")
            .count()
    }

    #[tokio::test(start_paused = true)]
    async fn fallback_press_goes_through_cover_and_never_replays() -> Result<(), CoreError> {
        // The native press reaches a covered target, an inert one behind a dialog included.
        for inert in [false, true] {
            let (connection, input, ref_id) =
                input_harness(HitTestResponse::Blocked("div#overlay")).await?;
            connection.configure(|state| state.native_press = NativePress::Pressed { inert })?;
            assert_eq!(input.click(&ref_id, ClickOptions::default()).await?, None);
            let calls = connection.take_calls()?;
            assert_eq!(press_count(&calls), 0);
            assert_eq!(native_press_count(&calls), 1);
            assert_eq!(dom_activation_count(&calls), 0);
            assert_eq!(
                input.dom_fallback_status(),
                Some(DomFallbackStatus::Dispatched {
                    trusted: true,
                    inert
                })
            );
        }

        // A refusal sends nothing more and keeps the native obstruction.
        let (connection, input, ref_id) =
            input_harness(HitTestResponse::Blocked("div#overlay")).await?;
        connection.configure(|state| state.native_press = NativePress::Refused("notVisible"))?;
        let result = input.click(&ref_id, ClickOptions::default()).await;
        assert!(matches!(
            result,
            Err(CoreError::ClickFallbackUnavailable { ref native, ref reason })
                if matches!(**native, CoreError::ElementCovered { .. })
                    && reason.contains("not visible")
        ));
        let calls = connection.take_calls()?;
        assert_eq!(native_press_count(&calls), 1);
        assert_eq!(dom_activation_count(&calls), 0);
        assert_eq!(input.dom_fallback_status(), None);

        // A build without DOM.click gets one untrusted script click, with a user
        // gesture so its handler can open popups and pickers.
        for fail_ack in [false, true] {
            let (connection, input, ref_id) =
                input_harness(HitTestResponse::Blocked("div#overlay")).await?;
            connection.configure(|state| {
                state.native_press = NativePress::Missing;
                state.fail_dom_ack = fail_ack;
            })?;
            let result = input.click(&ref_id, ClickOptions::default()).await;
            if fail_ack {
                assert!(matches!(result, Err(CoreError::ClickOutcomeUnknown(_))));
            } else {
                assert_eq!(result?, None);
            }
            let calls = connection.take_calls()?;
            assert_eq!(native_press_count(&calls), 1);
            assert_eq!(dom_activation_count(&calls), 1);
            assert!(calls.iter().any(|call| {
                call.params["functionDeclaration"]
                    .as_str()
                    .is_some_and(|script| script.contains("browserosActivateDom"))
                    && call.params["userGesture"] == true
            }));
            assert_eq!(
                input.dom_fallback_status(),
                Some(if fail_ack {
                    DomFallbackStatus::Started
                } else {
                    DomFallbackStatus::Dispatched {
                        trusted: false,
                        inert: false,
                    }
                })
            );
        }

        // A lost native acknowledgement is never followed by the script or a repeat.
        let (connection, input, ref_id) =
            input_harness(HitTestResponse::Blocked("div#overlay")).await?;
        connection.configure(|state| state.native_press = NativePress::Lost)?;
        let result = input.click(&ref_id, ClickOptions::default()).await;
        assert!(matches!(result, Err(CoreError::ClickOutcomeUnknown(_))));
        let calls = connection.take_calls()?;
        assert_eq!(press_count(&calls), 0);
        assert_eq!(native_press_count(&calls), 1);
        assert_eq!(dom_activation_count(&calls), 0);
        assert_eq!(
            input.dom_fallback_status(),
            Some(DomFallbackStatus::Started)
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn a_lost_press_or_release_ack_never_replays_a_click() -> Result<(), CoreError> {
        for failed_ack in ["mousePressed", "mouseReleased"] {
            for entry in 0..3 {
                let (connection, input, ref_id) = input_harness(HitTestResponse::Clear).await?;
                connection.configure(|state| state.fail_mouse_ack = Some(failed_ack))?;
                connection.take_calls()?;
                let result = match entry {
                    0 => input.click_at(50.0, 25.0, ClickOptions::default()).await,
                    1 => input
                        .click_backend_node(10, ClickOptions::default())
                        .await
                        .map(|_| ()),
                    _ => input
                        .click(&ref_id, ClickOptions::default())
                        .await
                        .map(|_| ()),
                };
                assert!(matches!(result, Err(CoreError::ClickOutcomeUnknown(_))));
                let calls = connection.take_calls()?;
                assert_eq!(press_count(&calls), 1, "{failed_ack}, entry {entry}");
                assert_eq!(native_press_count(&calls), 0);
                assert_eq!(dom_activation_count(&calls), 0);
                assert_eq!(input.dom_fallback_status(), None);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn click_reports_covered_ref_and_skips_dispatch() -> Result<(), CoreError> {
        let (connection, input, ref_id) =
            input_harness(HitTestResponse::Blocked("div#consent-banner")).await?;

        let result = input
            .click(
                &ref_id,
                ClickOptions {
                    click_count: Some(2),
                    ..Default::default()
                },
            )
            .await;

        let err = match result {
            Err(err) => err,
            Ok(other) => {
                return Err(CoreError::Message(format!(
                    "expected ElementCovered, got {other:?}"
                )));
            }
        };
        let CoreError::ClickFallbackUnavailable { native, reason } = err else {
            return Err(CoreError::Message(
                "Expected refused fallback for double click".into(),
            ));
        };
        assert!(reason.contains("repeated click"));
        match *native {
            CoreError::ElementCovered {
                target, blocker, ..
            } => {
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
        assert_eq!(dom_activation_count(&connection.take_calls()?), 0);
        Ok(())
    }

    #[tokio::test]
    async fn click_does_not_dispatch_when_hit_testing_is_unavailable() -> Result<(), CoreError> {
        let (connection, input, ref_id) = input_harness(HitTestResponse::Error).await?;

        assert!(input.click(&ref_id, ClickOptions::default()).await.is_err());
        assert_eq!(connection.mouse_events(), 0);
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
    async fn semantic_select_preserves_selection_while_click_uses_its_own_fallback()
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

        assert_eq!(input.click(&ref_id, ClickOptions::default()).await?, None);
        assert_eq!(native_press_count(&connection.take_calls()?), 1);
        assert!(connection.hit_test_calls() >= 2);
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
