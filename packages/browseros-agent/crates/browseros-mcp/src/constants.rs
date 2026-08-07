use std::time::Duration;

pub const INLINE_PAGE_CONTENT_MAX_CHARS: usize = 5_000;
pub const GREP_MAX_MATCHES: usize = 200;
pub const GREP_MATCH_LINE_MAX_CHARS: usize = 500;
pub const TOOL_POST_ACTION_CAPTURE_TIMEOUT: Duration = Duration::from_millis(5_000);
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_millis(60_000);
