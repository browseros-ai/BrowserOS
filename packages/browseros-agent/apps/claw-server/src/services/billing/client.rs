//! The billing gateway's HTTP API, and its errors turned into [`BillingError`].
//!
//! Nothing here logs a request or response body, a URL's query string, or an
//! `Authorization` header: the bodies can carry a pasted licence key or feedback text.

use super::{licence::SignedLicence, secret::Secret};
use reqwest::{Method, RequestBuilder, StatusCode, header::RETRY_AFTER, redirect::Policy};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use std::time::Duration;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use url::Url;

const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub enum BillingError {
    /// This build has no billing gateway.
    Unavailable,
    /// The gateway is down, timed out, or this computer is offline.
    Unreachable,
    /// The gateway no longer knows this install. Handled by registering again.
    Unauthorized,
    /// The gateway, or a local check, said no; the message is written for people.
    Refused(Refusal),
    Internal(String),
}

impl BillingError {
    /// What went wrong, for a log line: never the message or the computers, which can carry
    /// computer names.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Unreachable => "unreachable",
            Self::Unauthorized => "unauthorized",
            Self::Refused(_) => "refused",
            Self::Internal(_) => "internal",
        }
    }

    #[must_use]
    pub fn refused(code: RefusalCode, message: impl Into<String>) -> Self {
        Self::Refused(Refusal {
            code,
            message: message.into(),
            computers: Vec::new(),
            retry_after_seconds: None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalCode {
    InvalidRequest,
    UnknownPlan,
    AlreadySubscribed,
    NotLinked,
    KeyInvalid,
    KeyFull,
    TooManyTries,
    RateLimited,
    NotFound,
}

#[derive(Debug, Clone)]
pub struct Refusal {
    pub code: RefusalCode,
    pub message: String,
    /// The computers on a full key; empty otherwise.
    pub computers: Vec<Computer>,
    pub retry_after_seconds: Option<u32>,
}

/// The key this computer is on: its first group, its cap and its computers. Never the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub hint: String,
    pub cap: i32,
    pub computers: Vec<Computer>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Computer {
    pub install_id: String,
    pub label: Option<String>,
    pub platform: String,
    pub last_seen_at: i64,
    pub activated_at: i64,
    pub this_computer: bool,
}

/// What activate, remove and reset return: the fresh licence and the key afterwards.
#[derive(Clone, Debug)]
pub struct KeyChange {
    pub licence: SignedLicence,
    pub key: Option<Key>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pricing {
    #[serde(default)]
    pub plans: Vec<PricedPlan>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PricedPlan {
    pub plan: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub prices: Vec<Price>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    pub interval: String,
    pub interval_count: i32,
    pub amount: i64,
    pub currency: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutStarted {
    pub checkout_id: String,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CheckoutPoll {
    pub status: String,
    pub licence: Option<SignedLicence>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Registered {
    install_id: String,
    install_secret: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Registration<'a> {
    app: &'a str,
    app_version: &'a str,
    platform: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<&'a str>,
}

#[derive(Deserialize)]
struct WireKeyChange {
    licence: SignedLicence,
    current: Option<WireKey>,
}

#[derive(Deserialize)]
struct WireRenamed {
    current: WireKey,
}

#[derive(Deserialize)]
struct WireLicence {
    licence: SignedLicence,
}

#[derive(Deserialize)]
struct WirePortal {
    url: String,
}

#[derive(Deserialize)]
struct WireKey {
    hint: String,
    cap: i32,
    #[serde(default)]
    computers: Vec<WireComputer>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireComputer {
    install_id: String,
    label: Option<String>,
    platform: String,
    last_seen_at: String,
    activated_at: String,
    this_computer: bool,
}

#[derive(Deserialize)]
struct WireError {
    code: String,
    message: String,
    #[serde(default)]
    computers: Option<Vec<WireComputer>>,
}

#[derive(Clone, Debug)]
pub struct GatewayClient {
    http: reqwest::Client,
    base: Url,
}

impl GatewayClient {
    pub fn new(base: Url) -> Result<Self, BillingError> {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(Policy::none())
            .build()
            .map_err(|error| BillingError::Internal(format!("billing client: {error}")))?;
        Ok(Self { http, base })
    }

    /// Returns the new install's id and secret.
    pub async fn register(
        &self,
        app: &str,
        app_version: &str,
        platform: &str,
        label: Option<&str>,
    ) -> Result<(String, Secret), BillingError> {
        let body = Registration {
            app,
            app_version,
            platform,
            label,
        };
        let registered: Registered = self
            .send(
                self.request(Method::POST, "/v1/installs", None)?
                    .json(&body),
            )
            .await?;
        Ok((
            registered.install_id,
            Secret::new(registered.install_secret),
        ))
    }

    pub async fn pricing(&self, app: &str) -> Result<Pricing, BillingError> {
        self.send(
            self.request(Method::GET, "/v1/pricing", None)?
                .query(&[("app", app)]),
        )
        .await
    }

    pub async fn start_checkout(
        &self,
        secret: &Secret,
        plan: &str,
        interval: &str,
        interval_count: Option<i32>,
    ) -> Result<CheckoutStarted, BillingError> {
        let mut body = Map::new();
        body.insert("plan".into(), json!(plan));
        body.insert("interval".into(), json!(interval));
        if let Some(count) = interval_count {
            body.insert("intervalCount".into(), json!(count));
        }
        self.send(
            self.request(Method::POST, "/v1/checkouts", Some(secret))?
                .json(&Value::Object(body)),
        )
        .await
    }

    /// `checkout_id` must already be a UUID; it goes into the path as it is.
    pub async fn checkout(
        &self,
        secret: &Secret,
        checkout_id: &str,
    ) -> Result<CheckoutPoll, BillingError> {
        self.send(self.request(
            Method::GET,
            &format!("/v1/checkouts/{checkout_id}"),
            Some(secret),
        )?)
        .await
    }

    pub async fn current_key(&self, secret: &Secret) -> Result<Key, BillingError> {
        let key: WireKey = self
            .send(self.request(Method::GET, "/v1/keys/current", Some(secret))?)
            .await?;
        key.into_key()
    }

    pub async fn activate(
        &self,
        secret: &Secret,
        key: &Secret,
        replace: Option<&str>,
    ) -> Result<KeyChange, BillingError> {
        let mut body = Map::new();
        body.insert("key".into(), json!(key.expose()));
        if let Some(replace) = replace {
            body.insert("replace".into(), json!(replace));
        }
        let change: WireKeyChange = self
            .send(
                self.request(Method::POST, "/v1/keys/activate", Some(secret))?
                    .json(&Value::Object(body)),
            )
            .await?;
        change.into_change()
    }

    pub async fn deactivate(
        &self,
        secret: &Secret,
        install_id: &str,
    ) -> Result<KeyChange, BillingError> {
        let change: WireKeyChange = self
            .send(
                self.request(Method::POST, "/v1/keys/deactivate", Some(secret))?
                    .json(&json!({ "installId": install_id })),
            )
            .await?;
        change.into_change()
    }

    /// `install_id` must already be a UUID; it goes into the path as it is.
    pub async fn rename(
        &self,
        secret: &Secret,
        install_id: &str,
        label: &str,
    ) -> Result<Key, BillingError> {
        let renamed: WireRenamed = self
            .send(
                self.request(
                    Method::POST,
                    &format!("/v1/keys/computers/{install_id}/label"),
                    Some(secret),
                )?
                .json(&json!({ "label": label })),
            )
            .await?;
        renamed.current.into_key()
    }

    pub async fn reset(&self, secret: &Secret) -> Result<KeyChange, BillingError> {
        let change: WireKeyChange = self
            .send(self.request(Method::POST, "/v1/keys/reset", Some(secret))?)
            .await?;
        change.into_change()
    }

    pub async fn refresh(&self, secret: &Secret) -> Result<SignedLicence, BillingError> {
        self.send(self.request(Method::POST, "/v1/licence/refresh", Some(secret))?)
            .await
    }

    pub async fn cancel(
        &self,
        secret: &Secret,
        feedback: Option<&str>,
    ) -> Result<SignedLicence, BillingError> {
        let mut body = Map::new();
        if let Some(feedback) = feedback {
            body.insert("feedback".into(), json!(feedback));
        }
        let reply: WireLicence = self
            .send(
                self.request(Method::POST, "/v1/subscription/cancel", Some(secret))?
                    .json(&Value::Object(body)),
            )
            .await?;
        Ok(reply.licence)
    }

    pub async fn resume(&self, secret: &Secret) -> Result<SignedLicence, BillingError> {
        let reply: WireLicence = self
            .send(self.request(Method::POST, "/v1/subscription/resume", Some(secret))?)
            .await?;
        Ok(reply.licence)
    }

    pub async fn portal(&self, secret: &Secret, flow: &str) -> Result<String, BillingError> {
        let reply: WirePortal = self
            .send(
                self.request(Method::POST, "/v1/portal", Some(secret))?
                    .json(&json!({ "flow": flow })),
            )
            .await?;
        Ok(reply.url)
    }

    fn request(
        &self,
        method: Method,
        path: &str,
        secret: Option<&Secret>,
    ) -> Result<RequestBuilder, BillingError> {
        let url = self
            .base
            .join(path)
            .map_err(|_| BillingError::Internal("billing gateway path".to_string()))?;
        let builder = self.http.request(method, url);
        Ok(match secret {
            Some(secret) => builder.bearer_auth(secret.expose()),
            None => builder,
        })
    }

    async fn send<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T, BillingError> {
        let response = request.send().await.map_err(|error| {
            tracing::warn!(error = %error.without_url(), "billing gateway unreachable");
            BillingError::Unreachable
        })?;
        let status = response.status();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u32>().ok());
        let body = read_capped(response).await?;
        if status.is_success() {
            return serde_json::from_slice(&body).map_err(|_| {
                BillingError::Internal(format!("billing gateway reply unreadable ({status})"))
            });
        }
        Err(error_from(status, retry_after, &body))
    }
}

fn error_from(status: StatusCode, retry_after: Option<u32>, body: &[u8]) -> BillingError {
    if status == StatusCode::UNAUTHORIZED {
        return BillingError::Unauthorized;
    }
    if status.is_server_error() {
        tracing::warn!(%status, "billing gateway failed");
        return BillingError::Unreachable;
    }
    let Ok(error) = serde_json::from_slice::<WireError>(body) else {
        return BillingError::Internal(format!("billing gateway refused unreadably ({status})"));
    };
    let code = match error.code.as_str() {
        "invalidRequest" => RefusalCode::InvalidRequest,
        "unknownPlan" => RefusalCode::UnknownPlan,
        "alreadySubscribed" => RefusalCode::AlreadySubscribed,
        "notLinked" => RefusalCode::NotLinked,
        "keyInvalid" => RefusalCode::KeyInvalid,
        "keyFull" => RefusalCode::KeyFull,
        "tooManyTries" => RefusalCode::TooManyTries,
        "rateLimited" => RefusalCode::RateLimited,
        "notFound" => RefusalCode::NotFound,
        "unauthorized" => return BillingError::Unauthorized,
        other => {
            return BillingError::Internal(format!(
                "billing gateway refused with {other} ({status})"
            ));
        }
    };
    let computers = match error.computers {
        Some(computers) => match into_computers(computers) {
            Ok(computers) => computers,
            Err(error) => return error,
        },
        None => Vec::new(),
    };
    BillingError::Refused(Refusal {
        code,
        message: error.message,
        computers,
        retry_after_seconds: retry_after,
    })
}

/// Reads at most [`MAX_BODY_BYTES`], counting the bytes actually read rather than trusting
/// `Content-Length`.
async fn read_capped(mut response: reqwest::Response) -> Result<Vec<u8>, BillingError> {
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len().saturating_add(chunk.len()) > MAX_BODY_BYTES {
                    return Err(BillingError::Internal(
                        "billing gateway reply too large".to_string(),
                    ));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => return Ok(body),
            Err(error) => {
                tracing::warn!(error = %error.without_url(), "billing gateway reply cut off");
                return Err(BillingError::Unreachable);
            }
        }
    }
}

impl WireKeyChange {
    fn into_change(self) -> Result<KeyChange, BillingError> {
        Ok(KeyChange {
            licence: self.licence,
            key: self.current.map(WireKey::into_key).transpose()?,
        })
    }
}

impl WireKey {
    fn into_key(self) -> Result<Key, BillingError> {
        Ok(Key {
            hint: self.hint,
            cap: self.cap,
            computers: into_computers(self.computers)?,
        })
    }
}

fn into_computers(computers: Vec<WireComputer>) -> Result<Vec<Computer>, BillingError> {
    computers
        .into_iter()
        .map(|computer| {
            Ok(Computer {
                install_id: computer.install_id,
                label: computer.label,
                platform: computer.platform,
                last_seen_at: epoch_ms(&computer.last_seen_at)?,
                activated_at: epoch_ms(&computer.activated_at)?,
                this_computer: computer.this_computer,
            })
        })
        .collect()
}

/// The gateway sends ISO 8601 times; the local API speaks epoch milliseconds.
fn epoch_ms(iso: &str) -> Result<i64, BillingError> {
    let time = OffsetDateTime::parse(iso, &Rfc3339)
        .map_err(|_| BillingError::Internal("billing gateway sent an unreadable time".into()))?;
    i64::try_from(time.unix_timestamp_nanos() / 1_000_000)
        .map_err(|_| BillingError::Internal("billing gateway sent an unreadable time".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_times_become_epoch_milliseconds() -> Result<(), String> {
        let ms = epoch_ms("2026-10-27T09:41:00.123Z").map_err(|error| format!("{error:?}"))?;
        assert_eq!(ms, 1_793_094_060_123);
        assert!(epoch_ms("yesterday").is_err());
        Ok(())
    }

    #[test]
    fn gateway_errors_map_to_refusals() {
        let full = br#"{"code":"keyFull","message":"This key is on 5 computers.","computers":[
            {"installId":"a","label":"Mac","platform":"macos","lastSeenAt":"2026-10-27T09:41:00.000Z",
             "activatedAt":"2026-10-27T09:41:00.000Z","thisComputer":false}]}"#;
        let BillingError::Refused(refusal) = error_from(StatusCode::CONFLICT, None, full) else {
            panic!("key full is a refusal");
        };
        assert_eq!(refusal.code, RefusalCode::KeyFull);
        assert_eq!(refusal.computers.len(), 1);

        let limited = br#"{"code":"rateLimited","message":"Too many requests."}"#;
        let BillingError::Refused(refusal) =
            error_from(StatusCode::TOO_MANY_REQUESTS, Some(42), limited)
        else {
            panic!("rate limited is a refusal");
        };
        assert_eq!(refusal.code, RefusalCode::RateLimited);
        assert_eq!(refusal.retry_after_seconds, Some(42));

        assert!(matches!(
            error_from(StatusCode::UNAUTHORIZED, None, b"{}"),
            BillingError::Unauthorized
        ));
        assert!(matches!(
            error_from(StatusCode::BAD_GATEWAY, None, b"<html>"),
            BillingError::Unreachable
        ));
        assert!(matches!(
            error_from(StatusCode::BAD_REQUEST, None, b"not json"),
            BillingError::Internal(_)
        ));
    }
}
