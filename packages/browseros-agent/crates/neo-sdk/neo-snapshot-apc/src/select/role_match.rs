use neo_snapshot::Tristate;

use crate::tree::{self, NameVisibility, Role, TreeNode, TriState};

use super::accessible_text::AccessibleText;
use super::js_regex::JsRegex;
use super::role_query::{RoleQuery, StringMatchOperator, TextFilter};

/// Which engine a role part names. They differ in one rule: under `internal:role`, the
/// `getByRole` engine, a case-insensitive `=` on `name` or `description` matches a substring
/// (`roleSelectorEngine.ts:170-172`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RoleEngine {
    /// `role=…`, Playwright's public selector engine.
    Role,
    /// `internal:role=…`, what `getByRole` writes.
    InternalRole,
}

/// One validated role part, prepared to test tree nodes in `queryRole`'s order
/// (`roleSelectorEngine.ts:143-188`): role, the states, hidden-ness, then name and description.
#[derive(Clone, Debug)]
pub(crate) struct RoleMatch {
    query: RoleQuery,
    name: Option<TextMatch>,
    description: Option<TextMatch>,
}

/// A `name` or `description` filter with its needle prepared once, as `queryRole` prepares it on
/// first use: whitespace-normalized, upper-cased when case-insensitive, and `=` widened to `*=`
/// under `internal:role`.
#[derive(Clone, Debug)]
enum TextMatch {
    /// `matchesAttributePart` on strings (`selectorUtils.ts:31-57`).
    String {
        needle: String,
        op: StringMatchOperator,
        case: Case,
    },
    /// `matchesAttributePart` on a `RegExp`. The attribute parser marks every regular expression
    /// case-sensitive (`selectorParser.ts:412-416`), so its haystack is never upper-cased; an `i`
    /// flag is the regex's own business.
    Regex(JsRegex),
}

/// Whether a string filter compares case-insensitively, after `toUpperCase` on both sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    Sensitive,
    Insensitive,
}

impl RoleMatch {
    pub(crate) fn new(query: RoleQuery, engine: RoleEngine) -> Self {
        let name = query.name().map(|filter| TextMatch::new(filter, engine));
        let description = query
            .description()
            .map(|filter| TextMatch::new(filter, engine));
        Self {
            query,
            name,
            description,
        }
    }

    /// Whether `node` matches, reading its name and description through `text` only when every
    /// cheaper check passed.
    pub(crate) fn matches(&self, node: &TreeNode, text: &dyn AccessibleText) -> bool {
        let query = &self.query;
        if node.role().map(Role::as_str) != Some(query.role()) {
            return false;
        }
        let state = node.state();
        // `getAriaSelected` answers `false` for an element that has no selected state.
        if query
            .selected()
            .is_some_and(|wanted| state.selected().unwrap_or(false) != wanted)
        {
            return false;
        }
        if query
            .checked()
            .is_some_and(|wanted| aria_tristate(state.checked()) != wanted)
        {
            return false;
        }
        if query
            .pressed()
            .is_some_and(|wanted| aria_tristate(state.pressed()) != wanted)
        {
            return false;
        }
        // `getAriaExpanded` answers `undefined` for an unset attribute, which equals neither
        // `true` nor `false`.
        if query
            .expanded()
            .is_some_and(|wanted| state.expanded() != Some(wanted))
        {
            return false;
        }
        if query
            .level()
            .is_some_and(|wanted| !js_numbers_equal(f64::from(state.level().unwrap_or(0)), wanted))
        {
            return false;
        }
        // `getAriaDisabled`, which also covers roles outside `kAriaDisabledRoles` (a disabled
        // `<option>`), not the role-gated `State::disabled`.
        if query
            .disabled()
            .is_some_and(|wanted| state.is_disabled() != wanted)
        {
            return false;
        }
        let include_hidden = query.include_hidden() == Some(true);
        if !include_hidden && node.hidden().is_hidden_for_aria() {
            return false;
        }
        let visibility = if include_hidden {
            NameVisibility::IncludeHidden
        } else {
            NameVisibility::VisibleOnly
        };
        if let Some(name) = &self.name
            && !name.matches(&text.name(node.id(), visibility))
        {
            return false;
        }
        if let Some(description) = &self.description
            && !description.matches(&text.description(node.id(), visibility))
        {
            return false;
        }
        true
    }
}

impl TextMatch {
    fn new(filter: &TextFilter, engine: RoleEngine) -> Self {
        match filter {
            TextFilter::Regex(regex) => Self::Regex(regex.clone()),
            TextFilter::String { value, op, exact } => {
                let case = if *exact {
                    Case::Sensitive
                } else {
                    Case::Insensitive
                };
                let op = if engine == RoleEngine::InternalRole
                    && case == Case::Insensitive
                    && *op == StringMatchOperator::Eq
                {
                    StringMatchOperator::Contains
                } else {
                    *op
                };
                Self::String {
                    needle: case.apply(tree::normalize_whitespace(value)),
                    op,
                    case,
                }
            }
        }
    }

    /// `matchesAttributePart(normalizeWhiteSpace(text), filter)`. The name pass already
    /// normalizes its answers, but [`AccessibleText`] does not promise it, and upstream normalizes
    /// again here too, so the haystack is normalized whatever the seam returns.
    fn matches(&self, text: &str) -> bool {
        match self {
            Self::Regex(regex) => regex.is_match(&tree::normalize_whitespace(text)),
            Self::String { needle, op, case } => {
                let hay = case.apply(tree::normalize_whitespace(text));
                let needle = needle.as_str();
                match op {
                    StringMatchOperator::Eq => hay == needle,
                    StringMatchOperator::Contains => hay.contains(needle),
                    StringMatchOperator::Prefix => hay.starts_with(needle),
                    StringMatchOperator::Suffix => hay.ends_with(needle),
                    StringMatchOperator::DashMatch => {
                        hay == needle
                            || hay
                                .strip_prefix(needle)
                                .is_some_and(|rest| rest.starts_with('-'))
                    }
                    // JavaScript's `split(' ')` cuts on U+0020 alone, and `"".split(' ')` is `[""]`,
                    // which `str::split` reproduces.
                    StringMatchOperator::Word => hay.split(' ').any(|word| word == needle),
                }
            }
        }
    }
}

impl Case {
    /// JavaScript's `toUpperCase` when case-insensitive: full Unicode mappings, so `ß` becomes
    /// `SS`, which `str::to_uppercase` shares.
    fn apply(self, text: String) -> String {
        match self {
            Self::Sensitive => text,
            Self::Insensitive => text.to_uppercase(),
        }
    }
}

/// `getAriaChecked` / `getAriaPressed` as the query compares them: a role that has no such state
/// answers `'error'` upstream, which those functions turn into `false`.
fn aria_tristate(state: Option<TriState>) -> Tristate {
    match state {
        Some(TriState::True) => Tristate::True,
        Some(TriState::Mixed) => Tristate::Mixed,
        Some(TriState::False) | None => Tristate::False,
    }
}

/// JavaScript's `===` on two numbers.
#[expect(
    clippy::float_cmp,
    reason = "`getAriaLevel(element) !== options.level` compares exactly; the query's level is never NaN"
)]
fn js_numbers_equal(left: f64, right: f64) -> bool {
    left == right
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::{RoleEngine, RoleMatch, TextMatch};
    use crate::select::RoleQuery;

    /// The prepared `name` filter of `body` under `engine`.
    fn name_filter(body: &str, engine: RoleEngine) -> Result<TextMatch, Box<dyn Error>> {
        RoleMatch::new(RoleQuery::parse(body)?, engine)
            .name
            .ok_or_else(|| format!("{body} has no name filter").into())
    }

    #[test]
    fn each_string_operator_matches_as_matches_attribute_part() -> Result<(), Box<dyn Error>> {
        let cases = [
            ("button[name=\"Hello\"]", "Hello", true, "`=` is equality"),
            (
                "button[name=\"Hello\"]",
                "Hello world",
                false,
                "`=` is not a substring under `role`",
            ),
            (
                "button[name*=\"llo w\"]",
                "Hello world",
                true,
                "`*=` is a substring",
            ),
            ("button[name^=\"Hell\"]", "Hello", true, "`^=` is a prefix"),
            (
                "button[name^=\"ello\"]",
                "Hello",
                false,
                "`^=` anchors at the start",
            ),
            ("button[name$=\"llo\"]", "Hello", true, "`$=` is a suffix"),
            (
                "button[name|=\"en\"]",
                "en",
                true,
                "`|=` matches the whole value",
            ),
            (
                "button[name|=\"en\"]",
                "en-US",
                true,
                "`|=` matches the value followed by `-`",
            ),
            (
                "button[name|=\"en\"]",
                "english",
                false,
                "`|=` needs the hyphen",
            ),
            (
                "button[name~=\"b\"]",
                "a b c",
                true,
                "`~=` matches one space-separated word",
            ),
            (
                "button[name~=\"b\"]",
                "abc",
                false,
                "`~=` does not match inside a word",
            ),
            (
                "button[name~=\"\"]",
                "",
                true,
                "`\"\".split(' ')` is `[\"\"]`, so `~=\"\"` matches an empty name",
            ),
            (
                "button[name~=\"\"]",
                "a b",
                false,
                "a normalized name has no empty word",
            ),
            (
                "button[name=\"hello\"i]",
                "HeLLo",
                true,
                "`i` compares after upper-casing both sides",
            ),
            (
                "button[name=\"hello\"s]",
                "HeLLo",
                false,
                "`s` is case-sensitive",
            ),
            (
                "button[name=\" \n Hello \"]",
                " Hello ",
                true,
                "both sides are whitespace-normalized",
            ),
        ];
        for (body, name, expected, why) in cases {
            let filter = name_filter(body, RoleEngine::Role)?;
            assert_eq!(filter.matches(name), expected, "{body} on {name:?}: {why}");
        }
        Ok(())
    }

    #[test]
    fn internal_role_widens_only_case_insensitive_equality() -> Result<(), Box<dyn Error>> {
        let cases = [
            (
                "button[name=\"ell\"i]",
                RoleEngine::InternalRole,
                true,
                "a non-exact `=` is a substring under `internal:role`",
            ),
            (
                "button[name=\"ell\"i]",
                RoleEngine::Role,
                false,
                "`role` keeps `=` as equality",
            ),
            (
                "button[name=\"ell\"s]",
                RoleEngine::InternalRole,
                false,
                "an exact `=` stays equality",
            ),
            (
                "button[name^=\"ell\"i]",
                RoleEngine::InternalRole,
                false,
                "only `=` is widened",
            ),
        ];
        for (body, engine, expected, why) in cases {
            let filter = name_filter(body, engine)?;
            assert_eq!(
                filter.matches("Hello"),
                expected,
                "{body} under {engine:?}: {why}"
            );
        }
        Ok(())
    }

    #[test]
    fn case_insensitive_comparison_upper_cases_with_full_unicode_mappings()
    -> Result<(), Box<dyn Error>> {
        let filter = name_filter("button[name=\"straße\"i]", RoleEngine::Role)?;
        assert!(
            filter.matches("STRASSE"),
            "`toUpperCase` maps ß to SS, so \"straße\"i equals STRASSE"
        );
        assert!(
            filter.matches("Straße"),
            "both sides map ß to SS, so the original spelling still matches"
        );
        let exact = name_filter("button[name=\"straße\"]", RoleEngine::Role)?;
        assert!(
            !exact.matches("STRASSE"),
            "an exact filter compares the strings as written"
        );
        Ok(())
    }

    #[test]
    fn a_regex_is_never_upper_cased() -> Result<(), Box<dyn Error>> {
        let filter = name_filter("button[name=/^hello$/]", RoleEngine::InternalRole)?;
        assert!(
            !filter.matches("HELLO"),
            "a regex filter is case-sensitive, so neither side is upper-cased"
        );
        assert!(filter.matches("hello"), "the regex matches its own case");
        let flagged = name_filter("button[name=/^hello$/i]", RoleEngine::Role)?;
        assert!(flagged.matches("HELLO"), "the `i` flag is the regex's own");
        let spaced = name_filter("button[name=/^a b$/]", RoleEngine::Role)?;
        assert!(
            spaced.matches("  a \n b "),
            "a regex runs on the whitespace-normalized name"
        );
        Ok(())
    }
}
