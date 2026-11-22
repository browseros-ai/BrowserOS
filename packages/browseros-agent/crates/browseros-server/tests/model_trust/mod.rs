//! A machine with no certificate store (Linux without CA certificates). The clients are built
//! with no trusted authority, as `Clients::new` builds them when the store cannot be loaded: a
//! provider on plain HTTP still answers, and an HTTPS one gets the card that says what to install.

use serde_json::json;

use super::fake_provider::{FakeProvider, Reply};
use super::route::{Clients, Route, Trust};
use super::{Backend, Model, ModelMessage, Reasoning, StepRequest, tls};
use crate::store::{Credentials, ProviderConfig, ProviderKind, Secret};
use crate::turn::ChatErrorCode;

fn row(base: String, credentials: Credentials) -> ProviderConfig {
    ProviderConfig {
        id: "fixture".into(),
        kind: ProviderKind::OpenaiCompatible,
        model: "fixture-model".into(),
        base_url: Some(base),
        headers: Vec::new(),
        context_window: 8192,
        supports_images: false,
        reasoning_effort: None,
        reasoning_summary: None,
        resource_name: None,
        credentials,
    }
}

fn model(route: Route) -> Model {
    Model {
        route: Backend::Rig(Box::new(route)),
        name: "fixture-model".into(),
        window: 8192,
        supports_images: false,
        kind: Some(ProviderKind::OpenaiCompatible),
        reasoning: Reasoning::default(),
    }
}

#[tokio::test]
async fn without_a_certificate_store_https_says_what_to_install_and_http_still_works()
-> anyhow::Result<()> {
    let clients = Clients::build(Trust::Missing)?;

    let fake = FakeProvider::new(vec![Reply::events(&[
        json!({"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]}),
    ])])
    .await?;
    let messages = [ModelMessage::User {
        text: "Hello".into(),
        images: Vec::new(),
    }];
    let request = StepRequest {
        system: "",
        messages: &messages,
        tools: &[],
        effort: None,
    };
    let local = row(format!("{}/v1", fake.base), Credentials::None);
    model(Route::new(&local, &clients)?)
        .step(&request, &mut Vec::new())
        .await?;

    let server = tls::untrusted().await?;
    let remote = row(
        format!("https://127.0.0.1:{}/v1", server.port),
        Credentials::ApiKey(Secret::new("fixture-key")),
    );
    let Err(card) = model(Route::new(&remote, &clients)?)
        .step(&request, &mut Vec::new())
        .await
    else {
        anyhow::bail!("an HTTPS provider cannot be verified without a store");
    };
    assert_eq!(card.code, ChatErrorCode::ConnectionFailed);
    assert!(!card.retryable);
    assert!(
        card.message.contains("no trusted certificate authorities")
            && card.message.contains("ca-certificates"),
        "{}",
        card.message
    );
    Ok(())
}
