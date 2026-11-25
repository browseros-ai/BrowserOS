use std::collections::BTreeMap;
use std::mem;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Number;

use crate::aria_node::{Invalid, Tristate};

/// One node of a parsed Playwright aria snapshot template: Playwright's `AriaTemplateNode` at
/// 1.64.1 (vendored `isomorphic/ariaSnapshot.ts`).
///
/// Templates cross the wire as JSON tagged by `"kind": "role"` or `"kind": "text"`. Every inner
/// struct rejects unknown fields and `null` on optional fields so malformed templates fail at
/// decode time rather than silently degrading during matching.
///
/// Depth: the derived decode, encode, clone, comparison and `Debug` recurse once per level; drop
/// does not recurse (see [`AriaTemplateNode`]'s `Drop`), so a template built programmatically
/// to arbitrary depth drops without overflowing the thread's stack.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AriaTemplateNode {
    /// A role matcher (`kind: "role"`), matching an element or synthetic root fragment together
    /// with optional accessible attributes, properties and child matchers.
    Role(AriaTemplateRoleNode),
    /// A text matcher (`kind: "text"`), matching a text leaf in the aria tree.
    Text(AriaTemplateTextNode),
}

/// A role node in a parsed aria snapshot template: Playwright's `AriaTemplateRoleNode` at 1.64.1
/// (vendored `isomorphic/ariaSnapshot.ts`).
///
/// All fields are public data fields mirroring the wire JSON (`camelCase`). Optional fields omit
/// absent values on serialize and reject explicit `null` on deserialize.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AriaTemplateRoleNode {
    /// The expected ARIA role, or `"fragment"` to match any role node (including the synthetic
    /// root container).
    pub role: String,
    /// Optional accessible name matcher: an exact string or a JavaScript regular expression.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<TemplateName>,
    /// Expected `checked` state (`true`, `false` or `"mixed"`).
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<Tristate>,
    /// Expected `disabled` state.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    /// Expected `expanded` state.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    /// Accepted on the wire because Playwright's `KeyParser` emits `[active]`, and ignored during
    /// template matching just like Playwright 1.64.1.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    /// Expected `invalid` state (`true`, `false`, `"grammar"` or `"spelling"`).
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid: Option<Invalid>,
    /// Expected numeric `level` (for example heading or treeitem level).
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<Number>,
    /// Expected `pressed` state (`true`, `false` or `"mixed"`).
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pressed: Option<Tristate>,
    /// Expected `selected` state.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    /// Ordered child matchers interpreted according to [`Self::container_mode`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<AriaTemplateNode>,
    /// Slash-prefixed property matchers (for example `"url"` from `- /url: ...`). Only `"url"` is
    /// inspected during matching, matching Playwright 1.64.1.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub props: BTreeMap<String, AriaTextValue>,
    /// How [`Self::children`] are matched against a node's effective children.
    #[serde(default, deserialize_with = "present")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_mode: Option<ContainerMode>,
}

/// A text leaf matcher in a parsed aria snapshot template: Playwright's `AriaTemplateTextNode` at
/// 1.64.1.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AriaTemplateTextNode {
    /// Both the raw YAML string and its whitespace-normalized form used during matching.
    pub text: AriaTextValue,
}

/// A text or property pattern in a parsed aria snapshot template: Playwright's `AriaTextValue` at
/// 1.64.1.
///
/// Playwright passes both forms because a `/.../` value may match either literally or as a
/// JavaScript regular expression.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AriaTextValue {
    /// Original YAML scalar string before whitespace normalization; also inspected for `/.../`
    /// regex delimiters.
    pub raw: String,
    /// Whitespace-normalized form of [`Self::raw`]. Empty means any value matches.
    pub normalized: String,
}

/// A JavaScript regular expression source carried in a template name: Playwright's `AriaRegex` at
/// 1.64.1.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AriaRegex {
    /// JavaScript regular expression pattern without surrounding `/` delimiters.
    pub pattern: String,
}

/// The accessible name matcher on an [`AriaTemplateRoleNode`]: either an exact string or an
/// [`AriaRegex`] object (`AriaRegex | string` in Playwright's wire JSON).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TemplateName {
    /// Exact accessible name string. An empty string matches any name, including absent names.
    Exact(String),
    /// Regular expression pattern matched against non-empty accessible names.
    Regex(AriaRegex),
}

/// How an [`AriaTemplateRoleNode`]'s children are compared against a snapshot node's children:
/// Playwright's `containerMode` (`"contain"`, `"equal"`, or `"deep-equal"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContainerMode {
    /// Greedy ordered subsequence matching; extra snapshot children are allowed.
    Contain,
    /// Exact child count and order at this level; descendants revert to [`Self::Contain`] unless
    /// they specify their own mode.
    Equal,
    /// Exact child count and order at this level and recursively for descendants whose own
    /// `container_mode` is `None`.
    DeepEqual,
}

/// Drops a template tree iteratively on its own stack: each role node's children move onto one
/// list before the node drops, so deep templates cannot overflow the thread's stack. Keeping
/// `Drop` on [`AriaTemplateNode`] (rather than [`AriaTemplateRoleNode`]) also preserves struct
/// update syntax (`AriaTemplateRoleNode { role, ..AriaTemplateRoleNode::default() }`) for callers
/// while still draining every descendant of a standalone `AriaTemplateRoleNode` iteratively.
impl Drop for AriaTemplateNode {
    fn drop(&mut self) {
        let Self::Role(role) = self else {
            return;
        };
        let mut pending = mem::take(&mut role.children);
        while let Some(mut child) = pending.pop() {
            if let Self::Role(child_role) = &mut child {
                pending.append(&mut child_role.children);
            }
        }
    }
}

/// Reads a present optional field as `T` directly so explicit JSON `null` fails decoding instead
/// of silently becoming `None`.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error;

    use serde_json::{Number, Value, json};

    use crate::aria_node::{Invalid, Tristate};

    use super::{
        AriaRegex, AriaTemplateNode, AriaTemplateRoleNode, AriaTemplateTextNode, AriaTextValue,
        ContainerMode, TemplateName,
    };

    fn round_trip(value: &Value) -> Result<Value, Box<dyn Error>> {
        let node: AriaTemplateNode = serde_json::from_str(&value.to_string())?;
        Ok(serde_json::to_value(&node)?)
    }

    #[test]
    fn round_trips_every_playwright_template_shape() -> Result<(), Box<dyn Error>> {
        for sample in [
            json!({ "kind": "role", "role": "fragment" }),
            json!({ "kind": "role", "role": "button", "name": "" }),
            json!({ "kind": "role", "role": "button", "name": "Submit" }),
            json!({ "kind": "role", "role": "button", "name": { "pattern": "^Sub.*" } }),
            json!({
                "kind": "role",
                "role": "checkbox",
                "name": "Accept",
                "checked": "mixed",
                "disabled": false,
                "expanded": true,
                "active": true,
                "invalid": "spelling",
                "level": 2,
                "pressed": false,
                "selected": true,
                "children": [
                    {
                        "kind": "text",
                        "text": { "raw": "hello  world", "normalized": "hello world" }
                    },
                    {
                        "kind": "role",
                        "role": "link",
                        "props": {
                            "url": { "raw": "/https:\\/\\/.*/", "normalized": "/https:\\/\\/.*/" }
                        },
                        "containerMode": "deep-equal"
                    }
                ],
                "props": {
                    "url": { "raw": "https://example.com", "normalized": "https://example.com" }
                },
                "containerMode": "equal"
            }),
            json!({
                "kind": "role",
                "role": "list",
                "containerMode": "contain"
            }),
            json!({
                "kind": "text",
                "text": { "raw": "plain", "normalized": "plain" }
            }),
        ] {
            assert_eq!(
                round_trip(&sample)?,
                sample,
                "valid Playwright template JSON must round-trip without drift"
            );
        }
        Ok(())
    }

    #[test]
    fn decodes_typed_fields_with_expected_rust_variants() -> Result<(), Box<dyn Error>> {
        let parsed: AriaTemplateNode = serde_json::from_value(json!({
            "kind": "role",
            "role": "heading",
            "name": { "pattern": "Chapter \\d+" },
            "checked": "mixed",
            "disabled": false,
            "expanded": true,
            "active": false,
            "invalid": "grammar",
            "level": 3,
            "pressed": true,
            "selected": false,
            "containerMode": "deep-equal",
            "props": {
                "url": { "raw": "/foo/", "normalized": "/foo/" }
            },
            "children": [
                { "kind": "text", "text": { "raw": "a\nb", "normalized": "a b" } }
            ]
        }))?;
        let mut expected_props = BTreeMap::new();
        expected_props.insert(
            "url".to_owned(),
            AriaTextValue {
                raw: "/foo/".to_owned(),
                normalized: "/foo/".to_owned(),
            },
        );
        let expected = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "heading".to_owned(),
            name: Some(TemplateName::Regex(AriaRegex {
                pattern: "Chapter \\d+".to_owned(),
            })),
            checked: Some(Tristate::Mixed),
            disabled: Some(false),
            expanded: Some(true),
            active: Some(false),
            invalid: Some(Invalid::Grammar),
            level: Some(Number::from(3_u64)),
            pressed: Some(Tristate::True),
            selected: Some(false),
            children: vec![AriaTemplateNode::Text(AriaTemplateTextNode {
                text: AriaTextValue {
                    raw: "a\nb".to_owned(),
                    normalized: "a b".to_owned(),
                },
            })],
            props: expected_props,
            container_mode: Some(ContainerMode::DeepEqual),
        });
        assert_eq!(
            parsed, expected,
            "wire JSON must decode into the exact typed Rust fields"
        );
        Ok(())
    }

    #[test]
    fn rejects_null_on_every_optional_field_and_unknown_fields() -> Result<(), Box<dyn Error>> {
        for invalid in [
            json!({ "kind": "role", "role": "button", "name": null }),
            json!({ "kind": "role", "role": "checkbox", "checked": null }),
            json!({ "kind": "role", "role": "button", "disabled": null }),
            json!({ "kind": "role", "role": "button", "expanded": null }),
            json!({ "kind": "role", "role": "button", "active": null }),
            json!({ "kind": "role", "role": "textbox", "invalid": null }),
            json!({ "kind": "role", "role": "heading", "level": null }),
            json!({ "kind": "role", "role": "button", "pressed": null }),
            json!({ "kind": "role", "role": "tab", "selected": null }),
            json!({ "kind": "role", "role": "list", "children": null }),
            json!({ "kind": "role", "role": "link", "props": null }),
            json!({ "kind": "role", "role": "list", "containerMode": null }),
            json!({ "kind": "role", "role": "list", "containerMode": "deepEqual" }),
            json!({ "kind": "role", "role": "button", "unknown": true }),
            json!({ "kind": "role", "role": "button", "name": { "pattern": "a", "flags": "i" } }),
            json!({ "kind": "text", "text": { "raw": "a", "normalized": "a" }, "extra": 1 }),
            json!({ "kind": "text", "text": { "raw": "a", "normalized": "a", "extra": 1 } }),
            json!({ "kind": "other", "role": "button" }),
        ] {
            let encoded = serde_json::to_string(&invalid)?;
            assert!(
                serde_json::from_str::<AriaTemplateNode>(&encoded).is_err(),
                "{invalid} violates Playwright's template wire contract and must be rejected"
            );
        }
        Ok(())
    }

    #[test]
    fn drops_a_100_000_level_deep_template_without_stack_overflow() -> Result<(), Box<dyn Error>> {
        let mut node: AriaTemplateNode = serde_json::from_value(json!({
            "kind": "role",
            "role": "button",
            "name": "Deep"
        }))?;
        for _ in 1..100_000 {
            node = AriaTemplateNode::Role(AriaTemplateRoleNode {
                role: "generic".to_owned(),
                children: vec![node],
                ..AriaTemplateRoleNode::default()
            });
        }
        drop(node);
        Ok(())
    }
}
