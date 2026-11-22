//! A saved provider row as a configured Rig model. Rig speaks the wire; the server still decides
//! where each request goes, which headers and credential it carries, which proxy it may use, and
//! the exact JSON a row's reasoning settings add.
//!
//! ```text
//!   ProviderConfig ──route()──▶ Route { Rig model, Wire, secrets }
//!                                  │
//!                                  └─ Rig request ──▶ Pinned (the row's URL and headers) ──▶ reqwest 0.13
//! ```

use std::sync::Arc;
use std::time::Duration;

use http::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use reqwest_rig as reqwest;
use rig_core::DynModel;
use rig_core::http_client::{
    self, Bytes, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
};
use rig_core::operation::Completion;
use rig_core::providers::anthropic::AnthropicConfig;
use rig_core::providers::chatgpt;
use rig_core::providers::openai::OpenAIConfig;
use rig_core::providers::openai::wire::{self as openai, Auth};
use rig_core::wasm_compat::WasmCompatSend;
use rig_reqwest::ReqwestClient;
use serde_json::{Map, Value, json};
use url::{Host, Url};

use super::chatgpt::Session;
use super::{Effort, Reasoning, bedrock, errors};
use crate::store::{Credentials, ProviderConfig, ProviderKind};
use crate::turn::ChatError;

/// The two HTTP clients every provider call goes through, built once per server. Neither follows
/// a redirect (it could carry a saved credential to another destination) or retries (retrying is
/// `Model::step`'s call, which knows whether any output reached the turn). The remote one uses
/// the proxy environment variables exactly as the server's reqwest 0.12 client did; the local one
/// never uses a proxy, because a model server on this machine is never behind one.
#[derive(Clone)]
pub(super) struct Clients {
    remote: ReqwestClient,
    local: ReqwestClient,
}

impl Clients {
    pub(super) fn new() -> Result<Self, reqwest::Error> {
        let builder = || {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_secs(15))
        };
        Ok(Self {
            remote: ReqwestClient::from(builder().build()?),
            local: ReqwestClient::from(builder().no_proxy().build()?),
        })
    }

    fn pinned(&self, url: Url, headers: Vec<(HeaderName, HeaderValue)>) -> Pinned {
        let http = if is_loopback(&url) {
            self.local.clone()
        } else {
            self.remote.clone()
        };
        Pinned {
            http,
            url: Arc::new(url),
            headers: Arc::new(headers),
        }
    }
}

/// 127.0.0.0/8, ::1 and `localhost`.
fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        Some(Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

/// Rig's transport for one resolved provider. Rig forms its own URL from a base (dropping a
/// saved query string, and for Anthropic always ending in `/v1/messages`); this sends every
/// request to the URL the server resolved from the row instead, with the row's custom headers
/// over Rig's own. A resolved model makes one kind of request, so one URL serves all of them.
#[derive(Clone)]
struct Pinned {
    http: ReqwestClient,
    url: Arc<Url>,
    /// Custom headers, minus the ones the route's credential owns, then the route's own extras.
    headers: Arc<Vec<(HeaderName, HeaderValue)>>,
}

impl Pinned {
    fn pin<B>(&self, mut request: Request<B>) -> http_client::Result<Request<B>> {
        *request.uri_mut() = self.url.as_str().parse().map_err(http::Error::from)?;
        for (name, value) in self.headers.iter() {
            request.headers_mut().insert(name.clone(), value.clone());
        }
        Ok(request)
    }
}

impl HttpClientExt for Pinned {
    fn send(
        &self,
        request: Request<Bytes>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody>>> + WasmCompatSend + 'static
    {
        let pinned = self.pin(request);
        let http = self.http.clone();
        async move { http.send(pinned?).await }
    }

    fn send_multipart(
        &self,
        request: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody>>> + WasmCompatSend + 'static
    {
        let pinned = self.pin(request);
        let http = self.http.clone();
        async move { http.send_multipart(pinned?).await }
    }

    fn send_streaming(
        &self,
        request: Request<Bytes>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + WasmCompatSend {
        let pinned = self.pin(request);
        async move { self.http.send_streaming(pinned?).await }
    }
}

/// Which wire a route speaks, for what the server adds to each request on top of Rig's body.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Wire {
    Chat(ProviderKind),
    Responses,
    ChatGpt,
    Anthropic,
    Bedrock,
}

/// A resolved provider: the Rig model, its wire, and every credential and custom header value it
/// may send, so error text can be scrubbed of them.
pub(super) struct Route {
    pub model: DynModel<Completion>,
    pub wire: Wire,
    pub url: Option<Url>,
    pub secrets: HeaderMap,
}

impl Route {
    /// Every row type except the ChatGPT sign-in, whose credentials come from the token store.
    pub(super) fn new(provider: &ProviderConfig, clients: &Clients) -> Result<Self, ChatError> {
        use ProviderKind::*;
        match provider.kind {
            Google | Openrouter | Fireworks | Azure | OpenaiCompatible | Ollama | Lmstudio
            | Moonshot => chat(provider, clients),
            Openai => {
                let key = api_key(provider)?;
                let url = endpoint(base(provider, "https://api.openai.com/v1/"), "responses")?;
                let config = OpenAIConfig::new(key).with_auth(Auth::Bearer);
                let (headers, secrets) = headers(provider, &[(AUTHORIZATION, key)], Vec::new())?;
                let model = config
                    .connect(clients.pinned(url.clone(), headers))
                    .responses(&provider.model)
                    .erase();
                Ok(Self::http(model, Wire::Responses, url, secrets))
            }
            Anthropic => {
                let key = api_key(provider)?;
                let url = endpoint(base(provider, "https://api.anthropic.com/v1/"), "messages")?;
                let (headers, secrets) = headers(
                    provider,
                    &[(HeaderName::from_static("x-api-key"), key)],
                    Vec::new(),
                )?;
                let model = AnthropicConfig::new(key)
                    .connect(clients.pinned(url.clone(), headers))
                    .completion(&provider.model)
                    .erase();
                Ok(Self::http(model, Wire::Anthropic, url, secrets))
            }
            Bedrock => {
                let (model, secrets) = bedrock::model(provider)?;
                Ok(Self {
                    model,
                    wire: Wire::Bedrock,
                    url: None,
                    secrets,
                })
            }
            ChatgptPro => Err(errors::config(
                "ChatGPT Pro requires signing in to ChatGPT.",
            )),
        }
    }

    /// The signed-in ChatGPT backend. The sign-in's headers win over a row's custom ones, as the
    /// sign-in's own identity (`originator`) and credentials must reach the backend unchanged.
    pub(super) fn chatgpt(
        provider: &ProviderConfig,
        session: Session,
        clients: &Clients,
    ) -> Result<Self, ChatError> {
        let access = session.access.expose();
        let mut config = OpenAIConfig::with_key(&chatgpt::DIALECT, access);
        // The server sends its own system prompt and identity, not Rig's ChatGPT persona.
        config.instructions = None;
        config.identity = None;
        let mut owned = vec![(AUTHORIZATION, access)];
        if let Some(account) = session.account_id.as_deref() {
            config = config.with_account_id(account);
            owned.push((HeaderName::from_static("chatgpt-account-id"), account));
        }
        let extras = vec![
            (
                HeaderName::from_static("originator"),
                HeaderValue::from_static("browseros"),
            ),
            (
                HeaderName::from_static("openai-beta"),
                HeaderValue::from_static("responses=experimental"),
            ),
        ];
        let (headers, secrets) = headers(provider, &owned, extras)?;
        let model = config
            .connect(clients.pinned(session.url.clone(), headers))
            .responses(&provider.model)
            .erase();
        Ok(Self::http(model, Wire::ChatGpt, session.url, secrets))
    }

    fn http(model: DynModel<Completion>, wire: Wire, url: Url, secrets: HeaderMap) -> Self {
        Self {
            model,
            wire,
            url: Some(url),
            secrets,
        }
    }

    /// A Rig failure as the panel's error card, scrubbed of this route's secrets.
    pub(super) fn error(&self, error: &rig_core::ProviderError) -> ChatError {
        match self.wire {
            Wire::Bedrock => bedrock::error(error, &self.secrets),
            _ => errors::provider(error, self.url.as_ref(), &self.secrets),
        }
    }
}

/// The eight row types on Chat Completions. OpenRouter and Moonshot use Rig's dialects for them
/// (their reasoning and replay rules); the rest are the generic OpenAI dialect at the saved URL.
fn chat(provider: &ProviderConfig, clients: &Clients) -> Result<Route, ChatError> {
    use ProviderKind::*;
    let default = match provider.kind {
        Google => "https://generativelanguage.googleapis.com/v1beta/openai/",
        Openrouter => "https://openrouter.ai/api/v1/",
        Fireworks => "https://api.fireworks.ai/inference/v1/",
        Ollama => "http://localhost:11434/v1/",
        Lmstudio => "http://localhost:1234/v1/",
        _ => "",
    };
    let saved = provider
        .base_url
        .as_deref()
        .filter(|url| !url.trim().is_empty());
    if provider.kind == Azure && api_key(provider).is_err() {
        return Err(errors::config(
            "Azure requires an API key and either a resource name or a base URL.",
        ));
    }
    if provider.kind == Moonshot && (saved.is_none() || api_key(provider).is_err()) {
        return Err(errors::config(
            "Moonshot requires an API key and a base URL.",
        ));
    }
    let url = match (provider.kind, saved) {
        (Azure, None) => {
            let resource = provider
                .resource_name
                .as_deref()
                .filter(|name| {
                    !name.is_empty()
                        && name
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                })
                .ok_or_else(|| errors::config("Azure requires a resource name or a base URL."))?;
            endpoint(
                &format!("https://{resource}.openai.azure.com/openai/v1"),
                "chat/completions",
            )?
        }
        (OpenaiCompatible, None) => {
            return Err(errors::config(
                "An OpenAI-compatible provider requires a base URL.",
            ));
        }
        (_, saved) => endpoint(saved.unwrap_or(default), "chat/completions")?,
    };
    let dialect = match provider.kind {
        Openrouter => &openai::OPENROUTER,
        Moonshot => &openai::MOONSHOT,
        _ => &openai::OPENAI,
    };
    let (config, owned) = match provider.kind {
        Azure => {
            let key = api_key(provider)?;
            let owned = vec![(HeaderName::from_static("api-key"), key)];
            let config = OpenAIConfig::with_key(dialect, key).with_auth(Auth::ApiKeyHeader);
            // Azure authenticates with `api-key`; a custom Authorization must not ride along.
            (config, [owned, vec![(AUTHORIZATION, "")]].concat())
        }
        // A local server or reverse proxy with no credential configured gets no placeholder key,
        // and a custom Authorization header still reaches it.
        OpenaiCompatible | Ollama | Lmstudio
            if matches!(provider.credentials, Credentials::None) =>
        {
            let config = OpenAIConfig::with_key(dialect, "").with_auth(Auth::OptionalBearer);
            (config, Vec::new())
        }
        _ => {
            let key = api_key(provider)?;
            let config = OpenAIConfig::with_key(dialect, key).with_auth(Auth::Bearer);
            (config, vec![(AUTHORIZATION, key)])
        }
    };
    let (headers, secrets) = headers(provider, &owned, Vec::new())?;
    let model = config
        .connect(clients.pinned(url.clone(), headers))
        .chat(&provider.model)
        .erase();
    Ok(Route::http(model, Wire::Chat(provider.kind), url, secrets))
}

/// The JSON a step adds to Rig's body (`additional_params`, merged at the top level) and its
/// output-token limit. Exact JSON rather than Rig's typed options: those refuse a level a dialect
/// does not list, and the row stores the provider's own level names.
pub(super) struct StepOptions {
    pub params: Option<Value>,
    pub max_tokens: Option<u64>,
}

impl Wire {
    pub(super) fn options(
        self,
        model: &str,
        effort: Option<Effort>,
        reasoning: &Reasoning,
    ) -> StepOptions {
        use ProviderKind::*;
        let mut params = Map::new();
        let mut max_tokens = None;
        match self {
            Wire::Chat(kind @ (Google | Azure)) => {
                let gemini_xhigh = ["gemini-4-neon", "gemini-4-hydrogen"]
                    .iter()
                    .any(|family| model.trim_start_matches("models/").starts_with(family));
                let levels: &[_] = if kind == Google && !gemini_xhigh {
                    &["minimal", "low", "medium", "high"]
                } else {
                    &["minimal", "low", "medium", "high", "xhigh"]
                };
                if let Some(level) = reasoning.effort(effort, levels) {
                    params.insert("reasoning_effort".into(), json!(level));
                }
            }
            Wire::Chat(Openrouter) => {
                let levels = ["minimal", "low", "medium", "high", "xhigh", "max"];
                if let Some(level) = reasoning.effort(effort, &levels) {
                    // The established empty options object for the default medium case.
                    let value = if effort == Some(Effort::Medium) && reasoning.effort.is_none() {
                        json!({})
                    } else {
                        json!({"effort": level})
                    };
                    params.insert("reasoning".into(), value);
                }
            }
            Wire::Chat(_) => {}
            Wire::Responses | Wire::ChatGpt => {
                // A continuation is self-contained: with no server-side storage, Rig asks for the
                // encrypted reasoning and replays it with the matching function results.
                if self == Wire::Responses {
                    params.insert("store".into(), json!(false));
                }
                if let Some(level) = reasoning.effort(
                    effort,
                    &["minimal", "low", "medium", "high", "xhigh", "max"],
                ) {
                    let summary = reasoning
                        .summary
                        .as_deref()
                        .filter(|summary| !summary.is_empty())
                        .unwrap_or("auto");
                    params.insert(
                        "reasoning".into(),
                        json!({"effort": level, "summary": summary}),
                    );
                }
            }
            Wire::Anthropic => {
                max_tokens = Some(4096);
                if let Some((fields, limit)) = thinking_fields(model, effort, reasoning) {
                    params.extend(fields);
                    max_tokens = Some(limit);
                }
            }
            // Rig places these fields under `additionalModelRequestFields`.
            Wire::Bedrock => {
                if let Some((_, claude)) = model.split_once("anthropic.")
                    && let Some((fields, limit)) = thinking_fields(claude, effort, reasoning)
                {
                    params.extend(fields);
                    max_tokens = Some(limit);
                }
            }
        }
        StepOptions {
            params: (!params.is_empty()).then_some(Value::Object(params)),
            max_tokens: max_tokens.map(u64::from),
        }
    }

    /// The system prompt as sent. The ChatGPT backend refuses empty instructions.
    pub(super) fn system(self, system: &str) -> &str {
        if self == Wire::ChatGpt && system.trim().is_empty() {
            "You are a helpful assistant."
        } else {
            system
        }
    }
}

/// Claude on the Messages API and Bedrock uses the same model-specific thinking fields, and the
/// output limit that leaves room for them.
fn thinking_fields(
    model: &str,
    requested: Option<Effort>,
    reasoning: &Reasoning,
) -> Option<(Map<String, Value>, u32)> {
    let mut parts = model.split('-');
    let family = parts.next();
    let series = parts.next();
    let major = parts.next().and_then(|part| part.parse::<u32>().ok());
    let minor = parts.next().and_then(|part| part.parse::<u32>().ok());
    let adaptive = family == Some("claude")
        && (matches!(series, Some("fable" | "mythos"))
            || major.is_some_and(|major| major >= 5)
            || (major == Some(4)
                && matches!(series, Some("sonnet" | "opus"))
                && minor.is_some_and(|minor| minor >= 6)));
    let mut fields = Map::new();
    let max_tokens = if adaptive {
        // Claude 4.6 accepts max, with xhigh introduced for later models.
        let levels: &[_] = if major == Some(4) && minor == Some(6) {
            &["low", "medium", "high", "max"]
        } else {
            &["low", "medium", "high", "xhigh", "max"]
        };
        let effort = reasoning.effort(requested, levels)?;
        fields.insert(
            "thinking".into(),
            json!({"type": "adaptive", "display": "summarized"}),
        );
        fields.insert("output_config".into(), json!({"effort": effort}));
        32_768
    } else {
        let effort = reasoning.effort(
            requested,
            &["minimal", "low", "medium", "high", "xhigh", "max"],
        )?;
        let budget = match effort {
            "minimal" => 1024,
            "low" => 4096,
            "high" => 24_576,
            "xhigh" | "max" => 32_000,
            _ => 8192,
        };
        fields.insert(
            "thinking".into(),
            json!({"type": "enabled", "budget_tokens": budget}),
        );
        budget + 4096
    };
    Some((fields, max_tokens))
}

fn base<'a>(provider: &'a ProviderConfig, default: &'a str) -> &'a str {
    provider
        .base_url
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(default)
}

/// The saved base with the wire's operation path appended. The base's own path and query are
/// kept, and nothing is assumed about a `/v1` segment.
fn endpoint(base: &str, operation: &str) -> Result<Url, ChatError> {
    let mut url =
        Url::parse(base).map_err(|_| errors::config("Enter a valid provider base URL."))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(errors::config(
            "Enter an HTTP or HTTPS base URL without credentials or a fragment.",
        ));
    }
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| errors::config("Enter a valid provider base URL."))?;
        path.pop_if_empty().extend(operation.split('/'));
    }
    Ok(url)
}

/// The row's custom headers for the transport, and the scrub list for error text. A custom
/// header named like one the route's credential sets (`owned`) is dropped so the credential
/// always wins; `extras` are the route's own headers, applied after the custom ones.
fn headers(
    provider: &ProviderConfig,
    owned: &[(HeaderName, &str)],
    extras: Vec<(HeaderName, HeaderValue)>,
) -> Result<(Vec<(HeaderName, HeaderValue)>, HeaderMap), ChatError> {
    let custom = super::headers(&provider.headers)?;
    let mut secrets = custom.clone();
    for (name, value) in owned.iter().filter(|(_, value)| !value.is_empty()) {
        super::set_header(&mut secrets, name.clone(), value, true)?;
    }
    let sent = custom
        .into_iter()
        .filter_map(|(name, value)| Some((name?, value)))
        .filter(|(name, _)| !owned.iter().any(|(owned, _)| owned == name))
        .chain(extras)
        .collect();
    Ok((sent, secrets))
}

pub(super) fn api_key(provider: &ProviderConfig) -> Result<&str, ChatError> {
    match &provider.credentials {
        Credentials::ApiKey(key) if !key.expose().trim().is_empty() => Ok(key.expose()),
        _ => Err(errors::config(format!(
            "{} requires an API key.",
            provider.kind.as_str()
        ))),
    }
}
