//! The engine's host API, on the loopback TCP listener the parent turned on with
//! `CLAW_HOST_TCP` and a bearer token only the parent and the child can read. Two reads only:
//! the tab census (`GET /v1/tabs`) and a run's picture blobs (`GET /v1/runs/{run}/blobs/{id}`).
//! Redirects are never followed, so the token never leaves for another address, and every body
//! is read against a size bound.

use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

#[derive(Clone)]
pub(super) struct Host {
    client: Client,
    endpoint: String,
    token: String,
}

/// One tab of the census. `taskId` is the task holding the tab now (the SDK's in-memory task
/// book); the historical `taskIds` are not read, because a task that merely acted on a user's
/// tab is named there too.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CensusTab {
    pub(super) tab_id: i64,
    #[serde(default)]
    pub(super) task_id: Option<String>,
}

#[derive(Deserialize)]
struct TabList {
    items: Vec<CensusTab>,
}

/// A census of a thousand tabs is well under this.
const CENSUS_LIMIT: usize = 4 * 1024 * 1024;

impl Host {
    pub(super) fn new(port: u16, token: String) -> Result<Self, String> {
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_millis(500))
            .build()
            .map_err(|error| format!("building the host client: {error}"))?;
        Ok(Self {
            client,
            endpoint: format!("http://127.0.0.1:{port}"),
            token,
        })
    }

    pub(super) async fn tabs(&self) -> Result<Vec<CensusTab>, String> {
        let bytes = self
            .get(&["v1", "tabs"], CENSUS_LIMIT, Duration::from_secs(3))
            .await?;
        serde_json::from_slice::<TabList>(&bytes)
            .map(|list| list.items)
            .map_err(|error| format!("the tab census has an unexpected shape: {error}"))
    }

    pub(super) async fn blob(
        &self,
        run_id: &str,
        blob_id: &str,
        limit: usize,
    ) -> Result<Vec<u8>, String> {
        self.get(
            &["v1", "runs", run_id, "blobs", blob_id],
            limit,
            Duration::from_secs(15),
        )
        .await
    }

    /// The runs the engine has in flight (`GET /v1/runs?status=running`).
    #[cfg(feature = "test-support")]
    pub(super) async fn running_runs(&self) -> Result<usize, String> {
        #[derive(Deserialize)]
        struct RunList {
            items: Vec<serde_json::Value>,
        }
        let bytes = self
            .get_query(
                &["v1", "runs"],
                Some("status=running&limit=100"),
                CENSUS_LIMIT,
                Duration::from_secs(3),
            )
            .await?;
        serde_json::from_slice::<RunList>(&bytes)
            .map(|list| list.items.len())
            .map_err(|error| format!("the run list has an unexpected shape: {error}"))
    }

    async fn get(&self, path: &[&str], limit: usize, timeout: Duration) -> Result<Vec<u8>, String> {
        self.get_query(path, None, limit, timeout).await
    }

    async fn get_query(
        &self,
        path: &[&str],
        query: Option<&str>,
        limit: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, String> {
        let mut url = reqwest::Url::parse(&self.endpoint)
            .map_err(|error| format!("the host address does not parse: {error}"))?;
        url.path_segments_mut()
            .map_err(|()| "the host address cannot take a path".to_owned())?
            .extend(path);
        url.set_query(query);
        let mut response = self
            .client
            .get(url)
            .bearer_auth(&self.token)
            .timeout(timeout)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            // reqwest's error can name the request; it never carries the header, but say only
            // the status so nothing about the token reaches a log.
            .map_err(|error| match error.status() {
                Some(status) => format!("the host API answered {status}"),
                None => "the host API did not answer".to_owned(),
            })?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "the host API's reply broke off".to_owned())?
        {
            if bytes.len() + chunk.len() > limit {
                return Err(format!("the host API's reply is over {limit} bytes"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_census_reads_the_holder_and_ignores_the_rest() -> Result<(), serde_json::Error> {
        // The shape the engine answered in probe p1, with one held tab added.
        let body = r#"{"items":[
            {"tabId":1016412005,"targetId":"CE1B","url":"about:blank","title":"about:blank","taskIds":[],"owner":"neo"},
            {"tabId":1016412006,"targetId":"331A","url":"http://127.0.0.1/user","title":"Probe user","taskIds":["c1"]},
            {"tabId":1016412009,"targetId":"5E17","url":"about:blank","title":"","taskIds":["c1"],"taskId":"c1"}
        ]}"#;
        let list: TabList = serde_json::from_str(body)?;
        let held: Vec<_> = list
            .items
            .iter()
            .filter(|tab| tab.task_id.as_deref() == Some("c1"))
            .map(|tab| tab.tab_id)
            .collect();
        assert_eq!(held, vec![1016412009]);
        Ok(())
    }
}
