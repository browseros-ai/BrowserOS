//! Keeps page JavaScript on one path. Every `Runtime.evaluate` and `Runtime.callFunctionOn` that
//! BrowserOS sends goes through `browseros_core::page_js`, which decides the world a script runs
//! in. A raw send anywhere else would run in the page's own world whatever the caller intended.
//!
//! Neo is the one other runtime with its own path. Its crates never depend on browseros-core
//! (`crates/NEO_STYLE.md`), and two Neo files call into the page, with `Runtime.callFunctionOn`
//! only: `neo-runtime/src/world.rs`, on an object of Neo's own isolated world or on the context
//! `Page.createIsolatedWorld` returned for it, and `neo-runtime/src/main_world.rs`, the one door to
//! the page's own world that Playwright's evaluations need (`page.evaluate`, `JSHandle.evaluate`),
//! on the `UtilityScript` object it built there from a bundled artifact, or on a page object an
//! event named, to keep it in a run's handle group. Neither may send anything
//! else, so a `Runtime.evaluate` still fails here, in Neo as everywhere. Neo's own guard
//! (`neo-runtime/tests/page_js_guard.rs`) is stricter: it also refuses the other script-running
//! methods and requires every Neo `functionDeclaration` to be a built artifact.
//!
//! A send is recognised by the method literal in argument position (followed by a comma), which
//! leaves the CDP policy tables and error labels that merely name the methods alone.

use std::{
    fs,
    path::{Path, PathBuf},
};

const EVALUATE: &str = "\"Runtime.evaluate\"";
const CALL_FUNCTION_ON: &str = "\"Runtime.callFunctionOn\"";
const METHODS: &[&str] = &[EVALUATE, CALL_FUNCTION_ON];
/// The files that may send page JavaScript, each with the methods it may send.
const ALLOWED: &[(&str, &[&str])] = &[
    ("crates/browseros-core/src/page_js.rs", METHODS),
    ("crates/neo-runtime/src/world.rs", &[CALL_FUNCTION_ON]),
    ("crates/neo-runtime/src/main_world.rs", &[CALL_FUNCTION_ON]),
];
const SKIPPED_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "tests",
    "integration-tests",
];

#[test]
fn tests_that_page_javascript_is_only_sent_through_page_js() -> std::io::Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for root in ["crates", "apps"] {
        collect_rust_files(&workspace.join(root), &mut files)?;
    }
    assert!(
        files.len() > 50,
        "the guard found only {} Rust files under {}; is the workspace layout different?",
        files.len(),
        workspace.display()
    );

    let mut offenders = Vec::new();
    for file in files {
        let relative = file
            .strip_prefix(&workspace)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        if is_test_file(&relative) {
            continue;
        }
        let allowed = allowed_methods(&relative);
        let source = fs::read_to_string(&file)?;
        for (index, line) in production_lines(&source).enumerate() {
            if sends_page_script(line, allowed) {
                offenders.push(format!("{relative}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "page JavaScript must go through browseros_core::PageJs, which picks the isolated or the \
         page world (in Neo, through neo-runtime's World, which calls only Neo's isolated world):\n{}",
        offenders.join("\n")
    );
    Ok(())
}

#[test]
fn tests_that_neo_world_may_call_functions_but_never_evaluate() {
    let world = allowed_methods("crates/neo-runtime/src/world.rs");
    assert!(
        !sends_page_script(r#"self.send("Runtime.callFunctionOn", request)"#, world),
        "world.rs calls into Neo's isolated world"
    );
    assert!(
        sends_page_script(r#"self.send("Runtime.evaluate", request)"#, world),
        "world.rs may not evaluate, which defaults to the page's own world"
    );
    assert!(
        sends_page_script(
            r#"self.send("Runtime.callFunctionOn", request)"#,
            allowed_methods("crates/neo-runtime/src/page.rs"),
        ),
        "every other Neo file is held to the one path"
    );
    assert!(
        !sends_page_script(
            r#"client.send("Runtime.evaluate", params)"#,
            allowed_methods("crates/browseros-core/src/page_js.rs"),
        ),
        "page_js.rs sends both"
    );
}

#[test]
fn tests_that_neo_main_world_may_call_functions_but_never_evaluate() {
    let main_world = allowed_methods("crates/neo-runtime/src/main_world.rs");
    assert!(
        !sends_page_script(
            r#"frame.host.send("Runtime.callFunctionOn", request)"#,
            main_world
        ),
        "main_world.rs calls the UtilityScript it built in the page's own world"
    );
    assert!(
        sends_page_script(
            r#"frame.host.send("Runtime.evaluate", request)"#,
            main_world
        ),
        "main_world.rs may not evaluate"
    );
    let policy = allowed_methods("crates/neo-runtime/src/evaluate.rs");
    for line in [
        r#"frame.host.send("Runtime.callFunctionOn", request)"#,
        r#"frame.host.send("Runtime.evaluate", request)"#,
    ] {
        assert!(
            sends_page_script(line, policy),
            "evaluate.rs is policy and reaches the page only through main_world.rs: {line}"
        );
    }
}

/// The methods `relative` may send: none unless [`ALLOWED`] names the file.
fn allowed_methods(relative: &str) -> &'static [&'static str] {
    ALLOWED
        .iter()
        .find(|(path, _)| *path == relative)
        .map_or(&[], |(_, methods)| methods)
}

fn collect_rust_files(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry.file_type()?.is_dir() {
            if !name.starts_with('.') && !SKIPPED_DIRS.contains(&name.as_ref()) {
                collect_rust_files(&path, files)?;
            }
        } else if name.ends_with(".rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn is_test_file(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    name == "tests.rs" || name.ends_with("_tests.rs") || name == "test_support.rs"
}

/// The file up to its `#[cfg(test)] mod` block, whose fakes match on these method names.
fn production_lines(source: &str) -> impl Iterator<Item = &str> {
    let lines: Vec<&str> = source.lines().collect();
    let end = lines
        .windows(2)
        .position(|pair| {
            pair[0].trim() == "#[cfg(test)]" && pair[1].trim_start().starts_with("mod ")
        })
        .unwrap_or(lines.len());
    lines.into_iter().take(end)
}

fn sends_page_script(line: &str, allowed: &[&str]) -> bool {
    METHODS
        .iter()
        .filter(|method| !allowed.contains(method))
        .any(|method| {
            line.match_indices(method)
                .any(|(at, _)| line[at + method.len()..].trim_start().starts_with(','))
        })
}
