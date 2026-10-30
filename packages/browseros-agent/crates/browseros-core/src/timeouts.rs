use std::time::Duration;

pub const CDP_CONNECT: Duration = Duration::from_secs(10);
pub const CDP_CONNECT_RETRY_DELAY: Duration = Duration::from_secs(1);
pub const CDP_RECONNECT_DELAY: Duration = Duration::from_secs(5);
pub const CDP_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
pub const CDP_KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(10);
pub const CDP_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
pub const CONNECT_MAX_RETRIES: usize = 3;
pub const RECONNECT_MAX_RETRIES: usize = 3;
pub const WAIT_FOR_CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
pub const WAIT_FOR_CONNECTION_POLL: Duration = Duration::from_millis(50);
/// How many times to ask a freshly created tab for its info before giving up.
///
/// Only about the tab answering, not about it finishing. Readiness is decided
/// by `ARRIVAL_READY_BUDGET` against the page's own content.
pub const NEW_PAGE_ANSWERS_ATTEMPTS: usize = 10;
pub const NEW_PAGE_READY_POLL: Duration = Duration::from_millis(100);
/// Automatic post-action settle budget when the action did not navigate.
///
/// Has to clear more than one quiet interval, or the settle runs out before it
/// can read the readiness counter a second time and is inert. It was once 50ms,
/// below the 100ms interval on its own, so every settle expired unread. An
/// action that starts a navigation gets `ARRIVAL_READY_BUDGET` instead.
pub const ACTION_SETTLE_DEFAULT_TIMEOUT: Duration = Duration::from_millis(600);
pub const ACTION_SETTLE_DOM_QUIET: Duration = Duration::from_millis(100);
/// How long a reload or history step may take to report its navigation starting before the
/// load wait takes it to have stayed in the document (a fragment or pushState entry).
pub const NAVIGATION_START_GRACE: Duration = Duration::from_millis(150);
pub const ACTION_SETTLE_CDP_CALL_TIMEOUT: Duration = Duration::from_millis(250);

/// Ceiling for deciding a freshly arrived page is ready to be acted on.
///
/// Readiness is answered by the page's own content going quiet, not by the
/// load lifecycle finishing, so this is a ceiling rather than the usual cost.
/// A settled page answers after one quiet interval.
pub const ARRIVAL_READY_BUDGET: Duration = Duration::from_millis(1_500);

/// How long check/uncheck wait for a clicked control to report its new state. Frameworks
/// can apply it a tick after the click event, so one read straight after would misreport.
pub const CHECK_STATE_SETTLE: Duration = Duration::from_millis(500);
pub const CHECK_STATE_POLL: Duration = Duration::from_millis(50);

/// Mouse moves a drag takes between press and release. Steps past the first are what turn
/// a press into a drag for scripts that track movement, and give drop targets dragover.
pub const DRAG_STEPS: u32 = 10;

/// Agent pointer travel time: the page plays each move over about this long.
pub const POINTER_TRAVEL: Duration = Duration::from_millis(400);
/// Hard cap on waiting for the pointer to arrive. A safety net, not the normal
/// path: hidden pages skip drawing instead of waiting it out.
pub const POINTER_MOVE_DEADLINE: Duration = Duration::from_millis(1_000);
/// Cap on the no-wait pointer calls: click effect and screenshot hide/show.
pub const POINTER_EFFECT_DEADLINE: Duration = Duration::from_millis(250);
/// The page removes its pointer this long after the last agent action.
pub const POINTER_IDLE: Duration = Duration::from_millis(4_000);
