//! Neo's one screen as an RFB client sees it. A resize is one short connection of its own; the watcher is one long connection for
//! the whole process, alive only while a fit is active. Xvnc logs five lines per connection,
//! which is why there is one watcher and not one per fit.
//!
//! TigerVNC sends a layout only inside a framebuffer update. So both clients ask for the 1×1
//! top-left pixel alone, which keeps every update a few bytes, and keep such a request
//! outstanding, so a layout change always has an update to ride in. A layout's reason says who
//! changed it: the server (0), this client's SetDesktopSize (1), or another client (2), which
//! is how noVNC's own `resize=remote` shows up.

use crate::limits::{RESIZE_MESSAGES, RESIZE_TIMEOUT, WATCH_REDIAL};
use crate::rfb_server::{self, LayoutReason, Parse, ServerInit, UpdateRequest};
use crate::{PartError, ScreenSize, Xvnc};
use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;

/// Xvnc, the screen every fit shares.
pub(crate) struct Screen {
    xvnc: Xvnc,
}

impl Screen {
    pub(crate) fn new(xvnc: Xvnc) -> Self {
        Self { xvnc }
    }

    /// The size the screen rests at between fits (`NEO_SCREEN`).
    pub(crate) fn home(&self) -> ScreenSize {
        self.xvnc.home
    }

    /// Resizes the screen the way noVNC's `resize=remote` does, on a fresh connection, within
    /// 2 s: SetDesktopSize after the first layout, then Xvnc's verdict on it.
    ///
    /// # Errors
    /// `Screen` when Xvnc cannot be reached, speaks other than RFB 3.8, refuses the size, or
    /// gives no verdict in time.
    pub(crate) async fn resize(&self, size: ScreenSize) -> Result<(), PartError> {
        tokio::time::timeout(RESIZE_TIMEOUT, one_resize(self.xvnc.addr, size))
            .await
            .unwrap_or_else(|_| {
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("no verdict on {size} within {RESIZE_TIMEOUT:?}"),
                ))
            })
            .map_err(PartError::Screen)
    }

    /// Reports the screen's size on `sizes`: ServerInit's first, then each layout, whoever
    /// changed it. Runs until it is dropped. When Xvnc drops the connection, or cannot be
    /// reached, it dials again a second later.
    pub(crate) async fn watch(&self, sizes: &watch::Sender<Option<ScreenSize>>) {
        loop {
            let Err(error) = one_watch(self.xvnc.addr, sizes).await;
            tracing::debug!(%error, "the screen watcher lost Xvnc; dialing again in 1 s");
            tokio::time::sleep(WATCH_REDIAL).await;
        }
    }
}

async fn one_resize(addr: SocketAddr, size: ScreenSize) -> io::Result<()> {
    let (mut rfb, init) = Rfb::connect(addr).await?;
    let mut asked = false;
    for _ in 0..RESIZE_MESSAGES {
        match rfb.next_layout(init).await? {
            None => {}
            // The first layout is the screen as it is now. Its screen id and flags go back in
            // the request as listed: noVNC sends id 0 and TigerVNC remaps ids, so they are read
            // fresh on each connection, never remembered from an earlier one.
            Some(layout) if !asked => {
                let screen = layout.first_screen.unwrap_or_default();
                rfb.write(&rfb_server::set_desktop_size(size, screen))
                    .await?;
                asked = true;
            }
            Some(layout) if layout.reason == LayoutReason::ThisClient => {
                return match layout.status {
                    0 => Ok(()),
                    status => Err(io::Error::other(format!(
                        "Xvnc refused {size} (status {status})"
                    ))),
                };
            }
            // Another client's change, while this one waits for its own verdict.
            Some(_) => {}
        }
        rfb.write(&rfb_server::update_request(UpdateRequest::OnChange))
            .await?;
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("Xvnc gave no verdict on {size} within {RESIZE_MESSAGES} messages"),
    ))
}

/// One watcher connection, until it fails.
async fn one_watch(
    addr: SocketAddr,
    sizes: &watch::Sender<Option<ScreenSize>>,
) -> io::Result<Infallible> {
    let (mut rfb, init) = Rfb::connect(addr).await?;
    publish(sizes, init.size);
    loop {
        if let Some(layout) = rfb.next_layout(init).await? {
            publish(sizes, layout.size);
        }
        rfb.write(&rfb_server::update_request(UpdateRequest::OnChange))
            .await?;
    }
}

/// Wakes the watched size's readers only when it changed, so a pixel update or a layout that
/// restates the size costs them nothing.
fn publish(sizes: &watch::Sender<Option<ScreenSize>>, size: ScreenSize) {
    sizes.send_if_modified(|seen| {
        let changed = *seen != Some(size);
        *seen = Some(size);
        changed
    });
}

/// One RFB connection: the socket and the bytes read but not yet parsed.
struct Rfb {
    stream: TcpStream,
    buffer: Vec<u8>,
}

impl Rfb {
    /// Dials Xvnc and runs the handshake as a shared client with no security. It then asks
    /// for raw pixels and layouts, and for the top-left pixel now, whose reply brings the
    /// current layout.
    async fn connect(addr: SocketAddr) -> io::Result<(Self, ServerInit)> {
        let mut rfb = Self {
            stream: TcpStream::connect(addr).await?,
            buffer: Vec::new(),
        };
        rfb.read(rfb_server::parse_version).await?;
        rfb.write(rfb_server::VERSION).await?;
        rfb.read(rfb_server::parse_security_types).await?;
        rfb.write(&[rfb_server::SECURITY_NONE]).await?;
        rfb.read(rfb_server::parse_security_result).await?;
        rfb.write(&[rfb_server::SHARED]).await?;
        let init = rfb.read(rfb_server::parse_server_init).await?;
        rfb.write(&rfb_server::set_encodings()).await?;
        rfb.write(&rfb_server::update_request(UpdateRequest::Now))
            .await?;
        Ok((rfb, init))
    }

    /// The next server message, reduced to the layout it carried.
    async fn next_layout(
        &mut self,
        init: ServerInit,
    ) -> io::Result<Option<rfb_server::DesktopLayout>> {
        self.read(|buffer| rfb_server::parse_server_message(buffer, init.bytes_per_pixel))
            .await
    }

    async fn read<T>(&mut self, parse: impl Fn(&[u8]) -> Parse<T>) -> io::Result<T> {
        loop {
            match parse(&self.buffer) {
                Parse::Done(item, used) => {
                    self.buffer.drain(..used.min(self.buffer.len()));
                    return Ok(item);
                }
                Parse::Invalid(why) => return Err(io::Error::new(io::ErrorKind::InvalidData, why)),
                Parse::Incomplete => {
                    self.buffer.reserve(4096);
                    if self.stream.read_buf(&mut self.buffer).await? == 0 {
                        return Err(io::ErrorKind::UnexpectedEof.into());
                    }
                }
            }
        }
    }

    async fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.stream.write_all(bytes).await
    }
}

#[cfg(test)]
mod tests {
    use super::{Rfb, Screen};
    use crate::rfb_server::{self, UpdateRequest};
    use crate::testing::FakeXvnc;
    use crate::{PartError, ScreenSize, Xvnc};
    use std::time::Duration;
    use tokio::sync::watch;

    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

    const HOME: ScreenSize = ScreenSize {
        width: 1600,
        height: 1000,
    };
    const PHONE: ScreenSize = ScreenSize {
        width: 390,
        height: 844,
    };
    const PHONE_TURNED: ScreenSize = ScreenSize {
        width: 412,
        height: 915,
    };

    fn screen(xvnc: &FakeXvnc) -> Screen {
        Screen::new(xvnc.xvnc(HOME))
    }

    /// Waits up to 3 s for the watched size to be `size`.
    async fn watched(
        sizes: &mut watch::Receiver<Option<ScreenSize>>,
        size: ScreenSize,
    ) -> TestResult {
        tokio::time::timeout(
            Duration::from_secs(3),
            sizes.wait_for(|seen| *seen == Some(size)),
        )
        .await??;
        Ok(())
    }

    #[tokio::test]
    async fn a_resize_sends_back_the_screen_id_and_flags_its_own_connection_read() -> TestResult {
        let xvnc = FakeXvnc::start(HOME).await?;
        xvnc.set_screen(7, 0x10);
        screen(&xvnc).resize(PHONE).await?;
        assert_eq!(xvnc.size(), PHONE);
        // TigerVNC remaps screen ids; the next resize must not reuse the id it saw before.
        xvnc.set_screen(9, 0x20);
        screen(&xvnc).resize(HOME).await?;
        assert_eq!(xvnc.size(), HOME);
        assert_eq!(
            xvnc.resizes(),
            ["390x844 id 7 flags 16", "1600x1000 id 9 flags 32"]
        );
        assert_eq!(xvnc.connections(), 2, "a fresh connection for each resize");
        Ok(())
    }

    #[tokio::test]
    async fn a_resize_xvnc_refuses_or_never_answers_is_an_error() -> TestResult {
        let xvnc = FakeXvnc::start(HOME).await?;
        xvnc.refuse_next(3);
        let refused = screen(&xvnc).resize(PHONE).await;
        assert!(
            matches!(&refused, Err(PartError::Screen(error)) if error.to_string().contains("status 3")),
            "{refused:?}"
        );
        assert_eq!(xvnc.size(), HOME, "a refused size changes nothing");

        xvnc.stop_answering();
        let unanswered = screen(&xvnc).resize(PHONE).await;
        assert!(
            matches!(&unanswered, Err(PartError::Screen(error)) if error.to_string().contains("16 messages")),
            "{unanswered:?}"
        );

        // A port whose listener is already closed.
        let gone = Xvnc {
            addr: std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?,
            home: HOME,
        };
        let unreachable = Screen::new(gone).resize(PHONE).await;
        assert!(
            matches!(unreachable, Err(PartError::Screen(_))),
            "{unreachable:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn the_watcher_reports_every_change_and_dials_again_when_xvnc_drops_it() -> TestResult {
        let xvnc = FakeXvnc::start(HOME).await?;
        let (sizes, mut watched_sizes) = watch::channel(None);
        let watcher = screen(&xvnc);
        let watching = tokio::spawn(async move { watcher.watch(&sizes).await });
        watched(&mut watched_sizes, HOME).await?;

        // The owner's noVNC resizes the screen: the watcher sees another client's layout.
        xvnc.resize_from_viewer(PHONE);
        watched(&mut watched_sizes, PHONE).await?;
        // So does a resize of Neo's own, on its own connection.
        screen(&xvnc).resize(PHONE_TURNED).await?;
        watched(&mut watched_sizes, PHONE_TURNED).await?;

        // Xvnc drops its clients; the size changes while the watcher is away.
        xvnc.drop_connections();
        xvnc.resize_from_viewer(HOME);
        watched(&mut watched_sizes, HOME).await?;
        assert_eq!(
            xvnc.connections(),
            3,
            "the watcher's first connection, the resize's, and the watcher's second"
        );
        watching.abort();
        Ok(())
    }

    /// The test above counts on this: a connection the fake dropped carries no change made after
    /// the drop, not even one that answers a request it holds. A fake that gets this wrong still
    /// gets it right on about half of its drops, so the test drops 32 times.
    #[tokio::test]
    async fn a_connection_xvnc_dropped_carries_no_later_change() -> TestResult {
        let xvnc = FakeXvnc::start(HOME).await?;
        let on_change = rfb_server::update_request(UpdateRequest::OnChange);
        for round in 0..32 {
            let (mut rfb, init) = Rfb::connect(xvnc.addr()).await?;
            assert!(
                rfb.next_layout(init).await?.is_some(),
                "the first update's layout"
            );
            // Two requests at once: the change answers the first, and the fake reads the second
            // before this client sees that answer, so it holds the second when the drop comes.
            rfb.write(&on_change.repeat(2)).await?;
            xvnc.resize_from_viewer(PHONE);
            let layout = rfb.next_layout(init).await?;
            assert_eq!(layout.map(|layout| layout.size), Some(PHONE));

            xvnc.drop_connections();
            xvnc.resize_from_viewer(HOME);
            let after = tokio::time::timeout(Duration::from_secs(3), rfb.next_layout(init)).await?;
            assert!(
                after.is_err(),
                "round {round}: the dropped connection sent {after:?}"
            );
        }
        Ok(())
    }
}
