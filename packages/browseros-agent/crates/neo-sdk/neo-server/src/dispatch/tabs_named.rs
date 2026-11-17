//! The tabs a JSON value names through its page brands.

use browseros_core::TabId;
use serde_json::Value;

/// Adds the tab of every page brand in `value`, `{"$neo":"page","tab":1234}`, to `tabs`, once
/// each: a top-level target, a brand deep in an op's arguments, a locator's page, an event's
/// page, a popup's new tab or a new tab's opener. A shape added later is covered as well, so the
/// door can ask about every tab an op's input or result names without knowing its method.
pub(crate) fn tabs_named(value: &Value, tabs: &mut Vec<TabId>) {
    match value {
        Value::Object(fields) => {
            let page = fields.get("$neo").and_then(Value::as_str) == Some("page");
            if let (true, Some(tab)) = (page, fields.get("tab").and_then(Value::as_i64))
                && !tabs.contains(&TabId(tab))
            {
                tabs.push(TabId(tab));
            }
            for field in fields.values() {
                tabs_named(field, tabs);
            }
        }
        Value::Array(items) => {
            for item in items {
                tabs_named(item, tabs);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}
