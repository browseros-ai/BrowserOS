//! A tiny HTTP server for live tests' fixture pages. It listens on `127.0.0.1` and `[::1]` on
//! one port, so `127.0.0.1`, `localhost` and `[::1]` all reach it and Chromium treats them as
//! three sites: a page on one with an iframe on another gets an out-of-process iframe, as the
//! spike's `lib/dualserver.mjs` does. Shared by the crate's own live tests (through `#[path]`)
//! and the integration tests, so it names nothing from the crate.
//!
//! Each address is bound by itself, never through the `[::]` wildcard: with `SO_REUSEADDR`
//! (tokio's default) a wildcard bind succeeds on a port another process holds on `127.0.0.1`,
//! and the browser's requests for `127.0.0.1` then reach that process (measured: `page.goto`
//! failed with `ERR_HTTP_RESPONSE_CODE_FAILURE` on ports another process held on 127.0.0.1).

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

/// One page the server answers with.
#[derive(Clone, Debug)]
pub(crate) struct Page {
    pub(crate) body: String,
    pub(crate) content_type: &'static str,
    /// Extra response headers, such as a `content-security-policy`.
    pub(crate) headers: Vec<(&'static str, &'static str)>,
}

impl Page {
    /// An HTML page.
    pub(crate) fn html(body: &str) -> Self {
        Self {
            body: body.to_owned(),
            content_type: "text/html; charset=utf-8",
            headers: Vec::new(),
        }
    }

    /// A script.
    pub(crate) fn script(body: &str) -> Self {
        Self {
            content_type: "text/javascript",
            ..Self::html(body)
        }
    }

    /// The same page with one more response header.
    pub(crate) fn with_header(mut self, name: &'static str, value: &'static str) -> Self {
        self.headers.push((name, value));
        self
    }
}

/// The spike's nested-frame fixture (`tests/data/nest/`), under `/nest/`: four levels of
/// iframes over three sites (A `127.0.0.1`, B `localhost`, C `[::1]`), each frame with its
/// own button and field. Open `/nest/top.html` on `127.0.0.1`.
///
/// ```text
///  top A ─┬─ a1 A (same process) ── b2 B (OOPIF) ── b3 B (same process as b2)
///         └─ b1 B (OOPIF) ── c2 C (OOPIF) ── a3 A (its own target) ── c4 C (OOPIF)
/// ```
pub(crate) fn nest_pages() -> HashMap<String, Page> {
    let pages = [
        ("top", include_str!("../data/nest/top.html")),
        ("a1", include_str!("../data/nest/a1.html")),
        ("a3", include_str!("../data/nest/a3.html")),
        ("b1", include_str!("../data/nest/b1.html")),
        ("b2", include_str!("../data/nest/b2.html")),
        ("b3", include_str!("../data/nest/b3.html")),
        ("c2", include_str!("../data/nest/c2.html")),
        ("c4", include_str!("../data/nest/c4.html")),
    ];
    let mut served: HashMap<String, Page> = pages
        .into_iter()
        .map(|(name, body)| {
            let page = Page::html(body).with_header("cache-control", "no-store");
            (format!("/nest/{name}.html"), page)
        })
        .collect();
    served.insert(
        "/nest/frame.js".to_owned(),
        Page::script(include_str!("../data/nest/frame.js")),
    );
    served
}

/// The running server; it stops when dropped.
#[derive(Debug)]
pub(crate) struct FixtureServer {
    port: u16,
    /// One accept loop per address.
    listeners: JoinSet<()>,
}

/// Free ports to try before giving up: another process can hold the IPv6 side of a port the
/// IPv4 bind got.
const PORT_TRIES: usize = 20;

impl FixtureServer {
    /// Serves `pages` by path (`/nest/top.html`); the query string is ignored.
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

    /// The URL of `path` on `host` (`127.0.0.1`, `localhost` or `[::1]`).
    pub(crate) fn url(&self, host: &str, path: &str) -> String {
        format!("http://{host}:{}{path}", self.port)
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.listeners.abort_all();
    }
}

/// One request, then the connection closes.
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
            use std::fmt::Write as _;
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
