//! One background Strata client with bounded connect/list calls and startup backoff.
//! Only immutable metadata is cached; failed clients are dropped before another
//! attempt, and shutdown owns closing the successful transport.

use super::{
    ProxyState, Shared, Snapshot,
    client::{Client, Strata, TIMEOUT},
};
use rmcp::{
    RoleClient, ServiceExt,
    model::{ClientInfo, Implementation, Tool},
    service::{Peer, RunningService},
    transport::StreamableHttpClientTransport,
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::time::{Instant, timeout};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(super) struct ProxySession {
    pub peer: Peer<RoleClient>,
    pub tools: Vec<Tool>,
}

pub(super) async fn run(
    client: Client,
    shared: Arc<Shared>,
    cancel: CancellationToken,
    delays: Vec<Duration>,
) {
    for attempt in 0..=delays.len() {
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => return,
            result = connect(&client, &shared) => result,
        };
        match result {
            Ok((mut service, tools, strata_id)) => {
                {
                    let mut snapshot = shared.snapshot.write().unwrap_or_else(|e| e.into_inner());
                    *snapshot = Snapshot {
                        status: json!({"state":"ready","toolCount":tools.len()}),
                        proxy: ProxyState::Connected {
                            fingerprint: strata_id,
                        },
                        session: Some(ProxySession {
                            peer: service.peer().clone(),
                            tools,
                        }),
                    };
                }
                cancel.cancelled().await;
                shared
                    .snapshot
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .session = None;
                // rmcp closes the HTTP session (DELETE) and joins its worker.
                // Shutdown remains bounded even if the upstream vanished.
                let _ = service.close_with_timeout(Duration::from_secs(2)).await;
                return;
            }
            Err(error) => {
                if let Some(delay) = delays.get(attempt) {
                    shared
                        .snapshot
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .status = json!({
                        "state":"retrying","attempt":attempt+1,"nextRetryMs":delay.as_millis(),"error":error
                    });
                    tracing::info!(attempt = attempt + 1, %error, "retrying Klavis proxy connection");
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = tokio::time::sleep(*delay) => {}
                    }
                    shared
                        .snapshot
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .status = json!({"state":"connecting"});
                } else {
                    let mut snapshot = shared.snapshot.write().unwrap_or_else(|e| e.into_inner());
                    snapshot.status = json!({"state":"unavailable","error":error});
                    snapshot.proxy = ProxyState::Disconnected;
                    tracing::warn!(%error, "Klavis proxy unavailable after startup retries");
                }
            }
        }
    }
}

async fn connect(
    client: &Client,
    shared: &Shared,
) -> Result<(RunningService<RoleClient, ClientInfo>, Vec<Tool>, String), String> {
    let strata = metadata(client, shared).await?;
    let transport = StreamableHttpClientTransport::from_uri(strata.strata_server_url.clone());
    let mut info = ClientInfo::default();
    info.client_info = Implementation::new("browseros-klavis-proxy", "1.0.0");
    let service = timeout(TIMEOUT, info.serve(transport))
        .await
        .map_err(|_| "Klavis connect timed out".to_owned())?
        .map_err(|e| e.to_string())?;
    let tools = timeout(TIMEOUT, service.list_all_tools())
        .await
        .map_err(|_| "Klavis listTools timed out".to_owned())?
        .map_err(|e| e.to_string())?;
    Ok((service, tools, strata.strata_id))
}

async fn metadata(client: &Client, shared: &Shared) -> Result<Strata, String> {
    // One fetch is in flight per installation. Invalidation takes the same lock,
    // so an auth change cannot be overwritten by an older fetch completing late.
    let mut cache = shared.cache.lock().await;
    if let Some((strata, _)) = cache
        .as_ref()
        .filter(|(_, expires)| Instant::now() < *expires)
    {
        return Ok(strata.clone());
    }
    let names: Vec<_> = super::Klavis::catalog().iter().map(|s| s.name).collect();
    let mut strata = client.create(&names).await.map_err(|e| e.to_string())?;
    // Auth URLs are deliberately never retained in the metadata cache (TS parity).
    strata.oauth_urls.clear();
    strata.api_key_urls.clear();
    *cache = Some((strata.clone(), Instant::now() + Duration::from_secs(3600)));
    Ok(strata)
}
