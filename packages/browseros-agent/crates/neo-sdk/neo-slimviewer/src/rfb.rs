//! An ordered worker sends paste and keys through Xvnc, without requesting pixels.
//! Non-ASCII keyboard commits use private CDP; pre-write failures fall back to keysyms.

use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod cdp;
mod worker;
pub use cdp::agent::{Agent, FieldBox, Fields, PasteError, PasteOutcome, Press};
pub use worker::Input;

use crate::engine::Ev;
use crate::wire::MAX_PASTE_BYTES;

const XK_CONTROL_L: u32 = 0xffe3;
const XK_RETURN: u32 = 0xff0d;
const XK_TAB: u32 = 0xff09;
const IO_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_DESKTOP_NAME: u32 = 64 * 1024;
const MAX_HELD_KEYS: usize = 256;
const EXTENDED_CLIPBOARD: u32 = 0xc0a1_e5ce;
const DESKTOP_SIZE: i32 = -223;
const FENCE: i32 = -312;
const FENCE_REQUEST: u32 = 1 << 31;
const FENCE_BLOCK_BEFORE: u32 = 1;
const UTF8: u32 = 1;
const CAPS: u32 = 1 << 24;
const REQUEST: u32 = 1 << 25;
const PEEK: u32 = 1 << 26;
const NOTIFY: u32 = 1 << 27;
const PROVIDE: u32 = 1 << 28;
const MAX_SERVER_CLIPBOARD: usize = 160 * 1024;
/// A clipboard replaced while the application is still reading it for the previous Ctrl+V
/// loses that paste: a fast double Paste inserted once. Xvnc can serve that read from text it
/// already fetched (for our copy-out watcher), so no request tells us when it is done; a paste
/// waits this long after the previous one's Ctrl+V before it replaces the clipboard.
const PASTE_GAP: Duration = Duration::from_millis(500);

// Also serializes writes from the input worker and the clipboard responder. One current value,
// not a history: like a local clipboard, replacement could race an application's pending paste
// (PASTE_GAP).
type Clipboard = Arc<Mutex<Option<Vec<u8>>>>;

struct Rfb {
    conn: TcpStream,
    active: Arc<AtomicBool>,
    held_keys: BTreeSet<u32>,
    pointer: (u16, u16, u8),
    wheel_remainder: [f32; 2],
    clipboard: Clipboard,
    fence_replies: Receiver<(u32, Vec<u8>)>,
    next_fence: u64,
    last_paste: Option<Instant>,
    last_press: Option<Press>,
}

impl Rfb {
    pub fn connect(addr: &str, events: SyncSender<Ev>) -> Result<Rfb, String> {
        let mut conn = handshake(addr).map_err(|e| format!("RFB connect: {e}"))?;
        negotiate_clipboard(&mut conn).map_err(|e| format!("RFB clipboard: {e}"))?;
        let mut drain = conn.try_clone().map_err(|e| format!("RFB clone: {e}"))?;
        let clipboard = Arc::new(Mutex::new(None));
        let responding = clipboard.clone();
        let active = Arc::new(AtomicBool::new(true));
        let monitoring = active.clone();
        let (fence_acks, fence_replies) = mpsc::sync_channel(8);
        std::thread::Builder::new()
            .name("slimviewer-rfb".into())
            .spawn(move || {
                let result = serve_clipboard(&mut drain, &responding, &fence_acks);
                if monitoring.load(Ordering::Acquire) {
                    let reason = match result {
                        Ok(_) => "RFB connection lost".to_string(),
                        Err(e) => format!("RFB connection lost: {e}"),
                    };
                    let _ = events.send(Ev::Fatal(reason));
                }
            })
            .map_err(|e| format!("RFB monitor: {e}"))?;
        Ok(Rfb {
            conn,
            active,
            held_keys: BTreeSet::new(),
            pointer: (0, 0, 0),
            wheel_remainder: [0.0; 2],
            clipboard,
            fence_replies,
            next_fence: 0,
            last_paste: None,
            last_press: None,
        })
    }

    pub fn pointer(&mut self, x: u16, y: u16, buttons: u8) -> io::Result<()> {
        let at = wall_ms();
        self.send(&pointer_record(x, y, buttons))?;
        if buttons & 1 != 0 && self.pointer.2 & 1 == 0 {
            self.last_press = at.map(|wall_ms| Press { wall_ms, x, y });
        }
        self.pointer = (x, y, buttons);
        Ok(())
    }

    fn recent_press(&self, within: Duration) -> Option<Press> {
        let press = self.last_press?;
        let age = wall_ms()?.checked_sub(press.wall_ms)?;
        (u128::from(age) <= within.as_millis()).then_some(press)
    }

    pub fn key(&mut self, sym: u32, down: bool) -> io::Result<()> {
        if down && self.held_keys.len() == MAX_HELD_KEYS && !self.held_keys.contains(&sym) {
            return Ok(());
        }
        self.send(&key_record(sym, down))?;
        if down {
            self.held_keys.insert(sym);
        } else {
            self.held_keys.remove(&sym);
        }
        Ok(())
    }

    fn wheel(&mut self, x: u16, y: u16, dx: f32, dy: f32) -> io::Result<()> {
        let modifiers = self.held_keys.iter().fold(0, |flags, sym| {
            flags
                | match sym {
                    0xffe9 | 0xffea => 1,
                    0xffe3 | 0xffe4 => 2,
                    0xffeb | 0xffec => 4,
                    0xffe1 | 0xffe2 => 8,
                    _ => 0,
                }
        });
        let buttons =
            (self.pointer.2 & 1) | ((self.pointer.2 & 2) << 1) | ((self.pointer.2 & 4) >> 1);
        self.wheel_with(x, y, dx, dy, || {
            cdp::desktop::wheel(x, y, dx, dy, modifiers, buttons)
        })
    }

    fn wheel_with(
        &mut self,
        x: u16,
        y: u16,
        dx: f32,
        dy: f32,
        dispatch: impl FnOnce() -> Result<(), cdp::DeliveryError>,
    ) -> io::Result<()> {
        match dispatch() {
            Ok(()) => {
                self.wheel_remainder = [0.0; 2];
                return Ok(());
            }
            Err(cdp::DeliveryError::Uncertain) => {
                eprintln!("slimviewer: event=wheel delivery=uncertain fallback=none");
                return Ok(());
            }
            Err(cdp::DeliveryError::NotSent(_)) => {}
        }
        let mut msg = Vec::new();
        for (axis, delta, negative, positive) in [(0, dx, 32, 64), (1, dy, 8, 16)] {
            self.wheel_remainder[axis] += delta;
            while self.wheel_remainder[axis].abs() >= 50.0 {
                let direction = self.wheel_remainder[axis].signum();
                let button = if direction < 0.0 { negative } else { positive };
                msg.extend(pointer_record(x, y, self.pointer.2 | button));
                msg.extend(pointer_record(x, y, self.pointer.2));
                self.wheel_remainder[axis] -= direction * 50.0;
            }
        }
        self.send(&msg)
    }

    /// The RFB connection outlives owners, so handoff must release their held input.
    pub fn release_all(&mut self) -> io::Result<()> {
        let mut msg = Vec::with_capacity(self.held_keys.len() * 8 + 6);
        for &sym in &self.held_keys {
            msg.extend(key_record(sym, false));
        }
        let (x, y, buttons) = self.pointer;
        if buttons != 0 {
            msg.extend(pointer_record(x, y, 0));
        }
        self.send(&msg)?;
        self.held_keys.clear();
        self.pointer.2 = 0;
        self.wheel_remainder = [0.0; 2];
        self.last_press = None;
        Ok(())
    }

    fn drain(&mut self) -> io::Result<()> {
        self.drain_until(Instant::now() + IO_TIMEOUT)
    }

    fn drain_soft(&mut self) -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_millis(300);
        let marker = self.send_fence()?;
        match self.wait_fence(marker, deadline) {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::Unsupported
                ) =>
            {
                eprintln!("slimviewer: event=paste fence=failed reason={error}");
                Ok(())
            }
            result => result,
        }
    }

    fn drain_until(&mut self, deadline: Instant) -> io::Result<()> {
        let marker = self.send_fence()?;
        self.wait_fence(marker, deadline)
    }

    fn send_fence(&mut self) -> io::Result<[u8; 8]> {
        while self.fence_replies.try_recv().is_ok() {}
        self.next_fence = self.next_fence.wrapping_add(1);
        let marker = self.next_fence.to_be_bytes();
        self.send(&fence_record(FENCE_REQUEST | FENCE_BLOCK_BEFORE, &marker))?;
        Ok(marker)
    }

    fn wait_fence(&self, marker: [u8; 8], deadline: Instant) -> io::Result<()> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let (flags, payload) = self
                .fence_replies
                .recv_timeout(remaining)
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => {
                        io::Error::new(io::ErrorKind::TimedOut, "RFB input fence timeout")
                    }
                    mpsc::RecvTimeoutError::Disconnected => {
                        io::Error::new(io::ErrorKind::BrokenPipe, "RFB input fence disconnected")
                    }
                })?;
            if payload == marker {
                return if flags & FENCE_BLOCK_BEFORE != 0 {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "Xvnc cannot drain input",
                    ))
                };
            }
        }
    }

    /// Announce UTF-8, then Ctrl+V. The reader supplies text when Xvnc requests it, including
    /// after this call returns. Success means sent, not that the focused application accepted it.
    /// A paste right after another first waits out PASTE_GAP from that one's Ctrl+V.
    pub fn paste(&mut self, text: &str) -> io::Result<()> {
        validate_text(text)?;
        if text.is_empty() {
            return Ok(());
        }
        if let Some(at) = self.last_paste {
            std::thread::sleep((at + PASTE_GAP).saturating_duration_since(Instant::now()));
        }
        let provide = clipboard_provide(text);
        let mut msg = clipboard_record(NOTIFY | UTF8, &[]);
        let ctrl_held = self.held_keys.contains(&XK_CONTROL_L);
        if !ctrl_held {
            msg.extend(key_record(XK_CONTROL_L, true));
        }
        msg.extend(key_record(u32::from('v'), true));
        if !self.held_keys.contains(&u32::from('v')) {
            msg.extend(key_record(u32::from('v'), false));
        }
        if !ctrl_held {
            msg.extend(key_record(XK_CONTROL_L, false));
        }
        let msg = self.without_modifiers(msg, true);
        let mut clipboard = self.clipboard.lock().map_err(lock_error)?;
        *clipboard = Some(provide);
        self.conn.write_all(&msg)?;
        self.last_paste = Some(Instant::now());
        Ok(())
    }

    fn type_text(&mut self, text: &str) -> io::Result<()> {
        self.type_with(text, cdp::insert_text)
    }

    fn type_with(
        &mut self,
        text: &str,
        insert: impl FnOnce(&str) -> Result<(), cdp::DeliveryError>,
    ) -> io::Result<()> {
        validate_text(text)?;
        if text.is_empty() {
            return Ok(());
        }
        if !text.is_ascii() {
            match insert(text) {
                Ok(()) => return Ok(()),
                Err(cdp::DeliveryError::Uncertain) => {
                    eprintln!("slimviewer: event=type delivery=uncertain fallback=none");
                    return Err(io::Error::other("CDP input delivery uncertain"));
                }
                Err(cdp::DeliveryError::NotSent(reason)) => {
                    eprintln!("slimviewer: event=type fallback=keysym reason={reason}");
                }
            }
        }
        let mut msg = Vec::with_capacity(text.len() * 16);
        for c in text.chars() {
            let sym = keysym(c);
            msg.extend(key_record(sym, true));
            if !self.held_keys.contains(&sym) {
                msg.extend(key_record(sym, false));
            }
        }
        self.send(&self.without_modifiers(msg, false))
    }

    fn without_modifiers(&self, body: Vec<u8>, keep_control: bool) -> Vec<u8> {
        let modifiers: Vec<_> = self
            .held_keys
            .iter()
            .copied()
            .filter(|sym| {
                matches!(sym, 0xffe1..=0xffe4 | 0xffe7..=0xffee)
                    && !(keep_control && matches!(sym, 0xffe3 | 0xffe4))
            })
            .collect();
        let mut msg = Vec::with_capacity(body.len() + modifiers.len() * 16);
        for &sym in &modifiers {
            msg.extend(key_record(sym, false));
        }
        msg.extend(body);
        for sym in modifiers {
            msg.extend(key_record(sym, true));
        }
        msg
    }

    fn send(&mut self, msg: &[u8]) -> io::Result<()> {
        let _clipboard = self.clipboard.lock().map_err(lock_error)?;
        self.conn.write_all(msg)
    }
}

fn wall_ms() -> Option<u64> {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_millis(),
    )
    .ok()
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> io::Error {
    io::Error::other("RFB clipboard lock poisoned")
}

fn validate_text(text: &str) -> io::Result<()> {
    if text.len() > MAX_PASTE_BYTES || text.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "text exceeds 64 KiB or contains NUL",
        ));
    }
    Ok(())
}

fn clipboard_record(flags: u32, data: &[u8]) -> Vec<u8> {
    let mut msg = vec![6, 0, 0, 0];
    msg.extend((-(4 + data.len() as i32)).to_be_bytes());
    msg.extend(flags.to_be_bytes());
    msg.extend(data);
    msg
}

fn clipboard_provide(text: &str) -> Vec<u8> {
    let text = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n");
    let mut data = ((text.len() + 1) as u32).to_be_bytes().to_vec();
    data.extend(text.as_bytes());
    data.push(0);
    clipboard_record(PROVIDE | UTF8, &zlib_stored(&data))
}

/// RFC 1950/1951 with stored DEFLATE blocks: no compression dependency or incoming inflater.
/// Text is bounded at admission; the small header overhead buys a single Unicode paste path.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut chunks = data.chunks(65535).peekable();
    while let Some(chunk) = chunks.next() {
        out.push(u8::from(chunks.peek().is_none()));
        let len = chunk.len() as u16;
        out.extend(len.to_le_bytes());
        out.extend((!len).to_le_bytes());
        out.extend(chunk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    out.extend(((b << 16) | a).to_be_bytes());
    out
}

fn negotiate_clipboard(conn: &mut TcpStream) -> io::Result<()> {
    // Xvnc disconnects clients without resize support even when they request no pixels.
    let mut encodings = vec![2, 0, 0, 3];
    encodings.extend(EXTENDED_CLIPBOARD.to_be_bytes());
    encodings.extend(DESKTOP_SIZE.to_be_bytes());
    encodings.extend(FENCE.to_be_bytes());
    conn.write_all(&encodings)?;
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut clipboard_ready = false;
    let mut fence_ready = false;
    for _ in 0..16 {
        match read_server_record(&mut DeadlineReader { conn, deadline })? {
            ServerRecord::Fence(flags, payload) => {
                fence_ready = true;
                if flags & FENCE_REQUEST != 0 {
                    conn.write_all(&fence_record(flags & 3, &payload))?;
                }
            }
            ServerRecord::Clipboard(flags) => {
                if flags & CAPS == 0 {
                    continue;
                }
                let required = CAPS | UTF8 | NOTIFY | PROVIDE;
                if flags & required != required {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "Xvnc lacks UTF-8 clipboard support",
                    ));
                }
                // Accept requests/peeks/announcements, never request remote clipboard data.
                conn.write_all(&clipboard_record(
                    CAPS | UTF8 | REQUEST | PEEK | NOTIFY,
                    &0u32.to_be_bytes(),
                ))?;
                clipboard_ready = true;
            }
            ServerRecord::Other => {}
        }
        if clipboard_ready && fence_ready {
            conn.set_read_timeout(None)?;
            return Ok(());
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "missing clipboard or input fence capabilities",
    ))
}

fn fence_record(flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut record = vec![248, 0, 0, 0];
    record.extend(flags.to_be_bytes());
    record.push(payload.len() as u8);
    record.extend(payload);
    record
}

enum ServerRecord {
    Clipboard(u32),
    Fence(u32, Vec<u8>),
    Other,
}

fn read_server_record(from: &mut impl Read) -> io::Result<ServerRecord> {
    let mut kind = [0];
    from.read_exact(&mut kind)?;
    if kind[0] != 248 {
        return Ok(
            match read_clipboard(&mut io::Cursor::new(kind).chain(from))? {
                Some(flags) => ServerRecord::Clipboard(flags),
                None => ServerRecord::Other,
            },
        );
    }
    let mut header = [0; 8];
    from.read_exact(&mut header)?;
    let flags = u32::from_be_bytes(header[3..7].try_into().unwrap());
    let len = usize::from(header[7]);
    if len > 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "oversized RFB fence",
        ));
    }
    let mut payload = vec![0; len];
    from.read_exact(&mut payload)?;
    Ok(ServerRecord::Fence(flags, payload))
}

/// Only metadata is used. Unsolicited clipboard contents are bounded and discarded, never
/// decompressed or forwarded to the phone. No framebuffer update is ever requested.
fn read_clipboard(from: &mut impl Read) -> io::Result<Option<u32>> {
    let mut kind = [0];
    from.read_exact(&mut kind)?;
    if kind[0] == 2 {
        return Ok(None);
    } // Bell
    if kind[0] != 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected RFB message",
        ));
    }
    let mut header = [0; 7];
    from.read_exact(&mut header)?;
    let len = i32::from_be_bytes(header[3..].try_into().unwrap());
    let size = len.unsigned_abs() as usize;
    if size > MAX_SERVER_CLIPBOARD || (len < 0 && size < 4) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid clipboard length",
        ));
    }
    let mut body = vec![0; size];
    from.read_exact(&mut body)?;
    if len >= 0 {
        return Ok(None);
    }
    let flags = u32::from_be_bytes(body[..4].try_into().unwrap());
    let action = flags & 0xff00_0000;
    if (flags & CAPS != 0 && size != 4 + 4 * (flags & 0xffff).count_ones() as usize)
        || (flags & CAPS == 0 && action.count_ones() != 1)
        || (matches!(action, REQUEST | PEEK | NOTIFY) && size != 4)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid clipboard action length",
        ));
    }
    Ok(Some(flags))
}

fn serve_clipboard(
    conn: &mut TcpStream,
    clipboard: &Clipboard,
    fence_acks: &SyncSender<(u32, Vec<u8>)>,
) -> io::Result<()> {
    loop {
        // No idle timer. Once a message begins, trickling bytes cannot extend its deadline.
        conn.set_read_timeout(None)?;
        let mut first = [0];
        conn.read_exact(&mut first)?;
        let reader = DeadlineReader {
            conn,
            deadline: Instant::now() + IO_TIMEOUT,
        };
        let flags = match read_server_record(&mut io::Cursor::new(first).chain(reader))? {
            ServerRecord::Fence(flags, payload) => {
                if flags & FENCE_REQUEST != 0 {
                    let _clipboard = clipboard.lock().map_err(lock_error)?;
                    conn.write_all(&fence_record(flags & 3, &payload))?;
                } else {
                    let _ = fence_acks.try_send((flags, payload));
                }
                continue;
            }
            ServerRecord::Clipboard(flags) => flags,
            ServerRecord::Other => continue,
        };
        match flags & 0xff00_0000 {
            REQUEST if flags & UTF8 != 0 => {
                let data = clipboard.lock().map_err(lock_error)?;
                if let Some(data) = data.as_ref() {
                    conn.write_all(data)?;
                }
            }
            PEEK => {
                let data = clipboard.lock().map_err(lock_error)?;
                conn.write_all(&clipboard_record(
                    NOTIFY | if data.is_some() { UTF8 } else { 0 },
                    &[],
                ))?;
            }
            _ => {}
        }
    }
}

struct DeadlineReader<'a> {
    conn: &'a mut TcpStream,
    deadline: Instant,
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "RFB clipboard message timeout")
            })?;
        self.conn.set_read_timeout(Some(remaining))?;
        self.conn.read(buf)
    }
}

impl Drop for Rfb {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        let _ = self.conn.shutdown(Shutdown::Both);
    }
}

fn key_record(sym: u32, down: bool) -> [u8; 8] {
    let mut msg = [4, u8::from(down), 0, 0, 0, 0, 0, 0];
    msg[4..].copy_from_slice(&sym.to_be_bytes());
    msg
}

fn pointer_record(x: u16, y: u16, buttons: u8) -> [u8; 6] {
    let [xh, xl] = x.to_be_bytes();
    let [yh, yl] = y.to_be_bytes();
    [5, buttons, xh, xl, yh, yl]
}

pub fn keysym(c: char) -> u32 {
    match c {
        '\n' | '\r' => XK_RETURN,
        '\t' => XK_TAB,
        c if (' '..='~').contains(&c) || ('\u{a0}'..='\u{ff}').contains(&c) => u32::from(c),
        c => 0x0100_0000 + u32::from(c),
    }
}

/// RFB 3.8, security None, shared session. No pixels are requested.
fn handshake(addr: &str) -> io::Result<TcpStream> {
    let mut error = io::Error::new(io::ErrorKind::InvalidInput, "RFB address has no endpoints");
    let mut conn = None;
    for addr in addr.to_socket_addrs()?.take(8) {
        match TcpStream::connect_timeout(&addr, IO_TIMEOUT) {
            Ok(c) => {
                conn = Some(c);
                break;
            }
            Err(e) => error = e,
        }
    }
    let mut c = conn.ok_or(error)?;
    c.set_nodelay(true)?;
    c.set_read_timeout(Some(IO_TIMEOUT))?;
    c.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut version = [0u8; 12];
    c.read_exact(&mut version)?;
    if version != *b"RFB 003.008\n" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Xvnc must offer RFB 3.8",
        ));
    }
    c.write_all(b"RFB 003.008\n")?;
    let mut n = [0u8; 1];
    c.read_exact(&mut n)?;
    let mut types = vec![0u8; usize::from(n[0])];
    c.read_exact(&mut types)?;
    if !types.contains(&1) {
        return Err(io::Error::other("Xvnc offers no None security"));
    }
    c.write_all(&[1])?;
    let mut result = [0u8; 4];
    c.read_exact(&mut result)?;
    if result != [0; 4] {
        return Err(io::Error::other("RFB security refused"));
    }
    c.write_all(&[1])?; // A shared connection keeps the host's other RFB clients connected.
    let mut init = [0u8; 24];
    c.read_exact(&mut init)?;
    let name_len = u32::from_be_bytes([init[20], init[21], init[22], init[23]]);
    if name_len > MAX_DESKTOP_NAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "RFB desktop name exceeds 64 KiB",
        ));
    }
    let mut name = vec![0; name_len as usize];
    c.read_exact(&mut name)?;
    c.set_read_timeout(None)?;
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::{Rfb as Input, keysym};
    use crate::engine::Ev;
    use std::io::{self, Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::mpsc::{self, Receiver};
    use std::time::Duration;

    pub(super) fn input_pair() -> (Input, TcpStream, Receiver<Ev>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (connected, server) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream.write_all(b"RFB 003.008\n").unwrap();
            let mut reply = [0; 12];
            stream.read_exact(&mut reply).unwrap();
            assert_eq!(&reply, b"RFB 003.008\n");
            stream.write_all(&[1, 1]).unwrap();
            stream.read_exact(&mut reply[..1]).unwrap();
            assert_eq!(reply[0], 1);
            stream.write_all(&[0; 4]).unwrap();
            stream.read_exact(&mut reply[..1]).unwrap();
            assert_eq!(reply[0], 1); // shared: other RFB clients stay connected
            stream.write_all(&[0; 24]).unwrap();
            let mut encodings = [0; 16];
            stream.read_exact(&mut encodings).unwrap();
            assert_eq!(
                encodings,
                [
                    2, 0, 0, 3, 0xc0, 0xa1, 0xe5, 0xce, 0xff, 0xff, 0xff, 0x21, 0xff, 0xff, 0xfe,
                    0xc8
                ]
            );
            stream
                .write_all(&[
                    3, 0, 0, 0, 0xff, 0xff, 0xff, 0xf8, 0x1f, 0, 0, 1, 0, 0, 0, 0,
                ])
                .unwrap();
            let mut caps = [0; 16];
            stream.read_exact(&mut caps).unwrap();
            assert_eq!(
                caps,
                [
                    6, 0, 0, 0, 0xff, 0xff, 0xff, 0xf8, 0x0f, 0, 0, 1, 0, 0, 0, 0
                ]
            );
            stream
                .write_all(&super::fence_record(super::FENCE_REQUEST, &[]))
                .unwrap();
            let mut fence = [0; 9];
            stream.read_exact(&mut fence).unwrap();
            assert_eq!(fence, [248, 0, 0, 0, 0, 0, 0, 0, 0]);
            connected.send(stream).unwrap();
        });
        let (events, rx) = mpsc::sync_channel(4);
        let input = Input::connect(&addr, events).unwrap();
        (
            input,
            server.recv_timeout(Duration::from_secs(3)).unwrap(),
            rx,
        )
    }

    #[test]
    fn tests_that_eagerly_connects_and_preserves_input_order() {
        let (mut input, mut server, _) = input_pair();
        input.pointer(258, 515, 1).unwrap();
        input.key(0x1234, true).unwrap();
        let mut bytes = [0; 14];
        server.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, [5, 1, 1, 2, 2, 3, 4, 1, 0, 0, 0, 0, 0x12, 0x34]);
    }

    #[test]
    fn tests_that_recent_press_tracks_only_button_one_down_and_clears_on_release() {
        let (mut input, _server, _) = input_pair();
        let window = Duration::from_secs(1);
        assert!(input.recent_press(window).is_none());
        input.pointer(10, 20, 2).unwrap();
        assert!(input.recent_press(window).is_none());
        input.pointer(30, 40, 3).unwrap();
        let first = input.recent_press(window).unwrap();
        assert_eq!((first.x, first.y), (30, 40));
        input.pointer(50, 60, 1).unwrap();
        input.pointer(50, 60, 0).unwrap();
        assert_eq!(input.recent_press(window), Some(first));
        input.last_press.as_mut().unwrap().wall_ms -= 2000;
        assert!(input.recent_press(window).is_none());
        input.pointer(70, 80, 1).unwrap();
        assert_eq!(
            input.recent_press(window).map(|p| (p.x, p.y)),
            Some((70, 80))
        );
        input.release_all().unwrap();
        assert!(input.recent_press(window).is_none());
    }

    #[test]
    fn tests_that_soft_drain_times_out_without_losing_the_input_connection() {
        let (mut input, mut server, _) = input_pair();
        let (finished, done) = mpsc::channel();
        std::thread::spawn(move || {
            input.drain_soft().unwrap();
            input.key(97, true).unwrap();
            finished.send(input).unwrap();
        });
        let mut fence = [0; 17];
        server.read_exact(&mut fence).unwrap();
        assert!(done.recv_timeout(Duration::from_millis(50)).is_err());
        let input = done.recv_timeout(Duration::from_millis(800)).unwrap();
        let mut key = [0; 8];
        server.read_exact(&mut key).unwrap();
        assert_eq!(key, super::key_record(97, true));
        assert_eq!(input.conn.write_timeout().unwrap(), Some(super::IO_TIMEOUT));
    }

    #[test]
    fn tests_that_soft_drain_preserves_the_shared_socket_write_timeout_while_waiting() {
        let (mut input, mut server, _) = input_pair();
        let observer = input.conn.try_clone().unwrap();
        let timeout = Duration::from_secs(2);
        observer.set_write_timeout(Some(timeout)).unwrap();
        let (finished, done) = mpsc::channel();
        std::thread::spawn(move || {
            input.drain_soft().unwrap();
            finished.send(input).unwrap();
        });
        let mut fence = [0; 17];
        server.read_exact(&mut fence).unwrap();
        assert_eq!(observer.write_timeout().unwrap(), Some(timeout));
        server
            .write_all(&super::fence_record(super::FENCE_BLOCK_BEFORE, &fence[9..]))
            .unwrap();
        let input = done.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(input.conn.write_timeout().unwrap(), Some(timeout));
    }

    #[test]
    fn tests_that_late_and_current_fence_replies_can_arrive_together() {
        let (mut input, mut server, _) = input_pair();
        for _ in 0..16 {
            let (finished, done) = mpsc::channel();
            std::thread::spawn(move || {
                let result =
                    input.drain_until(std::time::Instant::now() + Duration::from_millis(500));
                finished.send((input, result)).unwrap();
            });
            let mut fence = [0; 17];
            server.read_exact(&mut fence).unwrap();
            let mut replies = super::fence_record(super::FENCE_BLOCK_BEFORE, &[0; 8]);
            replies.extend(super::fence_record(super::FENCE_BLOCK_BEFORE, &fence[9..]));
            server.write_all(&replies).unwrap();
            let (returned, result) = done.recv_timeout(Duration::from_secs(1)).unwrap();
            result.unwrap();
            input = returned;
        }
    }

    #[test]
    fn tests_that_soft_drain_propagates_write_failures() {
        let (mut input, _server, _) = input_pair();
        input.conn.shutdown(Shutdown::Write).unwrap();
        assert!(input.drain_soft().is_err());
    }

    #[test]
    fn tests_that_soft_drain_propagates_a_disconnected_reply_channel() {
        let (mut input, _server, _) = input_pair();
        let (sender, replies) = mpsc::sync_channel(8);
        input.fence_replies = replies;
        drop(sender);
        assert_eq!(
            input.drain_soft().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn tests_that_soft_drain_tolerates_an_unsupported_reply() {
        let (mut input, mut server, _) = input_pair();
        let (finished, done) = mpsc::channel();
        std::thread::spawn(move || finished.send(input.drain_soft()).unwrap());
        let mut fence = [0; 17];
        server.read_exact(&mut fence).unwrap();
        server
            .write_all(&super::fence_record(0, &fence[9..]))
            .unwrap();
        done.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
    }

    #[test]
    fn tests_that_input_drain_waits_for_its_matching_processed_fence() {
        let (mut input, mut server, _) = input_pair();
        input.key(0xffe1, true).unwrap();
        input.release_all().unwrap();
        let (finished, done) = mpsc::channel();
        std::thread::spawn(move || finished.send(input.drain()).unwrap());
        let mut keys = [0; 16];
        server.read_exact(&mut keys).unwrap();
        assert_eq!(keys[..8], super::key_record(0xffe1, true));
        assert_eq!(keys[8..], super::key_record(0xffe1, false));
        let mut fence = [0; 17];
        server.read_exact(&mut fence).unwrap();
        assert_eq!(&fence[..9], &[248, 0, 0, 0, 128, 0, 0, 1, 8]);
        assert!(done.try_recv().is_err());
        server
            .write_all(&super::fence_record(super::FENCE_BLOCK_BEFORE, &[0; 8]))
            .unwrap();
        assert!(done.recv_timeout(Duration::from_millis(50)).is_err());
        server
            .write_all(&super::fence_record(super::FENCE_BLOCK_BEFORE, &fence[9..]))
            .unwrap();
        done.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
    }

    #[test]
    fn tests_that_input_drain_rejects_a_fence_without_processing_guarantees() {
        let (mut input, mut server, _) = input_pair();
        let (finished, done) = mpsc::channel();
        std::thread::spawn(move || finished.send(input.drain()).unwrap());
        let mut fence = [0; 17];
        server.read_exact(&mut fence).unwrap();
        server
            .write_all(&super::fence_record(0, &fence[9..]))
            .unwrap();
        assert_eq!(
            done.recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap_err()
                .kind(),
            io::ErrorKind::Unsupported
        );
    }

    #[test]
    fn tests_that_server_fence_requests_get_bounded_responses() {
        let (_input, mut server, _) = input_pair();
        server
            .write_all(&super::fence_record(super::FENCE_REQUEST | 7, b"roundtrip"))
            .unwrap();
        let mut reply = [0; 18];
        server.read_exact(&mut reply).unwrap();
        assert_eq!(reply, super::fence_record(3, b"roundtrip")[..]);
        let mut oversized = super::fence_record(0, &[]);
        oversized[8] = 65;
        assert!(super::read_server_record(&mut &oversized[..]).is_err());
    }

    #[test]
    fn wheel_fallback_accumulates_both_axes_and_keeps_drag_buttons() {
        use super::{cdp::DeliveryError, pointer_record};
        let (mut input, mut server, _) = input_pair();
        input.pointer(20, 30, 1).unwrap();
        let mut held = [0; 6];
        server.read_exact(&mut held).unwrap();
        input
            .wheel_with(20, 30, 20.0, -20.0, || {
                Err(DeliveryError::NotSent("outside content"))
            })
            .unwrap();
        input
            .wheel_with(20, 30, 30.0, -30.0, || {
                Err(DeliveryError::NotSent("outside content"))
            })
            .unwrap();
        let mut bytes = [0; 24];
        server.read_exact(&mut bytes).unwrap();
        let expected: Vec<_> = [65, 1, 9, 1]
            .into_iter()
            .flat_map(|buttons| pointer_record(20, 30, buttons))
            .collect();
        assert_eq!(bytes.as_slice(), expected);
    }

    #[test]
    fn wheel_never_replays_a_successful_or_uncertain_cdp_mutation() {
        use super::cdp::DeliveryError;
        let (mut input, mut server, _) = input_pair();
        input.wheel_with(20, 30, 200.0, -200.0, || Ok(())).unwrap();
        input
            .wheel_with(20, 30, 200.0, -200.0, || Err(DeliveryError::Uncertain))
            .unwrap();
        input.key(97, true).unwrap();
        let mut bytes = [0; 8];
        server.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, super::key_record(97, true));
    }

    #[test]
    fn tests_that_reports_an_idle_rfb_disconnect() {
        let (_input, server, rx) = input_pair();
        server.shutdown(Shutdown::Both).unwrap();
        assert!(
            matches!(rx.recv_timeout(Duration::from_secs(3)), Ok(Ev::Fatal(reason)) if reason.contains("RFB connection lost"))
        );
    }

    #[test]
    fn tests_that_returns_input_write_failures() {
        let (mut input, _server, _) = input_pair();
        input.conn.shutdown(Shutdown::Both).unwrap();
        assert!(input.pointer(0, 0, 0).is_err());
        assert!(input.key(0x61, false).is_err());
        assert!(input.paste("hello").is_err());
    }

    #[test]
    fn tests_that_bounds_direct_paste_calls() {
        let (mut input, _server, _) = input_pair();
        assert_eq!(
            input
                .paste(&"x".repeat(super::MAX_PASTE_BYTES + 1))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn paste_announces_before_ctrl_v_and_only_provides_when_requested() {
        let (mut input, mut server, _) = input_pair();
        input.paste("café👋").unwrap();
        let mut bytes = [0; 44];
        server.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes[..12], &[6, 0, 0, 0, 255, 255, 255, 252, 8, 0, 0, 1]);
        assert_eq!(
            &bytes[12..],
            &[
                4, 1, 0, 0, 0, 0, 255, 227, 4, 1, 0, 0, 0, 0, 0, b'v', 4, 0, 0, 0, 0, 0, 0, b'v',
                4, 0, 0, 0, 0, 0, 255, 227,
            ]
        );
        // RFC request, deliberately fragmented across TCP writes.
        let request = [3, 0, 0, 0, 255, 255, 255, 252, 2, 0, 0, 1];
        for byte in request {
            server.write_all(&[byte]).unwrap();
        }
        let mut header = [0; 12];
        server.read_exact(&mut header).unwrap();
        assert_eq!(header[0], 6);
        assert_eq!(&header[8..], &[16, 0, 0, 1]);
        let size = -i32::from_be_bytes(header[4..8].try_into().unwrap()) as usize - 4;
        let mut body = vec![0; size];
        server.read_exact(&mut body).unwrap();
        // Independently generated with Python zlib.compress(data, level=0).
        assert_eq!(
            body,
            [
                120, 1, 1, 14, 0, 241, 255, 0, 0, 0, 10, 99, 97, 102, 195, 169, 240, 159, 145, 139,
                0, 30, 51, 5, 76
            ]
        );
        server.write_all(&request).unwrap();
        let mut repeated = vec![0; 12 + size];
        server.read_exact(&mut repeated).unwrap();
        assert_eq!(&repeated[..12], &header);
        assert_eq!(&repeated[12..], &body);
    }

    #[test]
    fn tests_that_ascii_never_contacts_cdp_and_prewrite_failure_uses_keysyms() {
        let (mut input, mut server, _) = input_pair();
        input
            .type_with("a", |_| panic!("ASCII contacted CDP"))
            .unwrap();
        input
            .type_with("é日", |_| {
                Err(super::cdp::DeliveryError::NotSent("no focused page"))
            })
            .unwrap();
        let expected: Vec<_> = ['a', 'é', '日']
            .into_iter()
            .flat_map(|c| {
                [
                    super::key_record(keysym(c), true),
                    super::key_record(keysym(c), false),
                ]
                .concat()
            })
            .collect();
        let mut actual = vec![0; expected.len()];
        server.read_exact(&mut actual).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn tests_that_cdp_success_and_uncertain_writes_never_fall_back() {
        let (mut input, mut server, _) = input_pair();
        input
            .type_with("नमस्ते", |text| {
                assert_eq!(text, "नमस्ते");
                Ok(())
            })
            .unwrap();
        assert!(
            input
                .type_with("👋🏽", |_| Err(super::cdp::DeliveryError::Uncertain))
                .is_err(),
            "an unconfirmed browser insertion cannot permit successful handback"
        );
        input.key(0xff08, true).unwrap();
        let mut actual = [0; 8];
        server.read_exact(&mut actual).unwrap();
        assert_eq!(actual, super::key_record(0xff08, true));
    }

    #[test]
    fn tests_that_a_quick_second_paste_waits_before_replacing_the_clipboard() {
        let (mut input, mut server, _) = input_pair();
        let start = std::time::Instant::now();
        input.paste("twice").unwrap();
        input.paste("twice").unwrap();
        assert!(start.elapsed() >= super::PASTE_GAP);
        // Both still reach Xvnc: an announcement and Ctrl+V each.
        let mut bytes = [0; 2 * 44];
        server.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes[..12], &[6, 0, 0, 0, 255, 255, 255, 252, 8, 0, 0, 1]);
        assert_eq!(bytes[..44], bytes[44..]);
    }

    #[test]
    fn tests_that_paste_preserves_a_held_control_key() {
        let (mut input, mut server, _) = input_pair();
        input.key(super::XK_CONTROL_L, true).unwrap();
        input.paste("x").unwrap();
        input.key(super::XK_CONTROL_L, false).unwrap();
        let mut expected = super::key_record(super::XK_CONTROL_L, true).to_vec();
        expected.extend([6, 0, 0, 0, 255, 255, 255, 252, 8, 0, 0, 1]);
        expected.extend(super::key_record(u32::from('v'), true));
        expected.extend(super::key_record(u32::from('v'), false));
        expected.extend(super::key_record(super::XK_CONTROL_L, false));
        let mut actual = vec![0; expected.len()];
        server.read_exact(&mut actual).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn tests_that_key_spam_is_bounded_without_rejecting_releases() {
        let (mut input, mut server, _) = input_pair();
        for sym in 1..=256 {
            input.key(sym, true).unwrap();
        }
        input.key(257, true).unwrap();
        input.key(1, false).unwrap();
        let mut actual = vec![0; 257 * 8];
        server.read_exact(&mut actual).unwrap();
        assert_eq!(&actual[256 * 8..], &super::key_record(1, false));
        input.key(257, true).unwrap();
        let mut resumed = [0; 8];
        server.read_exact(&mut resumed).unwrap();
        assert_eq!(resumed, super::key_record(257, true));
    }

    #[test]
    fn tests_that_owner_reset_releases_shift_once_before_new_input() {
        let (mut input, mut server, _) = input_pair();
        input.key(0xffe1, true).unwrap();
        input.release_all().unwrap();
        input.release_all().unwrap();
        input.key(u32::from('a'), true).unwrap();
        let expected = [
            super::key_record(0xffe1, true),
            super::key_record(0xffe1, false),
            super::key_record(u32::from('a'), true),
        ]
        .concat();
        let mut actual = vec![0; expected.len()];
        server.read_exact(&mut actual).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn tests_that_owner_reset_clears_buttons_at_the_latest_position() {
        let (mut input, mut server, _) = input_pair();
        input.pointer(1, 2, 1).unwrap();
        input.pointer(4, 5, 3).unwrap();
        input.release_all().unwrap();
        input.release_all().unwrap();
        input.pointer(8, 9, 0).unwrap();
        let mut actual = [0; 24];
        server.read_exact(&mut actual).unwrap();
        assert_eq!(
            actual,
            [
                5, 1, 0, 1, 0, 2, 5, 3, 0, 4, 0, 5, 5, 0, 0, 4, 0, 5, 5, 0, 0, 8, 0, 9,
            ]
        );
    }

    #[test]
    fn tests_that_key_up_removes_state_before_owner_reset() {
        let (mut input, mut server, _) = input_pair();
        input.key(0xffe1, true).unwrap();
        input.key(0xffe1, false).unwrap();
        input.release_all().unwrap();
        input.key(u32::from('a'), true).unwrap();
        let expected = [
            super::key_record(0xffe1, true),
            super::key_record(0xffe1, false),
            super::key_record(u32::from('a'), true),
        ]
        .concat();
        let mut actual = vec![0; expected.len()];
        server.read_exact(&mut actual).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn tests_that_key_repeats_do_not_fill_the_pressed_key_limit() {
        let (mut input, mut server, _) = input_pair();
        for _ in 0..256 {
            input.key(0xffe1, true).unwrap();
        }
        input.key(0xffe9, true).unwrap();
        input.release_all().unwrap();
        let mut actual = vec![0; 259 * 8];
        server.read_exact(&mut actual).unwrap();
        assert_eq!(
            &actual[257 * 8..],
            &[
                super::key_record(0xffe1, false),
                super::key_record(0xffe9, false),
            ]
            .concat()
        );
    }

    #[test]
    fn committed_text_keeps_held_unicode_keys_until_owner_reset() {
        let (mut input, mut server, _) = input_pair();
        let euro = super::keysym('€');
        input.key(euro, true).unwrap();
        input.type_text("a").unwrap();
        input.release_all().unwrap();
        input.key(u32::from('b'), true).unwrap();
        let expected = [
            super::key_record(euro, true),
            super::key_record(u32::from('a'), true),
            super::key_record(u32::from('a'), false),
            super::key_record(euro, false),
            super::key_record(u32::from('b'), true),
        ]
        .concat();
        let mut actual = vec![0; expected.len()];
        server.read_exact(&mut actual).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn tests_that_paste_keeps_held_ctrl_v_until_owner_reset() {
        let (mut input, mut server, _) = input_pair();
        input.key(super::XK_CONTROL_L, true).unwrap();
        input.key(u32::from('v'), true).unwrap();
        input.paste("x").unwrap();
        input.release_all().unwrap();
        let mut expected = [
            super::key_record(super::XK_CONTROL_L, true),
            super::key_record(u32::from('v'), true),
        ]
        .concat();
        expected.extend([6, 0, 0, 0, 255, 255, 255, 252, 8, 0, 0, 1]);
        expected.extend(super::key_record(u32::from('v'), true));
        expected.extend(super::key_record(u32::from('v'), false));
        expected.extend(super::key_record(super::XK_CONTROL_L, false));
        let mut actual = vec![0; expected.len()];
        server.read_exact(&mut actual).unwrap();
        assert_eq!(actual, expected);
    }

    fn mock_server(version: &'static [u8], name_len: u32, name: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || -> io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.write_all(version)?;
            let mut reply = [0; 12];
            stream.read_exact(&mut reply)?;
            stream.write_all(&[1, 1])?;
            stream.read_exact(&mut reply[..1])?;
            stream.write_all(&[0; 4])?;
            stream.read_exact(&mut reply[..1])?;
            let mut init = [0; 24];
            init[20..24].copy_from_slice(&name_len.to_be_bytes());
            stream.write_all(&init)?;
            stream.write_all(name)?;
            Ok(())
        });
        addr
    }

    #[test]
    fn tests_that_rejects_invalid_rfb_version() {
        let addr = mock_server(b"NOT 003.008\n", 0, b"");
        assert_eq!(
            super::handshake(&addr).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn tests_that_rejects_truncated_desktop_name() {
        let addr = mock_server(b"RFB 003.008\n", 5, b"four");
        assert_eq!(
            super::handshake(&addr).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn tests_that_bounds_desktop_name() {
        let addr = mock_server(b"RFB 003.008\n", u32::MAX, b"");
        assert_eq!(
            super::handshake(&addr).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn keysyms_follow_x11_rules() {
        assert_eq!(keysym('a'), 0x61);
        assert_eq!(keysym('é'), 0xe9);
        assert_eq!(keysym('\n'), 0xff0d);
        assert_eq!(keysym('€'), 0x0100_20ac);
    }

    #[test]
    fn malformed_clipboard_envelopes_are_rejected_before_body_allocation() {
        for size in [i32::MIN, -3, -(super::MAX_SERVER_CLIPBOARD as i32 + 1)] {
            let mut bytes = vec![3, 0, 0, 0];
            bytes.extend(size.to_be_bytes());
            assert_eq!(
                super::read_clipboard(&mut &bytes[..]).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        // Caps with UTF-8 set requires a size entry, unlike a request.
        let bytes = [3, 0, 0, 0, 255, 255, 255, 252, 1, 0, 0, 1];
        assert_eq!(
            super::read_clipboard(&mut &bytes[..]).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn empty_text_is_a_noop_and_nul_is_rejected() {
        let (mut input, mut server, _) = input_pair();
        input.paste("").unwrap();
        input.type_text("").unwrap();
        assert!(input.paste("a\0b").is_err());
        assert!(input.type_text("a\0b").is_err());
        input.pointer(1, 2, 0).unwrap();
        let mut bytes = [0; 6];
        server.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, [5, 0, 0, 1, 0, 2]);
    }

    #[test]
    fn committed_text_temporarily_releases_control_and_preserves_owner_state() {
        let (mut input, mut server, _) = input_pair();
        input.key(super::XK_CONTROL_L, true).unwrap();
        input.type_text("c").unwrap();
        input.release_all().unwrap();
        let expected = [
            super::key_record(super::XK_CONTROL_L, true),
            super::key_record(super::XK_CONTROL_L, false),
            super::key_record('c' as u32, true),
            super::key_record('c' as u32, false),
            super::key_record(super::XK_CONTROL_L, true),
            super::key_record(super::XK_CONTROL_L, false),
        ]
        .concat();
        let mut actual = vec![0; expected.len()];
        server.read_exact(&mut actual).unwrap();
        assert_eq!(actual, expected);
    }
}
