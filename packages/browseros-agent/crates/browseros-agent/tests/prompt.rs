//! Prompt strings are compared byte-for-byte with outputs captured from the TS functions.
use browseros_agent::prompt::*;
use serde_json::Value;
use std::{collections::BTreeSet, path::PathBuf};

fn fixtures() -> Result<Value, serde_json::Error> {
    serde_json::from_str(include_str!("../src/prompt/golden.json"))
}
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
fn path(value: &Value) -> Option<PathBuf> {
    value.as_str().map(PathBuf::from)
}
fn tab(value: &Value) -> TabView {
    TabView {
        id: value["id"].as_i64().unwrap_or_default(),
        page_id: value["pageId"].as_u64().map(|id| id as u32),
        url: value["url"].as_str().map(str::to_owned),
        title: value["title"].as_str().map(str::to_owned),
    }
}
fn session(value: &Value) -> SessionInputs {
    SessionInputs {
        mcp_server_key: value["mcpServerKey"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        working_dir: path(&value["workingDir"]),
        chat_mode: value["chatMode"] == true,
    }
}

#[test]
fn system_prompt_matches_all_ts_modes_byte_for_byte() -> Result<(), Box<dyn std::error::Error>> {
    for case in fixtures()?["prompts"].as_array().ok_or("prompts")? {
        let options = &case["options"];
        let inputs = PromptInputs {
            chat_mode: options["chatMode"] == true,
            is_scheduled_task: options["isScheduledTask"] == true,
            origin: if options["origin"] == "newtab" {
                Origin::Newtab
            } else {
                Origin::Sidepanel
            },
            workspace_dir: path(&options["workspaceDir"]),
            connected_apps: strings(&options["connectedApps"]),
            declined_apps: strings(&options["declinedApps"]),
            user_system_prompt: options["userSystemPrompt"].as_str().map(str::to_owned),
            active_tab_page_id: options["scheduledTaskPageId"].as_u64().map(|id| id as u32),
        };
        let tools: BTreeSet<_> = strings(&case["toolNames"]).into_iter().collect();
        assert_eq!(
            system_prompt(&inputs, &tools).as_bytes(),
            case["expected"].as_str().ok_or("expected")?.as_bytes(),
            "{options}"
        );
    }
    Ok(())
}

#[test]
fn user_wrapper_and_selection_sanitization_match_ts() -> Result<(), Box<dyn std::error::Error>> {
    for case in fixtures()?["formats"].as_array().ok_or("formats")? {
        let ctx = case.get("ctx").map(|value| BrowserContextView {
            window_id: value["windowId"].as_i64(),
            active_tab: value.get("activeTab").map(tab),
            selected_tabs: value["selectedTabs"]
                .as_array()
                .map(|tabs| tabs.iter().map(tab).collect())
                .unwrap_or_default(),
            ..Default::default()
        });
        let source = case
            .get("source")
            .and_then(|s| Some((s["url"].as_str()?, s["title"].as_str()?)));
        assert_eq!(
            format_user_message(
                case["message"].as_str().ok_or("message")?,
                ctx.as_ref(),
                case["selectedText"].as_str(),
                source
            ),
            case["expected"].as_str().ok_or("expected")?
        );
    }
    Ok(())
}

#[test]
fn context_changes_match_ts_sentences_and_order() -> Result<(), Box<dyn std::error::Error>> {
    for case in fixtures()?["sessions"].as_array().ok_or("sessions")? {
        assert_eq!(
            context_notices(&session(&case["before"]), &session(&case["after"])),
            strings(&case["expected"])
        );
    }
    Ok(())
}

#[test]
fn mcp_key_matches_ts_sorting_duplicates_and_klavis_default() {
    let ctx = BrowserContextView {
        enabled_mcp_servers: vec!["Slack".into(), "Gmail".into(), "Gmail".into()],
        custom_mcp_urls: vec!["https://z".into(), "https://a".into()],
        ..Default::default()
    };
    assert_eq!(
        mcp_server_key(Some(&ctx), None),
        "klavis:disabled,Gmail,Gmail,Slack,https://a,https://z"
    );
    assert_eq!(
        mcp_server_key(Some(&ctx), Some("ready")),
        "klavis:ready,Gmail,Gmail,Slack,https://a,https://z"
    );
    assert_eq!(mcp_server_key(None, Some("ready")), "");
}
