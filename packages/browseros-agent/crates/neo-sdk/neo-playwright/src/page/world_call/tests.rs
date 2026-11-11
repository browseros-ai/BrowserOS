use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use browseros_core::BrowserView;
use browseros_core::testing::{self, TestConnection};
use serde_json::{Value, json};
use tokio::time::{self, Instant};

use crate::test_env::{EnvCall, TestEnv};
use crate::test_support::{self, RecordingHooks};
use crate::{ErrorCode, Op, Playwright};

type TestResult = Result<(), Box<dyn Error>>;

/// The timeout every op below is called with.
const TIMEOUT: Duration = Duration::from_millis(2_000);

/// Upstream's timeout covers a whole `setContent`, `waitForFunction` or `waitForURL` call, the
/// frame's world included, so a renderer that never answers its world's install holds none of
/// them past it. Each one ends at its timeout, with upstream's message and nothing rechecked
/// or sent; the main-world ops end before the gateway's first sweep, the only part with a
/// budget of its own. The frame forms name a child frame, which is entered from the main
/// frame's world, the one that never installs here.
/// Paused: the ops' clocks run on tokio's.
#[tokio::test(start_paused = true)]
async fn tests_that_an_op_ends_at_its_timeout_while_its_frames_world_never_installs() -> TestResult
{
    let connection = TestConnection::new([]);
    let test_support::OneTab { browser } = test_support::one_tab(&connection).await?;
    // The session's setup for its worlds answers, and the world itself never installs.
    connection.always("Runtime.addBinding", json!({}))?;
    connection.always("Page.createIsolatedWorld", testing::cdp_pending())?;
    let playwright = Playwright::new(Arc::clone(&browser), Arc::new(RecordingHooks::default()));
    let brand = json!({ "$neo": "page", "tab": 7 });
    let script = json!({ "run": { "expression": "() => true", "isFunction": true,
                                  "arg": r#"{"v":"undefined"}"# } });
    let millis = TIMEOUT.as_millis();
    let cases: [(&str, &str, Value); 3] = [
        (
            "page.setContent",
            "page.setContent",
            json!({ "page": brand, "html": "<script>window.ran = 1</script>" }),
        ),
        (
            "page.waitForFunction",
            "frame.waitForFunction",
            json!({ "page": brand, "frame": "child-frame", "script": script }),
        ),
        (
            "page.waitForURL",
            "frame.waitForURL",
            json!({ "page": brand, "frame": "child-frame", "url": { "glob": "**/done" } }),
        ),
    ];
    for (name, api, mut input) in cases {
        let op = Op::from_name(name).ok_or(format!("{name} is an op"))?;
        if let Some(input) = input.as_object_mut() {
            input.insert(
                "call".to_owned(),
                json!({ "api": api, "timeoutMs": millis }),
            );
        }
        let env = TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None));
        let started = Instant::now();
        let ended = time::timeout(Duration::from_secs(60), playwright.invoke(&env, op, input))
            .await
            .map_err(|_elapsed| format!("{api} still ran a minute after its {millis}ms timeout"))?;
        let took = started.elapsed();
        let error = ended
            .err()
            .ok_or(format!("{api} passed with no world installed"))?;
        assert_eq!(
            error.error_code(),
            Some(ErrorCode::WaitTimeout),
            "{api}: {error}"
        );
        assert!(
            error
                .message()
                .starts_with(&format!("{api}: Timeout {millis}ms exceeded.")),
            "{api}: upstream's timeout message: {error}"
        );
        assert!(
            took >= TIMEOUT && took < TIMEOUT + Duration::from_millis(100),
            "{api}: ended at its timeout, after {took:?}"
        );
        if api == "page.setContent" {
            assert!(
                error
                    .message()
                    .contains("setting frame content, waiting until \"load\""),
                "upstream logs its line before it looks for the frame's world: {error}"
            );
        }
        let calls = env.calls();
        assert!(
            !calls
                .iter()
                .any(|call| matches!(call, EnvCall::Recheck | EnvCall::MarkSent)),
            "{api}: nothing was rechecked or sent: {calls:?}"
        );
    }
    Ok(())
}
