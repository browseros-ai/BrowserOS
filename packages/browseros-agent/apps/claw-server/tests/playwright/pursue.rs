//! The pursue MCP handoff on the Playwright run surface, where page numbers are
//! native tab IDs. All requests still go through the parent's HTTP fixtures.

use super::*;

async fn playwright_app(setup: Setup) -> anyhow::Result<TestApp> {
    test_app_with_behavior(
        setup,
        Behavior {
            playwright_run: true,
            ..Behavior::defaults(setup.mode)
        },
    )
    .await
}

async fn call_pursue(app: &TestApp, session: &str) -> anyhow::Result<Value> {
    let (status, _, body) = post(
        &app.router,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "pursue",
                "arguments": { "page": 4294967301_i64, "goal": "add Sensodyne Fresh Mint 150g to the cart" }
            }
        }),
        Some(session),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    Ok(body)
}

#[tokio::test]
async fn a_lapsed_plan_hands_back_to_run_without_asking_the_worker() -> anyhow::Result<()> {
    let app = playwright_app(Setup {
        ultrafast: false,
        ..ULTRAFAST_DESKTOP
    })
    .await?;
    let session = connect(&app).await?;

    let body = call_pursue(&app, &session).await?;

    assert_eq!(body["result"]["isError"], false, "{body:?}");
    let reply = &body["result"]["structuredContent"];
    assert_eq!(reply["outcome"], "handed_back");
    assert_eq!(reply["continueWith"], "run");
    assert!(
        reply["said"]
            .as_str()
            .unwrap_or_default()
            .contains("not on this plan")
    );
    assert!(app.worker.requests().is_empty());
    Ok(())
}

#[tokio::test]
async fn with_no_browser_connected_pursue_hands_back_before_asking_anything() -> anyhow::Result<()>
{
    let app = playwright_app(ULTRAFAST_DESKTOP).await?;
    let session = connect(&app).await?;

    let body = call_pursue(&app, &session).await?;

    assert_eq!(body["result"]["isError"], false, "{body:?}");
    let reply = &body["result"]["structuredContent"];
    assert_eq!(reply["outcome"], "handed_back");
    assert_eq!(reply["decisions"], 0);
    assert_eq!(
        reply["page"], 4294967301_i64,
        "native tab ids are not host u32 page ids"
    );
    assert!(app.worker.requests().is_empty());
    Ok(())
}

#[tokio::test]
async fn once_pursue_has_handed_a_task_back_it_does_not_take_it_again() -> anyhow::Result<()> {
    let app = playwright_app(ULTRAFAST_DESKTOP).await?;
    let session = connect(&app).await?;
    let other = connect(&app).await?;
    app.state
        .sessions
        .lookup(&SessionId::new(session.clone()))
        .await
        .ok_or_else(|| anyhow::anyhow!("no session"))?
        .mark_pursue_handed_back();

    let body = call_pursue(&app, &session).await?;
    let reply = &body["result"]["structuredContent"];
    assert_eq!(body["result"]["isError"], false, "{body:?}");
    assert_eq!(reply["outcome"], "handed_back");
    assert_eq!(reply["status"], "already_handed_back");
    assert_eq!(reply["continueWith"], "run");

    let elsewhere = call_pursue(&app, &other).await?;
    assert_ne!(
        elsewhere["result"]["structuredContent"]["status"], "already_handed_back",
        "another task still gets its own pursue"
    );
    assert!(app.worker.requests().is_empty());
    Ok(())
}

#[tokio::test]
async fn naming_a_new_task_offers_pursue_again() -> anyhow::Result<()> {
    let app = playwright_app(ULTRAFAST_DESKTOP).await?;
    let session = connect(&app).await?;
    let task = app
        .state
        .sessions
        .lookup(&SessionId::new(session.clone()))
        .await
        .ok_or_else(|| anyhow::anyhow!("no session"))?;
    // The after-run naming projection uses Session::rename. This route checks
    // that pursue observes its reset without needing an SDK process in the harness.
    task.rename("cart-errand".to_string()).await;
    task.mark_pursue_handed_back();
    let locked = call_pursue(&app, &session).await?;
    assert_eq!(
        locked["result"]["structuredContent"]["status"],
        "already_handed_back"
    );
    task.rename("flight-search".to_string()).await;
    let fresh = call_pursue(&app, &session).await?;
    assert_ne!(
        fresh["result"]["structuredContent"]["status"], "already_handed_back",
        "{fresh:?}"
    );
    Ok(())
}

#[tokio::test]
async fn agents_receive_the_current_instructions_for_their_pursue_gate() -> anyhow::Result<()> {
    for ultrafast in [true, false] {
        let app = playwright_app(Setup {
            ultrafast,
            ..ULTRAFAST_DESKTOP
        })
        .await?;
        let (_, instructions) = connect_reading_instructions(&app).await?;
        assert_eq!(
            instructions.contains("Get its numeric tab id from page.tabId."),
            ultrafast,
            "{instructions}"
        );
        assert_eq!(
            instructions.contains("Use run for browser work."),
            !ultrafast,
            "{instructions}"
        );
    }
    Ok(())
}
