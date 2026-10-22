pub mod config;
pub mod detect;
pub mod events;
pub mod page;
pub mod providers;
pub mod recaptcha;
pub mod reservation;
pub mod turnstile;
pub mod twocaptcha;
pub mod types;

use std::{future::Future, sync::Arc, time::Duration};

use browseros_core::{BrowserView, PageId};
use serde_json::json;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use self::{
    events::{CaptchaEventBus, EventLog},
    page::{CaptchaPage, LivePage},
    providers::{CaptchaBackend, ProviderChain},
    reservation::{ActiveSolve, SolveLease, TabReservations},
    types::{
        Attempts, CaptchaEventName, CaptchaKind, Challenge, ChallengeState, Detection, DocumentId,
        FlowResult, PageError, SolveMethod, SolveOutcome, SolveReason, SolveStatus, SolveStep,
    },
};

pub struct SolveCtx<'a, P: CaptchaPage, B: CaptchaBackend> {
    pub page: &'a P,
    pub backend: &'a B,
    pub events: &'a EventLog,
    pub lease: &'a SolveLease,
    pub cancel: &'a CancellationToken,
    pub deadline: tokio::time::Instant,
    pub page_url: String,
    pub document: DocumentId,
    pub user_agent: Option<String>,
}

pub struct CaptchaService {
    pub providers: ProviderChain,
    reservations: Arc<TabReservations>,
    events: CaptchaEventBus,
}

#[derive(Debug)]
pub enum SolveRefusal {
    InvalidPage(String),
    Busy(ActiveSolve),
    BrowserUnavailable,
}

impl CaptchaService {
    pub fn from_config(config: config::CaptchaConfig) -> Option<Arc<Self>> {
        if !config.has_provider() {
            return None;
        }
        Some(Arc::new(Self {
            providers: ProviderChain::new(&config),
            reservations: Arc::default(),
            events: CaptchaEventBus::new(),
        }))
    }

    pub fn from_env() -> Option<Arc<Self>> {
        Self::from_config(config::CaptchaConfig::from_env())
    }
    pub fn reservations(&self) -> &Arc<TabReservations> {
        &self.reservations
    }
    pub fn events(&self) -> &CaptchaEventBus {
        &self.events
    }

    pub async fn solve(
        &self,
        view: BrowserView,
        page: PageId,
        cancel: CancellationToken,
        human_help_available: bool,
    ) -> Result<SolveOutcome, SolveRefusal> {
        let deadline = Instant::now() + Duration::from_secs(120);
        let live = LivePage::new(view, page.clone());
        let validation = tokio::select! {
            biased;
            _ = cancel.cancelled() => None,
            _ = tokio::time::sleep_until(deadline) => None,
            result = live.view.page(page.clone()) => Some(result),
        };
        if let Some(result) = validation {
            result.map_err(|error| SolveRefusal::InvalidPage(error.to_string()))?;
        }
        self.solve_with(
            &live,
            page,
            cancel,
            human_help_available,
            deadline,
            &LiveDriver,
        )
        .await
    }

    async fn solve_with<P: CaptchaPage>(
        &self,
        page: &P,
        page_id: PageId,
        cancel: CancellationToken,
        human_help_available: bool,
        deadline: Instant,
        driver: &impl SolveDriver,
    ) -> Result<SolveOutcome, SolveRefusal> {
        let started = Instant::now();
        let request_id = format!("cap_{}", ulid::Ulid::new());
        let lease = self
            .reservations()
            .try_reserve(page_id.0, &request_id)
            .map_err(SolveRefusal::Busy)?;
        let events = EventLog::new(self.events().clone(), request_id.clone(), page_id.0);
        let mut host = None;
        let work = async {
            let mut user_agent = None;
            let mut read_user_agent = false;
            for rescan in 0..=1 {
                let detection = match driver.detect(page).await {
                    Err(PageError::Navigated) if rescan == 0 => continue,
                    result => match result {
                        Ok(detection) => detection,
                        Err(error) => return page_failure(error),
                    },
                };
                let current = match page.document().await {
                    Ok(document) => document,
                    Err(PageError::Navigated) if rescan == 0 => continue,
                    Err(error) => return page_failure(error),
                };
                if current != detection.document {
                    if rescan == 0 {
                        continue;
                    }
                    return page_failure(PageError::Navigated);
                }
                host = url::Url::parse(&detection.document.url)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned));
                let selected = detection
                    .challenges
                    .iter()
                    .find(|challenge| {
                        challenge.kind.is_supported() && challenge.state == ChallengeState::Active
                    })
                    .or_else(|| {
                        detection.challenges.iter().find(|challenge| {
                            !challenge.kind.is_supported()
                                && challenge.kind != CaptchaKind::Unknown
                                && matches!(
                                    challenge.state,
                                    ChallengeState::Active | ChallengeState::Unknown
                                )
                        })
                    })
                    .or_else(|| detection.challenges.first());
                if selected.is_some_and(|challenge| {
                    challenge.kind.is_supported() && challenge.state == ChallengeState::Active
                }) && !read_user_agent
                {
                    user_agent = page
                        .eval_main("navigator.userAgent")
                        .await
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_owned));
                    read_user_agent = true;
                    match page.document().await {
                        Ok(document) if document != detection.document => {
                            if rescan == 0 {
                                continue;
                            }
                            return page_failure(PageError::Navigated);
                        }
                        Err(error) => return page_failure(error),
                        _ => {}
                    }
                }
                if let Some(challenge) = selected {
                    lease.set_progress(Some(challenge.kind), SolveStep::Detecting);
                    events.emit(CaptchaEventName::Detected, json!({"type":challenge.kind,"state":challenge.state,"complete":detection.complete,"challenges":detection.challenges.len()}));
                    if challenge.kind.is_supported() && challenge.state == ChallengeState::Active {
                        let ctx = SolveCtx {
                            page,
                            backend: &self.providers,
                            events: &events,
                            lease: &lease,
                            cancel: &cancel,
                            deadline,
                            page_url: detection.document.url.clone(),
                            document: detection.document,
                            user_agent,
                        };
                        return driver.solve(&ctx, challenge).await;
                    }
                    if !challenge.kind.is_supported()
                        && challenge.kind != CaptchaKind::Unknown
                        && matches!(
                            challenge.state,
                            ChallengeState::Active | ChallengeState::Unknown
                        )
                    {
                        return terminal(
                            SolveStatus::Unsupported,
                            SolveReason::UnsupportedType,
                            "This CAPTCHA type is not supported.",
                        );
                    }
                }
                if !detection.complete {
                    return terminal(
                        SolveStatus::Failed,
                        SolveReason::ScanIncomplete,
                        "The CAPTCHA scan was incomplete. Take a fresh snapshot and retry.",
                    );
                }
                if detection.challenges.is_empty() {
                    return terminal(
                        SolveStatus::AlreadyClear,
                        SolveReason::NoCaptcha,
                        "No CAPTCHA was found. Continue the task.",
                    );
                }
                if detection
                    .challenges
                    .iter()
                    .all(|challenge| challenge.state == ChallengeState::Passed)
                {
                    return terminal(
                        SolveStatus::AlreadyClear,
                        SolveReason::AlreadyPassed,
                        "The CAPTCHA has already passed. Continue the task.",
                    );
                }
                if detection.challenges.iter().all(|challenge| {
                    matches!(
                        challenge.state,
                        ChallengeState::Passed | ChallengeState::WidgetOnly
                    )
                }) {
                    return terminal(
                        SolveStatus::AlreadyClear,
                        SolveReason::NotBlocking,
                        "The CAPTCHA widget is not blocking the page. Continue the task.",
                    );
                }
                return terminal(
                    SolveStatus::Failed,
                    SolveReason::ScanIncomplete,
                    "The CAPTCHA state could not be determined. Take a fresh snapshot and retry.",
                );
            }
            page_failure(PageError::Navigated)
        };
        let mut flow = tokio::select! {
            biased;
            _ = cancel.cancelled() => page_failure(PageError::Cancelled),
            _ = tokio::time::sleep_until(deadline) => terminal(SolveStatus::Failed, SolveReason::DeadlineExceeded, "The CAPTCHA solve exceeded its 120-second deadline. Take a fresh snapshot before retrying."),
            _ = wait_for_page_close(page) => page_failure(PageError::Closed),
            result = work => result,
        };
        let kind = self
            .reservations()
            .active(page_id.0)
            .and_then(|active| active.kind);
        let mut last_method = SolveMethod::None;
        for event in events
            .events()
            .iter()
            .filter(|event| event.event == CaptchaEventName::Solving)
        {
            if let Some(method) = event
                .data
                .get("method")
                .and_then(|value| serde_json::from_value::<SolveMethod>(value.clone()).ok())
            {
                last_method = method;
                let attempt = event
                    .data
                    .get("attempt")
                    .and_then(|value| value.as_u64())
                    .and_then(|value| u32::try_from(value).ok())
                    .unwrap_or(0);
                match method {
                    SolveMethod::Checkbox => {
                        flow.attempts.checkbox = flow.attempts.checkbox.max(attempt)
                    }
                    SolveMethod::Token => flow.attempts.token = flow.attempts.token.max(attempt),
                    SolveMethod::Tiles => {
                        flow.attempts.tile_rounds = flow.attempts.tile_rounds.max(
                            event
                                .data
                                .get("round")
                                .and_then(|value| value.as_u64())
                                .and_then(|value| u32::try_from(value).ok())
                                .unwrap_or(attempt),
                        )
                    }
                    _ => {}
                }
            }
        }
        if flow.method == SolveMethod::None {
            flow.method = last_method;
        }
        if matches!(
            flow.status,
            SolveStatus::NeedsHuman | SolveStatus::Unsupported
        ) {
            flow.message.push_str(if human_help_available {
                " Call request_human_help with kind \"captcha\"."
            } else {
                " Stop and tell the user that this CAPTCHA needs their help."
            });
        }
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match flow.status {
            SolveStatus::Solved => events.emit(
                CaptchaEventName::Solved,
                json!({"type":kind,"method":flow.method,"durationMs":duration_ms}),
            ),
            SolveStatus::NeedsHuman => events.emit(
                CaptchaEventName::Handoff,
                json!({"type":kind,"reason":flow.reason,"humanHelp":human_help_available}),
            ),
            SolveStatus::Failed | SolveStatus::Cancelled | SolveStatus::Unsupported => events.emit(
                CaptchaEventName::Failed,
                json!({"type":kind,"status":flow.status,"reason":flow.reason}),
            ),
            SolveStatus::AlreadyClear => {}
        }
        let outcome = SolveOutcome {
            status: flow.status,
            kind,
            method: flow.method,
            reason: flow.reason,
            request_id,
            page: page_id.0,
            duration_ms,
            attempts: flow.attempts,
            paid_tasks: flow.paid_tasks,
            cost_usd: flow.cost_usd,
            message: flow.message,
            events: events.events(),
        };
        events::log_outcome(&outcome, host.as_deref(), None);
        Ok(outcome)
    }
}

trait SolveDriver: Send + Sync {
    fn detect<P: CaptchaPage>(
        &self,
        page: &P,
    ) -> impl Future<Output = Result<Detection, PageError>> + Send;
    fn solve<P: CaptchaPage, B: CaptchaBackend>(
        &self,
        ctx: &SolveCtx<'_, P, B>,
        challenge: &Challenge,
    ) -> impl Future<Output = FlowResult> + Send;
}
struct LiveDriver;
impl SolveDriver for LiveDriver {
    async fn detect<P: CaptchaPage>(&self, page: &P) -> Result<Detection, PageError> {
        detect::detect(page, Duration::from_secs(2)).await
    }
    async fn solve<P: CaptchaPage, B: CaptchaBackend>(
        &self,
        ctx: &SolveCtx<'_, P, B>,
        challenge: &Challenge,
    ) -> FlowResult {
        match challenge.kind {
            CaptchaKind::RecaptchaV2 => recaptcha::solve(ctx, challenge).await,
            CaptchaKind::Turnstile => turnstile::solve(ctx, challenge).await,
            _ => terminal(
                SolveStatus::Unsupported,
                SolveReason::UnsupportedType,
                "This CAPTCHA type is not supported.",
            ),
        }
    }
}
fn terminal(status: SolveStatus, reason: SolveReason, message: &str) -> FlowResult {
    FlowResult {
        status,
        method: SolveMethod::None,
        reason,
        attempts: Attempts::default(),
        paid_tasks: 0,
        cost_usd: 0.0,
        message: message.into(),
    }
}
fn page_failure(error: PageError) -> FlowResult {
    let (status, reason, message) = match error {
        PageError::Closed => (
            SolveStatus::Cancelled,
            SolveReason::TabClosed,
            "The page closed during the CAPTCHA solve.",
        ),
        PageError::Navigated => (
            SolveStatus::Cancelled,
            SolveReason::Navigated,
            "The page navigated during the CAPTCHA solve. Take a fresh snapshot.",
        ),
        PageError::Cancelled => (
            SolveStatus::Cancelled,
            SolveReason::CallerCancelled,
            "The CAPTCHA solve was cancelled by the caller.",
        ),
        PageError::Timeout | PageError::Browser(_) => (
            SolveStatus::Failed,
            SolveReason::BrowserError,
            "A browser operation failed during the CAPTCHA solve. Take a fresh snapshot and retry.",
        ),
    };
    terminal(status, reason, message)
}
async fn wait_for_page_close(page: &impl CaptchaPage) {
    loop {
        if matches!(page.document().await, Err(PageError::Closed)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_that_coordination_stubs_use_stable_constructors() -> anyhow::Result<()> {
        let reservations = Arc::<TabReservations>::default();
        let lease = reservations
            .try_reserve(42, "cap_test")
            .map_err(|_| anyhow::anyhow!("unexpected busy page"))?;
        let log = EventLog::new(CaptchaEventBus::new(), "cap_test".into(), 42);
        assert_eq!(lease.page(), 42);
        assert_eq!(lease.request_id(), "cap_test");
        assert!(log.events().is_empty());
        Ok(())
    }

    #[test]
    fn tests_that_service_is_disabled_without_a_provider() {
        assert!(CaptchaService::from_config(config::CaptchaConfig::default()).is_none());
    }

    #[test]
    fn tests_that_service_is_enabled_with_a_provider() -> anyhow::Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), "test-key")?;
        let config = config::CaptchaConfig::from_lookup(|name| {
            (name == config::CAPSOLVER_KEY_FILE_ENV)
                .then(|| file.path().to_string_lossy().into_owned())
        });
        assert!(CaptchaService::from_config(config).is_some());
        Ok(())
    }
}

#[cfg(test)]
mod orchestration_tests {
    use super::*;
    use serde_json::{Value, json};
    use std::{collections::VecDeque, sync::Mutex, time::Duration};
    use types::{
        CaptchaEventName, CaptchaKind, Challenge, ChallengeState, Detection, FlowResult, FrameRef,
        PageError, Rect, ScanOutcome, SolveStep,
    };

    fn document(loader: &str) -> DocumentId {
        DocumentId {
            url: "https://fixture.test/captcha?private=yes".into(),
            loader_id: loader.into(),
        }
    }

    struct FakePage(Result<DocumentId, PageError>);
    impl CaptchaPage for FakePage {
        async fn eval_main(&self, _: &str) -> Result<Value, PageError> {
            Ok(json!("fake-agent"))
        }
        async fn document(&self) -> Result<DocumentId, PageError> {
            self.0.clone()
        }
        async fn frames(&self) -> Result<Vec<FrameRef>, PageError> {
            Ok(vec![])
        }
        async fn eval_in_frame(&self, _: &FrameRef, _: &str) -> Result<Value, PageError> {
            Ok(Value::Null)
        }
        async fn click_at(&self, _: f64, _: f64) -> Result<(), PageError> {
            Ok(())
        }
        async fn screenshot_clip(&self, _: Rect) -> Result<String, PageError> {
            Ok(String::new())
        }
        async fn query_rect_in_frame(
            &self,
            _: &FrameRef,
            _: &str,
        ) -> Result<Option<Rect>, PageError> {
            Ok(None)
        }
    }

    struct FakeDriver {
        scans: Mutex<VecDeque<Result<Detection, PageError>>>,
        stall: bool,
        result: Option<FlowResult>,
    }
    impl FakeDriver {
        fn new(detection: Detection) -> Self {
            Self {
                scans: Mutex::new(VecDeque::from([Ok(detection)])),
                stall: false,
                result: None,
            }
        }
    }
    impl SolveDriver for FakeDriver {
        async fn detect<P: CaptchaPage>(&self, _: &P) -> Result<Detection, PageError> {
            self.scans
                .lock()
                .map_err(|_| PageError::Browser("poisoned".into()))?
                .pop_front()
                .ok_or_else(|| PageError::Browser("no scan".into()))?
        }
        async fn solve<P: CaptchaPage, B: CaptchaBackend>(
            &self,
            ctx: &SolveCtx<'_, P, B>,
            challenge: &Challenge,
        ) -> FlowResult {
            let (method, reason) = match challenge.kind {
                CaptchaKind::RecaptchaV2 => (SolveMethod::Checkbox, SolveReason::CheckboxPassed),
                CaptchaKind::Turnstile => (SolveMethod::AutoPass, SolveReason::AutoPassed),
                _ => unreachable!("unsupported challenge reached flow"),
            };
            ctx.lease
                .set_progress(Some(challenge.kind), SolveStep::Checkbox);
            ctx.events.emit(
                CaptchaEventName::Solving,
                json!({"type":challenge.kind,"method":method,"attempt":1}),
            );
            if self.stall {
                std::future::pending::<()>().await;
            }
            self.result.clone().unwrap_or_else(|| FlowResult {
                status: SolveStatus::Solved,
                method,
                reason,
                attempts: Attempts {
                    checkbox: u32::from(method == SolveMethod::Checkbox),
                    ..Default::default()
                },
                paid_tasks: 0,
                cost_usd: 0.0,
                message: ctx.user_agent.clone().unwrap_or_default(),
            })
        }
    }
    fn service() -> CaptchaService {
        CaptchaService {
            providers: ProviderChain::new(&config::CaptchaConfig::default()),
            reservations: Arc::default(),
            events: CaptchaEventBus::new(),
        }
    }
    fn scan(challenges: &[(CaptchaKind, ChallengeState)], complete: bool) -> Detection {
        Detection {
            outcome: if challenges.is_empty() {
                ScanOutcome::None
            } else {
                ScanOutcome::Detected
            },
            complete,
            document: document("current"),
            challenges: challenges
                .iter()
                .map(|&(kind, state)| Challenge {
                    kind,
                    state,
                    frame: None,
                    challenge_frame: None,
                    sitekey: None,
                    invisible: false,
                    enterprise: false,
                    recaptcha_s: None,
                    action: None,
                    cdata: None,
                    callback: None,
                })
                .collect(),
        }
    }
    async fn run(driver: &FakeDriver, human_help: bool) -> anyhow::Result<SolveOutcome> {
        service()
            .solve_with(
                &FakePage(Ok(document("current"))),
                PageId(42),
                CancellationToken::new(),
                human_help,
                tokio::time::Instant::now() + Duration::from_secs(120),
                driver,
            )
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))
    }

    #[tokio::test]
    async fn tests_that_a_complete_empty_scan_has_the_contract_shape_and_no_events()
    -> anyhow::Result<()> {
        let outcome = run(&FakeDriver::new(scan(&[], true)), false).await?;
        assert_eq!(
            (outcome.status, outcome.reason),
            (SolveStatus::AlreadyClear, SolveReason::NoCaptcha)
        );
        assert!(outcome.events.is_empty());
        assert!(outcome.request_id.starts_with("cap_"));
        ulid::Ulid::from_string(&outcome.request_id[4..])?;
        let value = serde_json::to_value(&outcome)?;
        assert_eq!(value.as_object().map(|object| object.len()), Some(12));
        assert_eq!(value["type"], Value::Null);
        assert_eq!(
            value["attempts"],
            json!({"checkbox":0,"tileRounds":0,"token":0})
        );
        assert_eq!(value["paidTasks"], 0);
        assert_eq!(value["costUsd"], 0.0);
        assert_eq!(value["page"], 42);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_incomplete_scans_never_claim_the_page_is_clear() -> anyhow::Result<()> {
        for challenges in [
            vec![],
            vec![(CaptchaKind::RecaptchaV2, ChallengeState::Passed)],
        ] {
            let outcome = run(&FakeDriver::new(scan(&challenges, false)), false).await?;
            assert_eq!(
                (outcome.status, outcome.reason),
                (SolveStatus::Failed, SolveReason::ScanIncomplete)
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_active_hcaptcha_is_unsupported_and_has_ordered_events() -> anyhow::Result<()>
    {
        let outcome = run(
            &FakeDriver::new(scan(
                &[(CaptchaKind::Hcaptcha, ChallengeState::Active)],
                true,
            )),
            true,
        )
        .await?;
        assert_eq!(
            (outcome.status, outcome.reason),
            (SolveStatus::Unsupported, SolveReason::UnsupportedType)
        );
        assert!(outcome.message.contains("request_human_help"));
        assert_eq!(
            outcome
                .events
                .iter()
                .map(|event| event.event)
                .collect::<Vec<_>>(),
            [CaptchaEventName::Detected, CaptchaEventName::Failed]
        );
        assert_eq!(
            outcome.events[0].data,
            json!({"type":"hcaptcha","state":"active","complete":true,"challenges":1})
        );
        assert_eq!(
            outcome.events[1].data,
            json!({"type":"hcaptcha","status":"unsupported","reason":"unsupported_type"})
        );
        assert!(
            outcome
                .events
                .iter()
                .all(|event| event.page == 42 && event.request_id == outcome.request_id)
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_passed_and_nonblocking_widgets_are_already_clear() -> anyhow::Result<()> {
        for (kind, state, reason) in [
            (
                CaptchaKind::Hcaptcha,
                ChallengeState::Passed,
                SolveReason::AlreadyPassed,
            ),
            (
                CaptchaKind::RecaptchaV3,
                ChallengeState::WidgetOnly,
                SolveReason::NotBlocking,
            ),
        ] {
            let outcome = run(&FakeDriver::new(scan(&[(kind, state)], true)), false).await?;
            assert_eq!(
                (outcome.status, outcome.reason),
                (SolveStatus::AlreadyClear, reason)
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_supported_active_challenges_take_priority_and_reach_their_flow()
    -> anyhow::Result<()> {
        for (kind, method) in [
            (CaptchaKind::RecaptchaV2, SolveMethod::Checkbox),
            (CaptchaKind::Turnstile, SolveMethod::AutoPass),
        ] {
            let outcome = run(
                &FakeDriver::new(scan(
                    &[
                        (CaptchaKind::Hcaptcha, ChallengeState::Active),
                        (kind, ChallengeState::Active),
                    ],
                    true,
                )),
                false,
            )
            .await?;
            assert_eq!(
                (outcome.status, outcome.kind, outcome.method),
                (SolveStatus::Solved, Some(kind), method)
            );
            assert_eq!(outcome.message, "fake-agent");
            assert_eq!(
                outcome
                    .events
                    .iter()
                    .map(|event| event.event)
                    .collect::<Vec<_>>(),
                [
                    CaptchaEventName::Detected,
                    CaptchaEventName::Solving,
                    CaptchaEventName::Solved
                ]
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_stale_detection_is_rescanned_once_before_routing() -> anyhow::Result<()> {
        let mut stale = scan(&[(CaptchaKind::Hcaptcha, ChallengeState::Active)], true);
        stale.document = document("old");
        let driver = FakeDriver {
            scans: Mutex::new(VecDeque::from([Ok(stale.clone()), Ok(scan(&[], true))])),
            stall: false,
            result: None,
        };
        assert_eq!(run(&driver, false).await?.reason, SolveReason::NoCaptcha);
        let driver = FakeDriver {
            scans: Mutex::new(VecDeque::from([Ok(stale.clone()), Ok(stale)])),
            stall: false,
            result: None,
        };
        assert_eq!(run(&driver, false).await?.reason, SolveReason::Navigated);
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_busy_pages_are_refused_without_replacing_the_lease() -> anyhow::Result<()> {
        let service = service();
        let lease = service
            .reservations
            .try_reserve(42, "cap_existing")
            .map_err(|_| anyhow::anyhow!("busy"))?;
        let result = service
            .solve_with(
                &FakePage(Ok(document("current"))),
                PageId(42),
                CancellationToken::new(),
                false,
                tokio::time::Instant::now() + Duration::from_secs(120),
                &FakeDriver::new(scan(&[], true)),
            )
            .await;
        assert!(
            matches!(result,Err(SolveRefusal::Busy(active)) if active.request_id == lease.request_id())
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_cancellation_and_deadlines_release_reservations() -> anyhow::Result<()> {
        for cancelled in [false, true] {
            let service = service();
            let cancel = CancellationToken::new();
            let driver = FakeDriver {
                stall: true,
                ..FakeDriver::new(scan(
                    &[(CaptchaKind::RecaptchaV2, ChallengeState::Active)],
                    true,
                ))
            };
            if cancelled {
                let token = cancel.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    token.cancel();
                });
            }
            let outcome = service
                .solve_with(
                    &FakePage(Ok(document("current"))),
                    PageId(42),
                    cancel,
                    false,
                    tokio::time::Instant::now() + Duration::from_millis(10),
                    &driver,
                )
                .await
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            assert_eq!(
                outcome.reason,
                if cancelled {
                    SolveReason::CallerCancelled
                } else {
                    SolveReason::DeadlineExceeded
                }
            );
            assert_eq!(outcome.attempts.checkbox, 1);
            assert_eq!(outcome.kind, Some(CaptchaKind::RecaptchaV2));
            assert_eq!(
                outcome.events.last().map(|event| event.event),
                Some(CaptchaEventName::Failed)
            );
            assert!(service.reservations.active(42).is_none());
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_closed_pages_cancel_and_handoffs_explain_the_next_step()
    -> anyhow::Result<()> {
        let outcome = service()
            .solve_with(
                &FakePage(Err(PageError::Closed)),
                PageId(42),
                CancellationToken::new(),
                false,
                tokio::time::Instant::now() + Duration::from_secs(120),
                &FakeDriver::new(scan(&[], true)),
            )
            .await
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        assert_eq!(
            (outcome.status, outcome.reason),
            (SolveStatus::Cancelled, SolveReason::TabClosed)
        );
        for human_help in [false, true] {
            let driver = FakeDriver {
                result: Some(FlowResult {
                    status: SolveStatus::NeedsHuman,
                    method: SolveMethod::Token,
                    reason: SolveReason::ProviderUnavailable,
                    attempts: Attempts {
                        token: 1,
                        ..Default::default()
                    },
                    paid_tasks: 2,
                    cost_usd: 0.002,
                    message: "ERROR_ZERO_BALANCE".into(),
                }),
                ..FakeDriver::new(scan(
                    &[(CaptchaKind::Turnstile, ChallengeState::Active)],
                    true,
                ))
            };
            let outcome = run(&driver, human_help).await?;
            assert_eq!(outcome.paid_tasks, 2);
            assert_eq!(outcome.cost_usd, 0.002);
            assert!(outcome.message.contains("ERROR_ZERO_BALANCE"));
            assert_eq!(outcome.message.contains("request_human_help"), human_help);
            assert_eq!(outcome.message.contains("tell the user"), !human_help);
            assert_eq!(
                outcome.events.last().map(|event| &event.data),
                Some(
                    &json!({"type":"turnstile","reason":"provider_unavailable","humanHelp":human_help})
                )
            );
        }
        Ok(())
    }
}
