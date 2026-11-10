//! Neo: `context.debugger`, Playwright's server `Debugger` over a run's own calls. A script asks
//! it to pause (`requestPause`), and the next call Playwright's debugger stops at waits before it
//! starts, until another, not yet awaited, branch of the script resumes it, steps to the next call
//! (`next`) or runs to a location (`runTo`). `page.pause()` stops there once the debugger is
//! enabled. Each context has its own: the run's (`debugger@default`) and one per context the run
//! created (`debugger@<browserContextId>`), which see only their own context's calls.
//!
//! ```text
//!  Dispatcher::call (sync, in call order)  Debuggers::before_call → paused? record it, queue
//!                                          `pausedStateChanged`
//!  the call's task                         waits for the resume, the run's end or cancellation,
//!                                          then starts the call and its own timeout
//!  debugger.resume / next / runTo          wakes it; next / runTo say where to stop again
//! ```
//!
//! Its event is queued whether or not anything listens yet: a script pauses a call and only then
//! attaches its `once('pausedstatechanged')`, and the pause is decided inside the call's native
//! send, before that listener exists. The SDK delivers it in order with every other event.

use std::collections::HashMap;
use std::sync::Mutex;

use neo_runtime_spec::serve::{DebuggerCall, DebuggerRunToParamsLocation, MetadataLocation};
use neo_runtime_spec::{MethodFlag, MethodMeta};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::events::EventHub;
use crate::progress::{CallError, PlaywrightError};

/// Neo: the guid prefix of a context's debugger, `debugger@default` or `debugger@<id>`.
pub(crate) const DEBUGGER_PREFIX: &str = "debugger@";

/// Neo: the key of the run's own context among the debuggers.
pub(crate) const DEFAULT_CONTEXT: &str = "default";

/// Playwright: `server/debugger.ts` `Debugger.doResume`: `throw new Error('Debugger is not paused');`
const NOT_PAUSED: &str = "Debugger is not paused";

/// Playwright: `server/debugger.ts` `Debugger.requestPause`: `throw new Error('Debugger is already paused');`
const ALREADY_PAUSED: &str = "Debugger is already paused";

/// Playwright: `server/dispatchers/debuggerDispatcher.ts` `DebuggerDispatcher._serializePausedDetails`: `file: metadata.location?.file ?? '<unknown>',`
const UNKNOWN_FILE: &str = "<unknown>";

/// The run's debuggers, one per context key, made on first use.
#[derive(Default)]
pub(crate) struct Debuggers(Mutex<HashMap<String, State>>);

/// Playwright's `Debugger` fields a run can reach (no recorder, no `_enable` streaming).
#[derive(Default)]
struct State {
    enabled: bool,
    pause_at: PauseAt,
    pause_before_waiting_actions: bool,
    paused: Option<Paused>,
}

/// Playwright's `PauseAt`.
#[derive(Default)]
struct PauseAt {
    next: bool,
    location: Option<DebuggerRunToParamsLocation>,
}

/// Playwright's `_pausedCall`: its details as the event and getter report them, and the call's
/// wake-up.
struct Paused {
    details: Value,
    resume: oneshot::Sender<()>,
}

/// What the debugger needs to know of a call before it starts.
pub(crate) struct CallToGate<'a> {
    /// The context the call's object belongs to.
    pub(crate) context: &'a str,
    pub(crate) meta: &'static MethodMeta,
    pub(crate) internal: bool,
    pub(crate) location: Option<&'a MetadataLocation>,
    pub(crate) params: &'a Value,
    /// `metadata.title`, which overrides the method's.
    pub(crate) title: Option<&'a str>,
}

/// The guid of `context`'s debugger.
pub(crate) fn guid(context: &str) -> String {
    format!("{DEBUGGER_PREFIX}{context}")
}

impl Debuggers {
    /// Playwright's `Debugger.onBeforeCall`: whether this call stops before it starts. A stopped
    /// call's task waits on the answer until something resumes it; only one call is held at a
    /// time, as in Playwright.
    pub(crate) fn before_call(
        &self,
        call: &CallToGate<'_>,
        hub: &EventHub,
    ) -> Option<oneshot::Receiver<()>> {
        if call.internal {
            return None;
        }
        let mut debuggers = self.0.lock().ok()?;
        let state = debuggers.get_mut(call.context)?;
        // Playwright: `server/debugger.ts` `Debugger.onBeforeCall`: `const pauseOnPauseCall = this._enabled && metadata.type === 'BrowserContext' && metadata.method === 'pause';`
        let pause_call =
            state.enabled && call.meta.interface == "BrowserContext" && call.meta.method == "pause";
        // Playwright: `server/debugger.ts` `Debugger.onBeforeCall`: `const pauseBeforeAction = !!this._pauseAt.next && !!metainfo?.pause && (this._pauseBeforeWaitingActions || !metainfo?.isAutoWaiting);`
        let before_action = state.pause_at.next
            && call.meta.has(MethodFlag::Pause)
            && (state.pause_before_waiting_actions || !call.meta.has(MethodFlag::IsAutoWaiting));
        let at_location = state
            .pause_at
            .location
            .as_ref()
            .is_some_and(|location| matches_location(call.location, location));
        // Playwright: `server/debugger.ts` `Debugger._pause`: `if (this._pausedCall)`
        if !(pause_call || before_action || at_location) || state.paused.is_some() {
            return None;
        }
        state.pause_at = PauseAt::default();
        let details = paused_details(call);
        let (resume, resumed) = oneshot::channel();
        state.paused = Some(Paused {
            details: details.clone(),
            resume,
        });
        changed(hub, call.context, Some(&details));
        Some(resumed)
    }

    /// A `Debugger` call on `context`'s debugger.
    pub(crate) fn serve(
        &self,
        context: &str,
        call: &DebuggerCall,
        hub: &EventHub,
    ) -> Result<Option<Value>, CallError> {
        let mut debuggers = self
            .0
            .lock()
            .map_err(|_| PlaywrightError::error("neo-runtime: the debugger is unreadable"))?;
        let state = debuggers.entry(context.to_string()).or_default();
        match call {
            DebuggerCall::RequestPause(_) => {
                if state.paused.is_some() {
                    return Err(PlaywrightError::error(ALREADY_PAUSED).into());
                }
                state.pause_before_waiting_actions = true;
                state.set_pause_at(PauseAt {
                    next: true,
                    location: None,
                });
            }
            DebuggerCall::Resume(_) => {
                if state.paused.is_none() {
                    return Err(PlaywrightError::error(NOT_PAUSED).into());
                }
                state.resume(hub, context);
            }
            DebuggerCall::Next(_) => {
                if state.paused.is_none() {
                    return Err(PlaywrightError::error(NOT_PAUSED).into());
                }
                state.pause_before_waiting_actions = true;
                state.set_pause_at(PauseAt {
                    next: true,
                    location: None,
                });
                state.resume(hub, context);
            }
            DebuggerCall::RunTo(params) => {
                if state.paused.is_none() {
                    return Err(PlaywrightError::error(NOT_PAUSED).into());
                }
                state.pause_before_waiting_actions = true;
                state.set_pause_at(PauseAt {
                    next: false,
                    location: Some(params.location.clone()),
                });
                state.resume(hub, context);
            }
        }
        Ok(None)
    }

    /// `pausedDetails()`: the held call's location and title, or `null`.
    pub(crate) fn paused_details(&self, context: &str) -> Value {
        self.0
            .lock()
            .ok()
            .and_then(|debuggers| {
                debuggers
                    .get(context)
                    .and_then(|state| state.paused.as_ref().map(|paused| paused.details.clone()))
            })
            .unwrap_or(Value::Null)
    }
}

impl State {
    /// Playwright's `Debugger.setPauseAt`: enables the debugger and says where to stop next.
    fn set_pause_at(&mut self, at: PauseAt) {
        self.enabled = true;
        self.pause_at = at;
    }

    /// Playwright's `Debugger.resume`: the held call goes on.
    fn resume(&mut self, hub: &EventHub, context: &str) {
        if let Some(paused) = self.paused.take() {
            let _woken = paused.resume.send(());
            changed(hub, context, None);
        }
    }
}

/// Queues `pausedStateChanged` on `context`'s debugger with Playwright's params.
fn changed(hub: &EventHub, context: &str, details: Option<&Value>) {
    let params = details.map_or_else(|| json!({}), |details| json!({ "pausedDetails": details }));
    let _queued = hub.emit(&guid(context), "pausedStateChanged", &params);
}

/// Playwright's `matchesLocation`.
fn matches_location(at: Option<&MetadataLocation>, location: &DebuggerRunToParamsLocation) -> bool {
    // Playwright: `server/debugger.ts` `matchesLocation`: `return !!metadata.location?.file.includes(location.file) &&`
    at.is_some_and(|at| {
        at.file.contains(&location.file)
            && location.line.is_none_or(|line| at.line == Some(line))
            && location
                .column
                .is_none_or(|column| at.column == Some(column))
    })
}

/// Playwright's `DebuggerDispatcher._serializePausedDetails`.
fn paused_details(call: &CallToGate<'_>) -> Value {
    let mut location = json!({
        "file": call.location.map_or(UNKNOWN_FILE, |location| location.file.as_str()),
    });
    if let Some(line) = call.location.and_then(|location| location.line) {
        location["line"] = json!(line);
    }
    if let Some(column) = call.location.and_then(|location| location.column) {
        location["column"] = json!(column);
    }
    json!({ "location": location, "title": full_title(call) })
}

/// Playwright's `renderFullTitleForCall`: the method's title, then its subtitle when every
/// parameter it names is present.
fn full_title(call: &CallToGate<'_>) -> String {
    // Playwright: `isomorphic/protocolFormatter.ts` `renderTitleForCall`: `const titleFormat = metadata.title ?? getMetainfo(metadata)?.title ?? metadata.method;`
    let format = call.title.or(call.meta.title).unwrap_or(call.meta.method);
    let (title, _) = render(format, call.params);
    match call.meta.subtitle.map(|format| render(format, call.params)) {
        Some((subtitle, true)) => format!("{title} {subtitle}"),
        _ => title,
    }
}

/// `format` with each `{name}` (or `{a|b}`) replaced by its parameter, and whether every one was.
fn render(format: &str, params: &Value) -> (String, bool) {
    let mut out = String::new();
    let mut all = true;
    let mut rest = format;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|close| open + close) else {
            break;
        };
        out.push_str(&rest[..open]);
        let alternatives = &rest[open + 1..close];
        if let Some(value) = format_param(params, alternatives) {
            // Playwright: `isomorphic/protocolFormatter.ts` `formatProtocolParam`: `return _formatProtocolParam(params, alternatives, sdkLanguage)?.replaceAll('\n', '\\n');`
            out.push_str(&value.replace('\n', "\\n"));
        } else {
            all = false;
            out.push_str(&rest[open..=close]);
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    (out, all)
}

/// Playwright's `_formatProtocolParam`, the first of `alternatives` the call has.
///
/// Deviation: `isomorphic/protocolFormatter.ts` `_formatProtocolParam`: `return asLocatorDescription(sdkLanguage ?? 'javascript', value);`
/// Playwright shows a selector as the JavaScript locator that builds it (`locator('div')`); Rust
/// has no locator generator, so a paused call's title shows the selector itself (`div`).
fn format_param(params: &Value, alternatives: &str) -> Option<String> {
    alternatives.split('|').find_map(|name| {
        let value = name
            .split('.')
            .try_fold(params, |current, token| current.get(token))?;
        if name == "url"
            && let Some(text) = value.as_str()
        {
            return Some(format_url(text));
        }
        match value {
            Value::Null => Some("null".to_string()),
            Value::String(text) => Some(text.clone()),
            Value::Bool(flag) => Some(flag.to_string()),
            Value::Number(number) => Some(number.to_string()),
            Value::Array(_) | Value::Object(_) => Some("[object Object]".to_string()),
        }
    })
}

/// Playwright's `url` branch of `_formatProtocolParam`: `data:` URLs by their scheme, `about:`,
/// `chrome:` and `edge:` ones whole, others as host, path and query.
fn format_url(text: &str) -> String {
    let Ok(url) = url::Url::parse(text) else {
        return text.to_string();
    };
    let scheme = url.scheme();
    // Playwright: `isomorphic/protocolFormatter.ts` `_formatProtocolParam`: `if (urlObject.protocol === 'data:')`
    if scheme == "data" {
        return "data:".to_string();
    }
    // Playwright: `isomorphic/protocolFormatter.ts` `_formatProtocolParam`: `if (['about:', 'chrome:', 'edge:'].includes(urlObject.protocol))`
    if ["about", "chrome", "edge"].contains(&scheme) {
        return text.to_string();
    }
    let host = url.host_str().unwrap_or_default();
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    let query = url
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    format!("{host}{port}{}{query}", url.path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_runtime_spec::Command;
    use neo_runtime_spec::serve::{FrameClickParams, FrameGotoParams};

    use crate::events::EventHub;

    fn location(line: i64) -> MetadataLocation {
        MetadataLocation {
            file: "run.js".to_string(),
            line: Some(line),
            column: Some(5),
        }
    }

    fn click<'a>(at: &'a MetadataLocation, params: &'a Value) -> CallToGate<'a> {
        CallToGate {
            context: DEFAULT_CONTEXT,
            meta: FrameClickParams::META,
            internal: false,
            location: Some(at),
            params,
            title: None,
        }
    }

    #[test]
    fn a_requested_pause_holds_the_next_pausable_call_until_resumed() {
        let debuggers = Debuggers::default();
        let hub = EventHub::default();
        let params = json!({ "selector": "div" });
        let at = location(3);
        assert!(debuggers.before_call(&click(&at, &params), &hub).is_none());
        debuggers
            .serve(
                DEFAULT_CONTEXT,
                &DebuggerCall::RequestPause(serde_json::from_value(json!({})).unwrap()),
                &hub,
            )
            .unwrap();
        let mut resumed = debuggers.before_call(&click(&at, &params), &hub).unwrap();
        assert_eq!(
            debuggers.paused_details(DEFAULT_CONTEXT),
            json!({ "location": { "file": "run.js", "line": 3, "column": 5 }, "title": "Click div" })
        );
        assert!(
            debuggers.before_call(&click(&at, &params), &hub).is_none(),
            "only one call is held"
        );
        assert!(resumed.try_recv().is_err());
        debuggers
            .serve(
                DEFAULT_CONTEXT,
                &DebuggerCall::Resume(serde_json::from_value(json!({})).unwrap()),
                &hub,
            )
            .unwrap();
        assert!(resumed.try_recv().is_ok());
        assert_eq!(debuggers.paused_details(DEFAULT_CONTEXT), Value::Null);
        assert!(debuggers.before_call(&click(&at, &params), &hub).is_none());
    }

    #[test]
    fn resuming_an_unpaused_debugger_fails_as_playwrights() {
        let debuggers = Debuggers::default();
        let hub = EventHub::default();
        for call in [
            DebuggerCall::Resume(serde_json::from_value(json!({})).unwrap()),
            DebuggerCall::Next(serde_json::from_value(json!({})).unwrap()),
            DebuggerCall::RunTo(
                serde_json::from_value(json!({ "location": { "file": "run.js" } })).unwrap(),
            ),
        ] {
            assert_eq!(
                debuggers.serve(DEFAULT_CONTEXT, &call, &hub),
                Err(PlaywrightError::error(NOT_PAUSED).into())
            );
        }
    }

    #[test]
    fn run_to_stops_only_at_its_location() {
        let debuggers = Debuggers::default();
        let hub = EventHub::default();
        let params = json!({ "selector": "div" });
        let (first, other, target) = (location(1), location(2), location(9));
        debuggers
            .serve(
                DEFAULT_CONTEXT,
                &DebuggerCall::RequestPause(serde_json::from_value(json!({})).unwrap()),
                &hub,
            )
            .unwrap();
        let _held = debuggers
            .before_call(&click(&first, &params), &hub)
            .unwrap();
        debuggers
            .serve(
                DEFAULT_CONTEXT,
                &DebuggerCall::RunTo(
                    serde_json::from_value(json!({ "location": { "file": "run.js", "line": 9 } }))
                        .unwrap(),
                ),
                &hub,
            )
            .unwrap();
        assert!(
            debuggers
                .before_call(&click(&other, &params), &hub)
                .is_none()
        );
        assert!(
            debuggers
                .before_call(&click(&target, &params), &hub)
                .is_some()
        );
    }

    #[test]
    fn titles_follow_playwrights_formatter() {
        let goto = json!({ "url": "https://example.com:8080/a/b?x=1#frag" });
        let at = location(1);
        let call = CallToGate {
            context: DEFAULT_CONTEXT,
            meta: FrameGotoParams::META,
            internal: false,
            location: Some(&at),
            params: &goto,
            title: None,
        };
        assert_eq!(full_title(&call), "Navigate example.com:8080/a/b?x=1");
        assert_eq!(format_url("data:text/html,hi"), "data:");
        assert_eq!(format_url("about:blank"), "about:blank");
        let missing = json!({});
        let call = CallToGate {
            params: &missing,
            ..call
        };
        assert_eq!(
            full_title(&call),
            "Navigate",
            "an unresolved subtitle is left out"
        );
    }
}
