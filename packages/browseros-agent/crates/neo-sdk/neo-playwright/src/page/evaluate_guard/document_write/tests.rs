use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use super::DocumentWrite;
use crate::op_scope::OpScope;
use crate::page::evaluate_guard::DOCUMENT_REPLACED;
use crate::target::CallHeader;
use crate::test_env::EnvCall;
use crate::test_support::{ProbeAnswer, ScriptedRun, ScriptedTab, TabScript, TestResult};
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
/// world, after the host's rules ran again. One that a navigation cut off while it was pending
/// may have run, the HTML's inline scripts and all, so it is not written again: setContent fails
/// as upstream's does when that navigation destroys its evaluation's context.
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
        let write = DocumentWrite::open(&run.playwright, &scope, &run.page, None).await?;
        let replaced = write.replace_document(&scope, "<p>hi</p>").await;
        assert_eq!(
            tab.script().writes.load(Ordering::SeqCst),
            writes,
            "writes sent after the first failed with {first:?}"
        );
        let rechecks = run
            .env
            .calls()
            .iter()
            .filter(|call| **call == EnvCall::Recheck)
            .count();
        assert_eq!(rechecks, writes, "every write after a recheck of its own");
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
