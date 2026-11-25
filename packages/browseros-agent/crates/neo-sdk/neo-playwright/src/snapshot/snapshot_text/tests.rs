//! The text path's rows (design 5.6 and the wait's design): what each format answers, the event
//! wait inside a whole page's handler (its one run, its flag, its bound, the send count around
//! it), the clock's windows on a scripted clock, and the compare switch's one read.
//!
//! The handler rows run the real op over a [`ScriptedTab`] on tokio's paused clock, so a wait's
//! length is measured exactly: a run whose listener has a binding call queued while its pump is
//! in flight holds every event wait to its whole bound ([`held_listener`]).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::error::Error;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use browseros_core::{TabId, TargetId};
use neo_snapshot::{AriaNode, Call, Form, MaxTokens, Passed, Snapshot, TextForm};
use serde_json::{Value, json};
use tokio::time::{self, Instant};

use super::{CallClock, Finish, Look, finish_page_text};
use crate::events::{BindingCalled, PageEventKind, QueuedCall, Subscription};
use crate::probe::PROBE_PROTOCOL;
use crate::snapshot::{Baseline, SnapshotBaselines, SnapshotForm, TextForm as StoredTextForm};
use crate::test_env::TestEnv;
use crate::test_support::{ProbeAnswer, SCRIPTED_MAIN, ScriptedRun, ScriptedTab, TabScript};
use crate::{ErrorCode, Op, OpEnv, PlaywrightError, RunState, TimedOut};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// The whole bound of an op's event wait (`EVENTS_BEFORE_REPLY`).
const WAIT: Duration = Duration::from_secs(2);

/// The tree every scripted frame answers: a main section with one button.
fn tree() -> Value {
    json!([{ "role": "main", "ref": "e2",
        "children": [{ "role": "button", "name": "Go", "ref": "e3" }] }])
}

/// The text `render` prints for [`tree`] by `call`: what the op must answer.
fn printed(call: Call<'_>) -> TestResult<String> {
    let tree: Vec<AriaNode> = serde_json::from_value(tree())?;
    let snapshot = Snapshot {
        tree,
        ..Snapshot::default()
    };
    Ok(neo_snapshot::render(&snapshot, call).into_text())
}

/// One tab whose main frame answers [`tree`], after `after`, or with the probe's internal error.
#[derive(Default)]
struct OneMain {
    after: Duration,
    broken: bool,
    /// Snapshot requests the frame answered: one per read of the page.
    reads: AtomicUsize,
    seq: AtomicU64,
}

impl TabScript for OneMain {
    fn cdp_reply(&self, method: &str, params: &Value) -> Option<Value> {
        if method == "Runtime.callFunctionOn"
            && let Some(seq) = params["arguments"][0]["value"]["frameSeq"].as_u64()
        {
            self.seq.store(seq, Ordering::SeqCst);
        }
        // Clickable marking reads an empty, checked document.
        (method == "DOMSnapshot.captureSnapshot")
            .then(|| json!({ "documents": [{}], "strings": [] }))
    }

    fn probe(&self, _frame: &str, request: &Value) -> ProbeAnswer {
        match request["op"].as_str() {
            Some("snapshot") => {
                self.reads.fetch_add(1, Ordering::SeqCst);
                if self.broken {
                    return ProbeAnswer::unexpected();
                }
                ProbeAnswer {
                    after: self.after,
                    reply: Ok(json!({ "kind": "snapshot", "json": tree(), "iframes": [] })),
                }
            }
            Some("describe") => ProbeAnswer::reply(json!({ "kind": "described",
                "protocol": PROBE_PROTOCOL, "seq": self.seq.load(Ordering::SeqCst),
                "url": "https://a.test/" })),
            Some("read") => ProbeAnswer::reply(json!({ "kind": "read", "value": true })),
            Some("resolve") => ProbeAnswer::reply(
                json!({ "kind": "element", "pin": 1, "preview": "<main></main>" }),
            ),
            _ => ProbeAnswer::unexpected(),
        }
    }
}

fn op(name: &str) -> TestResult<Op> {
    Ok(Op::from_name(name).ok_or(format!("no op {name}"))?)
}

/// `page.ariaSnapshot`'s input on the scripted tab, with a `timeout` in ms (0: none).
fn page_input(run: &ScriptedRun, timeout: u64, rest: &Value) -> Value {
    let mut input = json!({ "call": { "timeoutMs": timeout }, "page": run.page });
    if let (Some(input), Some(rest)) = (input.as_object_mut(), rest.as_object()) {
        input.extend(rest.clone());
    }
    input
}

/// `locator.ariaSnapshot`'s input on the main section.
fn locator_input(run: &ScriptedRun, rest: &Value) -> Value {
    let mut input = json!({ "target": { "$neo": "locator", "page": run.page,
        "chunks": [{ "parts": [{ "name": "css", "body": "main", "source": "main" }] }],
        "display": "locator('main')" }, "strict": "strict" });
    if let (Some(input), Some(rest)) = (input.as_object_mut(), rest.as_object()) {
        input.extend(rest.clone());
    }
    input
}

/// A fresh run on the same browser and connection: its own state, as the host gives each run.
fn next_env(run: &ScriptedRun) -> TestEnv {
    TestEnv::over(run.env.view().clone())
}

/// The run of `env` listens on the tab, a binding call waits in its queue and a pump is in
/// flight: every event wait of an op of that run is then held to its whole bound.
fn held_listener<'run>(run: &ScriptedRun, env: &'run TestEnv) -> impl Sized + 'run {
    let state = env.run_state();
    run.playwright.events().bind(state);
    let target = TargetId(SCRIPTED_MAIN.to_owned());
    state.events.listen(
        Subscription::Page {
            tab: TabId(7),
            target: target.clone(),
            event: PageEventKind::Binding,
        },
        0,
        0,
    );
    state.events.queue_call(QueuedCall {
        seq: 1,
        at_ms: 0,
        tab: TabId(7),
        call: BindingCalled {
            target,
            context: 3,
            frame: None,
            payload: "{}".to_owned(),
        },
    });
    state.events.pumping()
}

/// What the tab's store holds for its current document once `env`'s run ended ok with every
/// text in its reply.
fn settled(run: &ScriptedRun, env: &TestEnv) -> Option<Baseline> {
    env.run_state().settle_snapshot_baselines(true, &|_| true);
    let store = run.playwright.snapshot_baselines();
    let target = TargetId(SCRIPTED_MAIN.to_owned());
    let document = store.generation(&target);
    store.stored(&target, document)
}

#[tokio::test(start_paused = true)]
async fn tests_that_a_format_answers_rusts_text_and_no_format_the_tree() -> TestResult {
    let tab = ScriptedTab::new(OneMain::default());
    let run = ScriptedRun::over(&tab).await?;
    let page = op("page.ariaSnapshot")?;
    let passed = Passed::default();
    let fraction = Passed {
        text: Some(false),
        urls: Some(true),
        max_tokens: MaxTokens::new(2.5),
        inert: Some(true),
    };
    for (rest, expected) in [
        (
            json!({ "mode": "ai", "format": { "neo": {} } }),
            printed(Call::Page(&passed))?,
        ),
        (
            json!({ "mode": "ai", "format": { "neo": { "text": false, "urls": true,
                "maxTokens": 2.5, "inert": true } } }),
            printed(Call::Page(&fraction))?,
        ),
        (
            json!({ "mode": "ai", "format": { "full": {} } }),
            printed(Call::Full { max_tokens: None })?,
        ),
        (
            json!({ "mode": "ai", "format": { "full": { "maxTokens": 2.5 } } }),
            printed(Call::Full {
                max_tokens: MaxTokens::new(2.5),
            })?,
        ),
        (
            json!({ "mode": "default", "format": "playwright" }),
            printed(Call::Playwright)?,
        ),
        (
            json!({ "mode": "ai", "format": "playwright" }),
            printed(Call::Playwright)?,
        ),
    ] {
        let answer = run
            .playwright
            .invoke(&run.env, page, page_input(&run, 0, &rest))
            .await?;
        assert_eq!(
            answer,
            json!({ "snapshot": expected }),
            "{rest}: the text alone"
        );
    }
    let locator = op("locator.ariaSnapshot")?;
    for (rest, expected) in [
        (
            json!({ "mode": "ai", "format": { "neo": {} } }),
            printed(Call::Element(&passed))?,
        ),
        (
            json!({ "mode": "ai", "format": { "full": {} } }),
            printed(Call::Full { max_tokens: None })?,
        ),
        (
            json!({ "mode": "default", "format": "playwright" }),
            printed(Call::Playwright)?,
        ),
    ] {
        let answer = run
            .playwright
            .invoke(&run.env, locator, locator_input(&run, &rest))
            .await?;
        assert_eq!(answer, json!({ "snapshot": expected }), "locator {rest}");
    }
    let tree_path = run
        .playwright
        .invoke(
            &run.env,
            page,
            page_input(&run, 0, &json!({ "mode": "ai", "clickable": true })),
        )
        .await?;
    assert_eq!(tree_path["json"], tree(), "no format: the tree, as before");
    assert!(tree_path.get("document").is_some() && tree_path.get("snapshotCall").is_some());
    assert!(tree_path.get("snapshot").is_none());
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn tests_that_the_json_ops_answer_the_tree_unmarked() -> TestResult {
    let tab = ScriptedTab::new(OneMain::default());
    let run = ScriptedRun::over(&tab).await?;
    let answer = run
        .playwright
        .invoke(
            &run.env,
            op("page.ariaSnapshotJSON")?,
            json!({ "page": run.page, "mode": "ai" }),
        )
        .await?;
    assert_eq!(answer, json!({ "snapshot": tree() }));
    let answer = run
        .playwright
        .invoke(
            &run.env,
            op("locator.ariaSnapshotJSON")?,
            locator_input(&run, &json!({ "mode": "default" })),
        )
        .await?;
    assert_eq!(answer, json!({ "snapshot": tree() }));
    assert_eq!(
        tab.sent("DOMSnapshot.captureSnapshot"),
        0,
        "upstream's command marks nothing"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn tests_that_pairs_the_prelude_never_sends_fail_before_the_page_is_read() -> TestResult {
    let tab = ScriptedTab::new(OneMain::default());
    let run = ScriptedRun::over(&tab).await?;
    let page = op("page.ariaSnapshot")?;
    for (rest, code) in [
        (
            json!({ "mode": "ai", "clickable": true, "format": { "neo": {} } }),
            ErrorCode::Internal,
        ),
        (
            json!({ "mode": "ai", "compare": true }),
            ErrorCode::Internal,
        ),
        (
            json!({ "mode": "default", "format": { "neo": {} } }),
            ErrorCode::Internal,
        ),
        (
            json!({ "mode": "ai", "format": { "full": { "maxTokens": 0 } } }),
            ErrorCode::Internal,
        ),
        (
            json!({ "mode": "ai", "format": { "neo": { "maxTokens": -1 } } }),
            ErrorCode::Internal,
        ),
        (
            json!({ "mode": "ai", "format": { "neo": { "cap": 1 } } }),
            ErrorCode::InvalidInput,
        ),
    ] {
        let error = run
            .playwright
            .invoke(&run.env, page, page_input(&run, 0, &rest))
            .await
            .err()
            .ok_or(format!("{rest} is refused"))?;
        assert_eq!(
            error.error_code(),
            Some(code),
            "{rest}: {}",
            error.message()
        );
    }
    let error = run
        .playwright
        .invoke(
            &run.env,
            op("locator.ariaSnapshot")?,
            locator_input(
                &run,
                &json!({ "mode": "ai", "format": { "neo": { "delta": true } } }),
            ),
        )
        .await
        .err()
        .ok_or("a locator's format carries no delta")?;
    assert_eq!(error.error_code(), Some(ErrorCode::InvalidInput));
    assert_eq!(
        tab.script().reads.load(Ordering::SeqCst),
        0,
        "refused before the page is read"
    );
    Ok(())
}

/// A whole page in Neo's format records its text as pending in its own form, once the run
/// settles it is the tab's baseline, and the next run's whole page finds it: the reply's tip.
#[tokio::test(start_paused = true)]
async fn tests_that_a_whole_page_records_its_text_and_a_repeat_notes_the_tip() -> TestResult {
    let tab = ScriptedTab::new(OneMain::default());
    let run = ScriptedRun::over(&tab).await?;
    let page = op("page.ariaSnapshot")?;
    let neo = json!({ "mode": "ai", "format": { "neo": {} } });
    let answer = run
        .playwright
        .invoke(&run.env, page, page_input(&run, 0, &neo))
        .await?;
    assert!(run.env.run_state().full_snapshot_repeats().is_empty());
    let stored = settled(&run, &run.env).ok_or("the page became the tab's baseline")?;
    assert_eq!(Some(stored.text.as_str()), answer["snapshot"].as_str());
    assert_eq!(
        stored.form,
        SnapshotForm {
            text: StoredTextForm::Kept,
            urls: false,
            max_tokens: 8000.0,
            inert: false,
        }
    );
    let next = next_env(&run);
    run.playwright
        .invoke(&next, page, page_input(&run, 0, &neo))
        .await?;
    assert_eq!(next.run_state().full_snapshot_repeats(), vec![TabId(7)]);
    Ok(())
}

/// The wait runs once for a whole page in Neo's format, inside the handler, and the generic one
/// is skipped: the call is held for one bound, not two. Every other text call is held once by
/// the generic wait. With no listener nothing waits.
#[tokio::test(start_paused = true)]
async fn tests_that_each_text_call_is_held_once_and_no_listener_holds_none() -> TestResult {
    let tab = ScriptedTab::new(OneMain::default());
    let run = ScriptedRun::over(&tab).await?;
    let page = op("page.ariaSnapshot")?;
    for rest in [
        json!({ "mode": "ai", "format": { "neo": {} } }),
        json!({ "mode": "ai", "format": { "full": {} } }),
        json!({ "mode": "default", "format": "playwright" }),
        json!({ "mode": "ai", "format": { "neo": {} }, "compare": true }),
        json!({ "mode": "ai", "clickable": true }),
    ] {
        let quiet = next_env(&run);
        let started = Instant::now();
        run.playwright
            .invoke(&quiet, page, page_input(&run, 0, &rest))
            .await?;
        assert_eq!(started.elapsed(), Duration::ZERO, "{rest}: no listener");
        let listening = next_env(&run);
        let _held = held_listener(&run, &listening);
        let started = Instant::now();
        run.playwright
            .invoke(&listening, page, page_input(&run, 0, &rest))
            .await?;
        assert_eq!(started.elapsed(), WAIT, "{rest}: one wait, never both");
    }
    Ok(())
}

/// The inside wait keeps its whole length though the call's 1 s timeout passes during it, as a
/// successful op's wait does; the reply it holds past the timeout is late: the text is returned,
/// nothing is recorded (no tip either), and the older baseline stays.
#[tokio::test(start_paused = true)]
async fn tests_that_the_wait_keeps_its_length_and_a_reply_it_holds_past_the_timeout_is_late()
-> TestResult {
    let tab = ScriptedTab::new(OneMain::default());
    let run = ScriptedRun::over(&tab).await?;
    let page = op("page.ariaSnapshot")?;
    run.playwright
        .invoke(
            &run.env,
            page,
            page_input(&run, 0, &json!({ "mode": "ai", "format": { "neo": {} } })),
        )
        .await?;
    let older = settled(&run, &run.env).ok_or("an older baseline")?;
    let late = next_env(&run);
    let _held = held_listener(&run, &late);
    let started = Instant::now();
    let rest = json!({ "mode": "ai", "format": { "neo": { "text": false } } });
    let answer = run
        .playwright
        .invoke(&late, page, page_input(&run, 1_000, &rest))
        .await?;
    assert_eq!(started.elapsed(), WAIT, "not cut to the call's timeout");
    let never = Passed {
        text: Some(false),
        ..Passed::default()
    };
    assert_eq!(answer, json!({ "snapshot": printed(Call::Page(&never))? }));
    assert!(late.run_state().full_snapshot_repeats().is_empty());
    assert_eq!(
        settled(&run, &late),
        Some(older),
        "returned, not recorded: the older baseline stays"
    );
    Ok(())
}

/// A tree that timed out answers at once: its handler failed before the fence, so the flag is
/// unset and the generic rule applies, which holds no timed-out op. A handler that failed
/// before the fence in any other way keeps the generic wait.
#[tokio::test(start_paused = true)]
async fn tests_that_a_handler_that_fails_before_the_fence_keeps_the_generic_rule() -> TestResult {
    let neo = json!({ "mode": "ai", "format": { "neo": {} } });
    let slow = ScriptedTab::new(OneMain {
        after: Duration::from_millis(1_500),
        ..OneMain::default()
    });
    let run = ScriptedRun::over(&slow).await?;
    let _held = held_listener(&run, &run.env);
    let started = Instant::now();
    let error = run
        .playwright
        .invoke(
            &run.env,
            op("page.ariaSnapshot")?,
            page_input(&run, 1_000, &neo),
        )
        .await
        .err()
        .ok_or("the tree times out")?;
    assert_eq!(error.timed_out(), TimedOut::Yes, "{}", error.message());
    assert_eq!(started.elapsed(), Duration::from_millis(1_000));
    let broken = ScriptedTab::new(OneMain {
        broken: true,
        ..OneMain::default()
    });
    let run = ScriptedRun::over(&broken).await?;
    let _held = held_listener(&run, &run.env);
    let started = Instant::now();
    let error = run
        .playwright
        .invoke(
            &run.env,
            op("page.ariaSnapshot")?,
            page_input(&run, 0, &neo),
        )
        .await
        .err()
        .ok_or("the frame's read fails")?;
    assert_eq!(error.timed_out(), TimedOut::No);
    assert_eq!(started.elapsed(), WAIT, "the generic wait, once");
    Ok(())
}

/// Dropping the op mid-wait (the host's cancel, the run's deadline) ends it, and nothing is
/// recorded; a script's abort during the wait fails the call at the next look of the clock,
/// records nothing, and is not held again by the generic wait.
#[tokio::test(start_paused = true)]
async fn tests_that_a_cancel_or_an_abort_during_the_wait_records_nothing() -> TestResult {
    let tab = ScriptedTab::new(OneMain::default());
    let run = ScriptedRun::over(&tab).await?;
    let page = op("page.ariaSnapshot")?;
    let neo = json!({ "mode": "ai", "format": { "neo": {} } });
    let cancelled = next_env(&run);
    {
        let _held = held_listener(&run, &cancelled);
        let call = run
            .playwright
            .invoke(&cancelled, page, page_input(&run, 0, &neo));
        let dropped = time::timeout(WAIT / 2, call).await;
        assert!(dropped.is_err(), "still held when the host drops it");
    }
    assert_eq!(settled(&run, &cancelled), None, "a cancel records nothing");
    let aborted = next_env(&run);
    let _held = held_listener(&run, &aborted);
    let started = Instant::now();
    let mut call = pin!(
        run.playwright
            .invoke(&aborted, page, page_input(&run, 0, &neo))
    );
    assert!(time::timeout(WAIT / 2, call.as_mut()).await.is_err());
    aborted.abort("stop");
    let error = call.await.err().ok_or("the abort fails the call")?;
    assert_eq!(error.error_code(), Some(ErrorCode::Aborted));
    assert_eq!(started.elapsed(), WAIT, "held once");
    assert_eq!(settled(&run, &aborted), None, "an abort records nothing");
    Ok(())
}

/// What the browser receives while a call of `run` with `rest` is parked in its event wait, until
/// it answers.
async fn sent_during_the_wait(
    tab: &ScriptedTab<OneMain>,
    run: &ScriptedRun,
    rest: &Value,
) -> TestResult<Vec<String>> {
    let env = next_env(run);
    let _held = held_listener(run, &env);
    let mut call = pin!(run.playwright.invoke(
        &env,
        op("page.ariaSnapshot")?,
        page_input(run, 0, rest)
    ));
    assert!(time::timeout(WAIT / 2, call.as_mut()).await.is_err());
    let parked = tab.sent_methods().len();
    call.await?;
    Ok(tab
        .sent_methods()
        .get(parked..)
        .unwrap_or_default()
        .to_vec())
}

/// Nothing is sent to the browser between the fence and the reply. The guarantee is structural:
/// `finish_page_text`, everything after the fence, is given no page, world or sender. This row
/// is its regression guard, for the handler around it. A `mode: 'full'` call's handler has
/// returned before its generic wait, so what the browser receives during that wait is the
/// browser side's own traffic (the page registry's listings); a whole page in Neo's format,
/// parked in the wait inside its handler, must add nothing to it. An admission after the fence
/// (which can attach to the page and send, as each baseline op of the prelude's path did) adds
/// what `a_send_after_the_fence_is_counted` shows an admission sends.
#[tokio::test(start_paused = true)]
async fn tests_that_nothing_is_sent_between_the_fence_and_the_reply() -> TestResult {
    let tab = ScriptedTab::new(OneMain::default());
    let run = ScriptedRun::over(&tab).await?;
    let control = sent_during_the_wait(
        &tab,
        &run,
        &json!({ "mode": "ai", "format": { "full": {} } }),
    )
    .await?;
    let inside = sent_during_the_wait(
        &tab,
        &run,
        &json!({ "mode": "ai", "format": { "neo": { "delta": false } } }),
    )
    .await?;
    assert_eq!(
        tab.script().reads.load(Ordering::SeqCst),
        2,
        "each tree is in"
    );
    assert_eq!(inside, control, "the op sent after its fence");
    assert!(
        inside.iter().all(|method| method == "Browser.getTabs"),
        "{inside:?}"
    );
    Ok(())
}

/// The count above sees an admission: one made after a snapshot's tree, by the prelude's path's
/// baseline op, sends a command the browser side's own listing never does.
#[tokio::test(start_paused = true)]
async fn a_send_after_the_fence_is_counted() -> TestResult {
    let tab = ScriptedTab::new(OneMain::default());
    let run = ScriptedRun::over(&tab).await?;
    run.playwright
        .invoke(
            &run.env,
            op("page.ariaSnapshot")?,
            page_input(&run, 0, &json!({ "mode": "ai", "format": { "neo": {} } })),
        )
        .await?;
    let before = tab.sent_methods().len();
    run.playwright
        .invoke(
            &run.env,
            op("page.snapshotBaseline")?,
            json!({ "page": run.page }),
        )
        .await?;
    let admitted = tab
        .sent_methods()
        .get(before..)
        .unwrap_or_default()
        .to_vec();
    assert!(
        admitted.iter().any(|method| method != "Browser.getTabs"),
        "an admission sends: {admitted:?}"
    );
    Ok(())
}

/// Design 2.3's compare switch on the op's side: one read of the page answers the tree the
/// prelude renders (with its document and call id, for its own baseline ops) and Rust's text
/// and pending record beside it; nothing is recorded, so the prelude's path records as without
/// the switch.
#[tokio::test(start_paused = true)]
async fn tests_that_compare_answers_the_tree_and_rusts_text_from_one_read() -> TestResult {
    let tab = ScriptedTab::new(OneMain::default());
    let run = ScriptedRun::over(&tab).await?;
    let answer = run
        .playwright
        .invoke(
            &run.env,
            op("page.ariaSnapshot")?,
            page_input(
                &run,
                0,
                &json!({ "mode": "ai", "format": { "neo": {} }, "compare": true }),
            ),
        )
        .await?;
    assert_eq!(tab.script().reads.load(Ordering::SeqCst), 1, "one read");
    assert_eq!(answer["json"], tree());
    assert!(answer.get("document").is_some() && answer.get("snapshotCall").is_some());
    let text = printed(Call::Page(&Passed::default()))?;
    assert_eq!(
        answer["rust"],
        json!({ "snapshot": text, "pending": { "text": text,
            "form": { "text": "kept", "urls": false, "maxTokens": 8000.0 } } })
    );
    assert!(run.env.run_state().full_snapshot_repeats().is_empty());
    assert_eq!(settled(&run, &run.env), None, "nothing recorded");
    let answer = run
        .playwright
        .invoke(
            &run.env,
            op("locator.ariaSnapshot")?,
            locator_input(
                &run,
                &json!({ "mode": "ai", "format": { "neo": {} }, "compare": true }),
            ),
        )
        .await?;
    assert_eq!(answer["json"], tree());
    assert_eq!(
        answer["rust"],
        json!({ "snapshot": printed(Call::Element(&Passed::default()))? }),
        "a locator keeps nothing"
    );
    Ok(())
}

/// A script of looks at the call's clock, in order; a look past its end fails the row.
struct Looks(RefCell<VecDeque<Result<Look, PlaywrightError>>>);

impl Looks {
    fn of(looks: Vec<Result<Look, PlaywrightError>>) -> Self {
        Self(RefCell::new(looks.into()))
    }

    fn left(&self) -> usize {
        self.0.borrow().len()
    }
}

impl CallClock for Looks {
    fn look(&self) -> Result<Look, PlaywrightError> {
        self.0.borrow_mut().pop_front().unwrap_or_else(|| {
            Err(PlaywrightError::new(
                ErrorCode::Internal,
                "the clock was looked at more often than scripted".to_owned(),
            ))
        })
    }
}

/// What [`finish_page_text`] is handed, for one tab and document.
struct Page {
    snapshot: Snapshot,
    store: Arc<SnapshotBaselines>,
    run: RunState,
    tab: TargetId,
    document: u32,
}

impl Page {
    fn new() -> TestResult<Self> {
        let store = Arc::new(SnapshotBaselines::default());
        let tab = TargetId("T".to_owned());
        let document = store.generation(&tab);
        Ok(Self {
            snapshot: Snapshot {
                tree: serde_json::from_value(tree())?,
                ..Snapshot::default()
            },
            store,
            run: RunState::default(),
            tab,
            document,
        })
    }

    fn finish(&self, passed: &Passed, clock: &dyn CallClock) -> Result<String, PlaywrightError> {
        self.call(passed, false, clock)
    }

    fn delta(&self, passed: &Passed, clock: &dyn CallClock) -> Result<String, PlaywrightError> {
        self.call(passed, true, clock)
    }

    fn call(
        &self,
        passed: &Passed,
        delta: bool,
        clock: &dyn CallClock,
    ) -> Result<String, PlaywrightError> {
        finish_page_text(
            &Finish {
                snapshot: &self.snapshot,
                passed,
                delta,
                tab: &self.tab,
                tab_id: TabId(7),
                document: self.document,
                store: &self.store,
                run: &self.run,
            },
            clock,
        )
    }

    /// The tab's baseline once the run settled ok with every text given.
    fn settled(&self) -> Option<Baseline> {
        self.run.settle_snapshot_baselines(true, &|_| true);
        self.store.stored(&self.tab, self.document)
    }
}

/// Design 1.7's third window on a scripted clock: in time, a whole page records its own text in
/// the form it was printed in; late, the text is returned and nothing is recorded, the older
/// baseline staying; aborted, the call fails and records nothing. A whole page looks once, before
/// the record.
#[test]
fn tests_that_a_whole_page_records_only_in_time() -> TestResult {
    let passed = Passed {
        text: Some(true),
        ..Passed::default()
    };
    let text = printed(Call::Page(&passed))?;
    let in_time = Page::new()?;
    let clock = Looks::of(vec![Ok(Look::InTime)]);
    assert_eq!(in_time.finish(&passed, &clock)?, text);
    assert_eq!(clock.left(), 0, "one look");
    let stored = in_time.settled().ok_or("recorded in time")?;
    assert_eq!(stored.text, text);
    assert_eq!(
        stored.form.form(),
        Some(Form {
            text: TextForm::Always,
            urls: false,
            max_tokens: MaxTokens::default(),
            inert: false,
        })
    );
    let late = Page::new()?;
    let older = Baseline {
        text: "older".to_owned(),
        form: SnapshotForm::of(Form {
            text: TextForm::Kept,
            urls: false,
            max_tokens: MaxTokens::default(),
            inert: false,
        }),
    };
    late.store.commit(&late.tab, late.document, older.clone());
    assert_eq!(
        late.finish(&passed, &Looks::of(vec![Ok(Look::Late)]))?,
        text
    );
    assert!(
        late.run.full_snapshot_repeats().is_empty(),
        "no tip when late"
    );
    assert_eq!(late.settled(), Some(older), "the older baseline stays");
    let aborted = Page::new()?;
    let abort = PlaywrightError::new(ErrorCode::Aborted, "page.ariaSnapshot: stop".to_owned());
    let error = aborted
        .finish(&passed, &Looks::of(vec![Err(abort)]))
        .err()
        .ok_or("an abort fails the call")?;
    assert_eq!(error.error_code(), Some(ErrorCode::Aborted));
    assert_eq!(aborted.settled(), None, "an abort records nothing");
    Ok(())
}

/// The tip, as the prelude's `page.snapshotPending` noted it: a whole page in time on a tab
/// whose store holds a text of this document could have been a delta.
#[test]
fn tests_that_the_tip_is_noted_in_time_on_a_stored_text_only() -> TestResult {
    let passed = Passed::default();
    let first = Page::new()?;
    first.finish(&passed, &Looks::of(vec![Ok(Look::InTime)]))?;
    assert!(
        first.run.full_snapshot_repeats().is_empty(),
        "nothing stored yet"
    );
    let repeat = Page::new()?;
    let stored = repeat.settled();
    assert_eq!(stored, None);
    repeat.store.commit(
        &repeat.tab,
        repeat.document,
        Baseline {
            text: "kept".to_owned(),
            form: SnapshotForm::of(Form {
                text: TextForm::Kept,
                urls: false,
                max_tokens: MaxTokens::default(),
                inert: false,
            }),
        },
    );
    repeat.finish(&passed, &Looks::of(vec![Ok(Look::InTime)]))?;
    assert_eq!(repeat.run.full_snapshot_repeats(), vec![TabId(7)]);
    Ok(())
}

/// The tab's text for the page in its default form, stored for the tab's document.
fn stored_page(page: &Page) -> TestResult<String> {
    let text = printed(Call::Page(&Passed::default()))?;
    page.store.commit(
        &page.tab,
        page.document,
        Baseline {
            text: text.clone(),
            form: SnapshotForm::of(Form {
                text: TextForm::Kept,
                urls: false,
                max_tokens: MaxTokens::default(),
                inert: false,
            }),
        },
    );
    Ok(text)
}

/// Design 1.7's second window on a scripted clock: a delta looks before it reads the stored
/// text. Late there, the stored text is not read: the whole page under the no-time line, nothing
/// recorded and no second look. In time, it compares with the stored text (or says there is
/// none), and records the page in the stored form with its answer as what the reply must show,
/// unless the second look is late. An abort at the first look fails the call.
#[test]
fn tests_that_a_delta_reads_the_stored_text_only_in_time() -> TestResult {
    let passed = Passed::default();
    let late = Page::new()?;
    stored_page(&late)?;
    // A second look is scripted in time, so a delta that read the stored text anyway answers
    // from it here instead of failing on a look the script lacks.
    let clock = Looks::of(vec![Ok(Look::Late), Ok(Look::InTime)]);
    let text = late.delta(&passed, &clock)?;
    assert!(
        text.starts_with(
            "# whole page: no time was left to look for an earlier snapshot of this page\n"
        ),
        "{text}"
    );
    assert_eq!(
        clock.left(),
        1,
        "one look, and none before a record it does not make"
    );
    assert_eq!(
        late.settled().map(|baseline| baseline.text),
        Some(printed(Call::Page(&passed))?),
        "the stored text stays"
    );

    let unchanged = Page::new()?;
    let stored = stored_page(&unchanged)?;
    let clock = Looks::of(vec![Ok(Look::InTime), Ok(Look::InTime)]);
    let answer = unchanged.delta(&passed, &clock)?;
    assert_eq!(answer, "# no change since your last snapshot of this tab");
    assert_eq!(clock.left(), 0);
    assert!(
        unchanged.run.full_snapshot_repeats().is_empty(),
        "a delta is no tip"
    );
    // The page in the stored form is kept, if the reply shows the answer.
    unchanged
        .run
        .settle_snapshot_baselines(true, &|shown| shown == answer);
    assert_eq!(
        unchanged
            .store
            .stored(&unchanged.tab, unchanged.document)
            .map(|baseline| baseline.text),
        Some(stored)
    );

    let first = Page::new()?;
    let answer = first.delta(
        &passed,
        &Looks::of(vec![Ok(Look::InTime), Ok(Look::InTime)]),
    )?;
    assert!(
        answer.starts_with("# whole page: no earlier snapshot of this page\n"),
        "{answer}"
    );
    assert!(
        first.settled().is_some(),
        "the whole page is the first baseline"
    );

    let late_record = Page::new()?;
    stored_page(&late_record)?;
    let answer = late_record.delta(
        &Passed {
            text: Some(false),
            ..Passed::default()
        },
        &Looks::of(vec![Ok(Look::InTime), Ok(Look::Late)]),
    )?;
    assert!(
        answer.starts_with("# whole page:"),
        "options that disagree: {answer}"
    );
    assert_eq!(
        late_record.settled().map(|baseline| baseline.form.text),
        Some(StoredTextForm::Kept),
        "returned, not recorded: the stored text stays"
    );

    let aborted = Page::new()?;
    stored_page(&aborted)?;
    let abort = PlaywrightError::new(ErrorCode::Aborted, "page.ariaSnapshot: stop".to_owned());
    let error = aborted
        .delta(&passed, &Looks::of(vec![Err(abort)]))
        .err()
        .ok_or("an abort fails the delta")?;
    assert_eq!(error.error_code(), Some(ErrorCode::Aborted));
    Ok(())
}

/// Through the op: a delta after a whole page the run's reply showed compares with it, and under
/// the compare switch Rust reads the stored text itself and answers the same, recording nothing.
#[tokio::test(start_paused = true)]
async fn tests_that_a_delta_through_the_op_compares_with_the_stored_text() -> TestResult {
    let tab = ScriptedTab::new(OneMain::default());
    let run = ScriptedRun::over(&tab).await?;
    let page = op("page.ariaSnapshot")?;
    run.playwright
        .invoke(
            &run.env,
            page,
            page_input(&run, 0, &json!({ "mode": "ai", "format": { "neo": {} } })),
        )
        .await?;
    assert!(settled(&run, &run.env).is_some());
    let delta = json!({ "mode": "ai", "format": { "neo": { "delta": true } } });
    let next = next_env(&run);
    let answer = run
        .playwright
        .invoke(&next, page, page_input(&run, 0, &delta))
        .await?;
    assert_eq!(
        answer,
        json!({ "snapshot": "# no change since your last snapshot of this tab" })
    );
    let compared = next_env(&run);
    let mut input = page_input(&run, 0, &delta);
    input["compare"] = json!(true);
    let answer = run.playwright.invoke(&compared, page, input).await?;
    assert_eq!(
        answer["rust"]["snapshot"],
        json!("# no change since your last snapshot of this tab")
    );
    assert_eq!(answer["json"], tree());
    compared
        .run_state()
        .settle_snapshot_baselines(true, &|_| true);
    assert!(
        compared.run_state().full_snapshot_repeats().is_empty(),
        "the compared call records nothing"
    );
    Ok(())
}

/// A delta never compares with a text pending in its own run: the whole page this run took is
/// not the tab's baseline until the run ends with it in the caller's hands, so a delta later in
/// the same run finds no stored text, and after a committed one it compares with that.
#[tokio::test(start_paused = true)]
async fn tests_that_a_delta_never_compares_with_a_text_pending_in_its_own_run() -> TestResult {
    let tab = ScriptedTab::new(OneMain::default());
    let run = ScriptedRun::over(&tab).await?;
    let page = op("page.ariaSnapshot")?;
    let whole = json!({ "mode": "ai", "format": { "neo": {} } });
    let delta = json!({ "mode": "ai", "format": { "neo": { "delta": true } } });
    run.playwright
        .invoke(&run.env, page, page_input(&run, 0, &whole))
        .await?;
    let answer = run
        .playwright
        .invoke(&run.env, page, page_input(&run, 0, &delta))
        .await?;
    let text = answer["snapshot"].as_str().unwrap_or_default();
    assert!(
        text.starts_with("# whole page: no earlier snapshot of this page\n"),
        "the run's own whole page is pending, not stored: {text}"
    );
    Ok(())
}
