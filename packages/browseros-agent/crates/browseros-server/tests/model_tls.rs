//! A provider whose certificate cannot be verified says so, in the connection test and in the
//! chat card, with what to do, and is not retried: retrying cannot change the certificate.

#[path = "support/fake_provider.rs"]
mod fake_provider;
#[path = "support/tls.rs"]
mod tls;

use browseros_server::store::{Credentials, ProviderKind, Secret};
use browseros_server::turn::ChatErrorCode;
use fake_provider::{models, provider, request};
use uuid::Uuid;

#[tokio::test]
async fn an_untrusted_certificate_says_so_in_the_test_and_in_the_chat() -> anyhow::Result<()> {
    let server = tls::untrusted().await?;
    let (_dir, models) = models().await?;
    let row = || {
        provider(
            ProviderKind::OpenaiCompatible,
            format!("https://127.0.0.1:{}/v1", server.port),
            Credentials::ApiKey(Secret::new("fixture-key")),
        )
    };
    let says_so = |message: &str| {
        message.contains("certificate of the provider at 127.0.0.1")
            && message.contains("not signed by an authority this computer trusts")
            && message.contains("CA certificates")
            && !message.contains("closed before the response finished")
    };

    let Err(tested) = models.test(row()).await else {
        anyhow::bail!("the connection test must fail");
    };
    assert!(says_so(&tested.message), "{}", tested.message);

    let model = models.resolve(row(), Uuid::new_v4()).await?;
    let Err(card) = model.step(&request(&[]), &mut Vec::new()).await else {
        anyhow::bail!("the step must fail");
    };
    assert!(says_so(&card.message), "{}", card.message);
    assert_eq!(card.code, ChatErrorCode::ConnectionFailed);
    assert_eq!(card.title, "Certificate not trusted");
    assert!(!card.retryable);
    assert_eq!(
        server.accepted(),
        2,
        "one attempt each: a refused certificate is not retried"
    );
    Ok(())
}
