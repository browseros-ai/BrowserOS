//! The screen: one X connection, one MIT-SHM segment the size of the root window, a Damage
//! object on the root that says *when* anything was drawn, and the root's ConfigureNotify that
//! says when the size changed (the host resizes Xvnc with RFB SetDesktopSize). A grab compares
//! rows with the last picture, so damage that changed no pixel costs no encode.

use std::sync::Arc;
use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant};

use x11rb::connection::{Connection, RequestConnection};
use x11rb::errors::ReplyError;
use x11rb::protocol::xproto::{
    self, ChangeWindowAttributesAux, ConnectionExt as _, EventMask, ImageFormat,
};
use x11rb::protocol::{ErrorKind, Event, damage, shm, xfixes};
use x11rb::rust_connection::RustConnection;

use crate::engine::Ev;

pub struct Capture {
    conn: Arc<RustConnection>,
    root: xproto::Window,
    damage: damage::Damage,
    depth: u8,
    seg: Option<Segment>,
    pub width: u16,
    pub height: u16,
    /// The last grabbed picture, BGRX rows of `width * 4` bytes.
    prev: Vec<u8>,
    lost: LostGrabs,
}

/// Why a grab failed.
#[derive(Debug, PartialEq)]
pub enum GrabError {
    /// The screen changed size under the grab (Neo fitting a phone, a rotation), so the segment
    /// no longer matches it. Refit and grab again; `first` is the first since the last good grab.
    Resized {
        first: bool,
    },
    Fatal(String),
}

/// More grabs than this lost to resizes within `LOST_WINDOW`, with no good grab between them,
/// means the size is not settling: give up as before.
const LOST_LIMIT: u32 = 20;
const LOST_WINDOW: Duration = Duration::from_secs(2);

/// Counts grabs lost to resizes since the last good grab.
#[derive(Default)]
struct LostGrabs {
    /// When the current count started (None: the last grab was good).
    since: Option<Instant>,
    count: u32,
}

impl LostGrabs {
    /// Counts one lost grab. Returns whether it is the first since a good grab, or an error once
    /// more than `LOST_LIMIT` fall within `LOST_WINDOW`.
    fn lost(&mut self, now: Instant) -> Result<bool, String> {
        let first = self.since.is_none();
        if self
            .since
            .is_none_or(|t| now.saturating_duration_since(t) > LOST_WINDOW)
        {
            (self.since, self.count) = (Some(now), 0);
        }
        self.count += 1;
        if self.count > LOST_LIMIT {
            return Err(format!(
                "{} grabs lost to screen resizes within {LOST_WINDOW:?}",
                self.count
            ));
        }
        Ok(first)
    }

    fn good(&mut self) {
        *self = LostGrabs::default();
    }
}

/// A GetImage BadMatch means the rectangle no longer fits the root: the screen shrank under us.
fn resized(e: &ReplyError) -> bool {
    matches!(e, ReplyError::X11Error(x) if x.error_kind == ErrorKind::Match)
}

struct Segment {
    xid: shm::Seg,
    addr: *mut u8,
    len: usize,
}

// The segment is only touched by the engine thread that owns the Capture.
unsafe impl Send for Capture {}

fn err<E: std::fmt::Debug>(what: &str) -> impl FnOnce(E) -> String + '_ {
    move |e| format!("{what}: {e:?}")
}

impl Capture {
    /// Connects, checks MIT-SHM and DAMAGE, and starts the event thread that feeds `events`.
    pub fn open(display: &str, events: SyncSender<Ev>) -> Result<Capture, String> {
        let (conn, screen) = RustConnection::connect(Some(display)).map_err(err("connect to X"))?;
        let conn = Arc::new(conn);
        let screen = &conn.setup().roots[screen];
        check_format(conn.setup(), screen)?;
        let (root, depth) = (screen.root, screen.root_depth);
        if conn
            .extension_information(shm::X11_EXTENSION_NAME)
            .map_err(err("shm"))?
            .is_none()
        {
            return Err("the X server has no MIT-SHM".into());
        }
        shm::query_version(&*conn)
            .map_err(err("shm"))?
            .reply()
            .map_err(err("shm"))?;
        xfixes::query_version(&*conn, 5, 0)
            .map_err(err("xfixes"))?
            .reply()
            .map_err(err("xfixes"))?;
        damage::query_version(&*conn, 1, 1)
            .map_err(err("damage"))?
            .reply()
            .map_err(err("damage"))?;
        let damage = conn.generate_id().map_err(err("id"))?;
        damage::create(&*conn, damage, root, damage::ReportLevel::NON_EMPTY)
            .map_err(err("damage"))?
            .check()
            .map_err(err("damage"))?;
        conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
        )
        .map_err(err("select"))?
        .check()
        .map_err(err("select"))?;
        let mut cap = Capture {
            conn: conn.clone(),
            root,
            damage,
            depth,
            seg: None,
            width: 0,
            height: 0,
            prev: Vec::new(),
            lost: LostGrabs::default(),
        };
        cap.fit()?;
        std::thread::Builder::new()
            .name("slimviewer-x".into())
            .spawn(move || {
                loop {
                    let ev = match conn.wait_for_event() {
                        Ok(Event::DamageNotify(_)) => Ev::Damage,
                        Ok(Event::ConfigureNotify(e)) if e.window == root => Ev::Resized,
                        Ok(Event::Error(e)) => Ev::Fatal(format!("X request failed: {e:?}")),
                        Ok(_) => continue,
                        Err(_) => Ev::XGone,
                    };
                    let gone = matches!(ev, Ev::XGone | Ev::Fatal(_));
                    if events.send(ev).is_err() || gone {
                        return;
                    }
                }
            })
            .map_err(err("X event thread"))?;
        Ok(cap)
    }

    /// Re-reads the root size; on a change, makes a new segment and forgets the last picture.
    /// Returns whether the size changed.
    pub fn fit(&mut self) -> Result<bool, String> {
        let g = self
            .conn
            .get_geometry(self.root)
            .map_err(err("geometry"))?
            .reply()
            .map_err(err("geometry"))?;
        if self.seg.is_some() && (g.width, g.height) == (self.width, self.height) {
            return Ok(false);
        }
        if g.depth != self.depth {
            return Err("X root depth changed".into());
        }
        let len = frame_len(g.width, g.height)?;
        let mut prev = Vec::new();
        prev.try_reserve_exact(len)
            .map_err(err("frame allocation"))?;
        prev.resize(len, 0);
        let seg = self.attach(len)?;
        self.detach();
        self.seg = Some(seg);
        (self.width, self.height) = (g.width, g.height);
        self.prev = prev;
        Ok(true)
    }

    fn attach(&self, len: usize) -> Result<Segment, String> {
        let xid = self.conn.generate_id().map_err(err("id"))?;
        let id = unsafe { libc::shmget(libc::IPC_PRIVATE, len, libc::IPC_CREAT | 0o600) };
        if id < 0 {
            return Err(format!("shmget: {}", std::io::Error::last_os_error()));
        }
        let addr = unsafe { libc::shmat(id, std::ptr::null(), 0) };
        if addr as isize == -1 {
            let error = std::io::Error::last_os_error();
            unsafe { libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut()) };
            return Err(format!("shmat: {error}"));
        }
        // Check the attach itself: an unrelated round trip does not report BadAccess here.
        let attached = shm::attach(&*self.conn, xid, id as u32, false)
            .map_err(err("SHM attach"))
            .and_then(|request| request.check().map_err(err("SHM attach")));
        // Removed now, freed by the kernel once both sides detach (also if this process dies).
        let removed = unsafe { libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut()) } == 0;
        let remove_error = std::io::Error::last_os_error();
        if attached.is_err() || !removed {
            if attached.is_ok() {
                let _ = shm::detach(&*self.conn, xid);
                let _ = self.conn.flush();
            }
            unsafe { libc::shmdt(addr) };
            if !removed {
                unsafe { libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut()) };
            }
            attached?;
            return Err(format!("shmctl IPC_RMID: {remove_error}"));
        }
        Ok(Segment {
            xid,
            addr: addr.cast(),
            len,
        })
    }

    fn detach(&mut self) {
        if let Some(seg) = self.seg.take() {
            let _ = shm::detach(&*self.conn, seg.xid);
            let _ = self.conn.flush();
            unsafe { libc::shmdt(seg.addr.cast()) };
        }
    }

    /// Grabs the screen. Returns the band of rows that differ from the last grab and how many
    /// rows in it changed, or None. Damage is cleared *before* the grab, so drawing that races it
    /// is damaged again. A resize racing the grab is `GrabError::Resized`, not fatal.
    pub fn grab(&mut self) -> Result<Option<(usize, usize, usize)>, GrabError> {
        let band = self.grab_once();
        if band.is_ok() {
            self.lost.good();
        }
        match band {
            Err(Grab::Resized(why)) => match self.lost.lost(Instant::now()) {
                Ok(first) => Err(GrabError::Resized { first }),
                Err(e) => Err(GrabError::Fatal(format!("grab: {e}; last: {why}"))),
            },
            Err(Grab::Fatal(e)) => Err(GrabError::Fatal(e)),
            Ok(band) => Ok(band),
        }
    }

    fn grab_once(&mut self) -> Result<Option<(usize, usize, usize)>, Grab> {
        damage::subtract(&*self.conn, self.damage, x11rb::NONE, x11rb::NONE)
            .map_err(err("damage"))?;
        let seg = self.seg.as_ref().ok_or("no segment")?;
        let reply = shm::get_image(
            &*self.conn,
            self.root,
            0,
            0,
            self.width,
            self.height,
            !0,
            ImageFormat::Z_PIXMAP.into(),
            seg.xid,
            0,
        )
        .map_err(err("grab"))?
        .reply()
        .map_err(|e| {
            let why = format!("grab: {e:?}");
            if resized(&e) {
                Grab::Resized(why)
            } else {
                Grab::Fatal(why)
            }
        })?;
        if reply.depth != self.depth || reply.size as usize != seg.len {
            return Err(Grab::Resized("SHM image layout changed".into()));
        }
        let now = unsafe { std::slice::from_raw_parts(seg.addr, seg.len) };
        let stride = usize::from(self.width) * 4;
        let Some((y0, y1, changed)) = dirty_band(now, &self.prev, stride) else {
            return Ok(None);
        };
        self.prev[y0 * stride..y1 * stride].copy_from_slice(&now[y0 * stride..y1 * stride]);
        Ok(Some((y0, y1, changed)))
    }

    /// The last grabbed picture (BGRX, `width * 4` bytes per row).
    pub fn picture(&self) -> &[u8] {
        &self.prev
    }

    /// Forget the last picture, so the next grab reports the whole screen.
    pub fn forget(&mut self) {
        self.prev.fill(0);
    }
}

/// One grab's failure, before `LostGrabs` decides whether a resize is still survivable.
enum Grab {
    Resized(String),
    Fatal(String),
}

impl<T: Into<String>> From<T> for Grab {
    fn from(e: T) -> Grab {
        Grab::Fatal(e.into())
    }
}

fn frame_len(width: u16, height: u16) -> Result<usize, String> {
    const MAX_PIXELS: usize = 16 * 1024 * 1024;
    let pixels = usize::from(width) * usize::from(height);
    if width < 4 || height < 4 || pixels > MAX_PIXELS {
        return Err(format!(
            "unsupported screen size {width}x{height} (minimum 4x4, maximum {MAX_PIXELS} pixels)"
        ));
    }
    Ok(pixels * 4)
}

fn check_format(setup: &xproto::Setup, screen: &xproto::Screen) -> Result<(), String> {
    let bgrx = setup.image_byte_order == xproto::ImageOrder::LSB_FIRST
        && setup.pixmap_formats.iter().any(|format| {
            format.depth == screen.root_depth
                && format.bits_per_pixel == 32
                && format.scanline_pad == 32
        })
        && screen
            .allowed_depths
            .iter()
            .filter(|d| d.depth == screen.root_depth)
            .flat_map(|d| &d.visuals)
            .any(|v| {
                v.visual_id == screen.root_visual
                    && v.class == xproto::VisualClass::TRUE_COLOR
                    && v.red_mask == 0xff0000
                    && v.green_mask == 0xff00
                    && v.blue_mask == 0xff
            });
    if !bgrx {
        return Err("unsupported X pixel format (requires little-endian 32-bit BGRX)".into());
    }
    Ok(())
}

/// First and one-past-last row where `a` and `b` differ, and how many rows differ: a scroll
/// changes most rows, a banner or a caret only its own (the band alone cannot tell them apart).
fn dirty_band(a: &[u8], b: &[u8], stride: usize) -> Option<(usize, usize, usize)> {
    let rows = a.len() / stride;
    let same = |y: usize| a[y * stride..(y + 1) * stride] == b[y * stride..(y + 1) * stride];
    let first = (0..rows).find(|&y| !same(y))?;
    let last = (first..rows).rev().find(|&y| !same(y)).unwrap_or(first);
    let changed = (first..=last).filter(|&y| !same(y)).count();
    Some((first, last + 1, changed))
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{LOST_LIMIT, LOST_WINDOW, LostGrabs, dirty_band};
    use std::time::{Duration, Instant};
    use x11rb::errors::{ConnectionError, ReplyError};
    use x11rb::protocol::{ErrorKind, xproto};
    use x11rb::x11_utils::X11Error;

    fn get_image_error(error_kind: ErrorKind, error_code: u8) -> ReplyError {
        ReplyError::X11Error(X11Error {
            error_kind,
            error_code,
            sequence: 0,
            bad_value: 0,
            minor_opcode: 4,
            major_opcode: 131,
            extension_name: Some("MIT-SHM".into()),
            request_name: Some("GetImage"),
        })
    }

    #[test]
    fn get_image_bad_match_is_a_resize_and_others_are_fatal() {
        assert!(super::resized(&get_image_error(ErrorKind::Match, 8)));
        assert!(!super::resized(&get_image_error(ErrorKind::Value, 2)));
        assert!(!super::resized(&get_image_error(ErrorKind::Access, 10)));
        assert!(!super::resized(&ReplyError::ConnectionError(
            ConnectionError::UnknownError
        )));
    }

    #[test]
    fn lost_grabs_turn_fatal_only_past_the_limit_within_the_window() {
        let t = Instant::now();
        let mut lost = LostGrabs::default();
        assert_eq!(lost.lost(t), Ok(true));
        for i in 1..LOST_LIMIT {
            assert_eq!(
                lost.lost(t + Duration::from_millis(u64::from(i))),
                Ok(false)
            );
        }
        assert!(lost.lost(t + Duration::from_millis(50)).is_err());

        // A good grab starts over, and so does a count older than the window.
        lost.good();
        assert_eq!(lost.lost(t), Ok(true));
        for _ in 1..LOST_LIMIT {
            assert_eq!(lost.lost(t), Ok(false));
        }
        assert_eq!(
            lost.lost(t + LOST_WINDOW + Duration::from_millis(1)),
            Ok(false)
        );
    }

    fn bgrx_setup() -> (xproto::Setup, xproto::Screen) {
        let setup = xproto::Setup {
            image_byte_order: xproto::ImageOrder::LSB_FIRST,
            pixmap_formats: vec![xproto::Format {
                depth: 24,
                bits_per_pixel: 32,
                scanline_pad: 32,
            }],
            ..Default::default()
        };
        let screen = xproto::Screen {
            root_depth: 24,
            root_visual: 7,
            allowed_depths: vec![xproto::Depth {
                depth: 24,
                visuals: vec![xproto::Visualtype {
                    visual_id: 7,
                    class: xproto::VisualClass::TRUE_COLOR,
                    red_mask: 0xff0000,
                    green_mask: 0xff00,
                    blue_mask: 0xff,
                    ..Default::default()
                }],
            }],
            ..Default::default()
        };
        (setup, screen)
    }

    #[test]
    fn tests_that_bgrx_layout_is_accepted() {
        let (setup, screen) = bgrx_setup();
        assert!(super::check_format(&setup, &screen).is_ok());
    }

    #[test]
    fn tests_that_incompatible_layouts_are_rejected() {
        let (mut setup, mut screen) = bgrx_setup();
        setup.pixmap_formats[0].bits_per_pixel = 16;
        assert!(super::check_format(&setup, &screen).is_err());
        setup.pixmap_formats[0].bits_per_pixel = 32;
        setup.image_byte_order = xproto::ImageOrder::MSB_FIRST;
        assert!(super::check_format(&setup, &screen).is_err());
        setup.image_byte_order = xproto::ImageOrder::LSB_FIRST;
        screen.allowed_depths[0].visuals[0].red_mask = 0xff;
        assert!(super::check_format(&setup, &screen).is_err());
    }

    #[test]
    fn tests_that_small_and_large_screens_are_rejected() {
        assert_eq!(super::frame_len(780, 1688).unwrap(), 780 * 1688 * 4);
        assert!(super::frame_len(0, 1688).is_err());
        assert!(super::frame_len(3, 4).is_err());
        assert!(super::frame_len(u16::MAX, u16::MAX).is_err());
    }

    #[test]
    fn band_spans_first_to_last_changed_row() {
        let a = vec![0u8; 4 * 10];
        let mut b = a.clone();
        assert_eq!(dirty_band(&a, &b, 4), None);
        b[4 * 3] = 1;
        assert_eq!(dirty_band(&a, &b, 4), Some((3, 4, 1)));
        b[4 * 6 + 2] = 1;
        assert_eq!(dirty_band(&a, &b, 4), Some((3, 7, 2)));
        b[4 * 4] = 1;
        assert_eq!(dirty_band(&a, &b, 4), Some((3, 7, 3)));
    }
}
