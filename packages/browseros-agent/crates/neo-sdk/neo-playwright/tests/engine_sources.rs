//! Source guards over the vendored Playwright engine and Neo's page-side code: the vendored
//! bytes are what `sync.sh` recorded, only the probe names engine internals, and the
//! snapshot's name rule uses Playwright's own list of name-prohibited roles.
//!
//! They read sources as text on purpose. Each guards an assumption the engine's world rests on
//! that no behavioral test could see break: an upgrade or an edit that falsifies one fails
//! here, at review time, instead of in a page.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

type TestResult = Result<(), Box<dyn Error>>;

/// From this crate's manifest directory.
const VENDORED: &str = "third_party/playwright";
const JS_PACKAGE: &str = "../neo-playwright-js";

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

/// `sync.sh` copies each file in its `FILES` list from an upstream tag and writes `SHA256SUMS`.
/// Every vendored file must be on that list, in the manifest, and byte for byte as copied:
/// the isolated world is reviewed as upstream's code, so a local edit must not slip in.
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
    "crates/neo-sdk/neo-playwright-js/tests/aria-ref.test.ts",
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

/// Only the probe (`crates/neo-sdk/neo-playwright-js/src/probe`) may name the engine's
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
    // Resolve the sibling's `..` before matching agent-root-relative allowlist paths.
    let agent_root = crate_root().join("../../..").canonicalize()?;
    let js_package = crate_root().join(JS_PACKAGE).canonicalize()?;
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
