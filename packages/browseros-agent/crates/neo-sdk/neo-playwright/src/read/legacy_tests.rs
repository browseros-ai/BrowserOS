//! Pin the JSON boundaries the unmoved page families still call. These expectations are the
//! old helpers' replies, independent of the typed core's serialization derives.

use serde_json::{Value, json};

use super::{element_handle, selector_wait, viewport_box};
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::resolve::FramePath;
use crate::target::{CallHeader, LocatorTarget, Strictness};
use crate::test_env::TestEnv;
use crate::test_support::{ProbeAnswer, ScriptedRun, ScriptedTab, TabScript, TestResult};
use crate::world::Place;
use crate::{ErrorCode, Op};

struct Match {
    found: bool,
}

impl TabScript for Match {
    fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
        ProbeAnswer::reply(if self.found {
            json!({ "kind": "read", "value": {
                "pin": 1, "preview": "<button>Go</button>", "visible": true, "count": 1
            } })
        } else {
            json!({ "kind": "none" })
        })
    }

    fn cdp_reply(&self, method: &str, params: &Value) -> Option<Value> {
        (method == "Runtime.callFunctionOn" && params["arguments"][0]["value"]["op"] == "pinned")
            .then(|| {
                json!({ "result": {
                "type": "object", "subtype": "node", "objectId": "pinned-node"
            } })
            })
    }
}

fn locator(run: &ScriptedRun) -> TestResult<LocatorTarget> {
    Ok(serde_json::from_value(json!({
        "page": run.page, "frame": null,
        "chunks": [{ "parts": [{ "name": "css", "body": "#go", "source": "#go" }] }],
        "display": "locator('#go')"
    }))?)
}

fn header() -> CallHeader {
    CallHeader {
        api: Some("page.waitForSelector".to_owned()),
        timeout_ms: Some(2_000),
    }
}

/// Every field is present in the old brand, including its page tag; there are no nullable
/// optional fields to silently omit when `RootHandle` starts being serialized directly.
async fn old_found(run: &ScriptedRun, scope: &OpScope<'_>) -> TestResult<Value> {
    let page = OpPage::admit_handle(&run.playwright, scope, &run.page).await?;
    let world = page.world(&page.main_endpoint(), Place::Main).await?;
    // A returning tab starts at a randomized sequence. Pin its independently observed
    // world identity, rather than assuming zero or deriving it from the wrapper's reply.
    Ok(
        json!({ "$neo": "element", "page": { "$neo": "page", "tab": 7 },
        "seq": world.seq(), "node": 1, "display": "<button>Go</button>" }),
    )
}

#[tokio::test(start_paused = true)]
async fn query_one_wrapper_preserves_found_and_not_found_values() -> TestResult {
    let op = Op::from_name("locator.elementHandle").ok_or("element handle op missing")?;
    for found in [true, false] {
        let tab = ScriptedTab::new(Match { found });
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, op, header());
        let value =
            element_handle::query_one(&run.playwright, &scope, &locator(&run)?, Strictness::Strict)
                .await?;
        let old = if found {
            old_found(&run, &scope).await?
        } else {
            Value::Null
        };
        assert_eq!(value, old);
        assert_eq!(value.to_string(), old.to_string());
    }
    Ok(())
}

#[tokio::test]
async fn wait_for_selector_wrapper_preserves_found_missing_and_timeout() -> TestResult {
    let op = Op::from_name("locator.waitFor").ok_or("wait op missing")?;
    for found in [true, false] {
        let tab = ScriptedTab::new(Match { found });
        let run = ScriptedRun::over(&tab).await?;
        let target = locator(&run)?;
        let scope = OpScope::new(&run.env, op, header());
        let state = if found {
            selector_wait::SelectorState::Attached
        } else {
            selector_wait::SelectorState::Hidden
        };
        let value = selector_wait::wait_for_selector(
            &run.playwright,
            &scope,
            &target,
            Strictness::Strict,
            state,
            selector_wait::Returning::Handle,
        )
        .await?;
        let old = if found {
            old_found(&run, &scope).await?
        } else {
            Value::Null
        };
        assert_eq!(value, old);
        assert_eq!(value.to_string(), old.to_string());
        if !found {
            let scope = OpScope::new(
                &run.env,
                op,
                CallHeader {
                    timeout_ms: Some(50),
                    ..header()
                },
            );
            let error = selector_wait::wait_for_selector(
                &run.playwright,
                &scope,
                &target,
                Strictness::Strict,
                selector_wait::SelectorState::Attached,
                selector_wait::Returning::Handle,
            )
            .await
            .err()
            .ok_or("missing timeout")?;
            assert_eq!(error.error_code(), Some(ErrorCode::NoMatch));
            assert!(
                error
                    .message()
                    .starts_with("page.waitForSelector: Timeout 50ms exceeded.")
            );
            assert!(error.message().contains("waiting for locator('#go')"));
        }
    }
    Ok(())
}

#[tokio::test]
async fn screenshot_wrapper_preserves_rect_and_null_values() -> TestResult {
    let env = TestEnv::new();
    let op = Op::from_name("page.screenshot").ok_or("screenshot op missing")?;
    let scope = OpScope::new(&env, op, CallHeader::default());
    let old_rect = json!({ "x": 1.25, "y": 2.5, "width": 3.0, "height": 4.0 });
    for old in [old_rect, Value::Null] {
        let value =
            viewport_box::for_screenshot(&scope, &FramePath::default(), old.clone()).await?;
        assert_eq!(value, old);
        assert_eq!(value.to_string(), old.to_string());
    }
    Ok(())
}
