//! AWS event-stream frames, as a Bedrock ConverseStream reply carries them: a prelude with the
//! lengths and its CRC32, the headers, a JSON payload, and the message's CRC32.

/// An `event` frame of type `kind`.
pub fn event(kind: &str, payload: &serde_json::Value) -> Vec<u8> {
    frame(
        &[(":message-type", "event"), (":event-type", kind)],
        payload,
    )
}

/// An `exception` frame of type `kind`, as Bedrock sends a failure in the middle of a stream.
pub fn exception(kind: &str, payload: &serde_json::Value) -> Vec<u8> {
    frame(
        &[(":message-type", "exception"), (":exception-type", kind)],
        payload,
    )
}

fn frame(headers: &[(&str, &str)], payload: &serde_json::Value) -> Vec<u8> {
    let mut encoded = Vec::new();
    for (name, value) in headers
        .iter()
        .chain(&[(":content-type", "application/json")])
    {
        encoded.push(u8::try_from(name.len()).unwrap_or(u8::MAX));
        encoded.extend_from_slice(name.as_bytes());
        encoded.push(7);
        encoded.extend_from_slice(&u16::try_from(value.len()).unwrap_or(u16::MAX).to_be_bytes());
        encoded.extend_from_slice(value.as_bytes());
    }
    let data = payload.to_string().into_bytes();
    let total = u32::try_from(16 + encoded.len() + data.len()).unwrap_or(u32::MAX);
    let mut message = total.to_be_bytes().to_vec();
    message.extend_from_slice(
        &u32::try_from(encoded.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    let prelude = crc32(&message);
    message.extend_from_slice(&prelude.to_be_bytes());
    message.extend(encoded);
    message.extend(data);
    let checksum = crc32(&message);
    message.extend_from_slice(&checksum.to_be_bytes());
    message
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// What the loopback ConverseStream saw: each request's headers and JSON body.
pub type Seen = std::sync::Arc<std::sync::Mutex<Vec<(axum::http::HeaderMap, serde_json::Value)>>>;

/// One reply of the loopback ConverseStream: its status, headers and body.
pub struct Answer {
    pub status: axum::http::StatusCode,
    pub headers: Vec<(&'static str, &'static str)>,
    pub body: Vec<u8>,
}

impl Answer {
    /// A successful stream of `events`.
    pub fn events(events: &[(&'static str, serde_json::Value)]) -> Self {
        Self {
            status: axum::http::StatusCode::OK,
            headers: vec![("content-type", "application/vnd.amazon.eventstream")],
            body: events
                .iter()
                .flat_map(|(kind, payload)| event(kind, payload))
                .collect(),
        }
    }
}

/// A loopback ConverseStream: records each request and plays the next reply's events.
pub async fn serve(
    replies: Vec<Vec<(&'static str, serde_json::Value)>>,
) -> anyhow::Result<(String, Seen, tokio::task::JoinHandle<()>)> {
    serve_answers(
        replies
            .iter()
            .map(|events| Answer::events(events))
            .collect(),
    )
    .await
}

/// A loopback ConverseStream that plays `answers` in order, recording each request.
pub async fn serve_answers(
    answers: Vec<Answer>,
) -> anyhow::Result<(String, Seen, tokio::task::JoinHandle<()>)> {
    use axum::extract::State;
    use axum::response::IntoResponse;
    type Queue = std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<Answer>>>;
    let seen: Seen = Seen::default();
    let queue: Queue = std::sync::Arc::new(std::sync::Mutex::new(answers.into()));
    let app = axum::Router::new()
        .fallback(
            |State((seen, queue)): State<(Seen, Queue)>,
             headers: axum::http::HeaderMap,
             body: axum::body::Bytes| async move {
                let json = serde_json::from_slice(&body).unwrap_or_default();
                if let Ok(mut seen) = seen.lock() {
                    seen.push((headers, json));
                }
                let reply = queue.lock().ok().and_then(|mut queue| queue.pop_front());
                let Some(Answer {
                    status,
                    headers,
                    body,
                }) = reply
                else {
                    return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
                };
                let mut response = (status, body).into_response();
                for (name, value) in headers {
                    if let (Ok(name), Ok(value)) = (
                        axum::http::HeaderName::try_from(name),
                        axum::http::HeaderValue::try_from(value),
                    ) {
                        response.headers_mut().insert(name, value);
                    }
                }
                response
            },
        )
        .with_state((seen.clone(), queue));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((base, seen, server))
}
