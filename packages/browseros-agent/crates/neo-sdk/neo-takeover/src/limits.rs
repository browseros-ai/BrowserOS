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

/// One browser step of a surface plan: Ctrl+0, a window state or a viewport. A viewport waits
/// for every holder of its tab, then up to 2 s for the page to report the change. The plan's
/// activation is bounded by [`ACTIVATE_TIMEOUT`] instead.
pub(crate) const STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// One `Browser.activateTab`, from any mover: a follower, a fit's claim, or the agent. Each
/// holds the front turn until it is answered, so this keeps one mover from holding back every
/// other for the CDP client's 60 s request timeout.
pub(crate) const ACTIVATE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long views stay refused after the agent activates a private tab, waiting for
/// `Browser.getTabs` to read it as in front. Then they are let back, and each view's admission
/// reads the front itself (decision 46).
pub(crate) const FRONT_SHOWN: Duration = Duration::from_secs(1);

/// How often that read repeats.
pub(crate) const FRONT_SHOWN_POLL: Duration = Duration::from_millis(25);

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

/// The longest paste a display takes, in bytes of ClientCutText body (plain or extended). A
/// longer one closes the display as soon as its length arrives, so no viewer can make Neo buffer
/// more than this for one message.
pub(crate) const MAX_PASTE: usize = 1 << 20;

/// The most screens a viewer's SetDesktopSize may list. Neo has one; noVNC sends one.
pub(crate) const MAX_DESKTOP_SCREENS: u8 = 16;

/// The most a display holds of its viewer's bytes while the first input waits for every open tab
/// to go private. They wait in memory, behind that input, so a viewer cannot grow them without
/// end while a commit stalls. Room for one more paste of the longest kind, and the pointer moves
/// and update requests around it; more closes the display.
pub(crate) const MAX_HELD_INPUT: usize = 2 * MAX_PASTE;

/// How long a display waits for Xvnc to accept its connection. Xvnc is on the same host, so a
/// longer wait means it is not there.
pub(crate) const XVNC_DIAL: Duration = Duration::from_secs(2);

/// The most Xvnc bytes one read takes before they go to the viewer as one frame.
pub(crate) const SERVER_CHUNK: usize = 64 * 1024;

/// How often a follower re-checks its pause and its tab, besides each operation the followed
/// task starts. It also covers a trace event stream that lagged.
pub(crate) const FOLLOW_TICK: Duration = Duration::from_secs(1);

/// The give-back's budget: a takeover's release steps together, from closing the owner's display
/// to restoring the fit. The unlock does not wait longer than this. The TTL release also starts
/// this long before the lock's deadline, so a give-back that takes all of it still finishes
/// while the agent is locked out, and a start waits for its fit only until then.
pub(crate) const GIVE_BACK: Duration = Duration::from_secs(10);

/// The shortest takeover (decision 6). Its TTL release comes 10 s before the end, so a much
/// shorter one would end before the owner could do anything.
pub(crate) const MIN_TAKEOVER_TTL: Duration = Duration::from_secs(60);

/// How long a released takeover stays readable, for `GET /v1/takeovers/{id}` and an idempotent
/// release. After that its id names nothing, so the released list stays bounded.
pub(crate) const KEEP_RELEASED_TAKEOVER: Duration = Duration::from_secs(60 * 60);

/// Takeover state changes the broadcast holds for a slow reader. The service's event log reads
/// them at once; one that falls this far behind gets `stream.reset` instead.
pub(crate) const TAKEOVER_CHANGES: usize = 256;

/// How often the front-tab watch reads which tab is in front while any view display is open
/// (decision 34). The browser sends no event when a tab comes to the front, so this bounds how
/// long a view can show a private tab that something other than Neo brought forward.
pub(crate) const FRONT_POLL: Duration = Duration::from_millis(250);
