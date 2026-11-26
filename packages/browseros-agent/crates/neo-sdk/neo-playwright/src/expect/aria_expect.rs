//! The four accessibility matchers and `toMatchAriaSnapshot`, answered from one attempt's APC
//! `SnapshotTree` instead of the page's JavaScript ARIA engine (design 5.6).
//!
//! ```text
//!  prelude params (ExpectParams) ─▶ AriaExpect::new ─┬─▶ Property(PropertyMatcher)   role, name,
//!                                  (once per op)     │     Property + ExpectedText     description,
//!                                                    │                                 error message
//!                                                    └─▶ Template(TemplateMatcher)   to.match.aria
//!
//!  each poll:
//!   SnapshotTree + NameCache + target NodeId ─▶ PropertyMatcher::check ─┐
//!   SnapshotTree + target NodeId ─▶ TemplateMatcher::check ─────────────┴─▶ AriaVerdict
//!     (tree.aria in template mode: the element's nodes, explicit states)   { matches, received }
//!                                                                            ─▶ Poll::Ran
//!   a failed property poll + Visibility ─▶ FailureSnapshot (what the caller renders)
//! ```
//!
//! Every read here is pure: no CDP, no page, no clock. The caller (the expect op after the
//! switch-over) resolves the target, builds the tree's `NameCache` once per attempt, and owns
//! `resolved`, the failure snapshot's text and the loop. The expected values are whatever the
//! script passed (a matcher may compare a secret), so nothing here that holds one implements
//! `Debug` (STYLE P4).

#![cfg_attr(not(test), expect(dead_code, reason = "wired by the switch-over"))]

use std::error::Error;
use std::fmt;

use neo_snapshot::template::{self, AriaTemplateNode, TemplateError};
use neo_snapshot::{Call, JsRegex, RegexError, Snapshot};
use neo_snapshot_apc::tree::{
    AriaAsk, AriaRoot, AriaTooDeep, NameCache, NameVisibility, NodeId, SnapshotTree, TreeNode,
    normalize_whitespace,
};
use serde::Deserialize;
use serde_json::{Map, Value};

use super::expectation::Expression;
use crate::probe::ExpectParams;

#[cfg(test)]
mod tests;

/// One of the five matchers the tree answers, decoded once per op from the prelude's params,
/// its expected value checked and its regex compiled before the first poll.
///
/// The two arms take different inputs per poll, so the caller matches on them: a property needs
/// the tree, its name pass and the target's node, which may be absent; a template needs the
/// target's node to emit the element's aria tree from, a walk the property matchers never pay.
pub(crate) enum AriaExpect {
    Property(PropertyMatcher),
    Template(TemplateMatcher),
}

/// `toHaveRole`, `toHaveAccessibleName`, `toHaveAccessibleDescription` or
/// `toHaveAccessibleErrorMessage`: the property read from the tree and the script's expected text.
pub(crate) struct PropertyMatcher {
    property: Property,
    expected: ExpectedText,
}

/// The element property a property matcher compares, each as upstream reads it
/// (`injectedScript.ts:1669-1676`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Property {
    /// `getAriaRole(element) || ''`.
    Role,
    /// `getElementAccessibleNameText(element, false)`.
    AccessibleName,
    /// `getElementAccessibleDescription(element, false).text`.
    AccessibleDescription,
    /// `getElementAccessibleErrorMessage(element)`.
    AccessibleErrorMessage,
}

/// `toMatchAriaSnapshot`: the template the prelude parsed from the script's YAML.
pub(crate) struct TemplateMatcher {
    template: AriaTemplateNode,
}

/// What one poll compared: the matcher's own result before `.not`, and the received value as the
/// probe's `expect` reply carries it today. For a template that is upstream's `{ raw }` object
/// (`MatcherReceived` without `regex`, which Neo never prints), so the caller passes it through
/// `Expression::printed_received` exactly as it does a probe reply.
pub(crate) struct AriaVerdict {
    pub(crate) matches: bool,
    pub(crate) received: Value,
}

/// What a failed property poll attaches as its aria snapshot, upstream's `_ariaSnapshotForExpect`
/// (`injectedScript.ts:1507-1520`) for these matchers. Rendering it is the caller's: design 5.6
/// renders one `tree.aria` at the final failure from the last attempt's tree, where upstream
/// renders one per failing poll. A template poll never attaches one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureSnapshot {
    /// The element's own line: `mode: 'default'`, `depth: 1` (a property matcher is never the
    /// containment matcher `to.have.text`, the one upstream snapshots whole).
    Element,
    /// The body of the frame the element is in, `mode: 'default'`, whole; no snapshot at all when
    /// that document has no body (upstream's `!this.document.body`).
    Body,
}

impl FailureSnapshot {
    /// What a property poll that failed (its result equal to `.not`) snapshots: the element's own
    /// line when the frame and the element are visible, else the frame's body. A poll that passed
    /// attaches none, and the caller asks only for one that failed, as upstream's `expect` does.
    #[must_use]
    pub(crate) const fn for_failed_property(visibility: Visibility) -> Self {
        if visibility.frame && visibility.element {
            Self::Element
        } else {
            Self::Body
        }
    }
}

/// Whether the frame and the element are visible, the two facts upstream's failure snapshot
/// decides on. `element` is `isElementVisible` (`computeBox(element).visible`): for a node of the
/// tree that is `TreeNode::hidden().is_box_visible()`; an element the tree has no node for needs
/// the page's answer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Visibility {
    pub(crate) frame: bool,
    pub(crate) element: bool,
}

/// Why a matcher cannot run. Each prints the text the script sees as the matcher's `Error:` line;
/// none quotes an expected value. Upstream throws the first three from the page on a poll that
/// found the element, so the caller raises them there, not when the op starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AriaExpectError {
    /// A property matcher with no `expectedText`: upstream's `Unknown expect matcher:` error.
    UnknownMatcher(Property),
    /// A regex of the expected text that does not compile here. Upstream's V8 text is
    /// `Invalid regular expression: /<source>/<flags>: <reason>`; the source is the script's
    /// expected value, so it is left out and the reason is the regex engine's own.
    InvalidRegex(RegexError),
    /// A template's name regex that does not compile (`TemplateError`'s own text).
    Template(TemplateError),
    /// Params no prelude sends, named by their path (the decode error is left out: serde quotes
    /// the value it could not read).
    Malformed(String),
    /// The element's tree is nested deeper than an aria tree may be: the snapshot's own refusal,
    /// which tells the script how to retry.
    TooDeep(AriaTooDeep),
}

impl fmt::Display for AriaExpectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownMatcher(property) => {
                write!(f, "Unknown expect matcher: {}", property.expression())
            }
            Self::InvalidRegex(error) => write!(f, "Invalid regular expression: {error}"),
            Self::Template(error) => write!(f, "{error}"),
            Self::Malformed(path) => {
                write!(f, "params.{path} is not what Playwright's matchers send")
            }
            Self::TooDeep(error) => write!(f, "{error}"),
        }
    }
}

impl Error for AriaExpectError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidRegex(error) => Some(error),
            Self::Template(error) => Some(error),
            Self::TooDeep(error) => Some(error),
            Self::UnknownMatcher(_) | Self::Malformed(_) => None,
        }
    }
}

/// The params this module reads. `Expectation::new` has already decoded the whole object with
/// `deny_unknown_fields` (and refused every expression it does not run), so this reads only the
/// keys it needs and ignores the rest.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AriaParams {
    expression: Expression,
    #[serde(default)]
    expected_text: Option<Vec<ExpectedTextValue>>,
    #[serde(default)]
    expected_value: Option<Value>,
}

impl AriaExpect {
    /// Reads the matcher from the prelude's params: `Ok(None)` for every expression the tree does
    /// not answer, which keeps running in the page.
    pub(crate) fn new(params: &ExpectParams) -> Result<Option<Self>, AriaExpectError> {
        let raw = Value::Object(params.0.clone());
        let AriaParams {
            expression,
            expected_text,
            expected_value,
        } = serde_path_to_error::deserialize(raw)
            .map_err(|error| AriaExpectError::Malformed(error.path().to_string()))?;
        let property = match expression {
            Expression::Role => Property::Role,
            Expression::AccessibleName => Property::AccessibleName,
            Expression::AccessibleDescription => Property::AccessibleDescription,
            Expression::AccessibleErrorMessage => Property::AccessibleErrorMessage,
            Expression::MatchAria => {
                let template = decode_template(expected_value)?;
                return Ok(Some(Self::Template(TemplateMatcher { template })));
            }
            _ => return Ok(None),
        };
        // Upstream reads `expectedText[0]` only after `options.expectedText` passed its truthy
        // test, so an absent (or `null`) list is the unknown matcher, and an empty one, which
        // upstream would crash on, is params no prelude sends.
        let first = match expected_text {
            None => return Err(AriaExpectError::UnknownMatcher(property)),
            Some(values) => values
                .into_iter()
                .next()
                .ok_or_else(|| AriaExpectError::Malformed("expectedText".to_owned()))?,
        };
        Ok(Some(Self::Property(PropertyMatcher {
            property,
            expected: ExpectedText::new(first)?,
        })))
    }
}

/// `expectedValue` as the template the prelude parsed. A missing one is params no prelude sends
/// (upstream's page would throw a `TypeError` reading its `kind`).
fn decode_template(expected_value: Option<Value>) -> Result<AriaTemplateNode, AriaExpectError> {
    let malformed = |path: String| AriaExpectError::Malformed(format!("expectedValue{path}"));
    let value = expected_value.ok_or_else(|| malformed(String::new()))?;
    serde_path_to_error::deserialize(value).map_err(|error| {
        let path = error.path().to_string();
        malformed(if path == "." {
            String::new()
        } else {
            format!(".{path}")
        })
    })
}

impl Property {
    /// Upstream's expression for this property, as matchers.ts sends it.
    pub(crate) const fn expression(self) -> &'static str {
        match self {
            Self::Role => "to.have.role",
            Self::AccessibleName => "to.have.accessible.name",
            Self::AccessibleDescription => "to.have.accessible.description",
            Self::AccessibleErrorMessage => "to.have.accessible.error.message",
        }
    }

    /// The property of `target` in `tree`. An element the tree has no node for (a plain `div` or
    /// `span` the retention rule left out, design 5.6) has no role and an empty name, description
    /// and error message.
    fn read<'a>(
        self,
        tree: &SnapshotTree,
        names: &'a NameCache<'_>,
        target: Option<NodeId>,
    ) -> &'a str {
        let Some(id) = target else {
            return "";
        };
        match self {
            Self::Role => tree
                .node(id)
                .and_then(TreeNode::role)
                .map_or("", |role| role.as_str()),
            Self::AccessibleName => names.name(id, NameVisibility::VisibleOnly).text(),
            Self::AccessibleDescription => {
                names.description(id, NameVisibility::VisibleOnly).text()
            }
            Self::AccessibleErrorMessage => names.error_message(id),
        }
    }
}

impl PropertyMatcher {
    /// One poll: the property of `target` (`None` when the tree has no node for the element)
    /// compared with the expected text. `names` is the attempt's one cache over `tree`, shared
    /// with its locator stages, so a name the locator already computed is not computed again.
    #[must_use]
    pub(crate) fn check(
        &self,
        tree: &SnapshotTree,
        names: &NameCache<'_>,
        target: Option<NodeId>,
    ) -> AriaVerdict {
        let received = self.property.read(tree, names, target);
        AriaVerdict {
            matches: self.expected.matches(received),
            received: Value::String(received.to_owned()),
        }
    }
}

impl TemplateMatcher {
    /// One poll: the template against `target`'s aria tree (the element's node or nodes at the
    /// top), and that tree's YAML as received.
    ///
    /// The tree is emitted here in template mode, where every state a node's role owns is
    /// explicit, as upstream's `toAriaNode` writes it, so `[checked=false]` matches an unchecked
    /// checkbox and `[expanded=false]` only an `aria-expanded="false"` (see
    /// `neo_snapshot::template`). The YAML is default mode's: the printer drops `false` and `0`,
    /// as upstream's does.
    pub(crate) fn check(
        &self,
        tree: &SnapshotTree,
        target: NodeId,
    ) -> Result<AriaVerdict, AriaExpectError> {
        let ask = AriaAsk::template_mode().with_root(AriaRoot::Subtree(target));
        let aria = tree.aria(ask).map_err(AriaExpectError::TooDeep)?;
        // The element's nodes alone: `Call::Playwright` with no partial frames prints upstream's
        // lines for them and nothing else.
        let snapshot = Snapshot {
            tree: aria.into_snapshot().tree,
            ..Snapshot::default()
        };
        let matches = template::matches_template(&snapshot.tree, &self.template)
            .map_err(AriaExpectError::Template)?;
        let raw = neo_snapshot::render(&snapshot, Call::Playwright).into_text();
        let mut received = Map::new();
        received.insert("raw".to_owned(), Value::String(raw));
        Ok(AriaVerdict {
            matches,
            received: Value::Object(received),
        })
    }
}

/// Upstream's `ExpectedTextValue` as the prelude serializes it (`serializeExpectedTextValues`):
/// a string or a regex's source and flags, and three options.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ExpectedTextValue {
    #[serde(default)]
    string: Option<String>,
    #[serde(default)]
    regex_source: Option<String>,
    #[serde(default)]
    regex_flags: Option<String>,
    #[serde(default)]
    match_substring: Option<bool>,
    #[serde(default)]
    ignore_case: Option<bool>,
    #[serde(default)]
    normalize_white_space: Option<bool>,
}

/// The port of upstream's `ExpectedTextMatcher` (`injectedScript.ts:1825-1879`), built once per
/// op. The expected side is normalized once here; the received side per poll, except under a
/// regex, which upstream tests on the text as the element has it.
pub(crate) struct ExpectedText {
    normalize: Normalize,
    compare: Compare,
}

/// Upstream's `normalize`: whitespace first, then case.
#[derive(Clone, Copy)]
struct Normalize {
    white_space: bool,
    ignore_case: bool,
}

/// How the received text is compared, in upstream's order of precedence.
enum Compare {
    /// `_string`: equal to the normalized expected string.
    Equal(String),
    /// `_substring` (`matchSubstring`): contains the normalized expected string.
    Contain(String),
    /// `_regex`, its `i` flag set or cleared by `ignoreCase`.
    Pattern(JsRegex),
}

impl ExpectedText {
    /// Upstream's constructor. A value with both a string and a regex, or with neither, is params
    /// no prelude sends; upstream would compare by the string, or answer `false` every time.
    fn new(value: ExpectedTextValue) -> Result<Self, AriaExpectError> {
        let ExpectedTextValue {
            string,
            regex_source,
            regex_flags,
            match_substring,
            ignore_case,
            normalize_white_space,
        } = value;
        let normalize = Normalize {
            white_space: normalize_white_space == Some(true),
            ignore_case: ignore_case == Some(true),
        };
        // `if (expected.regexSource)`: an empty source is no regex.
        let regex_source = regex_source.filter(|source| !source.is_empty());
        let compare = match (string, regex_source) {
            (Some(string), None) if match_substring == Some(true) => {
                Compare::Contain(normalize.apply(&string))
            }
            (Some(string), None) => Compare::Equal(normalize.apply(&string)),
            (None, Some(source)) => {
                let flags =
                    regex_flags_for(regex_flags.as_deref().unwrap_or_default(), ignore_case);
                Compare::Pattern(
                    JsRegex::compile_with_flags(&source, &flags)
                        .map_err(AriaExpectError::InvalidRegex)?,
                )
            }
            (Some(_), Some(_)) | (None, None) => {
                return Err(AriaExpectError::Malformed("expectedText[0]".to_owned()));
            }
        };
        Ok(Self { normalize, compare })
    }

    /// Upstream's `matches(text)`.
    #[must_use]
    pub(crate) fn matches(&self, text: &str) -> bool {
        match &self.compare {
            Compare::Equal(expected) => self.normalize.apply(text) == *expected,
            Compare::Contain(expected) => self.normalize.apply(text).contains(expected.as_str()),
            // `lastIndex = 0; test(text)`, on the text unnormalized.
            Compare::Pattern(regex) => regex.is_match(text),
        }
    }
}

impl Normalize {
    fn apply(self, text: &str) -> String {
        let text = if self.white_space {
            normalize_whitespace(text)
        } else {
            text.to_owned()
        };
        // `toLocaleLowerCase()` with no locale; Rust's full Unicode lowercasing (final sigma
        // included) is what V8 gives under the root and English locales.
        if self.ignore_case {
            text.to_lowercase()
        } else {
            text
        }
    }
}

/// `new Set(regexFlags.split(''))`, with `i` removed for `ignoreCase: false` and added for
/// `ignoreCase: true`, joined back. The set drops a repeated flag that `RegExp` itself would
/// refuse, so `"gg"` compiles as `g`, as upstream's does.
fn regex_flags_for(flags: &str, ignore_case: Option<bool>) -> String {
    let mut kept: Vec<char> = Vec::new();
    for flag in flags.chars() {
        if !kept.contains(&flag) {
            kept.push(flag);
        }
    }
    match ignore_case {
        Some(false) => kept.retain(|flag| *flag != 'i'),
        Some(true) if !kept.contains(&'i') => kept.push('i'),
        Some(true) | None => {}
    }
    kept.into_iter().collect()
}
