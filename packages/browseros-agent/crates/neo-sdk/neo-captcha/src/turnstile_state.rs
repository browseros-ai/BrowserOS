//! What a Cloudflare Turnstile widget shows inside its own frame: its success, failure or expired
//! view, its running check, or its checkbox. The page's `cf-turnstile-response` field cannot say
//! this: a token from an earlier pass stays in it while the widget reads "Verification failed".
//! Detection reads the frame to report the widget's state, and a solve can read it to confirm
//! that a pass really happened.
//!
//! ```text
//! page ─scan_dom(frame)─▶ widget document (its closed shadow root included) ─▶ views shown
//!      ─▶ TurnstileWidgetState ─▶ ChallengeState (the page's token decides only when unreadable)
//! ```

use serde::Deserialize;

use super::{
    detect::DOM_QUERIES,
    page::CaptchaPage,
    types::{ChallengeState, FrameRef, PageError},
};

/// Lists the widget's views that are shown, as the widget's own document lays them out: the
/// frame itself may be hidden in its page (a background check), so no box size counts. It runs
/// after `DOM_QUERIES`, whose `all` reaches into the widget's closed shadow root, and is `null`
/// in any document that is not a Turnstile widget's. Cloudflare's markup changes, so each view
/// is found by its element id or by its English words; a view neither finds reads as nothing.
const VIEWS: &str = r#"(() => {
    if (location.hostname !== 'challenges.cloudflare.com') return null;
    const shown = e => {
        if (!e || !e.isConnected) return false;
        for (let p = e; p; p = p.parentElement || p.getRootNode().host) {
            const s = getComputedStyle(p);
            if (s.display === 'none' || s.visibility === 'hidden' || s.visibility === 'collapse') return false;
        }
        return true;
    };
    const showing = selector => all(selector).some(shown);
    const text = all('*').filter(shown)
        .flatMap(e => [...e.childNodes].filter(n => n.nodeType === 3).map(n => n.nodeValue))
        .join(' ');
    const views = [];
    if (showing('#fail, #challenge-error') || /verification failed|\bfail(ed|ure)\b|\berror\b/i.test(text)) views.push('failed');
    if (showing('#expired, #timeout') || /\bexpired\b|timed out/i.test(text)) views.push('expired');
    if (showing('#verifying') || /\bverifying\b/i.test(text)) views.push('verifying');
    if (showing('input[type="checkbox"]')) views.push('checkbox');
    if (showing('#success') || /\bsuccess\b/i.test(text)) views.push('success');
    return views;
})()"#;

/// The widget's state as its own frame shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnstileWidgetState {
    /// Its success view: the check passed.
    Success,
    /// Its failure or error view ("Verification failed", "Error"). A token in the page is stale.
    Failed,
    /// Its expired or timed-out view, with a refresh link. A token in the page is stale.
    Expired,
    /// Its running check ("Verifying…"): no answer yet.
    Verifying,
    /// Its checkbox, waiting for a person.
    Interactive,
    /// Nothing above could be read: the frame failed, is no widget's, or shows markup this
    /// reading does not know.
    Unreadable,
}

impl TurnstileWidgetState {
    /// What detection reports for a Turnstile whose frame shows `self`. `page` is what the
    /// page's own response field said (`Passed` when it holds a token); it decides only when the
    /// frame is unreadable, since a token outlives the pass that made it.
    pub(crate) fn challenge_state(self, page: ChallengeState) -> ChallengeState {
        match self {
            Self::Success => ChallengeState::Passed,
            Self::Failed | Self::Expired | Self::Interactive => ChallengeState::Active,
            Self::Verifying => ChallengeState::Pending,
            Self::Unreadable if page == ChallengeState::Passed => ChallengeState::Passed,
            Self::Unreadable => ChallengeState::Active,
        }
    }

    /// The widget is checking, failed or expired, which holds the page's form back even while
    /// the page keeps the widget hidden (a background check does), so it counts there too.
    pub(crate) fn holds_the_page(self) -> bool {
        match self {
            Self::Failed | Self::Expired | Self::Verifying => true,
            Self::Success | Self::Interactive | Self::Unreadable => false,
        }
    }
}

/// One view of the widget that `VIEWS` reports shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum View {
    Success,
    Failed,
    Expired,
    Verifying,
    Checkbox,
}

/// The state the shown views add up to. A view that holds the page outranks one that passes:
/// two views at once are a widget between states, and it has passed only once success alone
/// shows.
fn state_of(views: &[View]) -> TurnstileWidgetState {
    [
        (View::Failed, TurnstileWidgetState::Failed),
        (View::Expired, TurnstileWidgetState::Expired),
        (View::Verifying, TurnstileWidgetState::Verifying),
        (View::Checkbox, TurnstileWidgetState::Interactive),
        (View::Success, TurnstileWidgetState::Success),
    ]
    .into_iter()
    .find(|(view, _)| views.contains(view))
    .map_or(TurnstileWidgetState::Unreadable, |(_, state)| state)
}

/// The DOM function `CaptchaPage::scan_dom` runs in the widget's frame.
fn script() -> String {
    format!(
        "function(closedRoots, truncated) {{ /* captcha:turnstile-state */ {DOM_QUERIES} return {VIEWS}; }}"
    )
}

/// Reads the Turnstile widget in `frame` (a `challenges.cloudflare.com` frame) from inside it.
/// A frame that fails or answers with something else is `Unreadable`, for the caller to fall
/// back on the page's token; only a closed page, a navigation or a cancel is an error.
pub(crate) async fn turnstile_widget_state<P: CaptchaPage>(
    page: &P,
    frame: &FrameRef,
) -> Result<TurnstileWidgetState, PageError> {
    match page.scan_dom(Some(frame), &script()).await {
        Ok(value) => Ok(serde_json::from_value::<Option<Vec<View>>>(value)
            .ok()
            .flatten()
            .map_or(TurnstileWidgetState::Unreadable, |views| state_of(&views))),
        Err(error @ (PageError::Closed | PageError::Navigated | PageError::Cancelled)) => {
            Err(error)
        }
        Err(PageError::Timeout | PageError::Browser(_)) => Ok(TurnstileWidgetState::Unreadable),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use serde_json::{Value, json};

    use super::*;
    use crate::types::{DocumentId, Rect};

    /// Answers `scan_dom` from a queue and records the frame and script it was asked for.
    #[derive(Default)]
    struct WidgetPage {
        answers: Mutex<VecDeque<Result<Value, PageError>>>,
        asked: Mutex<Vec<(Option<String>, String)>>,
    }
    impl WidgetPage {
        fn answering(answer: Result<Value, PageError>) -> Self {
            let page = Self::default();
            if let Ok(mut answers) = page.answers.lock() {
                answers.push_back(answer);
            }
            page
        }
    }
    impl CaptchaPage for WidgetPage {
        async fn eval_main(&self, _: &str) -> Result<Value, PageError> {
            panic!("the widget is read inside its frame")
        }
        async fn frames(&self) -> Result<Vec<FrameRef>, PageError> {
            panic!("the reading takes its frame from the caller")
        }
        async fn eval_in_frame(&self, _: &FrameRef, _: &str) -> Result<Value, PageError> {
            panic!("the reading goes through scan_dom, which reaches closed shadow roots")
        }
        async fn scan_dom(
            &self,
            frame: Option<&FrameRef>,
            function: &str,
        ) -> Result<Value, PageError> {
            self.asked
                .lock()
                .map_err(|error| PageError::Browser(error.to_string()))?
                .push((frame.map(|f| f.frame_id.clone()), function.to_owned()));
            self.answers
                .lock()
                .map_err(|error| PageError::Browser(error.to_string()))?
                .pop_front()
                .unwrap_or(Ok(Value::Null))
        }
        async fn click_at(&self, _: f64, _: f64) -> Result<(), PageError> {
            panic!("reading the widget never clicks")
        }
        async fn screenshot_clip(&self, _: Rect) -> Result<String, PageError> {
            panic!("reading the widget never captures")
        }
        async fn document(&self) -> Result<DocumentId, PageError> {
            panic!("reading the widget needs no document identity")
        }
        async fn query_rect_in_frame(
            &self,
            _: &FrameRef,
            _: &str,
        ) -> Result<Option<Rect>, PageError> {
            panic!("reading the widget measures nothing")
        }
    }

    fn widget_frame() -> FrameRef {
        FrameRef {
            frame_id: "cf".into(),
            url: "https://challenges.cloudflare.com/cdn-cgi/challenge-platform/h/g/turnstile/if/ov2/av0/rcv/id/0x4AAAA/light/normal".into(),
            rect: None,
        }
    }

    #[test]
    fn tests_that_a_view_holding_the_page_outranks_success() {
        use TurnstileWidgetState as S;
        for (views, state) in [
            (vec![View::Success], S::Success),
            (vec![View::Failed], S::Failed),
            (vec![View::Expired], S::Expired),
            (vec![View::Verifying], S::Verifying),
            (vec![View::Checkbox], S::Interactive),
            (vec![], S::Unreadable),
            (vec![View::Success, View::Failed], S::Failed),
            (vec![View::Success, View::Verifying], S::Verifying),
            (vec![View::Checkbox, View::Success], S::Interactive),
            (vec![View::Expired, View::Verifying], S::Expired),
            (vec![View::Checkbox, View::Failed], S::Failed),
        ] {
            assert_eq!(state_of(&views), state, "{views:?}");
        }
    }

    /// The frame outranks the page's token; the token decides only when the frame is unreadable.
    #[test]
    fn tests_that_the_frame_decides_over_a_stale_token() {
        use ChallengeState::{Active, Passed, Pending, Unknown, WidgetOnly};
        use TurnstileWidgetState as S;
        for page in [Passed, Active, Unknown, WidgetOnly] {
            assert_eq!(S::Failed.challenge_state(page), Active, "{page:?}");
            assert_eq!(S::Expired.challenge_state(page), Active, "{page:?}");
            assert_eq!(S::Interactive.challenge_state(page), Active, "{page:?}");
            assert_eq!(S::Verifying.challenge_state(page), Pending, "{page:?}");
            assert_eq!(S::Success.challenge_state(page), Passed, "{page:?}");
        }
        assert_eq!(S::Unreadable.challenge_state(Passed), Passed);
        for page in [Active, Unknown, WidgetOnly] {
            assert_eq!(S::Unreadable.challenge_state(page), Active, "{page:?}");
        }
    }

    #[test]
    fn tests_that_only_checking_failed_or_expired_widgets_hold_a_hidden_frame() {
        use TurnstileWidgetState as S;
        for (state, holds) in [
            (S::Failed, true),
            (S::Expired, true),
            (S::Verifying, true),
            (S::Success, false),
            (S::Interactive, false),
            (S::Unreadable, false),
        ] {
            assert_eq!(state.holds_the_page(), holds, "{state:?}");
        }
    }

    #[tokio::test]
    async fn tests_that_the_reading_runs_in_the_widget_frame_and_decodes_its_views()
    -> Result<(), PageError> {
        for (answer, state) in [
            (Ok(json!(["failed"])), TurnstileWidgetState::Failed),
            (Ok(json!(["verifying"])), TurnstileWidgetState::Verifying),
            (Ok(json!(["success"])), TurnstileWidgetState::Success),
            (Ok(json!([])), TurnstileWidgetState::Unreadable),
            // Not a widget document, or an answer this reading does not know.
            (Ok(Value::Null), TurnstileWidgetState::Unreadable),
            (Ok(json!(["sparkles"])), TurnstileWidgetState::Unreadable),
            (Ok(json!({"views": []})), TurnstileWidgetState::Unreadable),
            (
                Err(PageError::Browser("frame detached".into())),
                TurnstileWidgetState::Unreadable,
            ),
            (Err(PageError::Timeout), TurnstileWidgetState::Unreadable),
        ] {
            let page = WidgetPage::answering(answer.clone());
            assert_eq!(
                turnstile_widget_state(&page, &widget_frame()).await?,
                state,
                "{answer:?}"
            );
            let asked = page
                .asked
                .lock()
                .map_err(|error| PageError::Browser(error.to_string()))?
                .clone();
            assert_eq!(asked.len(), 1);
            assert_eq!(asked[0].0.as_deref(), Some("cf"));
            assert!(asked[0].1.contains("captcha:turnstile-state"));
            assert!(asked[0].1.contains("closedRoots"));
        }
        for error in [
            PageError::Closed,
            PageError::Navigated,
            PageError::Cancelled,
        ] {
            let page = WidgetPage::answering(Err(error.clone()));
            assert_eq!(
                turnstile_widget_state(&page, &widget_frame()).await,
                Err(error)
            );
        }
        Ok(())
    }

    /// A no-browser `run` tool context: the reading's JavaScript runs against a fake DOM.
    async fn script_context() -> browseros_mcp::framework::ToolCtx {
        use browseros_mcp::framework::{BrowserToolOptions, ToolCtx};
        use futures_util::future::BoxFuture;

        #[derive(Default)]
        struct NoBrowser {
            targeted: browseros_cdp::TargetedEvents,
        }
        impl browseros_core::CdpConnection for NoBrowser {
            fn send<'a>(
                &'a self,
                _: &'a str,
                _: Value,
                _: Option<&'a browseros_core::SessionId>,
            ) -> BoxFuture<'a, Result<Value, browseros_cdp::CdpError>> {
                Box::pin(async { Err(browseros_cdp::CdpError::NotConnected) })
            }
            fn send_raw_json<'a>(
                &'a self,
                _: &'a str,
                _: &'a str,
                _: Option<&'a browseros_core::SessionId>,
            ) -> BoxFuture<'a, Result<String, browseros_cdp::CdpError>> {
                Box::pin(async { Err(browseros_cdp::CdpError::NotConnected) })
            }
            fn events(&self) -> tokio::sync::broadcast::Receiver<browseros_cdp::CdpEvent> {
                tokio::sync::broadcast::channel(1).1
            }
            fn events_targeted(
                &self,
                method: &str,
            ) -> tokio::sync::mpsc::UnboundedReceiver<browseros_cdp::CdpEvent> {
                self.targeted.subscribe(method)
            }
            fn is_connected(&self) -> bool {
                true
            }
            fn connection_epoch(&self) -> u64 {
                1
            }
        }
        let browser = browseros_core::Browser::new(std::sync::Arc::new(NoBrowser::default()));
        ToolCtx::new(BrowserToolOptions {
            browser: browser.view().await,
            target: None,
            defaults: Default::default(),
            cancel: tokio_util::sync::CancellationToken::new(),
            output_files: browseros_mcp::output_file::create_browser_output_file_access(),
            inner_call_hook: None,
            preloaded_helpers: vec![],
        })
    }

    /// A small DOM for the reading: elements with an id, a type, a style and their own text; a
    /// closed shadow root is reachable only through `closedRoots`, as in the adapter.
    const FAKE_DOM: &str = r"
        const style = new Map();
        const el = (tag, attrs = {}, kids = [], text = '') => {
            const e = { tagName: tag, attrs, kids, isConnected: true, parentElement: null,
                shadowRoot: null, root: null,
                childNodes: text ? [{ nodeType: 3, nodeValue: text }] : [],
                getRootNode() { return this.root; } };
            for (const k of kids) k.parentElement = e;
            return e;
        };
        const matches = (e, selector) => selector.split(',').map(s => s.trim()).some(s =>
            s === '*' || (s.startsWith('#') && e.attrs.id === s.slice(1))
            || (s === 'input[type=\x22checkbox\x22]' && e.tagName === 'INPUT' && e.attrs.type === 'checkbox'));
        const tree = (root, kids) => {
            const all = [];
            const walk = e => { e.root = root; all.push(e); e.kids.forEach(walk); };
            kids.forEach(walk);
            root.querySelectorAll = selector => all.filter(e => matches(e, selector));
            return root;
        };
        const getComputedStyle = e => ({ display: 'block', visibility: 'visible', ...(style.get(e) || {}) });
    ";

    /// Runs the reading in a fake widget document: `body` builds `page` (the document's
    /// elements), `shadow` (a closed shadow root's) and `hidden` (elements styled out), all
    /// JavaScript over `FAKE_DOM`.
    async fn read_views(host: &str, body: &str) -> anyhow::Result<Value> {
        use browseros_mcp::framework::execute_tool;

        let code = format!(
            "{FAKE_DOM}
            const location = {{ hostname: {host} }};
            {body}
            const shadowHost = el('DIV', {{ id: 'host' }});
            const document = tree({{}}, [...page, shadowHost]);
            const closed = tree({{ host: shadowHost }}, shadow);
            for (const e of hidden) style.set(e, {{ display: 'none' }});
            return ({script})([closed], false);",
            host = json!(host),
            script = script(),
        );
        let result = execute_tool(
            &browseros_mcp::tools::run::definition(),
            json!({"code": code}),
            &script_context().await,
        )
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
        let output = result
            .structured_content
            .ok_or_else(|| anyhow::anyhow!("missing script result"))?;
        anyhow::ensure!(output["ok"] == true, "the reading threw: {output}");
        Ok(output["value"].clone())
    }

    #[tokio::test]
    async fn tests_that_the_reading_lists_only_shown_views_in_the_closed_root() -> anyhow::Result<()>
    {
        const WIDGET: &str = "challenges.cloudflare.com";
        for (body, views) in [
            // Cloudflare's views keep their element ids; the hidden ones are styled out.
            (
                "const fail = el('DIV', { id: 'fail' }, [el('SPAN', {}, [], 'Verification failed')]);
                 const success = el('DIV', { id: 'success' }, [el('SPAN', {}, [], 'Success!')]);
                 const page = []; const shadow = [fail, success]; const hidden = [success];",
                json!(["failed"]),
            ),
            (
                "const verifying = el('DIV', { id: 'verifying' }, [el('SPAN', {}, [], 'Verifying...')]);
                 const fail = el('DIV', { id: 'fail' }, [el('SPAN', {}, [], 'Verification failed')]);
                 const page = []; const shadow = [verifying, fail]; const hidden = [fail];",
                json!(["verifying"]),
            ),
            (
                "const box = el('LABEL', {}, [el('INPUT', { type: 'checkbox' }), el('SPAN', {}, [], 'Verify you are human')]);
                 const page = []; const shadow = [box]; const hidden = [];",
                json!(["checkbox"]),
            ),
            // Without the ids, the shown words still name the view.
            (
                "const page = [el('P', {}, [], 'Success!')]; const shadow = []; const hidden = [];",
                json!(["success"]),
            ),
            (
                "const old = el('P', {}, [], 'Verification expired');
                 const page = [old, el('A', {}, [], 'Refresh')]; const shadow = []; const hidden = [];",
                json!(["expired"]),
            ),
            (
                "const page = [el('P', {}, [], 'Error')]; const shadow = []; const hidden = [];",
                json!(["failed"]),
            ),
            // Words inside a hidden parent are not shown.
            (
                "const wrap = el('DIV', {}, [el('P', {}, [], 'Verification failed')]);
                 const page = [wrap, el('P', {}, [], 'Success!')]; const shadow = []; const hidden = [wrap];",
                json!(["success"]),
            ),
            (
                "const page = [el('P', {}, [], 'Cloudflare')]; const shadow = []; const hidden = [];",
                json!([]),
            ),
        ] {
            assert_eq!(read_views(WIDGET, body).await?, views, "{body}");
        }
        let other = read_views(
            "fixture.test",
            "const page = [el('P', {}, [], 'Verification failed')]; const shadow = []; const hidden = [];",
        )
        .await?;
        assert_eq!(other, Value::Null, "only a widget document is read");
        Ok(())
    }
}
