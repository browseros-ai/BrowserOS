//! `__neo_runtime`, the only door from a run's JS into Rust. neo-sdk's `runtime-bridge.ts`
//! calls `get` for every synchronous Playwright getter and the realm's `console` calls `log`.
//!
//! ```text
//!  page.url()                                    JS, on the run's thread
//!    └─ __neo_runtime.get('{"guid":"page@<id>","field":"url"}')
//!         └─ Getters::get: parse → (object, field) → getters::url   ← blocks this thread
//!              └─ CDP request ──▶ BrowserOS ──▶ reply, read by browseros-cdp's reader task
//!         ◀─ '"https://…"'                       JSON text of the value
//! ```

use std::future::Future;
use std::mem;
use std::sync::{Arc, Mutex};

use browseros_cdp::{CdpClient, CdpError, SessionId};
use rquickjs::function::Func;
use rquickjs::{Ctx, Exception, Object};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::task;

use crate::deadline::{Deadline, Stop};
use crate::getters;
use crate::run::{PageId, RunError};

/// The guid the realm gives a run's page, in the `<type>@<id>` form Playwright's protocol uses.
pub(crate) fn page_guid(page: &PageId) -> String {
    format!("page@{}", page.as_str())
}

/// Installs `__neo_runtime` as a global of the realm `ctx`.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, getters: Getters, logs: Logs) -> rquickjs::Result<()> {
    let bridge = Object::new(ctx.clone())?;
    bridge.set(
        "get",
        Func::from(move |ctx: Ctx<'js>, request: String| getters.get(&ctx, &request)),
    )?;
    bridge.set("log", Func::from(move |line: String| logs.push(line)))?;
    ctx.globals().set("__neo_runtime", bridge)
}

/// The most `console` text and lines a run keeps. Kept lines live in Rust, outside the realm's
/// memory limit, so without a cap one script could hold any amount of the server's memory.
const MAX_LOG_BYTES: usize = 1024 * 1024;
const MAX_LOG_LINES: usize = 1000;

/// Lines the script wrote with `console`. Shared because the realm owns the JS `log` function
/// while the run reads the lines back, also after a deadline dropped the realm mid-script.
///
/// Keeps the first lines up to [`MAX_LOG_LINES`] and [`MAX_LOG_BYTES`]. The first line past
/// either cap ends keeping: it and every later line are only counted, so the kept lines are
/// always the start of what the script wrote, and [`Logs::take`] ends them with a note saying
/// how many were dropped. Logging itself never fails, so the cap never throws into the script.
#[derive(Debug, Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<LogLines>>);

#[derive(Debug, Default)]
struct LogLines {
    kept: Vec<String>,
    kept_bytes: usize,
    /// Set by the first line that did not fit.
    overflow: Option<Overflow>,
}

#[derive(Debug)]
struct Overflow {
    limit: LogLimit,
    dropped: usize,
}

#[derive(Debug, Clone, Copy)]
enum LogLimit {
    Lines,
    Bytes,
}

impl Logs {
    fn push(&self, line: String) {
        if let Ok(mut lines) = self.0.lock() {
            lines.push(line);
        }
    }

    pub(crate) fn take(&self) -> Vec<String> {
        let Ok(mut lines) = self.0.lock() else {
            return Vec::new();
        };
        let LogLines {
            mut kept, overflow, ..
        } = mem::take(&mut *lines);
        kept.extend(overflow.map(|overflow| overflow.note()));
        kept
    }
}

impl LogLines {
    fn push(&mut self, line: String) {
        if let Some(overflow) = &mut self.overflow {
            overflow.dropped += 1;
            return;
        }
        if let Some(limit) = self.limit_passed_by(&line) {
            self.overflow = Some(Overflow { limit, dropped: 1 });
        } else {
            self.kept_bytes += line.len();
            self.kept.push(line);
        }
    }

    fn limit_passed_by(&self, line: &str) -> Option<LogLimit> {
        if self.kept.len() >= MAX_LOG_LINES {
            Some(LogLimit::Lines)
        } else if self.kept_bytes + line.len() > MAX_LOG_BYTES {
            Some(LogLimit::Bytes)
        } else {
            None
        }
    }
}

impl Overflow {
    fn note(&self) -> String {
        let lines = if self.dropped == 1 { "line" } else { "lines" };
        let limit = match self.limit {
            LogLimit::Lines => format!("{MAX_LOG_LINES} lines"),
            LogLimit::Bytes => format!("{} MiB of console text", MAX_LOG_BYTES >> 20),
        };
        format!(
            "neo-runtime dropped the last {} console {lines}: a run keeps at most {limit}",
            self.dropped
        )
    }
}

/// Everything `__neo_runtime.get` needs to answer for one run.
pub(crate) struct Getters {
    cdp: CdpClient,
    page: PageId,
    page_guid: String,
    session: SessionId,
    deadline: Deadline,
    tokio: Handle,
}

/// The run's objects a script can name. A run sees exactly one page, so a guid for any other
/// target, such as another tab of the same browser, names nothing.
#[derive(Debug, Clone, Copy)]
enum ObjectRef {
    Page,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum GetterField {
    Url,
    IsClosed,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetRequest {
    guid: String,
    field: GetterField,
}

/// Why a getter threw. Each maps to the JS error the script sees.
#[derive(Debug)]
enum GetError {
    BadRequest(String),
    Cdp(CdpError),
    Stopped(Stop),
}

impl Getters {
    pub(crate) fn new(
        cdp: CdpClient,
        page: PageId,
        session: SessionId,
        deadline: Deadline,
        tokio: Handle,
    ) -> Self {
        Self {
            page_guid: page_guid(&page),
            cdp,
            page,
            session,
            deadline,
            tokio,
        }
    }

    fn get(&self, ctx: &Ctx<'_>, request: &str) -> rquickjs::Result<String> {
        match self.answer(request) {
            Ok(value) => Ok(value.to_string()),
            Err(error) => Err(self.throw(ctx, error)),
        }
    }

    fn answer(&self, request: &str) -> Result<Value, GetError> {
        let request: GetRequest = serde_json::from_str(request)
            .map_err(|error| GetError::BadRequest(format!("bad getter request: {error}")))?;
        let object = self
            .resolve(&request.guid)
            .ok_or_else(|| GetError::BadRequest(format!("unknown guid {:?}", request.guid)))?;
        match (object, request.field) {
            (ObjectRef::Page, GetterField::Url) => self
                .wait(getters::url(&self.cdp, &self.session, &self.page))
                .map(|url| json!(url)),
            (ObjectRef::Page, GetterField::IsClosed) => self
                .wait(getters::is_closed(&self.cdp, &self.page))
                .map(|closed| json!(closed)),
        }
    }

    fn resolve(&self, guid: &str) -> Option<ObjectRef> {
        (guid == self.page_guid).then_some(ObjectRef::Page)
    }

    /// Waits on the current thread for one CDP reply, so a JS getter can return a plain
    /// value. This is safe because a run's JS is single-threaded: while the getter waits the
    /// script has nothing else it could do. `block_in_place` hands this tokio worker's other
    /// tasks to another thread first, so other runs and browseros-cdp's reader task, which
    /// delivers the reply, keep going. That needs tokio's multi-threaded runtime, which
    /// `Runtime::run` checks before a realm exists. The wait ends early at the run's deadline
    /// or on cancellation.
    fn wait<T>(&self, read: impl Future<Output = Result<T, CdpError>>) -> Result<T, GetError> {
        task::block_in_place(|| {
            self.tokio.block_on(async {
                tokio::select! {
                    stop = self.deadline.reached() => Err(GetError::Stopped(stop)),
                    result = read => result.map_err(GetError::Cdp),
                }
            })
        })
    }

    fn throw(&self, ctx: &Ctx<'_>, error: GetError) -> rquickjs::Error {
        let error = match error {
            GetError::BadRequest(message) => RunError::new("Error", message),
            GetError::Cdp(error) => RunError::new("Error", error.to_string()),
            GetError::Stopped(Stop::Timeout) => RunError::timeout(self.deadline.timeout()),
            GetError::Stopped(Stop::Cancelled) => {
                RunError::new("Error", "Run cancelled".to_string())
            }
        };
        throw_named(ctx, &error.name, &error.message)
    }
}

/// Throws a JS `Error` whose `name` is `name`, the way Playwright's own error classes read.
fn throw_named(ctx: &Ctx<'_>, name: &str, message: &str) -> rquickjs::Error {
    let exception = match Exception::from_message(ctx.clone(), message) {
        Ok(exception) => exception,
        Err(error) => return error,
    };
    if let Err(error) = exception.as_object().set("name", name) {
        return error;
    }
    exception.throw()
}
