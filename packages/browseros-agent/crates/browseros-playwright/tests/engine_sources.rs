//! Source guards over the vendored Playwright engine and Neo's page-side code: nothing reaches
//! another frame, every `.value` read in the engine is reviewed, the vendored bytes are what
//! `sync.sh` recorded, only the probe names engine internals, and the snapshot's name rule
//! uses Playwright's own list of name-prohibited roles.
//!
//! They read sources as text on purpose. Each guards an assumption the sealed world rests on
//! (sdk-privacy D3 and D4, design D5) that no behavioral test could see break: an upgrade or
//! an edit that falsifies one fails here, at review time, instead of leaking in a page.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use browseros_core::sensitive_fields::FRAME_CROSSING_TOKENS;
use sha2::{Digest, Sha256};

type TestResult = Result<(), Box<dyn Error>>;

/// From this crate's manifest directory.
const VENDORED: &str = "third_party/playwright";
const JS_PACKAGE: &str = "../../packages/browseros-playwright-js";

fn crate_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Every file below `dir`, sorted.
fn files_below(dir: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// `path` below `base`, with `/` between parts, as messages and the tables here name files.
fn relative(path: &Path, base: &Path) -> String {
    let below = path.strip_prefix(base).unwrap_or(path);
    below
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// The TypeScript files below `dir` (relative to `base`) with their text.
fn typescript_below(dir: &Path, base: &Path) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let mut sources = Vec::new();
    for path in files_below(dir)? {
        if path.extension().is_some_and(|extension| extension == "ts") {
            sources.push((relative(&path, base), fs::read_to_string(&path)?));
        }
    }
    Ok(sources)
}

/// The lines of `text` that hold `token`, as `path:line`.
fn lines_holding(path: &str, text: &str, token: &str) -> Vec<String> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.contains(token))
        .map(|(index, _)| format!("{path}:{}: {token}", index + 1))
        .collect()
}

/// Design D5's precise patterns: what the vendored engine would need to reach another frame's
/// document. The vendored code is full of locals named `parent` and fields named `top`, so the
/// bare tokens of core's list would flag correct code there (D5 measured 7 such hits).
const VENDORED_FRAME_CROSSING: &[&str] = &[
    "contentDocument",
    "contentWindow",
    "frameElement",
    "window.parent",
    "window.top",
    "window.frames",
    "self.parent",
];

/// The seal nulls `contentDocument` and `contentWindow` in the world it seals, but prototypes
/// are per frame: code that reached a child frame's document would read it through getters
/// that frame's seal may not have replaced yet (sdk-privacy D4). Neo hops frames in Rust and
/// seals each one itself, so no page-side source may cross. Neo's own page code (the probe,
/// and the stand-ins the engine build swaps in) meets core's full list.
#[test]
fn tests_that_no_page_side_source_reaches_another_frame() -> TestResult {
    let root = crate_root();
    let vendored = typescript_below(&root.join(VENDORED).join("packages"), &root.join(VENDORED))?;
    let js_package = root.join(JS_PACKAGE);
    let mut neo = typescript_below(&js_package.join("src/probe"), &js_package)?;
    neo.extend(typescript_below(
        &js_package.join("src/stubs"),
        &js_package,
    )?);
    assert!(
        vendored.len() >= 23 && !neo.is_empty(),
        "the engine's files and the probe were found"
    );
    let mut hits = Vec::new();
    for (path, text) in &vendored {
        for token in VENDORED_FRAME_CROSSING {
            hits.extend(lines_holding(path, text, token));
        }
    }
    for (path, text) in &neo {
        for token in FRAME_CROSSING_TOKENS {
            hits.extend(lines_holding(path, text, token));
        }
    }
    assert!(
        hits.is_empty(),
        "page-side code that crosses frames: {hits:#?}"
    );
    Ok(())
}

/// Why one `.value` read in the vendored page engine is safe in the sealed world.
#[derive(Clone, Copy, Debug)]
enum ValueRead {
    /// Not a form control: a parsed selector's attribute, a CSS token, an out-parameter, an
    /// option spec the agent passed, or a matcher's name.
    NotAControl,
    /// A control's value through the seal's getter: a sensitive field reads `••••`, an empty
    /// one stays empty (`seal.js` masks only non-empty values).
    Sealed,
    /// An `<option>`'s value: page markup, which the seal leaves as it is; never typed text.
    OptionMarkup,
    /// `fill`'s read-back on color, date, time, month, range and week inputs, types that are
    /// never sensitive (sdk-privacy D3).
    NeverSensitive,
    /// An `expect` value matcher's received value. For a sensitive field the probe compares
    /// through `__clawSeal.valueEquals` (a value attribute through `attributeEquals`) instead
    /// and prints the masked value (design D14).
    ProbeCompares,
    /// `selectText` on a textarea measures the value, and a sensitive one's masked length is
    /// not its own: sdk-privacy D3 has the probe select such a field by the seal's raw length.
    RawLength,
    /// Writes a value and reads none.
    Write,
}

/// Every line of the vendored page engine (`packages/injected/src`) that reads `.value`, as
/// reviewed against sdk-privacy's audit (D3: "a test checks every `.value` read in the
/// vendored source against this list"). A line upstream adds or changes fails the test until
/// it is reviewed here; a line that goes away fails it until its entry goes too.
const ENGINE_VALUE_READS: &[(&str, &str, ValueRead)] = &[
    (
        "roleUtils.ts",
        "const delimIndex = tokens.findIndex(token => token instanceof css.DelimToken && token.value === '/');",
        ValueRead::NotAControl,
    ),
    (
        "roleUtils.ts",
        "accumulated.push(tokens[index].value as string);",
        ValueRead::NotAControl,
    ),
    (
        "roleUtils.ts",
        "} else if (index + 2 < tokens.length && tokens[index] instanceof css.FunctionToken && tokens[index].value === 'attr' && tokens[index + 1] instanceof css.IdentToken && tokens[index + 2] instanceof css.CloseParenToken) {",
        ValueRead::NotAControl,
    ),
    (
        "roleUtils.ts",
        "const attrName = tokens[index + 1].value as string;",
        ValueRead::NotAControl,
    ),
    (
        "roleUtils.ts",
        "return { text: asFlatString(result.text), elements: result.elements, derivedFromContent: outDerivedFromContent.value };",
        ValueRead::NotAControl,
    ),
    (
        "roleUtils.ts",
        "options.outDerivedFromContent.value = true;",
        ValueRead::NotAControl,
    ),
    (
        "roleUtils.ts",
        "options.outDerivedFromContent.value = true;",
        ValueRead::NotAControl,
    ),
    // An embedded textbox's value in a label's accessible name.
    (
        "roleUtils.ts",
        "return compositeString((element as HTMLInputElement | HTMLTextAreaElement).value, element, options.collectElements);",
        ValueRead::Sealed,
    ),
    // A combobox input with no selected option names itself by its value.
    (
        "roleUtils.ts",
        "return compositeString((element as HTMLInputElement).value, element, options.collectElements);",
        ValueRead::Sealed,
    ),
    // A button, submit or reset input's name.
    (
        "roleUtils.ts",
        "const value = (element as HTMLInputElement).value || '';",
        ValueRead::Sealed,
    ),
    (
        "roleSelectorEngine.ts",
        "if (attr.op !== '<truthy>' && !values.includes(attr.value))",
        ValueRead::NotAControl,
    ),
    (
        "roleSelectorEngine.ts",
        "options.checked = attr.op === '<truthy>' ? true : attr.value;",
        ValueRead::NotAControl,
    ),
    (
        "roleSelectorEngine.ts",
        "options.pressed = attr.op === '<truthy>' ? true : attr.value;",
        ValueRead::NotAControl,
    ),
    (
        "roleSelectorEngine.ts",
        "options.selected = attr.op === '<truthy>' ? true : attr.value;",
        ValueRead::NotAControl,
    ),
    (
        "roleSelectorEngine.ts",
        "options.expanded = attr.op === '<truthy>' ? true : attr.value;",
        ValueRead::NotAControl,
    ),
    (
        "roleSelectorEngine.ts",
        "if (typeof attr.value === 'string')",
        ValueRead::NotAControl,
    ),
    (
        "roleSelectorEngine.ts",
        "attr.value = +attr.value;",
        ValueRead::NotAControl,
    ),
    (
        "roleSelectorEngine.ts",
        "if (attr.op !== '=' || typeof attr.value !== 'number' || Number.isNaN(attr.value))",
        ValueRead::NotAControl,
    ),
    (
        "roleSelectorEngine.ts",
        "options.level = attr.value;",
        ValueRead::NotAControl,
    ),
    (
        "roleSelectorEngine.ts",
        "options.disabled = attr.op === '<truthy>' ? true : attr.value;",
        ValueRead::NotAControl,
    ),
    (
        "roleSelectorEngine.ts",
        "if (typeof attr.value !== 'string' && !(attr.value instanceof RegExp))",
        ValueRead::NotAControl,
    ),
    (
        "roleSelectorEngine.ts",
        "options.name = attr.value;",
        ValueRead::NotAControl,
    ),
    (
        "roleSelectorEngine.ts",
        "if (typeof attr.value !== 'string' && !(attr.value instanceof RegExp))",
        ValueRead::NotAControl,
    ),
    (
        "roleSelectorEngine.ts",
        "options.description = attr.value;",
        ValueRead::NotAControl,
    ),
    (
        "roleSelectorEngine.ts",
        "options.includeHidden = attr.op === '<truthy>' ? true : attr.value;",
        ValueRead::NotAControl,
    ),
    // selectOptions matches the agent's choice against each option.
    (
        "injectedScript.ts",
        "matches = matches && (optionToSelect.valueOrLabel === option.value || matchesLabel(optionToSelect.valueOrLabel));",
        ValueRead::OptionMarkup,
    ),
    (
        "injectedScript.ts",
        "if (optionToSelect.value !== undefined)",
        ValueRead::NotAControl,
    ),
    (
        "injectedScript.ts",
        "matches = matches && optionToSelect.value === option.value;",
        ValueRead::OptionMarkup,
    ),
    (
        "injectedScript.ts",
        "select.value = undefined as any;",
        ValueRead::Write,
    ),
    // selectOptions returns the values of the options it selected.
    (
        "injectedScript.ts",
        "return selectedOptions.map(option => option.value);",
        ValueRead::OptionMarkup,
    ),
    (
        "injectedScript.ts",
        "input.value = value;",
        ValueRead::Write,
    ),
    (
        "injectedScript.ts",
        "if (input.value !== value)",
        ValueRead::NeverSensitive,
    ),
    (
        "injectedScript.ts",
        "textarea.selectionEnd = textarea.value.length;",
        ValueRead::RawLength,
    ),
    // `to.be.empty`: emptiness survives the mask.
    (
        "injectedScript.ts",
        "const value = (element as HTMLInputElement).value;",
        ValueRead::Sealed,
    ),
    // `to.have.values`.
    (
        "injectedScript.ts",
        "const received = [...(element as HTMLSelectElement).selectedOptions].map(o => o.value);",
        ValueRead::ProbeCompares,
    ),
    (
        "injectedScript.ts",
        "if (expression === 'to.have.attribute.value') {",
        ValueRead::NotAControl,
    ),
    (
        "injectedScript.ts",
        "} else if (expression === 'to.have.value') {",
        ValueRead::NotAControl,
    ),
    (
        "injectedScript.ts",
        "received = (element as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement).value;",
        ValueRead::ProbeCompares,
    ),
    // A textbox's value in the aria snapshot.
    (
        "ariaSnapshot.ts",
        "result.children = [element.value];",
        ValueRead::Sealed,
    ),
    (
        "selectorUtils.ts",
        "const attrValue = typeof attr.value === 'string' && !attr.caseSensitive ? attr.value.toUpperCase() : attr.value;",
        ValueRead::NotAControl,
    ),
    // A button, submit or reset input's text, for text matching.
    (
        "selectorUtils.ts",
        "value = { full: root.value, normalized: normalizeWhiteSpace(root.value), immediate: [root.value] };",
        ValueRead::Sealed,
    ),
];

/// The other vendored files, which never touch a DOM element: how many of their lines read a
/// `.value`, and what those values are. A changed count fails the test until it is reviewed.
const OTHER_VALUE_READS: &[(&str, usize, &str)] = &[
    (
        "packages/isomorphic/ariaSnapshot.ts",
        22,
        "YAML scalars of an aria template",
    ),
    ("packages/isomorphic/cssParser.ts", 9, "CSS tokens"),
    ("packages/isomorphic/cssTokenizer.ts", 45, "CSS tokens"),
    (
        "packages/isomorphic/locatorGenerators.ts",
        5,
        "parsed selector attributes",
    ),
    (
        "packages/isomorphic/utilityScriptSerializers.ts",
        3,
        "map, set and wrapper entries being serialized",
    ),
];

/// Whether `line` reads a property named exactly `value` (not `valueOrLabel`).
fn reads_value(line: &str) -> bool {
    line.split(".value")
        .skip(1)
        .any(|rest| !rest.starts_with(|next: char| next.is_alphanumeric() || next == '_'))
}

#[test]
fn tests_that_every_value_read_in_the_vendored_engine_is_reviewed() -> TestResult {
    let vendored = crate_root().join(VENDORED);
    let engine = vendored.join("packages/injected/src");
    let mut found = Vec::new();
    for (path, text) in typescript_below(&engine, &engine)? {
        for line in text.lines().filter(|line| reads_value(line)) {
            found.push((path.clone(), line.trim().to_owned()));
        }
    }
    let mut reviewed: Vec<(String, String)> = ENGINE_VALUE_READS
        .iter()
        .map(|(path, line, _)| ((*path).to_owned(), (*line).to_owned()))
        .collect();
    found.sort();
    reviewed.sort();
    let unreviewed: Vec<&(String, String)> = found
        .iter()
        .filter(|read| !reviewed.contains(read))
        .collect();
    let gone: Vec<&(String, String)> = reviewed
        .iter()
        .filter(|read| !found.contains(read))
        .collect();
    assert!(
        unreviewed.is_empty(),
        "review these engine reads of `.value`: {unreviewed:#?}"
    );
    assert!(
        gone.is_empty(),
        "no longer in the engine, drop their entries: {gone:#?}"
    );
    assert_eq!(
        found, reviewed,
        "each reviewed line occurs as often as it is listed"
    );

    let mut counts = BTreeMap::new();
    for (path, text) in typescript_below(&vendored.join("packages"), &vendored)? {
        let count = text.lines().filter(|line| reads_value(line)).count();
        if count > 0 && !path.starts_with("packages/injected/") {
            counts.insert(path, count);
        }
    }
    let expected: BTreeMap<String, usize> = OTHER_VALUE_READS
        .iter()
        .map(|(path, count, _)| ((*path).to_owned(), *count))
        .collect();
    assert_eq!(
        counts, expected,
        "review the `.value` reads of the files that changed"
    );
    Ok(())
}

/// `sync.sh` copies each file in its `FILES` list from an upstream tag and writes `SHA256SUMS`.
/// Every vendored file must be on that list, in the manifest, and byte for byte as copied:
/// the sealed world is reviewed as upstream's code, so a local edit must not slip in.
#[test]
fn tests_that_the_vendored_files_are_the_bytes_sync_sh_recorded() -> TestResult {
    vendored_as_synced(VENDORED)
}

/// The same for the `mime` tables the prelude types payloads with, which must stay the bytes
/// of the npm release Playwright bundles.
#[test]
fn tests_that_the_vendored_mime_files_are_the_bytes_sync_sh_recorded() -> TestResult {
    vendored_as_synced("third_party/mime")
}

/// The manifest check of one vendored tree (`dir`, from the crate's root) with its own
/// `sync.sh` and `SHA256SUMS`.
fn vendored_as_synced(dir: &str) -> TestResult {
    let vendored = crate_root().join(dir);
    let mut manifest = BTreeMap::new();
    for line in fs::read_to_string(vendored.join("SHA256SUMS"))?.lines() {
        let Some((hash, path)) = line.split_once("  ") else {
            return Err(format!(
                "{dir}/SHA256SUMS has a line that is not `<hash>  <path>`: {line}"
            )
            .into());
        };
        manifest.insert(path.to_owned(), hash.to_owned());
    }
    let script = fs::read_to_string(vendored.join("sync.sh"))?;
    let Some((_, list)) = script.split_once("FILES=(") else {
        return Err(format!("{dir}/sync.sh has no FILES list").into());
    };
    // A path with a space (upstream's `file to upload.txt`) is single-quoted in the bash array.
    let mut synced: Vec<&str> = list
        .lines()
        .map(|line| line.trim().trim_matches('\''))
        .take_while(|line| *line != ")")
        .filter(|line| !line.is_empty())
        .collect();
    synced.sort_unstable();
    let listed: Vec<&str> = manifest.keys().map(String::as_str).collect();
    assert_eq!(
        listed, synced,
        "SHA256SUMS lists exactly the files sync.sh copies"
    );

    let ours = ["README.md", "SHA256SUMS", "VERSION", "sync.sh"];
    let on_disk: Vec<String> = files_below(&vendored)?
        .iter()
        .map(|path| relative(path, &vendored))
        .filter(|path| !ours.contains(&path.as_str()))
        .collect();
    assert_eq!(
        on_disk, listed,
        "nothing is vendored that sync.sh did not copy"
    );

    let mut edited = Vec::new();
    for (path, hash) in &manifest {
        let mut actual = String::new();
        for byte in Sha256::digest(fs::read(vendored.join(path))?) {
            write!(actual, "{byte:02x}")?;
        }
        if actual != *hash {
            edited.push(path);
        }
    }
    assert!(
        edited.is_empty(),
        "changed after sync.sh copied them: {edited:?}"
    );
    Ok(())
}

/// Neo files outside the probe that may name an engine internal, and why.
const INTERNALS_ALLOWED: &[(&str, &str, &str)] = &[(
    "packages/browseros-playwright-js/tests/aria-ref.test.ts",
    "_ariaRef",
    "plays the engine: it sets the expando an ai snapshot leaves, to see the probe's re-key delete it",
)];

/// The `_name` tokens in `text` read as properties or string keys (`injected._engines`,
/// `'_ariaRef'`), which is how code reaches an engine internal; comment lines are skipped.
fn accessed_underscore_names(text: &str) -> BTreeSet<&str> {
    let mut names = BTreeSet::new();
    for line in text.lines() {
        let code = line.trim_start();
        if code.starts_with("//") || code.starts_with('*') || code.starts_with("/*") {
            continue;
        }
        for (index, _) in line.match_indices(['.', '\'', '"', '`', '[']) {
            let rest = line.get(index + 1..).unwrap_or_default();
            let name = rest
                .split(|next: char| !(next.is_alphanumeric() || next == '_'))
                .next()
                .unwrap_or_default();
            if name.starts_with('_') && name.chars().nth(1).is_some_and(char::is_alphabetic) {
                names.insert(name);
            }
        }
    }
    names
}

/// The `_name`s the vendored page engine uses: `InjectedScript`'s private members, its
/// evaluator's and generator's, and the `_ariaRef` expando on elements. None is API; any
/// release may rename one.
fn engine_internals() -> Result<BTreeSet<String>, Box<dyn Error>> {
    let engine = crate_root().join(VENDORED).join("packages/injected/src");
    let mut internals = BTreeSet::new();
    for (_, text) in typescript_below(&engine, &engine)? {
        for token in text.split(|next: char| !(next.is_alphanumeric() || next == '_')) {
            if token.starts_with('_') && token.chars().nth(1).is_some_and(char::is_alphabetic) {
                internals.insert(token.to_owned());
            }
        }
    }
    Ok(internals)
}

/// Only the probe (`packages/browseros-playwright-js/src/probe`) may name the engine's
/// internals, so an upgrade that renames one breaks one place, whose tests see it. The guard
/// reads Neo's other TypeScript, its scripts, and the crate's Rust, which could embed page
/// code in a string.
#[test]
fn tests_that_only_the_probe_names_engine_internals() -> TestResult {
    let internals = engine_internals()?;
    for expected in ["_engines", "_lastAriaSnapshotForQuery", "_ariaRef"] {
        assert!(
            internals.contains(expected),
            "the engine still uses {expected}"
        );
    }
    let agent_root = crate_root().join("../..");
    let js_package = crate_root().join(JS_PACKAGE);
    let mut sources = Vec::new();
    for dir in ["src", "scripts", "tests"] {
        for path in files_below(&js_package.join(dir))? {
            let code = path
                .extension()
                .is_some_and(|extension| ["ts", "mjs", "js"].iter().any(|ok| extension == *ok));
            if code && !path.starts_with(js_package.join("src/probe")) {
                sources.push(path);
            }
        }
    }
    for dir in ["src", "tests"] {
        for path in files_below(&crate_root().join(dir))? {
            let rust = path.extension().is_some_and(|extension| extension == "rs");
            if rust && !path.ends_with("tests/engine_sources.rs") {
                sources.push(path);
            }
        }
    }
    let mut named = Vec::new();
    let mut used = BTreeSet::new();
    for path in &sources {
        let name = relative(path, &agent_root);
        let name = name.trim_start_matches("crates/browseros-playwright/../../");
        let text = fs::read_to_string(path)?;
        for internal in accessed_underscore_names(&text) {
            if !internals.contains(internal) {
                continue;
            }
            match INTERNALS_ALLOWED
                .iter()
                .position(|(file, allowed, _)| name.ends_with(file) && *allowed == internal)
            {
                Some(entry) => {
                    used.insert(entry);
                }
                None => named.push(format!("{name}: {internal}")),
            }
        }
    }
    assert!(
        sources.len() > 10,
        "Neo's sources outside the probe were found"
    );
    assert!(
        named.is_empty(),
        "engine internals outside the probe: {named:#?}"
    );
    let stale: Vec<&str> = INTERNALS_ALLOWED
        .iter()
        .enumerate()
        .filter(|(entry, _)| !used.contains(entry))
        .map(|(_, (file, _, _))| *file)
        .collect();
    assert!(
        stale.is_empty(),
        "these exceptions are no longer needed: {stale:?}"
    );
    Ok(())
}

/// The quoted strings between `open` and `close` in `text`.
fn quoted_list<'a>(text: &'a str, open: &str, close: &str) -> Vec<&'a str> {
    let Some((_, rest)) = text.split_once(open) else {
        return Vec::new();
    };
    let Some((list, _)) = rest.split_once(close) else {
        return Vec::new();
    };
    list.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| item.trim_matches(|quote| quote == '\'' || quote == '"'))
        .collect()
}

/// The snapshot's name rule acts on exactly the roles whose names Playwright's own accessible
/// name code refuses (roleUtils' `elementProhibitsNaming`, ARIA's "name from: prohibited").
#[test]
fn tests_that_the_name_rule_uses_playwrights_list_of_name_prohibited_roles() -> TestResult {
    let role_utils = fs::read_to_string(
        crate_root()
            .join(VENDORED)
            .join("packages/injected/src/roleUtils.ts"),
    )?;
    let upstream = quoted_list(&role_utils, "const elementProhibitsNaming = [", "]");
    let rules = fs::read_to_string(crate_root().join("src/snapshot/snapshot_rules.rs"))?;
    let ours = quoted_list(
        &rules,
        "pub(crate) const NAME_PROHIBITED_ROLES: &[&str] = &[",
        "];",
    );
    assert!(!upstream.is_empty(), "roleUtils.ts still lists the roles");
    assert_eq!(ours, upstream, "NAME_PROHIBITED_ROLES is roleUtils' list");
    Ok(())
}
