//! Selector chunk decoding, attribute-selector parsing, ECMAScript regular expressions, and
//! validated role-query construction for the pure-Rust APC locator engine.
//!
//! The JS prelude keeps parsing selector strings with Playwright's vendored `splitSelectorByFrame`
//! and sends parsed per-frame chunks across the bridge. This subsystem decodes each chunk into a
//! typed [`ParsedSelector`], parses the bodies of parts Rust evaluates (`role` and
//! `internal:role`), and validates role attributes with upstream Playwright 1.64.1 error text:
//!
//! ```text
//!  prelude JSON chunk ─▶ ParsedSelector ─▶ SelectorPart (Text | Nested | Css)
//!                                               │
//!                              role / internal:role body
//!                                               ▼
//!                                   parse_attribute_selector ─▶ AttributeSelector
//!                                               │                     │
//!                                               ▼                     ▼
//!                                       RoleQuery::parse           JsRegex
//! ```

mod attribute_selector;
mod js_regex;
mod parsed_selector;
mod role_query;

pub use attribute_selector::{
    AttributeSelector, AttributeSelectorError, AttributeSelectorOperator, AttributeSelectorPart,
    AttributeValue, UnquotedStrings, parse_attribute_selector,
};
pub use js_regex::{JsRegex, RegexError};
pub use parsed_selector::{
    LAYOUT_SELECTOR_NAMES, NESTED_SELECTOR_NAMES, NestedSelectorBody, ParsedSelector, SelectorBody,
    SelectorPart,
};
pub use role_query::{RoleQuery, RoleQueryError, StringMatchOperator, TextFilter};
