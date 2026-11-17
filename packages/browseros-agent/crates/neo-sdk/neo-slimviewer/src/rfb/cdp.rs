//! Request-local CDP sessions: no target activation, cached focus, or mutation replay.
//! Focus check and insertText are not atomic. See README's typing limits.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tungstenite::{Message, WebSocket, client::client_with_config, protocol::WebSocketConfig};

pub(super) mod agent;
pub(super) mod desktop;
mod fields;
mod geometry;
mod paste;
mod protocol;
mod session;

const CDP_PORT: u16 = 9110;
const LIMIT: usize = 1024 * 1024;
const MAX_PAGES: usize = 32;
const BUDGET: Duration = Duration::from_millis(750);
const FOCUS: &str =
    "({focused:document.hasFocus(),visible:document.visibilityState === 'visible'})";

type Result<T> = std::result::Result<T, &'static str>;

#[derive(Debug)]
pub(super) enum DeliveryError {
    NotSent(&'static str),
    Uncertain,
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSent(reason) => write!(f, "not sent: {reason}"),
            Self::Uncertain => f.write_str("delivery uncertain; not replayed"),
        }
    }
}

pub(super) fn insert_text(text: &str) -> std::result::Result<(), DeliveryError> {
    insert_at(text, CDP_PORT)
}

fn insert_at(text: &str, port: u16) -> std::result::Result<(), DeliveryError> {
    let deadline = Instant::now() + BUDGET;
    let mut client = Client::connect(port, deadline).map_err(DeliveryError::NotSent)?;
    let page = client.focused_page().map_err(DeliveryError::NotSent)?;
    if !client
        .still_selected(&page)
        .map_err(DeliveryError::NotSent)?
    {
        return Err(DeliveryError::NotSent("focus changed"));
    }
    // Even an I/O error can follow a successful write. Never resend this command.
    client
        .call(
            "Input.insertText",
            json!({"text": text}),
            Some(&page.session),
        )
        .map_err(|_| DeliveryError::Uncertain)?;
    Ok(())
}

#[derive(Clone)]
struct FocusedPage {
    target: String,
    session: String,
    context: u64,
    evidence: FocusEvidence,
}

#[derive(Clone)]
enum FocusEvidence {
    Document,
    Native,
    Visible,
}

struct Focus {
    focused: bool,
    visible: bool,
}

pub(super) struct Client {
    socket: WebSocket<TimedStream>,
    next_id: u64,
    deadline: Instant,
}

impl Client {
    fn connect(port: u16, deadline: Instant) -> Result<Self> {
        let mut http = connect(port, deadline)?;
        write!(
            http,
            "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .map_err(|_| "discovery write failed")?;
        let version = discovery(&mut http, deadline)?;
        let url = version["webSocketDebuggerUrl"]
            .as_str()
            .ok_or("missing browser socket")?;
        let path = browser_path(url)?;
        // Dial the fixed loopback address, never the authority supplied by discovery.
        let url = format!("ws://127.0.0.1:{port}{path}");
        let config = WebSocketConfig::default()
            .read_buffer_size(4096)
            .write_buffer_size(0)
            .max_message_size(Some(LIMIT))
            .max_frame_size(Some(LIMIT));
        let (socket, _) = client_with_config(url, connect(port, deadline)?, Some(config))
            .map_err(|_| "CDP handshake failed")?;
        Ok(Self {
            socket,
            next_id: 0,
            deadline,
        })
    }

    pub(super) fn call(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value> {
        self.next_id += 1;
        let mut request = json!({"id": self.next_id, "method": method, "params": params});
        if let Some(session) = session {
            request["sessionId"] = json!(session);
        }
        timeouts(self.socket.get_ref(), self.deadline)?;
        self.socket
            .send(Message::Text(request.to_string().into()))
            .map_err(|_| "CDP write failed")?;
        loop {
            timeouts(self.socket.get_ref(), self.deadline)?;
            match self.socket.read().map_err(|_| "CDP read failed")? {
                Message::Text(text) => {
                    let reply: Value =
                        serde_json::from_str(&text).map_err(|_| "invalid CDP JSON")?;
                    if reply["id"].as_u64() != Some(self.next_id) {
                        continue;
                    }
                    if let Some(error) = reply.get("error") {
                        if error["code"] == -32601 {
                            return Err("CDP method unavailable");
                        }
                        return Err("CDP command rejected");
                    }
                    return reply.get("result").cloned().ok_or("missing CDP result");
                }
                Message::Close(_) => return Err("CDP closed"),
                Message::Ping(_) | Message::Pong(_) => {}
                _ => return Err("unexpected CDP message"),
            }
        }
    }

    fn focused_page(&mut self) -> Result<FocusedPage> {
        let targets = self.call("Target.getTargets", json!({}), None)?;
        let targets = targets["targetInfos"].as_array().ok_or("missing targets")?;
        let pages: Vec<_> = targets
            .iter()
            .filter(|t| t["type"] == "page" && t.get("subtype").is_none())
            .collect();
        if pages.len() > MAX_PAGES {
            return Err("too many pages");
        }
        let mut candidates = Vec::new();
        for target in pages {
            let id = target["targetId"].as_str().ok_or("missing target id")?;
            let attached = self.call(
                "Target.attachToTarget",
                json!({"targetId": id, "flatten": true}),
                None,
            )?;
            let session = attached["sessionId"]
                .as_str()
                .ok_or("missing session")?
                .to_owned();
            let tree = self.call("Page.getFrameTree", json!({}), Some(&session))?;
            let frame = tree["frameTree"]["frame"]["id"]
                .as_str()
                .ok_or("missing main frame")?;
            let world = self.call(
                "Page.createIsolatedWorld",
                json!({"frameId":frame,"worldName":"slimviewer-input"}),
                Some(&session),
            )?;
            let page = FocusedPage {
                target: id.to_owned(),
                session,
                context: world["executionContextId"]
                    .as_u64()
                    .ok_or("missing context")?,
                evidence: FocusEvidence::Document,
            };
            let focus = self.focus(&page)?;
            if focus.visible {
                candidates.push((page, focus.focused));
            }
        }
        let focused: Vec<_> = candidates
            .iter()
            .enumerate()
            .filter_map(|(i, (_, focused))| focused.then_some(i))
            .collect();
        if let [index] = focused[..] {
            return Ok(candidates.swap_remove(index).0);
        }
        // Neo emulates focus on agent tabs. Native window/tab state is independent
        // of that emulation and can disambiguate several visible, "focused" pages.
        if let Some(target) = self.native_front()? {
            let mut page = candidates
                .into_iter()
                .find(|(page, _)| page.target == target)
                .ok_or("front page is not visible")?
                .0;
            page.evidence = FocusEvidence::Native;
            return Ok(page);
        }
        if candidates.len() != 1 {
            return Err("no unique visible page");
        }
        let mut page = candidates.pop().unwrap().0;
        page.evidence = FocusEvidence::Visible;
        Ok(page)
    }

    fn still_selected(&mut self, page: &FocusedPage) -> Result<bool> {
        let focus = self.focus(page)?;
        if !focus.visible {
            return Ok(false);
        }
        match page.evidence {
            FocusEvidence::Document => Ok(focus.focused),
            FocusEvidence::Native => Ok(self.native_front()?.as_deref() == Some(&page.target)),
            FocusEvidence::Visible => Ok(true),
        }
    }

    fn native_front(&mut self) -> Result<Option<String>> {
        let windows = match self.call("Browser.getWindows", json!({}), None) {
            Ok(windows) => windows,
            Err("CDP method unavailable") => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut visible = Vec::new();
        for window in windows["windows"].as_array().ok_or("missing windows")? {
            let active = window["isActive"].as_bool().ok_or("invalid window focus")?;
            if window["isVisible"]
                .as_bool()
                .ok_or("invalid window visibility")?
            {
                visible.push((window, active));
            }
        }
        let active: Vec<_> = visible.iter().filter(|(_, active)| *active).collect();
        let window = match active.as_slice() {
            [window] => window.0,
            [] if visible.len() == 1 => visible[0].0,
            _ => return Err("no unique front window"),
        };
        let window_id = window["windowId"].as_i64().ok_or("missing window id")?;
        let result = match self.call("Browser.getActiveTab", json!({"windowId":window_id}), None) {
            Ok(result) => result,
            Err("CDP method unavailable") => return Ok(None),
            Err(error) => return Err(error),
        };
        let tab = &result["tab"];
        if tab["windowId"] != window_id || tab["isActive"] != true || tab["isHidden"] != false {
            return Err("no visible front tab");
        }
        Ok(Some(
            tab["targetId"]
                .as_str()
                .ok_or("missing front target")?
                .to_owned(),
        ))
    }

    fn focus(&mut self, page: &FocusedPage) -> Result<Focus> {
        let result = self.call("Runtime.evaluate", json!({"expression": FOCUS,"contextId":page.context,"returnByValue":true,"timeout":100}), Some(&page.session))?;
        if result.get("exceptionDetails").is_some() {
            return Err("focus probe failed");
        }
        let value = &result["result"]["value"];
        Ok(Focus {
            focused: value["focused"].as_bool().ok_or("invalid focus result")?,
            visible: value["visible"]
                .as_bool()
                .ok_or("invalid visibility result")?,
        })
    }
}

fn discovery(http: &mut TimedStream, deadline: Instant) -> Result<Value> {
    let mut bytes = Vec::new();
    let mut body = None;
    loop {
        if let Some((start, len)) = body {
            if bytes.len() >= start + len {
                return serde_json::from_slice(&bytes[start..start + len])
                    .map_err(|_| "invalid discovery JSON");
            }
        } else if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = std::str::from_utf8(&bytes[..end]).map_err(|_| "invalid HTTP header")?;
            if !head.starts_with("HTTP/1.1 200 ")
                || head.to_ascii_lowercase().contains("transfer-encoding:")
            {
                return Err("unsupported discovery response");
            }
            let mut lengths = head
                .lines()
                .filter_map(|l| l.split_once(':'))
                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"));
            let len: usize = lengths
                .next()
                .ok_or("missing content length")?
                .1
                .trim()
                .parse()
                .map_err(|_| "invalid content length")?;
            if lengths.next().is_some() || len > LIMIT || end + 4 + len > LIMIT {
                return Err("discovery response too large or ambiguous");
            }
            body = Some((end + 4, len));
            continue;
        }
        if bytes.len() >= LIMIT {
            return Err("discovery response too large");
        }
        timeouts(http, deadline)?;
        let mut chunk = [0; 4096];
        let n = http.read(&mut chunk).map_err(|_| "discovery read failed")?;
        if n == 0 {
            return Err("truncated discovery response");
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
}

fn browser_path(url: &str) -> Result<&str> {
    let (_, path) = url
        .strip_prefix("ws://")
        .ok_or("invalid browser socket")?
        .split_once('/')
        .ok_or("missing socket path")?;
    let id = path
        .strip_prefix("devtools/browser/")
        .ok_or("not a browser socket")?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err("invalid browser id");
    }
    Ok(&url[url.len() - path.len() - 1..])
}

fn connect(port: u16, deadline: Instant) -> Result<TimedStream> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let socket =
        TcpStream::connect_timeout(&addr, remaining(deadline)?).map_err(|_| "CDP unavailable")?;
    socket
        .set_nodelay(true)
        .map_err(|_| "socket option failed")?;
    let socket = TimedStream {
        inner: socket,
        deadline,
    };
    timeouts(&socket, deadline)?;
    Ok(socket)
}

// Apply the absolute deadline before every underlying read/write, including
// WebSocket handshakes and fragmented frames that need several socket reads.
struct TimedStream {
    inner: TcpStream,
    deadline: Instant,
}

impl TimedStream {
    fn apply_timeouts(&self, deadline: Instant) -> io::Result<()> {
        let left = remaining(deadline)
            .map_err(|reason| io::Error::new(io::ErrorKind::TimedOut, reason))?;
        self.inner.set_read_timeout(Some(left))?;
        self.inner.set_write_timeout(Some(left))
    }
}

impl Read for TimedStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.apply_timeouts(self.deadline)?;
        self.inner.read(buf)
    }
}

impl Write for TimedStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.apply_timeouts(self.deadline)?;
        self.inner.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or("CDP deadline exceeded")
}

fn timeouts(socket: &TimedStream, deadline: Instant) -> Result<()> {
    socket.apply_timeouts(deadline).map_err(|error| {
        if error.kind() == io::ErrorKind::TimedOut {
            "CDP deadline exceeded"
        } else {
            "socket timeout failed"
        }
    })
}

#[cfg(test)]
mod tests;
