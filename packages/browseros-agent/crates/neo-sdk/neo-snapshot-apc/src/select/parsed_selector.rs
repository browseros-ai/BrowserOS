use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, value};
use value::RawValue;

/// Selector engine names whose wire `body` is a [`NestedSelectorBody`], matching Playwright
/// 1.64.1's `kNestedSelectorNames` (`isomorphic/selectorParser.ts:23`).
pub const NESTED_SELECTOR_NAMES: &[&str] = &[
    "internal:has",
    "internal:has-not",
    "internal:and",
    "internal:or",
    "internal:chain",
    "left-of",
    "right-of",
    "above",
    "below",
    "near",
];

/// Layout selector engine names permitted to carry a `distance` on their [`NestedSelectorBody`],
/// matching Playwright 1.64.1's `kNestedSelectorNamesWithDistance`
/// (`isomorphic/selectorParser.ts:24`).
pub const LAYOUT_SELECTOR_NAMES: &[&str] = &["left-of", "right-of", "above", "below", "near"];

/// One per-frame parsed selector chunk emitted by the prelude's `splitSelectorByFrame`
/// (`isomorphic/selectorParser.ts:32-35`).
///
/// Deserialization validates the wire boundary (`deny_unknown_fields`, non-empty `parts`,
/// in-bounds `capture`, and body-variant agreement with each part's `name`) so downstream chunk
/// planning and evaluation never re-check JSON shape. Semantic rules such as where nested
/// combinators may appear belong to chunk planning rather than deserialization.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ParsedSelector {
    /// Non-empty ordered list of selector engine parts in this chunk.
    pub parts: Vec<SelectorPart>,
    /// Zero-based index into [`Self::parts`] captured by a leading `*` modifier, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<usize>,
}

/// One engine step inside a [`ParsedSelector`] (`isomorphic/selectorParser.ts:26-30`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SelectorPart {
    /// Selector engine name (for example `"css"`, `"internal:role"`, `"internal:has"`, `"nth"`).
    pub name: String,
    /// Engine-specific payload, validated against [`Self::name`] at deserialization.
    pub body: SelectorBody,
    /// Original unparsed engine body text preserved for stringification and diagnostics.
    pub source: String,
}

/// Typed body of a [`SelectorPart`], distinguishing opaque CSS AST arrays, nested selector
/// combinators, and plain engine body strings.
#[derive(Clone, Debug)]
pub enum SelectorBody {
    /// Plain string body (for example `internal:role`, `role`, `aria-ref`, `nth`, `internal:text`,
    /// `xpath`).
    Text(String),
    /// Nested selector body for combinators (`internal:has`, `internal:has-not`, `internal:and`,
    /// `internal:or`, `internal:chain`) and layout filters (`left-of`, `right-of`, `above`,
    /// `below`, `near`).
    Nested(NestedSelectorBody),
    /// Opaque `CSSComplexSelectorList` JSON array passed straight through to the page's CSS
    /// engine without re-parsing in Rust.
    Css(Box<RawValue>),
}

impl PartialEq for SelectorBody {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Text(lhs), Self::Text(rhs)) => lhs == rhs,
            (Self::Nested(lhs), Self::Nested(rhs)) => lhs == rhs,
            (Self::Css(lhs), Self::Css(rhs)) => lhs.get() == rhs.get(),
            (Self::Text(_), Self::Nested(_) | Self::Css(_))
            | (Self::Nested(_), Self::Text(_) | Self::Css(_))
            | (Self::Css(_), Self::Text(_) | Self::Nested(_)) => false,
        }
    }
}

impl Serialize for SelectorBody {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Text(text) => serializer.serialize_str(text),
            Self::Nested(nested) => nested.serialize(serializer),
            Self::Css(raw) => raw.serialize(serializer),
        }
    }
}

/// Payload of a nested or layout [`SelectorPart`] (`isomorphic/selectorParser.ts:22`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NestedSelectorBody {
    /// The nested selector evaluated by the combinator or layout engine.
    pub parsed: ParsedSelector,
    /// Optional pixel distance cap; valid only when the owning part's name is in
    /// [`LAYOUT_SELECTOR_NAMES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawParsedSelector {
    parts: Vec<SelectorPart>,
    #[serde(default)]
    capture: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSelectorPart {
    name: String,
    body: Value,
    source: String,
}

impl<'de> Deserialize<'de> for ParsedSelector {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawParsedSelector::deserialize(deserializer)?;
        if raw.parts.is_empty() {
            return Err(D::Error::custom(
                "parsed selector must contain at least one part",
            ));
        }
        if let Some(capture) = raw.capture
            && capture >= raw.parts.len()
        {
            return Err(D::Error::custom(format!(
                "capture index {capture} is out of range for {} parts",
                raw.parts.len()
            )));
        }
        Ok(Self {
            parts: raw.parts,
            capture: raw.capture,
        })
    }
}

impl<'de> Deserialize<'de> for SelectorPart {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Decode `body` via `Value` rather than an untagged enum or direct `RawValue`: callers
        // frequently decode chunks from `serde_json::Value` or inside a tagged parent envelope,
        // where serde's buffered content cannot feed `RawValue` directly.
        let raw = RawSelectorPart::deserialize(deserializer)?;
        let body = if raw.name == "css" {
            if !raw.body.is_array() {
                return Err(D::Error::custom(
                    "css selector part body must be a JSON array",
                ));
            }
            let css = value::to_raw_value(&raw.body).map_err(D::Error::custom)?;
            SelectorBody::Css(css)
        } else if NESTED_SELECTOR_NAMES.contains(&raw.name.as_str()) {
            let nested: NestedSelectorBody =
                serde_json::from_value(raw.body).map_err(D::Error::custom)?;
            if nested.distance.is_some() && !LAYOUT_SELECTOR_NAMES.contains(&raw.name.as_str()) {
                return Err(D::Error::custom(format!(
                    "selector part \"{}\" does not support a distance",
                    raw.name
                )));
            }
            SelectorBody::Nested(nested)
        } else {
            let Value::String(text) = raw.body else {
                return Err(D::Error::custom(format!(
                    "selector part \"{}\" body must be a string",
                    raw.name
                )));
            };
            SelectorBody::Text(text)
        };
        Ok(Self {
            name: raw.name,
            body,
            source: raw.source,
        })
    }
}
