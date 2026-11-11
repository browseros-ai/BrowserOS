use std::error::Error;
use std::sync::Arc;

use neo_trace::ErrorOwner;
use serde_json::json;

use super::{SetContentInput, set_content};
use crate::op_scope::OpScope;
use crate::target::CallHeader;
use crate::test_env::{self, EnvCall, TestEnv};
use crate::test_support::RecordingHooks;
use crate::{ErrorCode, Op, Playwright};

type TestResult = Result<(), Box<dyn Error>>;

fn input(frame: Option<&str>) -> Result<SetContentInput, serde_json::Error> {
    serde_json::from_value(json!({ "page": { "$neo": "page", "tab": 7 },
                                   "frame": frame, "html": "<script>window.ran = 1</script>" }))
}

/// A host serves an op unheld unless its spec declares `MainWorld`, and setContent's HTML runs
/// its scripts in the main world, so the document write's gateway refuses any other op, here
/// one that declares only `Capture`, before it looks up the page or the frame: `neo / internal`
/// with the declaration named, although the browser is gone. `page.setContent` itself goes on
/// to admit its page, which fails here for want of a browser.
/// Paused: core waits out its own attach timeout before it gives up on the page.
#[tokio::test(start_paused = true)]
async fn tests_that_only_an_op_declaring_main_world_writes_a_document() -> TestResult {
    let playwright = Playwright::new(test_env::offline_session(), Arc::new(RecordingHooks));
    let env = TestEnv::new();
    let screenshot = Op::from_name("page.screenshot").ok_or("page.screenshot is an op")?;
    let set_content_op = Op::from_name("page.setContent").ok_or("page.setContent is an op")?;
    for frame in [None, Some("child-frame")] {
        let undeclared = OpScope::new(&env, screenshot, CallHeader::default());
        let refused = set_content(&playwright, &undeclared, input(frame)?)
            .await
            .err()
            .ok_or("an op that does not declare MainWorld writes no document")?;
        assert_eq!(
            (refused.owner(), refused.error_code()),
            (ErrorOwner::Neo, Some(ErrorCode::Internal)),
            "{frame:?}: {refused}"
        );
        assert!(
            refused.message().contains("Capability::MainWorld"),
            "{frame:?}: refused at the gateway, not further on: {refused}"
        );
        let declared = OpScope::new(&env, set_content_op, CallHeader::default());
        let admitting = set_content(&playwright, &declared, input(frame)?)
            .await
            .err()
            .ok_or("no browser, no write")?;
        assert_eq!(
            admitting.error_code(),
            Some(ErrorCode::BrowserDisconnected),
            "{frame:?}: the declared op goes on to admit its page: {admitting}"
        );
    }
    let calls = env.calls();
    assert!(
        !calls
            .iter()
            .any(|call| matches!(call, EnvCall::Recheck | EnvCall::MarkSent)),
        "no op rechecked or marked anything sent: {calls:?}"
    );
    Ok(())
}
