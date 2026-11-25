//! Aria-template matching over [`crate::AriaNode`] trees: a pure-Rust port of Playwright 1.64.1's
//! `matchesExpectAriaTemplate` and `getAllElementsMatchingExpectAriaTemplate`
//! (`injected/src/ariaSnapshot.ts:308-462`) and its parsed template wire shape
//! (`isomorphic/ariaSnapshot.ts:81-105`).
//!
//! ```text
//!  prelude/aria-template.ts ─▶ JSON ─▶ AriaTemplateNode ─┐
//!  snapshot tree (default mode) ─────▶ &[AriaNode] ──────┴─▶ match_template / matches_template
//!                                                               │
//!                                                               └─▶ JsRegex (regress): JS RegExp
//! ```
//!
//! The prelude parses YAML templates into JSON; this module decodes that JSON into
//! [`AriaTemplateNode`] and evaluates it iteratively on explicit heap stacks so deep trees and
//! templates never overflow the call stack.
//!
//! Seam note: Playwright's internal matcher runs on `toAriaNode` trees where roles owning a state
//! carry explicit `false` or `0` (for example `selected: Some(false)` on an unselected `row`, or
//! `invalid: Some(Invalid::False)` on a valid `textbox`), whereas `AriaNodeJSON` omits those
//! defaults on the wire. Callers matching explicit negative or zero filters such as
//! `[selected=false]` or `[invalid=false]` must populate those `Some(false)` / `Some(0)` values on
//! their [`crate::AriaNode`] tree; this crate's printer already treats explicit `false` and `0` as
//! absent when printing YAML.

mod aria_template_node;
mod js_regex;
mod match_template;
#[cfg(test)]
mod tests;

pub use aria_template_node::{
    AriaRegex, AriaTemplateNode, AriaTemplateRoleNode, AriaTemplateTextNode, AriaTextValue,
    ContainerMode, TemplateName,
};
pub use js_regex::RegexError;
pub use match_template::{TemplateError, TemplateMatch, match_template, matches_template};
