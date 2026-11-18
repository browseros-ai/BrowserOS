use std::error::Error;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use browseros_core::TabId;
use futures_util::future;
use serde_json::{Value, json};
use tokio::time::{self, Instant};

use crate::Op;
use crate::test_support::{
    ProbeAnswer, SCRIPTED_NEW, ScriptedRun, ScriptedTab, TabScript, TestResult, client_timeout,
};

/// How late the new tab's renderer answers its first lifecycle command: past a `goto`'s own
/// 30 s, as a hidden tab's renderer answered on a loaded machine.
const FIRST_ANSWER: Duration = Duration::from_secs(40);

/// A browser whose new tab answers its first `Page.setLifecycleEventsEnabled` only after
/// [`FIRST_ANSWER`], and every other command at once.
#[derive(Default)]
struct SlowNewTab {
    lifecycles: AtomicUsize,
}

impl TabScript for SlowNewTab {
    fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
        ProbeAnswer::unexpected()
    }

    fn delay(&self, method: &str) -> Duration {
        if method == "Page.setLifecycleEventsEnabled"
            && self.lifecycles.fetch_add(1, Ordering::SeqCst) == 0
        {
            FIRST_ANSWER
        } else {
            Duration::ZERO
        }
    }
}

/// One op call as the door sends it: the op and its input.
struct Call {
    op: Op,
    input: Value,
}

/// `context.newPage` as the door sends it, with no Playwright timeout.
fn new_page() -> Result<Call, Box<dyn Error>> {
    Ok(Call {
        op: Op::from_name("context.newPage").ok_or("context.newPage is an op")?,
        input: json!({ "call": { "api": "context.newPage", "timeoutMs": 0 } }),
    })
}

/// `newPage` returns once the tab it opened answered its first lifecycle command, as upstream's
/// `newPage` waits for its page's setup, so that wait counts against no op's own timeout: the
/// first `goto` on the tab finds its lifecycle command answered at once.
#[tokio::test(start_paused = true)]
async fn tests_that_new_page_returns_once_its_tab_answers() -> TestResult {
    let tab = ScriptedTab::new(SlowNewTab::default());
    let run = ScriptedRun::over(&tab).await?;
    let Call { op, input } = new_page()?;
    let started = Instant::now();
    let opened = run.playwright.invoke(&run.env, op, input).await?;
    let took = started.elapsed();
    assert_eq!(opened["page"]["tab"], 9, "the new tab: {opened}");
    assert!(
        took >= FIRST_ANSWER,
        "newPage waited for its tab's first answer: {took:?}"
    );
    assert_eq!(
        tab.script().lifecycles.load(Ordering::SeqCst),
        1,
        "newPage enabled the tab's lifecycle events"
    );
    Ok(())
}

/// The host drops an op at the run's deadline or on a stop. `newPage`'s wait for its tab is part
/// of the op, so it ends there and not when the tab answers, and leaves nothing behind: no call
/// still waiting, nothing the next op waits on.
#[tokio::test(start_paused = true)]
async fn tests_that_the_runs_deadline_ends_new_pages_wait() -> TestResult {
    let tab = ScriptedTab::new(SlowNewTab::default());
    let run = ScriptedRun::over(&tab).await?;
    let Call { op, input } = new_page()?;
    let run_deadline = Duration::from_secs(10);
    let started = Instant::now();
    let cut = time::timeout(run_deadline, run.playwright.invoke(&run.env, op, input)).await;
    assert!(
        cut.is_err(),
        "the run's deadline came before the tab's answer"
    );
    assert_eq!(
        started.elapsed(),
        run_deadline,
        "newPage ended at the run's deadline"
    );
    assert_eq!(
        tab.slowed_in_flight(),
        0,
        "no call of newPage's wait outlived the op"
    );
    let pages = Op::from_name("context.pages").ok_or("context.pages is an op")?;
    let listed = time::timeout(
        Duration::from_millis(1),
        run.playwright
            .invoke(&run.env, pages, json!({ "call": {} })),
    )
    .await;
    assert!(
        matches!(listed, Ok(Ok(_))),
        "the next op waits on nothing newPage held: {listed:?}"
    );
    Ok(())
}

/// A tab that closes while `newPage` waits for its first answer, which can then never come, ends
/// the wait at once: `newPage` returns the tab it opened, as it did before it waited, and the
/// tab's first op meets the closed tab.
#[tokio::test(start_paused = true)]
async fn tests_that_new_page_stops_waiting_when_its_tab_closes() -> TestResult {
    let tab = ScriptedTab::new(SlowNewTab::default());
    let run = ScriptedRun::over(&tab).await?;
    let Call { op, input } = new_page()?;
    let closes_after = Duration::from_secs(5);
    let started = Instant::now();
    let opening = async {
        let opened = run.playwright.invoke(&run.env, op, input).await;
        (opened, started.elapsed())
    };
    let closing = async {
        time::sleep(closes_after).await;
        tab.emit(
            "Target.detachedFromTarget",
            json!({ "sessionId": "closed", "targetId": SCRIPTED_NEW }),
        );
    };
    let ((opened, took), ()) = future::join(opening, closing).await;
    let opened = opened?;
    assert_eq!(opened["page"]["tab"], 9, "the tab it opened: {opened}");
    assert_eq!(
        took, closes_after,
        "newPage waited until its tab closed, and no longer"
    );
    assert_eq!(
        tab.slowed_in_flight(),
        0,
        "the lifecycle call went with the wait"
    );
    Ok(())
}

/// A browser whose new tab answers its first `method` only after the CDP client gave up on it,
/// and every command at once.
struct AnswersAfterTheClient {
    method: &'static str,
    gave_up: AtomicBool,
}

impl AnswersAfterTheClient {
    fn on(method: &'static str) -> Self {
        Self {
            method,
            gave_up: AtomicBool::new(false),
        }
    }
}

impl TabScript for AnswersAfterTheClient {
    fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
        ProbeAnswer::unexpected()
    }

    fn outlasts_client(&self, method: &str) -> bool {
        method == self.method && !self.gave_up.swap(true, Ordering::SeqCst)
    }
}

/// A first answer that comes after the CDP client's own request timeout is not the end of
/// `newPage`'s wait: the step the client gave up on is sent again, and `newPage` returns once the
/// tab answered it. Core's attach that timed out at its `Page.enable` is admitted again, from a
/// new `Target.attachToTarget`, and then the tab's lifecycle events are enabled; a lifecycle
/// command that timed out is sent again on the same session.
#[tokio::test(start_paused = true)]
async fn tests_that_new_page_sends_again_what_the_client_gave_up_on() -> TestResult {
    for (late, attaches, lifecycles) in [
        ("Page.enable", 2, 1),
        ("Page.setLifecycleEventsEnabled", 1, 2),
    ] {
        let tab = ScriptedTab::new(AnswersAfterTheClient::on(late));
        let run = ScriptedRun::over(&tab).await?;
        let Call { op, input } = new_page()?;
        let started = Instant::now();
        let opened = run.playwright.invoke(&run.env, op, input).await?;
        let took = started.elapsed();
        assert_eq!(opened["page"]["tab"], 9, "the new tab: {opened}");
        assert_eq!(
            took,
            client_timeout(),
            "{late}: newPage waited past the client's timeout, for the answer after it"
        );
        // Worker discovery can attach an already-listed tab concurrently with newPage.
        // Count at least the required retry; the completed lifecycle setup and the elapsed
        // client timeout above prove that the caller waited through the failed attempt.
        assert!(
            tab.sent("Target.attachToTarget") >= attaches,
            "{late}: the attach was retried"
        );
        assert_eq!(
            tab.sent("Page.setLifecycleEventsEnabled"),
            lifecycles,
            "{late}: newPage waited for its lifecycle setup"
        );
    }
    Ok(())
}

/// The tab `newPage` opened is named to the host as soon as the browser named it, before the
/// long wait for its first answer: a host that hides tabs from a run must not hide this one
/// from the run's other ops meanwhile.
#[tokio::test(start_paused = true)]
async fn tests_that_new_page_names_its_tab_before_it_readies_it() -> TestResult {
    let tab = ScriptedTab::new(SlowNewTab::default());
    let run = ScriptedRun::over(&tab).await?;
    let Call { op, input } = new_page()?;
    let opening = run.playwright.invoke(&run.env, op, input);
    let early = async {
        time::sleep(Duration::from_secs(1)).await;
        run.env.opened()
    };
    let (opened, named_early) = future::join(opening, early).await;
    opened?;
    assert_eq!(
        named_early,
        vec![TabId(9)],
        "named a second in, while its first answer is still 39 s away"
    );
    Ok(())
}

/// `context.pages()` leaves out a tab the host hides from the run, as neo-server hides another
/// task's tab from a run bound to its own.
#[tokio::test]
async fn tests_that_context_pages_leaves_out_a_tab_the_host_hides() -> TestResult {
    let tab = ScriptedTab::new(SlowNewTab::default());
    let run = ScriptedRun::over(&tab).await?;
    let pages = Op::from_name("context.pages").ok_or("context.pages is an op")?;
    let listed = run
        .playwright
        .invoke(&run.env, pages, json!({ "call": {} }))
        .await?;
    assert_eq!(listed["pages"][0]["page"]["tab"], 7, "{listed}");
    run.env.hide(TabId(7));
    let hidden = run
        .playwright
        .invoke(&run.env, pages, json!({ "call": {} }))
        .await?;
    assert_eq!(hidden["pages"], json!([]), "{hidden}");
    Ok(())
}
