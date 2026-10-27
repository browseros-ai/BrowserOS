//! Keeps page JavaScript on one path. Every `Runtime.evaluate` and `Runtime.callFunctionOn` that
//! BrowserOS sends goes through `browseros_core::page_js`, which decides the world a script runs
//! in. A raw send anywhere else would run in the page's own world whatever the caller intended.
//!
//! A send is recognised by the method literal in argument position (followed by a comma), which
//! leaves the CDP policy tables and error labels that merely name the methods alone.

use std::{
    fs,
    path::{Path, PathBuf},
};

const METHODS: &[&str] = &["\"Runtime.evaluate\"", "\"Runtime.callFunctionOn\""];
const ALLOWED: &[&str] = &["crates/browseros-core/src/page_js.rs"];
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
         page world:\n{}",
        offenders.join("\n")
    );
    Ok(())
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
