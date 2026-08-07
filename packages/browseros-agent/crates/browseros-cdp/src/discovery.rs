use crate::{CdpError, ConnectOptions};
use serde::Deserialize;
use std::time::Duration;
use tokio::time::sleep;
use url::Url;

const LOOPBACK_HOSTS: &[&str] = &["127.0.0.1", "localhost", "::1"];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionResponse {
    web_socket_debugger_url: String,
}

pub async fn discover_websocket_url(
    opts: &ConnectOptions,
    last_successful_host: Option<String>,
) -> Result<(Url, String), CdpError> {
    let client = reqwest::Client::builder()
        .timeout(opts.connect_timeout)
        .build()
        .map_err(|err| CdpError::Discovery(err.to_string()))?;

    let hosts = candidate_hosts(opts.host.as_deref(), last_successful_host.as_deref());
    let mut last_error = None;
    for attempt in 0..opts.connect_max_retries {
        for host in &hosts {
            match try_discover(&client, opts.port, host, opts.connect_timeout).await {
                Ok(url) => return Ok((url, (*host).to_string())),
                Err(err) => last_error = Some(err),
            }
        }
        if attempt + 1 < opts.connect_max_retries {
            sleep(opts.connect_retry_delay).await;
        }
    }

    Err(last_error.unwrap_or_else(|| CdpError::Discovery("no loopback hosts to try".to_string())))
}

async fn try_discover(
    client: &reqwest::Client,
    port: u16,
    host: &str,
    timeout: Duration,
) -> Result<Url, CdpError> {
    let display_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let endpoint = format!("http://{display_host}:{port}/json/version");
    let response = tokio::time::timeout(timeout, client.get(endpoint).send())
        .await
        .map_err(|_| CdpError::Discovery(format!("timed out discovering CDP on {host}:{port}")))?
        .map_err(|err| CdpError::Discovery(err.to_string()))?
        .error_for_status()
        .map_err(|err| CdpError::Discovery(err.to_string()))?
        .json::<VersionResponse>()
        .await
        .map_err(|err| CdpError::Discovery(err.to_string()))?;

    let mut url = Url::parse(&response.web_socket_debugger_url)
        .map_err(|err| CdpError::Discovery(err.to_string()))?;
    let rewritten_host = host.trim_start_matches('[').trim_end_matches(']');
    url.set_host(Some(rewritten_host))
        .map_err(|_| CdpError::Discovery(format!("invalid debugger host {rewritten_host}")))?;
    Ok(url)
}

fn candidate_hosts(explicit: Option<&str>, last_successful: Option<&str>) -> Vec<&'static str> {
    let mut hosts = Vec::new();
    push_host(&mut hosts, last_successful);
    push_host(&mut hosts, explicit);
    for host in LOOPBACK_HOSTS {
        push_host(&mut hosts, Some(host));
    }
    hosts
}

fn push_host(hosts: &mut Vec<&'static str>, host: Option<&str>) {
    let Some(host) = host else {
        return;
    };
    let normalized = host.trim_start_matches('[').trim_end_matches(']');
    let known = LOOPBACK_HOSTS
        .iter()
        .find(|candidate| **candidate == normalized);
    if let Some(known) = known
        && !hosts.iter().any(|existing| existing == known)
    {
        hosts.push(known);
    }
}
