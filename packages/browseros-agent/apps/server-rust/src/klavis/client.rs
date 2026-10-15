//! Klavis proxy requests and upstream normalization. Auth URLs are data: API keys
//! always go back through the proxy, never to a host supplied by the requesting client.

use super::{Integration, KlavisError};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

pub(super) const TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const BASE_URL: &str = "https://llm.browseros.com/klavis";

#[derive(Clone)]
pub(super) struct Client {
    pub id: String,
    pub http: reqwest::Client,
    pub base_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Strata {
    pub strata_server_url: String,
    pub strata_id: String,
    pub added_servers: Vec<String>,
    #[serde(default)]
    pub oauth_urls: serde_json::Map<String, Value>,
    #[serde(default)]
    pub api_key_urls: serde_json::Map<String, Value>,
}

impl Client {
    pub fn require_id(&self) -> Result<(), KlavisError> {
        if self.id.is_empty() {
            Err(KlavisError::Unavailable(
                "browserosId not configured".into(),
            ))
        } else {
            Ok(())
        }
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, KlavisError> {
        let mut request = self
            .http
            .request(
                method,
                format!("{}{path}", self.base_url.trim_end_matches('/')),
            )
            .timeout(TIMEOUT)
            .header("Content-Type", "application/json");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(upstream)?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(upstream)?;
        if !status.is_success() {
            return Err(KlavisError::Upstream(format!(
                "Klavis error: {status} - {}",
                String::from_utf8_lossy(&bytes)
            )));
        }
        serde_json::from_slice(&bytes).map_err(|e| KlavisError::Upstream(e.to_string()))
    }

    pub async fn create(&self, names: &[&str]) -> Result<Strata, KlavisError> {
        self.require_id()?;
        let value = self
            .request(
                Method::POST,
                "/mcp-server/strata/create",
                Some(json!({"userId":self.id,"servers":names})),
            )
            .await?;
        serde_json::from_value(value).map_err(|e| KlavisError::Upstream(e.to_string()))
    }

    pub async fn integrations(&self) -> Result<Vec<Integration>, KlavisError> {
        self.require_id()?;
        let value = self
            .request(
                Method::GET,
                &format!("/user/{}/integrations", self.id),
                None,
            )
            .await?;
        Ok(value
            .get("integrations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| {
                if let Some(name) = item.as_str() {
                    return Some(Integration {
                        name: name.into(),
                        is_authenticated: false,
                    });
                }
                let name = item.get("name")?.as_str().filter(|s| !s.is_empty())?;
                let authenticated = item
                    .get("isAuthenticated")
                    .and_then(Value::as_bool)
                    .or_else(|| item.get("is_authenticated").and_then(Value::as_bool))
                    .unwrap_or(false);
                Some(Integration {
                    name: name.into(),
                    is_authenticated: authenticated,
                })
            })
            .collect())
    }

    pub async fn submit_key(
        &self,
        api_key_url: &url::Url,
        api_key: &str,
    ) -> Result<(), KlavisError> {
        let id = api_key_url
            .query_pairs()
            .find(|(key, _)| key == "instance_id")
            .map(|(_, value)| value.into_owned())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| KlavisError::Upstream("Missing instance_id in apiKeyUrl".into()))?;
        self.request(
            Method::POST,
            "/mcp-server/instance/set-auth",
            Some(json!({"instanceId":id,"authData":{"api_key":api_key}})),
        )
        .await?;
        Ok(())
    }

    pub async fn remove(&self, strata_id: &str, name: &str) -> Result<(), KlavisError> {
        let query = url::form_urlencoded::byte_serialize(name.as_bytes())
            .collect::<String>()
            .replace('+', "%20");
        self.request(
            Method::DELETE,
            &format!("/mcp-server/strata/{strata_id}/servers?servers={query}"),
            None,
        )
        .await?;
        Ok(())
    }
}

fn upstream(error: reqwest::Error) -> KlavisError {
    KlavisError::Upstream(if error.is_timeout() {
        "Klavis request timed out after 30000ms".into()
    } else {
        error.to_string()
    })
}

pub(super) fn auth_url(map: &serde_json::Map<String, Value>, name: &str) -> Option<String> {
    let normalize = |s: &str| {
        s.to_lowercase()
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
    };
    map.get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| {
            let key = normalize(name);
            map.iter()
                .find(|(name, _)| normalize(name) == key)
                .and_then(|(_, url)| url.as_str().map(String::from))
        })
}
