use std::net::SocketAddr;
use std::time::Duration;

use neo_takeover::{ScreenSize, Xvnc};

/// Every fixed value in claw-sdk, as an associated constant (`Defaults::RUN_TIMEOUT`), in one
/// place (design D2). None of them is a deployment setting: a change is a code change.
pub(crate) struct Defaults;

impl Defaults {
    /// The agent listener's port when the launch JSON names none (Chromium's own default).
    pub(crate) const SERVER_PORT: u16 = 9200;
    /// Chromium's CDP port when the launch JSON names none.
    pub(crate) const CDP_PORT: u16 = 49337;
    /// The log filter when `CLAW_LOG` is unset. rmcp logs every stateless request at info.
    pub(crate) const LOG_FILTER: &str = "info,rmcp=warn";
    /// Xvnc's RFB address when `CLAW_XVNC_ADDR` is unset: where the Neo image's entrypoint pins
    /// it.
    pub(crate) const XVNC_ADDR: SocketAddr = Xvnc::NEO_ADDR;
    /// Neo's screen size between takeover fits when `NEO_SCREEN` is unset, as the image's
    /// Dockerfile and `entrypoint.sh` set it for Xvnc.
    pub(crate) const SCREEN: ScreenSize = ScreenSize {
        width: 1600,
        height: 1000,
    };
    /// A run's budget when the model sends no `timeout`. Fits two CAPTCHA solves (120 s each)
    /// plus page work, so a host sets nothing (decision 4).
    pub(crate) const RUN_TIMEOUT: Duration = Duration::from_secs(300);
    /// The longest `timeout` a model may ask for.
    pub(crate) const RUN_TIMEOUT_MAX: Duration = Duration::from_mins(15);
    /// The most one run's QuickJS heap may grow to: a ceiling, never memory set aside. Each run
    /// has its own runtime, which allocates as the script needs and frees all of it when the run
    /// ends. Measured on Neo's Linux image (2026-11-12): `return 1` and a small page's snapshot
    /// stay under 8 MiB; an 88,000-element Hacker News thread needs 64 MiB for
    /// `ariaSnapshot()`, 76 for `{ mode: 'full' }` and 88 for `{ mode: 'default' }`, while that
    /// tab's renderer alone holds about 880 MiB. 128 MiB, the owner's choice, leaves the largest
    /// of those about half again.
    pub(crate) const JS_MEMORY_BYTES: usize = 128 * 1024 * 1024;
    /// QuickJS's stack per run.
    pub(crate) const JS_STACK_BYTES: usize = 512 * 1024;
    /// The longest one synchronous slice of JS may run before `script / long_task`. Without a
    /// concurrency limit, a `for(;;){}` must not hold a worker for the whole run. 50k rows of
    /// CPU-bound JS took 230 ms in the runtime spike, so this is about 20x headroom.
    pub(crate) const JS_SLICE_BUDGET: Duration = Duration::from_secs(5);
    /// The script's return value, as JSON.
    pub(crate) const MAX_RESULT_BYTES: usize = 256 * 1024;
    /// Console lines one run keeps; later lines are counted, not kept.
    pub(crate) const MAX_LOG_LINES: usize = 1000;
    /// Console text one run keeps, in bytes, across all its lines.
    pub(crate) const MAX_LOG_BYTES: usize = 1024 * 1024;
    /// How long a stopped run's script, and then its ops, get to end through the normal path
    /// before they are dropped. A script awaiting a promise that never settles ends only this
    /// way.
    pub(crate) const TEARDOWN_GRACE: Duration = Duration::from_secs(2);
    /// `hostData`, as JSON. The trace writer cuts at 2048 bytes, so a larger value is refused
    /// rather than stored cut.
    pub(crate) const HOST_DATA_MAX_BYTES: usize = 2048;
    /// `requestId` and `taskId`, each.
    pub(crate) const ID_MAX_BYTES: usize = 128;
    /// How long a finished run's reply answers a repeat of its `requestId`. The trace writer
    /// keeps the stored replay as long, so memory and the store agree.
    pub(crate) const REPLAY_WINDOW: Duration = Duration::from_mins(10);
    /// Requests whose run the trace never confirmed it stored, kept in memory for Neo's life as
    /// the only proof they ran. At this many, a new `requestId` is refused as
    /// `neo / trace_unavailable`: a trace that keeps losing writes cannot back the promise that
    /// a request never runs twice. About 0.5 KB each.
    pub(crate) const MAX_UNRECORDED_REQUESTS: usize = 4096;
    /// How long a `requestId` run outlives its last waiter, so an HTTP retry with the same id
    /// rejoins it instead of running it twice. Short, so a run the host gave up on stops acting
    /// on the owner's browser.
    pub(crate) const ORPHAN_GRACE: Duration = Duration::from_secs(15);
    /// rmcp's SSE keep-alive. Stateless rmcp notices a gone client only on its next write, so
    /// this bounds how long an abandoned run keeps going.
    pub(crate) const SSE_KEEP_ALIVE: Duration = Duration::from_secs(5);
    /// After SIGTERM, how long in-flight requests get to answer before Neo exits anyway, and
    /// then how long the trace writer gets to commit. Chromium sends SIGKILL 5 s after SIGTERM,
    /// so the two together stay under it.
    pub(crate) const SHUTDOWN_DRAIN: Duration = Duration::from_secs(3);
    pub(crate) const SHUTDOWN_FLUSH: Duration = Duration::from_secs(1);
    /// How often housekeeping closes idle agent tabs.
    pub(crate) const HOUSEKEEPING_EVERY: Duration = Duration::from_mins(1);
    /// How often housekeeping deletes trace records past `RetentionPolicy`'s age and cap.
    pub(crate) const TRACE_SWEEP_EVERY: Duration = Duration::from_mins(10);
    /// Tokio worker stacks. QuickJS plus nested futures overflowed the default 2 MiB in debug
    /// builds on Linux, which aborts the process. Reserved address space, committed as used.
    pub(crate) const WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;
}
