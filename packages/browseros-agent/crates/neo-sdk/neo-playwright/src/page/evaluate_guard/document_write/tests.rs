use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use browseros_core::TabId;
use serde_json::{Value, json};

use crate::op_scope::OpScope;
use crate::page::LoadState;
use crate::page::evaluate_guard::{DOCUMENT_REPLACED, EvaluateGuard};
use crate::target::CallHeader;
use crate::test_env::EnvCall;
use crate::test_support::{
    ProbeAnswer, SCRIPTED_CHILD, ScriptedLink, ScriptedRun, ScriptedTab, TabScript, TestResult,
};
use crate::{ErrorCode, Op};

/// A tab whose first write fails with `first`, Chromium's answer to the call, and whose later
/// writes replace the document.
struct FirstWriteFails {
    first: &'static str,
    writes: AtomicUsize,
}

impl TabScript for FirstWriteFails {
    fn probe(&self, _frame: &str, request: &Value) -> ProbeAnswer {
        if request["op"] != "setContent" {
            return ProbeAnswer::empty_sweep();
        }
        if self.writes.fetch_add(1, Ordering::SeqCst) == 0 {
            ProbeAnswer::error(self.first)
        } else {
            ProbeAnswer::reply(json!({ "kind": "replaced" }))
        }
    }
}

/// A write that found its world gone never ran, so it goes once more in the new document's
/// world. One that a navigation cut off while it was pending may have run, the HTML's inline
/// scripts and all, so it is not written again: setContent fails as upstream's does when that
/// navigation destroys its evaluation's context.
#[tokio::test(start_paused = true)]
async fn tests_that_a_write_cut_off_by_a_navigation_is_not_written_again() -> TestResult {
    for (first, writes, written_again) in [
        ("Cannot find context with specified id", 2, true),
        ("Inspected target navigated or closed", 1, false),
        ("Execution context was destroyed.", 1, false),
    ] {
        let tab = ScriptedTab::new(FirstWriteFails {
            first,
            writes: AtomicUsize::new(0),
        });
        let run = ScriptedRun::over(&tab).await?;
        let op = Op::from_name("page.setContent").ok_or("page.setContent is an op")?;
        let scope = OpScope::new(&run.env, op, CallHeader::default());
        let mut guard =
            EvaluateGuard::open_document(&run.playwright, &scope, &run.page, None).await?;
        let replaced = guard.core.replace_document(&scope, "<p>hi</p>").await;
        assert_eq!(
            tab.script().writes.load(Ordering::SeqCst),
            writes,
            "writes sent after the first failed with {first:?}"
        );
        match replaced {
            Ok(()) => assert!(written_again, "after {first:?}, the write went once more"),
            Err(error) => {
                assert!(!written_again, "after {first:?}: {error}");
                assert_eq!(
                    (error.error_code(), error.message()),
                    (
                        Some(ErrorCode::PageError),
                        format!("page.setContent: {DOCUMENT_REPLACED}").as_str()
                    ),
                    "after {first:?}"
                );
            }
        }
    }
    Ok(())
}

/// The value the document that replaced the first write's holds, which only a sweep after
/// that write finds.
const NEW_DOCUMENTS_VALUE: &str = "typed-in-the-new-document";

/// A tab whose first write finds its world gone, as when the document changed under it, and
/// whose sweeps from then on find [`NEW_DOCUMENTS_VALUE`]: every probe request, in order.
#[derive(Default)]
struct WorldGoneOnce {
    requests: Mutex<Vec<String>>,
}

impl WorldGoneOnce {
    fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TabScript for WorldGoneOnce {
    fn probe(&self, _frame: &str, request: &Value) -> ProbeAnswer {
        let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        let written = requests.iter().any(|sent| sent == "setContent");
        let op = request["op"].as_str().unwrap_or_default().to_owned();
        requests.push(op.clone());
        match (op.as_str(), written) {
            ("setContent", false) => ProbeAnswer::error("Cannot find context with specified id"),
            ("setContent", true) => ProbeAnswer::reply(json!({ "kind": "replaced" })),
            (_, true) => ProbeAnswer::reply(json!({ "kind": "sweep",
                                                    "values": [NEW_DOCUMENTS_VALUE] })),
            (_, false) => ProbeAnswer::empty_sweep(),
        }
    }
}

/// A write tried again goes to another document than the sweep before the first saw: the one
/// that replaced it, whose scripts may have copied a value anywhere, or opened another window.
/// So that document's site is swept, and the host told what it holds, before the host's
/// policies run again and the write goes out.
#[tokio::test(start_paused = true)]
async fn tests_that_a_write_tried_again_sweeps_the_new_document_first() -> TestResult {
    let tab = ScriptedTab::new(WorldGoneOnce::default());
    let run = ScriptedRun::over(&tab).await?;
    let op = Op::from_name("page.setContent").ok_or("page.setContent is an op")?;
    let scope = OpScope::new(&run.env, op, CallHeader::default());
    let mut guard = EvaluateGuard::open_document(&run.playwright, &scope, &run.page, None).await?;
    guard.core.replace_document(&scope, "<p>hi</p>").await?;
    assert_eq!(
        tab.script().requests(),
        ["sweep", "setContent", "sweep", "setContent"],
        "a sweep between the write that found its world gone and the write tried again"
    );
    assert!(
        run.hooks
            .swept()
            .iter()
            .any(|value| value == NEW_DOCUMENTS_VALUE),
        "the host was told the new document's value: {:?}",
        run.hooks.swept()
    );
    let host: Vec<EnvCall> = run
        .env
        .calls()
        .into_iter()
        .filter(|call| matches!(call, EnvCall::Recheck | EnvCall::MarkSent))
        .collect();
    assert_eq!(
        host,
        [
            EnvCall::Recheck,
            EnvCall::MarkSent,
            EnvCall::Recheck,
            EnvCall::MarkSent
        ],
        "the host's policies ran again before the write was sent again"
    );
    Ok(())
}

/// A tab whose writes all find their world gone, and whose same-site child frame answers no
/// sweep within its budget once a write went out.
#[derive(Default)]
struct ChildLateAfterWrite {
    writes: AtomicUsize,
}

impl TabScript for ChildLateAfterWrite {
    fn probe(&self, frame: &str, request: &Value) -> ProbeAnswer {
        if request["op"] == "setContent" {
            self.writes.fetch_add(1, Ordering::SeqCst);
            return ProbeAnswer::error("Cannot find context with specified id");
        }
        let written = self.writes.load(Ordering::SeqCst) > 0;
        ProbeAnswer {
            after: if frame == SCRIPTED_CHILD && written {
                Duration::from_millis(1_200)
            } else {
                Duration::ZERO
            },
            ..ProbeAnswer::empty_sweep()
        }
    }

    fn child(&self) -> bool {
        true
    }
}

/// The first write reached no document, so nothing ran, and the sweep before the write tried
/// again cannot reach a frame of the site: the write does not go again, and setContent fails
/// at once with Neo's refusal, saying the content was not set.
#[tokio::test(start_paused = true)]
async fn tests_that_a_retry_that_cannot_sweep_says_the_content_was_not_set() -> TestResult {
    let tab = ScriptedTab::new(ChildLateAfterWrite::default());
    let run = ScriptedRun::over(&tab).await?;
    let op = Op::from_name("page.setContent").ok_or("page.setContent is an op")?;
    let header = CallHeader {
        api: None,
        timeout_ms: Some(2_500),
    };
    let scope = OpScope::new(&run.env, op, header);
    let guard = EvaluateGuard::open_document(&run.playwright, &scope, &run.page, None).await?;
    let error = guard
        .write(&scope, "<p>hi</p>", LoadState::Load)
        .await
        .err()
        .ok_or("a write whose retry cannot sweep fails")?;
    assert_eq!(
        tab.script().writes.load(Ordering::SeqCst),
        1,
        "the write did not go again"
    );
    assert_eq!(
        (error.error_code(), error.message()),
        (
            Some(ErrorCode::Unsupported),
            "page.setContent: a frame of the script's site did not answer Neo's privacy sweep \
             within 1000ms, so the content was not set; try again once the frame responds"
        ),
    );
    Ok(())
}

/// A tab whose same-site child frame misses its sweep budget on its first sweep only.
#[derive(Default)]
struct ChildLateOnce {
    child_sweeps: AtomicUsize,
}

impl TabScript for ChildLateOnce {
    fn probe(&self, frame: &str, _request: &Value) -> ProbeAnswer {
        let first =
            frame == SCRIPTED_CHILD && self.child_sweeps.fetch_add(1, Ordering::SeqCst) == 0;
        ProbeAnswer {
            after: if first {
                Duration::from_millis(1_200)
            } else {
                Duration::ZERO
            },
            ..ProbeAnswer::empty_sweep()
        }
    }

    fn child(&self) -> bool {
        true
    }
}

/// The sweep before a write is any guard's sweep: a frame of the site late on its first try
/// is swept again on a fresh budget inside the op's timeout (M6b's late retry), and the write
/// may go once it answered, with nothing sent before.
#[tokio::test(start_paused = true)]
async fn tests_that_a_frame_late_once_is_swept_again_before_the_write() -> TestResult {
    let tab = ScriptedTab::new(ChildLateOnce::default());
    let run = ScriptedRun::over(&tab).await?;
    let op = Op::from_name("page.setContent").ok_or("page.setContent is an op")?;
    let header = CallHeader {
        api: None,
        timeout_ms: Some(5_000),
    };
    let scope = OpScope::new(&run.env, op, header);
    let opened = EvaluateGuard::open_document(&run.playwright, &scope, &run.page, None).await;
    assert!(opened.is_ok(), "opened once the frame answered");
    assert_eq!(
        tab.script().child_sweeps.load(Ordering::SeqCst),
        2,
        "the late frame was swept again"
    );
    assert!(
        run.env.calls().is_empty(),
        "nothing was rechecked or sent before the write: {:?}",
        run.env.calls()
    );
    Ok(())
}

/// The value the document that replaced a write's holds, swept before the write goes again.
const RETRY_VALUE: &str = "typed-before-the-retry";

/// A tab whose writes all find their world gone, and whose sweeps, once a write went out, find
/// [`RETRY_VALUE`] in the main frame and never answer in the same-site child.
#[derive(Default)]
struct ChildStallsAfterWrite {
    writes: AtomicUsize,
}

impl TabScript for ChildStallsAfterWrite {
    fn probe(&self, frame: &str, request: &Value) -> ProbeAnswer {
        if request["op"] == "setContent" {
            self.writes.fetch_add(1, Ordering::SeqCst);
            return ProbeAnswer::error("Cannot find context with specified id");
        }
        if self.writes.load(Ordering::SeqCst) == 0 {
            return ProbeAnswer::empty_sweep();
        }
        if frame == SCRIPTED_CHILD {
            return ProbeAnswer {
                after: Duration::from_secs(600),
                ..ProbeAnswer::empty_sweep()
            };
        }
        ProbeAnswer::reply(json!({ "kind": "sweep", "values": [RETRY_VALUE] }))
    }

    fn child(&self) -> bool {
        true
    }
}

/// The sweep before a write tried again runs to the op's deadline at the latest and reports
/// what the frames that answered found, though a same-site frame stalls past it: the write's
/// timeout never drops it whole (M2R review F2). The timeout comes before the frame's own
/// budget ends, so the op fails with upstream's timeout, not Neo's refusal.
#[tokio::test(start_paused = true)]
async fn tests_that_a_retry_sweep_the_timeout_cuts_reports_what_it_found() -> TestResult {
    let tab = ScriptedTab::new(ChildStallsAfterWrite::default());
    let run = ScriptedRun::over(&tab).await?;
    let op = Op::from_name("page.setContent").ok_or("page.setContent is an op")?;
    let header = CallHeader {
        api: None,
        timeout_ms: Some(500),
    };
    let scope = OpScope::new(&run.env, op, header);
    let guard = EvaluateGuard::open_document(&run.playwright, &scope, &run.page, None).await?;
    let error = guard
        .write(&scope, "<p>hi</p>", LoadState::Load)
        .await
        .err()
        .ok_or("a retry whose sweep stalls past the timeout fails")?;
    assert_eq!(error.error_code(), Some(ErrorCode::WaitTimeout), "{error}");
    assert!(
        error
            .message()
            .starts_with("page.setContent: Timeout 500ms exceeded."),
        "{error}"
    );
    assert!(
        run.hooks.swept().iter().any(|value| value == RETRY_VALUE),
        "the main frame's value reached the host: {:?}",
        run.hooks.swept()
    );
    Ok(())
}

/// The popup [`PopupBeforeRetry`]'s page opens while its first write is in flight: a page of
/// another site, which no sweep of the page's site reads, but whose window the written HTML's
/// scripts can act on.
const POPUP: ScriptedLink = ScriptedLink {
    target: "popup-of-the-new-document",
    url: "https://b.test/",
    tab: Some(12),
};

/// A tab whose first write finds its world gone, as when the document changed under it, and
/// whose page opened [`POPUP`] by then: the browser lists the popup from the first write on.
#[derive(Default)]
struct PopupBeforeRetry {
    writes: AtomicUsize,
}

impl TabScript for PopupBeforeRetry {
    fn probe(&self, _frame: &str, request: &Value) -> ProbeAnswer {
        if request["op"] != "setContent" {
            return ProbeAnswer::empty_sweep();
        }
        if self.writes.fetch_add(1, Ordering::SeqCst) == 0 {
            ProbeAnswer::error("Cannot find context with specified id")
        } else {
            ProbeAnswer::reply(json!({ "kind": "replaced" }))
        }
    }

    fn linked(&self) -> Option<ScriptedLink> {
        (self.writes.load(Ordering::SeqCst) > 0).then_some(POPUP)
    }
}

/// The document a write is tried again in may have opened a window the sweep before the first
/// write never saw. The retry's sweep finds it, so the host's rules run on its tab before the
/// write goes out again: the first write asked about no other tab, and the one tried again
/// asks about the popup.
#[tokio::test(start_paused = true)]
async fn tests_that_a_write_tried_again_checks_the_tabs_its_new_document_opened() -> TestResult {
    let tab = ScriptedTab::new(PopupBeforeRetry::default());
    let run = ScriptedRun::over(&tab).await?;
    let op = Op::from_name("page.setContent").ok_or("page.setContent is an op")?;
    let scope = OpScope::new(&run.env, op, CallHeader::default());
    let mut guard = EvaluateGuard::open_document(&run.playwright, &scope, &run.page, None).await?;
    guard.core.replace_document(&scope, "<p>hi</p>").await?;
    assert_eq!(
        run.env.calls(),
        [
            EnvCall::Recheck,
            EnvCall::MarkSent,
            EnvCall::Recheck,
            EnvCall::RecheckReach(vec![TabId(12)]),
            EnvCall::MarkSent,
        ],
    );
    Ok(())
}
