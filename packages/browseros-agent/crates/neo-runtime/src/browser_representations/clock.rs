//! Neo: the run's fake clock (`page.clock` and `browserContext.clock`, the same clock):
//! Playwright's server `Clock`.
//!
//! ```text
//!  first call  the clock's init script (clock-install.js, `__pwClock`), and the same in every
//!              frame of the pages the run has a session on (main world)
//!  each call   an init script that logs it (clock-call.js `log`), which a new document's clock
//!              replays; then the call in every frame (clock-call.js `<op>`), a throw being the
//!              call's error, as Playwright's `throwOnJSErrors`
//!  run end     after init_scripts.rs removed every script: clock-call.js `end` in every frame,
//!              which gives each document its own timers back
//! ```
//!
//! A time string is read as Playwright's server reads it, with `new Date(…)`: by the page's own
//! `Date` (clock-call.js `parse`), as Rust has no JavaScript date parser.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use neo_runtime_spec::MethodMeta;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tracing::debug;

use crate::actions::{ActionKind, NeoMethod};
use crate::browser_representations::init_scripts::{self, InitScripts};
use crate::browser_representations::page::{FrameTab, PageSession};
use crate::browser_representations::tabs::Tabs;
use crate::chromium_bridge::frames::FrameScope;
use crate::chromium_bridge::main_world::BuiltExpression;
use crate::debugger::DEFAULT_CONTEXT;
use crate::playwright_methods::click::PageError;
use crate::playwright_methods::evaluate;
use crate::progress::PlaywrightError;

/// Neo: the browser name Playwright's server hands the clock (`browser.options.name`): Neo's
/// browser is Chromium's, under the name Playwright gives that.
const BROWSER_NAME: &str = "chromium";

macro_rules! clock_meta {
    ($name:ident, $method:literal, $title:literal, $subtitle:expr, $params:expr) => {
        /// Neo: the trace metadata of this `BrowserContext` clock command
        /// (`neo-runtime-spec/spec/playwright/browserContext.yml`).
        pub(crate) static $name: NeoMethod = NeoMethod::new(
            MethodMeta {
                interface: "BrowserContext",
                method: $method,
                title: Some($title),
                subtitle: $subtitle,
                group: None,
                render_params: $params,
                internal: false,
                flags: &[],
            },
            ActionKind::Clock,
            false,
        );
    };
}

clock_meta!(
    CLOCK_INSTALL_META,
    "clockInstall",
    "Install clock",
    Some("{timeNumber|timeString}"),
    &["timeNumber", "timeString"]
);
clock_meta!(
    CLOCK_FAST_FORWARD_META,
    "clockFastForward",
    "Fast forward clock",
    Some("{ticksNumber|ticksString}"),
    &["ticksNumber", "ticksString"]
);
clock_meta!(
    CLOCK_PAUSE_AT_META,
    "clockPauseAt",
    "Pause clock",
    Some("{timeNumber|timeString}"),
    &["timeNumber", "timeString"]
);
clock_meta!(CLOCK_RESUME_META, "clockResume", "Resume clock", None, &[]);
clock_meta!(
    CLOCK_RUN_FOR_META,
    "clockRunFor",
    "Run clock",
    Some("{ticksNumber|ticksString}"),
    &["ticksNumber", "ticksString"]
);
clock_meta!(
    CLOCK_SET_FIXED_TIME_META,
    "clockSetFixedTime",
    "Set fixed time",
    Some("{timeNumber|timeString}"),
    &["timeNumber", "timeString"]
);
clock_meta!(
    CLOCK_SET_SYSTEM_TIME_META,
    "clockSetSystemTime",
    "Set system time",
    Some("{timeNumber|timeString}"),
    &["timeNumber", "timeString"]
);

/// A time or a tick count as the client sends it: Playwright's `timeString ?? timeNumber`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Given {
    Number(f64),
    String(String),
}

/// A clock command, routed by `method_router.rs`.
#[derive(Debug, PartialEq)]
pub(crate) enum ClockRoute {
    Install(Option<Given>),
    FastForward(Given),
    PauseAt(Given),
    Resume,
    RunFor(Given),
    SetFixedTime(Given),
    SetSystemTime(Given),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TimeParams {
    #[serde(default, deserialize_with = "number")]
    time_number: Option<f64>,
    #[serde(default, deserialize_with = "string")]
    time_string: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TicksParams {
    #[serde(default, deserialize_with = "number")]
    ticks_number: Option<f64>,
    #[serde(default, deserialize_with = "string")]
    ticks_string: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoParams {}

/// Neo: a `tOptional` field Playwright's validator refuses as `null`, where a plain `Option`
/// would read it as absent.
fn number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
    f64::deserialize(deserializer).map(Some)
}

fn string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

/// Playwright: `server/dispatchers/browserContextDispatcher.ts` `BrowserContextDispatcher.clockInstall`: `params.timeString ?? params.timeNumber ?? undefined`
fn time(params: TimeParams) -> Option<Given> {
    params
        .time_string
        .map(Given::String)
        .or_else(|| params.time_number.map(Given::Number))
}

/// Playwright: `server/dispatchers/browserContextDispatcher.ts` `BrowserContextDispatcher.clockRunFor`: `params.ticksString ?? params.ticksNumber ?? 0`
fn ticks(params: TicksParams) -> Given {
    params
        .ticks_string
        .map(Given::String)
        .or_else(|| params.ticks_number.map(Given::Number))
        .unwrap_or(Given::Number(0.0))
}

/// Whether `method` is a clock command of the context.
pub(crate) fn serves(method: &str) -> bool {
    matches!(
        method,
        "clockInstall"
            | "clockFastForward"
            | "clockPauseAt"
            | "clockResume"
            | "clockRunFor"
            | "clockSetFixedTime"
            | "clockSetSystemTime"
    )
}

/// A clock command on the context, or `None` for another method.
pub(crate) fn decode(
    method: &str,
    params: Option<Value>,
) -> Option<(&'static NeoMethod, Result<ClockRoute, PlaywrightError>)> {
    fn with<T: DeserializeOwned>(
        meta: &'static NeoMethod,
        params: Option<Value>,
        route: impl FnOnce(T) -> ClockRoute,
    ) -> (&'static NeoMethod, Result<ClockRoute, PlaywrightError>) {
        let decoded = serde_json::from_value::<T>(params.unwrap_or_else(|| json!({})))
            .map(route)
            .map_err(|error| {
                PlaywrightError::error(format!("{}.{}: {error}", meta.interface, meta.method))
            });
        (meta, decoded)
    }
    let zero = |time: Option<Given>| time.unwrap_or(Given::Number(0.0));
    Some(match method {
        "clockInstall" => with(&CLOCK_INSTALL_META, params, |p: TimeParams| {
            ClockRoute::Install(time(p))
        }),
        "clockFastForward" => with(&CLOCK_FAST_FORWARD_META, params, |p: TicksParams| {
            ClockRoute::FastForward(ticks(p))
        }),
        "clockPauseAt" => with(&CLOCK_PAUSE_AT_META, params, |p: TimeParams| {
            ClockRoute::PauseAt(zero(time(p)))
        }),
        "clockResume" => with(&CLOCK_RESUME_META, params, |_: NoParams| ClockRoute::Resume),
        "clockRunFor" => with(&CLOCK_RUN_FOR_META, params, |p: TicksParams| {
            ClockRoute::RunFor(ticks(p))
        }),
        "clockSetFixedTime" => with(&CLOCK_SET_FIXED_TIME_META, params, |p: TimeParams| {
            ClockRoute::SetFixedTime(zero(time(p)))
        }),
        "clockSetSystemTime" => with(&CLOCK_SET_SYSTEM_TIME_META, params, |p: TimeParams| {
            ClockRoute::SetSystemTime(zero(time(p)))
        }),
        _ => return None,
    })
}

/// The run's clock: whether it is on the run's pages, and whether the run has ended.
#[derive(Default)]
pub(crate) struct Clock {
    /// Serializes the clock's calls, as Playwright's server runs them; run end never waits on it.
    work: Mutex<()>,
    /// Set before the first call puts the clock on a page, so run end takes it back.
    installed: AtomicBool,
    /// Set as run end starts: a call still running stops at its next step.
    closed: AtomicBool,
}

impl Clock {
    /// One clock command, as Playwright's server `Clock` runs it.
    ///
    /// # Errors
    ///
    /// Playwright's errors for a time or tick count it cannot read, the page's error for a
    /// clock call that throws, and `TargetClosedError` after the run ended.
    pub(crate) async fn call(
        &self,
        tabs: &Tabs,
        scripts: &Arc<InitScripts>,
        route: ClockRoute,
    ) -> Result<(), PlaywrightError> {
        let _work = self.work.lock().await;
        let called = self.run_call(tabs, scripts, route).await;
        // Run end came while this call ran: whatever the call put on a page after that end ran,
        // this end takes back.
        if self.closed.load(Ordering::Acquire) {
            self.end(tabs).await;
        }
        called
    }

    async fn run_call(
        &self,
        tabs: &Tabs,
        scripts: &Arc<InitScripts>,
        route: ClockRoute,
    ) -> Result<(), PlaywrightError> {
        self.open()?;
        let cdp = tabs.cdp();
        let pages: Vec<Arc<PageSession>> = tabs
            .own_attached()
            .into_iter()
            .map(|(_, page)| page)
            .collect();
        if !self.installed.swap(true, Ordering::AcqRel) {
            // Playwright: `server/clock.ts` `Clock._installIfNeeded`: `const initScript = await this._browserContext.addInitScript(nullProgress, script);`
            scripts
                .add(cdp, None, DEFAULT_CONTEXT, &install_source())
                .await?;
            for page in &pages {
                scripts.ensure(cdp, page, DEFAULT_CONTEXT).await;
            }
            self.open()?;
            in_all_frames(
                &pages,
                BuiltExpression::ClockInstall,
                &[json!(BROWSER_NAME)],
                Some(&self.closed),
            )
            .await?;
        }
        self.open()?;
        let (op, param) = match route {
            // Playwright: `server/clock.ts` `Clock.install`: `const timeMillis = time !== undefined ? parseTime(time) : Date.now();`
            ClockRoute::Install(time) => match time {
                Some(time) => ("install", Some(parse_time(&pages, time).await?)),
                None => ("install", Some(now())),
            },
            ClockRoute::FastForward(ticks) => ("fastForward", Some(parse_ticks(&ticks)?)),
            ClockRoute::PauseAt(time) => ("pauseAt", Some(parse_time(&pages, time).await?)),
            ClockRoute::Resume => ("resume", None),
            ClockRoute::RunFor(ticks) => ("runFor", Some(parse_ticks(&ticks)?)),
            ClockRoute::SetFixedTime(time) => {
                ("setFixedTime", Some(parse_time(&pages, time).await?))
            }
            ClockRoute::SetSystemTime(time) => {
                ("setSystemTime", Some(parse_time(&pages, time).await?))
            }
        };
        // Playwright: `server/clock.ts` `Clock.install`: `__pwClock.controller.log('install', ${Date.now()}, ${timeMillis})`
        scripts
            .add(cdp, None, DEFAULT_CONTEXT, &log_source(op, now(), param))
            .await?;
        self.open()?;
        let mut args = vec![json!(op)];
        args.extend(param.map(|param| json!(param)));
        in_all_frames(
            &pages,
            BuiltExpression::ClockCall,
            &args,
            Some(&self.closed),
        )
        .await
    }

    /// `TargetClosedError` once run end has started.
    fn open(&self) -> Result<(), PlaywrightError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(PlaywrightError::target_closed());
        }
        Ok(())
    }

    /// Ends the run's clock, after `init_scripts.rs` removed every script: every frame of the
    /// run's pages gets its own timers back (clock-call.js `end`).
    /// It never waits on a call still running, which stops at its next step and ends the clock
    /// again on its way out.
    pub(crate) async fn close(&self, tabs: &Tabs) {
        self.closed.store(true, Ordering::Release);
        if self.installed.load(Ordering::Acquire) {
            self.end(tabs).await;
        }
    }

    async fn end(&self, tabs: &Tabs) {
        let pages: Vec<Arc<PageSession>> = tabs
            .own_attached()
            .into_iter()
            .map(|(_, page)| page)
            .collect();
        if let Err(error) =
            in_all_frames(&pages, BuiltExpression::ClockCall, &[json!("end")], None).await
        {
            debug!(?error, "a frame kept the run's clock");
        }
    }
}

/// The clock's init script.
fn install_source() -> String {
    format!(
        "({})({})",
        BuiltExpression::ClockInstall.source(),
        json!(BROWSER_NAME)
    )
}

/// The init script that logs one call, which a new document's clock replays.
fn log_source(op: &str, now: f64, param: Option<f64>) -> String {
    let mut args = vec![json!("log"), json!(op), json!(now)];
    args.extend(param.map(|param| json!(param)));
    let args: Vec<String> = args.iter().map(Value::to_string).collect();
    format!(
        "({})({})",
        BuiltExpression::ClockCall.source(),
        args.join(", ")
    )
}

/// Playwright's server `Date.now()`, in milliseconds.
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64() * 1000.0)
        .floor()
}

/// Playwright: `server/clock.ts` `parseTime`: `const parsed = new Date(epoch);`
/// Read in the run's first page with the page's own `Date` (clock-call.js `parse`).
async fn parse_time(pages: &[Arc<PageSession>], time: Given) -> Result<f64, PlaywrightError> {
    let epoch = match time {
        Given::Number(number) => json!(number),
        Given::String(text) => json!(text),
    };
    let page = pages.first().ok_or_else(PlaywrightError::target_closed)?;
    let tab = FrameTab {
        page: Arc::clone(page),
        scope: FrameScope::Main,
    };
    match evaluate::run_built(
        &tab,
        BuiltExpression::ClockCall,
        vec![json!("parse"), epoch],
    )
    .await
    {
        Ok(Value::String(message)) => Err(PlaywrightError::error(message)),
        Ok(answer) => millis(&answer),
        Err(error) => Err(page_error(error)),
    }
}

/// Neo: the time clock-call.js `parse` answered, which is a number.
fn millis(answer: &Value) -> Result<f64, PlaywrightError> {
    answer
        .as_f64()
        .ok_or_else(|| PlaywrightError::error("neo-runtime: the page read no time"))
}

/// Playwright: `server/clock.ts` `parseTicks`: `ms += parsed * Math.pow(60, l - i - 1);`
fn parse_ticks(ticks: &Given) -> Result<f64, PlaywrightError> {
    let text = match ticks {
        Given::Number(number) => {
            if !number.is_finite() {
                // Playwright: `server/clock.ts` `parseTicks`: `Invalid ticks: ${value}`
                return Err(PlaywrightError::error(format!("Invalid ticks: {number}")));
            }
            return Ok(*number);
        }
        Given::String(text) => text,
    };
    if text.is_empty() {
        return Ok(0.0);
    }
    let parts: Vec<&str> = text.split(':').collect();
    let well_formed = parts.len() <= 3
        && parts.iter().enumerate().all(|(index, part)| {
            let last = index + 1 == parts.len();
            part.chars().all(|c| c.is_ascii_digit())
                && if last {
                    (1..=2).contains(&part.len())
                } else {
                    part.len() == 2
                }
        });
    if !well_formed {
        // Playwright: `server/clock.ts` `parseTicks`: `Clock only understands numbers, 'mm:ss' and 'hh:mm:ss'`
        return Err(PlaywrightError::error(
            "Clock only understands numbers, 'mm:ss' and 'hh:mm:ss'",
        ));
    }
    let mut seconds = 0.0;
    for part in &parts {
        let parsed: f64 = part.parse().unwrap_or(0.0);
        if parsed >= 60.0 {
            // Playwright: `server/clock.ts` `parseTicks`: `Invalid time ${str}`
            return Err(PlaywrightError::error(format!("Invalid time {text}")));
        }
        seconds = seconds * 60.0 + parsed;
    }
    Ok(seconds * 1000.0)
}

/// Runs a clock call in every frame of `pages`' current documents, as Playwright's
/// `safeNonStallingEvaluateInAllFrames` with `throwOnJSErrors`: a frame that is gone is skipped,
/// a throw is the call's error.
async fn in_all_frames(
    pages: &[Arc<PageSession>],
    expression: BuiltExpression,
    args: &[Value],
    stop: Option<&AtomicBool>,
) -> Result<(), PlaywrightError> {
    let mut thrown = None;
    for page in pages {
        let target = init_scripts::target_of(page);
        let Ok(tree) = page.frames.tree().await else {
            continue;
        };
        for frame in tree {
            // A call cut short by run end puts nothing on another frame.
            if stop.is_some_and(|stop| stop.load(Ordering::Acquire)) {
                return Err(PlaywrightError::target_closed());
            }
            let tab = FrameTab {
                page: Arc::clone(page),
                scope: if frame.id == target {
                    FrameScope::Main
                } else {
                    FrameScope::Frame(frame.id.clone())
                },
            };
            match evaluate::run_built(&tab, expression, args.to_vec()).await {
                Ok(_) => {}
                Err(PageError::Threw(message)) => {
                    thrown.get_or_insert(message);
                }
                Err(error) => debug!(?error, frame = %frame.id, "a frame skipped a clock call"),
            }
        }
    }
    thrown.map_or(Ok(()), |message| Err(PlaywrightError::error(message)))
}

fn page_error(error: PageError) -> PlaywrightError {
    match error {
        PageError::Threw(message) => PlaywrightError::error(message),
        _ => PlaywrightError::target_closed(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticks_of(text: &str) -> Result<f64, String> {
        parse_ticks(&Given::String(text.to_string())).map_err(|error| error.message)
    }

    #[test]
    fn ticks_read_as_playwrights_server_reads_them() {
        assert_eq!(ticks_of("08"), Ok(8000.0));
        assert_eq!(ticks_of("01:00"), Ok(60_000.0));
        assert_eq!(ticks_of("02:34:10"), Ok(9_250_000.0));
        assert_eq!(ticks_of(""), Ok(0.0));
        assert_eq!(
            ticks_of("12:02:34:10").err().as_deref(),
            Some("Clock only understands numbers, 'mm:ss' and 'hh:mm:ss'")
        );
        assert_eq!(
            ticks_of("1:00").err().as_deref(),
            Some("Clock only understands numbers, 'mm:ss' and 'hh:mm:ss'")
        );
        assert_eq!(
            ticks_of("00:61").err().as_deref(),
            Some("Invalid time 00:61")
        );
        assert_eq!(parse_ticks(&Given::Number(120.0)).ok(), Some(120.0));
    }

    #[test]
    fn clock_commands_decode_as_playwrights_dispatcher_reads_them() {
        let route =
            |method: &str, params: Value| decode(method, Some(params)).map(|(_, route)| route);
        assert_eq!(
            route("clockInstall", json!({})).map(Result::ok),
            Some(Some(ClockRoute::Install(None)))
        );
        assert_eq!(
            route(
                "clockRunFor",
                json!({"ticksString": "01:00", "ticksNumber": 5})
            )
            .map(Result::ok),
            Some(Some(ClockRoute::RunFor(Given::String("01:00".into()))))
        );
        assert_eq!(
            route("clockPauseAt", json!({})).map(Result::ok),
            Some(Some(ClockRoute::PauseAt(Given::Number(0.0))))
        );
        assert!(
            route("clockSetSystemTime", json!({"timeNumber": null}))
                .is_some_and(|route| route.is_err())
        );
        assert!(route("clockResume", json!({"x": 1})).is_some_and(|route| route.is_err()));
        assert!(route("clockUninstall", json!({})).is_none());
    }

    #[test]
    fn a_log_script_applies_the_built_call_to_json_data() {
        let source = log_source("install", 1000.0, Some(5.0));
        assert!(
            source.ends_with(r#")("log", "install", 1000.0, 5.0)"#),
            "{source}"
        );
    }
}
