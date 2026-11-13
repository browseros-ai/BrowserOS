//! Every fixed number the host API runs with. The query and body bounds are the contract's
//! (`crates/neo-sdk/neo-sdk-api/openapi.yaml`); change them there first.

use std::ops::RangeInclusive;
use std::time::Duration;

/// Events the log keeps for `Last-Event-ID` resume. A client further behind gets
/// `stream.reset` and re-reads state, so memory stays bounded however slow a client is.
pub(crate) const EVENT_RING: usize = 1024;

/// How often an idle event stream sends an SSE comment, so idle peers keep it open.
pub(crate) const SSE_KEEP_ALIVE: Duration = Duration::from_secs(15);

/// The longest `wait=` a request may hold, in seconds.
pub(crate) const MAX_WAIT_SECONDS: u64 = 60;

/// How long one screenshot capture attempt may take before it is aborted (decision 21). A busy
/// renderer can take longer, so the attempt is retried (decision 37).
pub(crate) const CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);

/// Capture attempts per screenshot job: the first and two retries. Only an attempt past
/// `CAPTURE_TIMEOUT` is retried.
pub(crate) const CAPTURE_ATTEMPTS: u32 = 3;

/// The pause before a retry, which gives a busy renderer time to finish its work.
pub(crate) const CAPTURE_BACKOFF: Duration = Duration::from_millis(500);

/// The longest one job holds the camera, its attempts and pauses together; past it the job
/// answers `Dropped`. Three attempts and two pauses take 7 s, so it binds only if those grow.
pub(crate) const CAPTURE_BUDGET: Duration = Duration::from_secs(8);
/// How long the camera waits for the app's reading of what a tab shows (ruling R45), once before
/// and once after a capture. It reads every frame, cross-site ones included; a reading that does
/// not come in time withholds the picture.
pub(crate) const VERDICT_BUDGET: Duration = Duration::from_secs(5);

/// The screenshot jobs that may wait for the camera on one tab (decision 37). A newer job drops
/// the tab's oldest waiting one, which answers `Dropped`: a capture that far behind its
/// operation would show the page long after it.
pub(crate) const MAX_WAITING_PER_TAB: usize = 4;

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

/// A viewer's `width` and `height` bounds (`ViewerSize`): the sides a viewport can take.
pub(crate) const VIEWER_SIDE: RangeInclusive<u16> = 100..=4096;

/// The most characters the owner's typed address may have (`NavigateRequest.input`).
pub(crate) const MAX_NAVIGATE_INPUT: usize = 2000;

/// The largest WebSocket message or frame a display takes from its viewer: a 1 MiB paste, its
/// header and the messages merged with it, with room to spare. tungstenite would otherwise
/// buffer up to 64 MiB for one message.
pub(crate) const DISPLAY_MESSAGE_MAX: usize = 2 << 20;

/// How long a display's close frame may take to leave. A viewer that stopped reading would
/// otherwise hold the display's task open.
pub(crate) const CLOSE_FRAME_WAIT: Duration = Duration::from_secs(5);

/// `tabs.idleCloseSeconds`, in seconds. No shorter than a minute, because the janitor checks once
/// a minute (`HOUSEKEEPING_EVERY` in the app); no longer than a day, so agent tabs cannot pile up
/// without end.
pub(crate) const IDLE_CLOSE_SECONDS: RangeInclusive<u64> = 60..=86_400;

/// `trace.retentionSeconds`, in seconds. No shorter than an hour: a finished run must still
/// answer a repeat of its `requestId` for 10 minutes, and a run can last 15. No longer than 30
/// days, which protects the disk.
pub(crate) const RETENTION_SECONDS: RangeInclusive<u64> = 3_600..=2_592_000;

/// A host's optional tab keep TTL. An absent TTL has no deadline of its own.
pub(crate) const KEEP_SECONDS: RangeInclusive<u64> = 1..=2_592_000;

/// How long the janitor can recover a keep's exact end instant after DELETE or lazy expiry.
pub(crate) const ENDED_KEEP_RETENTION: Duration = Duration::from_secs(3_600);

/// `trace.valueBytes`, in bytes: up to Neo's own reply limit, 256 KiB, which is what the Neo
/// viewer asks for. `0` keeps the trace's default limits.
pub(crate) const VALUE_BYTES: RangeInclusive<u64> = 0..=262_144;
