use std::time::Duration;

/// Default page-text budget in UTF-8 bytes, about 5,000 estimated tokens.
pub const INLINE_PAGE_CONTENT_MAX_CHARS: usize = 15_000;
/// Caller-requested inline text is bounded independently of script return values
/// and saved parts, so a large accepted result can still be retained in full.
pub const MAX_INLINE_OVERRIDE_BYTES: usize = 150_000;
pub const GREP_MAX_MATCHES: usize = 200;
pub const GREP_MATCH_LINE_MAX_CHARS: usize = 500;
pub const TOOL_POST_ACTION_CAPTURE_TIMEOUT: Duration = Duration::from_millis(5_000);
