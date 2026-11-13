//! A fake Xvnc for the tests of the takeover parts and of the crates built on them (zuzu's
//! `fakeNeoScreen`, ported). Hidden from the documented API; nothing in production builds one.
//!
//! It speaks RFB 3.8 with no security and one screen, as TigerVNC does in Neo's image:
//! - the first update a client asks for carries the layout (reason 0);
//! - a SetDesktopSize is applied and answered in the client's next update (reason 1, with a
//!   status), and every other client sees the change in its next one (reason 2);
//! - an incremental update request is answered when the screen changes, so a client gets a
//!   layout only while it keeps a request outstanding. Messages sent meanwhile are still read;
//! - a test can paint the screen ([`FakeXvnc::paint`]): every update then carries that pixel,
//!   and a held request is answered with it, as Xvnc answers once the browser repaints;
//! - it takes every message a display may pass on, and records each one a connection sends, its
//!   handshake included ([`FakeXvnc::received`]). Any other message closes the connection;
//! - like Xvnc, it closes a connection at its client's end of stream. A test can make it busy
//!   instead ([`FakeXvnc::hold_reads`]: a client's bytes wait on the socket) or keep a connection
//!   open past that end ([`FakeXvnc::stay_open`]).
//!
//! The bytes are written and framed by hand, not with `rfb_server`'s or `rfb_client`'s parsers,
//! so a parser bug cannot hide behind a fake that shares it.
//!

use crate::{ScreenSize, Xvnc};
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// A fake Xvnc on a free local port. Dropping it stops it and closes every connection.
pub struct FakeXvnc {
    addr: SocketAddr,
    shared: Arc<Shared>,
    accepting: JoinHandle<()>,
}

struct Shared {
    state: Mutex<State>,
    /// Bumped by every change of the screen's size. A connection holding an incremental
    /// request waits for it, as Xvnc holds the request until something changes.
    changes: watch::Sender<u64>,
    /// Bumped by every paint, which a held request also waits for.
    paints: watch::Sender<u64>,
    /// While true, no connection reads what its client sends after the handshake.
    reads_held: watch::Sender<bool>,
}

struct State {
    size: ScreenSize,
    /// The top-left pixel every update carries.
    pixel: [u8; 4],
    screen_id: u32,
    screen_flags: u32,
    /// Each SetDesktopSize received, as `<w>x<h> id <id> flags <flags>`.
    requests: Vec<String>,
    /// Every message each connection sent, by the order connections were accepted.
    received: Vec<Vec<Vec<u8>>>,
    /// Each connection whose client's end of stream it read, by the same order.
    ended: Vec<bool>,
    /// Whether a connection stays open after its client's end of stream.
    stays_open: bool,
    answer: Answer,
    /// Cancelled to close every open connection.
    open: CancellationToken,
}

/// How the fake answers the next SetDesktopSize.
#[derive(Clone, Copy)]
enum Answer {
    /// Applies it: reason 1, status 0.
    Apply,
    /// Refuses it once with this status, leaving the size as it is.
    Refuse(u16),
    /// Never gives a verdict, and answers every update request at once.
    Never,
}

impl FakeXvnc {
    /// Listens on a free local port with a screen of `size`, screen id 7 and flags 0.
    ///
    /// # Errors
    /// When no local port can be bound.
    pub async fn start(size: ScreenSize) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                size,
                pixel: [0x20, 0x40, 0x60, 0],
                screen_id: 7,
                screen_flags: 0,
                requests: Vec::new(),
                received: Vec::new(),
                ended: Vec::new(),
                stays_open: false,
                answer: Answer::Apply,
                open: CancellationToken::new(),
            }),
            changes: watch::channel(0).0,
            paints: watch::channel(0).0,
            reads_held: watch::channel(false).0,
        });
        let accepting = tokio::spawn({
            let shared = Arc::clone(&shared);
            async move {
                while let Ok((socket, _)) = listener.accept().await {
                    let (connection, open) = {
                        let mut state = shared.state();
                        state.received.push(Vec::new());
                        state.ended.push(false);
                        (state.received.len() - 1, state.open.clone())
                    };
                    tokio::spawn(serve(Arc::clone(&shared), socket, connection, open));
                }
            }
        });
        Ok(Self {
            addr,
            shared,
            accepting,
        })
    }

    /// Where it listens.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// This fake as the takeover parts' Xvnc, resting at `home` between fits.
    #[must_use]
    pub fn xvnc(&self, home: ScreenSize) -> Xvnc {
        Xvnc {
            addr: self.addr,
            home,
        }
    }

    /// The screen's size now.
    #[must_use]
    pub fn size(&self) -> ScreenSize {
        self.shared.state().size
    }

    /// Every SetDesktopSize received so far, in order, as `<w>x<h> id <id> flags <flags>`.
    #[must_use]
    pub fn resizes(&self) -> Vec<String> {
        self.shared.state().requests.clone()
    }

    /// How many connections it has accepted. Xvnc logs each one.
    #[must_use]
    pub fn connections(&self) -> usize {
        self.shared.state().received.len()
    }

    /// Every message connection `connection` sent so far, in order, each with its type byte:
    /// the version, the security type and the ClientInit first. Connections count from 0, in
    /// the order they were accepted.
    #[must_use]
    pub fn received(&self, connection: usize) -> Vec<Vec<u8>> {
        self.shared
            .state()
            .received
            .get(connection)
            .cloned()
            .unwrap_or_default()
    }

    /// Whether connection `connection` read its client's end of stream: the client half-closed or
    /// closed it.
    #[must_use]
    pub fn saw_end(&self, connection: usize) -> bool {
        self.shared
            .state()
            .ended
            .get(connection)
            .copied()
            .unwrap_or(false)
    }

    /// Stops reading what clients send after their handshake, as a busy Xvnc does: the bytes
    /// wait on each socket, in order, until [`FakeXvnc::resume_reads`].
    pub fn hold_reads(&self) {
        self.shared.reads_held.send_replace(true);
    }

    /// Reads what clients send again, from where each connection stopped.
    pub fn resume_reads(&self) {
        self.shared.reads_held.send_replace(false);
    }

    /// Keeps each connection open after its client's end of stream from now on, instead of
    /// closing it as Xvnc does, until the fake drops it.
    pub fn stay_open(&self) {
        self.shared.state().stays_open = true;
    }

    /// Lists the screen with this id and these flags from now on, as TigerVNC does when it
    /// remaps the ids a client sent.
    pub fn set_screen(&self, id: u32, flags: u32) {
        let mut state = self.shared.state();
        state.screen_id = id;
        state.screen_flags = flags;
    }

    /// Refuses the next SetDesktopSize with `status`.
    pub fn refuse_next(&self, status: u16) {
        self.shared.state().answer = Answer::Refuse(status);
    }

    /// Never gives a verdict on a SetDesktopSize from now on, and answers every update request
    /// at once without a layout.
    pub fn stop_answering(&self) {
        self.shared.state().answer = Answer::Never;
    }

    /// The browser repaints the screen: every update carries `pixel` from now on, and an
    /// incremental request a connection holds is answered with it at once.
    pub fn paint(&self, pixel: [u8; 4]) {
        self.shared.state().pixel = pixel;
        self.shared.paints.send_modify(|paints| *paints += 1);
    }

    /// Another client resizes the screen, as the owner's noVNC does with `resize=remote`.
    pub fn resize_from_viewer(&self, size: ScreenSize) {
        self.shared.state().size = size;
        self.shared.changes.send_modify(|changes| *changes += 1);
    }

    /// Closes every open connection, as when Xvnc restarts. A dropped connection sends nothing
    /// more, so a client learns of a change made after this returns only once it dials again.
    pub fn drop_connections(&self) {
        let mut state = self.shared.state();
        state.open.cancel();
        state.open = CancellationToken::new();
    }
}

impl Drop for FakeXvnc {
    fn drop(&mut self) {
        self.accepting.abort();
        self.shared.state().open.cancel();
    }
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes a SetDesktopSize to `size` from one client. Returns the (reason, status) that
    /// client's next update carries, or `None` for no verdict at all.
    fn resize(&self, size: ScreenSize, screen: u32, flags: u32) -> Option<(u16, u16)> {
        let mut state = self.state();
        state
            .requests
            .push(format!("{size} id {screen} flags {flags}"));
        match state.answer {
            Answer::Apply => {
                state.size = size;
                drop(state);
                self.changes.send_modify(|changes| *changes += 1);
                Some((1, 0))
            }
            Answer::Refuse(status) => {
                state.answer = Answer::Apply;
                Some((1, status))
            }
            Answer::Never => None,
        }
    }

    /// A FramebufferUpdate: the layout when one is due, then the raw top-left pixel.
    fn update(&self, layout: Option<(u16, u16)>) -> Vec<u8> {
        let state = self.state();
        let rectangles: u16 = if layout.is_some() { 2 } else { 1 };
        let mut bytes = vec![0, 0];
        bytes.extend_from_slice(&rectangles.to_be_bytes());
        if let Some((reason, status)) = layout {
            let ScreenSize { width, height } = state.size;
            for value in [reason, status, width, height] {
                bytes.extend_from_slice(&value.to_be_bytes());
            }
            bytes.extend_from_slice(&(-308i32).to_be_bytes());
            bytes.extend_from_slice(&[1, 0, 0, 0]);
            bytes.extend_from_slice(&state.screen_id.to_be_bytes());
            for value in [0, 0, width, height] {
                bytes.extend_from_slice(&value.to_be_bytes());
            }
            bytes.extend_from_slice(&state.screen_flags.to_be_bytes());
        }
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0]);
        bytes.extend_from_slice(&state.pixel);
        bytes
    }

    fn answers_at_once(&self) -> bool {
        matches!(self.state().answer, Answer::Never)
    }

    /// A client's end of stream on `connection`: noted, then the connection closes as Xvnc's does,
    /// or stays open until the fake drops it.
    async fn client_ended(&self, connection: usize) -> io::Result<()> {
        let stays_open = {
            let mut state = self.state();
            if let Some(ended) = state.ended.get_mut(connection) {
                *ended = true;
            }
            state.stays_open
        };
        if stays_open {
            std::future::pending::<()>().await;
        }
        Ok(())
    }

    fn record(&self, connection: usize, message: &[u8]) {
        if let Some(received) = self.state().received.get_mut(connection) {
            received.push(message.to_vec());
        }
    }
}

/// One client's connection, until it closes, sends a message Xvnc would not take, or the fake
/// drops it.
async fn serve(
    shared: Arc<Shared>,
    mut socket: TcpStream,
    connection: usize,
    open: CancellationToken,
) {
    // Biased, so the first poll after a drop ends the connection without running `converse`.
    // Unbiased, `converse` could run first and answer a held request with a change made after
    // the drop, and the client would see that change before it dials again.
    tokio::select! {
        biased;
        () = open.cancelled() => {}
        _ = converse(&shared, &mut socket, connection) => {}
    }
}

async fn converse(shared: &Shared, socket: &mut TcpStream, connection: usize) -> io::Result<()> {
    socket.write_all(b"RFB 003.008\n").await?;
    let mut version = [0; 12];
    socket.read_exact(&mut version).await?;
    shared.record(connection, &version);
    // One security type, None; then the client's pick and a SecurityResult of 0.
    socket.write_all(&[1, 1]).await?;
    let security = socket.read_u8().await?;
    shared.record(connection, &[security]);
    socket.write_all(&[0, 0, 0, 0]).await?;
    // ClientInit, then ServerInit: the size, 32 bits per pixel true colour, the name "neo".
    let shared_flag = socket.read_u8().await?;
    shared.record(connection, &[shared_flag]);
    let ScreenSize { width, height } = shared.state().size;
    let mut init = Vec::new();
    init.extend_from_slice(&width.to_be_bytes());
    init.extend_from_slice(&height.to_be_bytes());
    init.extend_from_slice(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]);
    init.extend_from_slice(&3u32.to_be_bytes());
    init.extend_from_slice(b"neo");
    socket.write_all(&init).await?;

    let mut changes = shared.changes.subscribe();
    let mut paints = shared.paints.subscribe();
    let mut reads_held = shared.reads_held.subscribe();
    let mut layout = Some((0, 0));
    // An incremental update request the fake holds until the screen changes.
    let mut held = false;
    loop {
        let reading = !*reads_held.borrow_and_update();
        let kind = tokio::select! {
            biased;
            changed = changes.changed(), if held => {
                // Another client's resize, or nothing until the connection goes.
                if changed.is_err() {
                    return Ok(());
                }
                held = false;
                changes.borrow_and_update();
                socket.write_all(&shared.update(Some((2, 0)))).await?;
                continue;
            }
            painted = paints.changed(), if held => {
                if painted.is_err() {
                    return Ok(());
                }
                held = false;
                paints.borrow_and_update();
                socket.write_all(&shared.update(None)).await?;
                continue;
            }
            resumed = reads_held.changed(), if !reading => {
                if resumed.is_err() {
                    return Ok(());
                }
                continue;
            }
            kind = socket.read_u8(), if reading => match kind {
                Ok(kind) => kind,
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                    return shared.client_ended(connection).await;
                }
                Err(error) => return Err(error),
            },
        };
        let Some(message) = read_message(socket, kind).await? else {
            return Ok(());
        };
        shared.record(connection, &message);
        let byte = |at: usize| message.get(at).copied().unwrap_or(0);
        match kind {
            // FramebufferUpdateRequest: incremental, then x, y, w, h.
            3 if byte(1) == 1 && layout.is_none() && !shared.answers_at_once() => held = true,
            3 => {
                changes.borrow_and_update();
                paints.borrow_and_update();
                socket.write_all(&shared.update(layout.take())).await?;
            }
            // SetDesktopSize: padding, w, h, a screen count, padding, 16 bytes per screen.
            251 => {
                let size = ScreenSize {
                    width: u16::from_be_bytes([byte(2), byte(3)]),
                    height: u16::from_be_bytes([byte(4), byte(5)]),
                };
                let field = |at: usize| {
                    message
                        .get(8 + at..8 + at + 4)
                        .and_then(|bytes| bytes.try_into().ok())
                        .map_or(0, u32::from_be_bytes)
                };
                layout = shared.resize(size, field(0), field(12));
                // This client's own change is not another client's.
                changes.borrow_and_update();
                // A change answers the request the fake holds, as Xvnc's next update does.
                if held {
                    held = false;
                    socket.write_all(&shared.update(layout.take())).await?;
                }
            }
            _ => {}
        }
    }
}

/// The client message whose type byte was `kind`, read whole and framed by hand as RFB 3.8 says,
/// the type byte first. `None` for a message Neo's display never passes on.
async fn read_message(socket: &mut TcpStream, kind: u8) -> io::Result<Option<Vec<u8>>> {
    let mut message = vec![kind];
    let rest = match kind {
        0 => 19,
        3 | 150 => 9,
        4 => 7,
        5 => 5,
        // SetEncodings: padding, a u16 count, 4 bytes each.
        2 => {
            take(socket, &mut message, 3).await?;
            4 * usize::from(u16::from_be_bytes([message[2], message[3]]))
        }
        // ClientCutText: padding, an i32 length (negative: extended, −length bytes), the text.
        6 => {
            take(socket, &mut message, 7).await?;
            let length = i32::from_be_bytes([message[4], message[5], message[6], message[7]]);
            usize::try_from(length.unsigned_abs()).unwrap_or(usize::MAX)
        }
        // ClientFence: padding, u32 flags, a u8 length, the payload.
        248 => {
            take(socket, &mut message, 8).await?;
            usize::from(message[8])
        }
        // SetDesktopSize: padding, w, h, a screen count, padding, 16 bytes per screen.
        251 => {
            take(socket, &mut message, 7).await?;
            16 * usize::from(message[6])
        }
        // QEMU: a subtype; 0, the extended key event, has 10 bytes more.
        255 => {
            take(socket, &mut message, 1).await?;
            if message[1] != 0 {
                return Ok(None);
            }
            10
        }
        _ => return Ok(None),
    };
    take(socket, &mut message, rest).await?;
    Ok(Some(message))
}

/// Reads `count` more bytes onto `message`.
async fn take(socket: &mut TcpStream, message: &mut Vec<u8>, count: usize) -> io::Result<()> {
    let start = message.len();
    message.resize(start + count, 0);
    socket.read_exact(&mut message[start..]).await.map(drop)
}
