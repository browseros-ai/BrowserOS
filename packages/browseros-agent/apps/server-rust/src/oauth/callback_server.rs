//! Lazy, independent loopback listener for the registered Codex redirect URI. The router
//! holds only a weak manager reference, so dropping the application releases the listener;
//! graceful cancellation finishes the current HTML response before closing connections.

use super::{OAuthError, TokenManager};
use axum::{
    Router,
    extract::{Request, State},
    response::{IntoResponse, Response},
};
use http::{StatusCode, header};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Default)]
pub struct CallbackServer {
    running: Mutex<Option<Running>>,
}

#[derive(Debug)]
struct Running {
    cancel: CancellationToken,
    task: JoinHandle<()>,
    port: u16,
}

#[derive(Clone)]
struct CallbackState {
    manager: Weak<TokenManager>,
    cancel: CancellationToken,
}

impl CallbackServer {
    pub const PORT: u16 = 1455;

    pub fn new() -> Self {
        Self::default()
    }

    /// Serializes lazy binds, including cancellation of another instance. Port zero is used
    /// only by hermetic tests; authorization uses the actual bound port in that case.
    pub(super) async fn ensure_running(
        &self,
        manager: &Arc<TokenManager>,
        port: u16,
    ) -> Result<u16, OAuthError> {
        let mut running = self.running.lock().await;
        if let Some(server) = running
            .as_ref()
            .filter(|s| !s.cancel.is_cancelled() && !s.task.is_finished())
        {
            return Ok(server.port);
        }
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        for attempt in 0..5 {
            match TcpListener::bind(addr).await {
                Ok(listener) => {
                    let actual_port = listener.local_addr().map_err(upstream)?.port();
                    let cancel = CancellationToken::new();
                    let router = Router::new().fallback(callback).with_state(CallbackState {
                        manager: Arc::downgrade(manager),
                        cancel: cancel.clone(),
                    });
                    let stopped = cancel.clone();
                    let task = tokio::spawn(async move {
                        if let Err(error) = axum::serve(listener, router)
                            .with_graceful_shutdown(stopped.cancelled_owned())
                            .await
                        {
                            tracing::warn!(%error, "OAuth callback listener failed");
                        }
                    });
                    *running = Some(Running {
                        cancel,
                        task,
                        port: actual_port,
                    });
                    return Ok(actual_port);
                }
                Err(_) => {
                    if attempt == 0 {
                        // The fixed registration is shared with Codex CLI and other BrowserOS
                        // instances. Ask the holder to leave once, then retry the bind five times.
                        let _ = manager
                            .http
                            .get(format!("http://127.0.0.1:{port}/cancel"))
                            .timeout(Duration::from_secs(2))
                            .send()
                            .await;
                    }
                    if attempt < 4 {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                    }
                }
            }
        }
        Err(OAuthError::Unavailable(format!(
            "OAuth callback port {port} is in use by another process. Close other BrowserOS instances or CLI tools and try again."
        )))
    }

    pub async fn stop(&self) {
        if let Some(server) = self.running.lock().await.take() {
            // Do not join here: successful callbacks call stop from inside this very server.
            // Axum drops its accept listener and drains that response on cancellation.
            server.cancel.cancel();
        }
    }
}

impl Drop for CallbackServer {
    fn drop(&mut self) {
        if let Some(server) = self.running.get_mut().take() {
            server.cancel.cancel();
        }
    }
}

async fn callback(State(state): State<CallbackState>, request: Request) -> Response {
    match request.uri().path() {
        "/cancel" => {
            state.cancel.cancel();
            plain_text(StatusCode::OK, "Login cancelled")
        }
        "/auth/callback" => {
            // URLSearchParams.get() returns the first duplicate, unlike collect::<HashMap>().
            let mut query: HashMap<String, String> = HashMap::new();
            for (key, value) in
                url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
            {
                query
                    .entry(key.into_owned())
                    .or_insert_with(|| value.into_owned());
            }
            let error = query.get("error").filter(|s| !s.is_empty());
            let result = if let Some(error) = error {
                Err(query
                    .get("error_description")
                    .filter(|s| !s.is_empty())
                    .unwrap_or(error)
                    .clone())
            } else if let (Some(code), Some(oauth_state)) = (
                query.get("code").filter(|s| !s.is_empty()),
                query.get("state").filter(|s| !s.is_empty()),
            ) {
                match state.manager.upgrade() {
                    Some(manager) => manager
                        .handle_callback(code, oauth_state)
                        .await
                        .map_err(|e| e.to_string()),
                    None => Err("OAuth callback server not initialized".to_owned()),
                }
            } else {
                Err("Missing authorization code or state".to_owned())
            };
            let html = match result {
                Ok(()) => include_str!("success.html").to_owned(),
                Err(error) => include_str!("error.html").replace(
                    "${escaped}",
                    &error
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;"),
                ),
            };
            ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response()
        }
        _ => plain_text(StatusCode::NOT_FOUND, "Not found"),
    }
}

fn plain_text(status: StatusCode, text: &'static str) -> Response {
    // Bun.serve infers this exact content type for new Response(string).
    (
        status,
        [(header::CONTENT_TYPE, "text/plain;charset=utf-8")],
        text,
    )
        .into_response()
}

fn upstream(error: impl std::fmt::Display) -> OAuthError {
    OAuthError::Upstream(error.to_string())
}
