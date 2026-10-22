//! Wire contracts for the two management tools. Hosts retain structured data
//! only for tools declaring an output schema; their short text summaries cannot
//! carry saved/live identity, placement, or binding revision for clients.
//! Core owns identity serialization; these schemas describe that wire shape
//! without adding an MCP schema dependency to the shared runtime.

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde_json::{Value, json};
use std::borrow::Cow;

pub(super) struct ProfilesOutput;
pub(super) struct TargetOutput;

/// Core's serialized descriptor omits an absent live context. Saved identity
/// and directory remain available for unloaded profiles and recovery workflows.
fn profile() -> Value {
    json!({
        "type": "object",
        "properties": {
            "browserProfileId": { "type": "string" },
            "name": { "type": "string" },
            "directory": { "type": "string" },
            "loaded": { "type": "boolean" },
            "browserContextId": { "type": "string" }
        },
        "required": ["browserProfileId", "name", "directory", "loaded"]
    })
}

fn binding() -> Value {
    json!({
        "type": "object",
        "properties": {
            "browserProfileId": { "type": "string" },
            "name": { "type": "string" },
            "directory": { "type": "string" },
            "revision": { "type": "integer", "minimum": 0 },
            "window": {
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["working", "pinned"] },
                    "windowId": { "type": "integer" }
                },
                "required": ["mode", "windowId"]
            }
        },
        "required": ["browserProfileId", "name", "directory", "revision"]
    })
}

fn status() -> Value {
    // These fields are present even when native status cannot resolve the
    // selected profile or window. Null differs from the omitted binding.window.
    json!({
        "type": "object",
        "properties": {
            "profile": { "anyOf": [profile(), { "type": "null" }] },
            "windows": { "type": "array", "items": { "type": "integer" } },
            "placementWindowId": { "type": ["integer", "null"] },
            "error": { "type": ["string", "null"] }
        },
        "required": ["profile", "windows", "placementWindowId", "error"]
    })
}

impl JsonSchema for ProfilesOutput {
    fn schema_name() -> Cow<'static, str> {
        "BrowserProfilesOutput".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "object",
            "oneOf": [
                {
                    "properties": {
                        "action": { "type": "string", "const": "list" },
                        "profiles": { "type": "array", "items": profile() },
                        "boundBrowserProfileId": { "type": "string" }
                    },
                    "required": ["action", "profiles"]
                },
                {
                    "properties": {
                        "action": { "type": "string", "const": "load" },
                        "profile": profile(),
                        "loaded": { "type": "boolean" }
                    },
                    "required": ["action", "profile", "loaded"]
                }
            ]
        })
    }
}

impl JsonSchema for TargetOutput {
    fn schema_name() -> Cow<'static, str> {
        "BrowserTargetOutput".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "object",
            "oneOf": [
                {
                    "properties": {
                        "action": { "type": "string", "const": "get" },
                        "bound": { "type": "boolean" },
                        "binding": binding(),
                        "status": status(),
                        // Present only without a stored binding: the Default profile that
                        // unselected calls use, as resolved by the reporting call itself.
                        "default": {
                            "type": "object",
                            "properties": {
                                "directory": { "type": "string" },
                                "status": status()
                            },
                            "required": ["directory", "status"]
                        }
                    },
                    "required": ["action", "bound"],
                    "dependentRequired": { "binding": ["status"], "status": ["binding"] }
                },
                {
                    "properties": {
                        "action": { "type": "string", "enum": ["set", "set_window", "clear_window"] },
                        "binding": binding(),
                        "previous": binding()
                    },
                    "required": ["action", "binding"]
                },
                {
                    "properties": {
                        "action": { "type": "string", "const": "clear" },
                        "bound": { "type": "boolean" }
                    },
                    "required": ["action", "bound"]
                }
            ]
        })
    }
}
