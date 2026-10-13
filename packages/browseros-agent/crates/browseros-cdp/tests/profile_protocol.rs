//! Guard the saved-profile/live-context seam across the native snapshot and
//! generated Rust bindings. A saved handle remains useful before a live context exists.

use browseros_cdp::{browser, history};
use serde_json::{Value, json};
use std::{collections::BTreeSet, error::Error, io};

const PROTOCOL: &[u8] = include_bytes!("../protocol/protocol.json");

#[test]
fn native_profile_contract_keeps_catalog_handles_separate_from_contexts()
-> Result<(), Box<dyn Error>> {
    let protocol: Value = serde_json::from_slice(PROTOCOL)?;
    let browser = member(&protocol, "domains", "domain", "Browser")?;
    let id = member(browser, "types", "id", "BrowserProfileID")?;
    assert_eq!(id["type"], "string");

    let info = member(browser, "types", "id", "BrowserProfileInfo")?;
    let properties = info["properties"]
        .as_array()
        .ok_or_else(|| io::Error::other("BrowserProfileInfo needs properties"))?;
    assert_eq!(
        properties
            .iter()
            .filter_map(|property| property["name"].as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "browserProfileId",
            "name",
            "directory",
            "loaded",
            "browserContextId"
        ])
    );
    assert_field(
        info,
        "properties",
        "browserProfileId",
        "$ref",
        "BrowserProfileID",
        false,
    )?;
    assert_field(info, "properties", "name", "type", "string", false)?;
    assert_field(info, "properties", "directory", "type", "string", false)?;
    assert_field(info, "properties", "loaded", "type", "boolean", false)?;
    assert_field(
        info,
        "properties",
        "browserContextId",
        "$ref",
        "BrowserContextID",
        true,
    )?;

    let get = member(browser, "commands", "name", "getProfiles")?;
    assert!(get["parameters"].as_array().is_none_or(Vec::is_empty));
    let profiles = member(get, "returns", "name", "profiles")?;
    assert_eq!(profiles["type"], "array");
    assert_eq!(profiles["items"]["$ref"], "BrowserProfileInfo");

    let load = member(browser, "commands", "name", "loadProfile")?;
    assert_eq!(load["parameters"].as_array().map(Vec::len), Some(1));
    assert_field(
        load,
        "parameters",
        "browserProfileId",
        "$ref",
        "BrowserProfileID",
        false,
    )?;
    assert_field(
        load,
        "returns",
        "profile",
        "$ref",
        "BrowserProfileInfo",
        false,
    )?;
    assert_field(load, "returns", "loaded", "type", "boolean", false)?;
    Ok(())
}

#[test]
fn generated_profile_results_preserve_closed_loaded_and_reused_states() -> Result<(), Box<dyn Error>>
{
    let profiles: browser::GetProfilesResult = serde_json::from_value(json!({
        "profiles": [{
            "browserProfileId": "saved-eng", "name": "eng", "directory": "Profile 1", "loaded": false
        }]
    }))?;
    assert!(!profiles.profiles[0].loaded);
    assert!(profiles.profiles[0].browser_context_id.is_none());
    assert!(
        serde_json::to_value(&profiles)?["profiles"][0]
            .get("browserContextId")
            .is_none()
    );

    let handle: browser::BrowserProfileID = profiles.profiles[0].browser_profile_id.clone();
    let request = browser::LoadProfileParams {
        browser_profile_id: handle,
    };
    assert_eq!(
        serde_json::to_value(request)?,
        json!({"browserProfileId": "saved-eng"})
    );

    // Reusing an already loaded profile must not collapse the operation's result
    // flag into the catalog entry's loaded state; they have different lifetimes.
    let result: browser::LoadProfileResult = serde_json::from_value(json!({
        "profile": {
            "browserProfileId": "saved-eng", "name": "eng", "directory": "Profile 1",
            "loaded": true, "browserContextId": "live-eng"
        },
        "loaded": false
    }))?;
    assert!(!result.loaded);
    assert!(result.profile.loaded);
    assert_eq!(
        result.profile.browser_context_id.as_deref(),
        Some("live-eng")
    );
    Ok(())
}

#[test]
fn native_metadata_and_history_route_with_optional_genuine_context_ids()
-> Result<(), Box<dyn Error>> {
    let protocol: Value = serde_json::from_slice(PROTOCOL)?;
    let browser = member(&protocol, "domains", "domain", "Browser")?;
    for name in ["TabInfo", "WindowInfo"] {
        let info = member(browser, "types", "id", name)?;
        // Preserve the old field while exposing two explicitly different identities.
        assert_field(
            info,
            "properties",
            "browserContextId",
            "$ref",
            "BrowserContextID",
            true,
        )?;
        assert_field(
            info,
            "properties",
            "browserProfileId",
            "$ref",
            "BrowserProfileID",
            true,
        )?;
        assert_field(
            info,
            "properties",
            "liveBrowserContextId",
            "$ref",
            "BrowserContextID",
            true,
        )?;
    }
    for name in ["createTab", "createWindow"] {
        let command = member(browser, "commands", "name", name)?;
        assert_field(
            command,
            "parameters",
            "browserContextId",
            "$ref",
            "BrowserContextID",
            true,
        )?;
    }
    let history = member(&protocol, "domains", "domain", "History")?;
    for name in ["search", "getRecent", "deleteUrl", "deleteRange"] {
        let command = member(history, "commands", "name", name)?;
        assert_field(
            command,
            "parameters",
            "browserContextId",
            "$ref",
            "Browser.BrowserContextID",
            true,
        )?;
    }
    Ok(())
}

#[test]
fn generated_metadata_preserves_legacy_labels_alongside_routable_identities()
-> Result<(), Box<dyn Error>> {
    let window = browser::WindowInfo {
        browser_context_id: Some("Profile 1".to_string()),
        browser_profile_id: Some("saved-eng".to_string()),
        live_browser_context_id: Some("live-eng".to_string()),
        ..Default::default()
    };
    let tab = browser::TabInfo {
        browser_context_id: window.browser_context_id.clone(),
        browser_profile_id: window.browser_profile_id.clone(),
        live_browser_context_id: window.live_browser_context_id.clone(),
        ..Default::default()
    };
    for info in [serde_json::to_value(&window)?, serde_json::to_value(&tab)?] {
        assert_eq!(info["browserContextId"], "Profile 1");
        assert_eq!(info["browserProfileId"], "saved-eng");
        assert_eq!(info["liveBrowserContextId"], "live-eng");
    }

    // Older browsers still return the legacy label alone. The new optional
    // identities must not make those records fail deserialization.
    let mut old_window = serde_json::to_value(window)?;
    let mut old_tab = serde_json::to_value(tab)?;
    for info in [&mut old_window, &mut old_tab] {
        let properties = info
            .as_object_mut()
            .ok_or_else(|| io::Error::other("metadata must be an object"))?;
        properties.remove("browserProfileId");
        properties.remove("liveBrowserContextId");
    }
    let window: browser::WindowInfo = serde_json::from_value(old_window)?;
    let tab: browser::TabInfo = serde_json::from_value(old_tab)?;
    assert_eq!(window.browser_context_id.as_deref(), Some("Profile 1"));
    assert_eq!(tab.browser_context_id.as_deref(), Some("Profile 1"));
    assert!(window.browser_profile_id.is_none() && window.live_browser_context_id.is_none());
    assert!(tab.browser_profile_id.is_none() && tab.live_browser_context_id.is_none());
    Ok(())
}

#[test]
fn generated_history_requests_preserve_routing_and_unbound_compatibility()
-> Result<(), Box<dyn Error>> {
    let context = Some("live-eng".to_string());
    let scoped = [
        serde_json::to_value(history::SearchParams {
            browser_context_id: context.clone(),
            ..Default::default()
        })?,
        serde_json::to_value(history::GetRecentParams {
            browser_context_id: context.clone(),
            ..Default::default()
        })?,
        serde_json::to_value(history::DeleteUrlParams {
            browser_context_id: context.clone(),
            ..Default::default()
        })?,
        serde_json::to_value(history::DeleteRangeParams {
            browser_context_id: context,
            ..Default::default()
        })?,
    ];
    for request in scoped {
        assert_eq!(request["browserContextId"], "live-eng");
        assert!(request.get("browserProfileId").is_none());
    }
    let unbound = [
        serde_json::to_value(history::SearchParams::default())?,
        serde_json::to_value(history::GetRecentParams::default())?,
        serde_json::to_value(history::DeleteUrlParams::default())?,
        serde_json::to_value(history::DeleteRangeParams::default())?,
    ];
    for request in unbound {
        assert!(request.get("browserContextId").is_none());
    }
    Ok(())
}

fn member<'a>(
    parent: &'a Value,
    collection: &str,
    key: &str,
    name: &str,
) -> Result<&'a Value, io::Error> {
    parent[collection]
        .as_array()
        .and_then(|members| members.iter().find(|member| member[key] == name))
        .ok_or_else(|| io::Error::other(format!("missing {collection} member {name}")))
}

fn assert_field(
    parent: &Value,
    collection: &str,
    name: &str,
    key: &str,
    value: &str,
    optional: bool,
) -> Result<(), io::Error> {
    let field = member(parent, collection, "name", name)?;
    assert_eq!(field[key], value, "wire type of {name}");
    assert_eq!(
        field["optional"].as_bool().unwrap_or(false),
        optional,
        "optionality of {name}"
    );
    Ok(())
}
