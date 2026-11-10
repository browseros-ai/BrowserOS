//! The gate's own tests, over `tests/fixtures/provenance/`: a tree shaped like `crates/neo-sdk/`, with
//! fake Playwright sources in Playwright's formatting and fake neo sources that use every kind
//! of line. The tree is clean; each test copies it, breaks one rule, and checks that a problem
//! names the file, the line and the fix.

use std::env;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use crate::playwright;
use crate::rules::{ENFORCED, Gate, Problem, REGISTER};

const CLICK: &str = "neo-runtime/src/click.rs";
const OWN: &str = "neo-runtime/src/own.rs";
const GLUE: &str = "neo-playwright-injected/src/glue.ts";
const LOCATOR: &str = "neo-quickjs-injected/src/locator.ts";

fn fixture_root() -> PathBuf {
    PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default())
        .join("tests/fixtures/provenance")
}

/// A scratch copy of the fixture tree.
struct Tree(TempDir);

impl Tree {
    fn new() -> Self {
        let dir = TempDir::new().unwrap_or_else(|error| panic!("a temporary directory: {error}"));
        let root = fixture_root();
        let files =
            playwright::walk(&root).unwrap_or_else(|error| panic!("the fixture reads: {error}"));
        for relative in files {
            let target = dir.path().join(&relative);
            let copied = target
                .parent()
                .map_or(Ok(()), fs::create_dir_all)
                .and_then(|()| fs::copy(root.join(&relative), &target));
            copied.unwrap_or_else(|error| panic!("{relative} copies: {error}"));
        }
        Self(dir)
    }

    fn text(&self, file: &str) -> String {
        fs::read_to_string(self.0.path().join(file))
            .unwrap_or_else(|error| panic!("{file} reads: {error}"))
    }

    /// Replaces the one occurrence of `from` in `file`.
    fn edit(&self, file: &str, from: &str, to: &str) {
        let text = self.text(file);
        assert_eq!(text.matches(from).count(), 1, "{file} holds `{from}` once");
        fs::write(self.0.path().join(file), text.replacen(from, to, 1))
            .unwrap_or_else(|error| panic!("{file} writes: {error}"));
    }

    /// The 1-based line of `file` that holds `needle`.
    fn line(&self, file: &str, needle: &str) -> usize {
        let text = self.text(file);
        let index = text.lines().position(|line| line.contains(needle));
        index.unwrap_or_else(|| panic!("{file} holds `{needle}`")) + 1
    }

    fn enforce(&self, file: &str) {
        self.edit(
            ENFORCED,
            "neo-runtime/src/click.rs\n",
            &format!("neo-runtime/src/click.rs\n{file}\n"),
        );
    }

    fn problems(&self) -> Vec<String> {
        let gate = Gate::load(self.0.path())
            .unwrap_or_else(|error| panic!("the fixture tree loads: {error}"));
        let mut problems = gate.source_problems();
        let vendored = gate.vendor_problems();
        problems
            .extend(vendored.unwrap_or_else(|error| panic!("the fixture's trees read: {error}")));
        problems.extend(gate.register_problems());
        problems.iter().map(Problem::show).collect()
    }
}

#[track_caller]
fn assert_flags(problems: &[String], file: &str, line: usize, fragments: &[&str]) {
    let location = if line == 0 {
        format!("crates/neo-sdk/{file}  ")
    } else {
        format!("crates/neo-sdk/{file}:{line}  ")
    };
    let found = problems.iter().any(|problem| {
        problem.starts_with(&location)
            && fragments.iter().all(|fragment| problem.contains(fragment))
    });
    assert!(
        found,
        "no problem at {location}mentioning {fragments:?}; the gate found:\n{}",
        problems.join("\n")
    );
}

/// `NEO_RUNTIME_WRITE_FIXTURES=1` regenerates the fixture's register, as it does the real one.
#[test]
fn the_fixture_tree_is_clean() {
    if env::var_os("NEO_RUNTIME_WRITE_FIXTURES").is_some() {
        let gate = Gate::load(&fixture_root()).expect("the fixture tree loads");
        gate.write_register().expect("the fixture register writes");
    }
    let problems = Tree::new().problems();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_register_lists_each_deviation_and_question_by_item() {
    let tree = Tree::new();
    let register = Gate::load(tree.0.path())
        .expect("the fixture tree loads")
        .register();
    for row in [
        "| neo-runtime/src/click.rs | `MAX_CLICK_COUNT` | `server/frames.ts` `Frame.click` | \
         Playwright has no cap; a script's `clickCount: 1e9` must not take the server's memory \
         with it. |",
        "| neo-runtime/src/click.rs | `click` | `server/frames.ts` `Frame.click` | The fixture \
         tries once and never retries. |",
        "| neo-quickjs-injected/src/locator.ts | (file) | `server/frames.ts` `Frame.ariaSnapshot` | Absent: \
         the fixture serves no snapshots. |",
        "| neo-runtime/src/click.rs | `MEMORY_LIMIT_BYTES` | why the realm gets 64 MiB; no \
         measurement is recorded. |",
        "| neo-quickjs-injected/src/locator.ts | (file) | why the fixture's locator keeps no timeout of its \
         own. |",
        "## Deviations (3)",
        "## Unexplained: owner questions (2)",
    ] {
        assert!(
            register.contains(row),
            "the register has `{row}`:\n{register}"
        );
    }
}

#[test]
fn a_citation_of_an_unvendored_file_fails() {
    let tree = Tree::new();
    tree.edit(
        CLICK,
        "/// Playwright: `server/frames.ts` `Frame`\n",
        "/// Playwright: `server/frame.ts` `Frame`\n",
    );
    let line = tree.line(CLICK, "`server/frame.ts`");
    assert_flags(
        &tree.problems(),
        CLICK,
        line,
        &["`server/frame.ts` is not vendored", "--ignored sync"],
    );
}

#[test]
fn a_symbol_the_file_does_not_define_fails() {
    let tree = Tree::new();
    tree.edit(
        CLICK,
        "`Frame.click`: `const waitTime",
        "`Frame.clik`: `const waitTime",
    );
    let line = tree.line(CLICK, "`Frame.clik`");
    assert_flags(
        &tree.problems(),
        CLICK,
        line,
        &[
            "`server/frames.ts` `Frame.clik` does not resolve: no `clik` there",
            "cite a symbol the file defines",
        ],
    );
}

/// `Frame.expect` holds a local `const ariaSnapshot = …`; `Frame.ariaSnapshot` is the method
/// one level inside the class, lines 34-37, so a quote from inside that local's own block does
/// not resolve against it.
#[test]
fn a_quote_from_another_member_fails() {
    let tree = Tree::new();
    tree.edit(
        CLICK,
        "`return { snapshot: renderAriaSnapshotAsYaml(snapshot) };`",
        "`return injected.ariaSnapshotForExpectFailure(injected.document.body, { mode: 'default' });`",
    );
    let line = tree.line(CLICK, "`Frame.ariaSnapshot`");
    assert_flags(
        &tree.problems(),
        CLICK,
        line,
        &[
            "quote not in `server/frames.ts` `Frame.ariaSnapshot` (lines 34-37 at 26137face)",
            "copy Playwright's code exactly",
        ],
    );
}

#[test]
fn a_constant_that_differs_from_its_quote_fails() {
    let tree = Tree::new();
    tree.edit(
        CLICK,
        "[u64; 5] = [0, 20, 100, 100, 500];",
        "[u64; 5] = [0, 20, 100, 100, 600];",
    );
    tree.edit(CLICK, "\"null\", \"~\",", "\"null\", \"nil\",");
    tree.edit(
        GLUE,
        "const WAITS = [0, 20, 100, 100, 500]",
        "const WAITS = [0, 20, 100, 100, 50]",
    );
    let problems = tree.problems();
    assert_flags(
        &problems,
        CLICK,
        tree.line(CLICK, "const LOOP_B_WAITS_MS"),
        &[
            "`LOOP_B_WAITS_MS` is [0, 20, 100, 100, 600]; its quote says [0, 20, 100, 100, 500]",
            "tag a value Neo chose `Deviation:`",
        ],
    );
    assert_flags(
        &problems,
        CLICK,
        tree.line(CLICK, "const YAML_WORDS"),
        &["`YAML_WORDS` is [", "\"nil\"]; its quote says ["],
    );
    assert_flags(
        &problems,
        GLUE,
        tree.line(GLUE, "const WAITS"),
        &["`WAITS` is [0, 20, 100, 100, 50]"],
    );
}

#[test]
fn a_one_line_playwright_constant_quotes_its_value() {
    let tree = Tree::new();
    tree.edit(
        CLICK,
        "`Frame.click`: `const waitTime = [0, 20, 100, 100, 500];`",
        "`Frame.click`",
    );
    assert_flags(
        &tree.problems(),
        CLICK,
        tree.line(CLICK, "const LOOP_B_WAITS_MS"),
        &[
            "`LOOP_B_WAITS_MS` is tagged `Playwright:` without a quote",
            "`<symbol>`: `<code>`",
        ],
    );
}

#[test]
fn a_message_playwright_never_sends_fails() {
    let tree = Tree::new();
    tree.edit(
        CLICK,
        "PlaywrightError::error(\"Element is not visible\")",
        "PlaywrightError::error(\"Element is hidden\")",
    );
    tree.edit(
        LOCATOR,
        "new Error('Element is not visible')",
        "new Error('Element is gone')",
    );
    let problems = tree.problems();
    assert_flags(
        &problems,
        CLICK,
        tree.line(CLICK, "Element is hidden"),
        &[
            "\"Element is hidden\" matches no message of",
            "`server/frames.ts` `Frame.click`",
            "`// Deviation: …`",
        ],
    );
    assert_flags(
        &problems,
        LOCATOR,
        tree.line(LOCATOR, "Element is gone"),
        &["\"Element is gone\" matches no message of"],
    );
}

#[test]
fn a_deviation_on_the_line_lets_a_message_differ() {
    let tree = Tree::new();
    tree.edit(
        CLICK,
        "    // Deviation: `server/frames.ts` `Frame.click`: `for (let retry = 0; ; ++retry) {`\n    // The fixture tries once and never retries.\n",
        "",
    );
    assert_flags(
        &tree.problems(),
        CLICK,
        tree.line(CLICK, "tried once"),
        &["\"tried once, without retrying\" matches no message of"],
    );
}

/// `attempting click action${options.trial ? ' (trial run)' : ''}` matches a Rust `{}` and
/// either branch spelled out, and nothing else there.
#[test]
fn a_ternary_placeholder_matches_only_its_branches() {
    let tree = Tree::new();
    tree.edit(
        CLICK,
        "\"attempting click action (trial run)\"",
        "\"attempting click action (dry run)\"",
    );
    let problems = tree.problems();
    assert_flags(
        &problems,
        CLICK,
        tree.line(CLICK, "(dry run)"),
        &["\"attempting click action (dry run)\" matches no message of"],
    );
    let line = tree.line(CLICK, "attempting click action{trial}");
    assert!(
        !problems
            .iter()
            .any(|problem| problem.starts_with(&format!("crates/neo-sdk/{CLICK}:{line}  "))),
        "the placeholder form still matches"
    );
}

#[test]
fn a_line_without_its_reason_fails() {
    let tree = Tree::new();
    tree.edit(
        CLICK,
        "/// Neo: how long a detach may take before the session is dropped anyway; no script waits on\n/// it.",
        "/// Neo:",
    );
    tree.edit(
        CLICK,
        "/// Playwright has no cap; a script's `clickCount: 1e9` must not take the server's memory\n/// with it.\n",
        "",
    );
    let problems = tree.problems();
    assert_flags(
        &problems,
        CLICK,
        tree.line(CLICK, "/// Neo:"),
        &["`Neo:` gives no reason", "say why Neo needs it"],
    );
    assert_flags(
        &problems,
        CLICK,
        tree.line(CLICK, "/// Deviation: `server/frames.ts`"),
        &["`Deviation:` gives no reason", "say why Neo differs"],
    );
}

#[test]
fn a_malformed_line_fails() {
    let tree = Tree::new();
    tree.edit(
        CLICK,
        "/// Playwright: `server/frames.ts` `Frame`\n",
        "/// Playwright: server/frames.ts Frame\n",
    );
    assert_flags(
        &tree.problems(),
        CLICK,
        tree.line(CLICK, "/// Playwright: server/frames.ts Frame"),
        &[
            "malformed provenance line",
            "write `Playwright: `<path>` `<symbol>`: `<quote>``",
        ],
    );
}

/// `own.rs` has an untagged constant, a Playwright path in prose and a `Duration` literal, all
/// fine until it is listed.
#[test]
fn an_untagged_constant_fails_once_its_file_is_enforced() {
    let tree = Tree::new();
    assert!(
        !tree.problems().iter().any(|problem| problem.contains(OWN)),
        "an unlisted file's coverage is not checked"
    );
    tree.enforce(OWN);
    assert_flags(
        &tree.problems(),
        OWN,
        tree.line(OWN, "const POLL_MS"),
        &["untagged constant `POLL_MS`", "`/// Neo: …`"],
    );
}

#[test]
fn a_playwright_path_in_prose_fails_once_its_file_is_enforced() {
    let tree = Tree::new();
    tree.enforce(OWN);
    let line = tree.line(OWN, "It mirrors");
    assert_flags(
        &tree.problems(),
        OWN,
        line,
        &[
            "names Playwright's `server/frames.ts` in prose",
            "`Playwright:` or `Deviation:` line",
        ],
    );
    tree.edit(
        OWN,
        "`server/frames.ts` in prose",
        "`vendor/playwright/packages/playwright-core/src/server/frames.ts` in prose",
    );
    assert!(
        !tree
            .problems()
            .iter()
            .any(|problem| problem.starts_with(&format!("crates/neo-sdk/{OWN}:{line}  "))),
        "a vendored copy's path is Neo's"
    );
}

#[test]
fn a_duration_literal_fails_once_its_file_is_enforced() {
    let tree = Tree::new();
    tree.enforce(OWN);
    assert_flags(
        &tree.problems(),
        OWN,
        tree.line(OWN, "Duration::from_millis(10)"),
        &[
            "`Duration::from_millis` of a literal outside a constant",
            "name the duration as a `const`",
        ],
    );
}

#[test]
fn an_enforced_header_says_what_the_file_is() {
    let tree = Tree::new();
    tree.edit(
        CLICK,
        "//! Playwright: `server/frames.ts` `Frame.click`\n//! Playwright: `isomorphic/yaml.ts` `yamlStringNeedsQuotes`\n",
        "",
    );
    assert_flags(
        &tree.problems(),
        CLICK,
        1,
        &["the file header has no provenance line", "`//! Neo: <why>`"],
    );
    let tree = Tree::new();
    tree.edit(
        CLICK,
        "//! Playwright: `server/frames.ts` `Frame.click`\n",
        "//! Deviation: `server/frames.ts` `Frame.click` the fixture clicks once.\n",
    );
    assert_flags(
        &tree.problems(),
        CLICK,
        3,
        &["the header's first provenance line is `Deviation:`"],
    );
}

#[test]
fn an_untagged_item_in_a_port_of_several_fails() {
    let tree = Tree::new();
    tree.edit(
        CLICK,
        "/// Playwright: `server/frames.ts` `Frame`\nimpl",
        "impl",
    );
    tree.edit(
        GLUE,
        "  /** Playwright: `isomorphic/yaml.ts` `yamlStringNeedsQuotes` */\n",
        "",
    );
    let problems = tree.problems();
    assert_flags(
        &problems,
        CLICK,
        tree.line(CLICK, "impl FixtureFrame"),
        &[
            "untagged `impl FixtureFrame` in a port of several Playwright sources",
            "covers its members",
        ],
    );
    assert_flags(
        &problems,
        GLUE,
        tree.line(GLUE, "needsQuotes:"),
        &["untagged `glue.needsQuotes`"],
    );
}

#[test]
fn an_enforced_path_that_is_missing_fails() {
    let tree = Tree::new();
    tree.edit(
        ENFORCED,
        "neo-quickjs-injected/src/locator.ts\n",
        "neo-quickjs-injected/src/locator.ts\nneo-quickjs-injected/src/gone.ts\n",
    );
    assert_flags(
        &tree.problems(),
        ENFORCED,
        tree.line(ENFORCED, "gone.ts"),
        &[
            "`neo-quickjs-injected/src/gone.ts` does not exist",
            "remove the line",
        ],
    );
}

#[test]
fn a_copy_of_the_pin_that_disagrees_fails() {
    let tree = Tree::new();
    let sync_vendor = "neo-playwright-injected/scripts/sync-vendor.ts";
    tree.edit(
        sync_vendor,
        "26137face0500cacddf90c767dc1364da7262559",
        "36137face0500cacddf90c767dc1364da7262559",
    );
    tree.edit(
        "neo-quickjs-injected/tests/goldens/selector-corpus.ts",
        "'26137face'",
        "'26137fac0'",
    );
    let problems = tree.problems();
    assert_flags(
        &problems,
        sync_vendor,
        2,
        &[
            "pins Playwright at `36137face0500cacddf90c767dc1364da7262559`",
            "every copy of the pin moves together",
        ],
    );
    assert_flags(
        &problems,
        "neo-quickjs-injected/tests/goldens/selector-corpus.ts",
        2,
        &["pins Playwright at `26137fac0`"],
    );
}

#[test]
fn a_stale_register_fails() {
    let tree = Tree::new();
    tree.edit(
        REGISTER,
        "| neo-runtime/src/click.rs | `MAX_CLICK_COUNT` |",
        "| neo-runtime/src/click.rs | `MAX_CLICKS` |",
    );
    assert_flags(
        &tree.problems(),
        REGISTER,
        0,
        &[
            "stale: 1 row to add, 1 to remove",
            "NEO_RUNTIME_WRITE_FIXTURES=1 cargo test",
        ],
    );
}

#[test]
fn an_edited_or_uncited_vendored_file_fails() {
    let tree = Tree::new();
    let frames = "neo-runtime/vendor/playwright/packages/playwright-core/src/server/frames.ts";
    tree.edit(frames, "const timeout = waitTime", "const delay = waitTime");
    let dom = "neo-runtime/vendor/playwright/packages/playwright-core/src/server/dom.ts";
    fs::write(tree.0.path().join(dom), "export {};\n").expect("the uncited file writes");
    let problems = tree.problems();
    let manifest = "neo-runtime/vendor/playwright/VENDOR.md";
    assert_flags(
        &problems,
        manifest,
        tree.line(manifest, "server/frames.ts"),
        &[
            "`packages/playwright-core/src/server/frames.ts` was edited",
            "never edit",
        ],
    );
    assert_flags(
        &problems,
        dom,
        0,
        &[
            "vendored, but no provenance line cites it",
            "--ignored sync",
        ],
    );
    assert_flags(&problems, dom, 0, &["not listed in VENDOR.md"]);
}

/// The test module holds a false citation, a constant cited to it and a `Duration` literal;
/// none counts until the module stops being test code.
#[test]
fn test_code_is_not_checked() {
    let tree = Tree::new();
    tree.edit(CLICK, "#[cfg(test)]\nmod tests {", "mod tests {");
    let problems = tree.problems();
    assert_flags(
        &problems,
        CLICK,
        tree.line(CLICK, "`Frame.nope`"),
        &["no `nope` there"],
    );
    assert_flags(
        &problems,
        CLICK,
        tree.line(CLICK, "Duration::from_millis(5)"),
        &["`Duration::from_millis`"],
    );
}
