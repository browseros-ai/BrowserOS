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

    async fn harness(
        mutation_reads: impl Into<VecDeque<u64>>,
        tab_loads: impl Into<VecDeque<TabLoad>>,
    ) -> Result<(Arc<BrowserSession>, crate::PageId), CoreError> {
        let connection = HarnessConnection::new(HarnessState {
            tab_loads: tab_loads.into(),
            current_tab: TabLoad::ready(),
            mutation_reads: mutation_reads.into(),
            current_counter: 0,
            fail_evaluate: false,
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

    #[tokio::test]
    async fn automatic_settle_respects_50ms_total_budget() -> Result<(), CoreError> {
        let (session, page_id) = harness(VecDeque::from([1, 2, 3]), VecDeque::new()).await?;
        let budget = timeouts::ACTION_SETTLE_DEFAULT_TIMEOUT;
        assert_eq!(budget, Duration::from_millis(50));
        let start = Instant::now();
        let outcome = wait_for_action_settle(&session.pages, page_id, budget).await;
        assert_eq!(outcome, SettleOutcome::BudgetExpired);
        assert!(start.elapsed() >= budget);
        assert!(start.elapsed() < Duration::from_millis(150));
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
}
