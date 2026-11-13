//! Serves host routes behind the host auth. The app's `HostListener` binds each socket and
//! calls `axum::serve(listener, host_service(routes, auth))` once per bind.
//!
//! The auth fails closed. The peer facts come from the connection ([`PeerInfo`]), which axum
//! attaches only to a server built with `into_make_service_with_connect_info`, and
//! [`host_service`] is what builds it that way. Routes served any other way carry no
//! `PeerInfo`, so every request is refused instead of skipping the peer check.
//!
//! The peer rule runs before the token is read, so a 401 can never confirm a guessed token to
//! a process the rule keeps out.
//!
//! An admitted request carries an [`Authenticated`] marker that only this module can make. The
//! router itself refuses any request without it, so routes served with no auth at all (plain
//! `axum::serve(listener, service.router())`) refuse everything too.

use crate::api_error::ApiError;
use crate::host_auth::{self, HostAuth, PeerRule};
use crate::response_id;
use axum::Router;
use axum::extract::connect_info::{Connected, IntoMakeServiceWithConnectInfo};
use axum::extract::rejection::ExtensionRejection;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, HeaderValue};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::serve::IncomingStream;
use std::sync::Arc;
use tokio::net::{TcpListener, UnixListener};

/// Who is on the other end of one connection, as the OS reported it when it was accepted.
#[derive(Clone, Copy, Debug)]
pub struct PeerInfo(Peer);

#[derive(Clone, Copy, Debug)]
enum Peer {
    /// TCP carries no identity: the token is the only check.
    Tcp,
    /// A Unix connection, with the peer's credentials, or `None` when the OS gave none.
    Unix(Option<UnixPeer>),
}

/// A Unix peer's credentials: `SO_PEERCRED` on Linux, `LOCAL_PEERCRED` on macOS.
#[derive(Clone, Copy, Debug)]
struct UnixPeer {
    uid: u32,
    gid: u32,
    pid: Option<i32>,
}

impl PeerInfo {
    /// A TCP peer, for tests that layer `MockConnectInfo` over [`host_router`]. A TCP peer
    /// still needs the bearer token.
    #[doc(hidden)]
    #[must_use]
    pub const fn tcp() -> Self {
        Self(Peer::Tcp)
    }
}

impl Connected<IncomingStream<'_, UnixListener>> for PeerInfo {
    fn connect_info(stream: IncomingStream<'_, UnixListener>) -> Self {
        let credentials = stream.io().peer_cred().ok().map(|cred| UnixPeer {
            uid: cred.uid(),
            gid: cred.gid(),
            pid: cred.pid(),
        });
        Self(Peer::Unix(credentials))
    }
}

impl Connected<IncomingStream<'_, TcpListener>> for PeerInfo {
    fn connect_info(_stream: IncomingStream<'_, TcpListener>) -> Self {
        Self(Peer::Tcp)
    }
}

/// Proof that [`host_service`]'s auth admitted this request. The router refuses requests
/// without it. The private field means nothing outside this module can forge one.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Authenticated(());

/// `routes` behind the host auth, as the service `axum::serve` takes for one bound socket.
/// Every request passes the peer rule (on a Unix socket) and the bearer token, or gets the
/// contract's 401 with a request id.
#[must_use]
pub fn host_service(
    routes: Router,
    auth: HostAuth,
) -> IntoMakeServiceWithConnectInfo<Router, PeerInfo> {
    guarded(routes, auth).into_make_service_with_connect_info::<PeerInfo>()
}

/// `routes` with the host auth in front, for a server that attaches [`PeerInfo`] itself.
/// [`host_service`] is the usual way to serve them. Served with no connect info, every request
/// is refused.
pub fn host_router(routes: Router, auth: HostAuth) -> Router {
    guarded(routes, auth)
}

fn guarded(routes: Router, auth: HostAuth) -> Router {
    routes.layer(middleware::from_fn_with_state(Arc::new(auth), admit))
}

async fn admit(
    State(auth): State<Arc<HostAuth>>,
    peer: Result<ConnectInfo<PeerInfo>, ExtensionRejection>,
    mut request: Request,
    next: Next,
) -> Response {
    let verdict = match peer {
        Ok(ConnectInfo(PeerInfo(peer))) => {
            check(&auth, peer, request.uri().query(), request.headers()).await
        }
        Err(_) => {
            tracing::error!("a host request has no peer info; serve the routes with host_service");
            Err("no peer information for this connection")
        }
    };
    match verdict {
        Ok(()) => {
            request.extensions_mut().insert(Authenticated(()));
            next.run(request).await
        }
        Err(reason) => {
            tracing::warn!(reason, "host request refused");
            let mut response = ApiError::unauthorized(reason).into_response();
            response
                .headers_mut()
                .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            response_id::stamp_alone(response)
        }
    }
}

/// `Ok` to serve the request, or why it is refused.
async fn check(
    auth: &HostAuth,
    peer: Peer,
    query: Option<&str>,
    headers: &HeaderMap,
) -> Result<(), &'static str> {
    match peer {
        Peer::Tcp => {}
        Peer::Unix(None) => return Err("the socket gave no peer credentials"),
        Peer::Unix(Some(peer)) => {
            let Some(rule) = auth.peer else {
                return Err("this socket has no peer rule");
            };
            if !peer_admitted(rule, peer).await {
                return Err("this process may not use the host socket");
            }
        }
    }
    // A token in a URL ends up in logs and shell history. Refuse the request outright, even with
    // a good header, so a client that sends one finds out at once.
    if query.is_some_and(host_auth::has_query_token) {
        return Err("a token in the URL is refused; send it in the Authorization header");
    }
    let token = headers
        .get(AUTHORIZATION)
        .and_then(host_auth::bearer)
        .ok_or("missing bearer token")?;
    if auth.token.matches(token) {
        Ok(())
    } else {
        Err("wrong bearer token")
    }
}

/// The peer rule. The supplementary groups are read only when the uid and the primary gid do
/// not already admit the peer, so most requests never touch `/proc`.
async fn peer_admitted(rule: PeerRule, peer: UnixPeer) -> bool {
    rule.admits(peer.uid, peer.gid, &[])
        || rule.admits(peer.uid, peer.gid, &supplementary_groups(peer.pid).await)
}

/// The peer's supplementary groups, on Linux; macOS has no `/proc`. The pid is the one the OS
/// reported when the connection was accepted. A process that has exited since may have its
/// pid reused, but only processes the socket's mode already let connect are ever asked about.
/// A read that fails adds no group.
async fn supplementary_groups(pid: Option<i32>) -> Vec<u32> {
    let Some(pid) = pid.filter(|_| cfg!(target_os = "linux")) else {
        return Vec::new();
    };
    match tokio::fs::read_to_string(format!("/proc/{pid}/status")).await {
        Ok(status) => host_auth::supplementary_groups(&status),
        Err(error) => {
            tracing::warn!(%error, pid, "could not read a host peer's groups");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Peer, PeerInfo, UnixPeer, guarded};
    use crate::host_auth::{HostAuth, HostToken, PeerRule};
    use crate::response_id::REQUEST_ID;
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::extract::connect_info::MockConnectInfo;
    use axum::http::{Request, StatusCode, header};
    use axum::routing::get;
    use serde_json::Value;
    use std::error::Error;
    use tower::ServiceExt;

    const TOKEN: &str = "a-test-token-0123456789-abcdefghij";
    const NEO_UID: u32 = 501;
    const SOCKET_GROUP: u32 = 77;

    type TestResult = Result<(), Box<dyn Error>>;

    fn auth(peer: Option<PeerRule>) -> Result<HostAuth, Box<dyn Error>> {
        Ok(HostAuth {
            token: HostToken::parse(TOKEN)?,
            peer,
        })
    }

    fn socket_rule() -> PeerRule {
        PeerRule {
            uid: NEO_UID,
            gid: SOCKET_GROUP,
        }
    }

    fn unix_peer(uid: u32, gid: u32) -> PeerInfo {
        PeerInfo(Peer::Unix(Some(UnixPeer {
            uid,
            gid,
            pid: None,
        })))
    }

    /// One protected route behind the auth, connected as `peer` (or with no peer info).
    fn served(auth: HostAuth, peer: Option<PeerInfo>) -> Router {
        let routes = guarded(
            Router::new().route("/v1/runs", get(|| async { "runs" })),
            auth,
        );
        match peer {
            Some(peer) => routes.layer(MockConnectInfo(peer)),
            None => routes,
        }
    }

    async fn status_of(
        router: Router,
        uri: &str,
        token: Option<&str>,
    ) -> Result<StatusCode, Box<dyn Error>> {
        let mut request = Request::builder().uri(uri);
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let response = router.oneshot(request.body(Body::empty())?).await?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED {
            let id = response
                .headers()
                .get(REQUEST_ID)
                .and_then(|id| id.to_str().ok())
                .map(str::to_string);
            assert_eq!(
                response
                    .headers()
                    .get(header::WWW_AUTHENTICATE)
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer")
            );
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await?)?;
            assert_eq!(body["code"], "unauthorized");
            assert!(id.is_some(), "a refusal has a request id");
            assert_eq!(body["requestId"].as_str(), id.as_deref());
        }
        Ok(status)
    }

    #[tokio::test]
    async fn a_request_without_the_right_bearer_token_is_refused() -> TestResult {
        let peer = Some(unix_peer(NEO_UID, SOCKET_GROUP));
        let router =
            || -> Result<Router, Box<dyn Error>> { Ok(served(auth(Some(socket_rule()))?, peer)) };
        assert_eq!(
            status_of(router()?, "/v1/runs", None).await?,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status_of(
                router()?,
                "/v1/runs",
                Some("not-the-token-0123456789-abcdefghij")
            )
            .await?,
            StatusCode::UNAUTHORIZED
        );
        for query in [format!("token={TOKEN}"), format!("access_token={TOKEN}")] {
            assert_eq!(
                status_of(router()?, &format!("/v1/runs?{query}"), None).await?,
                StatusCode::UNAUTHORIZED,
                "a token in the query is ignored"
            );
        }
        assert_eq!(
            status_of(router()?, "/v1/runs", Some(TOKEN)).await?,
            StatusCode::OK
        );
        Ok(())
    }

    #[tokio::test]
    async fn routes_served_without_peer_info_refuse_every_request() -> TestResult {
        let router = served(auth(Some(socket_rule()))?, None);
        assert_eq!(
            status_of(router, "/v1/runs", Some(TOKEN)).await?,
            StatusCode::UNAUTHORIZED,
            "fail closed, even with the right token"
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_unix_peer_must_pass_the_peer_rule_before_its_token_counts() -> TestResult {
        let cases = [
            (unix_peer(NEO_UID, 20), StatusCode::OK, "Neo's own uid"),
            (
                unix_peer(1001, SOCKET_GROUP),
                StatusCode::OK,
                "the socket's group",
            ),
            (unix_peer(1001, 20), StatusCode::UNAUTHORIZED, "neither"),
            (
                PeerInfo(Peer::Unix(None)),
                StatusCode::UNAUTHORIZED,
                "no credentials",
            ),
            (PeerInfo(Peer::Tcp), StatusCode::OK, "TCP: the token alone"),
        ];
        for (peer, expected, case) in cases {
            let router = served(auth(Some(socket_rule()))?, Some(peer));
            assert_eq!(
                status_of(router, "/v1/runs", Some(TOKEN)).await?,
                expected,
                "{case}"
            );
        }
        let no_rule = served(auth(None)?, Some(unix_peer(NEO_UID, SOCKET_GROUP)));
        assert_eq!(
            status_of(no_rule, "/v1/runs", Some(TOKEN)).await?,
            StatusCode::UNAUTHORIZED,
            "a Unix socket served without its rule"
        );
        Ok(())
    }
}
