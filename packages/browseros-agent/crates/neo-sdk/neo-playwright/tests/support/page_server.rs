//! A tiny HTTP server for the live tests' fixture pages, on `127.0.0.1` with an ephemeral port.
//! `localhost` on the same port is another site, so a frame served from it is an
//! out-of-process iframe. Pages may name `{{SAME}}` (`http://127.0.0.1:<port>`) and `{{CROSS}}`
//! (`http://localhost:<port>`); the server fills both in.
//!
//! Routes beyond the fixtures: `/slow.png` answers after [`SLOW`], holding the load event of a
//! page that shows it; `/missing.html` is a 404 page; `/echo.html` prints the request's
//! `Referer` header in `#referer`; `/stall.html` takes the request and never answers, so a
//! navigation to it never commits; `/beforeunload.html` asks to stay whenever it is left.

use std::error::Error;
use std::future;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio::time;

/// How long `/slow.png` takes to answer.
pub(crate) const SLOW: Duration = Duration::from_millis(1500);

/// The fixture pages, by path.
const PAGES: [(&str, &str); 5] = [
    ("/slow.html", include_str!("../data/slow.html")),
    ("/child.html", include_str!("../data/child.html")),
    ("/frames.html", include_str!("../data/frames.html")),
    ("/nested.html", include_str!("../data/nested.html")),
    ("/popups.html", include_str!("../data/popups.html")),
];

/// The running server. Dropping it stops accepting connections.
pub(crate) struct PageServer {
    port: u16,
    task: JoinHandle<()>,
}

impl Drop for PageServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl PageServer {
    /// Binds an ephemeral port and serves until dropped.
    pub(crate) async fn start() -> Result<Self, Box<dyn Error>> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let task = tokio::spawn(async move {
            while let Ok((stream, _peer)) = listener.accept().await {
                tokio::spawn(serve(stream, port));
            }
        });
        Ok(Self { port, task })
    }

    /// `path` on the server's own site, `http://127.0.0.1:<port><path>`.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

/// Answers one request and closes the connection.
async fn serve(mut stream: TcpStream, port: u16) {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => request.extend_from_slice(chunk.get(..read).unwrap_or_default()),
        }
    }
    let request = String::from_utf8_lossy(&request);
    let target = request.split_whitespace().nth(1).unwrap_or("/");
    let path = target.split('?').next().unwrap_or(target);
    if path == "/stall.html" {
        // The connection stays open and silent until the test's runtime drops this task.
        future::pending::<()>().await;
    }
    let referer = request
        .lines()
        .find_map(|line| {
            line.strip_prefix("Referer: ")
                .or_else(|| line.strip_prefix("referer: "))
        })
        .unwrap_or("none");
    let (status, content_type, body) = match path {
        "/slow.png" => {
            time::sleep(SLOW).await;
            ("200 OK", "image/png", String::new())
        }
        "/missing.html" => ("404 Not Found", "text/html", "<p>missing</p>".to_owned()),
        "/echo.html" => (
            "200 OK",
            "text/html",
            format!("<!doctype html><p id=\"referer\">{referer}</p>"),
        ),
        "/beforeunload.html" => (
            "200 OK",
            "text/html",
            "<!doctype html><body style=\"height: 100vh\"><script>addEventListener(\
             'beforeunload', (event) => { event.preventDefault(); event.returnValue = ''; \
             });</script><p>stay</p></body>"
                .to_owned(),
        ),
        _ => match PAGES.iter().find(|(known, _)| *known == path) {
            Some((_, page)) => (
                "200 OK",
                "text/html; charset=utf-8",
                page.replace("{{SAME}}", &format!("http://127.0.0.1:{port}"))
                    .replace("{{CROSS}}", &format!("http://localhost:{port}")),
            ),
            None => ("404 Not Found", "text/html", String::new()),
        },
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}
