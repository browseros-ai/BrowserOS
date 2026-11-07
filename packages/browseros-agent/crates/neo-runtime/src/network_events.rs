//! Neo: the run's network events from one tab. The tab's watcher (`page.rs` `watch_events`)
//! applies each Network event to the tab's [`NetworkLog`], which answers what Playwright's server
//! would report for it ([`Happening`]); this module turns those into the context's `request`,
//! `response`, `requestFinished` and `requestFailed` event messages, each naming its page, for
//! the events the run listens to on that page or on the context.
//!
//! ```text
//!  Network.requestWillBeSent ─▶ log ─▶ Request ─────────▶ request          {request, page}
//!  Network.responseReceived ──▶ log ─▶ Response ────────▶ response         {response, page}
//!  loadingFinished / redirect ─▶ log ─▶ Finished ────────▶ requestFinished  {request, response, …}
//!  Network.loadingFailed ─────▶ log ─▶ Failed ──────────▶ requestFailed    {request, failureText, …}
//! ```
//!
//! Every object travels as its full `{guid, initializer}` record, which the SDK keeps once per
//! guid: Neo has no `__create__` messages, and a script that listens only to `requestfinished`
//! has seen no `request` event that would have introduced the object. A run that listens to
//! none of these pays one subscription check per happening.

use serde_json::{Value, json};

use crate::events::{CONTEXT_GUID, EventHub};
use crate::network::{Happening, HopId, LogError, NetworkLog};

/// Queues the event of each happening the run listens to, in the order the log reported them.
pub(crate) fn emit(hub: &EventHub, page_guid: &str, log: &NetworkLog, happenings: &[Happening]) {
    for happening in happenings {
        let (listener, method, id) = match happening {
            Happening::Request(id) => ("request", "request", id),
            Happening::Response(id) => ("response", "response", id),
            Happening::Finished(id) => ("requestfinished", "requestFinished", id),
            Happening::Failed(id) => ("requestfailed", "requestFailed", id),
            // Dropped past Neo's cap: Playwright's server has no such step to report.
            Happening::Forgotten(_) => continue,
        };
        if !hub.wants(page_guid, listener, listener) {
            continue;
        }
        // A record the log no longer holds (a chain dropped past its cap) has nothing to send.
        if let Ok(params) = params(log, page_guid, method, id) {
            hub.emit(CONTEXT_GUID, method, &params);
        }
    }
}

/// One event's params, with Playwright's field names.
fn params(log: &NetworkLog, page_guid: &str, method: &str, id: &HopId) -> Result<Value, LogError> {
    let page = json!({ "guid": page_guid });
    Ok(match method {
        // Playwright: `server/dispatchers/browserContextDispatcher.ts` `BrowserContextDispatcher.constructor`: `this._dispatchEvent('request', {`
        "request" => json!({
            "request": log.created_request(id)?,
            "page": page,
        }),
        // Playwright: `server/dispatchers/browserContextDispatcher.ts` `BrowserContextDispatcher.constructor`: `this._dispatchEvent('response', {`
        // Neo adds the response's request record, so the SDK builds both from the event alone,
        // even once the log dropped the hop (a long backlog of undelivered events).
        "response" => {
            let Some(response) = log.created_response(id)? else {
                return Err(LogError::Dropped("response"));
            };
            json!({ "response": response, "request": log.created_request(id)?, "page": page })
        }
        // Playwright: `server/dispatchers/browserContextDispatcher.ts` `BrowserContextDispatcher.constructor`: `responseEndTiming: request._responseEndTiming,`
        "requestFinished" => json!({
            "request": log.created_request(id)?,
            "response": log.created_response(id)?,
            "responseEndTiming": log.response_end(id)?,
            "page": page,
        }),
        _ => {
            let mut params = json!({
                "request": log.created_request(id)?,
                "responseEndTiming": log.response_end(id)?,
                "page": page,
            });
            // Playwright: `server/dispatchers/browserContextDispatcher.ts` `BrowserContextDispatcher.constructor`: `failureText: request._failureText || undefined,`
            if let Some(text) = log.failure(id)?.filter(|text| !text.is_empty()) {
                params["failureText"] = json!(text);
            }
            params
        }
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A subresource that starts, gets its response and finishes, and one that fails.
    fn log() -> NetworkLog {
        let log = NetworkLog::new("T".to_string());
        for (method, params) in [
            (
                "Network.requestWillBeSent",
                json!({
                    "requestId": "1.1", "loaderId": "L", "type": "Fetch", "frameId": "T",
                    "request": {"url": "http://a.test/data.json", "method": "GET", "headers": {}},
                    "timestamp": 1.0, "wallTime": 1000.0,
                }),
            ),
            (
                "Network.responseReceived",
                json!({
                    "requestId": "1.1",
                    "response": {"status": 200, "statusText": "OK", "headers": {}},
                    "hasExtraInfo": false,
                }),
            ),
            (
                "Network.loadingFinished",
                json!({"requestId": "1.1", "timestamp": 1.5, "encodedDataLength": 10.0}),
            ),
            (
                "Network.requestWillBeSent",
                json!({
                    "requestId": "1.2", "loaderId": "L", "type": "Image", "frameId": "C",
                    "request": {"url": "http://b.test/missing.png", "method": "GET", "headers": {}},
                    "timestamp": 2.0, "wallTime": 1001.0,
                }),
            ),
            (
                "Network.loadingFailed",
                json!({"requestId": "1.2", "timestamp": 2.5, "errorText": "net::ERR_FAILED"}),
            ),
        ] {
            log.apply(None, method, &params);
        }
        log
    }

    fn drained(hub: &EventHub) -> Vec<Value> {
        serde_json::from_str(&hub.drain(None)).expect("events")
    }

    #[test]
    fn tests_that_only_subscribed_events_are_queued_with_their_records() {
        let log = log();
        let hub = EventHub::default();
        let happenings = [
            Happening::Request(HopId {
                request: "1.1".to_string(),
                hop: 0,
            }),
            Happening::Finished(HopId {
                request: "1.1".to_string(),
                hop: 0,
            }),
        ];
        emit(&hub, "page@T", &log, &happenings);
        assert!(drained(&hub).is_empty(), "nothing listens");
        hub.subscribe(CONTEXT_GUID, "requestfinished", true);
        emit(&hub, "page@T", &log, &happenings);
        let events = drained(&hub);
        assert_eq!(events.len(), 1, "{events:?}");
        let params = &events[0]["params"];
        assert_eq!(events[0]["method"], "requestFinished");
        assert_eq!(params["page"]["guid"], "page@T");
        assert_eq!(params["request"]["guid"], "request@T:1.1:0");
        assert_eq!(params["request"]["initializer"]["resourceType"], "fetch");
        assert_eq!(params["request"]["initializer"]["frame"]["guid"], "frame@T");
        assert_eq!(params["response"]["guid"], "response@T:1.1:0");
        assert_eq!(params["responseEndTiming"], 500.0);
    }

    #[test]
    fn tests_that_a_failure_names_its_text_and_its_child_frame() {
        let log = log();
        let hub = EventHub::default();
        hub.subscribe("page@T", "requestfailed", true);
        emit(
            &hub,
            "page@T",
            &log,
            &[Happening::Failed(HopId {
                request: "1.2".to_string(),
                hop: 0,
            })],
        );
        let events = drained(&hub);
        let params = &events[0]["params"];
        assert_eq!(params["failureText"], "net::ERR_FAILED");
        assert_eq!(
            params["request"]["initializer"]["frame"]["guid"],
            "frame@T/C"
        );
        assert_eq!(
            params["responseEndTiming"], -1.0,
            "no response: never measured"
        );
    }
}
