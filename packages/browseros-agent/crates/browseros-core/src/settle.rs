use crate::{CoreError, PageId, ProtocolSession, pages::PageManager, timeouts};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{future::Future, time::Duration};
use tokio::time::{Instant, sleep, timeout};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettleOutcome {
    Settled { navigation_observed: bool },
    BudgetExpired,
    Skipped { reason: String },
}

#[derive(Debug, Deserialize)]
struct EvaluateResult {
    result: RemoteObject,
}

#[derive(Debug, Deserialize)]
struct RemoteObject {
    value: Option<Value>,
}

const INSTALL_OBSERVER_JS: &str = r#"
(() => {
  const root = document.body || document.documentElement;
  const previous = globalThis.__browseros_settle;
  if (previous && previous.observer) {
    try { previous.observer.disconnect(); } catch (_) {}
  }
  const state = { n: 0, observer: null };
  if (!root || typeof MutationObserver === 'undefined') {
    globalThis.__browseros_settle = state;
    return state.n;
  }
  state.observer = new MutationObserver(() => { state.n += 1; });
  state.observer.observe(root, {
    childList: true,
    subtree: true,
    attributes: true,
    characterData: true
  });
  globalThis.__browseros_settle = state;
  return state.n;
})()
"#;

const READ_COUNTER_JS: &str = r#"
(() => {
  const state = globalThis.__browseros_settle;
  return state && typeof state.n === 'number' ? state.n : 0;
})()
"#;

/// Whether a page's own content has stopped changing.
///
/// A separate question from `SettleOutcome`, and a separate counter, so the
/// two mechanisms cannot disturb each other on the same page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// The content was showing and the mutation counter did not move across a
    /// quiet interval.
    Stable,
    /// The ceiling was reached while the page was still changing, or still
    /// blank. Acting is still the right next move; the caller just knows the
    /// page was not finished.
    StillMoving,
    /// The page could not be read. Advisory, exactly like `SettleOutcome::Skipped`.
    Unknown { reason: String },
}

/// Installs the readiness counter if it is missing, then reads it.
///
/// Install and read are one call on purpose. A navigation replaces the body
/// and takes the observer with it, so a reader that only read would see a
/// counter frozen at zero on the new document and call a page that is still
/// filling in "stable". Reporting `installed` lets the caller discard the
/// reading that re-armed it rather than compare across the gap.
/// Reads the page's mutation counter, installing the observer if it is absent.
///
/// Known limit: a `MutationObserver` subtree does not cross an iframe or a
/// shadow boundary, so a change confined to embedded content leaves this
/// counter still. Covering those roots means observing each one and tracking
/// their lifetimes; until then a caller can be told a page is quiet while
/// embedded content is still moving.
const READINESS_PROBE_JS: &str = r#"
(() => {
  const g = globalThis;
  const root = document.body || document.documentElement;
  let s = g.__browseros_ready;
  let installed = false;
  if (!s || !s.observer || s.root !== root) {
    if (s && s.observer) { try { s.observer.disconnect(); } catch (_) {} }
    s = { n: 0, observer: null, root: root };
    if (root && typeof MutationObserver !== 'undefined') {
      s.observer = new MutationObserver(() => { s.n += 1; });
      s.observer.observe(root, {
        childList: true,
        subtree: true,
        attributes: true,
        characterData: true
      });
    }
    g.__browseros_ready = s;
    installed = true;
  }
  const body = document.body;
  const showing = !!body &&
    (body.childElementCount > 0 || (body.textContent || '').trim().length > 0);
  return {
    n: s.n,
    installed: installed,
    showing: showing,
    complete: document.readyState === 'complete'
  };
})()
"#;

#[derive(Debug, Deserialize)]
struct ReadingResult {
    result: ReadingObject,
}

#[derive(Debug, Deserialize)]
struct ReadingObject {
    value: Option<ReadinessReading>,
}

/// One reading of a page's readiness counter.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ReadinessReading {
    /// Mutations the observer has seen since it was installed.
    #[serde(rename = "n")]
    pub count: u64,
    /// Whether this call re-armed the observer. When it did, `count` started
    /// over and cannot be compared with the reading before it.
    pub installed: bool,
    /// Whether the body has anything in it.
    pub showing: bool,
    /// Whether the document reports itself finished.
    pub complete: bool,
}

impl ReadinessReading {
    /// Quiet is only evidence of readiness once there is something to be quiet
    /// about.
    ///
    /// A tab one interval old can be quiet because it has not started, and
    /// calling that ready hands back a blank page. `complete` is the other
    /// sufficient answer, and it is what keeps a genuinely empty document
    /// (about:blank, a 204) from burning the whole ceiling.
    #[must_use]
    pub fn settled_enough(self) -> bool {
        self.showing || self.complete
    }
}

/// Installs the readiness counter if it is missing, then reads it once.
///
/// For callers running their own loop and their own timeout, such as the
/// navigation wait, which has a dialog check to interleave.
pub async fn read_readiness(session: &ProtocolSession) -> Result<ReadinessReading, CoreError> {
    let call = session.send(
        "Runtime.evaluate",
        json!({ "expression": READINESS_PROBE_JS, "returnByValue": true }),
    );
    // Bounded like every other call on the settle path. The probe is advisory,
    // so a page whose main thread is busy must not park it for the transport's
    // default, which is two orders of magnitude longer than the budget any
    // caller of this function is working to.
    let result: ReadingResult = match timeout(timeouts::ACTION_SETTLE_CDP_CALL_TIMEOUT, call).await
    {
        Ok(result) => result?,
        Err(_elapsed) => {
            return Err(CoreError::Message(format!(
                "the readiness probe did not answer within {}ms",
                timeouts::ACTION_SETTLE_CDP_CALL_TIMEOUT.as_millis()
            )));
        }
    };
    result
        .result
        .value
        .ok_or_else(|| CoreError::Message("the readiness probe returned nothing".to_string()))
}

/// Waits until the page's content stops changing, or the ceiling is reached.
///
/// This is readiness as the agent experiences it. The load lifecycle answers a
/// different question, one about the network, and on a commercial page it
/// answers it late or never: measured on this tool surface, three of seven
/// page opens returned on a 3000ms exhaustion rather than on any signal, and
/// the work that followed each took under 25ms.
pub async fn wait_for_representation_stable(
    session: &ProtocolSession,
    budget: Duration,
) -> Readiness {
    if budget.is_zero() {
        return Readiness::StillMoving;
    }
    let deadline = Instant::now() + budget;
    let mut previous: Option<u64> = None;
    loop {
        let reading = match read_reading(session, deadline).await {
            Ok(reading) => reading,
            Err(readiness) => return readiness,
        };
        if !reading.installed && previous == Some(reading.count) && reading.settled_enough() {
            return Readiness::Stable;
        }
        previous = Some(reading.count);
        if sleep_for_budget(timeouts::ACTION_SETTLE_DOM_QUIET, deadline)
            .await
            .is_err()
        {
            return Readiness::StillMoving;
        }
    }
}

/// The same wait, for a caller holding a page rather than a session.
pub async fn wait_for_page_ready(
    pages: &PageManager,
    page_id: PageId,
    budget: Duration,
) -> Readiness {
    let deadline = Instant::now() + budget;
    let page = match call_with_deadline(pages.get_session(page_id), deadline).await {
        Ok(page) => page,
        Err(outcome) => return into_readiness(outcome),
    };
    let Ok(left) = remaining(deadline) else {
        return Readiness::StillMoving;
    };
    wait_for_representation_stable(&page.session, left).await
}

async fn read_reading(
    session: &ProtocolSession,
    deadline: Instant,
) -> Result<ReadinessReading, Readiness> {
    let result: ReadingResult = call_with_deadline(
        session.send(
            "Runtime.evaluate",
            json!({ "expression": READINESS_PROBE_JS, "returnByValue": true }),
        ),
        deadline,
    )
    .await
    .map_err(into_readiness)?;
    result.result.value.ok_or_else(|| Readiness::Unknown {
        reason: "the readiness probe returned nothing".to_string(),
    })
}

fn into_readiness(outcome: SettleOutcome) -> Readiness {
    match outcome {
        SettleOutcome::Skipped { reason } => Readiness::Unknown { reason },
        _ => Readiness::StillMoving,
    }
}

pub async fn wait_for_action_settle(
    pages: &PageManager,
    page_id: PageId,
    budget: Duration,
) -> SettleOutcome {
    if budget.is_zero() {
        return SettleOutcome::BudgetExpired;
    }

    let deadline = Instant::now() + budget;
    let navigation_observed = match detect_navigation(pages, page_id.clone(), deadline).await {
        Ok(observed) => observed,
        Err(outcome) => return outcome,
    };

    if navigation_observed
        && let Err(outcome) = wait_for_tab_load(pages, page_id.clone(), deadline).await
    {
        return outcome;
    }

    match wait_for_dom_quiet(pages, page_id, deadline).await {
        Ok(()) => SettleOutcome::Settled {
            navigation_observed,
        },
        Err(outcome) => outcome,
    }
}

async fn detect_navigation(
    pages: &PageManager,
    page_id: PageId,
    deadline: Instant,
) -> Result<bool, SettleOutcome> {
    let detect_until = earliest_deadline(
        deadline,
        Instant::now() + timeouts::ACTION_SETTLE_NAVIGATION_DETECT,
    );
    loop {
        let info = refresh_page(pages, page_id.clone(), deadline).await?;
        if tab_is_loading(&info) {
            return Ok(true);
        }
        if Instant::now() >= detect_until {
            return Ok(false);
        }
        sleep_until_budget(
            earliest_deadline(
                detect_until,
                Instant::now() + timeouts::ACTION_SETTLE_NAVIGATION_POLL,
            ),
            deadline,
        )
        .await?;
    }
}

async fn wait_for_tab_load(
    pages: &PageManager,
    page_id: PageId,
    deadline: Instant,
) -> Result<(), SettleOutcome> {
    loop {
        let info = refresh_page(pages, page_id.clone(), deadline).await?;
        if !tab_is_loading(&info) {
            return Ok(());
        }
        sleep_for_budget(timeouts::ACTION_SETTLE_NAVIGATION_POLL, deadline).await?;
    }
}

async fn wait_for_dom_quiet(
    pages: &PageManager,
    page_id: PageId,
    deadline: Instant,
) -> Result<(), SettleOutcome> {
    let page = call_with_deadline(pages.get_session(page_id), deadline).await?;
    let mut previous = evaluate_counter(&page.session, INSTALL_OBSERVER_JS, deadline).await?;

    loop {
        sleep_for_budget(timeouts::ACTION_SETTLE_DOM_QUIET, deadline).await?;
        let current = evaluate_counter(&page.session, READ_COUNTER_JS, deadline).await?;
        if current == previous {
            return Ok(());
        }
        previous = current;
    }
}

async fn refresh_page(
    pages: &PageManager,
    page_id: PageId,
    deadline: Instant,
) -> Result<crate::pages::PageInfo, SettleOutcome> {
    match call_with_deadline(pages.refresh(page_id.clone()), deadline).await? {
        Some(info) => Ok(info),
        None => Err(SettleOutcome::Skipped {
            reason: format!("page {page_id} not found"),
        }),
    }
}

async fn evaluate_counter(
    session: &ProtocolSession,
    expression: &str,
    deadline: Instant,
) -> Result<u64, SettleOutcome> {
    let result: EvaluateResult = call_with_deadline(
        session.send(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true }),
        ),
        deadline,
    )
    .await?;
    Ok(result
        .result
        .value
        .as_ref()
        .and_then(Value::as_u64)
        .unwrap_or(0))
}

async fn call_with_deadline<T, Fut>(future: Fut, deadline: Instant) -> Result<T, SettleOutcome>
where
    Fut: Future<Output = Result<T, CoreError>>,
{
    let call_budget = remaining(deadline)?.min(timeouts::ACTION_SETTLE_CDP_CALL_TIMEOUT);
    match timeout(call_budget, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(SettleOutcome::Skipped {
            reason: err.to_string(),
        }),
        Err(_elapsed) => {
            if remaining(deadline).is_err() {
                Err(SettleOutcome::BudgetExpired)
            } else {
                Err(SettleOutcome::Skipped {
                    reason: format!("CDP call timed out after {}ms", call_budget.as_millis()),
                })
            }
        }
    }
}

fn remaining(deadline: Instant) -> Result<Duration, SettleOutcome> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(SettleOutcome::BudgetExpired)
}

async fn sleep_for_budget(duration: Duration, deadline: Instant) -> Result<(), SettleOutcome> {
    let remaining = remaining(deadline)?;
    if remaining < duration {
        sleep(remaining).await;
        return Err(SettleOutcome::BudgetExpired);
    }
    sleep(duration).await;
    Ok(())
}

async fn sleep_until_budget(until: Instant, deadline: Instant) -> Result<(), SettleOutcome> {
    let now = Instant::now();
    if until <= now {
        return Ok(());
    }
    let capped = earliest_deadline(until, deadline);
    sleep_for_budget(capped.duration_since(now), deadline).await
}

fn earliest_deadline(a: Instant, b: Instant) -> Instant {
    if a <= b { a } else { b }
}

fn tab_is_loading(info: &crate::pages::PageInfo) -> bool {
    info.is_loading && info.load_progress < 1.0
}

#[cfg(test)]
mod tests {
    use super::{SettleOutcome, wait_for_action_settle};
    use crate::{
        BrowserSession, BrowserSessionHooks, CoreError, SessionId, connection::CdpConnection,
        timeouts,
    };
    use browseros_cdp::{CdpError, CdpEvent};
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::{sync::broadcast, time::Instant};

    #[derive(Debug, Clone, Copy)]
    struct TabLoad {
        is_loading: bool,
        load_progress: f64,
    }

    impl TabLoad {
        fn ready() -> Self {
            Self {
                is_loading: false,
                load_progress: 1.0,
            }
        }
    }

    #[derive(Debug)]
    struct HarnessState {
        tab_loads: VecDeque<TabLoad>,
        current_tab: TabLoad,
        mutation_reads: VecDeque<u64>,
        current_counter: u64,
        fail_evaluate: bool,
        /// Successive answers for the readiness probe. The last one repeats
        /// once the queue runs dry, so a test can say "stays blank forever"
        /// with a single entry.
        readiness: VecDeque<Value>,
        readiness_last: Value,
        /// When set, the readiness probe never answers, standing in for a page
        /// whose main thread is too busy to schedule the evaluation.
        stall_readiness: bool,
    }

    struct HarnessConnection {
        state: Mutex<HarnessState>,
        events: broadcast::Sender<CdpEvent>,
    }

    impl HarnessConnection {
        fn new(state: HarnessState) -> Arc<Self> {
            let (events, _rx) = broadcast::channel(1);
            Arc::new(Self {
                state: Mutex::new(state),
                events,
            })
        }
    }

    impl CdpConnection for HarnessConnection {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move {
                // Checked and released before parking: a std MutexGuard is not
                // Send, so it must not be alive across the await below.
                let stalls = {
                    let state = self.state.lock().map_err(|_err| CdpError::Protocol {
                        code: -1,
                        message: "poisoned test state".to_string(),
                    })?;
                    state.stall_readiness
                        && method == "Runtime.evaluate"
                        && params
                            .get("expression")
                            .and_then(Value::as_str)
                            .is_some_and(|expression| expression.contains("__browseros_ready"))
                };
                if stalls {
                    std::future::pending::<()>().await;
                }
                let mut state = self.state.lock().map_err(|_err| CdpError::Protocol {
                    code: -1,
                    message: "poisoned test state".to_string(),
                })?;
                match method {
                    "Browser.getTabs" => Ok(json!({ "tabs": [tab_json(state.current_tab)] })),
                    "Browser.getTabInfo" => {
                        if let Some(next) = state.tab_loads.pop_front() {
                            state.current_tab = next;
                        }
                        Ok(json!({ "tab": tab_json(state.current_tab) }))
                    }
                    "Target.attachToTarget" => Ok(json!({ "sessionId": "session-1" })),
                    "Page.enable"
                    | "DOM.enable"
                    | "Runtime.enable"
                    | "Accessibility.enable"
                    | "Runtime.runIfWaitingForDebugger" => Ok(json!({})),
                    "Runtime.evaluate" => {
                        if state.fail_evaluate {
                            return Err(CdpError::Protocol {
                                code: -32000,
                                message: "evaluate failed".to_string(),
                            });
                        }
                        let expression = params
                            .get("expression")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        // Checked first: the readiness probe also installs a
                        // MutationObserver, so the settle branch below would
                        // otherwise swallow it and answer with a bare number.
                        if expression.contains("__browseros_ready") {
                            if let Some(next) = state.readiness.pop_front() {
                                state.readiness_last = next;
                            }
                            return Ok(json!({ "result": { "value": state.readiness_last } }));
                        }
                        if expression.contains("new MutationObserver") {
                            state.current_counter = 0;
                            return Ok(json!({ "result": { "value": 0 } }));
                        }
                        if expression.contains("__browseros_settle")
                            && let Some(next) = state.mutation_reads.pop_front()
                        {
                            state.current_counter = next;
                        }
                        Ok(json!({ "result": { "value": state.current_counter } }))
                    }
                    _ => Ok(json!({})),
                }
            })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Ok("{}".to_string()) })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            self.events.subscribe()
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    /// One readiness answer, as the probe reports it.
    fn reading(n: u64, installed: bool, showing: bool, complete: bool) -> Value {
        json!({ "n": n, "installed": installed, "showing": showing, "complete": complete })
    }

    async fn harness(
        mutation_reads: impl Into<VecDeque<u64>>,
        tab_loads: impl Into<VecDeque<TabLoad>>,
    ) -> Result<(Arc<BrowserSession>, crate::PageId), CoreError> {
        harness_reading(mutation_reads, tab_loads, VecDeque::new()).await
    }

    async fn harness_reading(
        mutation_reads: impl Into<VecDeque<u64>>,
        tab_loads: impl Into<VecDeque<TabLoad>>,
        readiness: impl Into<VecDeque<Value>>,
    ) -> Result<(Arc<BrowserSession>, crate::PageId), CoreError> {
        let connection = HarnessConnection::new(HarnessState {
            tab_loads: tab_loads.into(),
            current_tab: TabLoad::ready(),
            mutation_reads: mutation_reads.into(),
            current_counter: 0,
            fail_evaluate: false,
            readiness: readiness.into(),
            readiness_last: reading(0, false, true, true),
            stall_readiness: false,
        });
        let session = BrowserSession::new(connection, BrowserSessionHooks::default());
        let pages = session.pages.list().await?;
        let Some(page) = pages.first() else {
            return Err(CoreError::Message("missing harness page".to_string()));
        };
        Ok((session, page.page_id.clone()))
    }

    fn tab_json(load: TabLoad) -> Value {
        json!({
            "tabId": 101,
            "targetId": "target-1",
            "url": "https://example.test/",
            "title": "Test",
            "isActive": true,
            "isLoading": load.is_loading,
            "loadProgress": load.load_progress,
            "isPinned": false,
            "isHidden": false,
            "windowId": 1,
            "index": 0
        })
    }

    #[tokio::test]
    async fn quiet_dom_settles_before_budget() -> Result<(), CoreError> {
        let (session, page_id) = harness(VecDeque::new(), VecDeque::new()).await?;
        let start = Instant::now();
        let outcome =
            wait_for_action_settle(&session.pages, page_id, Duration::from_millis(900)).await;
        assert_eq!(
            outcome,
            SettleOutcome::Settled {
                navigation_observed: false
            }
        );
        assert!(start.elapsed() < Duration::from_millis(500));
        Ok(())
    }

    #[tokio::test]
    async fn mutations_delay_settle_until_one_quiet_interval() -> Result<(), CoreError> {
        let (session, page_id) = harness(VecDeque::from([1, 2, 2]), VecDeque::new()).await?;
        let start = Instant::now();
        let outcome =
            wait_for_action_settle(&session.pages, page_id, Duration::from_millis(1_000)).await;
        assert_eq!(
            outcome,
            SettleOutcome::Settled {
                navigation_observed: false
            }
        );
        assert!(start.elapsed() >= Duration::from_millis(400));
        assert!(start.elapsed() < Duration::from_millis(800));
        Ok(())
    }

    /// The default budget has to be able to reach a second counter read.
    ///
    /// This replaces a test that asserted the opposite. It pinned a 50ms
    /// budget against a 100ms quiet interval, which is a budget that cannot
    /// complete one comparison, so the automatic settle installed an observer
    /// on every action and never compared it.
    #[test]
    fn the_default_budget_clears_detect_plus_one_quiet_interval() {
        let floor = timeouts::ACTION_SETTLE_NAVIGATION_DETECT + timeouts::ACTION_SETTLE_DOM_QUIET;
        assert!(
            timeouts::ACTION_SETTLE_DEFAULT_TIMEOUT > floor,
            "default settle budget {:?} cannot reach a comparison past {floor:?}",
            timeouts::ACTION_SETTLE_DEFAULT_TIMEOUT
        );
    }

    #[tokio::test]
    async fn the_default_budget_settles_a_quiet_page() -> Result<(), CoreError> {
        let (session, page_id) = harness(VecDeque::new(), VecDeque::new()).await?;
        let outcome = wait_for_action_settle(
            &session.pages,
            page_id,
            timeouts::ACTION_SETTLE_DEFAULT_TIMEOUT,
        )
        .await;
        assert_eq!(
            outcome,
            SettleOutcome::Settled {
                navigation_observed: false
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_budget_below_one_quiet_interval_expires() -> Result<(), CoreError> {
        let (session, page_id) = harness(VecDeque::from([1, 2, 3]), VecDeque::new()).await?;
        let budget = Duration::from_millis(50);
        let start = Instant::now();
        let outcome = wait_for_action_settle(&session.pages, page_id, budget).await;
        assert_eq!(outcome, SettleOutcome::BudgetExpired);
        assert!(start.elapsed() >= budget);
        Ok(())
    }

    async fn stalled_session() -> Result<(Arc<BrowserSession>, crate::PageId), CoreError> {
        let connection = HarnessConnection::new(HarnessState {
            tab_loads: VecDeque::new(),
            current_tab: TabLoad::ready(),
            mutation_reads: VecDeque::new(),
            current_counter: 0,
            fail_evaluate: false,
            readiness: VecDeque::new(),
            readiness_last: reading(0, false, true, true),
            stall_readiness: true,
        });
        let session = BrowserSession::new(connection, BrowserSessionHooks::default());
        let pages = session.pages.list().await?;
        let page = pages
            .first()
            .ok_or_else(|| CoreError::Message("missing harness page".to_string()))?;
        Ok((session, page.page_id.clone()))
    }

    /// A probe that cannot be scheduled must give up on its own budget, not on
    /// the transport's, which is 60s and would strand the caller.
    #[tokio::test]
    async fn an_unschedulable_probe_gives_up_on_the_settle_budget() -> Result<(), CoreError> {
        let (session, page_id) = stalled_session().await?;
        let page = session.pages.get_session(page_id).await?;
        let start = Instant::now();
        let Err(error) = super::read_readiness(&page.session).await else {
            return Err(CoreError::Message(
                "a stalled probe must not answer".to_string(),
            ));
        };
        assert!(
            error.to_string().contains("did not answer"),
            "unexpected error {error}"
        );
        assert!(
            start.elapsed() < timeouts::ACTION_SETTLE_CDP_CALL_TIMEOUT * 4,
            "the probe should give up on its own budget, took {:?}",
            start.elapsed()
        );
        Ok(())
    }

    #[tokio::test]
    async fn evaluate_failure_is_advisory() -> Result<(), CoreError> {
        let connection = HarnessConnection::new(HarnessState {
            tab_loads: VecDeque::new(),
            current_tab: TabLoad::ready(),
            mutation_reads: VecDeque::new(),
            current_counter: 0,
            fail_evaluate: true,
            readiness: VecDeque::new(),
            readiness_last: reading(0, false, true, true),
            stall_readiness: false,
        });
        let session = BrowserSession::new(connection, BrowserSessionHooks::default());
        let pages = session.pages.list().await?;
        let page_id = pages
            .first()
            .ok_or_else(|| CoreError::Message("missing harness page".to_string()))?
            .page_id
            .clone();
        let outcome =
            wait_for_action_settle(&session.pages, page_id, Duration::from_millis(900)).await;
        assert!(
            matches!(outcome, SettleOutcome::Skipped { reason } if reason.contains("evaluate failed"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_showing_page_is_stable_after_one_quiet_interval() -> Result<(), CoreError> {
        let (session, page_id) = harness_reading(
            VecDeque::new(),
            VecDeque::new(),
            VecDeque::from([
                reading(0, true, true, false),
                reading(0, false, true, false),
            ]),
        )
        .await?;
        let start = Instant::now();
        let outcome =
            super::wait_for_page_ready(&session.pages, page_id, timeouts::ARRIVAL_READY_BUDGET)
                .await;
        assert_eq!(outcome, super::Readiness::Stable);
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "a settled page should answer in about one quiet interval, took {:?}",
            start.elapsed()
        );
        Ok(())
    }

    /// Quiet because it has not started is not ready.
    ///
    /// Without this floor the first interval after a tab is created answers
    /// "stable" for a blank document, and the caller acts on nothing.
    #[tokio::test]
    async fn a_blank_page_is_not_called_ready() -> Result<(), CoreError> {
        let (session, page_id) = harness_reading(
            VecDeque::new(),
            VecDeque::new(),
            VecDeque::from([reading(0, false, false, false)]),
        )
        .await?;
        let budget = Duration::from_millis(400);
        let start = Instant::now();
        let outcome = super::wait_for_page_ready(&session.pages, page_id, budget).await;
        assert_eq!(outcome, super::Readiness::StillMoving);
        assert!(start.elapsed() >= Duration::from_millis(300));
        Ok(())
    }

    /// A document that really is empty should not burn the whole ceiling.
    #[tokio::test]
    async fn a_blank_but_complete_document_is_ready() -> Result<(), CoreError> {
        let (session, page_id) = harness_reading(
            VecDeque::new(),
            VecDeque::new(),
            VecDeque::from([reading(0, false, false, true)]),
        )
        .await?;
        let start = Instant::now();
        let outcome =
            super::wait_for_page_ready(&session.pages, page_id, timeouts::ARRIVAL_READY_BUDGET)
                .await;
        assert_eq!(outcome, super::Readiness::Stable);
        assert!(start.elapsed() < Duration::from_millis(500));
        Ok(())
    }

    /// A reinstall means the counter restarted, so the reading that re-armed
    /// it carries nothing to compare against.
    #[tokio::test]
    async fn a_reinstalled_counter_is_never_compared_across() -> Result<(), CoreError> {
        let (session, page_id) = harness_reading(
            VecDeque::new(),
            VecDeque::new(),
            VecDeque::from([
                reading(0, true, true, false),
                reading(0, true, true, false),
                reading(0, true, true, false),
            ]),
        )
        .await?;
        let outcome =
            super::wait_for_page_ready(&session.pages, page_id, Duration::from_millis(400)).await;
        assert_eq!(outcome, super::Readiness::StillMoving);
        Ok(())
    }

    #[tokio::test]
    async fn a_page_still_filling_in_waits_for_it_to_stop() -> Result<(), CoreError> {
        let (session, page_id) = harness_reading(
            VecDeque::new(),
            VecDeque::new(),
            VecDeque::from([
                reading(0, true, true, false),
                reading(4, false, true, false),
                reading(9, false, true, false),
                reading(9, false, true, false),
            ]),
        )
        .await?;
        let start = Instant::now();
        let outcome =
            super::wait_for_page_ready(&session.pages, page_id, timeouts::ARRIVAL_READY_BUDGET)
                .await;
        assert_eq!(outcome, super::Readiness::Stable);
        assert!(
            start.elapsed() >= Duration::from_millis(300),
            "three intervals should have passed, took {:?}",
            start.elapsed()
        );
        Ok(())
    }
}
