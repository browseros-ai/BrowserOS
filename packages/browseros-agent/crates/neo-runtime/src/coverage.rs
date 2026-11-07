//! Playwright: `server/chromium/crCoverage.ts` `CRCoverage`
//!
//! `page.coverage` on the page's own CDP session: precise JS coverage (`Profiler`, with the
//! `Debugger` domain for the scripts and their sources, never to evaluate) and CSS rule usage
//! (`CSS`, with `DOM`), each from its start to its stop.
//!
//! ```text
//!  start   subscribe cdp.events() (this session only), then enable the domains; a collector
//!          task records every script / stylesheet the page reports, fetching each source
//!  stop    the domains' final answer (precise coverage / rule usage), then the collector
//!          drains every event that preceded it and ends; entries are built from what it saw
//!  run end the session detaches, which ends the domains; the collector's guard ends it too
//! ```
//!
//! The collector never holds the page session, only its CDP client and session id, so a
//! coverage the script never stops ends with the session's last reference.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use browseros_cdp::{CdpClient, CdpEvent, SessionId};
use neo_runtime_spec::serve::{
    PageStartCSSCoverageParams, PageStartJSCoverageParams, PageStopCSSCoverageResult,
    PageStopCSSCoverageResultEntriesItem, PageStopCSSCoverageResultEntriesItemRangesItem,
    PageStopJSCoverageResult, PageStopJSCoverageResultEntriesItem,
    PageStopJSCoverageResultEntriesItemFunctionsItem,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::broadcast::{self, error::RecvError, error::TryRecvError};
use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::debug;

use crate::page::PageSession;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::tabs;

/// A page's coverage: at most one JS and one CSS collector (`PageSession::coverage`). The locks
/// serialize a page's starts and stops, as Playwright's server runs them one at a time.
#[derive(Default)]
pub(crate) struct Coverage {
    js: AsyncMutex<Option<Collector<JsSeen>>>,
    css: AsyncMutex<Option<Collector<CssSeen>>>,
}

/// What a JS collector saw: Playwright's `JSCoverage` fields.
struct JsSeen {
    reset_on_navigation: bool,
    report_anonymous_scripts: bool,
    script_ids: HashSet<String>,
    script_sources: HashMap<String, String>,
}

/// What a CSS collector saw: Playwright's `CSSCoverage` fields, `ids` keeping the order in which
/// Playwright's `Map` would list them.
struct CssSeen {
    reset_on_navigation: bool,
    ids: Vec<String>,
    urls: HashMap<String, String>,
    texts: HashMap<String, String>,
}

/// One running collector: its task, and the guard that ends it when the page session goes.
struct Collector<S> {
    seen: Arc<Mutex<S>>,
    stop: CancellationToken,
    task: JoinHandle<()>,
    _guard: DropGuard,
}

/// The session a collector reads and sends on, and the page's main frame (its target id).
#[derive(Clone)]
struct Feed {
    cdp: CdpClient,
    session: SessionId,
    main_frame: String,
}

impl Feed {
    fn of(page: &PageSession) -> Self {
        Self {
            cdp: page.cdp().clone(),
            session: page.session_id().clone(),
            main_frame: page.main_frame().to_string(),
        }
    }

    async fn send(&self, method: &str, params: Value) -> Result<Value, CallError> {
        self.cdp
            .send(method, params, Some(&self.session))
            .await
            .map_err(|error| tabs::cdp_error(&error).into())
    }

    /// Playwright: `server/chromium/crConnection.ts` `CRSession._sendMayFail`: `return this.send(method, params).catch((error: ProtocolError) => debugLogger.log('error', error));`
    fn send_may_fail<F>(&self, method: &'static str, params: Value, then: F)
    where
        F: FnOnce(Value) + Send + 'static,
    {
        let feed = self.clone();
        tokio::spawn(async move {
            match feed.cdp.send(method, params, Some(&feed.session)).await {
                Ok(answer) => then(answer),
                Err(error) => debug!(%error, method, "a coverage read failed"),
            }
        });
    }

    /// Neo: whether `event` is this session's main-frame commit of a new document, the moment
    /// Playwright's `Runtime.executionContextsCleared` reports. Neo enables no `Runtime` on the
    /// run's sessions; `Page` is always on, and its `frameNavigated` reaches the collector before
    /// any script or stylesheet of the new document.
    fn navigated(&self, event: &CdpEvent) -> bool {
        event.method == "Page.frameNavigated"
            && event.params["frame"]["id"].as_str() == Some(self.main_frame.as_str())
    }
}

impl<S: Send + 'static> Collector<S> {
    /// Subscribes before anything is sent, so no event the start causes is missed, and runs
    /// `apply` on every event of the session until stopped.
    fn start(feed: &Feed, seen: S, apply: fn(&Feed, &Arc<Mutex<S>>, &CdpEvent)) -> Self {
        let events = feed.cdp.events();
        let seen = Arc::new(Mutex::new(seen));
        let stop = CancellationToken::new();
        let task = tokio::spawn(collect(
            events,
            feed.clone(),
            Arc::clone(&seen),
            stop.clone(),
            apply,
        ));
        Self {
            seen,
            _guard: stop.clone().drop_guard(),
            stop,
            task,
        }
    }

    /// Ends the collector once it applied every event already received: the events that
    /// preceded the domains' final answer, as the CDP reader publishes events and replies in
    /// socket order.
    async fn finish(self) -> Arc<Mutex<S>> {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            debug!(%error, "a coverage collector ended early");
        }
        self.seen
    }
}

async fn collect<S>(
    mut events: broadcast::Receiver<CdpEvent>,
    feed: Feed,
    seen: Arc<Mutex<S>>,
    stop: CancellationToken,
    apply: fn(&Feed, &Arc<Mutex<S>>, &CdpEvent),
) {
    let mine = |event: &CdpEvent| event.session_id.as_ref() == Some(&feed.session);
    loop {
        let event = tokio::select! {
            biased;
            event = events.recv() => event,
            () = stop.cancelled() => {
                loop {
                    match events.try_recv() {
                        Ok(event) if mine(&event) => apply(&feed, &seen, &event),
                        Ok(_) => {}
                        Err(TryRecvError::Lagged(skipped)) => {
                            debug!(skipped, "a coverage collector missed events");
                        }
                        Err(TryRecvError::Empty | TryRecvError::Closed) => return,
                    }
                }
            }
        };
        match event {
            Ok(event) if mine(&event) => apply(&feed, &seen, &event),
            Ok(_) => {}
            Err(RecvError::Lagged(skipped)) => {
                debug!(skipped, "a coverage collector missed events")
            }
            Err(RecvError::Closed) => return,
        }
    }
}

/// Playwright: `server/chromium/crCoverage.ts` `JSCoverage.start`
///
/// # Errors
///
/// `JSCoverage is already enabled` while one runs, a domain's CDP error, and the call's stop.
pub(crate) async fn start_js(
    page: &PageSession,
    params: PageStartJSCoverageParams,
    progress: &Progress,
) -> Result<(), CallError> {
    let mut js = page.coverage.js.lock().await;
    if js.is_some() {
        // Playwright: `server/chromium/crCoverage.ts` `JSCoverage.start`: `assert(!this._enabled, 'JSCoverage is already enabled');`
        return Err(PlaywrightError::error("JSCoverage is already enabled").into());
    }
    let feed = Feed::of(page);
    let seen = JsSeen {
        // Playwright: `server/chromium/crCoverage.ts` `JSCoverage.start`: `resetOnNavigation = true,`
        reset_on_navigation: params.reset_on_navigation.unwrap_or(true),
        // Playwright: `server/chromium/crCoverage.ts` `JSCoverage.start`: `reportAnonymousScripts = false`
        report_anonymous_scripts: params.report_anonymous_scripts.unwrap_or(false),
        script_ids: HashSet::new(),
        script_sources: HashMap::new(),
    };
    let collector = Collector::start(&feed, seen, apply_js);
    let enabled = progress
        .race(async {
            tokio::try_join!(
                feed.send("Profiler.enable", json!({})),
                feed.send(
                    "Profiler.startPreciseCoverage",
                    json!({ "callCount": true, "detailed": true })
                ),
                feed.send("Debugger.enable", json!({})),
                feed.send("Debugger.setSkipAllPauses", json!({ "skip": true })),
            )
        })
        .await;
    match enabled {
        Ok(Ok(_)) => {
            *js = Some(collector);
            Ok(())
        }
        // Playwright: `server/chromium/crCoverage.ts` `CRCoverage.startJSCoverage`: `await raceUncancellableOperationWithCleanup(progress, () => this._jsCoverage.start(options), () => this._jsCoverage.stop());`
        Ok(Err(error)) | Err(error) => {
            stop_js_domains(&feed).await;
            collector.finish().await;
            Err(error)
        }
    }
}

fn apply_js(feed: &Feed, seen: &Arc<Mutex<JsSeen>>, event: &CdpEvent) {
    let Ok(mut state) = seen.lock() else {
        return;
    };
    if feed.navigated(event) {
        // Playwright: `server/chromium/crCoverage.ts` `JSCoverage._onExecutionContextsCleared`: `if (!this._resetOnNavigation)`
        if state.reset_on_navigation {
            state.script_ids.clear();
            state.script_sources.clear();
        }
        return;
    }
    match event.method.as_str() {
        "Debugger.scriptParsed" => {
            let Ok(parsed) = ScriptParsed::deserialize(&event.params) else {
                return;
            };
            state.script_ids.insert(parsed.script_id.clone());
            // Playwright: `server/chromium/crCoverage.ts` `JSCoverage._onScriptParsed`: `if (!event.url && !this._reportAnonymousScripts)`
            if parsed.url.is_empty() && !state.report_anonymous_scripts {
                return;
            }
            drop(state);
            let seen = Arc::clone(seen);
            let id = parsed.script_id.clone();
            feed.send_may_fail(
                "Debugger.getScriptSource",
                json!({ "scriptId": parsed.script_id }),
                move |answer| {
                    if let (Some(source), Ok(mut state)) =
                        (answer["scriptSource"].as_str(), seen.lock())
                    {
                        state.script_sources.insert(id, source.to_string());
                    }
                },
            );
        }
        // Playwright: `server/chromium/crCoverage.ts` `JSCoverage._onDebuggerPaused`: `this._client.send('Debugger.resume');`
        "Debugger.paused" => {
            drop(state);
            feed.send_may_fail("Debugger.resume", json!({}), |_| {});
        }
        _ => {}
    }
}

/// The domains a JS coverage turned on, turned off again, best effort.
async fn stop_js_domains(feed: &Feed) {
    let _stopped = tokio::join!(
        feed.send("Profiler.stopPreciseCoverage", json!({})),
        feed.send("Profiler.disable", json!({})),
        feed.send("Debugger.disable", json!({})),
    );
}

/// Playwright: `server/chromium/crCoverage.ts` `JSCoverage.stop`
///
/// # Errors
///
/// A domain's CDP error, and the call's stop.
pub(crate) async fn stop_js(
    page: &PageSession,
    progress: &Progress,
) -> Result<PageStopJSCoverageResult, CallError> {
    let mut js = page.coverage.js.lock().await;
    // Playwright: `server/chromium/crCoverage.ts` `JSCoverage.stop`: `return { entries: [] };`
    let Some(collector) = js.take() else {
        return Ok(PageStopJSCoverageResult {
            entries: Vec::new(),
        });
    };
    let feed = Feed::of(page);
    let answered = progress
        .race(async {
            tokio::try_join!(
                feed.send("Profiler.takePreciseCoverage", json!({})),
                feed.send("Profiler.stopPreciseCoverage", json!({})),
                feed.send("Profiler.disable", json!({})),
                feed.send("Debugger.disable", json!({})),
            )
        })
        .await;
    let seen = collector.finish().await;
    let (taken, _, _, _) = answered??;
    let taken = TakenCoverage::deserialize(&taken).map_err(internal)?;
    let seen = seen
        .lock()
        .map_err(|_| internal("a coverage collector failed"))?;
    let entries = taken
        .result
        .into_iter()
        // Playwright: `server/chromium/crCoverage.ts` `JSCoverage.stop`: `if (!this._scriptIds.has(entry.scriptId))`
        .filter(|entry| seen.script_ids.contains(&entry.script_id))
        // Playwright: `server/chromium/crCoverage.ts` `JSCoverage.stop`: `if (!entry.url && !this._reportAnonymousScripts)`
        .filter(|entry| !entry.url.is_empty() || seen.report_anonymous_scripts)
        .map(|entry| PageStopJSCoverageResultEntriesItem {
            source: seen.script_sources.get(&entry.script_id).cloned(),
            url: entry.url,
            script_id: entry.script_id,
            functions: entry.functions,
        })
        .collect();
    Ok(PageStopJSCoverageResult { entries })
}

/// Playwright: `server/chromium/crCoverage.ts` `CSSCoverage.start`
///
/// # Errors
///
/// `CSSCoverage is already enabled` while one runs, a domain's CDP error, and the call's stop.
pub(crate) async fn start_css(
    page: &PageSession,
    params: PageStartCSSCoverageParams,
    progress: &Progress,
) -> Result<(), CallError> {
    let mut css = page.coverage.css.lock().await;
    if css.is_some() {
        // Playwright: `server/chromium/crCoverage.ts` `CSSCoverage.start`: `assert(!this._enabled, 'CSSCoverage is already enabled');`
        return Err(PlaywrightError::error("CSSCoverage is already enabled").into());
    }
    let feed = Feed::of(page);
    let seen = CssSeen {
        // Playwright: `server/chromium/crCoverage.ts` `CSSCoverage.start`: `const { resetOnNavigation = true } = options;`
        reset_on_navigation: params.reset_on_navigation.unwrap_or(true),
        ids: Vec::new(),
        urls: HashMap::new(),
        texts: HashMap::new(),
    };
    let collector = Collector::start(&feed, seen, apply_css);
    let enabled = progress
        .race(async {
            tokio::try_join!(
                feed.send("DOM.enable", json!({})),
                feed.send("CSS.enable", json!({})),
                feed.send("CSS.startRuleUsageTracking", json!({})),
            )
        })
        .await;
    match enabled {
        Ok(Ok(_)) => {
            *css = Some(collector);
            Ok(())
        }
        // Playwright: `server/chromium/crCoverage.ts` `CRCoverage.startCSSCoverage`: `await raceUncancellableOperationWithCleanup(progress, () => this._cssCoverage.start(options), () => this._cssCoverage.stop());`
        Ok(Err(error)) | Err(error) => {
            stop_css_domains(&feed).await;
            collector.finish().await;
            Err(error)
        }
    }
}

fn apply_css(feed: &Feed, seen: &Arc<Mutex<CssSeen>>, event: &CdpEvent) {
    let Ok(mut state) = seen.lock() else {
        return;
    };
    if feed.navigated(event) {
        // Playwright: `server/chromium/crCoverage.ts` `CSSCoverage._onExecutionContextsCleared`: `if (!this._resetOnNavigation)`
        if state.reset_on_navigation {
            state.ids.clear();
            state.urls.clear();
            state.texts.clear();
        }
        return;
    }
    if event.method != "CSS.styleSheetAdded" {
        return;
    }
    drop(state);
    let Ok(StyleSheetAdded { header }) = StyleSheetAdded::deserialize(&event.params) else {
        return;
    };
    // Playwright: `server/chromium/crCoverage.ts` `CSSCoverage._onStyleSheet`: `if (!header.sourceURL)`
    if header.source_url.is_empty() {
        return;
    }
    let seen = Arc::clone(seen);
    feed.send_may_fail(
        "CSS.getStyleSheetText",
        json!({ "styleSheetId": header.style_sheet_id }),
        move |answer| {
            let (Some(text), Ok(mut state)) = (answer["text"].as_str(), seen.lock()) else {
                return;
            };
            if state
                .urls
                .insert(header.style_sheet_id.clone(), header.source_url)
                .is_none()
            {
                state.ids.push(header.style_sheet_id.clone());
            }
            state.texts.insert(header.style_sheet_id, text.to_string());
        },
    );
}

/// The domains a CSS coverage turned on, turned off again, best effort.
async fn stop_css_domains(feed: &Feed) {
    let _stopped = tokio::join!(
        feed.send("CSS.stopRuleUsageTracking", json!({})),
        feed.send("CSS.disable", json!({})),
        feed.send("DOM.disable", json!({})),
    );
}

/// Playwright: `server/chromium/crCoverage.ts` `CSSCoverage.stop`
///
/// # Errors
///
/// A domain's CDP error, and the call's stop.
pub(crate) async fn stop_css(
    page: &PageSession,
    progress: &Progress,
) -> Result<PageStopCSSCoverageResult, CallError> {
    let mut css = page.coverage.css.lock().await;
    // Playwright: `server/chromium/crCoverage.ts` `CSSCoverage.stop`: `return { entries: [] };`
    let Some(collector) = css.take() else {
        return Ok(PageStopCSSCoverageResult {
            entries: Vec::new(),
        });
    };
    let feed = Feed::of(page);
    let tracked = progress
        .race(feed.send("CSS.stopRuleUsageTracking", json!({})))
        .await;
    let disabled = progress
        .race(async {
            tokio::try_join!(
                feed.send("CSS.disable", json!({})),
                feed.send("DOM.disable", json!({})),
            )
        })
        .await;
    let seen = collector.finish().await;
    let tracked = RuleUsage::deserialize(&tracked??).map_err(internal)?;
    disabled??;
    let seen = seen
        .lock()
        .map_err(|_| internal("a coverage collector failed"))?;
    // Playwright: `server/chromium/crCoverage.ts` `CSSCoverage.stop`: `// aggregate by styleSheetId`
    let mut by_sheet: HashMap<&str, Vec<UsedRange>> = HashMap::new();
    for rule in &tracked.rule_usage {
        by_sheet
            .entry(rule.style_sheet_id.as_str())
            .or_default()
            .push(UsedRange {
                start_offset: offset(rule.start_offset),
                end_offset: offset(rule.end_offset),
                // Playwright: `server/chromium/crCoverage.ts` `CSSCoverage.stop`: `count: entry.used ? 1 : 0,`
                count: i64::from(rule.used),
            });
    }
    let entries = seen
        .ids
        .iter()
        .filter_map(|id| {
            let url = seen.urls.get(id)?;
            let ranges = by_sheet.get(id.as_str()).map_or(&[][..], Vec::as_slice);
            Some(PageStopCSSCoverageResultEntriesItem {
                url: url.clone(),
                text: seen.texts.get(id).cloned(),
                ranges: disjoint_ranges(ranges),
            })
        })
        .collect();
    Ok(PageStopCSSCoverageResult { entries })
}

/// Neo: an answer of Chromium's Neo cannot read, or a collector a panic left behind.
fn internal(what: impl std::fmt::Display) -> PlaywrightError {
    PlaywrightError::error(format!("neo-runtime: {what}"))
}

/// One rule's range and whether it was used, as `convertToDisjointRanges` takes it.
#[derive(Clone, Copy, Debug)]
struct UsedRange {
    start_offset: i64,
    end_offset: i64,
    count: i64,
}

/// Playwright: `server/chromium/crCoverage.ts` `convertToDisjointRanges`
fn disjoint_ranges(nested: &[UsedRange]) -> Vec<PageStopCSSCoverageResultEntriesItemRangesItem> {
    // (offset, is_end, range)
    let mut points: Vec<(i64, bool, UsedRange)> = nested
        .iter()
        .flat_map(|range| {
            [
                (range.start_offset, false, *range),
                (range.end_offset, true, *range),
            ]
        })
        .collect();
    // Playwright: `server/chromium/crCoverage.ts` `convertToDisjointRanges`: `// Sort points to form a valid parenthesis sequence.`
    points.sort_by(|a, b| {
        let length = |range: &UsedRange| range.end_offset - range.start_offset;
        a.0.cmp(&b.0)
            // All "end" points go before "start" points.
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| {
                if a.1 {
                    // For two "end" points, the shorter range goes first.
                    length(&a.2).cmp(&length(&b.2))
                } else {
                    // For two "start" points, the longer range goes first.
                    length(&b.2).cmp(&length(&a.2))
                }
            })
    });
    let mut counts: Vec<i64> = Vec::new();
    let mut results: Vec<(i64, i64)> = Vec::new();
    let mut last = 0;
    for (offset, is_end, range) in points {
        if counts.last().is_some_and(|&count| count > 0) && last < offset {
            match results.last_mut() {
                Some(previous) if previous.1 == last => previous.1 = offset,
                _ => results.push((last, offset)),
            }
        }
        last = offset;
        if is_end {
            counts.pop();
        } else {
            counts.push(range.count);
        }
    }
    results
        .into_iter()
        // Playwright: `server/chromium/crCoverage.ts` `convertToDisjointRanges`: `return results.filter(range => range.end - range.start > 1);`
        .filter(|(start, end)| end - start > 1)
        .map(|(start, end)| PageStopCSSCoverageResultEntriesItemRangesItem { start, end })
        .collect()
}

/// Neo: a CSS offset on the wire. CDP reports them as numbers; they are whole.
#[expect(
    clippy::cast_possible_truncation,
    reason = "CDP's CSS rule offsets are whole numbers of characters"
)]
fn offset(value: f64) -> i64 {
    value as i64
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScriptParsed {
    script_id: String,
    url: String,
}

#[derive(Deserialize)]
struct TakenCoverage {
    result: Vec<ScriptCoverage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScriptCoverage {
    script_id: String,
    url: String,
    functions: Vec<PageStopJSCoverageResultEntriesItemFunctionsItem>,
}

#[derive(Deserialize)]
struct StyleSheetAdded {
    header: StyleSheetHeader,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StyleSheetHeader {
    style_sheet_id: String,
    #[serde(rename = "sourceURL")]
    source_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuleUsage {
    rule_usage: Vec<RuleUse>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuleUse {
    style_sheet_id: String,
    start_offset: f64,
    end_offset: f64,
    used: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(nested: &[(i64, i64, i64)]) -> Vec<(i64, i64)> {
        let nested: Vec<UsedRange> = nested
            .iter()
            .map(|&(start_offset, end_offset, count)| UsedRange {
                start_offset,
                end_offset,
                count,
            })
            .collect();
        disjoint_ranges(&nested)
            .into_iter()
            .map(|range| (range.start, range.end))
            .collect()
    }

    #[test]
    fn nested_rule_usage_becomes_disjoint_used_ranges() {
        // Two used rules side by side merge into one range.
        assert_eq!(ranges(&[(0, 16, 1), (16, 39, 1)]), [(0, 39)]);
        // An unused rule inside a used one splits it.
        assert_eq!(ranges(&[(0, 30, 1), (10, 20, 0)]), [(0, 10), (20, 30)]);
        // Unused rules give nothing, and a range of one character is dropped.
        assert_eq!(ranges(&[(0, 10, 0), (12, 13, 1)]), []);
        assert_eq!(ranges(&[]), []);
    }
}
