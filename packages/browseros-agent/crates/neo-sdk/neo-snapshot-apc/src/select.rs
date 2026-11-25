//! Selector chunk decoding, attribute-selector parsing, ECMAScript regular expressions, role-query
//! validation, and tree stages: the pure-Rust half of the locator engine.
//!
//! The JS prelude keeps parsing selector strings with Playwright's vendored `splitSelectorByFrame`
//! and sends parsed per-frame chunks across the bridge. This subsystem decodes each chunk into a
//! typed [`ParsedSelector`], parses the bodies of parts Rust evaluates (`role` and
//! `internal:role`), and validates role attributes with upstream Playwright 1.64.1 error text. A
//! chunk made only of such parts becomes a [`TreeStage`], which selects elements from one
//! [`SnapshotTree`](crate::tree::SnapshotTree) with the semantics of the vendored
//! `InjectedScript.querySelectorAll`:
//!
//! ```text
//!  prelude JSON chunk ─▶ ParsedSelector ─▶ SelectorPart (Text | Nested | Css)
//!                             │                 │
//!                             │       role / internal:role body
//!                             │                 ▼
//!                             │     parse_attribute_selector ─▶ AttributeSelector ─▶ JsRegex
//!                             │                 ▼
//!                             │          RoleQuery::parse
//!                             ▼                 │
//!                    TreeStage::new ◀───────────┘
//!                             │
//!   SnapshotTree, Scope ─▶ TreeStage::select ─▶ TreeMatches ─▶ count, nth, strict mode
//!   AccessibleText (NameCache, chunk 4) ───┘    (NodeIds in      └─▶ StrictViolation::message
//!   RefResolver (refs, engine chunk 5) ────┘     Playwright's order)
//! ```
//!
//! A stage reads names through [`AccessibleText`], which the tree's
//! [`NameCache`](crate::tree::NameCache) implements, so one cache built per locator attempt serves
//! every stage and hint of that attempt. It reads refs through [`RefResolver`], because the tab's
//! ref table belongs to engine chunk 5.

mod accessible_text;
mod attribute_selector;
mod js_regex;
mod parsed_selector;
mod ref_resolver;
mod role_match;
mod role_query;
mod tree_matches;
mod tree_stage;

pub use accessible_text::AccessibleText;
pub use attribute_selector::{
    AttributeSelector, AttributeSelectorError, AttributeSelectorOperator, AttributeSelectorPart,
    AttributeValue, UnquotedStrings, parse_attribute_selector,
};
pub use js_regex::{JsRegex, RegexError};
pub use parsed_selector::{
    LAYOUT_SELECTOR_NAMES, NESTED_SELECTOR_NAMES, NestedSelectorBody, ParsedSelector, SelectorBody,
    SelectorPart,
};
pub use ref_resolver::RefResolver;
pub use role_query::{RoleQuery, RoleQueryError, StringMatchOperator, TextFilter};
pub use tree_matches::{StrictPreview, StrictViolation, TreeMatches};
pub use tree_stage::{NotTreeStage, Scope, TreeStage, TreeStageError};
