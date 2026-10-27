//! Opt-in acceptance through real MCP dispatch and an owned patched Chromium.
//! The fixture page holds a card form and the cards are well-known test
//! numbers, so this test must never point at a user's normal browser session.
use browseros_cdp::{CdpClient, ConnectOptions};
use browseros_core::{BrowserSession, BrowserSessionHooks, BrowserView, NativeCapabilities};
use browseros_mcp::{
    BrowserToolDefaults, BrowserToolOptions, StaticTargetStore, TargetStore, ToolCtx, ToolResult,
    catalog, execute_tool, output_file::create_browser_output_file_access,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const VISA: &str = "4111111111111111";
const MASTERCARD: &str = "5555555555554444";

async fn context(
    session: Arc<BrowserSession>,
    target: Arc<StaticTargetStore>,
) -> anyhow::Result<ToolCtx> {
    let binding = target.get().await.map_err(anyhow::Error::msg)?;
    Ok(ToolCtx::new(BrowserToolOptions {
        browser: BrowserView::capture(session, binding, None).await,
        target: Some(target),
        defaults: BrowserToolDefaults::default(),
        cancel: CancellationToken::new(),
        output_files: create_browser_output_file_access(),
        inner_call_hook: None,
        preloaded_helpers: Vec::new(),
    }))
}
async fn call(ctx: &ToolCtx, name: &str, args: Value) -> anyhow::Result<ToolResult> {
    let tool = catalog()
        .into_iter()
        .find(|tool| tool.name == name)
        .ok_or_else(|| anyhow::anyhow!("missing tool"))?;
    let result = execute_tool(&tool, args, ctx).await?;
    // Whatever the outcome, no tool output may carry a card number.
    let printed = serde_json::to_string(&(&result.content, &result.structured_content))?;
    anyhow::ensure!(
        !printed.contains(VISA) && !printed.contains(MASTERCARD),
        "{name} leaked a card number"
    );
    Ok(result)
}
fn data(result: &ToolResult) -> anyhow::Result<Value> {
    anyhow::ensure!(
        !result.is_error,
        "payment card tool returned an error: {:?}",
        result.content
    );
    result
        .structured_content
        .clone()
        .ok_or_else(|| anyhow::anyhow!("missing structured result"))
}
/// Chrome never overwrites an earlier fill's fields, so a new fill needs a fresh form.
async fn reload(ctx: &ToolCtx, page: u32) -> anyhow::Result<()> {
    let reloaded = call(ctx, "navigate", json!({"page":page,"action":"reload"})).await?;
    anyhow::ensure!(!reloaded.is_error, "reload failed: {:?}", reloaded.content);
    Ok(())
}
fn error_code(result: &ToolResult) -> Option<Value> {
    result
        .structured_content
        .as_ref()
        .and_then(|value| value.pointer("/error/code"))
        .cloned()
}

#[tokio::test]
#[ignore = "requires BROWSEROS_PAYMENTS_FIXTURE pointing at an owned synthetic browser fixture"]
async fn payment_cards_mcp_live_acceptance() -> anyhow::Result<()> {
    let fixture_path = std::env::var("BROWSEROS_PAYMENTS_FIXTURE")?;
    let fixture: Value = serde_json::from_slice(&std::fs::read(fixture_path)?)?;
    let port = u16::try_from(
        fixture["port"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing port"))?,
    )?;
    let client = CdpClient::connect(ConnectOptions::new(port)).await?;
    // This explicit identity is for the source-built fixture. Release packaging
    // stamps the same first-supported version from resources/BROWSEROS_VERSION.
    let session = BrowserSession::new_with_native(
        Arc::new(client.clone()),
        BrowserSessionHooks::default(),
        NativeCapabilities::from_browseros_version(Some("0.51.2.7")),
    );
    let target = Arc::new(StaticTargetStore::new());
    let ctx = context(session.clone(), target.clone()).await?;
    let listing = ctx.browser.list_pages().await?;
    let page = listing
        .pages
        .iter()
        .find(|page| page.url.starts_with("http://127.0.0.1:"))
        .ok_or_else(|| anyhow::anyhow!("missing fixture page"))?
        .page_id
        .clone();
    let mut examples = Vec::new();

    let visa = json!({"number":"4111 1111 1111 1111","expiryMonth":12,"expiryYear":2030,"name":"Rust Fixture","cvc":"123"});
    let saved = data(
        &call(
            &ctx,
            "payment_cards",
            json!({"action":"add","page":page.0,"card":visa}),
        )
        .await?,
    )?;
    assert!(matches!(
        saved["status"].as_str(),
        Some("saved" | "unchanged")
    ));
    assert_eq!(saved["card"]["network"], "visa");
    assert_eq!(saved["card"]["lastFour"], "1111");
    let card_id = saved["card"]["cardId"].clone();
    examples.push(saved);
    let unchanged = data(&call(&ctx, "payment_cards", json!({"action":"add","card":visa})).await?)?;
    assert_eq!(unchanged["status"], "unchanged");
    assert_eq!(unchanged["card"]["cardId"], card_id);
    examples.push(unchanged);

    let all = data(&call(&ctx, "payment_cards", json!({"action":"list"})).await?)?;
    assert!(all.get("page").is_none());
    assert!(
        all["cards"]
            .as_array()
            .is_some_and(|cards| cards.iter().any(|card| card["cardId"] == card_id))
    );
    examples.push(all);

    let filled = data(
        &call(
            &ctx,
            "payment_cards",
            json!({"action":"fill","page":page.0,"cardId":card_id}),
        )
        .await?,
    )?;
    assert_eq!(filled["source"], "saved");
    assert_eq!(filled["cardId"], card_id);
    assert!(
        filled["filledFields"]
            .as_array()
            .is_some_and(|fields| fields.contains(&json!("number")))
    );
    examples.push(filled);
    // The page now shows the number in a plain text field; a snapshot must not.
    let snapshot = call(&ctx, "snapshot", json!({"page":page.0})).await?;
    assert!(serde_json::to_string(&snapshot.content)?.contains("•••• 1111"));

    let mastercard =
        json!({"number":MASTERCARD,"expiryMonth":7,"expiryYear":2029,"name":"Supplied"});
    let refill = call(
        &ctx,
        "payment_cards",
        json!({"action":"fill","page":page.0,"card":mastercard}),
    )
    .await?;
    assert_eq!(error_code(&refill), Some(json!("form_already_filled")));
    examples.push(
        refill
            .structured_content
            .ok_or_else(|| anyhow::anyhow!("missing refill result"))?,
    );
    reload(&ctx, page.0).await?;
    let supplied = data(
        &call(
            &ctx,
            "payment_cards",
            json!({"action":"fill","page":page.0,"card":mastercard}),
        )
        .await?,
    )?;
    assert_eq!(supplied["source"], "supplied");
    assert_eq!(supplied["network"], "mastercard");
    assert!(supplied.get("cardId").is_none());
    examples.push(supplied);

    let scripted = data(
        &call(
            &ctx,
            "run",
            json!({"code":"return await browser.paymentCards({action:'list'});"}),
        )
        .await?,
    )?;
    assert!(scripted["value"]["cards"].is_array());
    reload(&ctx, page.0).await?;
    let scripted_fill = data(&call(&ctx,"run",json!({"code":format!("return await browser.page({}).paymentCards({{action:'fill',cardId:{}}});",page.0,card_id)})).await?)?;
    assert_eq!(scripted_fill["value"]["source"], "saved");

    for args in [
        json!({"action":"fill","page":page.0,"cardId":card_id,"card":visa}),
        json!({"action":"fill","card":visa}),
        json!({"action":"list","card":visa}),
    ] {
        let rejected = call(&ctx, "payment_cards", args).await?;
        assert!(rejected.is_error);
        assert!(
            rejected
                .audit_hints
                .is_some_and(|hints| hints.skip_screenshot)
        );
    }
    let invalid = call(&ctx,"payment_cards",json!({"action":"add","card":{"number":"4111 1111 1111 1112","expiryMonth":12,"expiryYear":2030}})).await?;
    assert_eq!(error_code(&invalid), Some(json!("invalid_card")));
    let unknown = call(
        &ctx,
        "payment_cards",
        json!({"action":"fill","page":page.0,"cardId":"card_unknown"}),
    )
    .await?;
    assert_eq!(error_code(&unknown), Some(json!("card_not_found")));
    examples.push(
        unknown
            .structured_content
            .ok_or_else(|| anyhow::anyhow!("missing error result"))?,
    );

    if let Ok(path) = std::env::var("BROWSEROS_PAYMENTS_RESULTS") {
        let tool = catalog()
            .into_iter()
            .find(|tool| tool.name == "payment_cards")
            .ok_or_else(|| anyhow::anyhow!("missing declaration"))?;
        std::fs::write(
            path,
            serde_json::to_vec_pretty(
                &json!({"inputSchema":tool.input_schema,"outputSchema":tool.output_schema,"examples":examples}),
            )?,
        )?;
    }
    client.disconnect().await;
    Ok(())
}
