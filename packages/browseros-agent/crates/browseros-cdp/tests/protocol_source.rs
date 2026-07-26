use browseros_cdp::{browser, page, runtime, target};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{error::Error, io};

const PROTOCOL: &[u8] = include_bytes!("../protocol/protocol.json");
const EXPECTED_SHA256: &str = include_str!("../protocol/protocol.sha256");
const SURFACE: &[u8] = include_bytes!("../protocol/surface.json");

#[test]
fn protocol_source_matches_pinned_chromium_input() -> Result<(), Box<dyn Error>> {
    let digest = format!("{:x}", Sha256::digest(PROTOCOL));
    assert_eq!(digest, EXPECTED_SHA256.trim());

    let protocol: Value = serde_json::from_slice(PROTOCOL)?;
    assert_eq!(
        protocol["domains"].as_array().map(Vec::len),
        Some(58),
        "the pinned ch1 combined protocol has 58 domains"
    );
    let surface: Value = serde_json::from_slice(SURFACE)?;
    assert_eq!(
        surface["domains"].as_array().map(Vec::len),
        Some(9),
        "the compiled Rust surface should remain focused"
    );
    Ok(())
}

#[test]
fn protocol_source_retains_hidden_compatibility_tombstones() -> Result<(), Box<dyn Error>> {
    let protocol: Value = serde_json::from_slice(PROTOCOL)?;
    let domains = protocol["domains"]
        .as_array()
        .ok_or_else(|| io::Error::other("protocol should contain domains"))?;
    let browser = domains
        .iter()
        .find(|domain| domain["domain"] == "Browser")
        .ok_or_else(|| io::Error::other("Browser domain should exist"))?;

    assert!(has_property(browser, "types", "TabInfo", "isHidden"));
    assert!(has_property(browser, "commands", "createWindow", "hidden"));
    assert!(has_property(
        browser,
        "commands",
        "getTabs",
        "includeHidden"
    ));
    assert!(has_member(browser, "commands", "showTab"));
    assert!(has_member(browser, "commands", "setWindowVisibility"));
    Ok(())
}

#[test]
fn generated_surface_uses_canonical_identifier_names() {
    let window_id: browser::WindowID = 7;
    let target_id: target::TargetID = "target-7".to_string();

    assert_eq!(window_id, 7);
    assert_eq!(target_id, "target-7");
}

#[test]
fn generated_surface_includes_runtime_members_and_tombstones() -> Result<(), Box<dyn Error>> {
    let _dialog = page::JavascriptDialogOpeningEvent::default();
    let _console = runtime::ConsoleAPICalledEvent::default();
    let _show_tab = browser::ShowTabResult::default();
    let params = browser::CreateWindowParams {
        hidden: Some(true),
        ..browser::CreateWindowParams::default()
    };

    assert_eq!(serde_json::to_value(params)?["hidden"], true);
    Ok(())
}

fn has_member(domain: &Value, collection: &str, name: &str) -> bool {
    domain[collection]
        .as_array()
        .is_some_and(|members| members.iter().any(|member| member["name"] == name))
}

fn has_property(domain: &Value, collection: &str, name: &str, property: &str) -> bool {
    domain[collection].as_array().is_some_and(|members| {
        members.iter().any(|member| {
            let member_name = member
                .get("id")
                .or_else(|| member.get("name"))
                .and_then(Value::as_str);
            member_name == Some(name)
                && member["properties"]
                    .as_array()
                    .or_else(|| member["parameters"].as_array())
                    .is_some_and(|properties| {
                        properties
                            .iter()
                            .any(|candidate| candidate["name"] == property)
                    })
        })
    })
}
