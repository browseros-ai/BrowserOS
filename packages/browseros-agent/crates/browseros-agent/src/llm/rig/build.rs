//! Provider construction: `LlmConfig` → `RigModel` for the 13 `LLM_PROVIDERS`, an exact port of
//! `agent/provider-factory.ts` (required fields and their error texts, base URL defaults, which
//! providers forward user headers) with each provider's rig wire:
//!
//! | provider | wire | endpoint (TS) |
//! |---|---|---|
//! | anthropic | Messages | `{base ?? api.anthropic.com/v1}/messages` |
//! | openai | Responses | `{base ?? api.openai.com/v1}/responses` |
//! | azure | Responses, `api-key` header | `@ai-sdk/azure` URL rules |
//! | google | GenerateContent, `x-goog-api-key` | `{base ?? …/v1beta}/models/{m}:streamGenerateContent?alt=sse` |
//! | openrouter | Chat, OpenRouter dialect | `{base ?? openrouter.ai/api/v1}/chat/completions` |
//! | ollama, lmstudio, openai-compatible, moonshot | Chat | `{base}/chat/completions` |
//! | chatgpt-pro | Responses, ChatGPT dialect | `chatgpt.com/backend-api/codex/responses` |
//! | github-copilot | Chat | `api.githubcopilot.com/chat/completions` |
//! | qwen-code | Chat | `portal.qwen.ai/v1/chat/completions` |
//! | bedrock | Converse (AWS SDK, SigV4) | regional runtime endpoint |
//!
//! The three OAuth providers never forward user headers (managed credentials).

use super::RigModel;
use super::transport::{BodyRewrite, PolicyClient, RequestPolicy};
use crate::llm::{HttpStack, LlmConfig, LlmConfigError, ProviderKind};
use http::{HeaderName, HeaderValue, Uri};
use rig_core::providers::anthropic::AnthropicConfig;
use rig_core::providers::chatgpt;
use rig_core::providers::gemini::GeminiConfig;
use rig_core::providers::openai::OpenAIConfig;
use rig_core::providers::openai::wire::{Auth, OPENROUTER};

const CODEX_API_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";
const GITHUB_COPILOT_API: &str = "https://api.githubcopilot.com";
const QWEN_CODE_API: &str = "https://portal.qwen.ai/v1";

/// `if (!config.x)`: absent and empty are both missing.
fn present(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.is_empty())
}

fn required<'a>(value: &'a Option<String>, message: &str) -> Result<&'a str, LlmConfigError> {
    present(value).ok_or_else(|| LlmConfigError(message.to_owned()))
}

/// AI SDK `withoutTrailingSlash`.
fn trimmed(url: &str) -> &str {
    url.trim_end_matches('/')
}

fn endpoint(url: String, http: &HttpStack) -> Result<Uri, LlmConfigError> {
    let uri = url
        .parse::<Uri>()
        .ok()
        .filter(|uri| uri.scheme().is_some() && uri.authority().is_some())
        .ok_or_else(|| LlmConfigError(format!("Invalid provider URL: {url}")))?;
    let Some(origin) = http.test_origin() else {
        return Ok(uri);
    };
    let mut parts = uri.into_parts();
    parts.scheme = origin.scheme().cloned();
    parts.authority = origin.authority().cloned();
    Uri::from_parts(parts).map_err(|_| LlmConfigError(format!("Invalid provider URL: {url}")))
}

/// A header as `fetch` sends it: the value's code points as Latin-1 bytes (a ByteString).
fn header(name: &str, value: &str) -> Result<(HeaderName, HeaderValue), LlmConfigError> {
    let invalid_value = || LlmConfigError(format!("Invalid header value for {name}"));
    let header_name = HeaderName::try_from(name)
        .map_err(|_| LlmConfigError(format!("Invalid header name: {name}")))?;
    let bytes = value
        .chars()
        .map(|ch| u8::try_from(u32::from(ch)).map_err(|_| invalid_value()))
        .collect::<Result<Vec<u8>, _>>()?;
    let value = HeaderValue::from_bytes(&bytes).map_err(|_| invalid_value())?;
    Ok((header_name, value))
}

/// `resolveProviderHeaders`: the user's headers with every `{{conversationId}}` replaced.
fn user_headers(config: &LlmConfig) -> Result<Vec<(HeaderName, HeaderValue)>, LlmConfigError> {
    let Some(headers) = &config.headers else {
        return Ok(Vec::new());
    };
    crate::llm::validate_headers(headers)?;
    headers
        .iter()
        .map(|(name, value)| {
            header(
                name,
                &value.replace(
                    crate::llm::CONVERSATION_ID_PLACEHOLDER,
                    &config.conversation_id,
                ),
            )
        })
        .collect()
}

/// `@ai-sdk/azure` `url({ path })` without deployment-based URLs: a non-Azure or already
/// versioned (`…/openai/v1`) base is used as is; otherwise `/v1` is appended and, outside Foundry
/// projects, `api-version=v1`.
fn azure_url(base_url: Option<&str>, resource_name: Option<&str>, path: &str) -> String {
    let (azure, foundry, versioned) = match base_url.and_then(|base| base.parse::<Uri>().ok()) {
        None if base_url.is_some() => (false, false, false),
        None => (true, false, false),
        Some(uri) => {
            let host = uri.host().unwrap_or_default();
            let azure = [
                ".openai.azure.com",
                ".services.ai.azure.com",
                ".cognitiveservices.azure.com",
            ]
            .iter()
            .any(|suffix| host.ends_with(suffix));
            let pathname = uri.path().trim_end_matches('/');
            (
                azure,
                host.ends_with(".services.ai.azure.com") && pathname.starts_with("/api/projects/"),
                azure && pathname.to_lowercase().ends_with("/openai/v1"),
            )
        }
    };
    let prefix = match base_url {
        Some(base) => trimmed(base).to_owned(),
        None => format!(
            "https://{}.openai.azure.com/openai",
            resource_name.unwrap_or_default()
        ),
    };
    let mut url = if !azure || versioned {
        format!("{prefix}{path}")
    } else {
        format!("{prefix}/v1{path}")
    };
    if azure && !versioned && !foundry {
        url.push_str(if url.contains('?') { "&" } else { "?" });
        url.push_str("api-version=v1");
    }
    url
}

/// `getModelPath` of `@ai-sdk/google`: a qualified path (`tunedModels/x`) is kept.
fn google_model_path(model: &str) -> String {
    if model.contains('/') {
        model.to_owned()
    } else {
        format!("models/{model}")
    }
}

/// An OpenAI-shaped Chat Completions model at `base`, as `createOpenAICompatible`: no
/// `Authorization` header when there is no key.
fn compatible_chat(
    http: &HttpStack,
    config: &LlmConfig,
    base: &str,
    key: Option<&str>,
    policy: RequestPolicy,
) -> rig_core::DynModel<rig_core::operation::Completion> {
    let auth = if key.is_some() {
        Auth::Bearer
    } else {
        Auth::OptionalBearer
    };
    OpenAIConfig::new(key.unwrap_or_default())
        .with_auth(auth)
        .with_base_url(trimmed(base))
        .connect(PolicyClient::new(http.client(), policy))
        .chat(config.model.clone())
        .erase()
}

fn policy(
    http: &HttpStack,
    endpoint_url: String,
    headers: Vec<(HeaderName, HeaderValue)>,
) -> Result<RequestPolicy, LlmConfigError> {
    Ok(RequestPolicy {
        endpoint: endpoint(endpoint_url, http)?,
        headers,
        body: BodyRewrite::None,
    })
}

pub(crate) fn build(config: &LlmConfig, http: &HttpStack) -> Result<RigModel, LlmConfigError> {
    let model_id = config.model.clone();
    let model = match config.provider {
        ProviderKind::Anthropic => {
            let key = required(&config.api_key, "Anthropic provider requires apiKey")?;
            let base = present(&config.base_url).unwrap_or("https://api.anthropic.com/v1");
            let policy = policy(
                http,
                format!("{}/messages", trimmed(base)),
                user_headers(config)?,
            )?;
            AnthropicConfig::new(key)
                .with_base_url(base)
                .connect(PolicyClient::new(http.client(), policy))
                .completion(model_id.clone())
                .erase()
        }
        ProviderKind::OpenAi => {
            let key = required(&config.api_key, "OpenAI provider requires apiKey")?;
            let base = present(&config.base_url).unwrap_or("https://api.openai.com/v1");
            let policy = policy(
                http,
                format!("{}/responses", trimmed(base)),
                user_headers(config)?,
            )?;
            OpenAIConfig::new(key)
                .with_base_url(trimmed(base))
                .connect(PolicyClient::new(http.client(), policy))
                .responses(model_id.clone())
                .erase()
        }
        ProviderKind::Google => {
            let key = required(&config.api_key, "Google provider requires apiKey")?;
            let base = present(&config.base_url)
                .unwrap_or("https://generativelanguage.googleapis.com/v1beta");
            // The key rides in the header as in `@ai-sdk/google`, never in the URL (rig would put
            // it in `?key=`; the endpoint override drops that).
            let mut headers = vec![header("x-goog-api-key", key)?];
            headers.extend(user_headers(config)?);
            let policy = policy(
                http,
                format!(
                    "{}/{}:streamGenerateContent?alt=sse",
                    trimmed(base),
                    google_model_path(&model_id)
                ),
                headers,
            )?;
            GeminiConfig::new(key)
                .connect(PolicyClient::new(http.client(), policy))
                .completion(model_id.clone())
                .erase()
        }
        ProviderKind::OpenRouter => {
            let key = required(&config.api_key, "OpenRouter provider requires apiKey")?;
            let base = present(&config.base_url).unwrap_or("https://openrouter.ai/api/v1");
            let policy = policy(
                http,
                format!("{}/chat/completions", trimmed(base)),
                user_headers(config)?,
            )?;
            OpenAIConfig::with_key(&OPENROUTER, key)
                .with_base_url(trimmed(base))
                .connect(PolicyClient::new(http.client(), policy))
                .chat(model_id.clone())
                .erase()
        }
        ProviderKind::Azure => {
            let (Some(key), resource, base) = (
                present(&config.api_key),
                present(&config.resource_name),
                present(&config.base_url),
            ) else {
                return Err(azure_config_error());
            };
            if resource.is_none() && base.is_none() {
                return Err(azure_config_error());
            }
            let url = azure_url(base, resource, "/responses");
            let policy = policy(http, url.clone(), user_headers(config)?)?;
            OpenAIConfig::new(key)
                .with_auth(Auth::ApiKeyHeader)
                .with_base_url(url.trim_end_matches("/responses"))
                .connect(PolicyClient::new(http.client(), policy))
                .responses(model_id.clone())
                .erase()
        }
        ProviderKind::LmStudio | ProviderKind::Ollama | ProviderKind::OpenAiCompatible => {
            let missing = match config.provider {
                ProviderKind::LmStudio => "LMStudio provider requires baseUrl",
                ProviderKind::Ollama => "Ollama provider requires baseUrl",
                _ => "OpenAI-compatible provider requires baseUrl",
            };
            let base = required(&config.base_url, missing)?;
            let mut policy = policy(
                http,
                format!("{}/chat/completions", trimmed(base)),
                user_headers(config)?,
            )?;
            policy.body = BodyRewrite::CompatChat;
            compatible_chat(http, config, base, present(&config.api_key), policy)
        }
        ProviderKind::Moonshot => {
            let base = required(&config.base_url, "Moonshot provider requires baseUrl")?;
            let key = required(&config.api_key, "Moonshot provider requires apiKey")?;
            let mut policy = policy(
                http,
                format!("{}/chat/completions", trimmed(base)),
                user_headers(config)?,
            )?;
            policy.body = BodyRewrite::CompatChat;
            compatible_chat(http, config, base, Some(key), policy)
        }
        ProviderKind::QwenCode => {
            let key = required(&config.api_key, "Qwen Code requires OAuth authentication")?;
            let mut policy = policy(
                http,
                format!("{QWEN_CODE_API}/chat/completions"),
                Vec::new(),
            )?;
            policy.body = BodyRewrite::CompatChat;
            compatible_chat(http, config, QWEN_CODE_API, Some(key), policy)
        }
        ProviderKind::GitHubCopilot => {
            let key = required(
                &config.api_key,
                "GitHub Copilot requires OAuth authentication",
            )?;
            let mut policy = policy(
                http,
                format!("{GITHUB_COPILOT_API}/chat/completions"),
                vec![
                    header("openai-intent", "conversation-edits")?,
                    // TS sends `user` on every step; rig's Copilot wire would send `agent` on
                    // follow-ups. Old-wire parity wins.
                    header("x-initiator", "user")?,
                ],
            )?;
            policy.body = BodyRewrite::Copilot;
            compatible_chat(http, config, GITHUB_COPILOT_API, Some(key), policy)
        }
        ProviderKind::ChatGptPro => {
            let key = required(&config.api_key, "ChatGPT requires OAuth authentication")?;
            let mut headers = Vec::new();
            if let Some(account) = present(&config.account_id) {
                headers.push(header("chatgpt-account-id", account)?);
            }
            headers.push(header("originator", "browseros")?);
            headers.push(header("openai-beta", "responses=experimental")?);
            let mut policy = policy(http, CODEX_API_ENDPOINT.to_owned(), headers)?;
            policy.body = BodyRewrite::Codex;
            // The system prompt stays an input message (made `developer` by the Codex rewrite)
            // and rig's "You are ChatGPT…" default instructions are dropped, so `instructions`
            // is Codex's own default, as `codex-fetch.ts` leaves it.
            let mut openai = OpenAIConfig::with_key(&chatgpt::DIALECT, key)
                .with_system_instructions_as_messages();
            openai.instructions = None;
            if let Some(account) = present(&config.account_id) {
                openai = openai.with_account_id(account);
            }
            openai
                .connect(PolicyClient::new(http.client(), policy))
                .responses(model_id.clone())
                .erase()
        }
        ProviderKind::Bedrock => bedrock(config, http)?,
    };
    Ok(RigModel {
        provider: config.provider,
        model_id,
        model,
    })
}

fn azure_config_error() -> LlmConfigError {
    LlmConfigError("Azure provider requires apiKey and either resourceName or baseUrl".to_owned())
}

#[cfg(not(feature = "bedrock"))]
fn bedrock(
    _config: &LlmConfig,
    _http: &HttpStack,
) -> Result<rig_core::DynModel<rig_core::operation::Completion>, LlmConfigError> {
    Err(LlmConfigError(
        "Bedrock provider is not available in this build".to_owned(),
    ))
}

#[cfg(feature = "bedrock")]
fn bedrock(
    config: &LlmConfig,
    http: &HttpStack,
) -> Result<rig_core::DynModel<rig_core::operation::Completion>, LlmConfigError> {
    use aws_sdk_bedrockruntime::config::{
        BehaviorVersion, Credentials, Region, StalledStreamProtectionConfig, retry::RetryConfig,
    };
    let missing = || {
        LlmConfigError(
            "Bedrock provider requires accessKeyId, secretAccessKey, and region".to_owned(),
        )
    };
    let access_key = present(&config.access_key_id).ok_or_else(missing)?;
    let secret = present(&config.secret_access_key).ok_or_else(missing)?;
    let region = present(&config.region).ok_or_else(missing)?;
    let credentials = Credentials::new(
        access_key,
        secret,
        present(&config.session_token).map(str::to_owned),
        None,
        "browseros",
    );
    let mut builder = aws_sdk_bedrockruntime::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(region.to_owned()))
        .credentials_provider(credentials)
        // The agent loop owns retries (AI SDK parity); a long silent thought is not a stall.
        .retry_config(RetryConfig::disabled())
        .stalled_stream_protection(StalledStreamProtectionConfig::disabled())
        .interceptor(bedrock_headers::UserHeaders(user_headers(config)?));
    if let Some(origin) = http.test_origin() {
        builder = builder.endpoint_url(origin.to_string().trim_end_matches('/'));
    }
    let client = aws_sdk_bedrockruntime::Client::from_conf(builder.build());
    Ok(rig_bedrock::client::BedrockRuntime::from(client)
        .completion(config.model.clone())
        .erase())
}

/// The user's custom headers on Bedrock requests (`createAmazonBedrock({ headers })`), added
/// before SigV4 signing so they are signed like any other header.
#[cfg(feature = "bedrock")]
mod bedrock_headers {
    use aws_sdk_bedrockruntime::config::interceptors::BeforeTransmitInterceptorContextMut;
    use aws_sdk_bedrockruntime::config::{ConfigBag, Intercept, RuntimeComponents};
    use http::{HeaderName, HeaderValue};

    #[derive(Debug)]
    pub(super) struct UserHeaders(pub Vec<(HeaderName, HeaderValue)>);

    impl Intercept for UserHeaders {
        fn name(&self) -> &'static str {
            "BrowserOsUserHeaders"
        }

        fn modify_before_signing(
            &self,
            context: &mut BeforeTransmitInterceptorContextMut<'_>,
            _runtime_components: &RuntimeComponents,
            _cfg: &mut ConfigBag,
        ) -> Result<(), aws_sdk_bedrockruntime::error::BoxError> {
            let headers = context.request_mut().headers_mut();
            for (name, value) in &self.0 {
                if let Ok(value) = value.to_str() {
                    headers.insert(name.as_str().to_owned(), value.to_owned());
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn azure_urls_follow_ai_sdk_azure() {
        assert_eq!(
            azure_url(None, Some("myres"), "/responses"),
            "https://myres.openai.azure.com/openai/v1/responses?api-version=v1"
        );
        assert_eq!(
            azure_url(
                Some("https://gateway.example/openai/v1/"),
                None,
                "/responses"
            ),
            "https://gateway.example/openai/v1/responses"
        );
        assert_eq!(
            azure_url(
                Some("https://myres.openai.azure.com/openai/v1"),
                None,
                "/responses"
            ),
            "https://myres.openai.azure.com/openai/v1/responses"
        );
        assert_eq!(
            azure_url(
                Some("https://myres.openai.azure.com/openai"),
                None,
                "/responses"
            ),
            "https://myres.openai.azure.com/openai/v1/responses?api-version=v1"
        );
        assert_eq!(
            azure_url(
                Some("https://hub.services.ai.azure.com/api/projects/p1/openai"),
                None,
                "/responses"
            ),
            "https://hub.services.ai.azure.com/api/projects/p1/openai/v1/responses"
        );
    }

    #[test]
    fn google_model_paths() {
        assert_eq!(google_model_path("gemini-2.5-pro"), "models/gemini-2.5-pro");
        assert_eq!(google_model_path("tunedModels/abc"), "tunedModels/abc");
    }
}
