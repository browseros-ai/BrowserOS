//! Every fixed number the takeover parts run with (synthesis item 8). None of them is a setting:
//! v1 has no config file, and a per-request choice belongs in the request (decision 19).

use std::time::Duration;

/// The shortest lock a host can take.
pub(crate) const MIN_LOCK_TTL: Duration = Duration::from_secs(1);

/// The longest lock, so a host that crashed cannot keep the agent off a tab for longer
/// (decision 19). The contract's `ttlSeconds` maximum is the same 1200.
pub(crate) const MAX_LOCK_TTL: Duration = Duration::from_secs(20 * 60);

/// How long taking a lock waits for the agent's in-flight writes on its tab to finish.
pub(crate) const SETTLE_BUDGET: Duration = Duration::from_secs(2);

/// How often the settle reads the tab's open writes from the trace store.
pub(crate) const SETTLE_POLL: Duration = Duration::from_millis(50);

/// How long an ended lock stays readable, for `GET /v1/locks/{id}` and an idempotent `DELETE`.
/// After that its id names nothing, so the ended list stays bounded.
pub(crate) const KEEP_ENDED_LOCK: Duration = Duration::from_secs(60 * 60);

/// Lock state changes the broadcast holds for a slow reader. The service's event log reads
/// them at once; one that falls this far behind gets `stream.reset` instead.
pub(crate) const LOCK_CHANGES: usize = 256;

/// How long one CDP step on a frame, or on the target that hosts it, may take before the
/// sealed-world work skips it (design D13), so one stuck iframe cannot hide the rest of a tab.
pub(crate) const FRAME_BUDGET: Duration = Duration::from_secs(2);

/// How long the control journal waits after its last entry before it registers what it holds
/// (decision 32). A second with no entry is taken as the owner done with the field: its value is
/// registered, and the prefixes typed on the way to it are not.
pub(crate) const JOURNAL_QUIET: Duration = Duration::from_secs(1);

/// The most values the control journal holds unregistered. A page can raise `input` events on
/// its own fields, so past this the value typed longest ago is registered at once, and each entry
/// is checked against a bounded set.
pub(crate) const MAX_JOURNAL_PENDING: usize = 64;
/// A screen side under this gets no fit at all: Xvnc can report one, and no viewport is that
/// small (core's `viewport::MIN_SIZE`).
pub(crate) const MIN_SIDE: u16 = 100;

/// The tallest mobile viewport a fit sets (core's `viewport::MAX_SIZE`).
pub(crate) const MAX_SIDE: u16 = 4096;

/// The widest screen that gets the phone layout: the owner page's own breakpoint (decision 20).
pub(crate) const MOBILE_MAX_WIDTH: u16 = 600;

/// One Xvnc resize, from dialing to the verdict on its request.
pub(crate) const RESIZE_TIMEOUT: Duration = Duration::from_secs(2);

/// How many server messages a resize reads for its verdict before it gives up.
pub(crate) const RESIZE_MESSAGES: usize = 16;

/// How long the screen watcher waits before it dials Xvnc again.
pub(crate) const WATCH_REDIAL: Duration = Duration::from_secs(1);

/// How long a window state change may take to read back. Fullscreen settles in about 5 ms.
pub(crate) const WINDOW_SETTLE: Duration = Duration::from_secs(3);

/// How often a window state change is read back.
pub(crate) const WINDOW_POLL: Duration = Duration::from_millis(25);

/// One browser step of a surface plan: bringing a tab to the front, Ctrl+0, a window state or
/// a viewport. A viewport waits for every holder of its tab, then up to 2 s for the page to
/// report the change.
pub(crate) const STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a fit waits for the screen watcher to report the size it just set, so the fits
/// loop never takes an older report for the owner's resize.
pub(crate) const WATCH_CATCH_UP: Duration = Duration::from_secs(1);

/// How often each active fit's owner view is read. The owner's viewer drops keyboard boxes
/// older than 1.5 s.
pub(crate) const OWNER_VIEW_TICK: Duration = Duration::from_millis(750);

/// One owner view read, a popup pulled into the fitted tab included.
pub(crate) const OWNER_VIEW_BUDGET: Duration = Duration::from_secs(5);

/// The most keyboard boxes one read reports, and the most fields it looks at to find them.
pub(crate) const MAX_KEYBOARD_BOXES: usize = 128;
pub(crate) const MAX_SCANNED_FIELDS: usize = 512;

/// How long a pulled-in popup may take to close before it is left where it is, and how often
/// that is checked.
pub(crate) const POPUP_CLOSE_WAIT: Duration = Duration::from_secs(1);
pub(crate) const POPUP_CLOSE_POLL: Duration = Duration::from_millis(100);

/// One move of the owner's browser bar: back, forward, reload or open.
pub(crate) const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(20);

/// Where the owner's typed text goes when it is not a web address (decision 19).
pub(crate) const SEARCH_URL: &str = "https://www.google.com/search?q=";

/// The longest address the owner's browser bar opens.
pub(crate) const MAX_ADDRESS_LEN: usize = 2000;

/// How long a restored fit stays readable, for `GET /v1/fits/{id}` and an idempotent restore.
pub(crate) const KEEP_RESTORED_FIT: Duration = Duration::from_secs(60 * 60);

/// The longest framebuffer rectangle the screen's own clients accept, in bytes: a 4096×4096
/// screen at 32 bits. They ask for 1×1 updates, so anything near this is not Xvnc.
pub(crate) const MAX_RECT_BYTES: usize = 4096 * 4096 * 4;

/// The longest desktop name a ServerInit may carry.
pub(crate) const MAX_DESKTOP_NAME: usize = 64 * 1024;
