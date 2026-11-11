//! Keeps page JavaScript on known paths. Every `Runtime.evaluate` and `Runtime.callFunctionOn`
//! that BrowserOS sends goes through `browseros_core::page_js`, which decides the world a script
//! runs in, or through the neo-playwright files in `ALLOWED`, which own its sealed
//! `browseros-sdk` world, main-world `page.evaluate`, and dedicated-worker evaluation. A raw
//! send anywhere else would run in the target's default world whatever the caller intended.
//!
//! A send is recognised by the method literal in argument position (followed by a comma), which
//! leaves the CDP policy tables and error labels that merely name the methods alone.

use std::{
    fs,
    path::{Path, PathBuf},
};

const METHODS: &[&str] = &["\"Runtime.evaluate\"", "\"Runtime.callFunctionOn\""];
const ALLOWED: &[&str] = &[
    "crates/browseros-core/src/page_js.rs",
    // `PageJs` cannot name neo-playwright's sealed world. These files install it, speak
    // its probe protocol, and run main-world scripts after the operation's capability check.
    "crates/neo-sdk/neo-playwright/src/world/sealed_world.rs",
    "crates/neo-sdk/neo-playwright/src/probe/probe_call.rs",
    "crates/neo-sdk/neo-playwright/src/page/evaluate_guard.rs",
    // Workers have no document or isolated world. This gateway selects the worker target's
    // startup context and races its lifetime, after checking the owning page's capability.
    "crates/neo-sdk/neo-playwright/src/page/worker_evaluate.rs",
];
const SKIPPED_DIRS: &[&str] = &["target", "node_modules", "dist", "tests"];

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
        if ALLOWED.contains(&relative.as_str()) || is_test_file(&relative) {
            continue;
        }
        let source = fs::read_to_string(&file)?;
        for (index, line) in production_lines(&source).enumerate() {
            if sends_page_script(line) {
                offenders.push(format!("{relative}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "page JavaScript must go through browseros_core::PageJs, which picks the isolated or the \
         page world, or through the neo-playwright files in ALLOWED:\n{}",
        offenders.join("\n")
    );
    Ok(())
}

/// An entry whose file is gone guards nothing: a renamed file would send page script unchecked
/// while the list still read as if it were reviewed.
#[test]
fn tests_that_every_allowed_file_exists() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let missing: Vec<&str> = ALLOWED
        .iter()
        .copied()
        .filter(|allowed| !workspace.join(allowed).is_file())
        .collect();
    assert!(
        missing.is_empty(),
        "ALLOWED names files that do not exist; rename the entry with its file: {missing:?}"
    );
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

fn sends_page_script(line: &str) -> bool {
    METHODS.iter().any(|method| {
        line.match_indices(method)
            .any(|(at, _)| line[at + method.len()..].trim_start().starts_with(','))
    })
}
