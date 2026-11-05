//! A fake Xvnc for the tests of the takeover parts and of the crates built on them (zuzu's
//! `fakeNeoScreen`, ported). Hidden from the documented API; nothing in production builds one.
//!
//! It speaks RFB 3.8 with no security and one screen, as TigerVNC does in Neo's image:
//! - the first update a client asks for carries the layout (reason 0);
//! - a SetDesktopSize is applied and answered in the client's next update (reason 1, with a
//!   status), and every other client sees the change in its next one (reason 2);
//! - an incremental update request waits until the screen changes, so a client gets a layout
//!   only while it keeps a request outstanding;
//! - any other message closes the connection.
//!
//! The bytes are written by hand, not with `rfb_server`'s parsers, so a parser bug cannot hide
//! behind a fake that shares it.

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
}

struct State {
    size: ScreenSize,
    screen_id: u32,
    screen_flags: u32,
    /// Each SetDesktopSize received, as `<w>x<h> id <id> flags <flags>`.
    requests: Vec<String>,
    connections: usize,
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
                screen_id: 7,
                screen_flags: 0,
                requests: Vec::new(),
                connections: 0,
                answer: Answer::Apply,
                open: CancellationToken::new(),
            }),
            changes: watch::channel(0).0,
        });
        let accepting = tokio::spawn({
            let shared = Arc::clone(&shared);
            async move {
                while let Ok((socket, _)) = listener.accept().await {
                    let open = {
                        let mut state = shared.state();
                        state.connections += 1;
                        state.open.clone()
                    };
                    tokio::spawn(serve(Arc::clone(&shared), socket, open));
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
        self.shared.state().connections
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
        bytes.extend_from_slice(&[0x20, 0x40, 0x60, 0]);
        bytes
    }

    fn answers_at_once(&self) -> bool {
        matches!(self.state().answer, Answer::Never)
    }
}

/// One client's connection, until it closes, sends a message Xvnc would not take, or the fake
/// drops it.
async fn serve(shared: Arc<Shared>, mut socket: TcpStream, open: CancellationToken) {
    // Biased, so the first poll after a drop ends the connection without running `converse`.
    // Unbiased, `converse` could run first and answer a held request with a change made after
    // the drop, and the client would see that change before it dials again.
    tokio::select! {
        biased;
        () = open.cancelled() => {}
        _ = converse(&shared, &mut socket) => {}
    }
}

async fn converse(shared: &Shared, socket: &mut TcpStream) -> io::Result<()> {
    socket.write_all(b"RFB 003.008\n").await?;
    socket.read_exact(&mut [0; 12]).await?;
    // One security type, None; then the client's pick and a SecurityResult of 0.
    socket.write_all(&[1, 1]).await?;
    socket.read_u8().await?;
    socket.write_all(&[0, 0, 0, 0]).await?;
    // ClientInit, then ServerInit: the size, 32 bits per pixel true colour, the name "neo".
    socket.read_u8().await?;
    let ScreenSize { width, height } = shared.state().size;
    let mut init = Vec::new();
    init.extend_from_slice(&width.to_be_bytes());
    init.extend_from_slice(&height.to_be_bytes());
    init.extend_from_slice(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]);
    init.extend_from_slice(&3u32.to_be_bytes());
    init.extend_from_slice(b"neo");
    socket.write_all(&init).await?;

    let mut changes = shared.changes.subscribe();
    let mut layout = Some((0, 0));
    loop {
        match socket.read_u8().await? {
            // SetEncodings: padding, a u16 count, 4 bytes each.
            2 => {
                let mut head = [0; 3];
                socket.read_exact(&mut head).await?;
                let count = u16::from_be_bytes([head[1], head[2]]);
                socket
                    .read_exact(&mut vec![0; 4 * usize::from(count)])
                    .await?;
            }
            // FramebufferUpdateRequest: incremental, then x, y, w, h.
            3 => {
                let mut request = [0; 9];
                socket.read_exact(&mut request).await?;
                if request[0] == 1 && layout.is_none() && !shared.answers_at_once() {
                    // Another client's resize, or nothing until the connection goes.
                    if changes.changed().await.is_err() {
                        return Ok(());
                    }
                    layout = Some((2, 0));
                }
                changes.borrow_and_update();
                socket.write_all(&shared.update(layout.take())).await?;
            }
            // SetDesktopSize: padding, w, h, a screen count, padding, 16 bytes per screen.
            251 => {
                let mut head = [0; 7];
                socket.read_exact(&mut head).await?;
                let mut screens = vec![0; 16 * usize::from(head[5])];
                socket.read_exact(&mut screens).await?;
                let size = ScreenSize {
                    width: u16::from_be_bytes([head[1], head[2]]),
                    height: u16::from_be_bytes([head[3], head[4]]),
                };
                let field = |at: usize| {
                    screens
                        .get(at..at + 4)
                        .and_then(|bytes| bytes.try_into().ok())
                        .map_or(0, u32::from_be_bytes)
                };
                layout = shared.resize(size, field(0), field(12));
                // This client's own change is not another client's.
                changes.borrow_and_update();
            }
            _ => return Ok(()),
        }
    }
}
