//! Credential behavior through the public MCP catalog/dispatcher, including errors
//! before a handler can run. Synthetic input must never reach diagnostic text.
use super::fake_ctx;
use crate::framework::{catalog, execute_tool};
use browseros_core::NativeCapabilities;
use serde_json::json;

#[tokio::test]
async fn credentials_are_native_gated_and_omit_scope_from_the_contract() -> anyhow::Result<()> {
    let definition = catalog()
        .into_iter()
        .find(|tool| tool.name == "credentials")
        .ok_or_else(|| anyhow::anyhow!("credentials tool is missing"))?;
    assert!(!definition.available(NativeCapabilities::default()));
    assert!(!definition.available(NativeCapabilities::from_browseros_version(Some("0.51.2.0"))));
    assert!(definition.available(NativeCapabilities::from_browseros_version(Some("0.51.2.1"))));
    assert!(definition.input_schema["properties"].get("scope").is_none());
    assert!(
        !definition.input_schema["required"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing required-field schema"))?
            .contains(&json!("page"))
    );
    assert!(definition.output_schema.is_some());
    let result = execute_tool(
        &definition,
        json!({"action":"add","password":"synthetic-credential-secret"}),
        &fake_ctx(),
    )
    .await?;
    assert!(result.is_error);
    assert!(!serde_json::to_string(&result.content)?.contains("synthetic-credential-secret"));
    assert!(
        result
            .audit_hints
            .is_some_and(|hints| hints.skip_screenshot)
    );
    Ok(())
}

#[tokio::test]
async fn run_marks_password_inputs_before_authorization_can_reject_them() -> anyhow::Result<()> {
    use crate::framework::{InnerCallHook, InnerCallRecord};
    use futures_util::future::BoxFuture;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Probe(Arc<AtomicBool>);
    impl InnerCallHook for Probe {
        fn mark_credential_input(&self) {
            self.0.store(true, Ordering::Relaxed);
        }
        fn authorize<'a>(&'a self, _page: Option<u32>) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async { Err("fixture authorization refused".into()) })
        }
        fn record<'a>(&'a self, _record: InnerCallRecord<'a>) -> BoxFuture<'a, ()> {
            Box::pin(async {})
        }
        fn on_page_created<'a>(&'a self, _page: u32) -> BoxFuture<'a, ()> {
            Box::pin(async {})
        }
    }
    let marked = Arc::new(AtomicBool::new(false));
    let mut ctx = fake_ctx();
    ctx.inner_call_hook = Some(Arc::new(Probe(marked.clone())));
    let run = catalog()
        .into_iter()
        .find(|tool| tool.name == "run")
        .ok_or_else(|| anyhow::anyhow!("missing run"))?;
    let result=execute_tool(&run,json!({"code":"return await browser.cdp('Autofill.addCredential',{password:'synthetic-run-secret'});"}),&ctx).await?;
    assert!(result.is_error);
    assert!(marked.load(Ordering::Relaxed));
    assert!(!serde_json::to_string(&result.content)?.contains("synthetic-run-secret"));
    Ok(())
}
