//! The tree's role locator for one element: the `aka` a strict-mode violation lists it by
//! (apc-engine-design 5.5).
//!
//! Upstream's selector generator offers `getByRole(role, { name })` for every element that has a
//! role and a name, scored 100 (`selectorGenerator.ts` at v1.64.1), below every other candidate
//! but a test id. The page runs the generator with the stand-in role engine, which knows no role,
//! so it never offers one; the tree does, and a caller puts it in place of the page's `aka`
//! unless that is a test id. Only the role-with-name branch is ported: its text alternatives,
//! their order by score and the strict-uniqueness check, with the check run as a tree stage over
//! the element's document. A name the alternatives cannot make unique keeps the page's locator,
//! where upstream would chain a parent or add `nth`.

use serde_json::json;

use super::accessible_text::AccessibleText;
use super::chunk_plan::{ChunkPlan, PlannedStage};
use super::parsed_selector::ParsedSelector;
use super::ref_resolver::RefResolver;
use super::tree_stage::Scope;
use crate::tree::{NameVisibility, NodeId, SnapshotTree};

/// Upstream's scores for the candidates ported here (`selectorGenerator.ts:37-59`).
const ROLE_WITH_NAME_SCORE: u32 = 100;
const EXACT_PENALTY: u32 = 5;
const TEXT_SCORE_RANGE: u32 = 10;
const BEGIN_PENALIZED_SCORE: u32 = 50;
const END_PENALIZED_SCORE: u32 = 300;

/// One role candidate: its name text, whether it matches exactly, and its score.
struct Candidate {
    name: String,
    exact: bool,
    score: u32,
}

/// `node`'s role locator as `asLocator` prints it (`getByRole('button', { name: 'Pay' })`), when
/// the node has a role other than `none` or `presentation`, a name that is not private-use
/// glyphs only, and a role candidate that selects it alone in its document.
#[must_use]
pub fn role_aka(
    tree: &SnapshotTree,
    text: &dyn AccessibleText,
    refs: &dyn RefResolver,
    node: NodeId,
) -> Option<String> {
    let found = tree.node(node)?;
    let role = found.role()?.as_str();
    if matches!(role, "none" | "presentation") {
        return None;
    }
    let name = text.name(node, NameVisibility::VisibleOnly).into_owned();
    if name.is_empty() || name.chars().all(is_private_use) {
        return None;
    }
    let doc_root = tree.document(found.doc())?.root_node()?;
    let mut candidates = vec![candidate(role, name.clone(), true, 0)];
    for (alternative, bonus) in suitable_text_alternatives(&name) {
        candidates.push(candidate(role, alternative, false, bonus));
    }
    // Stable, as upstream's sort is: equal scores keep the order they were offered in.
    candidates.sort_by_key(|candidate| candidate.score);
    candidates.into_iter().find_map(|candidate| {
        let body = role_body(role, &candidate);
        let parsed: ParsedSelector = serde_json::from_value(json!({
            "parts": [{ "name": "internal:role", "body": body, "source": body }],
        }))
        .ok()?;
        let plan = ChunkPlan::of(&parsed).ok()?;
        let PlannedStage::Tree(run) = plan.stages().ok()?.first() else {
            return None;
        };
        let matches = run
            .stage()
            .select(tree, Scope::Root(doc_root), text, refs)
            .ok()?;
        (matches.nodes() == [node]).then(|| role_locator(role, &candidate))
    })
}

fn candidate(role: &str, name: String, exact: bool, bonus: u32) -> Candidate {
    let base = if exact {
        ROLE_WITH_NAME_SCORE + EXACT_PENALTY
    } else {
        ROLE_WITH_NAME_SCORE - bonus
    };
    let mut candidate = Candidate {
        name,
        exact,
        score: base,
    };
    // `penalizeScoreForLength`: a candidate in the text range pays for its selector's length.
    if candidate.score > BEGIN_PENALIZED_SCORE && candidate.score < END_PENALIZED_SCORE {
        let length = u32::try_from(role_body(role, &candidate).chars().count() / 10)
            .unwrap_or(TEXT_SCORE_RANGE);
        candidate.score += length.min(TEXT_SCORE_RANGE);
    }
    candidate
}

/// The candidate's `internal:role` body, as `getByRoleSelector` writes it.
fn role_body(role: &str, candidate: &Candidate) -> String {
    format!(
        "{role}[name={}]",
        escape_for_attribute_selector(&candidate.name, candidate.exact)
    )
}

/// `asLocator`'s JavaScript form of the candidate.
fn role_locator(role: &str, candidate: &Candidate) -> String {
    let exact = if candidate.exact { ", exact: true" } else { "" };
    format!(
        "getByRole({}, {{ name: {}{exact} }})",
        quote(role),
        quote(&candidate.name)
    )
}

/// `escapeForAttributeSelector` for a string (`isomorphic/stringUtils.ts:118-126`).
fn escape_for_attribute_selector(value: &str, exact: bool) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"{}", if exact { 's' } else { 'i' })
}

/// `escapeWithQuotes(text, "'")` (`isomorphic/stringUtils.ts:18-28`): `JSON.stringify`'s escapes
/// with double quotes left bare and single quotes escaped.
fn quote(text: &str) -> String {
    let stringified = serde_json::to_string(text).unwrap_or_default();
    let inner = stringified
        .get(1..stringified.len().saturating_sub(1))
        .unwrap_or_default()
        .replace("\\\"", "\"")
        .replace('\'', "\\'");
    format!("'{inner}'")
}

/// Whether `c` is in Unicode's private use areas (`\p{Co}`): icon-font glyphs, which make no
/// locator.
fn is_private_use(c: char) -> bool {
    matches!(u32::from(c), 0xE000..=0xF8FF | 0xF_0000..=0xF_FFFD | 0x10_0000..=0x10_FFFD)
}

/// `suitableTextAlternatives` (`selectorGenerator.ts:502-535`): the name with a leading or
/// trailing number left out, and cut at a word boundary when it is long, each with its bonus.
fn suitable_text_alternatives(text: &str) -> Vec<(String, u32)> {
    let mut result = Vec::new();
    let leading = leading_number_length(text);
    if leading > 0 {
        let alternative =
            trim_word_boundary(text.get(leading..).unwrap_or_default().trim_start(), 80);
        let bonus = if alternative.chars().count() <= 30 {
            2
        } else {
            1
        };
        result.push((alternative, bonus));
    }
    let trailing = trailing_number_length(text);
    if trailing > 0 {
        let end = text.len() - trailing;
        let alternative = trim_word_boundary(text.get(..end).unwrap_or_default().trim_end(), 80);
        let bonus = if alternative.chars().count() <= 30 {
            2
        } else {
            1
        };
        result.push((alternative, bonus));
    }
    if text.chars().count() <= 30 {
        result.push((text.to_owned(), 0));
    } else {
        result.push((trim_word_boundary(text, 80), 0));
        result.push((trim_word_boundary(text, 30), 1));
    }
    result.retain(|(alternative, _)| !alternative.is_empty());
    if result.is_empty() {
        result.push((text.chars().take(80).collect(), 0));
    }
    result
}

/// `/^([\d.,]+)[^.,\w]/`: the byte length of a leading run of digits, dots and commas that a
/// character other than those and word characters follows.
fn leading_number_length(text: &str) -> usize {
    let run = text
        .char_indices()
        .find(|&(_, c)| !is_number_char(c))
        .map_or(text.len(), |(at, _)| at);
    match text.get(run..).and_then(|rest| rest.chars().next()) {
        Some(next) if run > 0 && !is_number_char(next) && !is_word_char(next) => run,
        _ => 0,
    }
}

/// `/[^.,\w]([\d.,]+)$/`: the byte length of a trailing run of digits, dots and commas that a
/// character other than those and word characters precedes.
fn trailing_number_length(text: &str) -> usize {
    let start = text
        .char_indices()
        .rev()
        .find(|&(_, c)| !is_number_char(c))
        .map_or(0, |(at, c)| at + c.len_utf8());
    let run = text.len() - start;
    match text
        .get(..start)
        .and_then(|before| before.chars().next_back())
    {
        Some(before) if run > 0 && !is_number_char(before) && !is_word_char(before) => run,
        _ => 0,
    }
}

fn is_number_char(c: char) -> bool {
    c.is_ascii_digit() || c == '.' || c == ','
}

/// JavaScript's `\w`: ASCII letters, digits and `_`.
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `trimWordBoundary` (`selectorGenerator.ts:491-500`): the first `max` characters cut back to
/// their last word boundary, as `/^(.*)\b(.+?)$/` finds it, then trimmed at the end.
fn trim_word_boundary(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let cut: Vec<char> = text.chars().take(max).collect();
    // The last position with at least one character after it where `\b` holds: between a word
    // character and a non-word one, or at the start before a word character.
    let boundary = (0..cut.len()).rev().find(|&at| {
        let before = at.checked_sub(1).and_then(|i| cut.get(i)).copied();
        let after = cut.get(at).copied();
        before.is_some_and(is_word_char) != after.is_some_and(is_word_char)
    });
    match boundary {
        Some(at) => cut
            .iter()
            .take(at)
            .collect::<String>()
            .trim_end()
            .to_owned(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{quote, suitable_text_alternatives, trim_word_boundary};

    #[test]
    fn tests_that_names_split_and_cut_as_upstreams_alternatives_do() {
        assert_eq!(
            suitable_text_alternatives("Pay"),
            vec![("Pay".to_owned(), 0)]
        );
        assert_eq!(
            suitable_text_alternatives("12 items"),
            vec![("items".to_owned(), 2), ("12 items".to_owned(), 0)]
        );
        assert_eq!(
            suitable_text_alternatives("Inbox (3)"),
            vec![("Inbox (3)".to_owned(), 0)],
            "a closing parenthesis is neither a number nor a word character after the run"
        );
        assert_eq!(
            suitable_text_alternatives("Page 4"),
            vec![("Page".to_owned(), 2), ("Page 4".to_owned(), 0)]
        );
        assert_eq!(trim_word_boundary("one two three", 9), "one two");
        assert_eq!(trim_word_boundary("abcdefghij", 5), "");
    }

    #[test]
    fn tests_that_a_name_quotes_as_as_locator_does() {
        assert_eq!(quote("Pay"), "'Pay'");
        assert_eq!(quote("it's \"x\""), r#"'it\'s "x"'"#);
        assert_eq!(quote("a\nb\\c"), r"'a\nb\\c'");
    }
}
