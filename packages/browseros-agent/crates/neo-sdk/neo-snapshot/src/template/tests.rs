//! Parity, differential and stack-safety tests for [`crate::template`].
//!
//! This suite verifies three independent guarantees through the public [`crate::template`] API:
//!
//! 1. **Playwright 1.64.1 spec parity** (`tests/page/to-match-aria-snapshot.spec.ts`, cases
//!    `AR-107`..=`AR-142`): every template JSON literal is taken verbatim from
//!    `parseTemplate(unshift(...))` in `neo-playwright-js/src/prelude/aria-template.ts`.
//! 2. **QuickJS differential oracle**: executes Playwright 1.64.1's `matchesNode`, `listEqual`,
//!    `containsList`, `matchesNodeDeep`, `matchesStringOrRegex`, `matchesTextValue` and
//!    `cachedRegex` (`injected/src/ariaSnapshot.ts:308-462`) in QuickJS across a diverse matrix of
//!    trees and templates, asserting exact agreement with Rust on match verdicts, error outcomes,
//!    match counts and matched node paths.
//! 3. **Deep-tree stack safety**: matches 100,000-level deep [`AriaNode`] trees and
//!    `deep-equal` [`AriaTemplateNode`] trees without overflowing the thread stack.
//!
//! ## Spec Case Coverage (`AR-107`..=`AR-142`)
//!
//! Restated in this module (33 of 36 cases): `AR-107`..=`AR-128`, `AR-131`..=`AR-132`, and
//! `AR-134`..=`AR-142`.
//!
//! Cases that cannot be restated at the [`AriaNode`] matcher boundary:
//! - `AR-129` (`should report error in YAML`): asserts YAML document parse errors (`Aria snapshot
//!   must be a YAML sequence...`, `Nested mappings are not allowed...`) produced by
//!   `prelude/aria-template.ts` before JSON ever reaches `neo-snapshot`.
//! - `AR-130` (`should report error in YAML keys`): asserts `KeyParser` syntax and attribute parse
//!   errors (`Unterminated string`, `Unterminated regex`, `[level=a]`, `[expanded=FALSE]`,
//!   `[checked=foo]`, `[level=]`, `[bogus]`, `heading invalid`) in `prelude/aria-template.ts`
//!   before JSON reaches `neo-snapshot`.
//! - `AR-133` (`should not unshift actual template text`): asserts Playwright's test-runner YAML
//!   `unshift` and mismatch diff formatting when template lines have uneven indentation, which
//!   lives outside `neo-snapshot`.
//!
//! Sub-assertions outside `neo-snapshot` within restated cases:
//! - `AR-110`, `AR-119`..=`AR-124`, `AR-127`, `AR-131`: retry/timeout polling, `KeyParser` human
//!   error messages and unified diff / call-log error envelopes live outside `neo-snapshot`. All
//!   match/mismatch verdicts, typed serde rejection of invalid attribute wire values
//!   (`AR-119`..=`AR-124`) and [`Call::Playwright`] YAML rendering of received trees (`AR-127`,
//!   `AR-131`) are tested here.

use std::error::Error;
use std::ptr;

use rquickjs::{CatchResultExt, Context, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::template::{
    AriaTemplateNode, AriaTemplateRoleNode, ContainerMode, TemplateError, TemplateMatch,
    TemplateName, match_template, matches_template,
};
use crate::{AriaChild, AriaNode, Call, Snapshot, render};

fn parse_tree(value: &Value) -> Result<Vec<AriaNode>, Box<dyn Error>> {
    Ok(serde_json::from_value(value.clone())?)
}

fn parse_template(value: &Value) -> Result<AriaTemplateNode, Box<dyn Error>> {
    Ok(serde_json::from_value(value.clone())?)
}

fn render_playwright(tree: Vec<AriaNode>) -> String {
    let snapshot = Snapshot {
        tree,
        ..Snapshot::default()
    };
    render(&snapshot, Call::Playwright).text().to_owned()
}

fn bare_aria_node(role: &str) -> AriaNode {
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

/// Resolves a [`TemplateMatch`] to its `"root"` or dot-delimited index path (e.g. `"0.1.2"`) by
/// pointer identity, iteratively so arbitrary depths stay stack-safe.
fn match_path(tree: &[AriaNode], found: TemplateMatch<'_>) -> Result<String, Box<dyn Error>> {
    let target = match found {
        TemplateMatch::Root => return Ok("root".to_owned()),
        TemplateMatch::Node(node) => node,
    };
    let mut stack: Vec<(&AriaNode, String)> = tree
        .iter()
        .enumerate()
        .rev()
        .map(|(index, node)| (node, index.to_string()))
        .collect();
    while let Some((node, path)) = stack.pop() {
        if ptr::eq(node, target) {
            return Ok(path);
        }
        for (index, child) in node.children.iter().enumerate().rev() {
            if let AriaChild::Node(child_node) = child {
                stack.push((child_node, format!("{path}.{index}")));
            }
        }
    }
    Err("matched node pointer was not found in the input tree".into())
}

// ---------------------------------------------------------------------------
// Playwright 1.64.1 `to-match-aria-snapshot.spec.ts` (AR-107 ..= AR-142)
// ---------------------------------------------------------------------------

#[test]
fn ar_107_should_match_page() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        { "role": "heading", "name": "title", "level": 1 }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role", "role": "heading", "name": "title"
    }))?;
    assert!(
        matches_template(&tree, &template)?,
        "AR-107: page with <h1>title</h1> must match `- heading \"title\"`"
    );
    assert_eq!(
        match_template(&tree, &template)?,
        vec![TemplateMatch::Node(&tree[0])],
        "AR-107: match_template must return the matched heading node"
    );
    Ok(())
}

#[test]
fn ar_108_should_match_page_complex() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        { "role": "heading", "name": "Microsoft", "level": 1 },
        { "role": "text", "text": "Open source projects and samples from Microsoft" },
        {
            "role": "list",
            "children": [
                {
                    "role": "listitem",
                    "children": [
                        { "role": "link", "name": "Playwright", "url": "about:blank" }
                    ]
                }
            ]
        }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role",
        "role": "fragment",
        "children": [
            { "kind": "role", "role": "heading", "name": "Microsoft" },
            {
                "kind": "text",
                "text": {
                    "raw": "Open source projects and samples from Microsoft",
                    "normalized": "Open source projects and samples from Microsoft"
                }
            },
            {
                "kind": "role",
                "role": "list",
                "name": "",
                "children": [
                    {
                        "kind": "role",
                        "role": "listitem",
                        "name": "",
                        "children": [
                            { "kind": "role", "role": "link", "name": "Playwright" }
                        ]
                    }
                ]
            }
        ]
    }))?;
    assert!(
        matches_template(&tree, &template)?,
        "AR-108: complex page with heading, top-level text and nested list must match"
    );
    assert_eq!(
        match_template(&tree, &template)?,
        vec![TemplateMatch::Root],
        "AR-108: multi-root fragment template matches the synthetic root container"
    );
    Ok(())
}

#[test]
fn ar_109_should_match_page_with_not() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        { "role": "heading", "name": "title", "level": 1 }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role", "role": "heading", "name": "wrong"
    }))?;
    assert!(
        !matches_template(&tree, &template)?,
        "AR-109: heading \"title\" must not match `- heading \"wrong\"`"
    );
    assert!(
        match_template(&tree, &template)?.is_empty(),
        "AR-109: non-matching template must yield zero matched nodes"
    );
    Ok(())
}

#[test]
fn ar_110_should_fail_page_with_timeout() -> Result<(), Box<dyn Error>> {
    // Timeout polling and `expect(page)...` error formatting live outside neo-snapshot;
    // the matcher's role is returning the mismatch verdict.
    let tree = parse_tree(&json!([
        { "role": "heading", "name": "title", "level": 1 }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role", "role": "heading", "name": "wrong"
    }))?;
    assert!(
        !matches_template(&tree, &template)?,
        "AR-110: mismatched heading must report false so the outer assertion times out"
    );
    Ok(())
}

#[test]
fn ar_111_should_match() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        { "role": "heading", "name": "title", "level": 1 }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role", "role": "heading", "name": "title"
    }))?;
    assert!(
        matches_template(&tree, &template)?,
        "AR-111: single heading must match `- heading \"title\"`"
    );
    Ok(())
}

#[test]
fn ar_112_should_match_in_list() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        { "role": "heading", "name": "title", "level": 1 },
        { "role": "heading", "name": "title 2", "level": 1 }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role", "role": "heading", "name": "title"
    }))?;
    assert!(
        matches_template(&tree, &template)?,
        "AR-112: first heading in sibling list must match `- heading \"title\"`"
    );
    assert_eq!(
        match_template(&tree, &template)?,
        vec![TemplateMatch::Node(&tree[0])],
        "AR-112: only the matching sibling heading is returned"
    );
    Ok(())
}

#[test]
fn ar_113_should_match_list_with_accessible_name() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        {
            "role": "list",
            "name": "my list",
            "children": [
                { "role": "listitem", "text": "one" },
                { "role": "listitem", "text": "two" }
            ]
        }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role",
        "role": "list",
        "name": "my list",
        "children": [
            {
                "kind": "role",
                "role": "listitem",
                "name": "",
                "children": [{ "kind": "text", "text": { "raw": "one", "normalized": "one" } }]
            },
            {
                "kind": "role",
                "role": "listitem",
                "name": "",
                "children": [{ "kind": "text", "text": { "raw": "two", "normalized": "two" } }]
            }
        ]
    }))?;
    assert!(
        matches_template(&tree, &template)?,
        "AR-113: list with accessible name \"my list\" and items \"one\"/\"two\" must match"
    );

    let wrong_name = parse_template(&json!({
        "kind": "role",
        "role": "list",
        "name": "other list",
        "children": []
    }))?;
    assert!(
        !matches_template(&tree, &wrong_name)?,
        "AR-113: list accessible name mismatch must fail"
    );
    Ok(())
}

#[test]
fn ar_114_should_match_deep_item() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        {
            "role": "generic",
            "children": [
                { "role": "heading", "name": "title", "level": 1 },
                { "role": "heading", "name": "title 2", "level": 1 }
            ]
        }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role", "role": "heading", "name": "title"
    }))?;
    assert!(
        matches_template(&tree, &template)?,
        "AR-114: deep search must find nested heading \"title\" inside a container"
    );
    let matches = match_template(&tree, &template)?;
    assert_eq!(
        matches.len(),
        1,
        "AR-114: exactly one nested heading matches"
    );
    assert_eq!(
        match_path(&tree, matches[0])?,
        "0.0",
        "AR-114: matched node must be the nested heading, not its wrapper"
    );
    Ok(())
}

#[test]
fn ar_115_should_match_complex() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        {
            "role": "list",
            "children": [
                {
                    "role": "listitem",
                    "children": [
                        { "role": "link", "name": "link", "url": "about:blank" }
                    ]
                }
            ]
        }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role",
        "role": "list",
        "name": "",
        "children": [
            {
                "kind": "role",
                "role": "listitem",
                "name": "",
                "children": [
                    { "kind": "role", "role": "link", "name": "link" }
                ]
            }
        ]
    }))?;
    assert!(
        matches_template(&tree, &template)?,
        "AR-115: nested list -> listitem -> link \"link\" must match"
    );
    Ok(())
}

#[test]
fn ar_116_should_match_regex() -> Result<(), Box<dyn Error>> {
    let subcases = [
        (
            "Issues 12",
            "Issues abc",
            json!({ "kind": "role", "role": "heading", "name": { "pattern": r"Issues \d+" } }),
            "Issues \\d+",
        ),
        (
            "Issues 1/2",
            "Issues 1-2",
            json!({ "kind": "role", "role": "heading", "name": { "pattern": "Issues 1[/]2" } }),
            "Issues 1[/]2",
        ),
        (
            "Issues 1[",
            "Issues 1]",
            json!({ "kind": "role", "role": "heading", "name": { "pattern": r"Issues 1\[" } }),
            "Issues 1\\[",
        ),
        (
            "Issues 1]]2",
            "Issues 1]2",
            json!({ "kind": "role", "role": "heading", "name": { "pattern": r"Issues 1[\]]]2" } }),
            "Issues 1[\\]]]2",
        ),
    ];
    for (matching_name, non_matching_name, template_json, label) in subcases {
        let template = parse_template(&template_json)?;
        let matching_tree = parse_tree(&json!([
            { "role": "heading", "name": matching_name, "level": 1 }
        ]))?;
        let non_matching_tree = parse_tree(&json!([
            { "role": "heading", "name": non_matching_name, "level": 1 }
        ]))?;
        assert!(
            matches_template(&matching_tree, &template)?,
            "AR-116: regex /{label}/ must match heading {matching_name:?}"
        );
        assert!(
            !matches_template(&non_matching_tree, &template)?,
            "AR-116: regex /{label}/ must not match heading {non_matching_name:?}"
        );
    }
    Ok(())
}

#[test]
fn ar_117_should_allow_text_nodes() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        { "role": "heading", "name": "Microsoft", "level": 1 },
        { "role": "text", "text": "Open source projects and samples from Microsoft" }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role",
        "role": "fragment",
        "children": [
            { "kind": "role", "role": "heading", "name": "Microsoft" },
            {
                "kind": "text",
                "text": {
                    "raw": "Open source projects and samples from Microsoft",
                    "normalized": "Open source projects and samples from Microsoft"
                }
            }
        ]
    }))?;
    assert!(
        matches_template(&tree, &template)?,
        "AR-117: fragment containing heading and top-level text node must match"
    );
    Ok(())
}

#[test]
fn ar_118_details_visibility() -> Result<(), Box<dyn Error>> {
    // Closed <details><summary>Summary</summary><div>Details</div></details> in default mode
    // exposes only the visible summary text inside `group`.
    let tree = parse_tree(&json!([
        { "role": "group", "text": "Summary" }
    ]))?;
    let summary = parse_template(&json!({
        "kind": "role",
        "role": "group",
        "name": "",
        "children": [
            { "kind": "text", "text": { "raw": "Summary", "normalized": "Summary" } }
        ]
    }))?;
    let hidden_details = parse_template(&json!({
        "kind": "role",
        "role": "group",
        "name": "",
        "children": [
            { "kind": "text", "text": { "raw": "Details", "normalized": "Details" } }
        ]
    }))?;
    assert!(
        matches_template(&tree, &summary)?,
        "AR-118: closed <details> group must match `- group: \"Summary\"`"
    );
    assert!(
        !matches_template(&tree, &hidden_details)?,
        "AR-118: hidden details content must not match a closed <details> group"
    );
    Ok(())
}

#[test]
fn ar_119_checked_attribute() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        { "role": "checkbox", "checked": true }
    ]))?;
    for (template_json, expected, label) in [
        (
            json!({ "kind": "role", "role": "checkbox", "name": "" }),
            true,
            "- checkbox",
        ),
        (
            json!({ "kind": "role", "role": "checkbox", "name": "", "checked": true }),
            true,
            "- checkbox [checked] / [checked=true]",
        ),
        (
            json!({ "kind": "role", "role": "checkbox", "name": "", "checked": false }),
            false,
            "- checkbox [checked=false]",
        ),
        (
            json!({ "kind": "role", "role": "checkbox", "name": "", "checked": "mixed" }),
            false,
            "- checkbox [checked=mixed]",
        ),
    ] {
        let template = parse_template(&template_json)?;
        assert_eq!(
            matches_template(&tree, &template)?,
            expected,
            "AR-119: checked checkbox against `{label}` must return {expected}"
        );
    }
    let invalid_wire = json!({ "kind": "role", "role": "checkbox", "name": "", "checked": 5 });
    assert!(
        serde_json::from_value::<AriaTemplateNode>(invalid_wire).is_err(),
        "AR-119: invalid wire attribute `checked: 5` must fail AriaTemplateNode deserialization"
    );
    Ok(())
}

#[test]
fn ar_120_disabled_attribute() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        { "role": "button", "name": "Click me", "disabled": true }
    ]))?;
    for (template_json, expected, label) in [
        (
            json!({ "kind": "role", "role": "button", "name": "" }),
            true,
            "- button",
        ),
        (
            json!({ "kind": "role", "role": "button", "name": "", "disabled": true }),
            true,
            "- button [disabled] / [disabled=true]",
        ),
        (
            json!({ "kind": "role", "role": "button", "name": "", "disabled": false }),
            false,
            "- button [disabled=false]",
        ),
    ] {
        let template = parse_template(&template_json)?;
        assert_eq!(
            matches_template(&tree, &template)?,
            expected,
            "AR-120: disabled button against `{label}` must return {expected}"
        );
    }
    let invalid_wire =
        json!({ "kind": "role", "role": "button", "name": "", "disabled": "invalid" });
    assert!(
        serde_json::from_value::<AriaTemplateNode>(invalid_wire).is_err(),
        "AR-120: invalid wire attribute `disabled: \"invalid\"` must fail deserialization"
    );
    Ok(())
}

#[test]
fn ar_121_expanded_attribute() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        { "role": "button", "name": "Toggle", "expanded": true }
    ]))?;
    for (template_json, expected, label) in [
        (
            json!({ "kind": "role", "role": "button", "name": "" }),
            true,
            "- button",
        ),
        (
            json!({ "kind": "role", "role": "button", "name": "", "expanded": true }),
            true,
            "- button [expanded] / [expanded=true]",
        ),
        (
            json!({ "kind": "role", "role": "button", "name": "", "expanded": false }),
            false,
            "- button [expanded=false]",
        ),
    ] {
        let template = parse_template(&template_json)?;
        assert_eq!(
            matches_template(&tree, &template)?,
            expected,
            "AR-121: expanded button against `{label}` must return {expected}"
        );
    }
    let invalid_wire =
        json!({ "kind": "role", "role": "button", "name": "", "expanded": "invalid" });
    assert!(
        serde_json::from_value::<AriaTemplateNode>(invalid_wire).is_err(),
        "AR-121: invalid wire attribute `expanded: \"invalid\"` must fail deserialization"
    );
    Ok(())
}

#[test]
fn ar_122_level_attribute() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        { "role": "heading", "name": "Section Title", "level": 2 }
    ]))?;
    for (template_json, expected, label) in [
        (
            json!({ "kind": "role", "role": "heading", "name": "" }),
            true,
            "- heading",
        ),
        (
            json!({ "kind": "role", "role": "heading", "name": "", "level": 2 }),
            true,
            "- heading [level=2]",
        ),
        (
            json!({ "kind": "role", "role": "heading", "name": "", "level": 3 }),
            false,
            "- heading [level=3]",
        ),
    ] {
        let template = parse_template(&template_json)?;
        assert_eq!(
            matches_template(&tree, &template)?,
            expected,
            "AR-122: level-2 heading against `{label}` must return {expected}"
        );
    }
    let invalid_wire = json!({ "kind": "role", "role": "heading", "name": "", "level": "two" });
    assert!(
        serde_json::from_value::<AriaTemplateNode>(invalid_wire).is_err(),
        "AR-122: invalid wire attribute `level: \"two\"` must fail deserialization"
    );
    Ok(())
}

#[test]
fn ar_123_pressed_attribute() -> Result<(), Box<dyn Error>> {
    let pressed_true = parse_tree(&json!([
        { "role": "button", "name": "Like", "pressed": true }
    ]))?;
    for (template_json, expected, label) in [
        (
            json!({ "kind": "role", "role": "button", "name": "" }),
            true,
            "- button",
        ),
        (
            json!({ "kind": "role", "role": "button", "name": "", "pressed": true }),
            true,
            "- button [pressed] / [pressed=true]",
        ),
        (
            json!({ "kind": "role", "role": "button", "name": "", "pressed": false }),
            false,
            "- button [pressed=false]",
        ),
    ] {
        let template = parse_template(&template_json)?;
        assert_eq!(
            matches_template(&pressed_true, &template)?,
            expected,
            "AR-123: pressed=true button against `{label}` must return {expected}"
        );
    }

    let pressed_mixed = parse_tree(&json!([
        { "role": "button", "name": "Like", "pressed": "mixed" }
    ]))?;
    let mixed_template = parse_template(&json!({
        "kind": "role", "role": "button", "name": "", "pressed": "mixed"
    }))?;
    let true_template = parse_template(&json!({
        "kind": "role", "role": "button", "name": "", "pressed": true
    }))?;
    assert!(
        matches_template(&pressed_mixed, &mixed_template)?,
        "AR-123: pressed=mixed button must match `- button [pressed=mixed]`"
    );
    assert!(
        !matches_template(&pressed_mixed, &true_template)?,
        "AR-123: pressed=mixed button must not match `- button [pressed=true]`"
    );

    let invalid_wire = json!({ "kind": "role", "role": "button", "name": "", "pressed": 5 });
    assert!(
        serde_json::from_value::<AriaTemplateNode>(invalid_wire).is_err(),
        "AR-123: invalid wire attribute `pressed: 5` must fail deserialization"
    );
    Ok(())
}

fn ar_124_table_tree() -> Result<Vec<AriaNode>, Box<dyn Error>> {
    // Playwright's `toAriaNode` sets explicit `selected: false` on unselected `row` elements.
    parse_tree(&json!([
        {
            "role": "table",
            "children": [
                {
                    "role": "rowgroup",
                    "children": [
                        {
                            "role": "row",
                            "name": "Row",
                            "selected": true,
                            "children": [{ "role": "cell", "name": "Row" }]
                        },
                        {
                            "role": "row",
                            "name": "Row 2",
                            "selected": false,
                            "children": [{ "role": "cell", "name": "Row 2" }]
                        }
                    ]
                }
            ]
        }
    ]))
}

#[test]
fn ar_124_selected_attribute() -> Result<(), Box<dyn Error>> {
    let tree = ar_124_table_tree()?;

    let row_pair_cases = [
        (
            json!({
                "kind": "role", "role": "fragment",
                "children": [
                    { "kind": "role", "role": "row", "name": "" },
                    { "kind": "role", "role": "row", "name": "" }
                ]
            }),
            Some("0.0"),
            "fragment [row, row]",
        ),
        (
            json!({
                "kind": "role", "role": "fragment",
                "children": [
                    { "kind": "role", "role": "row", "name": "", "selected": true },
                    { "kind": "role", "role": "row", "name": "" }
                ]
            }),
            Some("0.0"),
            "fragment [row [selected], row]",
        ),
        (
            json!({
                "kind": "role", "role": "fragment",
                "children": [
                    { "kind": "role", "role": "row", "name": "", "selected": true },
                    { "kind": "role", "role": "row", "name": "", "selected": false }
                ]
            }),
            Some("0.0"),
            "fragment [row [selected=true], row [selected=false]]",
        ),
        (
            json!({
                "kind": "role", "role": "table", "name": "",
                "children": [
                    {
                        "kind": "role", "role": "rowgroup", "name": "",
                        "children": [
                            { "kind": "role", "role": "row", "name": "", "selected": true },
                            { "kind": "role", "role": "row", "name": "", "selected": false }
                        ]
                    }
                ]
            }),
            Some("0"),
            "table -> rowgroup -> [row [selected=true], row [selected=false]]",
        ),
        (
            json!({
                "kind": "role", "role": "fragment",
                "children": [
                    { "kind": "role", "role": "row", "name": "", "selected": false },
                    { "kind": "role", "role": "row", "name": "", "selected": false }
                ]
            }),
            None,
            "fragment [row [selected=false], row [selected=false]]",
        ),
    ];

    for (template_json, expected_path, label) in row_pair_cases {
        let template = parse_template(&template_json)?;
        let matches = match_template(&tree, &template)?;
        let actual_path = matches
            .first()
            .map(|found| match_path(&tree, *found))
            .transpose()?;
        assert_eq!(
            actual_path.as_deref(),
            expected_path,
            "AR-124: `{label}` must resolve to expected match path {expected_path:?}"
        );
    }

    let invalid_wire = json!({ "kind": "role", "role": "row", "name": "", "selected": "invalid" });
    assert!(
        serde_json::from_value::<AriaTemplateNode>(invalid_wire).is_err(),
        "AR-124: invalid wire attribute `selected: \"invalid\"` must fail deserialization"
    );
    Ok(())
}

#[test]
fn ar_125_integration_test() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        { "role": "heading", "name": "Microsoft", "level": 1 },
        { "role": "text", "text": "Open source projects and samples from Microsoft" },
        {
            "role": "list",
            "children": [
                {
                    "role": "listitem",
                    "children": [
                        { "role": "group", "text": "Verified" }
                    ]
                },
                {
                    "role": "listitem",
                    "children": [
                        { "role": "link", "name": "Sponsor", "url": "about:blank" }
                    ]
                }
            ]
        }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role",
        "role": "fragment",
        "children": [
            { "kind": "role", "role": "heading", "name": "Microsoft" },
            {
                "kind": "text",
                "text": {
                    "raw": "Open source projects and samples from Microsoft",
                    "normalized": "Open source projects and samples from Microsoft"
                }
            },
            {
                "kind": "role",
                "role": "list",
                "name": "",
                "children": [
                    {
                        "kind": "role",
                        "role": "listitem",
                        "name": "",
                        "children": [
                            {
                                "kind": "role",
                                "role": "group",
                                "name": "",
                                "children": [
                                    { "kind": "text", "text": { "raw": "Verified", "normalized": "Verified" } }
                                ]
                            }
                        ]
                    },
                    {
                        "kind": "role",
                        "role": "listitem",
                        "name": "",
                        "children": [
                            { "kind": "role", "role": "link", "name": "Sponsor" }
                        ]
                    }
                ]
            }
        ]
    }))?;
    assert!(
        matches_template(&tree, &template)?,
        "AR-125: integration snapshot with closed <details> and sponsor link must match"
    );
    Ok(())
}

#[test]
fn ar_126_integration_test_2() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        {
            "role": "banner",
            "children": [
                { "role": "heading", "name": "todos", "level": 1 },
                { "role": "textbox", "name": "What needs to be done?" }
            ]
        }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role",
        "role": "fragment",
        "children": [
            { "kind": "role", "role": "heading", "name": "todos" },
            { "kind": "role", "role": "textbox", "name": "What needs to be done?" }
        ]
    }))?;
    assert!(
        matches_template(&tree, &template)?,
        "AR-126: fragment template must match banner's children"
    );
    assert_eq!(
        match_template(&tree, &template)?,
        vec![TemplateMatch::Node(&tree[0])],
        "AR-126: fragment matches the enclosing banner element"
    );
    Ok(())
}

#[test]
fn ar_127_expected_formatter() -> Result<(), Box<dyn Error>> {
    // Diff envelope formatting lives outside neo-snapshot; verify the mismatch verdict and that
    // `render(..., Call::Playwright)` prints the exact received YAML lines from the spec.
    let tree = parse_tree(&json!([
        {
            "role": "banner",
            "children": [
                { "role": "heading", "name": "todos", "level": 1 },
                { "role": "textbox", "name": "What needs to be done?" },
                { "role": "button", "name": "Time 15:30" }
            ]
        }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role",
        "role": "fragment",
        "children": [
            { "kind": "role", "role": "heading", "name": "todos" },
            { "kind": "role", "role": "textbox", "name": "Wrong text" }
        ]
    }))?;
    assert!(
        !matches_template(&tree, &template)?,
        "AR-127: textbox \"What needs to be done?\" must not match \"Wrong text\""
    );
    let expected_yaml = "\
- banner:
  - heading \"todos\" [level=1]
  - textbox \"What needs to be done?\"
  - button \"Time 15:30\"";
    assert_eq!(
        render_playwright(tree),
        expected_yaml,
        "AR-127: Call::Playwright must print the exact received snapshot lines without regex conversion"
    );
    Ok(())
}

#[test]
fn ar_128_should_unpack_escaped_names() -> Result<(), Box<dyn Error>> {
    let subcases: Vec<(Value, Vec<Value>, &str)> = vec![
        (
            json!([{ "role": "button", "name": "Click: me" }]),
            vec![
                json!({ "kind": "role", "role": "button", "name": "Click: me" }),
                json!({ "kind": "role", "role": "button", "name": { "pattern": "Click: me" } }),
            ],
            "subcase 1: colon in button name",
        ),
        (
            json!([{ "role": "button", "name": "Click / me" }]),
            vec![
                json!({ "kind": "role", "role": "button", "name": "Click / me" }),
                json!({ "kind": "role", "role": "button", "name": { "pattern": r"Click \/ me" } }),
                json!({ "kind": "role", "role": "button", "name": { "pattern": r"Click \/ me" } }),
            ],
            "subcase 2: slash in button name",
        ),
        (
            json!([{ "role": "button", "name": "Click \" me" }]),
            vec![
                json!({ "kind": "role", "role": "button", "name": "Click \" me" }),
                json!({ "kind": "role", "role": "button", "name": { "pattern": "Click \" me" } }),
                json!({ "kind": "role", "role": "button", "name": { "pattern": r#"Click \" me"# } }),
            ],
            "subcase 3: double quote in button name",
        ),
        (
            json!([{ "role": "button", "name": r"Click \ me" }]),
            vec![
                json!({ "kind": "role", "role": "button", "name": r"Click \ me" }),
                json!({ "kind": "role", "role": "button", "name": { "pattern": r"Click \\ me" } }),
                json!({ "kind": "role", "role": "button", "name": { "pattern": r"Click \\ me" } }),
            ],
            "subcase 4: backslash in button name",
        ),
        (
            json!([{ "role": "button", "name": "Click ' me" }]),
            vec![json!({ "kind": "role", "role": "button", "name": "Click ' me" })],
            "subcase 5: single quote in button name",
        ),
        (
            json!([{ "role": "heading", "name": "heading \"name\" [level=1]", "level": 1 }]),
            vec![json!({
                "kind": "role", "role": "heading",
                "name": "heading \"name\" [level=1]", "level": 1
            })],
            "subcase 6: quoted name containing quotes and [level=1]",
        ),
        (
            json!([{ "role": "heading", "name": r#"heading \" [level=2]"#, "level": 1 }]),
            vec![json!({
                "kind": "role", "role": "heading",
                "name": r#"heading \" [level=2]"#, "level": 1
            })],
            "subcase 7: literal block scalar with escaped backslash+quote and [level=2]",
        ),
    ];

    let mut total_assertions = 0_usize;
    for (tree_json, templates, label) in &subcases {
        let tree = parse_tree(tree_json)?;
        for (index, template_json) in templates.iter().enumerate() {
            let template = parse_template(template_json)?;
            assert!(
                matches_template(&tree, &template)?,
                "AR-128 ({label}, assertion {index}): unpacked template must match element"
            );
            total_assertions += 1;
        }
    }
    assert_eq!(
        total_assertions, 14,
        "AR-128: all 14 exact and regex assertions across the 7 subcases must run"
    );

    let level_mismatch_tree = parse_tree(&json!([
        { "role": "heading", "name": r#"heading \" [level=2]"#, "level": 2 }
    ]))?;
    let level_one_template = parse_template(&json!({
        "kind": "role", "role": "heading",
        "name": r#"heading \" [level=2]"#, "level": 1
    }))?;
    assert!(
        !matches_template(&level_mismatch_tree, &level_one_template)?,
        "AR-128 (subcase 7): parsed attribute level=1 must still reject a level-2 heading"
    );
    Ok(())
}

#[test]
fn ar_131_call_log_should_contain_actual_snapshot() -> Result<(), Box<dyn Error>> {
    // Call-log envelope formatting lives outside neo-snapshot; verify the mismatch verdict and
    // Playwright YAML output for the received snapshot.
    let tree = parse_tree(&json!([
        { "role": "heading", "name": "todos", "level": 1 }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role", "role": "heading", "name": "wrong"
    }))?;
    assert!(
        !matches_template(&tree, &template)?,
        "AR-131: heading \"todos\" must not match `- heading \"wrong\"`"
    );
    assert_eq!(
        render_playwright(tree),
        "- heading \"todos\" [level=1]",
        "AR-131: Call::Playwright must render `- heading \"todos\" [level=1]`"
    );
    Ok(())
}

#[test]
fn ar_132_should_parse_attributes() -> Result<(), Box<dyn Error>> {
    let button_tree = parse_tree(&json!([
        { "role": "button", "name": "hello world", "pressed": "mixed" }
    ]))?;
    let pressed_mixed = parse_template(&json!({
        "kind": "role", "role": "button", "name": "", "pressed": "mixed"
    }))?;
    assert!(
        matches_template(&button_tree, &pressed_mixed)?,
        "AR-132: `[pressed=mixed ]` must match button with pressed=mixed"
    );

    let h2_tree = parse_tree(&json!([
        { "role": "heading", "name": "hello world", "level": 2 }
    ]))?;
    let negative_level = parse_template(&json!({
        "kind": "role", "role": "heading", "name": "", "level": -3
    }))?;
    assert!(
        !matches_template(&h2_tree, &negative_level)?,
        "AR-132: `[level = -3]` must not match an h2 heading (level=2)"
    );

    let neg_three_tree = parse_tree(&json!([
        { "role": "heading", "name": "hello world", "level": -3 }
    ]))?;
    assert!(
        matches_template(&neg_three_tree, &negative_level)?,
        "AR-132: `[level = -3]` must match a heading whose level is -3"
    );
    Ok(())
}

#[test]
fn ar_134_should_not_match_what_is_not_matched() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        { "role": "paragraph", "text": "Text" }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role",
        "role": "paragraph",
        "name": "",
        "children": [
            { "kind": "role", "role": "button", "name": "bogus" }
        ]
    }))?;
    assert!(
        !matches_template(&tree, &template)?,
        "AR-134: paragraph with text child \"Text\" must not match paragraph expecting button \"bogus\""
    );
    Ok(())
}

#[test]
fn ar_135_should_match_url() -> Result<(), Box<dyn Error>> {
    let template = parse_template(&json!({
        "kind": "role",
        "role": "link",
        "name": "",
        "children": [],
        "props": {
            "url": { "raw": "/.*example.com/", "normalized": "/.*example.com/" }
        }
    }))?;
    let matching = parse_tree(&json!([
        { "role": "link", "name": "Link", "url": "https://example.com" }
    ]))?;
    let mismatched = parse_tree(&json!([
        { "role": "link", "name": "Link", "url": "https://other.org" }
    ]))?;
    let missing_url = parse_tree(&json!([
        { "role": "link", "name": "Link" }
    ]))?;

    assert!(
        matches_template(&matching, &template)?,
        "AR-135: `/.*example.com/` must match link url `https://example.com`"
    );
    assert!(
        !matches_template(&mismatched, &template)?,
        "AR-135: `/.*example.com/` must reject non-matching link url"
    );
    assert!(
        !matches_template(&missing_url, &template)?,
        "AR-135: `/.*example.com/` must reject link without url"
    );
    Ok(())
}

#[test]
fn ar_136_should_detect_unexpected_children_equal() -> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        {
            "role": "list",
            "children": [
                { "role": "listitem", "text": "One" },
                { "role": "listitem", "text": "Two" },
                { "role": "listitem", "text": "Three" }
            ]
        }
    ]))?;
    let contain_template = parse_template(&json!({
        "kind": "role",
        "role": "list",
        "name": "",
        "children": [
            {
                "kind": "role", "role": "listitem", "name": "",
                "children": [{ "kind": "text", "text": { "raw": "One", "normalized": "One" } }]
            },
            {
                "kind": "role", "role": "listitem", "name": "",
                "children": [{ "kind": "text", "text": { "raw": "Three", "normalized": "Three" } }]
            }
        ]
    }))?;
    let equal_template = parse_template(&json!({
        "kind": "role",
        "role": "list",
        "name": "",
        "containerMode": "equal",
        "children": [
            {
                "kind": "role", "role": "listitem", "name": "",
                "children": [{ "kind": "text", "text": { "raw": "One", "normalized": "One" } }]
            },
            {
                "kind": "role", "role": "listitem", "name": "",
                "children": [{ "kind": "text", "text": { "raw": "Three", "normalized": "Three" } }]
            }
        ]
    }))?;
    assert!(
        matches_template(&tree, &contain_template)?,
        "AR-136: default contain mode allows skipped sibling `Two`"
    );
    assert!(
        !matches_template(&tree, &equal_template)?,
        "AR-136: `/children: equal` must reject unexpected sibling `Two`"
    );
    Ok(())
}

fn nested_two_item_list_tree() -> Result<Vec<AriaNode>, Box<dyn Error>> {
    parse_tree(&json!([
        {
            "role": "list",
            "children": [
                {
                    "role": "listitem",
                    "children": [
                        {
                            "role": "list",
                            "children": [
                                { "role": "listitem", "text": "1.1" },
                                { "role": "listitem", "text": "1.2" }
                            ]
                        }
                    ]
                }
            ]
        }
    ]))
}

fn nested_single_item_template(
    outer_mode: Option<&str>,
    inner_mode: Option<&str>,
) -> Result<AriaTemplateNode, Box<dyn Error>> {
    let mut inner_list = json!({
        "kind": "role",
        "role": "list",
        "name": "",
        "children": [
            {
                "kind": "role",
                "role": "listitem",
                "name": "",
                "children": [
                    { "kind": "text", "text": { "raw": "1.1", "normalized": "1.1" } }
                ]
            }
        ]
    });
    if let (Some(mode), Some(map)) = (inner_mode, inner_list.as_object_mut()) {
        map.insert("containerMode".to_owned(), json!(mode));
    }
    let mut outer_list = json!({
        "kind": "role",
        "role": "list",
        "name": "",
        "children": [
            {
                "kind": "role",
                "role": "listitem",
                "name": "",
                "children": [inner_list]
            }
        ]
    });
    if let (Some(mode), Some(map)) = (outer_mode, outer_list.as_object_mut()) {
        map.insert("containerMode".to_owned(), json!(mode));
    }
    parse_template(&outer_list)
}

#[test]
fn ar_137_should_detect_unexpected_children_deep_equal() -> Result<(), Box<dyn Error>> {
    let tree = nested_two_item_list_tree()?;
    let contain = nested_single_item_template(None, None)?;
    let equal = nested_single_item_template(Some("equal"), None)?;
    let deep_equal = nested_single_item_template(Some("deep-equal"), None)?;

    assert!(
        matches_template(&tree, &contain)?,
        "AR-137: default contain mode allows extra nested listitem `1.2`"
    );
    assert!(
        matches_template(&tree, &equal)?,
        "AR-137: outer `equal` only checks direct children, allowing extra nested listitem `1.2`"
    );
    assert!(
        !matches_template(&tree, &deep_equal)?,
        "AR-137: `deep-equal` propagates to descendants and rejects extra nested listitem `1.2`"
    );
    Ok(())
}

#[test]
fn ar_138_should_allow_restoring_contain_mode_inside_deep_equal() -> Result<(), Box<dyn Error>> {
    let tree = nested_two_item_list_tree()?;
    let without_override = nested_single_item_template(Some("deep-equal"), None)?;
    let with_contain_override = nested_single_item_template(Some("deep-equal"), Some("contain"))?;

    assert!(
        !matches_template(&tree, &without_override)?,
        "AR-138: without an inner override, deep-equal rejects extra nested listitem `1.2`"
    );
    assert!(
        matches_template(&tree, &with_contain_override)?,
        "AR-138: inner `/children: contain` restores contain semantics under outer `deep-equal`"
    );
    Ok(())
}

#[test]
fn ar_139_top_level_deep_equal() -> Result<(), Box<dyn Error>> {
    let non_empty_list = nested_two_item_list_tree()?;
    let empty_list = parse_tree(&json!([{ "role": "list" }]))?;
    let template = parse_template(&json!({
        "kind": "role",
        "role": "fragment",
        "containerMode": "deep-equal",
        "children": [
            { "kind": "role", "role": "list", "name": "" }
        ]
    }))?;

    assert!(
        !matches_template(&non_empty_list, &template)?,
        "AR-139: top-level deep-equal with `- list` must reject a list that has children"
    );
    assert!(
        matches_template(&empty_list, &template)?,
        "AR-139: top-level deep-equal with `- list` must match an empty list"
    );
    Ok(())
}

#[test]
fn ar_140_generated_snapshot_includes_text_children_when_name_is_longer_than_text()
-> Result<(), Box<dyn Error>> {
    let tree = parse_tree(&json!([
        { "role": "progressbar", "name": "Alpha Beta", "text": "Alpha 7" }
    ]))?;
    let template = parse_template(&json!({
        "kind": "role",
        "role": "fragment",
        "containerMode": "deep-equal",
        "children": [
            {
                "kind": "role",
                "role": "progressbar",
                "name": "Alpha Beta",
                "children": [
                    { "kind": "text", "text": { "raw": "Alpha 7", "normalized": "Alpha 7" } }
                ]
            }
        ]
    }))?;
    assert!(
        matches_template(&tree, &template)?,
        "AR-140: progressbar with name longer than text must match its text child under deep-equal"
    );

    let without_text = parse_tree(&json!([
        { "role": "progressbar", "name": "Alpha Beta" }
    ]))?;
    assert!(
        !matches_template(&without_text, &template)?,
        "AR-140: progressbar missing its text child must fail the deep-equal template"
    );
    Ok(())
}

#[test]
fn ar_141_treat_bad_regex_as_a_string() -> Result<(), Box<dyn Error>> {
    let template = parse_template(&json!({
        "kind": "role",
        "role": "link",
        "name": "Log in",
        "children": [],
        "props": {
            "url": { "raw": "/[a/", "normalized": "/[a/" }
        }
    }))?;
    let foo_tree = parse_tree(&json!([
        { "role": "link", "name": "Log in", "url": "/foo" }
    ]))?;
    let literal_tree = parse_tree(&json!([
        { "role": "link", "name": "Log in", "url": "/[a/" }
    ]))?;

    assert!(
        !matches_template(&foo_tree, &template)?,
        "AR-141: invalid /url regex `/[a/` on `/foo` must return Ok(false) rather than erroring"
    );
    assert!(
        matches_template(&literal_tree, &template)?,
        "AR-141: invalid /url regex `/[a/` must match literal url `/[a/` as a string"
    );
    Ok(())
}

#[test]
fn ar_142_invalid_attribute() -> Result<(), Box<dyn Error>> {
    let email_and_name = parse_tree(&json!([
        { "role": "textbox", "name": "Email", "invalid": true, "text": "not-an-email" },
        { "role": "textbox", "name": "Name", "invalid": false, "text": "Alice" }
    ]))?;
    let shorthand = parse_template(&json!({
        "kind": "role", "role": "fragment",
        "children": [
            {
                "kind": "role", "role": "textbox", "name": "Email", "invalid": true,
                "children": [{ "kind": "text", "text": { "raw": "not-an-email", "normalized": "not-an-email" } }]
            },
            {
                "kind": "role", "role": "textbox", "name": "Name",
                "children": [{ "kind": "text", "text": { "raw": "Alice", "normalized": "Alice" } }]
            }
        ]
    }))?;
    let explicit_bools = parse_template(&json!({
        "kind": "role", "role": "fragment",
        "children": [
            {
                "kind": "role", "role": "textbox", "name": "Email", "invalid": true,
                "children": [{ "kind": "text", "text": { "raw": "not-an-email", "normalized": "not-an-email" } }]
            },
            {
                "kind": "role", "role": "textbox", "name": "Name", "invalid": false,
                "children": [{ "kind": "text", "text": { "raw": "Alice", "normalized": "Alice" } }]
            }
        ]
    }))?;
    assert!(
        matches_template(&email_and_name, &shorthand)?,
        "AR-142: `[invalid]` must match invalid=true while omitting invalid matches invalid=false"
    );
    assert!(
        matches_template(&email_and_name, &explicit_bools)?,
        "AR-142: explicit `[invalid=true]` and `[invalid=false]` must match their exact states"
    );

    let grammar_and_spelling = parse_tree(&json!([
        { "role": "textbox", "name": "Bio", "invalid": "grammar" },
        { "role": "textbox", "name": "Note", "invalid": "spelling" }
    ]))?;
    let grammar_spelling_template = parse_template(&json!({
        "kind": "role", "role": "fragment",
        "children": [
            { "kind": "role", "role": "textbox", "name": "Bio", "invalid": "grammar" },
            { "kind": "role", "role": "textbox", "name": "Note", "invalid": "spelling" }
        ]
    }))?;
    let bio_true = parse_template(&json!({
        "kind": "role", "role": "textbox", "name": "Bio", "invalid": true
    }))?;
    assert!(
        matches_template(&grammar_and_spelling, &grammar_spelling_template)?,
        "AR-142: `[invalid=grammar]` and `[invalid=spelling]` must match their respective states"
    );
    assert!(
        !matches_template(&grammar_and_spelling, &bio_true)?,
        "AR-142: `invalid=grammar` is distinct from `invalid=true`"
    );

    // Playwright's `toAriaNode` normalizes unknown non-false `aria-invalid` values (`"garbage"`)
    // to `true`.
    let garbage_normalized = parse_tree(&json!([
        { "role": "textbox", "name": "Zip", "invalid": true }
    ]))?;
    let zip_invalid = parse_template(&json!({
        "kind": "role", "role": "textbox", "name": "Zip", "invalid": true
    }))?;
    assert!(
        matches_template(&garbage_normalized, &zip_invalid)?,
        "AR-142: non-false `aria-invalid=\"garbage\"` normalized to `true` must match `[invalid]`"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// QuickJS Differential Oracle (Playwright `ariaSnapshot.ts:308-462`)
// ---------------------------------------------------------------------------

/// Direct JS port of Playwright 1.64.1 `injected/src/ariaSnapshot.ts:308-462` plus an adapter
/// turning `AriaNode[]` JSON into Playwright's internal `{ role: 'fragment', children }` shape
/// tagged with dot-delimited `__path` identities.
const PLAYWRIGHT_ORACLE_JS: &str = r"
const cachedRegexSymbol = Symbol('cachedRegex');

function matchesStringOrRegex(text, template) {
  if (!template)
    return true;
  if (!text)
    return false;
  if (typeof template === 'string')
    return text === template;
  return !!text.match(new RegExp(template.pattern));
}

function cachedRegex(template) {
  if (template[cachedRegexSymbol] !== undefined)
    return template[cachedRegexSymbol];
  const { raw } = template;
  const canBeRegex = raw.startsWith('/') && raw.endsWith('/') && raw.length > 1;
  let regex;
  try {
    regex = canBeRegex ? new RegExp(raw.slice(1, -1)) : null;
  } catch (e) {
    regex = null;
  }
  template[cachedRegexSymbol] = regex;
  return regex;
}

function matchesTextValue(text, template) {
  if (!template?.normalized)
    return true;
  if (!text)
    return false;
  if (text === template.normalized)
    return true;
  if (text === template.raw)
    return true;
  const regex = cachedRegex(template);
  if (regex)
    return !!text.match(regex);
  return false;
}

function listEqual(children, template, isDeepEqual) {
  if (template.length !== children.length)
    return false;
  for (let i = 0; i < template.length; ++i) {
    if (!matchesNode(children[i], template[i], isDeepEqual))
      return false;
  }
  return true;
}

function containsList(children, template) {
  if (template.length > children.length)
    return false;
  const cc = children.slice();
  const tt = template.slice();
  for (const t of tt) {
    let c = cc.shift();
    while (c) {
      if (matchesNode(c, t, false))
        break;
      c = cc.shift();
    }
    if (!c)
      return false;
  }
  return true;
}

function matchesNode(node, template, isDeepEqual) {
  if (typeof node === 'string' && template.kind === 'text')
    return matchesTextValue(node, template.text);

  if (node === null || typeof node !== 'object' || template.kind !== 'role')
    return false;

  if (template.role !== 'fragment' && template.role !== node.role)
    return false;
  if (template.checked !== undefined && template.checked !== node.checked)
    return false;
  if (template.disabled !== undefined && template.disabled !== node.disabled)
    return false;
  if (template.expanded !== undefined && template.expanded !== node.expanded)
    return false;
  if (template.invalid !== undefined && template.invalid !== node.invalid)
    return false;
  if (template.level !== undefined && template.level !== node.level)
    return false;
  if (template.pressed !== undefined && template.pressed !== node.pressed)
    return false;
  if (template.selected !== undefined && template.selected !== node.selected)
    return false;
  if (!matchesStringOrRegex(node.name, template.name))
    return false;
  if (!matchesTextValue(node.props.url, template.props?.url))
    return false;

  if (template.containerMode === 'contain')
    return containsList(node.children || [], template.children || []);
  if (template.containerMode === 'equal')
    return listEqual(node.children || [], template.children || [], false);
  if (template.containerMode === 'deep-equal' || isDeepEqual)
    return listEqual(node.children || [], template.children || [], true);
  return containsList(node.children || [], template.children || []);
}

function matchesNodeDeep(root, template, collectAll, isDeepEqual) {
  const results = [];
  const visit = (node, parent) => {
    if (matchesNode(node, template, isDeepEqual)) {
      const result = typeof node === 'string' ? parent : node;
      if (result)
        results.push(result);
      return !collectAll;
    }
    if (typeof node === 'string')
      return false;
    for (const child of node.children || []) {
      if (visit(child, node))
        return true;
    }
    return false;
  };
  visit(root, null);
  return results;
}

function fromJsonNode(node, path) {
  if (typeof node === 'string')
    return node;
  if (node.role === 'text')
    return node.text ?? '';
  const props = {};
  if (node.url !== undefined) props.url = node.url;
  if (node.placeholder !== undefined) props.placeholder = node.placeholder;
  const children = node.text !== undefined
    ? [node.text]
    : (node.children || []).map((child, idx) => fromJsonNode(child, path + '.' + idx));
  const out = { __path: path, role: node.role, name: node.name ?? '', props, children };
  if (node.checked !== undefined) out.checked = node.checked;
  if (node.disabled !== undefined) out.disabled = node.disabled;
  if (node.expanded !== undefined) out.expanded = node.expanded;
  if (node.active !== undefined) out.active = node.active;
  if (node.invalid !== undefined) out.invalid = node.invalid;
  if (node.level !== undefined) out.level = node.level;
  if (node.pressed !== undefined) out.pressed = node.pressed;
  if (node.selected !== undefined) out.selected = node.selected;
  return out;
}

function fromJsonTree(tree) {
  return {
    __path: 'root',
    role: 'fragment',
    name: '',
    props: {},
    children: tree.map((node, idx) => fromJsonNode(node, String(idx))),
  };
}

function runOne(entry) {
  const root = fromJsonTree(entry.tree);
  let firstError = false;
  let firstMatched = false;
  try {
    firstMatched = matchesNodeDeep(root, structuredClone(entry.template), false, false).length > 0;
  } catch (e) {
    firstError = true;
  }
  let allError = false;
  let allPaths = [];
  try {
    allPaths = matchesNodeDeep(root, structuredClone(entry.template), true, false).map((n) => n.__path);
  } catch (e) {
    allError = true;
  }
  return { firstError, firstMatched, allError, allPaths };
}

function structuredClone(value) {
  return JSON.parse(JSON.stringify(value));
}

JSON.stringify(JSON.parse(input).map(runOne));
";

#[derive(Serialize)]
struct OracleCase {
    label: &'static str,
    tree: Value,
    template: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OracleAnswer {
    first_error: bool,
    first_matched: bool,
    all_error: bool,
    all_paths: Vec<String>,
}

fn run_quickjs_oracle(cases: &[OracleCase]) -> Result<Vec<OracleAnswer>, Box<dyn Error>> {
    let input = serde_json::to_string(cases)?;
    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    let output = context.with(|ctx| -> Result<String, String> {
        ctx.globals()
            .set("input", input)
            .catch(&ctx)
            .map_err(|caught| caught.to_string())?;
        ctx.eval::<String, _>(PLAYWRIGHT_ORACLE_JS)
            .catch(&ctx)
            .map_err(|caught| caught.to_string())
    })?;
    Ok(serde_json::from_str(&output)?)
}

fn oracle_regex_and_text_cases() -> Vec<OracleCase> {
    vec![
        OracleCase {
            label: "regex name digits match",
            tree: json!([{ "role": "heading", "name": "Issues 42", "level": 1 }]),
            template: json!({ "kind": "role", "role": "heading", "name": { "pattern": r"Issues \d+" } }),
        },
        OracleCase {
            label: "regex name bracketed slash and double closing bracket",
            tree: json!([{ "role": "heading", "name": "Issues 1]]2" }]),
            template: json!({ "kind": "role", "role": "heading", "name": { "pattern": r"Issues 1[\]]]2" } }),
        },
        OracleCase {
            label: "regex name empty pattern on named node",
            tree: json!([{ "role": "button", "name": "Go" }]),
            template: json!({ "kind": "role", "role": "button", "name": { "pattern": "" } }),
        },
        OracleCase {
            label: "regex name empty pattern on unnamed node",
            tree: json!([{ "role": "button" }]),
            template: json!({ "kind": "role", "role": "button", "name": { "pattern": "" } }),
        },
        OracleCase {
            label: "bad regex name on unnamed node returns false before compile",
            tree: json!([{ "role": "button" }]),
            template: json!({ "kind": "role", "role": "button", "name": { "pattern": "(" } }),
        },
        OracleCase {
            label: "bad regex name on named node errors",
            tree: json!([{ "role": "button", "name": "Click" }]),
            template: json!({ "kind": "role", "role": "button", "name": { "pattern": "(" } }),
        },
        OracleCase {
            label: "text regex matches leaf",
            tree: json!([{ "role": "paragraph", "text": "Count: 123 items" }]),
            template: json!({ "kind": "text", "text": { "raw": r"/Count: \d+/", "normalized": r"/Count: \d+/" } }),
        },
        OracleCase {
            label: "text bad regex fails on non-matching text without error",
            tree: json!([{ "role": "paragraph", "text": "hello" }]),
            template: json!({ "kind": "text", "text": { "raw": "/[a/", "normalized": "/[a/" } }),
        },
        OracleCase {
            label: "text bad regex matches literal string",
            tree: json!([{ "role": "paragraph", "text": "/[a/" }]),
            template: json!({ "kind": "text", "text": { "raw": "/[a/", "normalized": "/[a/" } }),
        },
        OracleCase {
            label: "text raw differs from normalized and matches raw",
            tree: json!([{ "role": "text", "text": "a   b" }]),
            template: json!({ "kind": "text", "text": { "raw": "a   b", "normalized": "a b" } }),
        },
        OracleCase {
            label: "text single slash is never a regex",
            tree: json!([{ "role": "text", "text": "abc" }]),
            template: json!({ "kind": "text", "text": { "raw": "/", "normalized": "/" } }),
        },
        OracleCase {
            label: "text double slash compiles as empty regex matching non-empty text",
            tree: json!([{ "role": "text", "text": "abc" }]),
            template: json!({ "kind": "text", "text": { "raw": "//", "normalized": "//" } }),
        },
        OracleCase {
            label: "empty normalized text matches any text leaf",
            tree: json!([{ "role": "paragraph", "text": "anything" }]),
            template: json!({ "kind": "text", "text": { "raw": "   ", "normalized": "" } }),
        },
        OracleCase {
            label: "top-level role=text node matches text template and resolves to root",
            tree: json!([{ "role": "text", "text": "top" }, { "role": "button", "name": "ok" }]),
            template: json!({ "kind": "text", "text": { "raw": "top", "normalized": "top" } }),
        },
        OracleCase {
            label: "regex name escaped slash matches button name",
            tree: json!([{ "role": "button", "name": "Click / me" }]),
            template: json!({ "kind": "role", "role": "button", "name": { "pattern": r"Click \/ me" } }),
        },
        OracleCase {
            label: "regex name escaped backslash matches button name",
            tree: json!([{ "role": "button", "name": r"Click \ me" }]),
            template: json!({ "kind": "role", "role": "button", "name": { "pattern": r"Click \\ me" } }),
        },
        OracleCase {
            label: "regex name .* rejects empty button name",
            tree: json!([{ "role": "button", "name": "" }]),
            template: json!({ "kind": "role", "role": "button", "name": { "pattern": ".*" } }),
        },
        OracleCase {
            label: "top-level role=text is not matched by role template with role=text",
            tree: json!([{ "role": "text", "text": "top" }]),
            template: json!({ "kind": "role", "role": "text" }),
        },
    ]
}

fn oracle_utf16_cases() -> Vec<OracleCase> {
    // Unflagged JavaScript RegExp operates on UTF-16 code units (UCS-2), so a supplementary
    // emoji is two code units rather than one Unicode scalar value.
    vec![
        OracleCase {
            label: "unflagged regex name ^.$ rejects supplementary emoji (2 UTF-16 code units)",
            tree: json!([{ "role": "button", "name": "😀" }]),
            template: json!({ "kind": "role", "role": "button", "name": { "pattern": "^.$" } }),
        },
        OracleCase {
            label: "unflagged regex name ^..$ matches supplementary emoji (2 UTF-16 code units)",
            tree: json!([{ "role": "button", "name": "😀" }]),
            template: json!({ "kind": "role", "role": "button", "name": { "pattern": "^..$" } }),
        },
        OracleCase {
            label: "unflagged regex name literal emoji matches emoji",
            tree: json!([{ "role": "button", "name": "😀" }]),
            template: json!({ "kind": "role", "role": "button", "name": { "pattern": "^😀$" } }),
        },
        OracleCase {
            label: "unflagged regex name [😀] splits into surrogate units and matches shared high surrogate",
            tree: json!([{ "role": "button", "name": "😁" }]),
            template: json!({ "kind": "role", "role": "button", "name": { "pattern": "[😀]" } }),
        },
        OracleCase {
            label: "unflagged regex name ^[😀]$ rejects full emoji",
            tree: json!([{ "role": "button", "name": "😀" }]),
            template: json!({ "kind": "role", "role": "button", "name": { "pattern": "^[😀]$" } }),
        },
        OracleCase {
            label: "unflagged regex text ^.$ rejects emoji",
            tree: json!([{ "role": "paragraph", "text": "😀" }]),
            template: json!({ "kind": "text", "text": { "raw": "/^.$/", "normalized": "/^.$/" } }),
        },
        OracleCase {
            label: "unflagged regex text ^..$ matches emoji",
            tree: json!([{ "role": "paragraph", "text": "😀" }]),
            template: json!({ "kind": "text", "text": { "raw": "/^..$/", "normalized": "/^..$/" } }),
        },
        OracleCase {
            label: "unflagged regex /url ^.$ rejects emoji",
            tree: json!([{ "role": "link", "name": "e", "url": "😀" }]),
            template: json!({
                "kind": "role", "role": "link",
                "props": { "url": { "raw": "/^.$/", "normalized": "/^.$/" } }
            }),
        },
        OracleCase {
            label: "unflagged regex /url ^..$ matches emoji",
            tree: json!([{ "role": "link", "name": "e", "url": "😀" }]),
            template: json!({
                "kind": "role", "role": "link",
                "props": { "url": { "raw": "/^..$/", "normalized": "/^..$/" } }
            }),
        },
    ]
}

fn oracle_container_mode_cases() -> Vec<OracleCase> {
    let nested_lists = json!([
        {
            "role": "list",
            "children": [
                {
                    "role": "listitem",
                    "children": [
                        {
                            "role": "list",
                            "children": [
                                { "role": "listitem", "text": "1.1" },
                                { "role": "listitem", "text": "1.2" }
                            ]
                        }
                    ]
                }
            ]
        }
    ]);
    let nested_template = |outer_mode: &str, inner_mode: Option<&str>| {
        let mut inner = json!({
            "kind": "role", "role": "list",
            "children": [{
                "kind": "role", "role": "listitem",
                "children": [{ "kind": "text", "text": { "raw": "1.1", "normalized": "1.1" } }]
            }]
        });
        if let (Some(mode), Some(obj)) = (inner_mode, inner.as_object_mut()) {
            obj.insert("containerMode".to_owned(), json!(mode));
        }
        json!({
            "kind": "role", "role": "list", "containerMode": outer_mode,
            "children": [{ "kind": "role", "role": "listitem", "children": [inner] }]
        })
    };
    vec![
        OracleCase {
            label: "explicit contain mode matches nested subsequence",
            tree: nested_lists.clone(),
            template: nested_template("contain", None),
        },
        OracleCase {
            label: "equal mode checks direct children only",
            tree: nested_lists.clone(),
            template: nested_template("equal", None),
        },
        OracleCase {
            label: "deep-equal rejects extra grandchild",
            tree: nested_lists.clone(),
            template: nested_template("deep-equal", None),
        },
        OracleCase {
            label: "deep-equal with inner contain override succeeds",
            tree: nested_lists.clone(),
            template: nested_template("deep-equal", Some("contain")),
        },
        OracleCase {
            label: "deep-equal with inner equal override rejects extra child at inner list",
            tree: nested_lists.clone(),
            template: nested_template("deep-equal", Some("equal")),
        },
        OracleCase {
            label: "deep-equal -> equal resets deep-equal for grandchildren",
            tree: nested_lists,
            template: json!({
                "kind": "role", "role": "list", "containerMode": "deep-equal",
                "children": [{
                    "kind": "role", "role": "listitem", "containerMode": "equal",
                    "children": [{
                        "kind": "role", "role": "list",
                        "children": [{
                            "kind": "role", "role": "listitem",
                            "children": [{ "kind": "text", "text": { "raw": "1.1", "normalized": "1.1" } }]
                        }]
                    }]
                }]
            }),
        },
        OracleCase {
            label: "node.text leaf matches single text child under deep-equal",
            tree: json!([{ "role": "paragraph", "text": "Only child" }]),
            template: json!({
                "kind": "role", "role": "paragraph", "containerMode": "deep-equal",
                "children": [{ "kind": "text", "text": { "raw": "Only child", "normalized": "Only child" } }]
            }),
        },
        OracleCase {
            label: "top-level fragment deep-equal rejects non-empty child",
            tree: json!([{ "role": "list", "children": [{ "role": "listitem", "text": "x" }] }]),
            template: json!({
                "kind": "role", "role": "fragment", "containerMode": "deep-equal",
                "children": [{ "kind": "role", "role": "list", "name": "" }]
            }),
        },
    ]
}

fn oracle_multimatch_cases() -> Vec<OracleCase> {
    vec![
        OracleCase {
            label: "contain greedy non-backtracking",
            tree: json!([{
                "role": "region",
                "children": [
                    { "role": "group", "children": [{ "role": "button", "name": "A" }, { "role": "button", "name": "B" }] },
                    { "role": "group", "children": [{ "role": "button", "name": "A" }] }
                ]
            }]),
            template: json!({
                "kind": "role", "role": "region",
                "children": [
                    { "kind": "role", "role": "group", "children": [{ "kind": "role", "role": "button", "name": "A" }] },
                    { "kind": "role", "role": "group", "children": [{ "kind": "role", "role": "button", "name": "B" }] }
                ]
            }),
        },
        OracleCase {
            label: "collectAll stops at matched ancestors without descending",
            tree: json!([
                {
                    "role": "group", "name": "outer-1",
                    "children": [
                        { "role": "group", "name": "inner-1" },
                        { "role": "button", "name": "btn" }
                    ]
                },
                {
                    "role": "section",
                    "children": [
                        { "role": "group", "name": "nested-2" },
                        { "role": "group", "name": "nested-3" }
                    ]
                }
            ]),
            template: json!({ "kind": "role", "role": "group" }),
        },
        OracleCase {
            label: "collectAll reports same parent twice for two matching text siblings",
            tree: json!([
                {
                    "role": "paragraph",
                    "children": ["hit", { "role": "strong", "text": "miss" }, "hit"]
                },
                { "role": "paragraph", "text": "hit" }
            ]),
            template: json!({ "kind": "text", "text": { "raw": "hit", "normalized": "hit" } }),
        },
    ]
}

fn oracle_attribute_and_url_cases() -> Vec<OracleCase> {
    let rich_tree = json!([
        {
            "role": "checkbox",
            "name": "Terms",
            "checked": "mixed",
            "disabled": false,
            "expanded": true,
            "active": false,
            "invalid": "grammar",
            "level": 2.0,
            "pressed": false,
            "selected": true,
            "url": "https://example.com/path?a=1",
            "placeholder": "ignored"
        },
        {
            "role": "heading",
            "name": "Zero",
            "level": -0.0
        },
        {
            "role": "link",
            "name": "BadRegexUrl",
            "url": "/[a/"
        },
        {
            "role": "button",
            "name": ""
        }
    ]);
    vec![
        OracleCase {
            label: "all attributes exact match with active and placeholder ignored",
            tree: rich_tree.clone(),
            template: json!({
                "kind": "role",
                "role": "checkbox",
                "name": "Terms",
                "checked": "mixed",
                "disabled": false,
                "expanded": true,
                "active": true,
                "invalid": "grammar",
                "level": 2,
                "pressed": false,
                "selected": true,
                "props": {
                    "url": { "raw": "/example\\.com/", "normalized": "/example\\.com/" },
                    "placeholder": { "raw": "mismatch", "normalized": "mismatch" }
                }
            }),
        },
        OracleCase {
            label: "level 0 equals -0.0",
            tree: rich_tree.clone(),
            template: json!({ "kind": "role", "role": "heading", "level": 0 }),
        },
        OracleCase {
            label: "explicit false attribute does not match absent field",
            tree: rich_tree.clone(),
            template: json!({ "kind": "role", "role": "heading", "disabled": false }),
        },
        OracleCase {
            label: "invalid spelling does not match invalid grammar",
            tree: rich_tree.clone(),
            template: json!({ "kind": "role", "role": "checkbox", "invalid": "spelling" }),
        },
        OracleCase {
            label: "bad /url regex matches literal url",
            tree: rich_tree.clone(),
            template: json!({
                "kind": "role", "role": "link",
                "props": { "url": { "raw": "/[a/", "normalized": "/[a/" } }
            }),
        },
        OracleCase {
            label: "empty normalized /url matches even link without url",
            tree: json!([{ "role": "link", "name": "NoHref" }]),
            template: json!({
                "kind": "role", "role": "link",
                "props": { "url": { "raw": " ", "normalized": "" } }
            }),
        },
        OracleCase {
            label: "empty exact name matches empty-named button",
            tree: rich_tree.clone(),
            template: json!({ "kind": "role", "role": "button", "name": "" }),
        },
        OracleCase {
            label: "non-empty exact name rejects empty-named button",
            tree: rich_tree,
            template: json!({ "kind": "role", "role": "button", "name": "Submit" }),
        },
    ]
}

#[test]
fn differential_matches_playwright_quickjs_oracle() -> Result<(), Box<dyn Error>> {
    let mut cases = oracle_regex_and_text_cases();
    cases.extend(oracle_utf16_cases());
    cases.extend(oracle_container_mode_cases());
    cases.extend(oracle_multimatch_cases());
    cases.extend(oracle_attribute_and_url_cases());

    let oracle_answers = run_quickjs_oracle(&cases)?;
    assert_eq!(
        oracle_answers.len(),
        cases.len(),
        "QuickJS oracle must produce one answer per differential case"
    );

    for (case, oracle) in cases.iter().zip(&oracle_answers) {
        let tree = parse_tree(&case.tree)?;
        let template = parse_template(&case.template)?;

        match matches_template(&tree, &template) {
            Ok(matched) => {
                assert!(
                    !oracle.first_error,
                    "case {:?}: Rust succeeded with {matched} while QuickJS threw",
                    case.label
                );
                assert_eq!(
                    matched, oracle.first_matched,
                    "case {:?}: matches_template disagreed with QuickJS (collectAll=false)",
                    case.label
                );
            }
            Err(TemplateError::InvalidRegex(_)) => {
                assert!(
                    oracle.first_error,
                    "case {:?}: Rust returned InvalidRegex while QuickJS returned {}",
                    case.label, oracle.first_matched
                );
            }
        }

        match match_template(&tree, &template) {
            Ok(matches) => {
                assert!(
                    !oracle.all_error,
                    "case {:?}: Rust match_template succeeded while QuickJS threw",
                    case.label
                );
                let rust_paths: Vec<String> = matches
                    .into_iter()
                    .map(|found| match_path(&tree, found))
                    .collect::<Result<_, _>>()?;
                assert_eq!(
                    rust_paths, oracle.all_paths,
                    "case {:?}: match_template paths disagreed with QuickJS (collectAll=true)",
                    case.label
                );
            }
            Err(TemplateError::InvalidRegex(_)) => {
                assert!(
                    oracle.all_error,
                    "case {:?}: Rust match_template returned InvalidRegex while QuickJS succeeded",
                    case.label
                );
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Deep Tree Stack-Safety Stress Tests (100,000 levels)
// ---------------------------------------------------------------------------

/// Builds a `depth`-level deep tree (`main` root, `depth - 2` `generic` wrappers, leaf `button`).
/// Using a distinct `"main"` root avoids O(depth^2) retries when testing a mismatching
/// `depth`-level template at the root.
fn build_deep_tree(depth: usize, leaf_name: &str) -> Vec<AriaNode> {
    let mut node = bare_aria_node("button");
    node.name = Some(leaf_name.to_owned());
    for level in 1..depth {
        let role = if level + 1 == depth {
            "main"
        } else {
            "generic"
        };
        let mut parent = bare_aria_node(role);
        parent.children.push(AriaChild::Node(Box::new(node)));
        node = parent;
    }
    vec![node]
}

/// Builds a `depth`-level deep template with `ContainerMode::DeepEqual` on the outer `"main"`
/// node so exact-children matching propagates through all `depth` levels.
fn build_deep_equal_template(depth: usize, leaf_name: &str) -> AriaTemplateNode {
    let mut template = AriaTemplateNode::Role(AriaTemplateRoleNode {
        role: "button".to_owned(),
        name: Some(TemplateName::Exact(leaf_name.to_owned())),
        ..AriaTemplateRoleNode::default()
    });
    for level in 1..depth {
        let (role, container_mode) = if level + 1 == depth {
            ("main", Some(ContainerMode::DeepEqual))
        } else {
            ("generic", None)
        };
        template = AriaTemplateNode::Role(AriaTemplateRoleNode {
            role: role.to_owned(),
            container_mode,
            children: vec![template],
            ..AriaTemplateRoleNode::default()
        });
    }
    template
}

#[test]
fn stress_matches_and_scans_100_000_level_deep_trees_without_stack_overflow()
-> Result<(), Box<dyn Error>> {
    const DEPTH: usize = 100_000;
    let tree = build_deep_tree(DEPTH, "Deep Leaf");

    let matching_leaf = AriaTemplateNode::Role(AriaTemplateRoleNode {
        role: "button".to_owned(),
        name: Some(TemplateName::Exact("Deep Leaf".to_owned())),
        ..AriaTemplateRoleNode::default()
    });
    assert!(
        matches_template(&tree, &matching_leaf)?,
        "matching a leaf 100,000 levels deep must succeed without stack overflow"
    );

    let missing_leaf = AriaTemplateNode::Role(AriaTemplateRoleNode {
        role: "button".to_owned(),
        name: Some(TemplateName::Exact("Not Present".to_owned())),
        ..AriaTemplateRoleNode::default()
    });
    assert!(
        !matches_template(&tree, &missing_leaf)?,
        "scanning all 100,000 levels for a non-matching leaf must finish cleanly with false"
    );
    assert!(
        match_template(&tree, &missing_leaf)?.is_empty(),
        "match_template scanning all 100,000 levels for a missing leaf must return empty"
    );

    let found = match_template(&tree, &matching_leaf)?;
    assert_eq!(
        found.len(),
        1,
        "match_template must collect the single leaf at depth 100,000"
    );
    let TemplateMatch::Node(leaf) = found[0] else {
        return Err("expected TemplateMatch::Node for the deep leaf".into());
    };
    assert_eq!(
        (leaf.role.as_str(), leaf.name.as_deref()),
        ("button", Some("Deep Leaf")),
        "match_template must return a reference to the leaf button at depth 100,000"
    );

    let matching_deep_equal = build_deep_equal_template(DEPTH, "Deep Leaf");
    assert!(
        matches_template(&tree, &matching_deep_equal)?,
        "a 100,000-level deep-equal template must match a matching 100,000-level tree"
    );

    let mismatching_deep_equal = build_deep_equal_template(DEPTH, "Wrong Leaf");
    assert!(
        !matches_template(&tree, &mismatching_deep_equal)?,
        "a 100,000-level deep-equal template must reject a mismatching leaf at depth 100,000"
    );
    Ok(())
}
