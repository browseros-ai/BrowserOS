//! A navigation op's result as telemetry counts it: its reply's HTTP status, or the network
//! failure neo-playwright kept apart from its message.
//!
//! It reads the live result the op is about to answer with, never the trace, so the `trace`
//! settings change no number. The match over the crate's failure is exhaustive (STYLE.md P8): a
//! new variant fails the build here instead of reaching a metric under a guessed label.

use neo_playwright::{NetFailure, PlaywrightError};
use neo_telemetry::Navigated;
use neo_telemetry::label::{HttpStatus, NetError};
use serde_json::Value;

/// The ops that navigate a page's document, each counted once as a navigation.
const NAVIGATIONS: [&str; 4] = ["page.goto", "page.reload", "page.goBack", "page.goForward"];

/// The navigation `op` ran, as telemetry counts it, or `None` for an op that navigates nothing.
/// Its status is the reply's `response.status`, when it is an HTTP one; its error, the network
/// failure the crate kept apart from its message. The op's own outcome is not read: a goto
/// answered 404 is still an `ok` op.
#[must_use]
pub(super) fn navigated(op: &str, result: &Result<Value, PlaywrightError>) -> Option<Navigated> {
    if !NAVIGATIONS.contains(&op) {
        return None;
    }
    Some(match result {
        Ok(reply) => Navigated {
            status: reply
                .pointer("/response/status")
                .and_then(Value::as_u64)
                .and_then(|code| u16::try_from(code).ok())
                .and_then(HttpStatus::new),
            error: None,
        },
        Err(error) => Navigated {
            status: None,
            error: error.net_failure().map(net_error),
        },
    })
}

fn net_error(failure: &NetFailure) -> NetError {
    match failure {
        NetFailure::Named(text) => NetError::of_chromium(text),
        NetFailure::ErrorPage => NetError::Other,
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use neo_playwright::{ErrorCode, NetFailure, PlaywrightError};
    use neo_telemetry::label::{HttpStatus, NetError, RunStatus};
    use neo_telemetry::{Navigated, RunFacts, RunVerdict, Telemetry, TelemetryConfig};
    use serde_json::{Value, json};

    use super::navigated;
    use crate::telemetry_rig::Scrape;

    /// A navigation op's reply, as neo-playwright serializes it.
    fn reply(status: &Value) -> Value {
        json!({
            "response": {
                "url": "https://example.com/",
                "status": status,
                "statusText": "",
                "httpVersion": "HTTP/1.1",
                "ok": false,
                "headers": {},
            },
            "url": "https://example.com/",
            "frameSnapshotAttempted": true,
        })
    }

    fn failed(code: ErrorCode, net: Option<NetFailure>) -> Result<Value, PlaywrightError> {
        let error = PlaywrightError::new(code, "page.goto: failed".to_owned());
        Err(match net {
            Some(net) => error.with_net_failure(net),
            None => error,
        })
    }

    #[test]
    fn only_the_four_navigation_ops_are_navigations() {
        for op in ["page.goto", "page.reload", "page.goBack", "page.goForward"] {
            assert!(
                navigated(op, &Ok(reply(&json!(200)))).is_some(),
                "{op} navigates"
            );
        }
        for op in [
            "page.waitForNavigation",
            "page.waitForURL",
            "page.evaluate",
            "page.title",
            "neo.solveCaptcha",
        ] {
            assert_eq!(
                navigated(op, &Ok(reply(&json!(200)))),
                None,
                "{op} may see a navigation, but none of its own"
            );
        }
    }

    #[test]
    fn a_status_is_an_http_one_or_none() {
        for (status, expected) in [
            (json!(200), HttpStatus::new(200)),
            (json!(404), HttpStatus::new(404)),
            (json!(599), HttpStatus::new(599)),
            (json!(0), None),
            (json!(99), None),
            (json!(600), None),
            (json!(70_000), None),
            (json!(-1), None),
            (json!("404"), None),
            (Value::Null, None),
        ] {
            assert_eq!(
                navigated("page.goto", &Ok(reply(&status))),
                Some(Navigated {
                    status: expected,
                    error: None
                }),
                "a reply with status {status}"
            );
        }
        assert_eq!(
            navigated("page.goBack", &Ok(json!({ "response": null, "url": null }))),
            Some(Navigated {
                status: None,
                error: None
            }),
            "a same-document step reads no response"
        );
    }

    #[test]
    fn only_a_network_failure_is_a_net_error() {
        let cases = [
            (
                failed(
                    ErrorCode::NavigationFailed,
                    Some(NetFailure::Named("net::ERR_NAME_NOT_RESOLVED".into())),
                ),
                Some(NetError::NameNotResolved),
            ),
            (
                failed(
                    ErrorCode::NavigationFailed,
                    Some(NetFailure::Named("net::ERR_QUIC_PROTOCOL_ERROR".into())),
                ),
                Some(NetError::Other),
            ),
            (
                failed(ErrorCode::NavigationFailed, Some(NetFailure::ErrorPage)),
                Some(NetError::Other),
            ),
            (failed(ErrorCode::NavigationFailed, None), None),
            (failed(ErrorCode::WaitTimeout, None), None),
            (failed(ErrorCode::TargetCrashed, None), None),
        ];
        for (result, error) in cases {
            assert_eq!(
                navigated("page.reload", &result),
                Some(Navigated {
                    status: None,
                    error
                }),
                "{result:?}"
            );
        }
    }

    /// What the mapping hands over, through the live pipeline to the scrape: a 404 goto and a
    /// net error are both navigations, and only the second is a navigation error.
    #[test]
    fn navigations_reach_the_scrape_under_their_closed_labels() -> Result<(), Box<dyn Error>> {
        let (telemetry, _probe) = Telemetry::in_memory(&TelemetryConfig::default());
        let run = telemetry.start_run(RunFacts {
            run_id: &"run-1",
            task_id: None,
            parent: None,
        });
        for result in [
            Ok(reply(&json!(404))),
            Ok(reply(&json!(200))),
            failed(
                ErrorCode::NavigationFailed,
                Some(NetFailure::Named("net::ERR_CONNECTION_REFUSED".into())),
            ),
        ] {
            let navigation = navigated("page.goto", &result).ok_or("a goto navigates")?;
            run.ops().navigated(navigation);
        }
        run.finish(RunVerdict {
            status: RunStatus::Ok,
            error: None,
        });
        assert_eq!(
            Scrape::of(&telemetry)?
                .lines_of(&["neo_navigations_total", "neo_navigation_errors_total"]),
            [
                r#"neo_navigation_errors_total{error_type="connection_refused"} 1"#,
                "neo_navigations_total 1",
                r#"neo_navigations_total{http_response_status_code="200"} 1"#,
                r#"neo_navigations_total{http_response_status_code="404"} 1"#,
            ],
            "a 404 goto is a navigation like any other; the refused one is also an error"
        );
        Ok(())
    }
}
