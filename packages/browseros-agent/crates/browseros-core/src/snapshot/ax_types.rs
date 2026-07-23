use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AxValue {
    #[serde(rename = "type")]
    pub value_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AxProperty {
    pub name: String,
    pub value: AxValue,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AxNode {
    pub node_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignored: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<AxValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<AxValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<AxValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<AxValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<Vec<AxProperty>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_ids: Option<Vec<String>>,
    #[serde(rename = "backendDOMNodeId", skip_serializing_if = "Option::is_none")]
    pub backend_dom_node_id: Option<i64>,
}

impl AxValue {
    #[must_use]
    pub fn string(value: impl Into<String>) -> Self {
        Self {
            value_type: "computedString".to_string(),
            value: Some(Value::String(value.into())),
        }
    }

    #[must_use]
    pub fn role(value: impl Into<String>) -> Self {
        Self {
            value_type: "role".to_string(),
            value: Some(Value::String(value.into())),
        }
    }
}
