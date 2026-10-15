//! The HTTP seam under every rig model: a `HttpClientExt` wrapper that applies BrowserOS's
//! per-provider request policy before the shared reqwest client sends. It is the Rust form of the
//! TS server's custom `fetch` wrappers (`codex-fetch.ts`, `copilot-fetch.ts`) plus the URL and
//! header rules of the AI SDK provider factories:
//!
//! ```text
//!  rig wire (body + auth headers) ──► PolicyClient::apply ──► DynHttpClient (one reqwest pool)
//!                                       │ endpoint: the URL the TS SDK would call
//!                                       │ headers:  user headers / provider headers (insert)
//!                                       └ body:     Codex or Copilot rewrite
//! ```
//!
//! The endpoint replaces rig's URI wholesale, so URL conventions (Anthropic's `/v1` handling,
//! Gemini's `?key=`, Azure's `api-version`) stay exactly TS's whatever rig builds. A model is
//! built per provider config and only ever sends completions, so one fixed endpoint is correct.

use super::copilot;
use bytes::Bytes;
use http::{HeaderName, HeaderValue, Request, Uri};
use rig_core::http_client::{
    self, DynHttpClient, HttpClientExt, LazyBody, MultipartForm, Response, StreamingResponse,
};
use serde_json::Value;
use std::{future::Future, sync::Arc};

/// The body rewrite a provider needs to send what TS sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyRewrite {
    None,
    /// `createOpenAICompatible` sends a system prompt as a plain string; rig sends text parts,
    /// which some local servers reject.
    CompatChat,
    /// `codex-fetch.ts`: the ChatGPT-subscription Codex backend contract.
    Codex,
    /// `copilot-fetch.ts` (vision flag and VS Code's image shrink), on a compat chat body.
    Copilot,
}

#[derive(Debug, Clone)]
pub(crate) struct RequestPolicy {
    pub endpoint: Uri,
    /// Inserted (replacing rig's value) in order; later entries win.
    pub headers: Vec<(HeaderName, HeaderValue)>,
    pub body: BodyRewrite,
}

#[derive(Clone)]
pub(crate) struct PolicyClient {
    inner: DynHttpClient,
    policy: Arc<RequestPolicy>,
}

impl PolicyClient {
    pub(crate) fn new(inner: DynHttpClient, policy: RequestPolicy) -> Self {
        Self {
            inner,
            policy: Arc::new(policy),
        }
    }
}

impl RequestPolicy {
    async fn apply(&self, request: Request<Bytes>) -> http_client::Result<Request<Bytes>> {
        let (mut parts, body) = request.into_parts();
        parts.uri = self.endpoint.clone();
        for (name, value) in &self.headers {
            parts.headers.insert(name.clone(), value.clone());
        }
        let body = match self.body {
            BodyRewrite::None => body,
            BodyRewrite::CompatChat => compat_chat_body(body),
            BodyRewrite::Codex => codex_body(body),
            BodyRewrite::Copilot => {
                let (body, vision) = copilot::rewrite_body(compat_chat_body(body)).await;
                if vision {
                    parts.headers.insert(
                        HeaderName::from_static("copilot-vision-request"),
                        HeaderValue::from_static("true"),
                    );
                }
                body
            }
        };
        Ok(Request::from_parts(parts, body))
    }
}

/// System messages whose content is only text parts become one string, as the AI SDK compatible
/// provider sends them.
fn compat_chat_body(body: Bytes) -> Bytes {
    let Ok(Value::Object(mut json)) = serde_json::from_slice::<Value>(&body) else {
        return body;
    };
    let Some(Value::Array(messages)) = json.get_mut("messages") else {
        return body;
    };
    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("system") {
            continue;
        }
        let Some(Value::Array(parts)) = message.get("content") else {
            continue;
        };
        let texts: Option<Vec<&str>> = parts
            .iter()
            .map(|part| {
                (part.get("type").and_then(Value::as_str) == Some("text"))
                    .then(|| part.get("text").and_then(Value::as_str))
                    .flatten()
            })
            .collect();
        if let Some(texts) = texts {
            let text = texts.join("\n");
            message["content"] = Value::String(text);
        }
    }
    serde_json::to_vec(&Value::Object(json))
        .map(Bytes::from)
        .unwrap_or(body)
}

/// `codex-fetch.ts` body rules: always stream, never store, drop sampling caps and response
/// chaining, default `instructions`, and strip every input item id (nothing is stored server-side
/// with `store:false`). The system prompt travels as a `developer` input item, as `@ai-sdk/openai`
/// sends it to a reasoning model, so `instructions` gets Codex's default exactly as in TS.
/// Unparseable bodies pass through, as TS logs and sends them as-is.
fn codex_body(body: Bytes) -> Bytes {
    let Ok(Value::Object(mut json)) = serde_json::from_slice::<Value>(&body) else {
        tracing::warn!("Failed to inject Codex-required fields into request body");
        return body;
    };
    json.insert("stream".to_owned(), true.into());
    json.insert("store".to_owned(), false.into());
    for key in [
        "previous_response_id",
        "temperature",
        "max_tokens",
        "max_output_tokens",
        "top_p",
    ] {
        json.shift_remove(key);
    }
    if !json.get("instructions").is_some_and(js_truthy) {
        json.insert(
            "instructions".to_owned(),
            "You are a helpful assistant.".into(),
        );
    }
    if let Some(Value::Array(items)) = json.get_mut("input") {
        for item in items {
            if let Value::Object(item) = item {
                item.shift_remove("id");
                if item.get("role").and_then(Value::as_str) == Some("system") {
                    item.insert("role".to_owned(), "developer".into());
                }
            }
        }
    }
    serde_json::to_vec(&Value::Object(json))
        .map(Bytes::from)
        .unwrap_or(body)
}

/// JS truthiness, for the `if (!json.instructions)` check.
fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

impl HttpClientExt for PolicyClient {
    fn send<T, U>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        let inner = self.inner.clone();
        let policy = Arc::clone(&self.policy);
        let request = req.map(Into::into);
        async move { inner.send(policy.apply(request).await?).await }
    }

    fn send_multipart<U>(
        &self,
        req: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        // Completion wires never send multipart; pass through untouched.
        let inner = self.inner.clone();
        async move { inner.send_multipart(req).await }
    }

    fn send_streaming<T>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + Send
    where
        T: Into<Bytes> + Send,
    {
        let inner = self.inner.clone();
        let policy = Arc::clone(&self.policy);
        let request = req.map(Into::into);
        async move { inner.send_streaming(policy.apply(request).await?).await }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codex_body_matches_codex_fetch() -> anyhow::Result<()> {
        let body = json!({
            "model": "gpt-6-astra",
            "temperature": 0.2,
            "max_output_tokens": 10,
            "top_p": 1,
            "previous_response_id": "resp_1",
            "input": [{"role": "system", "content": "be brief"}, {"id": "msg_1", "role": "user", "content": "hi"}, {"type": "function_call", "id": "fc_1"}],
        });
        let out: Value = serde_json::from_slice(&codex_body(Bytes::from(body.to_string())))?;
        assert_eq!(
            out,
            json!({
                "model": "gpt-6-astra",
                "input": [{"role": "developer", "content": "be brief"}, {"role": "user", "content": "hi"}, {"type": "function_call"}],
                "stream": true,
                "store": false,
                "instructions": "You are a helpful assistant.",
            })
        );
        Ok(())
    }

    #[test]
    fn compat_chat_flattens_text_only_system_content() -> anyhow::Result<()> {
        let body = json!({"messages": [
            {"role": "system", "content": [{"type": "text", "text": "You are BrowserOS."}]},
            {"role": "user", "content": [{"type": "text", "text": "hi"}, {"type": "image_url"}]},
        ]});
        let out: Value = serde_json::from_slice(&compat_chat_body(Bytes::from(body.to_string())))?;
        assert_eq!(out["messages"][0]["content"], "You are BrowserOS.");
        assert_eq!(out["messages"][1], body["messages"][1]);
        Ok(())
    }

    #[test]
    fn codex_body_keeps_given_instructions() -> anyhow::Result<()> {
        let body = json!({"instructions": "You are BrowserOS."});
        let out: Value = serde_json::from_slice(&codex_body(Bytes::from(body.to_string())))?;
        assert_eq!(out["instructions"], "You are BrowserOS.");
        Ok(())
    }
}
