//! Which provider calls use the proxy the environment names. A provider on this machine
//! (127.0.0.0/8, ::1, `localhost`) is reached directly; every other address keeps the
//! environment's proxy, as the server's client always has.
//!
//! The proxy variables must be in the process environment before the client is built, and
//! setting them in-process is unsafe, so the parent test runs this binary again with them set
//! and a fake proxy listening, and the child makes the calls.

#[path = "support/fake_provider.rs"]
mod fake_provider;

use browseros_server::store::{Credentials, ProviderKind};
use fake_provider::{FakeProvider, Reply, models, provider, request};
use serde_json::json;
use uuid::Uuid;

/// Set only in the child the parent test starts.
const CHILD: &str = "BROWSEROS_TEST_PROXY_CHILD";

fn finished() -> Reply {
    Reply::events(&[
        json!({"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]}),
    ])
}

#[tokio::test]
async fn a_local_provider_skips_the_proxy_and_a_remote_one_keeps_it() -> anyhow::Result<()> {
    let proxy = FakeProvider::new(vec![finished()]).await?;
    let mut child = tokio::process::Command::new(std::env::current_exe()?);
    child
        .args(["--exact", "child_calls_three_providers", "--ignored"])
        .env(CHILD, "1")
        .env_remove("REQUEST_METHOD");
    for name in ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        child.env(name, &proxy.base);
    }
    for name in ["NO_PROXY", "no_proxy", "HTTPS_PROXY", "https_proxy"] {
        child.env_remove(name);
    }
    let output = child.output().await?;
    assert!(
        output.status.success(),
        "the child's calls failed:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let calls = proxy
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock"))?;
    assert_eq!(calls.len(), 1, "only the remote provider went to the proxy");
    assert_eq!(calls[0].uri.host(), Some("192.0.2.1"));
    assert_eq!(calls[0].uri.path(), "/v1/chat/completions");
    Ok(())
}

#[tokio::test]
#[ignore = "run by a_local_provider_skips_the_proxy_and_a_remote_one_keeps_it"]
async fn child_calls_three_providers() -> anyhow::Result<()> {
    if std::env::var_os(CHILD).is_none() {
        return Ok(());
    }
    let local = FakeProvider::new(vec![finished(), finished()]).await?;
    let port = local
        .base
        .rsplit(':')
        .next()
        .ok_or_else(|| anyhow::anyhow!("fake port"))?;
    let (_dir, models) = models().await?;
    for base in [
        format!("http://127.0.0.1:{port}/v1"),
        format!("http://localhost:{port}/v1"),
        // TEST-NET-1: unroutable, so only the proxy can answer it.
        "http://192.0.2.1/v1".to_owned(),
    ] {
        let config = provider(ProviderKind::OpenaiCompatible, base, Credentials::None);
        let model = models.resolve(config, Uuid::new_v4()).await?;
        model.step(&request(&[]), &mut Vec::new()).await?;
    }
    let calls = local
        .calls
        .lock()
        .map_err(|_| anyhow::anyhow!("capture lock"))?;
    assert_eq!(calls.len(), 2, "both local addresses were reached directly");
    Ok(())
}
