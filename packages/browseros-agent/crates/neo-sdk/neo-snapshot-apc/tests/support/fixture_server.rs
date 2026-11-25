//! Tiny dual-stack (`127.0.0.1` + `[::1]`) HTTP fixture server on one ephemeral port.
//!
//! Binding both loopback addresses on the same port lets Chromium treat `127.0.0.1`, `localhost`,
//! and `[::1]` as three distinct sites without relying on external DNS. Modeled on
//! `crates/neo-sdk/neo-playwright/tests/support/fixture_server.rs`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io;
use std::sync::Arc;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

/// One HTTP response served by [`FixtureServer`].
#[derive(Clone, Debug)]
pub(crate) struct Page {
    pub(crate) body: String,
    pub(crate) content_type: &'static str,
    /// Additional HTTP response headers emitted after `content-length`.
    pub(crate) headers: Vec<(&'static str, &'static str)>,
}

impl Page {
    /// Creates a UTF-8 HTML response with `cache-control: no-store`.
    pub(crate) fn html(body: &str) -> Self {
        Self {
            body: body.to_owned(),
            content_type: "text/html; charset=utf-8",
            headers: Vec::new(),
        }
        .with_header("cache-control", "no-store")
    }

    /// Appends one extra response header.
    pub(crate) fn with_header(mut self, name: &'static str, value: &'static str) -> Self {
        self.headers.push((name, value));
        self
    }
}

/// Running loopback HTTP server that aborts its accept tasks on [`Drop`].
#[derive(Debug)]
pub(crate) struct FixtureServer {
    port: u16,
    listeners: JoinSet<()>,
}

/// Maximum ephemeral-port attempts before giving up on finding a free dual-stack port.
const PORT_TRIES: usize = 20;

impl FixtureServer {
    /// Starts serving `pages` by request path (ignoring any query string) on both `127.0.0.1` and
    /// `[::1]` using the same ephemeral port.
    pub(crate) async fn start(pages: HashMap<String, Page>) -> io::Result<Self> {
        let pages = Arc::new(pages);
        let mut taken = None;
        for _ in 0..PORT_TRIES {
            let v4 = TcpListener::bind("127.0.0.1:0").await?;
            let port = v4.local_addr()?.port();
            match TcpListener::bind(("::1", port)).await {
                Ok(v6) => {
                    taken = Some((port, [v4, v6]));
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => {}
                Err(error) => return Err(error),
            }
        }
        let Some((port, bound)) = taken else {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "no port free on both 127.0.0.1 and [::1]",
            ));
        };
        let mut listeners = JoinSet::new();
        for listener in bound {
            let pages = Arc::clone(&pages);
            listeners.spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    drop(tokio::spawn(serve(stream, Arc::clone(&pages))));
                }
            });
        }
        Ok(Self { port, listeners })
    }

    /// Builds an HTTP URL for `path` on `host` (`127.0.0.1`, `localhost`, or `[::1]`).
    pub(crate) fn url(&self, host: &str, path: &str) -> String {
        format!("http://{host}:{}{path}", self.port)
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.listeners.abort_all();
    }
}

/// Handles one HTTP/1.1 request and closes the connection.
async fn serve(mut stream: TcpStream, pages: Arc<HashMap<String, Page>>) {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    while !request.windows(4).any(|end| end == b"\r\n\r\n") {
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => request.extend_from_slice(buffer.get(..read).unwrap_or_default()),
        }
    }
    let request = String::from_utf8_lossy(&request);
    let path = request
        .split_whitespace()
        .nth(1)
        .and_then(|target| target.split('?').next())
        .unwrap_or("/");
    let response = match pages.get(path) {
        Some(page) => {
            let mut headers = String::new();
            for (name, value) in &page.headers {
                let _infallible = write!(headers, "{name}: {value}\r\n");
            }
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {}\r\ncontent-length: {}\r\n{headers}connection: close\r\n\r\n{}",
                page.content_type,
                page.body.len(),
                page.body
            )
        }
        None => {
            "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned()
        }
    };
    let _written = stream.write_all(response.as_bytes()).await;
    let _closed = stream.shutdown().await;
}
