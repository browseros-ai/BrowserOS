use super::*;
use std::net::TcpListener;
use std::thread::{self, JoinHandle};

// A real HTTP/WebSocket peer catches selection, framing, and no-replay mistakes.
fn peer(focus: Vec<Vec<bool>>, lose_ack: bool) -> (u16, JoinHandle<Vec<Value>>) {
    peer_with_visibility(
        focus
            .into_iter()
            .map(|states| {
                states
                    .into_iter()
                    .map(|focused| (focused, focused))
                    .collect()
            })
            .collect(),
        None,
        lose_ack,
    )
}

struct Native {
    windows: Vec<Value>,
    tabs: Vec<Value>,
}

impl Native {
    fn front(targets: &[usize]) -> Self {
        Self {
            windows: vec![json!({"windows":[{"windowId":7,"isActive":true,"isVisible":true}]})],
            tabs: targets.iter().map(|id| json!({"tab":{"targetId":id.to_string(),"windowId":7,"isActive":true,"isHidden":false}})).collect(),
        }
    }
}

fn peer_with_visibility(
    focus: Vec<Vec<(bool, bool)>>,
    native: Option<Native>,
    lose_ack: bool,
) -> (u16, JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let thread = thread::spawn(move || {
        let (mut http, _) = listener.accept().unwrap();
        http.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            http.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let body =
            json!({"webSocketDebuggerUrl":"ws://untrusted.invalid:123/devtools/browser/test-123"})
                .to_string();
        write!(
            http,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        // Chromium keeps this socket open even when the client asks for close.
        let _keep_http_alive = http;
        let (socket, _) = listener.accept().unwrap();
        socket.set_nodelay(true).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut ws = tungstenite::accept(socket).unwrap();
        let mut transcript = Vec::new();
        let mut probes = vec![0; focus.len()];
        let mut window_probes = 0;
        let mut tab_probes = 0;
        while let Ok(Message::Text(text)) = ws.read() {
            let m: Value = serde_json::from_str(&text).unwrap();
            transcript.push(m.clone());
            let session = m["sessionId"]
                .as_str()
                .unwrap_or("0")
                .parse::<usize>()
                .unwrap();
            let result = match m["method"].as_str().unwrap() {
                "Target.getTargets" => {
                    json!({"targetInfos":(0..focus.len()).map(|i|json!({"type":"page","targetId":i.to_string(),"attached":true})).collect::<Vec<_>>()})
                }
                "Target.attachToTarget" => json!({"sessionId":m["params"]["targetId"]}),
                "Page.getFrameTree" => json!({"frameTree":{"frame":{"id":"main"}}}),
                "Page.createIsolatedWorld" => json!({"executionContextId":session+1}),
                "Runtime.evaluate" => {
                    assert_eq!(m["params"]["contextId"], session + 1);
                    let values = &focus[session];
                    let (focused, visible) = values[probes[session].min(values.len() - 1)];
                    probes[session] += 1;
                    let value = if m["params"]["expression"]
                        == "document.hasFocus() && document.visibilityState === 'visible'"
                    {
                        json!(focused && visible)
                    } else {
                        json!({"focused":focused,"visible":visible})
                    };
                    json!({"result":{"value":value}})
                }
                "Browser.getWindows" | "Browser.getActiveTab" => {
                    let Some(native) = &native else {
                        ws.send(Message::Text(json!({"id":m["id"],"error":{"code":-32601,"message":"Method not found"}}).to_string().into())).unwrap();
                        continue;
                    };
                    let (values, count) = if m["method"] == "Browser.getWindows" {
                        (&native.windows, &mut window_probes)
                    } else {
                        assert_eq!(m["params"]["windowId"], 7);
                        (&native.tabs, &mut tab_probes)
                    };
                    let result = values[(*count).min(values.len() - 1)].clone();
                    *count += 1;
                    result
                }
                "Input.insertText" => {
                    if lose_ack {
                        break;
                    }
                    json!({})
                }
                other => panic!("unexpected CDP mutation: {other}"),
            };
            // Unsolicited events and other clients' attached status must not confuse replies.
            ws.send(Message::Text(
                json!({"method":"Target.targetInfoChanged","params":{}})
                    .to_string()
                    .into(),
            ))
            .unwrap();
            ws.send(Message::Text(
                json!({"id":m["id"],"result":result}).to_string().into(),
            ))
            .unwrap();
        }
        transcript
    });
    (port, thread)
}

fn insertions(log: &[Value]) -> Vec<&Value> {
    log.iter()
        .filter(|m| m["method"] == "Input.insertText")
        .collect()
}

#[test]
fn targets_the_focused_page_and_preserves_exact_text() {
    let (port, server) = peer(vec![vec![false], vec![true]], false);
    let text = "héllo ÀÉÎÕÜ ñ ß 👋🏽 👨‍👩‍👧 नमस्ते 你好 日本語 한국어\n\"\\";
    insert_at(text, port).unwrap();
    let log = server.join().unwrap();
    let sent = insertions(&log);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["sessionId"], "1");
    assert_eq!(sent[0]["params"]["text"], text);
}

#[test]
fn absent_ambiguous_or_changed_focus_never_inserts() {
    for focus in [
        vec![vec![false]],
        vec![vec![true], vec![true]],
        vec![vec![true, false]],
    ] {
        let (port, server) = peer(focus, false);
        assert!(matches!(
            insert_at("secret", port),
            Err(DeliveryError::NotSent(_))
        ));
        assert!(insertions(&server.join().unwrap()).is_empty());
    }
}

#[test]
fn an_unacknowledged_insert_is_never_replayed() {
    let (port, server) = peer(vec![vec![true]], true);
    assert!(matches!(
        insert_at("secret", port),
        Err(DeliveryError::Uncertain)
    ));
    assert_eq!(insertions(&server.join().unwrap()).len(), 1);
    // A later explicit input independently discovers the current browser.
    let (port, server) = peer(vec![vec![true]], false);
    insert_at("next", port).unwrap();
    assert_eq!(insertions(&server.join().unwrap()).len(), 1);
}

#[test]
fn rejects_unbounded_target_scan_and_unsafe_socket_paths() {
    let (port, server) = peer(vec![vec![false]; MAX_PAGES + 1], false);
    assert!(matches!(
        insert_at("secret", port),
        Err(DeliveryError::NotSent("too many pages"))
    ));
    assert_eq!(server.join().unwrap().len(), 1);
    for url in [
        "wss://localhost/devtools/browser/x",
        "ws://localhost/devtools/page/x",
        "ws://localhost/devtools/browser/../../x",
        "ws://localhost/devtools/browser/x?query",
        "ws://localhost/devtools/browser/x\r\nHeader: y",
    ] {
        assert!(browser_path(url).is_err());
    }
}

#[test]
fn tests_that_two_visible_pages_use_the_document_with_focus() {
    let (port, server) =
        peer_with_visibility(vec![vec![(false, true)], vec![(true, true)]], None, false);
    insert_at("नमस्ते", port).unwrap();
    let log = server.join().unwrap();
    assert_eq!(insertions(&log)[0]["sessionId"], "1");
}

#[test]
fn tests_that_a_single_visible_page_can_type_without_document_focus() {
    let (port, server) =
        peer_with_visibility(vec![vec![(false, false)], vec![(false, true)]], None, false);
    insert_at("日本", port).unwrap();
    let log = server.join().unwrap();
    assert_eq!(insertions(&log)[0]["sessionId"], "1");
}

#[test]
fn tests_that_native_front_tab_resolves_emulated_focus_on_multiple_pages() {
    let (port, server) = peer_with_visibility(
        vec![vec![(true, true)]; 3],
        Some(Native::front(&[1])),
        false,
    );
    insert_at("secret", port).unwrap();
    let log = server.join().unwrap();
    let sent = insertions(&log);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["sessionId"], "1");
    assert_eq!(
        log.iter()
            .filter(|m| m["method"] == "Browser.getActiveTab")
            .count(),
        2
    );
}

#[test]
fn tests_that_an_uncertain_native_front_tab_never_receives_text() {
    let ambiguous = json!({"windows":[
        {"windowId":7,"isActive":true,"isVisible":true},
        {"windowId":8,"isActive":true,"isVisible":true}
    ]});
    for native in [
        Native::front(&[0, 1]),
        Native {
            windows: vec![json!({"windows":[]})],
            tabs: vec![],
        },
        Native {
            windows: vec![ambiguous],
            tabs: vec![],
        },
        Native {
            windows: Native::front(&[0]).windows,
            tabs: vec![json!({})],
        },
        Native::front(&[9]),
    ] {
        let (port, server) = peer_with_visibility(vec![vec![(true, true)]; 2], Some(native), false);
        assert!(matches!(
            insert_at("secret", port),
            Err(DeliveryError::NotSent(_))
        ));
        assert!(insertions(&server.join().unwrap()).is_empty());
    }
}

#[test]
fn tests_that_a_visible_fallback_is_rechecked_before_insertion() {
    let (port, server) =
        peer_with_visibility(vec![vec![(false, true), (false, false)]], None, false);
    assert!(matches!(
        insert_at("secret", port),
        Err(DeliveryError::NotSent(_))
    ));
    assert!(insertions(&server.join().unwrap()).is_empty());
}

fn timed_pair() -> (TimedStream, TcpStream) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (peer, _) = listener.accept().unwrap();
    peer.set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let stream = TimedStream {
        inner: client,
        deadline: Instant::now() - Duration::from_millis(1),
    };
    (stream, peer)
}

#[test]
fn tests_that_an_expired_read_deadline_is_a_timeout_and_reset_preserves_queued_data() {
    let (mut stream, mut peer) = timed_pair();
    peer.write_all(b"ready").unwrap();
    let mut received = [0; 5];
    assert_eq!(
        stream.read(&mut received).unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    stream.deadline = Instant::now() + Duration::from_secs(1);
    stream.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"ready");
}

#[test]
fn tests_that_an_expired_write_deadline_is_a_timeout_and_reset_does_not_replay_bytes() {
    let (mut stream, mut peer) = timed_pair();
    assert_eq!(
        stream.write(b"expired").unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert!(
        matches!(peer.read(&mut [0]), Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut))
    );
    stream.deadline = Instant::now() + Duration::from_secs(1);
    stream.write_all(b"fresh").unwrap();
    let mut received = [0; 5];
    peer.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"fresh");
}

#[test]
fn tests_that_a_live_deadline_does_not_disguise_a_broken_socket_as_timeout() {
    let (mut stream, _peer) = timed_pair();
    stream.deadline = Instant::now() + Duration::from_secs(1);
    stream.inner.shutdown(std::net::Shutdown::Write).unwrap();
    assert_eq!(
        stream.write(b"closed").unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
}
