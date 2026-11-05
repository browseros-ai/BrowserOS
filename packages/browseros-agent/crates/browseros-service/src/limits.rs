//! Every fixed number the host API runs with. The query and body bounds are the contract's
//! (`contracts/claw-sdk-api/openapi.yaml`); change them there first.

use std::time::Duration;

/// Events the log keeps for `Last-Event-ID` resume. A client further behind gets
/// `stream.reset` and re-reads state, so memory stays bounded however slow a client is.
pub(crate) const EVENT_RING: usize = 1024;

/// How often an idle event stream sends an SSE comment, so idle peers keep it open.
pub(crate) const SSE_KEEP_ALIVE: Duration = Duration::from_secs(15);

/// The longest `wait=` a request may hold, in seconds.
pub(crate) const MAX_WAIT_SECONDS: u64 = 60;

/// How long one screenshot capture may take before its job answers `Dropped` (decision 21).
pub(crate) const CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);

/// The longest one capture holds the camera. Past `CAPTURE_TIMEOUT` its job is already
/// answered, but the next job waits for it, so a stuck capture still counts against the
/// camera's concurrency of 1; at this cap it is aborted.
pub(crate) const STUCK_CAPTURE_CAP: Duration = Duration::from_secs(10);

/// JPEG quality of operation screenshots: about 150 KB at 1600×913 (D7).
pub(crate) const SCREENSHOT_QUALITY: i64 = 60;

/// The most screenshots `screenshots=last:N` inlines.
pub(crate) const MAX_INLINE_SCREENSHOTS: u32 = 50;

/// `GET /v1/runs` page size when `limit` is absent.
pub(crate) const DEFAULT_LIMIT: u32 = 25;

/// The largest `limit`.
pub(crate) const MAX_LIMIT: u32 = 100;

/// `GET /v1/trace` operations when `tail` is absent.
pub(crate) const DEFAULT_TAIL: u32 = 100;

/// The largest `tail`.
pub(crate) const MAX_TAIL: u32 = 1000;

/// The shortest host token accepted, in bytes. A hex-encoded 16-byte random token is the
/// shortest that passes; the app refuses to serve with anything shorter.
pub(crate) const MIN_TOKEN_BYTES: usize = 32;

/// `POST /v1/secrets` `ttlSeconds` bounds: a minute to a day.
pub(crate) const SECRET_TTL_SECONDS: std::ops::RangeInclusive<u64> = 60..=86_400;

/// A viewer's `width` and `height` bounds (`ViewerSize`): the sides a viewport can take.
pub(crate) const VIEWER_SIDE: std::ops::RangeInclusive<u16> = 100..=4096;

/// The most characters the owner's typed address may have (`NavigateRequest.input`).
pub(crate) const MAX_NAVIGATE_INPUT: usize = 2000;

/// The largest WebSocket message or frame a display takes from its viewer: a 1 MiB paste, its
/// header and the messages merged with it, with room to spare. tungstenite would otherwise
/// buffer up to 64 MiB for one message.
pub(crate) const DISPLAY_MESSAGE_MAX: usize = 2 << 20;

/// How long a display's close frame may take to leave. A viewer that stopped reading would
/// otherwise hold the display's task open.
pub(crate) const CLOSE_FRAME_WAIT: Duration = Duration::from_secs(5);
