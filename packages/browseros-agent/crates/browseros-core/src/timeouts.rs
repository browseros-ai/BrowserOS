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
pub const WAIT_FOR_LOAD_TIMEOUT: Duration = Duration::from_secs(30);
pub const WAIT_FOR_LOAD_POLL: Duration = Duration::from_millis(150);
pub const NEW_PAGE_READY_ATTEMPTS: usize = 30;
pub const NEW_PAGE_READY_POLL: Duration = Duration::from_millis(100);
/// Total automatic post-action settle budget, deliberately short to keep agent actions responsive.
/// Callers awaiting slower asynchronous page changes should use the explicit `wait` tool.
pub const ACTION_SETTLE_DEFAULT_TIMEOUT: Duration = Duration::from_millis(50);
pub const ACTION_SETTLE_NAVIGATION_DETECT: Duration = Duration::from_millis(150);
pub const ACTION_SETTLE_NAVIGATION_POLL: Duration = Duration::from_millis(50);
pub const ACTION_SETTLE_DOM_QUIET: Duration = Duration::from_millis(100);
pub const ACTION_SETTLE_CDP_CALL_TIMEOUT: Duration = Duration::from_millis(250);
