use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AxValue {
    #[serde(rename = "type")]
    pub value_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// How Chromium computed a name, in the order it tried. Only the parts the snapshot reads
    /// are kept: which native source named the node, and the nodes it named it from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<AxValueSource>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AxValueSource {
    /// For example "labelfor" or "labelwrapped": the control is named by a `<label>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_source_value: Option<AxRelatedValue>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AxRelatedValue {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_nodes: Vec<AxRelatedNode>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AxRelatedNode {
    #[serde(rename = "backendDOMNodeId", skip_serializing_if = "Option::is_none")]
    pub backend_dom_node_id: Option<i64>,
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
            sources: None,
        }
    }

    #[must_use]
    pub fn role(value: impl Into<String>) -> Self {
        Self {
            value_type: "role".to_string(),
            value: Some(Value::String(value.into())),
            sources: None,
        }
    }
}
