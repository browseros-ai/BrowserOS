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
/// Total automatic post-action settle budget.
///
/// Has to clear the navigation detect window plus at least one quiet interval,
/// or `wait_for_dom_quiet` runs out of budget before it can read the mutation
/// counter a second time and the settle is inert. It was 50ms, below the 100ms
/// quiet interval on its own, so every automatic settle returned
/// `BudgetExpired` having installed an observer it never read twice.
pub const ACTION_SETTLE_DEFAULT_TIMEOUT: Duration = Duration::from_millis(600);
pub const ACTION_SETTLE_NAVIGATION_DETECT: Duration = Duration::from_millis(150);
pub const ACTION_SETTLE_NAVIGATION_POLL: Duration = Duration::from_millis(50);
pub const ACTION_SETTLE_DOM_QUIET: Duration = Duration::from_millis(100);
pub const ACTION_SETTLE_CDP_CALL_TIMEOUT: Duration = Duration::from_millis(250);

/// Ceiling for deciding a freshly arrived page is ready to be acted on.
///
/// Readiness is answered by the page's own content going quiet, not by the
/// load lifecycle finishing, so this is a ceiling rather than the usual cost.
/// A settled page answers after one quiet interval.
pub const ARRIVAL_READY_BUDGET: Duration = Duration::from_millis(1_500);
