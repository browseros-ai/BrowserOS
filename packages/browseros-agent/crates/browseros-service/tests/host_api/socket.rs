//! `host_service` on a real Unix socket: the peer's credentials come from the kernel, and the
//! token only from the `Authorization` header.

use crate::support::{BoxError, TestResult, harness};
use browseros_service::{HostAuth, HostToken, PeerRule, host_service};
use std::future::IntoFuture;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

const TOKEN: &str = "socket-test-token-0123456789abcdef";

#[tokio::test]
async fn the_host_socket_serves_only_requests_with_the_header_token() -> TestResult {
    let h = harness().await?;
    let socket = h.dir.path().join("host.sock");
    let listener = UnixListener::bind(&socket)?;
    let auth = HostAuth {
        token: HostToken::parse(TOKEN)?,
        peer: Some(PeerRule::from_socket(&socket)?),
    };
    let server =
        tokio::spawn(axum::serve(listener, host_service(h.routes.clone(), auth)).into_future());

    let refused = [
        ("/v1/runs".to_string(), None),
        (format!("/v1/runs?token={TOKEN}"), None),
        (format!("/v1/runs?access_token={TOKEN}"), None),
        (
            "/v1/runs".to_string(),
            Some("a-wrong-token-0123456789abcdefghij"),
        ),
        ("/v1/no-such-route".to_string(), None),
    ];
    for (uri, token) in refused {
        assert_eq!(status(&socket, &uri, token).await?, 401, "{uri} {token:?}");
    }
    // A token in the URL is refused even beside a good header: it would end up in logs.
    for uri in [
        format!("/v1/runs?token={TOKEN}"),
        format!("/v1/runs?tail=1&access_token={TOKEN}"),
    ] {
        assert_eq!(status(&socket, &uri, Some(TOKEN)).await?, 401, "{uri}");
    }
    assert_eq!(status(&socket, "/v1/runs", Some(TOKEN)).await?, 200);
    assert_eq!(
        status(&socket, "/v1/no-such-route", Some(TOKEN)).await?,
        404
    );
    server.abort();
    Ok(())
}

/// Sends one HTTP/1.1 GET over the socket, as `curl --unix-socket` does, and reads its status.
/// Every response carries `x-request-id`, refusals included.
async fn status(socket: &Path, uri: &str, token: Option<&str>) -> Result<u16, BoxError> {
    let mut stream = UnixStream::connect(socket).await?;
    let authorization = token
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    let request =
        format!("GET {uri} HTTP/1.1\r\nHost: neo\r\n{authorization}Connection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    assert!(
        response.to_ascii_lowercase().contains("\r\nx-request-id: "),
        "{response}"
    );
    Ok(response
        .split(' ')
        .nth(1)
        .ok_or("no status line")?
        .parse()?)
}

/// Served with plain `axum::serve` and no `host_service`, the routes refuse every request, even
/// one with the right token: forgetting the wrapper cannot expose them.
#[tokio::test]
async fn the_routes_served_without_the_host_auth_refuse_everything() -> TestResult {
    let h = harness().await?;
    let socket = h.dir.path().join("bare.sock");
    let listener = UnixListener::bind(&socket)?;
    let server = tokio::spawn(axum::serve(listener, h.routes.clone()).into_future());
    assert_eq!(status(&socket, "/v1/runs", None).await?, 401);
    assert_eq!(status(&socket, "/v1/runs", Some(TOKEN)).await?, 401);
    server.abort();
    Ok(())
}
