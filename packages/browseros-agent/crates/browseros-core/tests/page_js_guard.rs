//! Keeps page JavaScript on one path. Every `Runtime.evaluate` and `Runtime.callFunctionOn` that
//! BrowserOS sends goes through `browseros_core::page_js`, which decides the world a script runs
//! in. A raw send anywhere else would run in the page's own world whatever the caller intended.
//!
//! Neo is the one other runtime with its own path. Its crates never depend on browseros-core
//! (`crates/NEO_STYLE.md`), and two Neo files call into the page, with `Runtime.callFunctionOn`
//! only: `neo-runtime/src/chromium_bridge/playwright_injected_methods.rs`, on an object of Neo's own isolated world or on the context
//! `Page.createIsolatedWorld` returned for it, and `neo-runtime/src/chromium_bridge/main_world.rs`, the one door to
//! the page's own world that Playwright's evaluations need (`page.evaluate`, `JSHandle.evaluate`),
//! on the `UtilityScript` object it built there from a bundled artifact, or on a page object an
//! event named, to keep it in a run's handle group. Neither may send anything
//! else, so a `Runtime.evaluate` still fails here, in Neo as everywhere. Neo's own guard
//! (`neo-runtime/tests/page_js_guard.rs`) is stricter: it also refuses the other script-running
//! methods and requires every Neo `functionDeclaration` to be a built artifact.
//!
//! A send is recognised by the method literal in argument position (followed by a comma), which
//! leaves the CDP policy tables and error labels that merely name the methods alone. Literals are
//! read as the compiler reads them (`proc-macro2` tokens, decoded by `syn`), so an escaped, raw or
//! byte string literal is its value: `"\x52untime.evaluate"` is `Runtime.evaluate`.

use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use proc_macro2::{Delimiter, Literal, TokenStream, TokenTree};

const EVALUATE: &str = "Runtime.evaluate";
const CALL_FUNCTION_ON: &str = "Runtime.callFunctionOn";
const METHODS: &[&str] = &[EVALUATE, CALL_FUNCTION_ON];
/// The files that may send page JavaScript, each with the methods it may send.
const ALLOWED: &[(&str, &[&str])] = &[
    ("crates/browseros-core/src/page_js.rs", METHODS),
    (
        "crates/neo-runtime/src/chromium_bridge/playwright_injected_methods.rs",
        &[CALL_FUNCTION_ON],
    ),
    (
        "crates/neo-runtime/src/chromium_bridge/main_world.rs",
        &[CALL_FUNCTION_ON],
    ),
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
        let lines: Vec<&str> = source.lines().collect();
        for line in sends(&source, allowed) {
            let text = lines
                .get(line.saturating_sub(1))
                .copied()
                .unwrap_or_default();
            offenders.push(format!("{relative}:{line}: {}", text.trim()));
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
    let world =
        allowed_methods("crates/neo-runtime/src/chromium_bridge/playwright_injected_methods.rs");
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
            allowed_methods("crates/neo-runtime/src/browser_representations/page.rs"),
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
    let main_world = allowed_methods("crates/neo-runtime/src/chromium_bridge/main_world.rs");
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
    let policy = allowed_methods("crates/neo-runtime/src/playwright_methods/evaluate.rs");
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

#[test]
fn tests_that_test_modules_are_skipped_but_code_after_them_is_read() {
    let source = "mod a {\n    #[cfg(test)]\n    mod tests {\n        fn f() { send(\"Runtime.evaluate\", p) }\n    }\n}\nfn g() { send(\"Runtime.evaluate\", p) }\n";
    assert_eq!(
        sends(source, &[]),
        vec![7],
        "only the send after the nested test module"
    );
}

#[test]
fn tests_that_escaped_raw_and_byte_literals_are_read_as_their_value() {
    for send in [
        r#"client.send("\x52untime.evaluate", params)"#,
        r#"client.send("\u{52}untime.evaluate", params)"#,
        r##"client.send(r#"Runtime.evaluate"#, params)"##,
        r#"client.send(b"Runtime.evaluate", params)"#,
        r#"const EVAL: &str = "\x52untime.callFunctionOn"; fn f() { send("\x52untime.callFunctionOn", p) }"#,
    ] {
        assert!(sends_page_script(send, &[]), "not refused: {send}");
    }
    assert!(
        !sends_page_script(
            r#"send("\x52untime.callFunctionOn", p)"#,
            &[CALL_FUNCTION_ON]
        ),
        "an allowed method stays allowed however it is spelled"
    );
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

fn sends_page_script(source: &str, allowed: &[&str]) -> bool {
    !sends(source, allowed).is_empty()
}

/// The lines of `source` with a method `allowed` does not name in argument position, outside its
/// `#[cfg(test)] mod` blocks (whose fakes match on these method names). Source that does not
/// tokenize as Rust answers line 0, so nothing unreadable passes.
fn sends(source: &str, allowed: &[&str]) -> Vec<usize> {
    let Ok(stream) = TokenStream::from_str(source) else {
        return vec![0];
    };
    let mut lines = Vec::new();
    scan(stream, allowed, &mut lines);
    lines
}

fn scan(stream: TokenStream, allowed: &[&str], lines: &mut Vec<usize>) {
    let trees: Vec<TokenTree> = stream.into_iter().collect();
    let mut index = 0;
    while index < trees.len() {
        if is_test_module(&trees[index..]) {
            index += 5;
            continue;
        }
        match &trees[index] {
            TokenTree::Group(group) => scan(group.stream(), allowed, lines),
            TokenTree::Literal(literal) => {
                let argument = matches!(
                    trees.get(index + 1),
                    Some(TokenTree::Punct(comma)) if comma.as_char() == ','
                );
                let sent = string_value(literal).is_some_and(|value| {
                    METHODS
                        .iter()
                        .any(|method| *method == value && !allowed.contains(method))
                });
                if argument && sent {
                    lines.push(literal.span().start().line);
                }
            }
            TokenTree::Ident(_) | TokenTree::Punct(_) => {}
        }
        index += 1;
    }
}

/// Whether `trees` starts with `#[cfg(test)] mod name { … }`.
fn is_test_module(trees: &[TokenTree]) -> bool {
    let [
        TokenTree::Punct(hash),
        TokenTree::Group(attribute),
        TokenTree::Ident(keyword),
        TokenTree::Ident(_),
        TokenTree::Group(body),
        ..,
    ] = trees
    else {
        return false;
    };
    let cfg_test = match attribute
        .stream()
        .into_iter()
        .collect::<Vec<_>>()
        .as_slice()
    {
        [TokenTree::Ident(cfg), TokenTree::Group(arguments)] => {
            cfg == "cfg"
                && arguments.delimiter() == Delimiter::Parenthesis
                && arguments.stream().to_string() == "test"
        }
        _ => false,
    };
    hash.as_char() == '#'
        && attribute.delimiter() == Delimiter::Bracket
        && cfg_test
        && keyword == "mod"
        && body.delimiter() == Delimiter::Brace
}

/// A string literal's value as the compiler reads it (escapes decoded; raw, byte and C strings
/// included); a literal `syn` does not classify is read as written. `None` for other literals.
fn string_value(literal: &Literal) -> Option<String> {
    match syn::Lit::new(literal.clone()) {
        syn::Lit::Str(text) => Some(text.value()),
        syn::Lit::ByteStr(bytes) => Some(String::from_utf8_lossy(&bytes.value()).into_owned()),
        syn::Lit::CStr(text) => Some(text.value().to_string_lossy().into_owned()),
        syn::Lit::Verbatim(other) => Some(other.to_string()),
        _ => None,
    }
}
