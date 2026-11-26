//! The tree's answers to the four accessibility matchers and `toMatchAriaSnapshot`, each held to
//! upstream's `expectSingleElement`, `ExpectedTextMatcher` and `matchesExpectAriaTemplate` rules,
//! over hand-built APC trees (`ApcBuilder`).

use std::error::Error;

use neo_snapshot_apc::proto::{BrowserOsRef, BrowserOsTriState, FormControlType, SelectOption};
use neo_snapshot_apc::tree::{
    ApcBuilder, BuildOptions, DocToken, DomNodeId, NodeBuilder, NodeId, NodeKey, SnapshotTree,
};
use serde_json::{Map, Value, json};

use super::{
    AriaExpect, AriaExpectError, ExpectedText, ExpectedTextValue, FailureSnapshot, PropertyMatcher,
    TemplateMatcher, Visibility,
};
use crate::probe::ExpectParams;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const BUTTON: i32 = 10;
const INPUT: i32 = 20;
const ERROR_SPAN: i32 = 30;
const PLAIN: i32 = 40;

/// A page with a described button, a text box whose error message keeps an interior NBSP (the
/// error message is flattened, not whitespace-normalized), and a `div` with no role.
fn page() -> TestResult<SnapshotTree> {
    let root = NodeBuilder::root()
        .child(
            NodeBuilder::element("button")
                .dom_id(BUTTON)
                .aria_description("Charges the card")
                .child(NodeBuilder::text("Pay now")),
        )
        .child(
            NodeBuilder::form_control(FormControlType::InputText)
                .tag("input")
                .dom_id(INPUT)
                .aria_label("Card number")
                .aria_invalid("true")
                .errormessage(vec![BrowserOsRef {
                    dom_node_id: Some(ERROR_SPAN),
                    unrendered_text: None,
                }]),
        )
        .child(
            NodeBuilder::element("span")
                .dom_id(ERROR_SPAN)
                .child(NodeBuilder::text("Card\u{00A0}declined")),
        )
        .child(
            NodeBuilder::element("div")
                .dom_id(PLAIN)
                .child(NodeBuilder::text("plain")),
        );
    build(root)
}

fn build(root: NodeBuilder) -> TestResult<SnapshotTree> {
    let apc = ApcBuilder::new("doc-main").root(root).build();
    Ok(SnapshotTree::build(apc, BuildOptions::default())?)
}

fn node(tree: &SnapshotTree, dom: i32) -> TestResult<NodeId> {
    let token = DocToken::new("doc-main").ok_or("the token is not empty")?;
    let key = NodeKey::new(token, DomNodeId::new(dom));
    Ok(tree
        .node_by_key(&key)
        .ok_or("the page holds this node")?
        .id())
}

/// The matcher the prelude's params for `expression` decode to, with `expected_text` as
/// `expectedText`.
fn decoded(expression: &str, expected_text: Value) -> Result<Option<AriaExpect>, AriaExpectError> {
    AriaExpect::new(&with(expression, "expectedText", expected_text))
}

/// The prelude's params for `expression` with `value` under `key`.
fn with(expression: &str, key: &str, value: Value) -> ExpectParams {
    let mut map = Map::new();
    map.insert("expression".to_owned(), json!(expression));
    map.insert(key.to_owned(), value);
    map.insert("isNot".to_owned(), json!(false));
    ExpectParams(map)
}

fn property(expression: &str, expected: Value) -> TestResult<PropertyMatcher> {
    match decoded(expression, Value::Array(vec![expected]))? {
        Some(AriaExpect::Property(matcher)) => Ok(matcher),
        Some(AriaExpect::Template(_)) | None => Err(format!("{expression} is a property").into()),
    }
}

fn template(value: Value) -> TestResult<TemplateMatcher> {
    match AriaExpect::new(&with("to.match.aria", "expectedValue", value))? {
        Some(AriaExpect::Template(matcher)) => Ok(matcher),
        Some(AriaExpect::Property(_)) | None => Err("to.match.aria is a template".into()),
    }
}

fn expected(value: Value) -> TestResult<ExpectedText> {
    let value: ExpectedTextValue = serde_json::from_value(value)?;
    Ok(ExpectedText::new(value)?)
}

/// `(matches, received)` of `expression` with `expected` on `target`.
fn polled(
    tree: &SnapshotTree,
    expression: &str,
    expected: Value,
    target: Option<NodeId>,
) -> TestResult<(bool, Value)> {
    let names = tree.name_cache();
    let verdict = property(expression, expected)?.check(tree, &names, target);
    Ok((verdict.matches, verdict.received))
}

#[test]
fn tests_that_each_property_matcher_reads_its_property_from_the_tree() -> TestResult {
    let tree = page()?;
    let button = Some(node(&tree, BUTTON)?);
    let input = Some(node(&tree, INPUT)?);
    let rows = [
        ("to.have.role", button, "button"),
        ("to.have.role", input, "textbox"),
        ("to.have.accessible.name", button, "Pay now"),
        ("to.have.accessible.name", input, "Card number"),
        ("to.have.accessible.description", button, "Charges the card"),
        (
            "to.have.accessible.error.message",
            input,
            "Card\u{00A0}declined",
        ),
    ];
    for (expression, target, value) in rows {
        let (matches, received) = polled(&tree, expression, json!({ "string": value }), target)?;
        assert_eq!(
            received,
            json!(value),
            "{expression} receives the tree's own value as a string"
        );
        assert!(matches, "{expression} matches its own value exactly");
        let (matches, _) = polled(&tree, expression, json!({ "string": "other" }), target)?;
        assert!(!matches, "{expression} differs from another value");
    }
    Ok(())
}

#[test]
fn tests_that_an_element_without_a_role_or_a_node_reads_empty() -> TestResult {
    let tree = page()?;
    let plain = Some(node(&tree, PLAIN)?);
    for expression in [
        "to.have.role",
        "to.have.accessible.name",
        "to.have.accessible.description",
        "to.have.accessible.error.message",
    ] {
        for target in [None, plain] {
            let (matches, received) = polled(&tree, expression, json!({ "string": "" }), target)?;
            assert_eq!(
                received,
                json!(""),
                "an element the tree has no node for, or a plain div, has no {expression} value \
                 (design 5.6), as upstream's `getAriaRole(element) || ''`"
            );
            assert!(matches, "the empty value equals the empty string");
        }
    }
    Ok(())
}

#[test]
fn tests_that_a_string_matches_exactly_or_as_a_substring() -> TestResult {
    let exact = expected(json!({ "string": "Pay now" }))?;
    assert!(exact.matches("Pay now"), "the same text is equal");
    assert!(!exact.matches("Pay now!"), "exact is not a prefix test");
    assert!(!exact.matches("pay now"), "case counts without ignoreCase");
    assert!(
        !exact.matches("Pay  now"),
        "whitespace counts without normalizeWhiteSpace"
    );

    let substring = expected(json!({ "string": "now", "matchSubstring": true }))?;
    assert!(substring.matches("Pay now"), "the text holds the string");
    assert!(!substring.matches("Pay later"), "the text lacks the string");
    let empty = expected(json!({ "string": "", "matchSubstring": true }))?;
    assert!(
        empty.matches("anything"),
        "every text holds the empty string"
    );
    Ok(())
}

#[test]
fn tests_that_normalize_white_space_and_ignore_case_apply_to_both_sides() -> TestResult {
    let spaced = expected(json!({ "string": "  Pay\n  now ", "normalizeWhiteSpace": true }))?;
    assert!(
        spaced.matches("Pay\u{00A0}\u{200B}now"),
        "both sides collapse whitespace, NBSP included, and drop zero-width spaces"
    );
    assert!(!spaced.matches("Paynow"), "a space is still a space");

    let cased = expected(json!({ "string": "PAY Now", "ignoreCase": true }))?;
    assert!(cased.matches("pay NOW"), "both sides are lowercased");
    assert!(
        !cased.matches("pay  now"),
        "ignoreCase alone leaves whitespace alone"
    );

    let both = expected(json!({
        "string": "PAY   NOW",
        "matchSubstring": true,
        "normalizeWhiteSpace": true,
        "ignoreCase": true,
    }))?;
    assert!(
        both.matches("Click to pay\tnow"),
        "a substring after both normalizations"
    );
    Ok(())
}

#[test]
fn tests_that_a_regex_runs_with_its_flags_on_the_text_unnormalized() -> TestResult {
    let anchored = expected(json!({ "regexSource": "^Pay", "regexFlags": "" }))?;
    assert!(anchored.matches("Pay now"), "the pattern matches");
    assert!(!anchored.matches("  Pay now"), "the text is not trimmed");

    let multiline = expected(json!({ "regexSource": "^now$", "regexFlags": "m" }))?;
    assert!(
        multiline.matches("Pay\nnow"),
        "the m flag reaches the regex"
    );

    let spaced = expected(json!({
        "regexSource": "Pay now",
        "regexFlags": "",
        "normalizeWhiteSpace": true,
    }))?;
    assert!(
        !spaced.matches("Pay   now"),
        "normalizeWhiteSpace never touches the text a regex tests"
    );

    let sticky = expected(json!({ "regexSource": "now", "regexFlags": "gy" }))?;
    assert!(
        !sticky.matches("Pay now"),
        "a sticky regex tests from lastIndex 0"
    );
    let repeated = expected(json!({ "regexSource": "now", "regexFlags": "gg" }))?;
    assert!(
        repeated.matches("Pay now"),
        "upstream's flag set drops the repeated g RegExp would refuse"
    );
    Ok(())
}

#[test]
fn tests_that_ignore_case_adds_and_removes_the_i_flag() -> TestResult {
    let added = expected(json!({ "regexSource": "pay", "regexFlags": "", "ignoreCase": true }))?;
    assert!(added.matches("PAY"), "ignoreCase: true adds i");
    let removed =
        expected(json!({ "regexSource": "pay", "regexFlags": "i", "ignoreCase": false }))?;
    assert!(!removed.matches("PAY"), "ignoreCase: false removes i");
    let kept = expected(json!({ "regexSource": "pay", "regexFlags": "i" }))?;
    assert!(kept.matches("PAY"), "no ignoreCase keeps the script's i");
    Ok(())
}

#[test]
fn tests_that_a_regex_reads_the_error_message_as_the_tree_flattens_it() -> TestResult {
    let tree = page()?;
    let input = Some(node(&tree, INPUT)?);
    let expression = "to.have.accessible.error.message";
    let literal_space = json!({ "regexSource": "Card declined", "regexFlags": "" });
    let (matches, _) = polled(&tree, expression, literal_space, input)?;
    assert!(
        !matches,
        "the NBSP the error message keeps is not an ASCII space to a regex"
    );
    let any_space = json!({ "regexSource": "Card\\sdeclined", "regexFlags": "" });
    let (matches, _) = polled(&tree, expression, any_space, input)?;
    assert!(matches, "\\s matches the NBSP");
    let normalized = json!({ "string": "Card declined", "normalizeWhiteSpace": true });
    let (matches, _) = polled(&tree, expression, normalized, input)?;
    assert!(matches, "normalizeWhiteSpace turns the NBSP into a space");
    Ok(())
}

#[test]
fn tests_that_a_property_matcher_without_expected_text_is_unknown() -> TestResult {
    for expected_text in [None, Some(Value::Null)] {
        let mut map = Map::new();
        map.insert("expression".to_owned(), json!("to.have.role"));
        map.insert("isNot".to_owned(), json!(false));
        if let Some(value) = expected_text {
            map.insert("expectedText".to_owned(), value);
        }
        let error = AriaExpect::new(&ExpectParams(map))
            .err()
            .ok_or("a property matcher needs expectedText")?;
        assert_eq!(
            error.to_string(),
            "Unknown expect matcher: to.have.role",
            "upstream throws this when `options.expectedText` is not set"
        );
    }
    Ok(())
}

#[test]
fn tests_that_params_no_prelude_sends_are_refused_by_path() -> TestResult {
    let rows = [
        (
            json!([]),
            "params.expectedText is not what Playwright's matchers send",
        ),
        (
            json!([{}]),
            "params.expectedText[0] is not what Playwright's matchers send",
        ),
        (
            json!([{ "string": "a", "regexSource": "a" }]),
            "params.expectedText[0] is not what Playwright's matchers send",
        ),
        (
            json!([{ "string": "secret", "extra": "secret" }]),
            "params.expectedText[0].extra is not what Playwright's matchers send",
        ),
    ];
    for (expected_text, message) in rows {
        let error = decoded("to.have.accessible.name", expected_text)
            .err()
            .ok_or("the prelude never sends this")?;
        assert_eq!(
            error.to_string(),
            message,
            "the refusal names the path and never quotes the value"
        );
    }
    Ok(())
}

#[test]
fn tests_that_an_invalid_regex_fails_without_quoting_it() -> TestResult {
    let error = decoded(
        "to.have.accessible.name",
        json!([{ "regexSource": "secret(", "regexFlags": "" }]),
    )
    .err()
    .ok_or("an unterminated group does not compile")?;
    let message = error.to_string();
    assert!(
        message.starts_with("Invalid regular expression: "),
        "upstream's RegExp error prefix: {message}"
    );
    assert!(
        !message.contains("secret"),
        "the source is the script's expected value: {message}"
    );
    Ok(())
}

#[test]
fn tests_that_other_expressions_stay_in_the_page() -> TestResult {
    for expression in [
        "to.have.text",
        "to.be.visible",
        "to.have.count",
        "to.have.title",
    ] {
        assert!(
            decoded(expression, json!([{ "string": "x" }]))?.is_none(),
            "{expression} is not a tree matcher"
        );
    }
    Ok(())
}

const MAIN: i32 = 60;

fn landmark_page() -> TestResult<SnapshotTree> {
    let list = NodeBuilder::element("ul")
        .child(NodeBuilder::element("li").child(NodeBuilder::text("One")))
        .child(NodeBuilder::element("li").child(NodeBuilder::text("Two")));
    let main = NodeBuilder::element("main")
        .dom_id(MAIN)
        .child(NodeBuilder::element("h1").child(NodeBuilder::text("Checkout")))
        .child(
            NodeBuilder::element("a")
                .href("/cart")
                .child(NodeBuilder::text("Back")),
        )
        .child(NodeBuilder::text("Total due"))
        .child(list);
    build(NodeBuilder::root().child(main))
}

const MAIN_YAML: &str = "- main:\n  - heading \"Checkout\" [level=1]\n  - link \"Back\":\n    - /url: /cart\n  - text: Total due\n  - list:\n    - listitem: One\n    - listitem: Two";

fn role(role: &str, extra: Value) -> Value {
    let mut node = json!({ "kind": "role", "role": role });
    if let (Value::Object(node), Value::Object(extra)) = (&mut node, extra) {
        node.extend(extra);
    }
    node
}

fn text(value: &str) -> Value {
    json!({ "kind": "text", "text": { "raw": value, "normalized": value } })
}

#[test]
fn tests_that_a_template_matches_the_element_tree_and_receives_its_yaml() -> TestResult {
    let template = template(role(
        "fragment",
        json!({ "children": [role("main", json!({ "children": [
            role("heading", json!({ "name": "Checkout", "level": 1 })),
            text("Total due"),
            role("list", json!({ "children": [role("listitem", json!({ "children": [text("Two")] }))] })),
        ] }))] }),
    ))?;
    let tree = landmark_page()?;
    let verdict = template.check(&tree, node(&tree, MAIN)?)?;
    assert!(
        verdict.matches,
        "contain mode: a subsequence of the main's children"
    );
    assert_eq!(
        verdict.received,
        json!({ "raw": MAIN_YAML }),
        "received is upstream's `{{ raw }}`, the default-mode YAML"
    );
    Ok(())
}

#[test]
fn tests_that_a_template_that_does_not_match_still_receives_the_yaml() -> TestResult {
    let rows = [
        role("heading", json!({ "name": "Payment" })),
        role("heading", json!({ "level": 2 })),
        role(
            "list",
            json!({ "containerMode": "equal", "children": [role("listitem", json!({}))] }),
        ),
    ];
    let tree = landmark_page()?;
    for row in rows {
        let verdict = template(row)?.check(&tree, node(&tree, MAIN)?)?;
        assert!(
            !verdict.matches,
            "no subtree has that name, level or exact child list"
        );
        assert_eq!(
            verdict.received,
            json!({ "raw": MAIN_YAML }),
            "a failed match prints the same tree"
        );
    }
    Ok(())
}

#[test]
fn tests_that_a_template_errors_on_a_name_regex_that_does_not_compile() -> TestResult {
    let template = template(role("heading", json!({ "name": { "pattern": "(" } })))?;
    let tree = landmark_page()?;
    let error = template
        .check(&tree, node(&tree, MAIN)?)
        .err()
        .ok_or("an unterminated group does not compile")?;
    assert!(
        matches!(error, AriaExpectError::Template(_)),
        "the template's own error, raised where upstream's page throws"
    );
    Ok(())
}

const TERMS: i32 = 70;
const SAVE: i32 = 71;
const MENU: i32 = 72;
const COUNTRY: i32 = 73;

/// An unchecked checkbox, a button with no `aria-expanded`, one with `aria-expanded="false"`, and
/// a `<select>` whose options APC sends as data, which the tree synthesizes as option nodes.
fn states_page() -> TestResult<SnapshotTree> {
    let option = |text: &str, selected: bool| SelectOption {
        value: Some(text.to_lowercase()),
        text: Some(text.to_owned()),
        is_selected: Some(selected),
        is_disabled: Some(false),
    };
    let root = NodeBuilder::root()
        .child(
            NodeBuilder::form_control(FormControlType::InputCheckbox)
                .tag("input")
                .dom_id(TERMS)
                .aria_label("Terms"),
        )
        .child(
            NodeBuilder::element("button")
                .dom_id(SAVE)
                .child(NodeBuilder::text("Save")),
        )
        .child(
            NodeBuilder::element("button")
                .dom_id(MENU)
                .expanded_state(BrowserOsTriState::False)
                .child(NodeBuilder::text("Menu")),
        )
        .child(
            NodeBuilder::form_control(FormControlType::SelectOne)
                .tag("select")
                .dom_id(COUNTRY)
                .aria_label("Country")
                .select_option(option("France", true))
                .select_option(option("Spain", false)),
        );
    build(root)
}

fn matches_at(tree: &SnapshotTree, dom: i32, template: Value) -> TestResult<bool> {
    Ok(self::template(template)?
        .check(tree, node(tree, dom)?)?
        .matches)
}

#[test]
fn tests_that_a_template_compares_the_explicit_false_states_upstream_writes() -> TestResult {
    let tree = states_page()?;
    let rows = [
        (
            TERMS,
            role("checkbox", json!({ "checked": false })),
            true,
            "[checked=false] matches an unchecked checkbox",
        ),
        (
            TERMS,
            role("checkbox", json!({ "checked": true })),
            false,
            "[checked] does not",
        ),
        (
            MENU,
            role("button", json!({ "expanded": false })),
            true,
            "[expanded=false] matches aria-expanded=\"false\"",
        ),
        (
            SAVE,
            role("button", json!({ "expanded": false })),
            false,
            "and not a button with no aria-expanded, which upstream leaves undefined",
        ),
        (
            SAVE,
            role("button", json!({ "pressed": false, "disabled": false })),
            true,
            "a plain button is not pressed and not disabled",
        ),
        (
            COUNTRY,
            role("option", json!({ "name": "Spain", "selected": false })),
            true,
            "[selected=false] matches an unselected synthesized option",
        ),
        (
            COUNTRY,
            role("option", json!({ "name": "France", "selected": false })),
            false,
            "and not the selected one",
        ),
    ];
    for (dom, row, expected, why) in rows {
        assert_eq!(matches_at(&tree, dom, row)?, expected, "{why}");
    }
    let verdict = template(role("checkbox", json!({ "checked": false })))?
        .check(&tree, node(&tree, TERMS)?)?;
    assert_eq!(
        verdict.received,
        json!({ "raw": "- checkbox \"Terms\"" }),
        "received prints default mode's YAML, where false is absent"
    );
    Ok(())
}

#[test]
fn tests_that_a_template_missing_or_malformed_is_refused_by_path() -> TestResult {
    let rows = [
        (
            None,
            "params.expectedValue is not what Playwright's matchers send",
        ),
        (
            Some(json!({ "kind": "role", "role": "button", "secret": "x" })),
            "params.expectedValue is not what Playwright's matchers send",
        ),
    ];
    for (value, message) in rows {
        let mut map = Map::new();
        map.insert("expression".to_owned(), json!("to.match.aria"));
        map.insert("isNot".to_owned(), json!(false));
        if let Some(value) = value {
            map.insert("expectedValue".to_owned(), value);
        }
        let error = AriaExpect::new(&ExpectParams(map))
            .err()
            .ok_or("the prelude always sends a parsed template")?;
        assert_eq!(
            error.to_string(),
            message,
            "the refusal never quotes the template"
        );
    }
    Ok(())
}

#[test]
fn tests_the_failure_snapshot_decision_table() {
    let rows = [
        (true, true, FailureSnapshot::Element),
        (true, false, FailureSnapshot::Body),
        (false, true, FailureSnapshot::Body),
        (false, false, FailureSnapshot::Body),
    ];
    for (frame, element, snapshot) in rows {
        assert_eq!(
            FailureSnapshot::for_failed_poll(Visibility { frame, element }),
            snapshot,
            "upstream snapshots the element's line only when both the frame and the element are \
             visible (frame {frame}, element {element})"
        );
    }
}
