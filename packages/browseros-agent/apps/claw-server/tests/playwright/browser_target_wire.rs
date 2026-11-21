//! Verify that the Playwright tool list and refusals reach the actual HTTP wire.
//! This fixture uses the same disposable native peer as the catalog tests.

use super::*;
use claw_server::config::{Behavior, ServerMode};

#[tokio::test]
async fn desktop_targeting_tools_are_unlisted_and_refused_without_starting_a_task()
-> anyhow::Result<()> {
    let peer = HttpPeer::start_configured(|config| {
        config.mode = ServerMode::Desktop;
        config.behavior = Behavior {
            playwright_run: true,
            ..Behavior::defaults(ServerMode::Desktop)
        };
    })
    .await?;
    let tools = peer.request("tools/list", json!({})).await?;
    let names = tools["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("tools missing: {tools}"))?
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["run", "request_human_help", "await_human_help"]);
    for name in ["browser_profiles", "browser_target"] {
        let refusal = peer
            .request_with_tool_errors(
                "tools/call",
                &mut json!({"name":name, "arguments":{"action":"get"}}),
                true,
            )
            .await?;
        assert_eq!(refusal["isError"], true, "{refusal}");
        assert!(
            refusal["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("run")),
            "{refusal}"
        );
        assert!(
            refusal["_meta"]["com.browseros.neo/session"].is_null(),
            "{refusal}"
        );
    }
    Ok(())
}
