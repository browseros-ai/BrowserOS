//! Bounded, authenticated reads from the child SDK's separate host listener.
//! Only metadata crosses into the host audit pipeline. Values and page contents
//! are deliberately absent from these types so projection cannot persist them.

use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

use super::{EngineUnavailable, NeoEngine};

#[derive(Clone)]
pub(super) struct HostApi {
    client: Client,
    endpoint: String,
    token: String,
}

/// The safe subset of an SDK operation used by cockpit child rows.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOperation {
    pub name: OperationName,
    pub tab_id: Option<i64>,
    pub started_at: i64,
    pub duration_ms: Option<i64>,
    pub outcome: Option<OperationOutcome>,
}

/// Translate SDK calls into the existing cockpit vocabulary. Unknown method
/// names can be supplied by scripts to a failed dispatch, so never persist them.
#[derive(Deserialize)]
pub enum OperationName {
    #[serde(rename = "page.goto", alias = "frame.goto")]
    Navigate,
    #[serde(rename = "context.newPage", alias = "task.newPage")]
    NewPage,
    #[serde(rename = "page.goBack")]
    Back,
    #[serde(rename = "page.goForward")]
    Forward,
    #[serde(rename = "page.reload")]
    Reload,
    #[serde(
        rename = "locator.click",
        alias = "locator.dblclick",
        alias = "page.mouse.click"
    )]
    Click,
    #[serde(
        rename = "locator.fill",
        alias = "locator.type",
        alias = "locator.pressSequentially",
        alias = "page.keyboard.type",
        alias = "page.keyboard.insertText"
    )]
    Fill,
    #[serde(rename = "locator.press", alias = "page.keyboard.press")]
    Press,
    #[serde(rename = "locator.hover", alias = "locator.focus")]
    Hover,
    #[serde(rename = "locator.selectOption")]
    Select,
    #[serde(
        rename = "page.title",
        alias = "page.content",
        alias = "page.snapshot",
        alias = "locator.innerText",
        alias = "locator.textContent",
        alias = "locator.count",
        alias = "page.evaluate",
        alias = "locator.evaluate"
    )]
    Read,
    #[serde(rename = "page.screenshot", alias = "locator.screenshot")]
    Screenshot,
    #[serde(rename = "page.close")]
    Close,
    #[serde(other)]
    Other,
}

impl OperationName {
    #[must_use]
    pub fn audit_name(&self) -> &'static str {
        match self {
            Self::Navigate => "nav.goto",
            Self::NewPage => "pages.newPage",
            Self::Back => "nav.back",
            Self::Forward => "nav.forward",
            Self::Reload => "nav.reload",
            Self::Click => "input.click",
            Self::Fill => "input.fill",
            Self::Press => "input.press",
            Self::Hover => "input.hover",
            Self::Select => "input.selectOption",
            Self::Read => "read",
            Self::Screenshot => "screenshot",
            Self::Close => "pages.close",
            Self::Other => "sdk.operation",
        }
    }
}

/// Unknown future outcomes decode without failing the metadata read.
/// No SDK error message is admitted to the audit projection.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationOutcome {
    Ok,
    Error,
    Cancelled,
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
struct RunRecord {
    operations: Vec<RunOperation>,
}

impl HostApi {
    pub(super) fn new(port: u16, token: String) -> Result<Self, EngineUnavailable> {
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_millis(250))
                .timeout(Duration::from_millis(2500))
                .build()
                .map_err(|_| EngineUnavailable::new("could not create the SDK host client"))?,
            endpoint: format!("http://127.0.0.1:{port}"),
            token,
        })
    }

    async fn operations(&self, run_id: &str) -> Result<Vec<RunOperation>, EngineUnavailable> {
        let mut url = reqwest::Url::parse(&self.endpoint)
            .map_err(|_| EngineUnavailable::new("invalid SDK host endpoint"))?;
        url.path_segments_mut()
            .map_err(|()| EngineUnavailable::new("invalid SDK host endpoint"))?
            .extend(["v1", "runs", run_id]);
        url.set_query(Some("wait=2&internal=false&fields=summary"));
        let mut response = self
            .client
            .get(url)
            .bearer_auth(&self.token)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            // Error details can include a bearer-bearing request. Log only this
            // fixed diagnostic at the caller, never response bodies or values.
            .map_err(|_| EngineUnavailable::new("SDK run metadata could not be read"))?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| EngineUnavailable::new("SDK run metadata read failed"))?
        {
            if bytes.len().saturating_add(chunk.len()) > 8 * 1024 * 1024 {
                return Err(EngineUnavailable::new(
                    "SDK run metadata exceeded its limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice::<RunRecord>(&bytes)
            .map(|run| run.operations)
            .map_err(|_| EngineUnavailable::new("SDK run metadata had an invalid shape"))
    }
}

impl NeoEngine {
    /// A missing or failed host listener never changes the completed run reply.
    pub async fn operations(&self, run_id: &str) -> Vec<RunOperation> {
        let host = self
            .inner
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .host
            .clone();
        let Some(host) = host else {
            tracing::warn!("SDK run metadata unavailable without a host connection");
            return Vec::new();
        };
        match host.operations(run_id).await {
            Ok(operations) => operations,
            Err(error) => {
                tracing::warn!(%error, "SDK run metadata projection skipped");
                Vec::new()
            }
        }
    }
}
