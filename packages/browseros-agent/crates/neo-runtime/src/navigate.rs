//! Main-frame navigation and title policy. CDP lives in `page.rs`; this seam keeps event
//! ordering, lifecycle waits and cancellation testable without a browser or input turns.

use neo_runtime_spec::serve::{FrameGotoParams, FrameGotoResult, FrameTitleResult, LifecycleEvent};
use url::Url;

use crate::click::PageError;
use crate::progress::{CallError, PlaywrightError, Progress};

/// Validated navigation options. Unsupported lifecycle modes fail before preparation or logs.
#[derive(Debug)]
pub(crate) struct GotoOptions {
    pub(crate) url: String,
    pub(crate) wait_until: LifecycleEvent,
    pub(crate) referer: Option<String>,
}

impl GotoOptions {
    pub(crate) fn from_params(params: FrameGotoParams) -> Result<Self, PlaywrightError> {
        if params.wait_until == Some(LifecycleEvent::Networkidle) {
            return Err(PlaywrightError::error(
                "Neo does not support waitUntil: \"networkidle\" yet",
            ));
        }
        Ok(Self {
            url: params.url,
            wait_until: params.wait_until.unwrap_or(LifecycleEvent::Load),
            referer: params.referer,
        })
    }
}

/// One committed main-frame navigation; `None` identifies a same-document commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Navigation {
    pub(crate) document: Option<String>,
    pub(crate) url: String,
}

/// The current document's lifecycle, sampled by an observation without consuming commits.
#[derive(Clone, Debug, Default)]
pub(crate) struct NavigationState {
    pub(crate) pending_url: Option<String>,
    pub(crate) dom_content_loaded: bool,
    pub(crate) load: bool,
}

/// Subscribed before `navigate`: commits must remain queued while its reply is in flight.
pub(crate) trait Observation: Send {
    fn state(&self) -> NavigationState;
    async fn next(&mut self) -> Result<Option<Navigation>, PageError>;
}

/// Mechanism supplied by a page, independently of click's `Tab` and its input lock.
pub(crate) trait NavigationTab: Sync {
    type Observation: Observation;

    async fn prepare(&self) -> Result<(), PageError>;
    fn observe(&self) -> Result<Self::Observation, PageError>;
    async fn navigate(
        &self,
        url: &str,
        referer: Option<&str>,
    ) -> Result<NavigateReply, PlaywrightError>;
    async fn read_title(&self) -> Result<String, PageError>;
}

/// The relevant fields of `Page.navigate`'s reply.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NavigateReply {
    pub(crate) loader_id: Option<String>,
    pub(crate) error_text: Option<String>,
    #[serde(default)]
    pub(crate) is_download: bool,
}

pub(crate) async fn goto(
    tab: &impl NavigationTab,
    options: &GotoOptions,
    progress: &Progress,
) -> Result<FrameGotoResult, CallError> {
    // server/frames.ts:690-753: normalize before logging, completeUserURL only afterwards.
    let url = Url::parse(&options.url).map_or_else(|_| options.url.clone(), |url| url.to_string());
    let wait_until = match options.wait_until {
        LifecycleEvent::Commit => "commit",
        LifecycleEvent::Domcontentloaded => "domcontentloaded",
        LifecycleEvent::Load => "load",
        LifecycleEvent::Networkidle => {
            return Err(PlaywrightError::error(
                "Neo does not support waitUntil: \"networkidle\" yet",
            )
            .into());
        }
    };
    progress.log(format!(
        "navigating to \"{url}\", waiting until \"{wait_until}\""
    ));
    let url = if url.starts_with("localhost") || url.starts_with("127.0.0.1") {
        format!("http://{url}")
    } else {
        url
    };
    progress.race(tab.prepare()).await?.map_err(page_error)?;
    let mut events = tab.observe().map_err(page_error)?;
    // The subscription outlives the request: commits can arrive before its acknowledgement.
    let reply = progress
        .race(tab.navigate(&url, options.referer.as_deref()))
        .await??;
    if reply.is_download {
        return Err(PlaywrightError::error("Download is starting").into());
    }
    if let Some(error) = reply.error_text.filter(|error| !error.is_empty()) {
        return Err(PlaywrightError::error(format!("{error} at {url}")).into());
    }
    let document = reply.loader_id.filter(|id| !id.is_empty());
    loop {
        let Some(navigation) = progress.race(events.next()).await?.map_err(page_error)? else {
            continue;
        };
        match (&document, &navigation.document) {
            (Some(expected), Some(actual)) if expected != actual => {
                return Err(PlaywrightError::error(format!(
                    "Navigation to \"{url}\" is interrupted by another navigation to \"{}\"",
                    navigation.url,
                ))
                .into());
            }
            (Some(_), Some(_)) | (None, None) => break,
            _ => {}
        }
    }
    loop {
        let state = events.state();
        let done = match options.wait_until {
            LifecycleEvent::Commit => true,
            LifecycleEvent::Domcontentloaded => state.dom_content_loaded,
            LifecycleEvent::Load => state.load,
            LifecycleEvent::Networkidle => false,
        };
        if done {
            return Ok(FrameGotoResult { response: None });
        }
        progress.race(events.next()).await?.map_err(page_error)?;
    }
}

pub(crate) async fn title(
    tab: &impl NavigationTab,
    progress: &Progress,
) -> Result<FrameTitleResult, CallError> {
    // server/frames.ts:1711-1727: navigation can stall evaluation indefinitely. Observe it
    // while evaluating, so a pending navigation returns its URL without waiting for the page.
    progress.race(tab.prepare()).await?.map_err(page_error)?;
    let mut events = tab.observe().map_err(page_error)?;
    for attempt in 0..2 {
        if let Some(url) = events.state().pending_url {
            return Ok(FrameTitleResult {
                value: format!("Loading {url}"),
            });
        }
        let result = progress.race(async {
            tokio::select! {
                result = tab.read_title() => result,
                pending = pending_url(&mut events) => pending.map(|url| format!("Loading {url}")),
            }
        }).await?;
        match result {
            Ok(value) => return Ok(FrameTitleResult { value }),
            Err(PageError::NotConnected) if attempt == 0 => {}
            Err(_) => break,
        }
    }
    Ok(FrameTitleResult {
        value: events
            .state()
            .pending_url
            .map_or_else(String::new, |url| format!("Loading {url}")),
    })
}

async fn pending_url(events: &mut impl Observation) -> Result<String, PageError> {
    loop {
        if let Some(url) = events.state().pending_url {
            return Ok(url);
        }
        events.next().await?;
    }
}

fn page_error(error: PageError) -> PlaywrightError {
    match error {
        PageError::Closed => PlaywrightError::target_closed(),
        PageError::Threw(message) | PageError::Contract(message) => PlaywrightError::error(message),
        PageError::NotConnected => PlaywrightError::error(
            "Execution context was destroyed, most likely because of a navigation",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::future;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use serde_json::json;
    use tokio::time::{self, Instant};
    use tokio_util::sync::CancellationToken;
    use tokio_util::task::TaskTracker;

    use super::*;
    use crate::deadline::Deadline;
    use crate::progress::{CallEntry, CallStart, RunScope};
    use crate::run::{CallResult, RunEnd};

    struct Step {
        at: u64,
        navigation: Option<Navigation>,
        state: NavigationState,
    }

    struct Events {
        start: Instant,
        state: NavigationState,
        steps: VecDeque<Step>,
    }

    impl Observation for Events {
        fn state(&self) -> NavigationState {
            self.state.clone()
        }

        async fn next(&mut self) -> Result<Option<Navigation>, PageError> {
            let Some(step) = self.steps.front() else {
                return future::pending().await;
            };
            time::sleep_until(self.start + Duration::from_millis(step.at)).await;
            let step = self.steps.pop_front().expect("the awaited step");
            self.state = step.state;
            Ok(step.navigation)
        }
    }

    struct FakeTab {
        steps: Mutex<Option<VecDeque<Step>>>,
        initial: NavigationState,
        reply: Result<NavigateReply, PlaywrightError>,
        reply_after: u64,
        titles: Mutex<VecDeque<Result<String, PageError>>>,
        title_after: u64,
        requests: Mutex<Vec<(String, Option<String>)>>,
    }

    impl FakeTab {
        fn new(steps: impl IntoIterator<Item = Step>) -> Self {
            Self {
                steps: Mutex::new(Some(steps.into_iter().collect())),
                initial: NavigationState::default(),
                reply: Ok(NavigateReply {
                    loader_id: Some("new".to_string()),
                    ..NavigateReply::default()
                }),
                reply_after: 5,
                titles: Mutex::new([Ok("Example".to_string())].into()),
                title_after: 0,
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl NavigationTab for FakeTab {
        type Observation = Events;

        async fn prepare(&self) -> Result<(), PageError> {
            Ok(())
        }

        fn observe(&self) -> Result<Events, PageError> {
            Ok(Events {
                start: Instant::now(),
                state: self.initial.clone(),
                steps: self
                    .steps
                    .lock()
                    .expect("steps")
                    .take()
                    .expect("one observer"),
            })
        }

        async fn navigate(
            &self,
            url: &str,
            referer: Option<&str>,
        ) -> Result<NavigateReply, PlaywrightError> {
            self.requests
                .lock()
                .expect("requests")
                .push((url.to_string(), referer.map(str::to_string)));
            time::sleep(Duration::from_millis(self.reply_after)).await;
            self.reply.clone()
        }

        async fn read_title(&self) -> Result<String, PageError> {
            time::sleep(Duration::from_millis(self.title_after)).await;
            self.titles
                .lock()
                .expect("titles")
                .pop_front()
                .expect("scripted title")
        }
    }

    fn commit(at: u64, document: Option<&str>) -> Step {
        Step {
            at,
            navigation: Some(Navigation {
                document: document.map(str::to_string),
                url: "https://other.test/".to_string(),
            }),
            state: NavigationState::default(),
        }
    }

    fn lifecycle(at: u64, dom: bool, load: bool) -> Step {
        Step {
            at,
            navigation: None,
            state: NavigationState {
                dom_content_loaded: dom,
                load,
                ..NavigationState::default()
            },
        }
    }

    fn options(wait_until: LifecycleEvent) -> GotoOptions {
        GotoOptions {
            url: "https://EXAMPLE.test".to_string(),
            wait_until,
            referer: None,
        }
    }

    fn progress(timeout: f64) -> (Progress, Arc<CallEntry>, RunScope) {
        let entry = Arc::new(CallEntry::new(CallStart {
            call_id: 1,
            guid: "frame@T".to_string(),
            method: "goto".to_string(),
            title: Some("Navigate"),
            params: json!({}),
            location: None,
            input: false,
        }));
        let run = RunScope::new(
            Deadline::new(Duration::from_mins(1), CancellationToken::new()),
            TaskTracker::new(),
        );
        (
            Progress::new(
                Arc::clone(&entry),
                timeout,
                run.clone(),
                CancellationToken::new(),
            ),
            entry,
            run,
        )
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_commits_before_and_after_the_reply_are_observed() {
        for at in [1, 10] {
            let tab = FakeTab::new([commit(at, Some("new"))]);
            let (progress, _, _) = progress(100.0);
            assert_eq!(
                goto(&tab, &options(LifecycleEvent::Commit), &progress).await,
                Ok(FrameGotoResult { response: None })
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_each_supported_wait_until_waits_for_its_event() {
        for (wait, expected) in [
            (LifecycleEvent::Commit, 5),
            (LifecycleEvent::Domcontentloaded, 10),
            (LifecycleEvent::Load, 20),
        ] {
            let tab = FakeTab::new([
                commit(1, Some("new")),
                lifecycle(10, true, false),
                lifecycle(20, true, true),
            ]);
            let (progress, _, _) = progress(100.0);
            let start = Instant::now();
            assert!(goto(&tab, &options(wait), &progress).await.is_ok());
            assert_eq!(start.elapsed(), Duration::from_millis(expected));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_same_document_navigation_keeps_the_loaded_lifecycle() {
        let mut step = commit(1, None);
        step.state = NavigationState {
            dom_content_loaded: true,
            load: true,
            ..NavigationState::default()
        };
        let mut tab = FakeTab::new([step]);
        tab.reply = Ok(NavigateReply::default());
        let (progress, _, _) = progress(100.0);
        assert!(
            goto(&tab, &options(LifecycleEvent::Load), &progress)
                .await
                .is_ok()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_an_intervening_commit_interrupts_navigation() {
        let tab = FakeTab::new([commit(1, Some("other")), commit(2, Some("new"))]);
        let (progress, _, _) = progress(100.0);
        assert_eq!(goto(&tab, &options(LifecycleEvent::Commit), &progress).await,
            Err(PlaywrightError::error("Navigation to \"https://example.test/\" is interrupted by another navigation to \"https://other.test/\"").into()));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_navigation_failures_keep_playwright_messages() {
        for (reply, expected) in [
            (
                Ok(NavigateReply {
                    is_download: true,
                    ..NavigateReply::default()
                }),
                "Download is starting",
            ),
            (
                Ok(NavigateReply {
                    error_text: Some("net::ERR_NAME_NOT_RESOLVED".to_string()),
                    ..NavigateReply::default()
                }),
                "net::ERR_NAME_NOT_RESOLVED at https://example.test/",
            ),
            (
                Err(PlaywrightError::error(
                    "Protocol error (Page.navigate): Cannot navigate to invalid URL",
                )),
                "Protocol error (Page.navigate): Cannot navigate to invalid URL",
            ),
        ] {
            let mut tab = FakeTab::new([]);
            tab.reply = reply;
            let (progress, _, _) = progress(100.0);
            assert_eq!(
                goto(&tab, &options(LifecycleEvent::Load), &progress).await,
                Err(PlaywrightError::error(expected).into())
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_timeout_and_log_match_playwright() {
        for steps in [vec![], vec![commit(1, Some("new"))]] {
            let tab = FakeTab::new(steps);
            let (progress, entry, _) = progress(30.0);
            assert_eq!(
                goto(&tab, &options(LifecycleEvent::Load), &progress).await,
                Err(PlaywrightError::timeout(30.0).into())
            );
            assert_eq!(
                entry.finish(CallResult::Ok),
                ["  - navigating to \"https://example.test/\", waiting until \"load\""]
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_the_run_can_end_mid_wait() {
        let tab = FakeTab::new([]);
        let (progress, _, run) = progress(0.0);
        let end = async {
            time::sleep(Duration::from_millis(10)).await;
            run.end(RunEnd::Cancelled);
        };
        let options = options(LifecycleEvent::Load);
        let (result, ()) = tokio::join!(goto(&tab, &options, &progress), end);
        assert_eq!(result, Err(CallError::RunEnded(RunEnd::Cancelled)));
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_localhost_completion_happens_after_logging_and_preserves_referer() {
        let tab = FakeTab::new([commit(1, Some("new"))]);
        let (progress, entry, _) = progress(100.0);
        let mut options = options(LifecycleEvent::Commit);
        options.url = "localhost:1234/path".to_string();
        options.referer = Some("https://from.test/".to_string());
        assert!(goto(&tab, &options, &progress).await.is_ok());
        assert_eq!(
            *tab.requests.lock().expect("requests"),
            [("http://localhost:1234/path".to_string(), options.referer)]
        );
        assert_eq!(
            entry.finish(CallResult::Ok),
            ["  - navigating to \"localhost:1234/path\", waiting until \"commit\""]
        );
    }

    #[test]
    fn tests_that_networkidle_is_refused_before_any_work() {
        assert_eq!(
            GotoOptions::from_params(FrameGotoParams {
                url: "x".to_string(),
                wait_until: Some(LifecycleEvent::Networkidle),
                referer: None
            })
            .unwrap_err(),
            PlaywrightError::error("Neo does not support waitUntil: \"networkidle\" yet")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_title_reads_and_reattaches_once_then_falls_back() {
        for (answers, expected) in [
            (vec![Ok("Example".to_string())], "Example"),
            (
                vec![Err(PageError::NotConnected), Ok("New page".to_string())],
                "New page",
            ),
            (
                vec![Err(PageError::NotConnected), Err(PageError::NotConnected)],
                "",
            ),
            (vec![Err(PageError::Threw("gone".to_string()))], ""),
        ] {
            let tab = FakeTab::new([]);
            *tab.titles.lock().expect("titles") = answers.into();
            let (progress, _, _) = progress(0.0);
            assert_eq!(
                title(&tab, &progress).await,
                Ok(FrameTitleResult {
                    value: expected.to_string()
                })
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_title_returns_a_pending_url_even_mid_evaluation() {
        for already_pending in [false, true] {
            let pending = NavigationState {
                pending_url: Some("https://slow.test/".to_string()),
                ..NavigationState::default()
            };
            let mut tab = FakeTab::new([Step {
                at: 10,
                navigation: None,
                state: pending.clone(),
            }]);
            if already_pending {
                tab.initial = pending;
            }
            tab.title_after = 200;
            let (progress, _, _) = progress(0.0);
            assert_eq!(
                title(&tab, &progress).await,
                Ok(FrameTitleResult {
                    value: "Loading https://slow.test/".to_string()
                })
            );
            assert!(
                !tab.titles.lock().expect("titles").is_empty(),
                "the pending title does not await the renderer"
            );
        }
    }
}
