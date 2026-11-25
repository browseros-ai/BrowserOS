//! Differential and round-trip tests for `neo_snapshot_apc::select` against Playwright 1.64.1
//! running in QuickJS, plus vendored SHA-256 pin verification.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use neo_snapshot::Tristate;
use neo_snapshot_apc::select::{
    AttributeSelector, AttributeSelectorError, AttributeSelectorOperator, AttributeSelectorPart,
    AttributeValue, JsRegex, LAYOUT_SELECTOR_NAMES, NESTED_SELECTOR_NAMES, NestedSelectorBody,
    ParsedSelector, RoleQuery, RoleQueryError, SelectorBody, StringMatchOperator, TextFilter,
    UnquotedStrings, parse_attribute_selector,
};
use rquickjs::{CatchResultExt, Context, Runtime};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const PINNED_SOURCES: &[(&str, &str)] = &[
    (
        "packages/isomorphic/selectorParser.ts",
        "f8f7f8b97d0c4d40c54cf2101f273490171a05a569d292196e56e02d11a0acc1",
    ),
    (
        "packages/injected/src/roleSelectorEngine.ts",
        "f5ac36e7cd00993740ba8cb48d29680dd4fd53298358ca88eb9efb87d4bd8268",
    ),
    (
        "packages/injected/src/roleUtils.ts",
        "534e7dd5469e60ae74ced37264589676c574fa812b4809f6e9e7f135bbb1cd7e",
    ),
];

/// Ported verbatim (types stripped) from vendored Playwright 1.64.1
/// `packages/injected/src/roleUtils.ts:1070-1160` and
/// `packages/injected/src/roleSelectorEngine.ts:43-141, 209-213`.
///
/// The SHA-256 pin test ensures any upgrade to those vendored files fails this suite loudly.
const ROLE_VALIDATION_ORACLE_JS: &str = r#"
const kAriaSelectedRoles = ['gridcell', 'option', 'row', 'tab', 'rowheader', 'columnheader', 'treeitem'];
const kAriaCheckedRoles = ['checkbox', 'menuitemcheckbox', 'option', 'radio', 'switch', 'menuitemradio', 'treeitem'];
const kAriaPressedRoles = ['button'];
const kAriaExpandedRoles = ['application', 'button', 'checkbox', 'combobox', 'gridcell', 'link', 'listbox', 'menuitem', 'row', 'rowheader', 'tab', 'treeitem', 'columnheader', 'menuitemcheckbox', 'menuitemradio', 'rowheader', 'switch'];
const kAriaLevelRoles = ['heading', 'listitem', 'row', 'treeitem'];

const kSupportedAttributes = ['selected', 'checked', 'pressed', 'expanded', 'level', 'disabled', 'name', 'description', 'include-hidden'];
kSupportedAttributes.sort();

function validateSupportedRole(attr, roles, role) {
  if (!roles.includes(role))
    throw new Error(`"${attr}" attribute is only supported for roles: ${roles.slice().sort().map(r => `"${r}"`).join(', ')}`);
}

function validateSupportedValues(attr, values) {
  if (attr.op !== '<truthy>' && !values.includes(attr.value))
    throw new Error(`"${attr.name}" must be one of ${values.map(v => JSON.stringify(v)).join(', ')}`);
}

function validateSupportedOp(attr, ops) {
  if (!ops.includes(attr.op))
    throw new Error(`"${attr.name}" does not support "${attr.op}" matcher`);
}

function validateAttributes(attrs, role) {
  const options = { role };
  for (const attr of attrs) {
    switch (attr.name) {
      case 'checked': {
        validateSupportedRole(attr.name, kAriaCheckedRoles, role);
        validateSupportedValues(attr, [true, false, 'mixed']);
        validateSupportedOp(attr, ['<truthy>', '=']);
        options.checked = attr.op === '<truthy>' ? true : attr.value;
        break;
      }
      case 'pressed': {
        validateSupportedRole(attr.name, kAriaPressedRoles, role);
        validateSupportedValues(attr, [true, false, 'mixed']);
        validateSupportedOp(attr, ['<truthy>', '=']);
        options.pressed = attr.op === '<truthy>' ? true : attr.value;
        break;
      }
      case 'selected': {
        validateSupportedRole(attr.name, kAriaSelectedRoles, role);
        validateSupportedValues(attr, [true, false]);
        validateSupportedOp(attr, ['<truthy>', '=']);
        options.selected = attr.op === '<truthy>' ? true : attr.value;
        break;
      }
      case 'expanded': {
        validateSupportedRole(attr.name, kAriaExpandedRoles, role);
        validateSupportedValues(attr, [true, false]);
        validateSupportedOp(attr, ['<truthy>', '=']);
        options.expanded = attr.op === '<truthy>' ? true : attr.value;
        break;
      }
      case 'level': {
        validateSupportedRole(attr.name, kAriaLevelRoles, role);
        if (typeof attr.value === 'string')
          attr.value = +attr.value;
        if (attr.op !== '=' || typeof attr.value !== 'number' || Number.isNaN(attr.value))
          throw new Error(`"level" attribute must be compared to a number`);
        options.level = attr.value;
        break;
      }
      case 'disabled': {
        validateSupportedValues(attr, [true, false]);
        validateSupportedOp(attr, ['<truthy>', '=']);
        options.disabled = attr.op === '<truthy>' ? true : attr.value;
        break;
      }
      case 'name': {
        if (attr.op === '<truthy>')
          throw new Error(`"name" attribute must have a value`);
        if (typeof attr.value !== 'string' && !(attr.value instanceof RegExp))
          throw new Error(`"name" attribute must be a string or a regular expression`);
        options.name = attr.value;
        options.nameOp = attr.op;
        options.nameExact = attr.caseSensitive;
        break;
      }
      case 'description': {
        if (attr.op === '<truthy>')
          throw new Error(`"description" attribute must have a value`);
        if (typeof attr.value !== 'string' && !(attr.value instanceof RegExp))
          throw new Error(`"description" attribute must be a string or a regular expression`);
        options.description = attr.value;
        options.descriptionOp = attr.op;
        options.descriptionExact = attr.caseSensitive;
        break;
      }
      case 'include-hidden': {
        validateSupportedValues(attr, [true, false]);
        validateSupportedOp(attr, ['<truthy>', '=']);
        options.includeHidden = attr.op === '<truthy>' ? true : attr.value;
        break;
      }
      default: {
        throw new Error(`Unknown attribute "${attr.name}", must be one of ${kSupportedAttributes.map(a => `"${a}"`).join(', ')}.`);
      }
    }
  }
  return options;
}

function validateRoleSelector(body) {
  const parsed = globalThis.__oracle.parseAttributeSelector(body, true);
  const role = parsed.name.toLowerCase();
  if (!role)
    throw new Error('Role must not be empty');
  return validateAttributes(parsed.attributes, role);
}
"#;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum JsAttrOutcome {
    Ok {
        name: String,
        attributes: Vec<JsAttrPart>,
    },
    Err {
        message: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsAttrPart {
    name: String,
    json_path: Vec<String>,
    op: String,
    value: JsAttrValue,
    case_sensitive: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum JsAttrValue {
    Null,
    Bool { value: bool },
    Number { bits: [u32; 2] },
    String { value: String },
    Regex { source: String, flags: String },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum JsRoleOutcome {
    Ok(JsRoleOptions),
    Err { message: String },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsRoleOptions {
    role: String,
    name: Option<JsTextFilter>,
    description: Option<JsTextFilter>,
    checked: Option<Value>,
    pressed: Option<Value>,
    selected: Option<bool>,
    expanded: Option<bool>,
    level_bits: Option<[u32; 2]>,
    disabled: Option<bool>,
    include_hidden: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum JsTextFilter {
    String {
        value: String,
        op: String,
        exact: bool,
    },
    Regex {
        source: String,
        flags: String,
    },
}

#[derive(Deserialize)]
struct EscapedRegex {
    escaped: String,
    source: String,
    flags: String,
}

fn neo_playwright_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../neo-playwright")
}

fn load_prelude_oracle_script() -> Result<String, Box<dyn Error>> {
    let prelude_path = neo_playwright_dir().join("assets/prelude.js");
    let source = fs::read_to_string(&prelude_path)?;
    let trimmed = source.trim_end();
    let Some(without_iife_end) = trimmed.strip_suffix("})();") else {
        return Err("prelude.js no longer ends with `})();`".into());
    };
    Ok(format!(
        "globalThis.__neo = {{ brands: {{ register() {{}} }}, onReply() {{}}, onSettle() {{}}, provide() {{}} }};\n\
         {without_iife_end}\n\
         globalThis.__oracle = {{\n\
           parseAttributeSelector,\n\
           splitSelectorByFrame,\n\
           getByRoleSelector,\n\
           escapeForAttributeSelector,\n\
           escapeRegexForSelector,\n\
         }};\n\
         }})();"
    ))
}

fn eval_with_prelude<T: DeserializeOwned>(
    input: &Value,
    script: &str,
) -> Result<T, Box<dyn Error>> {
    let prelude = load_prelude_oracle_script()?;
    let input_json = serde_json::to_string(input)?;
    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    let output: String = context.with(|ctx| -> Result<String, String> {
        ctx.eval::<(), _>(prelude.as_str())
            .catch(&ctx)
            .map_err(|caught| caught.to_string())?;
        ctx.globals()
            .set("input", input_json)
            .catch(&ctx)
            .map_err(|caught| caught.to_string())?;
        ctx.eval::<String, _>(script)
            .catch(&ctx)
            .map_err(|caught| caught.to_string())
    })?;
    Ok(serde_json::from_str(&output)?)
}

fn eval_quickjs<T: DeserializeOwned>(input: &Value, script: &str) -> Result<T, Box<dyn Error>> {
    let input_json = serde_json::to_string(input)?;
    let runtime = Runtime::new()?;
    let context = Context::full(&runtime)?;
    let output: String = context.with(|ctx| -> Result<String, String> {
        ctx.globals()
            .set("input", input_json)
            .catch(&ctx)
            .map_err(|caught| caught.to_string())?;
        ctx.eval::<String, _>(script)
            .catch(&ctx)
            .map_err(|caught| caught.to_string())
    })?;
    Ok(serde_json::from_str(&output)?)
}

fn number_bits(value: f64) -> Result<[u32; 2], Box<dyn Error>> {
    let bits = value.to_bits();
    Ok([
        u32::try_from(bits >> 32)?,
        u32::try_from(bits & 0xffff_ffff)?,
    ])
}

fn generated_get_by_role_bodies() -> Result<Vec<String>, Box<dyn Error>> {
    let script = r#"
        const { getByRoleSelector } = globalThis.__oracle;
        const calls = [
            ['menuitem', {}],
            ['option', { selected: true }],
            ['option', { selected: false }],
            ['checkbox', { checked: true }],
            ['checkbox', { checked: false }],
            ['checkbox', { checked: 'mixed' }],
            ['button', { pressed: true }],
            ['button', { pressed: false }],
            ['button', { pressed: 'mixed' }],
            ['treeitem', {}],
            ['treeitem', { expanded: true }],
            ['treeitem', { expanded: false }],
            ['button', { disabled: true }],
            ['button', { disabled: false }],
            ['option', { disabled: true }],
            ['option', { disabled: false }],
            ['option', {}],
            ['heading', { level: 1 }],
            ['heading', { level: 3 }],
            ['heading', { level: 5 }],
            ['button', { name: 'Hello' }],
            ['button', { name: /^H[ae]llo$/ }],
            ['button', { name: /h.*o/i }],
            ['button', { name: 'Hello', includeHidden: true }],
            ['button', { name: 'hello', includeHidden: true }],
            ['button', { name: '123', includeHidden: true }],
            ['option', { name: 'HTML' }],
            ['status', { name: 'Output1' }],
            ['listitem', { name: 'Parent list' }],
            ['listitem', {}],
            ['link', { name: 'issues 123' }],
            ['link', { name: 'sues' }],
            ['link', { name: '  he    \n  llo ' }],
            ['button', { name: 'issues' }],
            ['link', { name: 'sues', exact: true }],
            ['link', { name: '   he \n llo 56 ', exact: true }],
            ['button', { name: 'Click me', exact: true }],
            ['button', { name: 'Click \\me', exact: true }],
            ['button', { name: 'Click \\\\me', exact: true }],
            ['button', { name: 'Click \\\\\\me', exact: true }],
            ['button', { name: 'foo"bar', exact: true }],
            ['button', { name: /Click me/v }],
            ['button', { name: /click me/iv }],
            ['button', { name: /Missing/v }],
            ['alert', { description: 'doc-2025' }],
            ['alert', { description: 'report-2026' }],
            ['alert', { name: 'Upload successful', description: 'doc-2025' }],
            ['alert', { name: 'Upload successful', description: 'report-2026' }],
            ['alert', { name: 'Invalid file', description: 'doc-2025' }],
            ['alert', { description: 'doc-2025', exact: true }],
            ['alert', { description: 'File doc-2025.pdf was uploaded successfully', exact: true }],
            ['alert', { description: /report-\d+/ }],
            ['alert', { description: /uploaded successfully$/ }],
            ['button', { name: 'Submit', description: 'form data' }],
            ['button', { name: 'Submit', description: 'draft' }],
            ['button', { description: 'Submits' }],
            ['alert', { description: '  doc-2025.pdf \n was  uploaded ' }],
            ['button', { name: /foo/ }],
            ['button', { name: /foo/g }],
            ['button', { name: /foo/y }],
            ['button', { name: /let's/i }],
            ['button', { name: /let\'s/i }],
            ['button', { name: /'s/i }],
            ['button', { name: /\'s/i }],
            ['button', { name: /let['abc]s/i }],
            ['button', { name: /let\\'s/i }],
            ['button', { name: /let's let\'s/i }],
            ['button', { name: /let\'s let's/i }],
            ['button', { name: /let\\\'s/i }],
            ['button', { name: /let\\'s let\\\'s/i }],
            ['button', { name: /let\\\'s let\\'s/i }],
            ['button', { name: /let's/iu }],
            ['button', { checked: true, disabled: false, selected: true, expanded: false, includeHidden: true, level: 2, name: 'All', description: 'Desc', pressed: 'mixed', exact: true }],
        ];
        JSON.stringify(calls.map(([role, opts]) => {
            const full = getByRoleSelector(role, opts);
            if (!full.startsWith('internal:role='))
                throw new Error('unexpected prefix: ' + full);
            return full.slice('internal:role='.length);
        }));
    "#;
    eval_with_prelude(&Value::Null, script)
}

const ATTRIBUTE_SELECTOR_EXTRA_CASES: &[&str] = &[
    // Raw role=... bodies from selectors-role.spec.ts
    "button",
    "listbox",
    "combobox",
    "heading",
    "group",
    "dialog",
    "menuitem",
    "option[selected]",
    "option[selected=true]",
    "option[selected=false]",
    "checkbox[checked]",
    "checkbox[checked=true]",
    "checkbox[checked=false]",
    "checkbox[checked=\"mixed\"]",
    "checkbox[checked=mixed]",
    "button[pressed]",
    "button[pressed=true]",
    "button[pressed=false]",
    "button[pressed=\"mixed\"]",
    "treeitem[expanded]",
    "treeitem[expanded=true]",
    "treeitem[expanded=false]",
    "button[disabled]",
    "button[disabled=true]",
    "button[disabled=false]",
    "heading[level=1]",
    "heading[level=3]",
    "heading[level=5]",
    "button[include-hidden]",
    "button[include-hidden=true]",
    "button[include-hidden=false]",
    "button[name=\"Hello\"]",
    "button[name=\" \n Hello \"]",
    "button[name*=\"all\"]",
    "button[name=/^H[ae]llo$/]",
    "button[name=/h.*o/i]",
    "button[name=\"Hello\"][include-hidden]",
    "button[name=Hello]",
    "button[name=123][include-hidden]",
    // All 9 error bodies from selectors-role.spec.ts:509-536
    "[bar]",
    "foo[sElected]",
    "foo[bar . qux=true]",
    "heading[level=\"bar\"]",
    "checkbox[checked=\"bar\"]",
    "checkbox[checked~=true]",
    "button[level=3]",
    "button[name]",
    "treeitem[expanded=\"none\"]",
    // Dotted & quoted property paths
    "foo[a.b.c=1]",
    "foo['a b' . \"c \\\"d\\\"\" = \"val\"i]",
    "foo[\"foo.bar\".baz]",
    "foo[ 'k' . \"v\" = true ]",
    "foo[a . b]",
    "foo[\"\"=1]",
    "foo[a.=1]",
    "foo[.a=1]",
    // All 7 operators and case flags
    "foo[a]",
    "foo[a=\"x\"]",
    "foo[a*=\"x\"]",
    "foo[a|=\"x\"]",
    "foo[a^=\"x\"]",
    "foo[a$=\"x\"]",
    "foo[a~=\"x\"]",
    "foo[a=\"x\"i]",
    "foo[a=\"x\"I]",
    "foo[a=\"x\"s]",
    "foo[a=\"x\"S]",
    "foo[a='x' i]",
    "foo[a='x' s]",
    "foo[a=\"x\"  I]",
    "foo[a=x]",
    // Quoted string escaping (single-backslash strip, no \n unescaping)
    "foo[a=\"x\\\"y\"]",
    "foo[a='x\\'y']",
    "foo[a=\"a\\\\b\"]",
    "foo[a=\"a\\nb\"]",
    "foo[a=\"\"]",
    "foo[a='']",
    "foo[a=\"abc\\\\\"]",
    // Regex literals (character classes, escaped slashes/brackets, flags)
    "foo[a=/abc/]",
    "foo[a=/a\\/b/i]",
    "foo[a=/[a/b]/]",
    "foo[a=/[a\\]/b]/dgimsuy]",
    "foo[a=/[a\\]\\/b]/dgimsvy]",
    "foo[a=/a\\\\/]",
    "foo[a=//]",
    "foo[a=/Click me/v]",
    "foo[a=/click me/iv]",
    // ECMA-262 whitespace and non-ASCII CSS name chars
    " \t\n\r\u{000b}\u{000c}\u{00a0}\u{1680}\u{2000}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}button\u{feff}[\u{00a0}name\u{3000}=\u{2000}\"ok\"\u{2028}i\u{2029}]\u{00a0}",
    "foo\u{00a0}bar[x=1]",
    "foo\u{0085}bar[x=1]",
    // Numeric conversions (Allow vs Forbid)
    "foo[n=]",
    "foo[n=0]",
    "foo[n=-0]",
    "foo[n=+0]",
    "foo[n=42]",
    "foo[n=-3.14]",
    "foo[n=.5]",
    "foo[n=5.]",
    "foo[n=1e5]",
    "foo[n=1E-5]",
    "foo[n=1e+21]",
    "foo[n=0x10]",
    "foo[n=0Xff]",
    "foo[n=0o17]",
    "foo[n=0O10]",
    "foo[n=0b1010]",
    "foo[n=0B11]",
    "foo[n=Infinity]",
    "foo[n=-Infinity]",
    "foo[n=+Infinity]",
    "foo[n=0x20000000000001]",
    "foo[n=0x20000000000003]",
    "foo[n=1e500]",
    "foo[n=inf]",
    "foo[n=NaN]",
    "foo[n=infinity]",
    "foo[n=+0x10]",
    "foo[n=-0x10]",
    "foo[n=1_000]",
    "foo[n=abc]",
    // Non-`=` operators with non-string values
    "foo[n*=true]",
    "foo[n~=false]",
    "foo[n^=42]",
    "foo[n$=-0]",
    "foo[n|=Infinity]",
    "foo[n*=1e21]",
    "foo[n*=1e-7]",
    "foo[n*=0.000001]",
    "foo[n*=]",
    // Supplementary Unicode characters (UTF-16 wp tracking)
    "😀[a=\"b\"]",
    "foo[name=\"😀\" ?]",
    "😀[a?]",
    "foo[a 😀 \"b\"]",
    "foo[a 😀]",
    "foo[a😀",
    // Malformed selectors covering every error path
    "",
    "   ",
    "foo bar",
    "foo[a",
    "foo[]",
    "foo[a!]",
    "foo[a!=1]",
    "foo[a!",
    "foo[a=",
    "foo[a=\"unterminated",
    "foo[a=\"unterminated\\",
    "foo[a='unterminated",
    "foo[a=/unterminated",
    "foo[a=/unterminated\\",
    "foo[a=/[a]",
    "foo[a*=/abc/]",
    "foo[a~=/abc/]",
    "foo[a=/abc/ii]",
    "foo[a=/abc/uv]",
    "foo[a=/[/]",
    "foo[a=/abc/z]",
    "foo[a=\"x\"x]",
    "foo[a=\"x\" 1]",
    "foo[a=1 2]",
    "foo[a=1",
];

#[test]
fn attribute_selector_matches_quickjs_oracle() -> Result<(), Box<dyn Error>> {
    let mut selectors = generated_get_by_role_bodies()?;
    selectors.extend(
        ATTRIBUTE_SELECTOR_EXTRA_CASES
            .iter()
            .map(|item| (*item).to_owned()),
    );

    let mut cases: Vec<(String, bool)> = Vec::with_capacity(selectors.len() * 2);
    for selector in &selectors {
        cases.push((selector.clone(), true));
        cases.push((selector.clone(), false));
    }

    let oracle_script = r"
        const { parseAttributeSelector } = globalThis.__oracle;
        const buf = new ArrayBuffer(8);
        const view = new DataView(buf);
        function encodeValue(value) {
            if (value === null) return { kind: 'null' };
            if (typeof value === 'boolean') return { kind: 'bool', value };
            if (typeof value === 'number') {
                view.setFloat64(0, value, false);
                return { kind: 'number', bits: [view.getUint32(0, false), view.getUint32(4, false)] };
            }
            if (typeof value === 'string') return { kind: 'string', value };
            if (value instanceof RegExp) return { kind: 'regex', source: value.source, flags: value.flags };
            throw new Error('unexpected value type: ' + typeof value);
        }
        const cases = JSON.parse(input);
        JSON.stringify(cases.map(([selector, allow]) => {
            try {
                const parsed = parseAttributeSelector(selector, allow);
                return {
                    kind: 'ok',
                    name: parsed.name,
                    attributes: parsed.attributes.map(attr => ({
                        name: attr.name,
                        jsonPath: attr.jsonPath,
                        op: attr.op,
                        value: encodeValue(attr.value),
                        caseSensitive: attr.caseSensitive,
                    })),
                };
            } catch (e) {
                return { kind: 'err', message: e.message };
            }
        }));
    ";

    let js_outcomes: Vec<JsAttrOutcome> = eval_with_prelude(&json!(cases), oracle_script)?;
    assert_eq!(
        js_outcomes.len(),
        cases.len(),
        "QuickJS oracle must produce one outcome per (selector, allow) case"
    );

    for ((selector, allow), js_outcome) in cases.iter().zip(&js_outcomes) {
        let mode = if *allow {
            UnquotedStrings::Allow
        } else {
            UnquotedStrings::Forbid
        };
        assert_attribute_outcome_eq(
            selector,
            *allow,
            parse_attribute_selector(selector, mode),
            js_outcome,
        )?;
    }

    Ok(())
}

fn assert_attribute_outcome_eq(
    selector: &str,
    allow: bool,
    rust_outcome: Result<AttributeSelector, AttributeSelectorError>,
    js_outcome: &JsAttrOutcome,
) -> Result<(), Box<dyn Error>> {
    match (rust_outcome, js_outcome) {
        (
            Ok(parsed),
            JsAttrOutcome::Ok {
                name: js_name,
                attributes: js_attrs,
            },
        ) => {
            assert_eq!(
                &parsed.name, js_name,
                "selector {selector:?} (allow={allow}): name mismatch"
            );
            assert_eq!(
                parsed.attributes.len(),
                js_attrs.len(),
                "selector {selector:?} (allow={allow}): attribute count mismatch"
            );
            for (rust_attr, js_attr) in parsed.attributes.iter().zip(js_attrs) {
                assert_eq!(
                    rust_attr.name, js_attr.name,
                    "selector {selector:?} (allow={allow}): attr name mismatch"
                );
                assert_eq!(
                    rust_attr.json_path, js_attr.json_path,
                    "selector {selector:?} (allow={allow}): jsonPath mismatch"
                );
                assert_eq!(
                    rust_attr.op.as_str(),
                    js_attr.op,
                    "selector {selector:?} (allow={allow}): op mismatch"
                );
                assert_eq!(
                    rust_attr.case_sensitive, js_attr.case_sensitive,
                    "selector {selector:?} (allow={allow}): caseSensitive mismatch"
                );
                assert_attr_value_eq(selector, allow, &rust_attr.value, &js_attr.value)?;
            }
            Ok(())
        }
        (Err(rust_err), JsAttrOutcome::Err { message: js_msg }) => {
            let rust_msg = rust_err.to_string();
            let regex_prefix = format!("Error while parsing selector `{selector}`: ");
            if js_msg.starts_with(&regex_prefix) {
                assert!(
                    rust_msg.starts_with(&regex_prefix),
                    "selector {selector:?} (allow={allow}): regex error must start with {regex_prefix:?}, got {rust_msg:?}"
                );
            } else {
                assert_eq!(
                    rust_msg, *js_msg,
                    "selector {selector:?} (allow={allow}): error message must match upstream verbatim"
                );
            }
            Ok(())
        }
        (rust_side, js_side) => Err(format!(
            "selector {selector:?} (allow={allow}): outcome mismatch:\n  rust: {rust_side:?}\n  js:   {js_side:?}"
        )
        .into()),
    }
}

fn assert_attr_value_eq(
    selector: &str,
    allow: bool,
    rust_val: &AttributeValue,
    js_val: &JsAttrValue,
) -> Result<(), Box<dyn Error>> {
    match (rust_val, js_val) {
        (AttributeValue::Null, JsAttrValue::Null) => Ok(()),
        (AttributeValue::Bool(lhs), JsAttrValue::Bool { value: rhs }) => {
            assert_eq!(
                lhs, rhs,
                "selector {selector:?} (allow={allow}): bool value mismatch"
            );
            Ok(())
        }
        (AttributeValue::Number(lhs), JsAttrValue::Number { bits: rhs_bits }) => {
            let lhs_bits = number_bits(*lhs)?;
            assert_eq!(
                lhs_bits, *rhs_bits,
                "selector {selector:?} (allow={allow}): IEEE-754 float bits mismatch for {lhs}"
            );
            Ok(())
        }
        (AttributeValue::String(lhs), JsAttrValue::String { value: rhs }) => {
            assert_eq!(
                lhs, rhs,
                "selector {selector:?} (allow={allow}): string value mismatch"
            );
            Ok(())
        }
        (
            AttributeValue::RegExp(lhs),
            JsAttrValue::Regex {
                source: rhs_source,
                flags: rhs_flags,
            },
        ) => {
            // JS `new RegExp("", flags).source` reports `"(?:)"`, whereas `JsRegex::pattern()`
            // keeps the raw pattern `""` passed to `compile_with_flags`.
            let expected_js_source = if lhs.pattern().is_empty() {
                "(?:)"
            } else {
                lhs.pattern()
            };
            assert_eq!(
                expected_js_source, rhs_source,
                "selector {selector:?} (allow={allow}): regex source mismatch"
            );
            assert_eq!(
                lhs.flags(),
                rhs_flags,
                "selector {selector:?} (allow={allow}): regex flags mismatch"
            );
            Ok(())
        }
        _ => Err(format!(
            "selector {selector:?} (allow={allow}): value kind mismatch: rust={rust_val:?}, js={js_val:?}"
        )
        .into()),
    }
}

const ROLE_QUERY_EXTRA_BODIES: &[&str] = &[
    // All 9 error cases from selectors-role.spec.ts:509-536
    "[bar]",
    "foo[sElected]",
    "foo[bar . qux=true]",
    "heading[level=\"bar\"]",
    "checkbox[checked=\"bar\"]",
    "checkbox[checked~=true]",
    "button[level=3]",
    "button[name]",
    "treeitem[expanded=\"none\"]",
    // Case folding and empty role
    "BUTTON[name=\"Save\"s]",
    "Heading[level=2]",
    "[name=\"x\"]",
    "",
    // Every supported role for checked, pressed, selected, expanded, level
    "checkbox[checked]",
    "menuitemcheckbox[checked=false]",
    "menuitemradio[checked=mixed]",
    "option[checked=\"mixed\"]",
    "radio[checked=true]",
    "switch[checked=\"mixed\"i]",
    "treeitem[checked=false]",
    "button[pressed]",
    "button[pressed=mixed]",
    "button[pressed=false]",
    "gridcell[selected]",
    "option[selected=false]",
    "row[selected=true]",
    "tab[selected]",
    "rowheader[selected=false]",
    "columnheader[selected=true]",
    "treeitem[selected]",
    "application[expanded]",
    "button[expanded=false]",
    "checkbox[expanded=true]",
    "combobox[expanded]",
    "gridcell[expanded=false]",
    "link[expanded=true]",
    "listbox[expanded]",
    "menuitem[expanded=false]",
    "row[expanded=true]",
    "rowheader[expanded]",
    "tab[expanded=false]",
    "treeitem[expanded=true]",
    "columnheader[expanded]",
    "menuitemcheckbox[expanded=false]",
    "menuitemradio[expanded=true]",
    "switch[expanded]",
    "heading[level=1]",
    "listitem[level=2]",
    "row[level=3]",
    "treeitem[level=4]",
    // Unsupported role on every role-restricted attribute (including expanded's duplicate rowheader)
    "button[checked]",
    "link[pressed]",
    "button[selected]",
    "heading[expanded]",
    "option[level=1]",
    // Validation precedence: role before value before operator
    "button[checked*=\"nope\"]",
    "checkbox[checked*=\"nope\"]",
    "checkbox[checked*=\"mixed\"]",
    "checkbox[checked^=\"mixed\"]",
    "checkbox[checked$=\"mixed\"]",
    "checkbox[checked|=\"mixed\"]",
    "checkbox[checked~=\"mixed\"]",
    "button[pressed*=\"mixed\"]",
    // Invalid values for tristates and booleans
    "checkbox[checked=\"MIXED\"i]",
    "checkbox[checked=\"true\"]",
    "checkbox[checked=/mixed/]",
    "button[pressed=\"true\"]",
    "button[pressed=\"nope\"]",
    "option[selected=\"true\"]",
    "option[selected=mixed]",
    "treeitem[expanded=\"true\"]",
    "treeitem[expanded=/true/]",
    "button[disabled=\"true\"]",
    "button[disabled=mixed]",
    "button[include-hidden=\"true\"]",
    "button[include-hidden=mixed]",
    // Level edge cases
    "heading[level=\"\"]",
    "heading[level=]",
    "heading[level=\" 3 \"]",
    "heading[level=\"0x10\"]",
    "heading[level=Infinity]",
    "heading[level=-Infinity]",
    "heading[level=-0]",
    "heading[level=3.5]",
    "heading[level]",
    "heading[level=true]",
    "heading[level=false]",
    "heading[level=/3/]",
    "heading[level*=3]",
    "heading[level*=\"3\"]",
    "heading[level=\"NaN\"]",
    "heading[level=\"inf\"]",
    // Name and description valid & invalid forms
    "button[name=\"a\"][name*=\"b\"i]",
    "button[name^=\"pre\"s]",
    "button[name$=\"suf\"i]",
    "button[name|=\"lang\"]",
    "button[name~=\"word\"i]",
    "button[name=unquoted]",
    "button[name=\"true\"]",
    "button[description=\"a\"][description=/b/iv]",
    "button[description*=\"sub\"i]",
    "button[description]",
    "button[name=true]",
    "button[name=false]",
    "button[description=true]",
    "button[description=false]",
    // Unknown attributes & syntax errors
    "button[unknown=\"1\"]",
    "button[foo=/bar/ii]",
];

#[test]
fn role_query_validation_matches_quickjs_oracle() -> Result<(), Box<dyn Error>> {
    let mut bodies = generated_get_by_role_bodies()?;
    bodies.extend(
        ROLE_QUERY_EXTRA_BODIES
            .iter()
            .map(|item| (*item).to_owned()),
    );

    let script = format!(
        "{ROLE_VALIDATION_ORACLE_JS}\n\
        const buf = new ArrayBuffer(8);\n\
        const view = new DataView(buf);\n\
        function encodeText(val, op, exact) {{\n\
            if (val === undefined) return null;\n\
            if (val instanceof RegExp) return {{ kind: 'regex', source: val.source, flags: val.flags }};\n\
            return {{ kind: 'string', value: val, op, exact }};\n\
        }}\n\
        function encodeLevel(level) {{\n\
            if (level === undefined) return null;\n\
            view.setFloat64(0, level, false);\n\
            return [view.getUint32(0, false), view.getUint32(4, false)];\n\
        }}\n\
        const bodies = JSON.parse(input);\n\
        JSON.stringify(bodies.map(body => {{\n\
            try {{\n\
                const opts = validateRoleSelector(body);\n\
                return {{\n\
                    kind: 'ok',\n\
                    role: opts.role,\n\
                    name: encodeText(opts.name, opts.nameOp, opts.nameExact),\n\
                    description: encodeText(opts.description, opts.descriptionOp, opts.descriptionExact),\n\
                    checked: opts.checked === undefined ? null : opts.checked,\n\
                    pressed: opts.pressed === undefined ? null : opts.pressed,\n\
                    selected: opts.selected === undefined ? null : opts.selected,\n\
                    expanded: opts.expanded === undefined ? null : opts.expanded,\n\
                    levelBits: encodeLevel(opts.level),\n\
                    disabled: opts.disabled === undefined ? null : opts.disabled,\n\
                    includeHidden: opts.includeHidden === undefined ? null : opts.includeHidden,\n\
                }};\n\
            }} catch (e) {{\n\
                return {{ kind: 'err', message: e.message }};\n\
            }}\n\
        }}));"
    );

    let js_outcomes: Vec<JsRoleOutcome> = eval_with_prelude(&json!(bodies), &script)?;
    assert_eq!(
        js_outcomes.len(),
        bodies.len(),
        "QuickJS oracle must produce one role outcome per body"
    );

    for (body, js_outcome) in bodies.iter().zip(&js_outcomes) {
        assert_role_outcome_eq(body, RoleQuery::parse(body), js_outcome)?;
    }

    let expanded_err = RoleQuery::parse("heading[expanded]")
        .err()
        .ok_or("heading[expanded] must be rejected")?;
    assert!(
        expanded_err
            .to_string()
            .contains("\"rowheader\", \"rowheader\""),
        "expanded error must preserve upstream's duplicate rowheader entry verbatim"
    );

    let numeric_parsed = parse_attribute_selector("heading[level=3]", UnquotedStrings::Forbid)?;
    let query = RoleQuery::from_attribute_selector(&numeric_parsed)?;
    assert_eq!(
        query.level(),
        Some(3.0),
        "from_attribute_selector must accept AttributeValue::Number for level"
    );

    let numeric_name = parse_attribute_selector("button[name=3]", UnquotedStrings::Forbid)?;
    assert_eq!(
        RoleQuery::from_attribute_selector(&numeric_name),
        Err(RoleQueryError::InvalidTextValue { attribute: "name" }),
        "from_attribute_selector must reject numeric AttributeValue for name"
    );

    let bad_regex_op = AttributeSelector {
        name: "button".to_owned(),
        attributes: vec![AttributeSelectorPart {
            name: "name".to_owned(),
            json_path: vec!["name".to_owned()],
            op: AttributeSelectorOperator::Contains,
            value: AttributeValue::RegExp(JsRegex::compile("ok")?),
            case_sensitive: true,
        }],
    };
    assert_eq!(
        RoleQuery::from_attribute_selector(&bad_regex_op),
        Err(RoleQueryError::UnsupportedOperator {
            attribute: "name",
            op: AttributeSelectorOperator::Contains,
        }),
        "from_attribute_selector must reject non-`=` operators paired with RegExp values"
    );

    Ok(())
}

fn assert_role_outcome_eq(
    body: &str,
    rust_outcome: Result<RoleQuery, RoleQueryError>,
    js_outcome: &JsRoleOutcome,
) -> Result<(), Box<dyn Error>> {
    match (rust_outcome, js_outcome) {
        (Ok(query), JsRoleOutcome::Ok(js_opts)) => {
            assert_eq!(query.role(), js_opts.role, "body {body:?}: role mismatch");
            assert_text_filter_eq(body, "name", query.name(), js_opts.name.as_ref())?;
            assert_text_filter_eq(
                body,
                "description",
                query.description(),
                js_opts.description.as_ref(),
            )?;
            assert_tristate_eq(body, "checked", query.checked(), js_opts.checked.as_ref())?;
            assert_tristate_eq(body, "pressed", query.pressed(), js_opts.pressed.as_ref())?;
            assert_eq!(
                query.selected(),
                js_opts.selected,
                "body {body:?}: selected mismatch"
            );
            assert_eq!(
                query.expanded(),
                js_opts.expanded,
                "body {body:?}: expanded mismatch"
            );
            assert_eq!(
                query.disabled(),
                js_opts.disabled,
                "body {body:?}: disabled mismatch"
            );
            assert_eq!(
                query.include_hidden(),
                js_opts.include_hidden,
                "body {body:?}: include-hidden mismatch"
            );
            let rust_level_bits = query.level().map(number_bits).transpose()?;
            assert_eq!(
                rust_level_bits, js_opts.level_bits,
                "body {body:?}: level IEEE-754 bits mismatch"
            );
            Ok(())
        }
        (Err(rust_err), JsRoleOutcome::Err { message: js_msg }) => {
            let rust_msg = rust_err.to_string();
            let regex_prefix = format!("Error while parsing selector `{body}`: ");
            if js_msg.starts_with(&regex_prefix) {
                assert!(
                    rust_msg.starts_with(&regex_prefix),
                    "body {body:?}: regex error must preserve upstream prefix"
                );
            } else {
                assert_eq!(
                    rust_msg, *js_msg,
                    "body {body:?}: RoleQuery error message must match upstream byte for byte"
                );
            }
            Ok(())
        }
        (rust_side, js_side) => Err(format!(
            "body {body:?}: RoleQuery outcome mismatch:\n  rust: {rust_side:?}\n  js:   {js_side:?}"
        )
        .into()),
    }
}

fn assert_tristate_eq(
    body: &str,
    field: &str,
    rust_val: Option<Tristate>,
    js_val: Option<&Value>,
) -> Result<(), Box<dyn Error>> {
    let expected = match js_val {
        None | Some(Value::Null) => None,
        Some(Value::Bool(true)) => Some(Tristate::True),
        Some(Value::Bool(false)) => Some(Tristate::False),
        Some(Value::String(text)) if text == "mixed" => Some(Tristate::Mixed),
        other => return Err(format!("unexpected JS tristate {other:?} for {body:?}").into()),
    };
    assert_eq!(
        rust_val, expected,
        "body {body:?}: {field} tristate mismatch"
    );
    Ok(())
}

fn assert_text_filter_eq(
    body: &str,
    field: &str,
    rust_val: Option<&TextFilter>,
    js_val: Option<&JsTextFilter>,
) -> Result<(), Box<dyn Error>> {
    match (rust_val, js_val) {
        (None, None) => Ok(()),
        (
            Some(TextFilter::String { value, op, exact }),
            Some(JsTextFilter::String {
                value: js_value,
                op: js_op,
                exact: js_exact,
            }),
        ) => {
            let op_str = match op {
                StringMatchOperator::Eq => "=",
                StringMatchOperator::Contains => "*=",
                StringMatchOperator::DashMatch => "|=",
                StringMatchOperator::Prefix => "^=",
                StringMatchOperator::Suffix => "$=",
                StringMatchOperator::Word => "~=",
            };
            assert_eq!(value, js_value, "body {body:?}: {field} string mismatch");
            assert_eq!(op_str, js_op, "body {body:?}: {field} operator mismatch");
            assert_eq!(exact, js_exact, "body {body:?}: {field} exact mismatch");
            Ok(())
        }
        (Some(TextFilter::Regex(regex)), Some(JsTextFilter::Regex { source, flags })) => {
            let expected_js_source = if regex.pattern().is_empty() {
                "(?:)"
            } else {
                regex.pattern()
            };
            assert_eq!(
                expected_js_source, source,
                "body {body:?}: {field} regex source mismatch"
            );
            assert_eq!(
                regex.flags(),
                flags,
                "body {body:?}: {field} regex flags mismatch"
            );
            Ok(())
        }
        _ => Err(format!(
            "body {body:?}: {field} filter kind mismatch: rust={rust_val:?}, js={js_val:?}"
        )
        .into()),
    }
}

const CORPUS_REGEX_CASES: &[(&str, &str, &[&str])] = &[
    // selectors-role.spec.ts
    ("^H[ae]llo$", "", &["Hello", "Hallo", "Hillo", " Hello "]),
    ("h.*o", "i", &["Hello", "Hallo", "world", "HO"]),
    // selectors-get-by.spec.ts
    ("Click me", "v", &["Click me", "click me", "Submit"]),
    ("click me", "iv", &["Click me", "CLICK ME", "Submit"]),
    ("Missing", "v", &["Click me", "Missing"]),
    (
        r"report-\d+",
        "",
        &[
            "File doc-2025.pdf was uploaded successfully",
            "File report-2026.pdf was uploaded successfully",
        ],
    ),
    (
        "uploaded successfully$",
        "",
        &[
            "File doc-2025.pdf was uploaded successfully",
            "File demo.doc has an invalid file format",
        ],
    ),
    ("foo", "", &["foo", "bar foo", "baz"]),
    ("foo", "g", &["foo", "bar foo", "baz"]),
    ("foo", "y", &["foo", "foo bar", "bar foo", "baz"]),
    ("He[l]*o", "", &["Hello", "Heo", "Hi"]),
    ("ye", "", &["yo", "ya", "ye"]),
    ("e", "", &["yo", "ye"]),
    (
        r"Last\s+name",
        "i",
        &["Last Name", "last   name", "LastName"],
    ),
    ("last name", "", &["Last Name", "last name"]),
    ("wor", "i", &["Hello", "Hello World"]),
    ("foo >> bar", "", &["foo >> bar", "foo > bar"]),
    // expect-to-have-accessible.spec.ts
    (r"ell\w", "", &["Hello", "ell", "ell1"]),
    ("hello", "", &["Hello", "hello"]),
    ("hello", "i", &["Hello", "hello", "world"]),
    (
        "first error message.",
        "i",
        &["First error message. Second error message.", "nope"],
    ),
    (
        "second error message.",
        "i",
        &["First error message. Second error message.", "nope"],
    ),
    (
        "This should not be considered.",
        "i",
        &[
            "First error message. Second error message.",
            "This should not be considered.",
        ],
    ),
    ("button|checkbox", "", &["button", "checkbox", "link"]),
    // locator-query.spec.ts (all 14 quote-escape patterns + special symbol patterns)
    ("Foo.*", "", &["Foobar", "Bar"]),
    (r#"Hello "world""#, "", &[r#"Hello "world""#, "Hello world"]),
    (
        "let's",
        "i",
        &["let's let'shello", r"let\'s let\'shello", "let's hello"],
    ),
    (r"let\'s", "i", &["let's let'shello", r"let\'s let\'shello"]),
    ("'s", "i", &["let's let'shello", "nope"]),
    (r"\'s", "i", &["let's let'shello", r"let\'s"]),
    ("let['abc]s", "i", &["let's", "letas", "letbs", "letds"]),
    (
        r"let\\'s",
        "i",
        &["let's let'shello", r"let\'s let\'shello"],
    ),
    (
        r"let's let\'s",
        "i",
        &["let's let'shello", r"let\'s let\'shello"],
    ),
    (
        r"let\'s let's",
        "i",
        &["let's let'shello", r"let\'s let\'shello"],
    ),
    (
        r"let\\\'s",
        "i",
        &["let's let'shello", r"let\'s let\'shello"],
    ),
    (
        r"let\\'s let\\\'s",
        "i",
        &["let's let'shello", r"let\'s let\'shello"],
    ),
    (
        r"let\\\'s let\\'s",
        "i",
        &["let's let'shello", r"let\'s let\'shello"],
    ),
    ("let's", "iu", &["let's hello", "LET'S HELLO", r"let\'s"]),
    (
        r#"hElLo "world""#,
        "i",
        &[r#"Hello "world""#, "Hello world"],
    ),
    ("^title text$", "i", &["Title Text", "Title  Text"]),
    (
        r#"^first\/".*"second\\$"#,
        "si",
        &["First/\"and\"\nSecond\\", "First/\"and\"Second\\"],
    ),
    ("bar|foo", "", &["foo", "bar", "baz"]),
    // Sticky, global, multiline, dotAll, hasIndices combinations
    ("foo", "gy", &["foo", "foo bar", "bar foo"]),
    ("foo", "my", &["foo\nbar", "bar\nfoo"]),
    (
        "^foo",
        "m",
        &["bar\nfoo", "bar\rfoo", "bar\u{2028}foo", "bar\u{2029}foo"],
    ),
    ("^foo", "my", &["foo\nbar", "bar\nfoo"]),
    (
        "a.b",
        "",
        &["a\nb", "a\rb", "a\u{2028}b", "a\u{2029}b", "axb"],
    ),
    (
        "a.b",
        "s",
        &["a\nb", "a\rb", "a\u{2028}b", "a\u{2029}b", "axb"],
    ),
    ("hello", "dgimsuy", &["HELLO world", "say HELLO"]),
    ("hello", "dgimsvy", &["HELLO world", "say HELLO"]),
    // Unflagged UCS-2 vs /u
    ("^.$", "", &["a", "😀"]),
    ("^..$", "", &["ab", "😀"]),
    ("^.$", "u", &["a", "😀"]),
    ("^..$", "u", &["ab", "😀"]),
    (r"\u{1F600}", "u", &["😀", "a"]),
    (r"\p{Letter}+", "u", &["Café", "123"]),
    // Lookahead, lookbehind, backreferences, named groups
    (r"foo(?=bar)", "", &["foobar", "foobaz"]),
    (r"foo(?!bar)", "", &["foobar", "foobaz"]),
    (r"(?<=foo)bar", "", &["foobar", "bazbar"]),
    (r"(?<!foo)bar", "", &["foobar", "bazbar"]),
    (r"(\w+)\s+\1", "", &["echo echo", "echo delta"]),
    (r"(?<w>\w+)\s+\k<w>", "u", &["hi hi", "hi bye"]),
];

#[test]
fn js_regex_matches_quickjs_on_corpus_and_edge_cases() -> Result<(), Box<dyn Error>> {
    let escaped_roundtrip_script = r"
        const { escapeRegexForSelector } = globalThis.__oracle;
        const regexes = [
            /let's/i, /let\'s/i, /'s/i, /\'s/i, /let['abc]s/i, /let\\'s/i,
            /let's let\'s/i, /let\'s let's/i, /let\\\'s/i, /let\\'s let\\\'s/i,
            /let\\\'s let\\'s/i, /let's/iu, /foo >> bar/, /Click me/v, /click me/iv,
        ];
        JSON.stringify(regexes.map(re => ({
            escaped: escapeRegexForSelector(re),
            source: re.source,
            flags: re.flags,
        })));
    ";
    let escaped_regexes: Vec<EscapedRegex> =
        eval_with_prelude(&Value::Null, escaped_roundtrip_script)?;

    let mut pairs: Vec<(String, String, String)> = Vec::new();
    for &(pattern, flags, haystacks) in CORPUS_REGEX_CASES {
        for &text in haystacks {
            pairs.push((pattern.to_owned(), flags.to_owned(), text.to_owned()));
        }
    }
    for entry in &escaped_regexes {
        let body = format!("button[name={}]", entry.escaped);
        let parsed = parse_attribute_selector(&body, UnquotedStrings::Allow)?;
        let Some(AttributeValue::RegExp(re)) = parsed.attributes.first().map(|a| &a.value) else {
            return Err(format!("expected regex attribute for {body:?}").into());
        };
        assert_eq!(
            re.flags(),
            entry.flags,
            "escaped regex {body:?} flags must round-trip"
        );
        for sample in [
            "let's let'shello",
            r"let\'s let\'shello",
            "let's hello",
            "foo >> bar",
            "Click me",
            "CLICK ME",
        ] {
            pairs.push((
                re.pattern().to_owned(),
                re.flags().to_owned(),
                sample.to_owned(),
            ));
            pairs.push((entry.source.clone(), entry.flags.clone(), sample.to_owned()));
        }
    }

    let js_results: Vec<bool> = eval_quickjs(
        &json!(pairs),
        r"JSON.stringify(JSON.parse(input).map(([pattern, flags, text]) => {
            const re = new RegExp(pattern, flags);
            re.lastIndex = 0;
            return Boolean(text.match(re));
        }))",
    )?;
    assert_eq!(
        js_results.len(),
        pairs.len(),
        "QuickJS must return one boolean verdict per regex/text pair"
    );

    for ((pattern, flags, text), &js_match) in pairs.iter().zip(&js_results) {
        let compiled = JsRegex::compile_with_flags(pattern, flags)?;
        assert_eq!(
            compiled.is_match(text),
            js_match,
            "regress diverged from QuickJS on /{pattern}/{flags} against {text:?}"
        );
    }

    assert_invalid_regexes_rejected()?;
    Ok(())
}

fn assert_invalid_regexes_rejected() -> Result<(), Box<dyn Error>> {
    let invalid_cases: &[(&str, &str)] = &[
        ("abc", "uv"),
        ("abc", "vu"),
        ("abc", "ii"),
        ("abc", "gg"),
        ("abc", "dd"),
        ("abc", "z"),
        ("abc", "I"),
        ("[", ""),
        ("(", ""),
        ("*", ""),
        (r"\a", "u"),
    ];
    let js_invalid: Vec<bool> = eval_quickjs(
        &json!(invalid_cases),
        r"JSON.stringify(JSON.parse(input).map(([pattern, flags]) => {
            try { new RegExp(pattern, flags); return false; } catch { return true; }
        }))",
    )?;
    for (&(pattern, flags), &js_failed) in invalid_cases.iter().zip(&js_invalid) {
        assert!(
            js_failed,
            "QuickJS was expected to reject /{pattern}/{flags}"
        );
        assert!(
            JsRegex::compile_with_flags(pattern, flags).is_err(),
            "JsRegex must reject /{pattern}/{flags} just like QuickJS"
        );
    }
    Ok(())
}

#[test]
fn js_regex_matches_es2024_v_flag_unicode_mode_semantics() -> Result<(), Box<dyn Error>> {
    // ES2024 specifies that `/v` (`unicodeSets`) implies full Unicode mode (code-point matching,
    // `\u{...}`, `\p{...}`, class-set subtraction/intersection, strict identity escapes, and
    // reserved class-set characters such as unescaped `/`), matching Chromium's V8. Bundled
    // QuickJS-NG 0.12.1 only implements `LRE_FLAG_UNICODE_SETS` as a partial stub in
    // `libregexp.c` without `is_unicode` or set operators, so these V8/ES2024 invariants are
    // asserted directly.
    let v_cases: &[(&str, &str, &str, bool)] = &[
        ("^.$", "v", "😀", true),
        ("^..$", "v", "😀", false),
        (r"\u{1F600}", "v", "😀", true),
        (r"\u{1F600}", "v", "a", false),
        (r"\p{ASCII_Hex_Digit}+", "v", "deadBEEF", true),
        (r"\p{ASCII_Hex_Digit}+", "v", "xyz", false),
        (r"[[a-z]--[aeiou]]", "v", "b", true),
        (r"[[a-z]--[aeiou]]", "v", "e", false),
        (r"[[a-z]--[aeiou]]", "v", "A", false),
        (r"[[a-z]&&[aeiou]]", "v", "a", true),
        (r"[[a-z]&&[aeiou]]", "v", "b", false),
        (r"[[a-z]--[aeiou]]", "iv", "B", true),
        (r"[[a-z]--[aeiou]]", "iv", "E", false),
    ];
    for &(pattern, flags, text, expected) in v_cases {
        assert_eq!(
            JsRegex::compile_with_flags(pattern, flags)?.is_match(text),
            expected,
            "/{pattern}/{flags} against {text:?} must match ES2024/V8 semantics"
        );
    }
    assert!(
        JsRegex::compile_with_flags(r"\a", "v").is_err(),
        "/v must reject non-syntax identity escape \\a (matches V8)"
    );
    assert!(
        parse_attribute_selector(r"foo[a=/[a\]/b]/v]", UnquotedStrings::Allow).is_err(),
        "/v must reject unescaped '/' inside a character class (matches V8)"
    );
    Ok(())
}

const PRELUDE_SELECTOR_CORPUS: &[&str] = &[
    "button",
    "div > span.primary:has-text(\"Pay\")",
    "css:light=button",
    "internal:role=button[name=\"Pay\"i]",
    "role=heading[level=2]",
    "aria-ref=e5",
    "aria-ref=f2e10",
    "aria-template={\"kind\":\"role\",\"role\":\"button\"}",
    "div >> internal:role=button >> nth=0",
    "div >> internal:role=button >> nth=-1",
    "internal:text=\"Hello\"i",
    "//button[@id='ok']",
    "..",
    "*css=div >> span",
    "div >> *internal:role=button >> nth=0",
    "div >> internal:has=\"internal:role=button[name=\\\"Save\\\"s]\"",
    "div >> internal:has-not=\"span.error\"",
    "internal:role=button >> internal:and=\"css=.primary\"",
    "internal:role=button >> internal:or=\"internal:role=link\"",
    "div >> internal:chain=\"internal:role=row >> internal:role=cell\"",
    "section >> internal:has=\"ul >> internal:has-not=\\\"li.disabled\\\" >> internal:and=\\\"internal:role=listitem\\\"\"",
    "input >> left-of=\"label\"",
    "input >> left-of=\"label\", 50",
    "button >> right-of=\"input\", 12.5",
    "button >> above=\"footer\"",
    "button >> below=\"header\", 100",
    "button >> near=\"span\", 25",
    // Frame-split selectors with nested frame-prefix stripping and capture adjustment
    "iframe#f >> internal:control=enter-frame >> tr >> internal:has=\"iframe#f >> internal:control=enter-frame >> internal:role=button[name=\\\"Dana\\\"i]\"",
    "iframe#f >> internal:control=enter-frame >> body >> internal:chain=\"iframe#f >> internal:control=enter-frame >> *css=section >> span\"",
];

#[test]
fn parsed_selector_roundtrips_real_prelude_chunks() -> Result<(), Box<dyn Error>> {
    let chunks_by_selector: Vec<Vec<Value>> = eval_with_prelude(
        &json!(PRELUDE_SELECTOR_CORPUS),
        r"JSON.stringify(JSON.parse(input).map(s => globalThis.__oracle.splitSelectorByFrame(s).chunks))",
    )?;
    assert_eq!(
        chunks_by_selector.len(),
        PRELUDE_SELECTOR_CORPUS.len(),
        "prelude splitSelectorByFrame must return one chunk list per selector"
    );

    let mut roundtripped_pairs: Vec<(String, Value)> = Vec::new();
    for (selector_text, chunks) in PRELUDE_SELECTOR_CORPUS.iter().zip(&chunks_by_selector) {
        assert!(
            !chunks.is_empty(),
            "splitSelectorByFrame for {selector_text:?} must emit at least one chunk"
        );
        for chunk_value in chunks {
            let chunk_json = serde_json::to_string(chunk_value)?;
            let from_str: ParsedSelector = serde_json::from_str(&chunk_json)?;
            let from_val: ParsedSelector = serde_json::from_value(chunk_value.clone())?;
            assert_eq!(
                from_str, from_val,
                "decoding from &str and Value must yield equal ParsedSelector for {selector_text:?}"
            );
            verify_body_kinds(&from_str)?;

            let reserialized = serde_json::to_string(&from_str)?;
            let reparsed: ParsedSelector = serde_json::from_str(&reserialized)?;
            assert_eq!(
                reparsed, from_str,
                "re-serialized ParsedSelector must round-trip identically for {selector_text:?}"
            );
            roundtripped_pairs.push((reserialized, chunk_value.clone()));
        }
    }

    assert_js_deep_json_equal(&roundtripped_pairs)?;

    let frame_capture_chunks = chunks_by_selector
        .last()
        .ok_or("missing last frame-capture test selector")?;
    assert_eq!(
        frame_capture_chunks.len(),
        2,
        "iframe enter-frame selector must split into two chunks"
    );
    let inner_chunk: ParsedSelector = serde_json::from_value(frame_capture_chunks[1].clone())?;
    let Some(SelectorBody::Nested(NestedSelectorBody { parsed, distance })) =
        inner_chunk.parts.get(1).map(|part| &part.body)
    else {
        return Err("expected NestedSelectorBody for internal:chain".into());
    };
    assert_eq!(
        parsed.capture,
        Some(0),
        "capture index must shift to 0 after stripping the shared iframe prefix"
    );
    assert_eq!(
        *distance, None,
        "internal:chain must not carry a layout distance"
    );
    Ok(())
}

fn assert_js_deep_json_equal(pairs: &[(String, Value)]) -> Result<(), Box<dyn Error>> {
    let js_equal: Vec<bool> = eval_quickjs(
        &json!(pairs),
        r"function deepEq(a, b) {
            if (a === b) return true;
            if (typeof a !== typeof b || a === null || b === null) return false;
            if (Array.isArray(a)) {
                return Array.isArray(b) && a.length === b.length && a.every((v, i) => deepEq(v, b[i]));
            }
            if (typeof a === 'object') {
                const ka = Object.keys(a).sort();
                const kb = Object.keys(b).sort();
                return ka.length === kb.length && ka.every((k, i) => k === kb[i] && deepEq(a[k], b[k]));
            }
            return false;
        }
        JSON.stringify(JSON.parse(input).map(([rustJson, jsChunk]) => deepEq(JSON.parse(rustJson), jsChunk)))",
    )?;
    for (index, equal) in js_equal.iter().enumerate() {
        assert!(
            *equal,
            "round-tripped chunk #{index} must equal prelude JSON in JS semantics"
        );
    }
    Ok(())
}

#[test]
fn parsed_selector_accepts_leading_nested_and_rejects_malformed_shapes()
-> Result<(), Box<dyn Error>> {
    let leading_nested: ParsedSelector = serde_json::from_value(json!({
        "parts": [{
            "name": "internal:has",
            "source": "\"button\"",
            "body": {
                "parsed": {
                    "parts": [{ "name": "internal:role", "body": "button", "source": "button" }]
                }
            }
        }]
    }))?;
    assert_eq!(
        leading_nested.parts.len(),
        1,
        "deserialization must not reject a nested combinator at parts[0]"
    );

    let role_part = json!({ "name": "role", "body": "button", "source": "button" });
    let malformed: Vec<(&str, Value)> = vec![
        ("empty parts", json!({ "parts": [] })),
        (
            "unknown top-level field",
            json!({ "parts": [role_part.clone()], "extra": 1 }),
        ),
        (
            "unknown part field",
            json!({ "parts": [{ "name": "role", "body": "button", "source": "button", "extra": true }] }),
        ),
        (
            "unknown nested body field",
            json!({ "parts": [{
                "name": "internal:has",
                "source": "\"button\"",
                "body": { "parsed": { "parts": [role_part.clone()] }, "extra": 1 }
            }] }),
        ),
        (
            "capture equal to parts.len()",
            json!({ "parts": [role_part.clone()], "capture": 1 }),
        ),
        (
            "capture greater than parts.len()",
            json!({ "parts": [role_part.clone()], "capture": 5 }),
        ),
        (
            "negative capture",
            json!({ "parts": [role_part.clone()], "capture": -1 }),
        ),
        (
            "css with string body",
            json!({ "parts": [{ "name": "css", "body": "button", "source": "button" }] }),
        ),
        (
            "css with object body",
            json!({ "parts": [{ "name": "css", "body": {}, "source": "button" }] }),
        ),
        (
            "text part with array body",
            json!({ "parts": [{ "name": "internal:role", "body": [], "source": "button" }] }),
        ),
        (
            "text part with object body",
            json!({ "parts": [{ "name": "aria-ref", "body": {}, "source": "e5" }] }),
        ),
        (
            "nested part with string body",
            json!({ "parts": [{ "name": "internal:has", "body": "button", "source": "button" }] }),
        ),
        (
            "distance on internal:has",
            nested_with_distance("internal:has", &role_part),
        ),
        (
            "distance on internal:has-not",
            nested_with_distance("internal:has-not", &role_part),
        ),
        (
            "distance on internal:and",
            nested_with_distance("internal:and", &role_part),
        ),
        (
            "distance on internal:or",
            nested_with_distance("internal:or", &role_part),
        ),
        (
            "distance on internal:chain",
            nested_with_distance("internal:chain", &role_part),
        ),
    ];

    for (label, value) in malformed {
        assert!(
            serde_json::from_value::<ParsedSelector>(value).is_err(),
            "malformed ParsedSelector ({label}) must be rejected"
        );
    }

    Ok(())
}

fn nested_with_distance(name: &str, inner_part: &Value) -> Value {
    json!({
        "parts": [{
            "name": name,
            "source": "\"button\"",
            "body": {
                "parsed": { "parts": [inner_part.clone()] },
                "distance": 10
            }
        }]
    })
}

fn verify_body_kinds(selector: &ParsedSelector) -> Result<(), Box<dyn Error>> {
    assert!(
        !selector.parts.is_empty(),
        "valid ParsedSelector must have non-empty parts"
    );
    if let Some(capture) = selector.capture {
        assert!(
            capture < selector.parts.len(),
            "capture index must be within parts.len()"
        );
    }
    for part in &selector.parts {
        if part.name == "css" {
            assert!(
                matches!(part.body, SelectorBody::Css(_)),
                "css part must decode into SelectorBody::Css"
            );
        } else if NESTED_SELECTOR_NAMES.contains(&part.name.as_str()) {
            let SelectorBody::Nested(nested) = &part.body else {
                return Err(format!("nested part {} did not decode into Nested", part.name).into());
            };
            if nested.distance.is_some() {
                assert!(
                    LAYOUT_SELECTOR_NAMES.contains(&part.name.as_str()),
                    "only layout parts may carry a distance"
                );
            }
            verify_body_kinds(&nested.parsed)?;
        } else {
            assert!(
                matches!(part.body, SelectorBody::Text(_)),
                "non-css, non-nested part {} must decode into SelectorBody::Text",
                part.name
            );
        }
    }
    Ok(())
}

#[test]
fn vendored_playwright_sources_match_pinned_sha256() -> Result<(), Box<dyn Error>> {
    let vendor_root = neo_playwright_dir().join("third_party/playwright");
    let sums = fs::read_to_string(vendor_root.join("SHA256SUMS"))?;

    for &(rel_path, expected_sha) in PINNED_SOURCES {
        let expected_line = format!("{expected_sha}  {rel_path}");
        assert!(
            sums.lines().any(|line| line == expected_line),
            "SHA256SUMS must record {expected_line:?}; update select/ port if upstream changed"
        );

        let bytes = fs::read(vendor_root.join(rel_path))?;
        let digest = Sha256::digest(&bytes);
        let mut actual_sha = String::with_capacity(64);
        for byte in digest {
            write!(actual_sha, "{byte:02x}")?;
        }
        assert_eq!(
            actual_sha, expected_sha,
            "vendored {rel_path} bytes drifted from pinned SHA-256"
        );
    }

    Ok(())
}

#[test]
fn attribute_selector_operator_display_matches_as_str() {
    let all = [
        (AttributeSelectorOperator::Truthy, "<truthy>"),
        (AttributeSelectorOperator::Eq, "="),
        (AttributeSelectorOperator::Contains, "*="),
        (AttributeSelectorOperator::DashMatch, "|="),
        (AttributeSelectorOperator::Prefix, "^="),
        (AttributeSelectorOperator::Suffix, "$="),
        (AttributeSelectorOperator::Word, "~="),
    ];
    for (op, expected) in all {
        assert_eq!(
            op.as_str(),
            expected,
            "as_str must match Playwright's operator spelling"
        );
        assert_eq!(
            op.to_string(),
            expected,
            "Display must match as_str for operator {expected}"
        );
    }
}
