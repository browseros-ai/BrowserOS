//! Opt-in acceptance through real MCP dispatch and an owned patched Chromium.
//! The fixture contains synthetic accounts only. It also exercises task profile
//! rebinding, so this test must never point at a user's normal browser session.
use browseros_cdp::{CdpClient, ConnectOptions};
use browseros_core::{BrowserSession, BrowserSessionHooks, BrowserView, NativeCapabilities};
use browseros_mcp::{
    BrowserToolDefaults, BrowserToolOptions, StaticTargetStore, TargetStore, ToolCtx, ToolResult,
    catalog, execute_tool, output_file::create_browser_output_file_access,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

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
    Ok(execute_tool(&tool, args, ctx).await?)
}
fn data(result: &ToolResult) -> anyhow::Result<Value> {
    anyhow::ensure!(
        !result.is_error,
        "credential tool returned an error: {:?}",
        result.content
    );
    result
        .structured_content
        .clone()
        .ok_or_else(|| anyhow::anyhow!("missing structured result"))
}

#[tokio::test]
#[ignore = "requires BROWSEROS_CREDENTIALS_FIXTURE pointing at an owned synthetic browser fixture"]
async fn credentials_mcp_live_profile_and_fill_acceptance() -> anyhow::Result<()> {
    let fixture_path = std::env::var("BROWSEROS_CREDENTIALS_FIXTURE")?;
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
        NativeCapabilities::from_browseros_version(Some("0.51.2.1")),
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
    let username = format!("rust-{}@example.test", uuid::Uuid::new_v4());
    let mut examples = Vec::new();
    let all = data(&call(&ctx, "credentials", json!({"action":"list"})).await?)?;
    assert_eq!(all["browserProfileId"], fixture["profileIds"][0]);
    assert!(all.get("page").is_none());
    assert!(all.get("scope").is_none());
    examples.push(all.clone());
    let site = data(
        &call(
            &ctx,
            "credentials",
            json!({"action":"list","page":page.0,"query":"ALICE"}),
        )
        .await?,
    )?;
    assert_eq!(site["total"], 1);
    assert_eq!(site["page"], page.0);
    examples.push(site.clone());
    let saved = data(&call(&ctx,"credentials",json!({"action":"add","page":page.0,"username":username,"password":"synthetic-rust-password"})).await?)?;
    assert_eq!(saved["status"], "saved");
    let credential_id = saved["credential"]["credentialId"].clone();
    examples.push(saved);
    let unchanged = data(&call(&ctx,"credentials",json!({"action":"add","page":page.0,"username":username,"password":"synthetic-rust-password"})).await?)?;
    assert_eq!(unchanged["status"], "unchanged");
    assert_eq!(unchanged["credential"]["credentialId"], credential_id);
    examples.push(unchanged);
    let fill = call(
        &ctx,
        "credentials",
        json!({"action":"fill","page":page.0,"credentialId":credential_id}),
    )
    .await?;
    assert_eq!(
        fill.content.len(),
        1,
        "credential filling must not append readbacks"
    );
    let filled = data(&fill)?;
    assert_eq!(filled["source"], "saved");
    assert_eq!(filled["filledFields"], json!(["username", "password"]));
    examples.push(filled);
    let direct = data(&call(&ctx,"credentials",json!({"action":"fill","page":page.0,"username":"unsaved-rust","password":"synthetic-unsaved-password"})).await?)?;
    assert_eq!(direct["source"], "provided");
    assert!(direct.get("credentialId").is_none());
    examples.push(direct);
    let scripted = data(
        &call(
            &ctx,
            "run",
            json!({"code":"return await browser.credentials({action:'list'});"}),
        )
        .await?,
    )?;
    assert_eq!(
        scripted["value"]["browserProfileId"],
        fixture["profileIds"][0]
    );
    let scripted_fill=data(&call(&ctx,"run",json!({"code":format!("return await browser.page({}).credentials({{action:'fill',credentialId:{}}});",page.0,credential_id)})).await?)?;
    assert_eq!(scripted_fill["value"]["source"], "saved");
    let after = data(&call(&ctx, "credentials", json!({"action":"list"})).await?)?;
    assert_eq!(
        after["total"].as_u64(),
        all["total"].as_u64().map(|n| n + 1)
    );
    for args in [
        json!({"action":"list","scope":"profile"}),
        json!({"action":"fill","page":page.0,"credentialId":credential_id,"password":"synthetic-error-password"}),
        json!({"action":"fill","page":page.0,"password":"synthetic-error-password"}),
        json!({"action":"add","page":page.0,"username":"empty","password":""}),
        json!({"action":"list","page":null}),
    ] {
        let rejected = call(&ctx, "credentials", args).await?;
        assert!(rejected.is_error);
        assert!(!serde_json::to_string(&rejected.content)?.contains("synthetic-error-password"));
        assert!(
            rejected
                .audit_hints
                .is_some_and(|hints| hints.skip_screenshot)
        );
    }
    let conflict = call(
        &ctx,
        "credentials",
        json!({"action":"add","page":page.0,"username":username,"password":"synthetic-conflict"}),
    )
    .await?;
    assert!(conflict.is_error);
    assert_eq!(
        conflict
            .structured_content
            .as_ref()
            .and_then(|value| value.pointer("/error/code")),
        Some(&json!("credential_conflict"))
    );
    examples.push(
        conflict
            .structured_content
            .ok_or_else(|| anyhow::anyhow!("missing conflict result"))?,
    );
    data(
        &call(
            &ctx,
            "browser_target",
            json!({"action":"set","browserProfileId":fixture["profileIds"][1]}),
        )
        .await?,
    )?;
    let other = context(session.clone(), target.clone()).await?;
    let other_list = data(&call(&other, "credentials", json!({"action":"list"})).await?)?;
    assert_eq!(other_list["browserProfileId"], fixture["profileIds"][1]);
    assert_eq!(other_list["total"], 0);
    let blocked = call(
        &other,
        "credentials",
        json!({"action":"fill","page":page.0,"credentialId":credential_id}),
    )
    .await?;
    assert!(blocked.is_error);
    // A previously captured call keeps its own profile when the host rebinds.
    assert_eq!(
        data(&call(&ctx, "credentials", json!({"action":"list"})).await?)?["browserProfileId"],
        fixture["profileIds"][0]
    );
    data(&call(&other, "browser_target", json!({"action":"clear"})).await?)?;
    let cleared = context(session, target).await?;
    assert_eq!(
        data(&call(&cleared, "credentials", json!({"action":"list"})).await?)?["browserProfileId"],
        fixture["profileIds"][0]
    );
    if let Ok(path) = std::env::var("BROWSEROS_CREDENTIALS_RESULTS") {
        let tool = catalog()
            .into_iter()
            .find(|tool| tool.name == "credentials")
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
