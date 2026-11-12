//! Payment card behavior through the public MCP catalog/dispatcher, including
//! errors before a handler can run. Synthetic card data must never reach
//! diagnostic text.
use super::{FakeConnection, fake_ctx};
use crate::{
    framework::{
        BrowserToolDefaults, BrowserToolOptions, InnerCallHook, InnerCallRecord, ToolCtx, catalog,
        execute_tool,
    },
    output_file::create_browser_output_file_access,
};
use browseros_core::{BrowserSession, BrowserSessionHooks, BrowserView, NativeCapabilities};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio_util::sync::CancellationToken;

const SYNTHETIC_NUMBER: &str = "4111111111111111";
const SYNTHETIC_CVC: &str = "987";

/// A browser that reports the first BrowserOS with payment cards, so requests
/// pass the native gate and reach argument parsing.
fn payments_ctx() -> ToolCtx {
    ToolCtx::new(BrowserToolOptions {
        browser: BrowserView::unbound(
            BrowserSession::new_with_native(
                Arc::new(FakeConnection::new()),
                BrowserSessionHooks::default(),
                NativeCapabilities::from_browseros_version(Some("0.51.2.7")),
            ),
            None,
        ),
        target: None,
        defaults: BrowserToolDefaults::default(),
        cancel: CancellationToken::new(),
        output_files: create_browser_output_file_access(),
        inner_call_hook: None,
        preloaded_helpers: Vec::new(),
    })
}

async fn assert_refused_without_echo(ctx: &ToolCtx, args: Value) -> anyhow::Result<()> {
    let definition = catalog()
        .into_iter()
        .find(|tool| tool.name == "payment_cards")
        .ok_or_else(|| anyhow::anyhow!("payment_cards tool is missing"))?;
    let shown = args.to_string();
    let result = execute_tool(&definition, args, ctx).await?;
    assert!(result.is_error, "{shown}");
    let text = serde_json::to_string(&result.content)?;
    assert!(!text.contains(SYNTHETIC_NUMBER), "{shown}: {text}");
    assert!(!text.contains(SYNTHETIC_CVC), "{shown}: {text}");
    assert!(
        result
            .audit_hints
            .is_some_and(|hints| hints.skip_screenshot && hints.redact_all_values),
        "{shown}"
    );
    Ok(())
}

#[tokio::test]
async fn payment_cards_are_native_gated() -> anyhow::Result<()> {
    let definition = catalog()
        .into_iter()
        .find(|tool| tool.name == "payment_cards")
        .ok_or_else(|| anyhow::anyhow!("payment_cards tool is missing"))?;
    assert!(!definition.available(NativeCapabilities::default()));
    assert!(!definition.available(NativeCapabilities::from_browseros_version(Some("0.51.2.6"))));
    assert!(definition.available(NativeCapabilities::from_browseros_version(Some("0.51.2.7"))));
    assert!(definition.input_schema["properties"]["card"].is_object());
    assert!(definition.output_schema.is_some());
    assert_refused_without_echo(
        &fake_ctx(),
        json!({"action":"add","card":{"number":SYNTHETIC_NUMBER,"expiryMonth":12,"expiryYear":2030,"cvc":SYNTHETIC_CVC}}),
    )
    .await
}

#[tokio::test]
async fn malformed_card_requests_never_echo_card_data() -> anyhow::Result<()> {
    let card = json!({"number": SYNTHETIC_NUMBER, "expiryMonth": 12, "expiryYear": 2030, "cvc": SYNTHETIC_CVC});
    let ctx = payments_ctx();
    for args in [
        // Two sources.
        json!({"action":"fill","cardId":"card_x","card":card}),
        // A fill needs a page.
        json!({"action":"fill","card":card}),
        // list takes no card.
        json!({"action":"list","card":card}),
        // Serde would quote a number passed as a JSON number.
        json!({"action":"add","card":{"number":4_111_111_111_111_111_u64,"expiryMonth":12,"expiryYear":2030,"cvc":SYNTHETIC_CVC}}),
        // An unknown field next to the number.
        json!({"action":"add","card":{"number":SYNTHETIC_NUMBER,"expiryMonth":12,"expiryYear":2030,"cvc":SYNTHETIC_CVC,"pin":"0000"}}),
    ] {
        assert_refused_without_echo(&ctx, args).await?;
    }
    Ok(())
}

struct Probe(Arc<AtomicBool>);
impl InnerCallHook for Probe {
    fn mark_sensitive_input(&self) {
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

#[tokio::test]
async fn run_marks_card_inputs_before_authorization_can_reject_them() -> anyhow::Result<()> {
    let run = catalog()
        .into_iter()
        .find(|tool| tool.name == "run")
        .ok_or_else(|| anyhow::anyhow!("missing run"))?;
    for code in [
        format!(
            "return await browser.cdp('Payments.fillCard',{{card:{{number:'{SYNTHETIC_NUMBER}'}}}});"
        ),
        format!(
            "return await browser.cdp('Autofill.trigger',{{fieldId:1,card:{{number:'{SYNTHETIC_NUMBER}'}}}});"
        ),
    ] {
        let marked = Arc::new(AtomicBool::new(false));
        let mut ctx = fake_ctx();
        ctx.inner_call_hook = Some(Arc::new(Probe(marked.clone())));
        let result = execute_tool(&run, json!({"code": code}), &ctx).await?;
        assert!(result.is_error);
        assert!(marked.load(Ordering::Relaxed), "unmarked: {code}");
        assert!(!serde_json::to_string(&result.content)?.contains(SYNTHETIC_NUMBER));
    }
    Ok(())
}

struct HoldProbe(Arc<std::sync::Mutex<Vec<u32>>>);
impl InnerCallHook for HoldProbe {
    fn hold_card_data<'a>(&'a self, page: u32) -> BoxFuture<'a, ()> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(page);
        Box::pin(async {})
    }
    fn authorize<'a>(&'a self, _page: Option<u32>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    fn record<'a>(&'a self, _record: InnerCallRecord<'a>) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
    fn on_page_created<'a>(&'a self, _page: u32) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

#[tokio::test]
async fn run_holds_card_pages_before_dispatching_raw_fills() -> anyhow::Result<()> {
    let (mut ctx, _connection, page) = super::harness_ctx().await;
    let held = Arc::new(std::sync::Mutex::new(Vec::new()));
    ctx.inner_call_hook = Some(Arc::new(HoldProbe(held.clone())));
    let run = catalog()
        .into_iter()
        .find(|tool| tool.name == "run")
        .ok_or_else(|| anyhow::anyhow!("missing run"))?;
    // Each fill fails after dispatch (this browser has no Payments domain);
    // the hold must already be in place. A read on the same session holds nothing.
    let code = format!(
        "const params = {{documentId: 'd', card: {{number: '{SYNTHETIC_NUMBER}'}}}};
         for (const call of [
           () => browser.cdp('Payments.fillCard', params, 'session-1'),
           () => browser.page({page}).cdp('Autofill.trigger', JSON.stringify(params)),
           () => browser.cdp('Payments.listCards', {{}}, 'session-1'),
         ]) {{ try {{ await call(); }} catch (error) {{}} }}
         return true;"
    );
    let result = execute_tool(&run, json!({"code": code}), &ctx).await?;
    assert!(!result.is_error, "{:?}", result.content);
    assert_eq!(
        *held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![page, page]
    );
    Ok(())
}
