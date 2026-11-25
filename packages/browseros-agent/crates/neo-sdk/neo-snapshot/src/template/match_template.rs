use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::num::FpCategory;

use serde_json::Number;

use crate::aria_node::{AriaChild, AriaNode, Invalid, Tristate};

use super::aria_template_node::{
    AriaTemplateNode, AriaTemplateRoleNode, AriaTextValue, ContainerMode, TemplateName,
};
use super::js_regex::{JsRegex, RegexError};

/// One subtree of a snapshot matched by [`match_template`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TemplateMatch<'a> {
    /// Playwright's synthetic root `fragment` container around the top-level snapshot slice.
    Root,
    /// A concrete node inside the snapshot tree.
    Node(&'a AriaNode),
}

/// Errors returned while matching an [`AriaTemplateNode`] against a snapshot tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplateError {
    /// A template role node's `name` regular expression failed JavaScript compilation.
    InvalidRegex(RegexError),
}

impl fmt::Display for TemplateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRegex(error) => write!(formatter, "invalid template regex: {error}"),
        }
    }
}

impl Error for TemplateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidRegex(error) => Some(error),
        }
    }
}

/// Reports whether `template` matches any subtree of `tree`, stopping at the first match in
/// pre-order depth-first order (`collectAll = false` in Playwright 1.64.1's `matchesNodeDeep`).
///
/// The walk and container comparison are both iterative on explicit heap stacks, so arbitrarily
/// deep trees and templates cannot overflow the thread's stack.
///
/// Attribute parity note: matching explicit negative or zero template filters such as
/// `[checked=false]`, `[disabled=false]`, `[expanded=false]`, `[invalid=false]`,
/// `[pressed=false]`, `[selected=false]` or `[level=0]` compares strictly against the snapshot
/// node's field, so the input [`AriaNode`] must carry explicit `Some(false)`,
/// `Some(Tristate::False)`, `Some(Invalid::False)` or `Some(0)` on roles that own those states
/// (matching Playwright's `toAriaNode`).
pub fn matches_template(
    tree: &[AriaNode],
    template: &AriaTemplateNode,
) -> Result<bool, TemplateError> {
    let matches = matches_node_deep(tree, template, Collect::First)?;
    Ok(!matches.is_empty())
}

/// Collects every top-most subtree of `tree` matching `template` in pre-order depth-first order,
/// without descending into matched subtrees (`collectAll = true` in Playwright 1.64.1's
/// `matchesNodeDeep`).
///
/// A matching text leaf reports its parent container ([`TemplateMatch::Root`] for a top-level
/// text node, or [`TemplateMatch::Node`] for a text child of an element). Both the tree walk and
/// recursive container checks run iteratively on explicit heap stacks with per-call regex caching.
///
/// Attribute parity note: matching explicit negative or zero template filters such as
/// `[checked=false]`, `[disabled=false]`, `[expanded=false]`, `[invalid=false]`,
/// `[pressed=false]`, `[selected=false]` or `[level=0]` compares strictly against the snapshot
/// node's field, so the input [`AriaNode`] must carry explicit `Some(false)`,
/// `Some(Tristate::False)`, `Some(Invalid::False)` or `Some(0)` on roles that own those states
/// (matching Playwright's `toAriaNode`).
pub fn match_template<'a>(
    tree: &'a [AriaNode],
    template: &AriaTemplateNode,
) -> Result<Vec<TemplateMatch<'a>>, TemplateError> {
    matches_node_deep(tree, template, Collect::All)
}

/// Whether the deep traversal stops at the first match or collects every top-most match.
#[derive(Clone, Copy)]
enum Collect {
    First,
    All,
}

/// Whether an unspecified `container_mode` inherits `deep-equal` from an ancestor template node.
#[derive(Clone, Copy)]
enum DepthMode {
    Default,
    DeepEqual,
}

/// Active child-list comparison mode for one role container frame.
#[derive(Clone, Copy)]
enum ListMode {
    /// Greedy ordered subsequence (`containsList`), always passing [`DepthMode::Default`] to
    /// children.
    Contain,
    /// Exact 1-to-1 sequence (`listEqual`), propagating its [`DepthMode`] to children.
    Equal(DepthMode),
}

/// Uniform borrowed view of a snapshot tree item matching Playwright's internal `AriaNode | string`
/// representation over Rust's [`AriaNode`] JSON shape.
#[derive(Clone, Copy)]
enum TreeItem<'a> {
    /// Synthetic `{ role: "fragment", name: "", children: [...] }` root wrapping `tree`.
    Root(&'a [AriaNode]),
    /// Element node (any [`AriaNode`] whose `role` is not `"text"`).
    Node(&'a AriaNode),
    /// Text leaf: an [`AriaChild::Text`], a single `node.text` child, or an `AriaNode` with
    /// `role == "text"`.
    Text(&'a str),
}

impl<'a> TreeItem<'a> {
    fn from_node(node: &'a AriaNode) -> Self {
        if node.role == "text" {
            Self::Text(node.text.as_deref().unwrap_or_default())
        } else {
            Self::Node(node)
        }
    }

    fn from_child(child: &'a AriaChild) -> Self {
        match child {
            AriaChild::Text(text) => Self::Text(text),
            AriaChild::Node(node) => Self::from_node(node),
        }
    }
}

/// Allocation-free view of a [`TreeItem`]'s effective children.
#[derive(Clone, Copy)]
enum Children<'a> {
    Empty,
    Root(&'a [AriaNode]),
    SingleText(&'a str),
    Slice(&'a [AriaChild]),
}

impl<'a> Children<'a> {
    fn of(item: TreeItem<'a>) -> Self {
        match item {
            TreeItem::Root(nodes) => Self::Root(nodes),
            // `renderAriaTreeAsJSON` and `tree_text` treat `node.text` as the node's sole child.
            TreeItem::Node(node) => {
                if let Some(text) = node.text.as_deref() {
                    Self::SingleText(text)
                } else if node.children.is_empty() {
                    Self::Empty
                } else {
                    Self::Slice(&node.children)
                }
            }
            TreeItem::Text(_) => Self::Empty,
        }
    }

    fn len(self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Root(nodes) => nodes.len(),
            Self::SingleText(_) => 1,
            Self::Slice(children) => children.len(),
        }
    }

    fn get(self, index: usize) -> Option<TreeItem<'a>> {
        match self {
            Self::Empty => None,
            Self::Root(nodes) => nodes.get(index).map(TreeItem::from_node),
            Self::SingleText(text) => (index == 0).then_some(TreeItem::Text(text)),
            Self::Slice(children) => children.get(index).map(TreeItem::from_child),
        }
    }
}

/// Borrowed attributes of a [`TreeItem::Root`] or [`TreeItem::Node`] inspected by `matchesNode`.
#[derive(Clone, Copy)]
struct RoleProps<'a> {
    role: &'a str,
    name: Option<&'a str>,
    checked: Option<Tristate>,
    disabled: Option<bool>,
    expanded: Option<bool>,
    invalid: Option<Invalid>,
    level: Option<&'a Number>,
    pressed: Option<Tristate>,
    selected: Option<bool>,
    url: Option<&'a str>,
}

impl<'a> RoleProps<'a> {
    const ROOT: Self = Self {
        role: "fragment",
        name: None,
        checked: None,
        disabled: None,
        expanded: None,
        invalid: None,
        level: None,
        pressed: None,
        selected: None,
        url: None,
    };

    fn from_node(node: &'a AriaNode) -> Self {
        Self {
            role: &node.role,
            name: node.name.as_deref(),
            checked: node.checked,
            disabled: node.disabled,
            expanded: node.expanded,
            invalid: node.invalid,
            level: node.level.as_ref(),
            pressed: node.pressed,
            selected: node.selected,
            url: node.url.as_deref(),
        }
    }
}

/// One pending `(item, template)` comparison in the iterative `matches_node` loop.
struct MatchTask<'a, 't> {
    item: TreeItem<'a>,
    template: &'t AriaTemplateNode,
    depth_mode: DepthMode,
}

/// Active container comparison suspended while checking one child pair.
struct MatchFrame<'a, 't> {
    children: Children<'a>,
    templates: &'t [AriaTemplateNode],
    mode: ListMode,
    child_index: usize,
    template_index: usize,
}

/// Result of advancing a [`MatchFrame`] after its active child check completes.
enum FrameStep<'a, 't> {
    Done(bool),
    Next(MatchTask<'a, 't>),
}

impl<'a, 't> MatchFrame<'a, 't> {
    fn start(&mut self) -> Option<MatchTask<'a, 't>> {
        let item = self.children.get(0)?;
        let template = self.templates.first()?;
        self.child_index = 1;
        let depth_mode = match self.mode {
            ListMode::Contain => DepthMode::Default,
            ListMode::Equal(depth) => {
                self.template_index = 1;
                depth
            }
        };
        Some(MatchTask {
            item,
            template,
            depth_mode,
        })
    }

    fn advance(&mut self, child_matched: bool) -> FrameStep<'a, 't> {
        match self.mode {
            ListMode::Equal(depth_mode) => {
                if !child_matched {
                    return FrameStep::Done(false);
                }
                if self.template_index == self.templates.len() {
                    return FrameStep::Done(true);
                }
                match (
                    self.children.get(self.child_index),
                    self.templates.get(self.template_index),
                ) {
                    (Some(item), Some(template)) => {
                        self.child_index += 1;
                        self.template_index += 1;
                        FrameStep::Next(MatchTask {
                            item,
                            template,
                            depth_mode,
                        })
                    }
                    _ => FrameStep::Done(false),
                }
            }
            ListMode::Contain => {
                if child_matched {
                    self.template_index += 1;
                    if self.template_index == self.templates.len() {
                        return FrameStep::Done(true);
                    }
                }
                let remaining_children = self.children.len().saturating_sub(self.child_index);
                let remaining_templates = self.templates.len().saturating_sub(self.template_index);
                if remaining_children < remaining_templates {
                    return FrameStep::Done(false);
                }
                match (
                    self.children.get(self.child_index),
                    self.templates.get(self.template_index),
                ) {
                    (Some(item), Some(template)) => {
                        self.child_index += 1;
                        FrameStep::Next(MatchTask {
                            item,
                            template,
                            depth_mode: DepthMode::Default,
                        })
                    }
                    _ => FrameStep::Done(false),
                }
            }
        }
    }
}

/// Immediate outcome of checking one `(item, template)` pair before descending into children.
enum ShallowOutcome<'a, 't> {
    Matched,
    NotMatched,
    CheckChildren(MatchFrame<'a, 't>),
}

/// One pending node in the iterative pre-order `matches_node_deep` walk.
struct DeepEntry<'a> {
    item: TreeItem<'a>,
    parent: Option<TemplateMatch<'a>>,
}

/// Per-call cache of compiled JavaScript regular expressions, keyed by borrowed pattern slice so
/// matching a large tree compiles each pattern at most once.
struct RegexCache<'t> {
    entries: HashMap<&'t str, Result<JsRegex, RegexError>>,
}

impl<'t> RegexCache<'t> {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn get_or_compile(&mut self, pattern: &'t str) -> Result<&JsRegex, &RegexError> {
        self.entries
            .entry(pattern)
            .or_insert_with(|| JsRegex::compile(pattern))
            .as_ref()
    }
}

/// Iterative pre-order DFS port of Playwright 1.64.1's `matchesNodeDeep`.
fn matches_node_deep<'a>(
    tree: &'a [AriaNode],
    template: &AriaTemplateNode,
    collect: Collect,
) -> Result<Vec<TemplateMatch<'a>>, TemplateError> {
    let mut results = Vec::new();
    let mut cache = RegexCache::new();
    let mut match_stack = Vec::new();
    let mut walk_stack = vec![DeepEntry {
        item: TreeItem::Root(tree),
        parent: None,
    }];

    while let Some(entry) = walk_stack.pop() {
        if matches_node(entry.item, template, &mut cache, &mut match_stack)? {
            let matched = match entry.item {
                TreeItem::Root(_) => Some(TemplateMatch::Root),
                TreeItem::Node(node) => Some(TemplateMatch::Node(node)),
                TreeItem::Text(_) => entry.parent,
            };
            if let Some(matched) = matched {
                results.push(matched);
            }
            match collect {
                Collect::First => return Ok(results),
                // Matches Playwright `ariaSnapshot.ts:446-451`: never descend into a matched
                // subtree when collecting all matches.
                Collect::All => continue,
            }
        }

        let next_parent = match entry.item {
            TreeItem::Root(_) => TemplateMatch::Root,
            TreeItem::Node(node) => TemplateMatch::Node(node),
            TreeItem::Text(_) => continue,
        };
        let children = Children::of(entry.item);
        // Push in reverse order so the LIFO stack visits children in forward document order.
        for index in (0..children.len()).rev() {
            if let Some(child) = children.get(index) {
                walk_stack.push(DeepEntry {
                    item: child,
                    parent: Some(next_parent),
                });
            }
        }
    }

    Ok(results)
}

/// Iterative port of Playwright 1.64.1's `matchesNode`, `listEqual` and `containsList`.
fn matches_node<'a, 't>(
    root_item: TreeItem<'a>,
    root_template: &'t AriaTemplateNode,
    cache: &mut RegexCache<'t>,
    stack: &mut Vec<MatchFrame<'a, 't>>,
) -> Result<bool, TemplateError> {
    stack.clear();

    let mut current = Some(MatchTask {
        item: root_item,
        template: root_template,
        depth_mode: DepthMode::Default,
    });
    let mut last_result = false;

    loop {
        if let Some(task) = current.take() {
            match check_shallow(task.item, task.template, task.depth_mode, cache)? {
                ShallowOutcome::Matched => last_result = true,
                ShallowOutcome::NotMatched => last_result = false,
                ShallowOutcome::CheckChildren(mut frame) => {
                    if let Some(next) = frame.start() {
                        stack.push(frame);
                        current = Some(next);
                        continue;
                    }
                    last_result = false;
                }
            }
        }

        let Some(frame) = stack.last_mut() else {
            return Ok(last_result);
        };

        match frame.advance(last_result) {
            FrameStep::Done(matched) => {
                stack.pop();
                last_result = matched;
            }
            FrameStep::Next(next) => {
                current = Some(next);
            }
        }
    }
}

/// Checks one `(item, template)` node without recursing into its children.
fn check_shallow<'a, 't>(
    item: TreeItem<'a>,
    template: &'t AriaTemplateNode,
    depth_mode: DepthMode,
    cache: &mut RegexCache<'t>,
) -> Result<ShallowOutcome<'a, 't>, TemplateError> {
    match (item, template) {
        (TreeItem::Text(text), AriaTemplateNode::Text(expected)) => {
            if matches_text_value(Some(text), Some(&expected.text), cache) {
                Ok(ShallowOutcome::Matched)
            } else {
                Ok(ShallowOutcome::NotMatched)
            }
        }
        (TreeItem::Root(_), AriaTemplateNode::Role(expected)) => check_role_children(
            RoleProps::ROOT,
            Children::of(item),
            expected,
            depth_mode,
            cache,
        ),
        (TreeItem::Node(node), AriaTemplateNode::Role(expected)) => check_role_children(
            RoleProps::from_node(node),
            Children::of(item),
            expected,
            depth_mode,
            cache,
        ),
        (TreeItem::Text(_), AriaTemplateNode::Role(_))
        | (TreeItem::Root(_) | TreeItem::Node(_), AriaTemplateNode::Text(_)) => {
            Ok(ShallowOutcome::NotMatched)
        }
    }
}

/// Checks a role node's own attributes and prepares its child-list comparison frame if needed.
fn check_role_children<'a, 't>(
    props: RoleProps<'_>,
    children: Children<'a>,
    expected: &'t AriaTemplateRoleNode,
    depth_mode: DepthMode,
    cache: &mut RegexCache<'t>,
) -> Result<ShallowOutcome<'a, 't>, TemplateError> {
    if !matches_role_attrs(props, expected, cache)? {
        return Ok(ShallowOutcome::NotMatched);
    }
    let templates = expected.children.as_slice();
    // Container mode precedence matches Playwright `ariaSnapshot.ts:406-412`.
    let mode = match (expected.container_mode, depth_mode) {
        (Some(ContainerMode::Contain), DepthMode::Default | DepthMode::DeepEqual)
        | (None, DepthMode::Default) => ListMode::Contain,
        (Some(ContainerMode::Equal), DepthMode::Default | DepthMode::DeepEqual) => {
            ListMode::Equal(DepthMode::Default)
        }
        (Some(ContainerMode::DeepEqual), DepthMode::Default | DepthMode::DeepEqual)
        | (None, DepthMode::DeepEqual) => ListMode::Equal(DepthMode::DeepEqual),
    };
    let length_ok = match mode {
        ListMode::Equal(_) => children.len() == templates.len(),
        ListMode::Contain => templates.len() <= children.len(),
    };
    if !length_ok {
        return Ok(ShallowOutcome::NotMatched);
    }
    if templates.is_empty() {
        return Ok(ShallowOutcome::Matched);
    }
    Ok(ShallowOutcome::CheckChildren(MatchFrame {
        children,
        templates,
        mode,
        child_index: 0,
        template_index: 0,
    }))
}

/// Checks a role node's attributes, accessible name and `/url` property in Playwright's exact order.
fn matches_role_attrs<'t>(
    props: RoleProps<'_>,
    template: &'t AriaTemplateRoleNode,
    cache: &mut RegexCache<'t>,
) -> Result<bool, TemplateError> {
    if template.role != "fragment" && template.role != props.role {
        return Ok(false);
    }
    if template.checked.is_some() && template.checked != props.checked {
        return Ok(false);
    }
    if template.disabled.is_some() && template.disabled != props.disabled {
        return Ok(false);
    }
    if template.expanded.is_some() && template.expanded != props.expanded {
        return Ok(false);
    }
    if template.invalid.is_some() && template.invalid != props.invalid {
        return Ok(false);
    }
    if let Some(expected_level) = &template.level
        && !props
            .level
            .is_some_and(|actual| numbers_equal(expected_level, actual))
    {
        return Ok(false);
    }
    if template.pressed.is_some() && template.pressed != props.pressed {
        return Ok(false);
    }
    if template.selected.is_some() && template.selected != props.selected {
        return Ok(false);
    }
    if !matches_string_or_regex(props.name, template.name.as_ref(), cache)? {
        return Ok(false);
    }
    Ok(matches_text_value(
        props.url,
        template.props.get("url"),
        cache,
    ))
}

/// Port of Playwright 1.64.1's `matchesStringOrRegex` (`ariaSnapshot.ts:308-316`).
fn matches_string_or_regex<'t>(
    text: Option<&str>,
    template: Option<&'t TemplateName>,
    cache: &mut RegexCache<'t>,
) -> Result<bool, TemplateError> {
    let Some(expected) = template else {
        return Ok(true);
    };
    if matches!(expected, TemplateName::Exact(exact) if exact.is_empty()) {
        return Ok(true);
    }
    // Check emptiness before compiling a regex so an invalid pattern on an unnamed node returns
    // `Ok(false)` just like Playwright's `if (!text) return false`.
    let Some(actual) = text.filter(|value| !value.is_empty()) else {
        return Ok(false);
    };
    match expected {
        TemplateName::Exact(exact) => Ok(actual == exact),
        TemplateName::Regex(regex) => match cache.get_or_compile(&regex.pattern) {
            Ok(compiled) => Ok(compiled.is_match(actual)),
            Err(error) => Err(TemplateError::InvalidRegex(error.clone())),
        },
    }
}

/// Port of Playwright 1.64.1's `matchesTextValue` and `cachedRegex` (`ariaSnapshot.ts:318-351`).
fn matches_text_value<'t>(
    text: Option<&str>,
    template: Option<&'t AriaTextValue>,
    cache: &mut RegexCache<'t>,
) -> bool {
    let Some(expected) = template.filter(|value| !value.normalized.is_empty()) else {
        return true;
    };
    let Some(actual) = text.filter(|value| !value.is_empty()) else {
        return false;
    };
    if actual == expected.normalized || actual == expected.raw {
        return true;
    }
    // `strip_prefix('/')` followed by `strip_suffix('/')` succeeds iff `raw.len() > 1` and both
    // ends are `'/'`, returning the inner slice without byte indexing.
    let Some(pattern) = expected
        .raw
        .strip_prefix('/')
        .and_then(|rest| rest.strip_suffix('/'))
    else {
        return false;
    };
    // Invalid `/.../` text patterns cache their compile failure and return `false`, matching
    // Playwright's `try { ... } catch { regex = null }`.
    cache
        .get_or_compile(pattern)
        .is_ok_and(|compiled| compiled.is_match(actual))
}

/// Compares two JSON numbers with JavaScript `===` semantics so `2 == 2.0` and `-0.0 == 0`.
fn numbers_equal(lhs: &Number, rhs: &Number) -> bool {
    json_number_bits(lhs) == json_number_bits(rhs)
}

/// Canonical IEEE 754 bit pattern of the double JavaScript's `JSON.parse` produces for `number`,
/// collapsing `-0.0` to `+0.0`.
fn json_number_bits(number: &Number) -> u64 {
    let double = number.as_f64().unwrap_or_else(|| {
        if number.to_string().starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        }
    });
    if double.classify() == FpCategory::Zero {
        0
    } else {
        double.to_bits()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error;
    use std::slice;

    use serde_json::{Number, json};

    use crate::aria_node::{AriaChild, AriaNode, Invalid, Tristate};

    use super::super::aria_template_node::{
        AriaRegex, AriaTemplateNode, AriaTemplateRoleNode, AriaTemplateTextNode, AriaTextValue,
        ContainerMode, TemplateName,
    };
    use super::{TemplateError, TemplateMatch, match_template, matches_template};

    fn aria_node(role: &str) -> AriaNode {
        AriaNode {
            role: role.to_owned(),
            name: None,
            checked: None,
            disabled: None,
            expanded: None,
            active: None,
            inert: None,
            on_top: None,
            invalid: None,
            level: None,
            pressed: None,
            selected: None,
            aria_hidden: None,
            r#ref: None,
            cursor: None,
            r#box: None,
            url: None,
            placeholder: None,
            text: None,
            children: Vec::new(),
        }
    }

    fn role_template(role: &str) -> AriaTemplateRoleNode {
        AriaTemplateRoleNode {
            role: role.to_owned(),
            ..AriaTemplateRoleNode::default()
        }
    }

    fn text_value(raw: &str, normalized: &str) -> AriaTextValue {
        AriaTextValue {
            raw: raw.to_owned(),
            normalized: normalized.to_owned(),
        }
    }

    fn text_template(raw: &str, normalized: &str) -> AriaTemplateNode {
        AriaTemplateNode::Text(AriaTemplateTextNode {
            text: text_value(raw, normalized),
        })
    }

    #[test]
    fn matches_root_fragment_and_collects_without_descending_into_matched_subtrees()
    -> Result<(), Box<dyn Error>> {
        let tree: Vec<AriaNode> = serde_json::from_value(json!([
            {
                "role": "group",
                "name": "outer",
                "children": [
                    { "role": "group", "name": "inner" },
                    { "role": "button", "name": "A" }
                ]
            },
            { "role": "group", "name": "sibling" }
        ]))?;

        let fragment = AriaTemplateNode::Role(role_template("fragment"));
        assert_eq!(
            match_template(&tree, &fragment)?,
            vec![TemplateMatch::Root],
            "an empty fragment matches the synthetic root and does not descend"
        );
        assert!(
            matches_template(&[], &fragment)?,
            "an empty fragment matches an empty tree via the synthetic root"
        );

        let group = AriaTemplateNode::Role(role_template("group"));
        let found = match_template(&tree, &group)?;
        assert_eq!(
            found,
            vec![TemplateMatch::Node(&tree[0]), TemplateMatch::Node(&tree[1])],
            "collect_all collects sibling groups in pre-order without descending into outer's inner group"
        );
        assert!(
            matches_template(&tree, &group)?,
            "matches_template stops at the first matching group"
        );
        Ok(())
    }

    #[test]
    fn exposes_text_nodes_and_node_text_with_playwright_parent_semantics()
    -> Result<(), Box<dyn Error>> {
        let tree: Vec<AriaNode> = serde_json::from_value(json!([
            { "role": "text", "text": "top-level" },
            { "role": "paragraph", "text": "single child" },
            {
                "role": "paragraph",
                "children": [
                    "plain ",
                    { "role": "text", "text": "nested-role-text" }
                ]
            }
        ]))?;

        let top_text = text_template("top-level", "top-level");
        assert_eq!(
            match_template(&tree, &top_text)?,
            vec![TemplateMatch::Root],
            "a top-level role=text node resolves its match to the synthetic Root"
        );

        let role_text = AriaTemplateNode::Role(role_template("text"));
        assert!(
            !matches_template(&tree, &role_text)?,
            "role=text nodes are exposed as text leaves, not role objects"
        );

        let single_child_text = text_template("single child", "single child");
        assert_eq!(
            match_template(&tree, &single_child_text)?,
            vec![TemplateMatch::Node(&tree[1])],
            "node.text acts as the node's single effective text child and resolves to that node"
        );

        let nested_role_text = text_template("nested-role-text", "nested-role-text");
        assert_eq!(
            match_template(&tree, &nested_role_text)?,
            vec![TemplateMatch::Node(&tree[2])],
            "a nested AriaChild::Node with role=text acts as a text child of its parent"
        );

        let mut paragraph_with_both = aria_node("paragraph");
        paragraph_with_both.text = Some("winning text".to_owned());
        paragraph_with_both
            .children
            .push(AriaChild::Text("ignored child".to_owned()));
        let winning = text_template("winning text", "winning text");
        let ignored = text_template("ignored child", "ignored child");
        assert!(
            matches_template(slice::from_ref(&paragraph_with_both), &winning)?,
            "node.text supplies the single effective child when present"
        );
        assert!(
            !matches_template(&[paragraph_with_both], &ignored)?,
            "node.children is ignored when node.text is present"
        );
        Ok(())
    }

    #[test]
    fn matches_all_aria_attributes_strictly_and_ignores_active_and_non_url_props()
    -> Result<(), Box<dyn Error>> {
        let tree: Vec<AriaNode> = serde_json::from_value(json!([
            {
                "role": "checkbox",
                "name": "Agree",
                "checked": "mixed",
                "disabled": false,
                "expanded": false,
                "invalid": "grammar",
                "level": 2.0,
                "pressed": false,
                "selected": true,
                "url": "https://example.com/a?x=1"
            },
            {
                "role": "heading",
                "name": "Zero",
                "level": -0.0
            }
        ]))?;

        let mut props = BTreeMap::new();
        props.insert(
            "url".to_owned(),
            text_value(
                "/https:\\/\\/example\\.com\\/.*/",
                "/https:\\/\\/example\\.com\\/.*/",
            ),
        );
        props.insert(
            "placeholder".to_owned(),
            text_value("nonexistent", "nonexistent"),
        );

        let matching = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "checkbox".to_owned(),
            name: Some(TemplateName::Exact("Agree".to_owned())),
            checked: Some(Tristate::Mixed),
            disabled: Some(false),
            expanded: Some(false),
            active: Some(true),
            invalid: Some(Invalid::Grammar),
            level: Some(Number::from(2_u64)),
            pressed: Some(Tristate::False),
            selected: Some(true),
            props,
            ..AriaTemplateRoleNode::default()
        });
        assert_eq!(
            match_template(&tree, &matching)?,
            vec![TemplateMatch::Node(&tree[0])],
            "integer 2 matches float 2.0, active and non-url props are ignored, and /url regex matches"
        );

        let zero_level = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "heading".to_owned(),
            level: Some(Number::from(0_u64)),
            ..AriaTemplateRoleNode::default()
        });
        assert_eq!(
            match_template(&tree, &zero_level)?,
            vec![TemplateMatch::Node(&tree[1])],
            "0 equals -0.0 under JavaScript === semantics"
        );

        let explicit_false_absent = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "heading".to_owned(),
            disabled: Some(false),
            ..AriaTemplateRoleNode::default()
        });
        assert!(
            !matches_template(&tree, &explicit_false_absent)?,
            "explicit disabled=false requires Some(false), not None"
        );
        Ok(())
    }

    #[test]
    fn enforces_name_and_text_value_regex_semantics_and_error_handling()
    -> Result<(), Box<dyn Error>> {
        let unnamed = vec![aria_node("button")];
        let mut empty_named = aria_node("button");
        empty_named.name = Some(String::new());
        let mut named = aria_node("button");
        named.name = Some("Submit".to_owned());

        let bad_name_regex = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "button".to_owned(),
            name: Some(TemplateName::Regex(AriaRegex {
                pattern: "[".to_owned(),
            })),
            ..AriaTemplateRoleNode::default()
        });
        assert!(
            !matches_template(&unnamed, &bad_name_regex)?,
            "absent name returns Ok(false) before compiling an invalid name regex"
        );
        assert!(
            !matches_template(&[empty_named], &bad_name_regex)?,
            "empty name returns Ok(false) before compiling an invalid name regex"
        );
        let Err(error) = matches_template(&[named.clone()], &bad_name_regex) else {
            return Err("invalid name regex on a non-empty name must error".into());
        };
        assert!(
            matches!(error, TemplateError::InvalidRegex(_)),
            "error variant must report InvalidRegex: {error}"
        );
        assert!(
            error.source().is_some(),
            "TemplateError must expose the underlying RegexError via Error::source"
        );

        let empty_exact_name = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "button".to_owned(),
            name: Some(TemplateName::Exact(String::new())),
            ..AriaTemplateRoleNode::default()
        });
        assert!(
            matches_template(&unnamed, &empty_exact_name)?,
            "Exact(\"\") matches unnamed nodes just like Playwright's `!template`"
        );

        let good_regex = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "button".to_owned(),
            name: Some(TemplateName::Regex(AriaRegex {
                pattern: "^Sub".to_owned(),
            })),
            ..AriaTemplateRoleNode::default()
        });
        assert!(
            matches_template(&[named], &good_regex)?,
            "valid name regex matches non-empty accessible name"
        );

        let text_tree: Vec<AriaNode> = serde_json::from_value(json!([
            { "role": "text", "text": "hello world" }
        ]))?;
        let bad_text_regex = text_template("/[/", "/[/");
        assert!(
            !matches_template(&text_tree, &bad_text_regex)?,
            "invalid /.../ text regex caches failure and returns Ok(false)"
        );
        let single_slash = text_template("/", "/");
        assert!(
            !matches_template(&text_tree, &single_slash)?,
            "a lone '/' has length 1 and is never treated as a regex"
        );
        let empty_regex_text = text_template("//", "//");
        assert!(
            matches_template(&text_tree, &empty_regex_text)?,
            "'//' has length 2 and compiles as an empty regex matching non-empty text"
        );
        let raw_literal = text_template("hello world", "different");
        assert!(
            matches_template(&text_tree, &raw_literal)?,
            "matches_text_value accepts an exact match against raw as well as normalized"
        );
        Ok(())
    }

    #[test]
    fn honors_container_mode_precedence() -> Result<(), Box<dyn Error>> {
        let nested: Vec<AriaNode> = serde_json::from_value(json!([
            {
                "role": "list",
                "children": [
                    {
                        "role": "listitem",
                        "children": [
                            { "role": "button", "name": "A" },
                            { "role": "button", "name": "B" }
                        ]
                    }
                ]
            }
        ]))?;

        let only_a = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "button".to_owned(),
            name: Some(TemplateName::Exact("A".to_owned())),
            ..AriaTemplateRoleNode::default()
        });

        let equal_list = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "list".to_owned(),
            container_mode: Some(ContainerMode::Equal),
            children: vec![AriaTemplateNode::Role(AriaTemplateRoleNode {
                role: "listitem".to_owned(),
                children: vec![only_a.clone()],
                ..AriaTemplateRoleNode::default()
            })],
            ..AriaTemplateRoleNode::default()
        });
        assert!(
            matches_template(&nested, &equal_list)?,
            "Equal checks exact children at list, while listitem reverts to Contain"
        );

        let deep_equal_list = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "list".to_owned(),
            container_mode: Some(ContainerMode::DeepEqual),
            children: vec![AriaTemplateNode::Role(AriaTemplateRoleNode {
                role: "listitem".to_owned(),
                children: vec![only_a.clone()],
                ..AriaTemplateRoleNode::default()
            })],
            ..AriaTemplateRoleNode::default()
        });
        assert!(
            !matches_template(&nested, &deep_equal_list)?,
            "DeepEqual propagates exact-length checking into listitem"
        );

        let overridden = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "list".to_owned(),
            container_mode: Some(ContainerMode::DeepEqual),
            children: vec![AriaTemplateNode::Role(AriaTemplateRoleNode {
                role: "listitem".to_owned(),
                container_mode: Some(ContainerMode::Contain),
                children: vec![only_a],
                ..AriaTemplateRoleNode::default()
            })],
            ..AriaTemplateRoleNode::default()
        });
        assert!(
            matches_template(&nested, &overridden)?,
            "explicit Contain on a child overrides ancestor DeepEqual"
        );
        Ok(())
    }

    #[test]
    fn matches_contain_greedily_without_backtracking() -> Result<(), Box<dyn Error>> {
        // First group matches `[button "A"]`, leaving only the second group `[button "A"]` for
        // `[button "B"]`, which must fail rather than backtracking.
        let greedy_tree: Vec<AriaNode> = serde_json::from_value(json!([
            {
                "role": "region",
                "children": [
                    {
                        "role": "group",
                        "children": [
                            { "role": "button", "name": "A" },
                            { "role": "button", "name": "B" }
                        ]
                    },
                    {
                        "role": "group",
                        "children": [
                            { "role": "button", "name": "A" }
                        ]
                    }
                ]
            }
        ]))?;
        let only_a = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "button".to_owned(),
            name: Some(TemplateName::Exact("A".to_owned())),
            ..AriaTemplateRoleNode::default()
        });
        let only_b = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "button".to_owned(),
            name: Some(TemplateName::Exact("B".to_owned())),
            ..AriaTemplateRoleNode::default()
        });
        let greedy_template = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "region".to_owned(),
            children: vec![
                AriaTemplateNode::Role(AriaTemplateRoleNode {
                    role: "group".to_owned(),
                    children: vec![only_a],
                    ..AriaTemplateRoleNode::default()
                }),
                AriaTemplateNode::Role(AriaTemplateRoleNode {
                    role: "group".to_owned(),
                    children: vec![only_b],
                    ..AriaTemplateRoleNode::default()
                }),
            ],
            ..AriaTemplateRoleNode::default()
        });
        assert!(
            !matches_template(&greedy_tree, &greedy_template)?,
            "containsList matches greedily in order without backtracking"
        );
        Ok(())
    }

    #[test]
    fn matches_100_000_level_deep_trees_and_templates_without_stack_overflow()
    -> Result<(), Box<dyn Error>> {
        let mut deep_tree = aria_node("button");
        deep_tree.name = Some("Leaf".to_owned());
        let mut deep_template = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "button".to_owned(),
            name: Some(TemplateName::Exact("Leaf".to_owned())),
            ..AriaTemplateRoleNode::default()
        });

        for _ in 1..100_000 {
            let mut parent = aria_node("generic");
            parent.children.push(AriaChild::Node(Box::new(deep_tree)));
            deep_tree = parent;

            deep_template = AriaTemplateNode::Role(AriaTemplateRoleNode {
                role: "generic".to_owned(),
                container_mode: Some(ContainerMode::Equal),
                children: vec![deep_template],
                ..AriaTemplateRoleNode::default()
            });
        }

        let tree = vec![deep_tree];
        let leaf_template = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: "button".to_owned(),
            name: Some(TemplateName::Exact("Leaf".to_owned())),
            ..AriaTemplateRoleNode::default()
        });
        assert!(
            matches_template(&tree, &leaf_template)?,
            "iterative matches_node_deep finds a leaf 100,000 levels down"
        );
        assert!(
            matches_template(&tree, &deep_template)?,
            "iterative matches_node verifies a 100,000-level deep template"
        );
        Ok(())
    }
}
