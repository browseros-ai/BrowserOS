//! X11 CLIPBOARD changes, independent of Xvnc's disabled ServerCutText channel.
//! Each owner and transfer gets its own X window; late replies cannot cross a handoff.

use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, GetPropertyReply, Property,
    Window, WindowClass,
};
use x11rb::protocol::{Event, xfixes};
use x11rb::rust_connection::RustConnection;

use crate::engine::Ev;
use crate::wire::MAX_PASTE_BYTES;

const TRANSFER_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Shared {
    access: Mutex<Access>,
    closed: AtomicBool,
}

#[derive(Default)]
struct Access {
    allowed: u64,
    active: u64,
}

impl Access {
    fn authorize(&mut self, owner: u64) {
        if owner != self.allowed {
            self.allowed = owner;
            self.active = 0;
        }
    }

    fn activate(&mut self, owner: u64) {
        if owner == self.allowed {
            self.active = owner;
        }
    }
}

impl Shared {
    fn owner(&self) -> u64 {
        self.access.lock().map_or(0, |access| access.active)
    }
}

pub struct Clipboard {
    shared: Arc<Shared>,
    wake: UnixStream,
}

impl Clipboard {
    pub fn open(display: &str, events: SyncSender<Ev>) -> Result<Self, String> {
        let (conn, screen) = RustConnection::connect(Some(display)).map_err(error)?;
        let root = conn.setup().roots[screen].root;
        xfixes::query_version(&conn, 5, 0)
            .map_err(error)?
            .reply()
            .map_err(error)?;
        let atoms = Atoms {
            clipboard: atom(&conn, b"CLIPBOARD")?,
            utf8: atom(&conn, b"UTF8_STRING")?,
            incr: atom(&conn, b"INCR")?,
            data: atom(&conn, b"SLIMVIEWER_CLIPBOARD")?,
        };
        let (wake, notified) = UnixStream::pair().map_err(error)?;
        wake.set_nonblocking(true).map_err(error)?;
        notified.set_nonblocking(true).map_err(error)?;
        let shared = Arc::new(Shared::default());
        let mut worker = Worker {
            conn,
            root,
            atoms,
            shared: shared.clone(),
            notified,
            events,
            scope: None,
            transfer: None,
        };
        std::thread::Builder::new()
            .name("slimviewer-clipboard".into())
            .spawn(move || {
                if let Err(reason) = worker.run() {
                    eprintln!("slimviewer: event=clipboard unavailable={reason}");
                }
            })
            .map_err(error)?;
        Ok(Self { shared, wake })
    }

    /// Permission changes happen on the engine thread. Activation waits for the input queue.
    pub fn authorize(&self, owner: Option<u64>) {
        if let Ok(mut access) = self.shared.access.lock() {
            access.authorize(owner.unwrap_or(0));
        }
        let _ = (&self.wake).write(&[1]);
    }

    /// Queued after the prior owner's input and RFB processing fence. Stale commands do nothing.
    pub fn activate(&self, owner: u64) {
        if let Ok(mut access) = self.shared.access.lock() {
            access.activate(owner);
        }
        let _ = (&self.wake).write(&[1]);
    }

    pub fn deactivate(&self) {
        if let Ok(mut access) = self.shared.access.lock() {
            access.active = 0;
        }
        let _ = (&self.wake).write(&[1]);
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
        let _ = (&self.wake).write(&[1]);
    }
}

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn atom(conn: &RustConnection, name: &[u8]) -> Result<Atom, String> {
    Ok(conn
        .intern_atom(false, name)
        .map_err(error)?
        .reply()
        .map_err(error)?
        .atom)
}

struct Atoms {
    clipboard: Atom,
    utf8: Atom,
    incr: Atom,
    data: Atom,
}

struct Scope {
    owner: u64,
    window: Window,
}

struct Transfer {
    owner: u64,
    window: Window,
    deadline: Instant,
    incremental: bool,
    bytes: Vec<u8>,
}

struct Worker {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
    shared: Arc<Shared>,
    notified: UnixStream,
    events: SyncSender<Ev>,
    scope: Option<Scope>,
    transfer: Option<Transfer>,
}

impl Worker {
    fn window(&self) -> Result<Window, String> {
        let window = self.conn.generate_id().map_err(error)?;
        self.conn
            .create_window(
                0,
                window,
                self.root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            )
            .map_err(error)?
            .check()
            .map_err(error)?;
        Ok(window)
    }

    fn cancel_transfer(&mut self) -> Result<(), String> {
        if let Some(t) = self.transfer.take() {
            self.conn.destroy_window(t.window).map_err(error)?;
        }
        Ok(())
    }

    fn sync_owner(&mut self) -> Result<(), String> {
        let owner = self.shared.owner();
        if self.scope.as_ref().map_or(0, |s| s.owner) == owner {
            return Ok(());
        }
        self.cancel_transfer()?;
        if let Some(s) = self.scope.take() {
            self.conn.destroy_window(s.window).map_err(error)?;
        }
        if owner != 0 {
            let window = self.window()?;
            xfixes::select_selection_input(
                &self.conn,
                window,
                self.atoms.clipboard,
                xfixes::SelectionEventMask::SET_SELECTION_OWNER,
            )
            .map_err(error)?
            .check()
            .map_err(error)?;
            self.scope = Some(Scope { owner, window });
        }
        self.conn.flush().map_err(error)
    }

    fn run(&mut self) -> Result<(), String> {
        loop {
            if self.shared.closed.load(Ordering::Acquire) {
                return Ok(());
            }
            self.sync_owner()?;
            // Limit each batch so a busy X peer cannot postpone revocation or deadlines.
            let mut pending = false;
            for _ in 0..64 {
                let Some(event) = self.conn.poll_for_event().map_err(error)? else {
                    break;
                };
                pending = true;
                self.sync_owner()?;
                self.event(event)?;
            }
            if self
                .transfer
                .as_ref()
                .is_some_and(|t| Instant::now() >= t.deadline)
            {
                self.cancel_transfer()?;
            }
            self.conn.flush().map_err(error)?;
            if pending {
                continue;
            }
            let timeout = self.transfer.as_ref().map_or(-1, |t| {
                t.deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .min(2000) as i32
            });
            let mut fds = [
                libc::pollfd {
                    fd: self.conn.stream().as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.notified.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            // Both fds outlive this poll; it only reports readiness, never accesses Rust data.
            let status =
                unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
            if status < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return Err(error(io::Error::last_os_error()));
            }
            if fds[1].revents != 0 {
                let mut buf = [0; 128];
                while self.notified.read(&mut buf).is_ok_and(|n| n > 0) {}
            }
        }
    }

    fn event(&mut self, event: Event) -> Result<(), String> {
        match event {
            Event::XfixesSelectionNotify(e)
                if self.scope.as_ref().is_some_and(|s| s.window == e.window)
                    && e.selection == self.atoms.clipboard =>
            {
                self.cancel_transfer()?;
                if e.owner == 0 {
                    return Ok(());
                }
                let owner = self.scope.as_ref().unwrap().owner;
                let window = self.window()?;
                self.conn
                    .convert_selection(
                        window,
                        self.atoms.clipboard,
                        self.atoms.utf8,
                        self.atoms.data,
                        e.timestamp,
                    )
                    .map_err(error)?;
                self.transfer = Some(Transfer {
                    owner,
                    window,
                    deadline: Instant::now() + TRANSFER_TIMEOUT,
                    incremental: false,
                    bytes: Vec::new(),
                });
            }
            Event::SelectionNotify(e)
                if self
                    .transfer
                    .as_ref()
                    .is_some_and(|t| t.window == e.requestor)
                    && e.selection == self.atoms.clipboard
                    && e.target == self.atoms.utf8 =>
            {
                if e.property != self.atoms.data {
                    return self.cancel_transfer();
                }
                self.property(false)?;
            }
            Event::PropertyNotify(e)
                if e.atom == self.atoms.data
                    && e.state == Property::NEW_VALUE
                    && self
                        .transfer
                        .as_ref()
                        .is_some_and(|t| t.window == e.window && t.incremental) =>
            {
                self.property(true)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn property(&mut self, chunk: bool) -> Result<(), String> {
        let t = self.transfer.as_ref().unwrap();
        let reply = self
            .conn
            .get_property(
                true,
                t.window,
                self.atoms.data,
                AtomEnum::ANY,
                0,
                (MAX_PASTE_BYTES / 4 + 1) as u32,
            )
            .map_err(error)?
            .reply()
            .map_err(error)?;
        if !chunk && reply.type_ == self.atoms.incr {
            let size = reply.value32().and_then(|mut n| n.next());
            if reply.format != 32
                || reply.value.len() != 4
                || reply.bytes_after != 0
                || size.is_none_or(|n| n as usize > MAX_PASTE_BYTES)
            {
                return self.cancel_transfer();
            }
            self.transfer.as_mut().unwrap().incremental = true;
            return Ok(());
        }
        if !valid_property(&reply, self.atoms.utf8) {
            return self.cancel_transfer();
        }
        let t = self.transfer.as_mut().unwrap();
        if t.bytes.len() + reply.value.len() > MAX_PASTE_BYTES {
            return self.cancel_transfer();
        }
        t.bytes.extend_from_slice(&reply.value);
        if !chunk || reply.value.is_empty() {
            let bytes = std::mem::take(&mut t.bytes);
            if let Ok(text) = String::from_utf8(bytes)
                && !text.contains('\0')
                && t.owner == self.shared.owner()
            {
                // Clipboard traffic never blocks input/video, even if a peer floods changes.
                let _ = self.events.try_send(Ev::Clipboard(t.owner, text));
            }
            self.cancel_transfer()?;
        }
        Ok(())
    }
}

fn valid_property(reply: &GetPropertyReply, utf8: Atom) -> bool {
    reply.type_ == utf8
        && reply.format == 8
        && reply.bytes_after == 0
        && reply.value.len() <= MAX_PASTE_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_watches_cannot_revive_a_revoked_owner_or_run_before_a_handoff() {
        let mut access = Access::default();
        access.authorize(1);
        access.activate(1);
        assert_eq!(access.active, 1);
        access.authorize(0);
        access.activate(1);
        assert_eq!(access.active, 0);
        access.authorize(2);
        access.activate(1);
        assert_eq!(
            access.active, 0,
            "new owner remains inactive while old input drains"
        );
        access.activate(2);
        assert_eq!(access.active, 2);
        access.authorize(0);
        access.activate(2);
        assert_eq!(access.active, 0);
    }

    #[test]
    fn rejects_truncated_wrong_type_and_oversized_selection_properties() {
        let mut p = GetPropertyReply {
            format: 8,
            sequence: 0,
            length: 0,
            type_: 42,
            bytes_after: 0,
            value_len: MAX_PASTE_BYTES as u32,
            value: vec![b'x'; MAX_PASTE_BYTES],
        };
        assert!(valid_property(&p, 42));
        assert!(!valid_property(&p, 43));
        p.value.push(b'x');
        assert!(!valid_property(&p, 42));
        p.value.clear();
        p.bytes_after = 1;
        assert!(!valid_property(&p, 42));
        p.bytes_after = 0;
        p.format = 16;
        assert!(!valid_property(&p, 42));
    }
}
