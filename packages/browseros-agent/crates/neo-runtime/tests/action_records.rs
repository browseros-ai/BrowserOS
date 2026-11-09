//! Neo: failed calls keep known action labels; unknown commands omit the JSON field.

#[path = "support/fake_browser.rs"]
#[expect(
    dead_code,
    reason = "this test only needs the fake browser's initial tab"
)]
mod fake_browser;

use std::time::Duration;

use fake_browser::{FakeBrowser, PAGE, Replies};
use neo_runtime::{CallResult, RunStatus};
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// Neo: enough time for admission failures on a loaded test machine.
const RUN_TIMEOUT: Duration = Duration::from_secs(15);

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_failed_calls_keep_labels_and_unknown_calls_omit_them() {
    let browser = FakeBrowser::start(Replies::default()).await;
    let code = format!(
        r"
        const commands = [
            ['frame@{PAGE}', 'fill'],
            ['frame@{PAGE}', 'evaluateExpression'],
            ['context@default', 'setHTTPCredentials'],
            ['frame@{PAGE}', 'unknownMethod'],
            ['disposable@missing', 'unknownMethod'],
            ['frame@missing', 'click'],
        ];
        for (const [guid, method] of commands) {{
            await __neo_runtime.call(JSON.stringify({{id: 1, guid, method, params: {{}}}}));
        }}
    "
    );
    let outcome = browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            &code,
            RUN_TIMEOUT,
            CancellationToken::new(),
        ))
        .await;
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    let expected = [
        Some(json!({"kind": "fill", "group": "type"})),
        Some(json!({"kind": "evaluate", "group": "pageScript"})),
        Some(json!({"kind": "setHTTPCredentials", "group": "network"})),
        None,
        None,
        None,
    ];
    assert_eq!(
        outcome.calls.len(),
        expected.len(),
        "every call is recorded"
    );
    for (call, action) in outcome.calls.iter().zip(expected) {
        assert!(matches!(call.result, CallResult::Error { .. }), "{call:?}");
        let record =
            serde_json::to_value(call).unwrap_or_else(|error| panic!("record JSON: {error}"));
        assert_eq!(
            record.get("action"),
            action.as_ref(),
            "{} on {}",
            call.method,
            call.guid
        );
    }
}
