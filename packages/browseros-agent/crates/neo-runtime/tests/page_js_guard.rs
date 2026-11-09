//! Neo's page JavaScript guard: which Neo file may reach which page world, and with what code.
//!
//! The owner's constraint: page JavaScript never bypasses one narrow, deliberate path. In Neo
//! that path is four files of `neo-runtime`:
//!
//! - `src/chromium_bridge/playwright_injected_methods.rs`, Neo's isolated (utility) world: `Page.createIsolatedWorld` and
//!   `Runtime.callFunctionOn`, for every read and action.
//! - `src/chromium_bridge/main_world.rs`, the page's own world, for Playwright's evaluations: `Runtime.callFunctionOn`
//!   only, on the `UtilityScript` it builds there, and on a page object an event named (the
//!   built `(object) => object`, which keeps it in a run's handle group).
//! - `src/browser_representations/init_scripts.rs`, the agent's init scripts: `Page.addScriptToEvaluateOnNewDocument` and
//!   `Page.removeScriptToEvaluateOnNewDocument`, in the page's own world only (no `worldName`).
//! - `src/browser_representations/bindings.rs`, `exposeBinding`: `Runtime.addBinding` and `Runtime.removeBinding` of
//!   Playwright's one CDP binding, which runs no script; the page half is built artifacts it hands
//!   to `init_scripts.rs` and `main_world.rs`.
//!
//! In both, every `functionDeclaration` is a `const` whose initializer is exactly
//! `include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
//! "/../neo-playwright-injected/dist/<name>.js"))` and that nothing else in the file re-binds (or a
//! parameter every caller in the file passes one of those consts), so agent code can reach a page
//! only as an argument value.
//!
//! Literals are read as the compiler reads them: `proc-macro2` tokenizes the file and `syn`
//! decodes each string literal (escapes, raw, byte and C strings), so `"\x52untime.evaluate"` is
//! `Runtime.evaluate`; the literals of a `concat!` are also read joined.
//!
//! Every other Neo Rust source (`src/`, `examples/`, `codegen/`, `build.rs` of each `crates/neo-*`
//! crate) may not hold a string literal naming a script-running or world-reaching CDP method at
//! all, nor a fragment that composes one (`"Runtime.{}"`, `".evaluate"`, `"evaluate"`), nor build
//! a `functionDeclaration`. Comments and `#[cfg(test)] mod` blocks are not code; test directories
//! (`tests/`, `integration-tests/`) drive pages through their own raw CDP clients and are not
//! scanned. browseros-core's `tests/page_js_guard.rs` holds the same two files to its rule too.
//!
//! A later slice that needs another method (init scripts: `Page.addScriptToEvaluateOnNewDocument`;
//! bindings: `Runtime.addBinding`) adds exactly one [`ALLOWED`] row and a self-test for it.

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use proc_macro2::{Delimiter, Literal, Span, TokenStream, TokenTree};
use syn::punctuated::Punctuated;
use syn::{Expr, Item, Lit, Token as SynToken};

/// The methods that run script or reach a world of the page.
const GUARDED: &[&str] = &[
    "Runtime.evaluate",
    "Runtime.callFunctionOn",
    "Runtime.compileScript",
    "Runtime.runScript",
    "Runtime.addBinding",
    "Runtime.removeBinding",
    "Page.addScriptToEvaluateOnNewDocument",
    "Page.removeScriptToEvaluateOnNewDocument",
    "Page.createIsolatedWorld",
    "Debugger.evaluateOnCallFrame",
];

/// The files that may name guarded methods, each with the methods it may name. Paths are
/// relative to `crates/`.
const ALLOWED: &[(&str, &[&str])] = &[
    (
        "neo-runtime/src/chromium_bridge/playwright_injected_methods.rs",
        &["Runtime.callFunctionOn", "Page.createIsolatedWorld"],
    ),
    (
        "neo-runtime/src/chromium_bridge/main_world.rs",
        &["Runtime.callFunctionOn"],
    ),
    (
        INIT_SCRIPTS,
        &[
            "Page.addScriptToEvaluateOnNewDocument",
            "Page.removeScriptToEvaluateOnNewDocument",
        ],
    ),
    (BINDINGS, &["Runtime.addBinding", "Runtime.removeBinding"]),
];

/// The init scripts' file: its scripts run in the page's own world, so it never names a world.
const INIT_SCRIPTS: &str = "neo-runtime/src/browser_representations/init_scripts.rs";

/// The bindings' file: it adds and removes Playwright's CDP binding, and runs no script itself.
const BINDINGS: &str = "neo-runtime/src/browser_representations/bindings.rs";

/// Where a built artifact lives: the only source a `functionDeclaration` may have.
const BUILT_PREFIX: &str = "/../neo-playwright-injected/dist/";

#[test]
fn tests_that_only_the_two_world_doors_reach_page_javascript() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for entry in fs::read_dir(&crates).expect("crates/ lists") {
        let entry = entry.expect("a crates/ entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("neo-") || !entry.path().is_dir() {
            continue;
        }
        for sub in ["src", "examples", "codegen"] {
            collect(&entry.path().join(sub), &mut files);
        }
        let build = entry.path().join("build.rs");
        if build.is_file() {
            files.push(build);
        }
    }
    assert!(
        files.len() > 40,
        "the guard found only {} neo Rust sources under {}",
        files.len(),
        crates.display()
    );
    let mut problems = Vec::new();
    for file in &files {
        let relative = file
            .strip_prefix(&crates)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        // Unit-test files of the code generator, as browseros-core's guard skips them.
        if relative.ends_with("/tests.rs") {
            continue;
        }
        let source = fs::read_to_string(file).expect("a neo source reads");
        problems.extend(check(&relative, &source));
    }
    assert!(
        problems.is_empty(),
        "page JavaScript must go through neo-runtime's world.rs (isolated world) or main_world.rs \
         (the page's world, Runtime.callFunctionOn of built artifacts only); ask med-pw-eval for \
         an ALLOWED row rather than adding a raw send:\n{}",
        problems.join("\n")
    );
}

#[test]
fn tests_that_a_const_alias_of_a_guarded_method_is_refused() {
    let problems = check(
        "neo-runtime/src/method_router.rs",
        "const EVAL: &str = \"Runtime.evaluate\";\nfn f() {}\n",
    );
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("Runtime.evaluate"), "{problems:?}");
}

#[test]
fn tests_that_a_composed_method_name_is_refused() {
    for line in [
        "let m = format!(\"Runtime.{}\", name);",
        "let m = concat!(\"Runtime\", \".evaluate\");",
        "let m = [domain, \"evaluate\"].join(\".\");",
        "let m = format!(\"{}.{}\", \"Page\", \"addScriptToEvaluateOnNewDocument\");",
    ] {
        let problems = check("neo-runtime/src/browser_representations/page.rs", line);
        assert!(!problems.is_empty(), "not refused: {line}");
    }
}

#[test]
fn tests_that_a_method_is_refused_outside_its_file() {
    let send = "self.send(\"Runtime.callFunctionOn\", request).await";
    assert!(check("neo-runtime/src/chromium_bridge/main_world.rs", send).is_empty());
    assert!(
        check(
            "neo-runtime/src/chromium_bridge/playwright_injected_methods.rs",
            send
        )
        .is_empty()
    );
    assert!(!check("neo-runtime/src/playwright_methods/evaluate.rs", send).is_empty());
    assert!(!check("neo-runtime/src/browser_representations/page.rs", send).is_empty());
    assert!(!check("neo-quickjs-injected/src/x.rs", send).is_empty());
    let world = "self.send(\"Page.createIsolatedWorld\", params)";
    assert!(
        check(
            "neo-runtime/src/chromium_bridge/playwright_injected_methods.rs",
            world
        )
        .is_empty()
    );
    assert!(
        !check("neo-runtime/src/chromium_bridge/main_world.rs", world).is_empty(),
        "main_world.rs reaches the page's own world only"
    );
    for method in [
        "Runtime.evaluate",
        "Runtime.addBinding",
        "Runtime.runScript",
    ] {
        let line = format!("self.send(\"{method}\", params)");
        assert!(
            !check("neo-runtime/src/chromium_bridge/main_world.rs", &line).is_empty(),
            "{method} in main_world.rs"
        );
        assert!(
            !check(
                "neo-runtime/src/chromium_bridge/playwright_injected_methods.rs",
                &line
            )
            .is_empty(),
            "{method} in world.rs"
        );
    }
}

#[test]
fn tests_that_markdown_prose_in_a_literal_is_not_a_send() {
    let prose = "let doc = \"crosses as a `Runtime.callFunctionOn` argument\";";
    assert!(check("neo-runtime-spec/codegen/contract.rs", prose).is_empty());
    let half = "let m = \"`Runtime.evaluate\";";
    assert!(!check("neo-runtime-spec/codegen/contract.rs", half).is_empty());
}

#[test]
fn tests_that_comments_and_test_modules_are_not_code() {
    let source = "//! Sends `\"Runtime.evaluate\"` nowhere.\n\
                  /* \"Runtime.evaluate\" */\n\
                  fn f() {}\n\
                  #[cfg(test)]\n\
                  mod tests {\n    const X: &str = \"Runtime.evaluate\";\n}\n";
    assert!(check("neo-runtime/src/browser_representations/page.rs", source).is_empty());
    let after = format!("{source}const Y: &str = \"Runtime.evaluate\";\n");
    assert_eq!(
        check("neo-runtime/src/browser_representations/page.rs", &after).len(),
        1,
        "code after a test module is still code"
    );
}

#[test]
fn tests_that_a_function_declaration_must_be_a_built_artifact() {
    let built = "const GLUE: &str = include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../neo-playwright-injected/dist/call.js\"));\n";
    let ok = format!("{built}fn f() {{ json!({{\"functionDeclaration\": GLUE}}); }}\n");
    assert!(check("neo-runtime/src/chromium_bridge/main_world.rs", &ok).is_empty());

    let literal = format!(
        "{built}fn f() {{ json!({{\"functionDeclaration\": \"() => document.title\"}}); }}\n"
    );
    assert!(!check("neo-runtime/src/chromium_bridge/main_world.rs", &literal).is_empty());

    let other_const =
        "const JS: &str = \"() => 1\";\nfn f() { json!({\"functionDeclaration\": JS}); }\n";
    assert!(
        !check(
            "neo-runtime/src/chromium_bridge/playwright_injected_methods.rs",
            other_const
        )
        .is_empty()
    );

    let wrong_dir = "const JS: &str = include_str!(\"../payload.js\");\nfn f() { json!({\"functionDeclaration\": JS}); }\n";
    assert!(
        !check(
            "neo-runtime/src/chromium_bridge/playwright_injected_methods.rs",
            wrong_dir
        )
        .is_empty()
    );

    let relay_ok = format!(
        "{built}impl W {{ async fn run(&self, context: i64, function: &str) {{ json!({{\"functionDeclaration\": function}}); }}\n\
         async fn a(&self) {{ self.run(1, GLUE).await; }} }}\n"
    );
    assert!(
        check(
            "neo-runtime/src/chromium_bridge/playwright_injected_methods.rs",
            &relay_ok
        )
        .is_empty()
    );
    let relay_bad = format!(
        "{built}impl W {{ async fn run(&self, context: i64, function: &str) {{ json!({{\"functionDeclaration\": function}}); }}\n\
         async fn a(&self, js: &str) {{ self.run(1, js).await; }} }}\n"
    );
    assert!(
        !check(
            "neo-runtime/src/chromium_bridge/playwright_injected_methods.rs",
            &relay_bad
        )
        .is_empty()
    );

    let elsewhere = "fn f() { json!({\"functionDeclaration\": X}); }\n";
    assert!(!check("neo-runtime/src/playwright_methods/evaluate.rs", elsewhere).is_empty());
}

// ---------------------------------------------------------------------------------------------
// The check.
// ---------------------------------------------------------------------------------------------

/// Every problem in one source file, as `file:line: what`.
fn check(relative: &str, source: &str) -> Vec<String> {
    let allowed = ALLOWED
        .iter()
        .find(|(path, _)| *path == relative)
        .map_or(&[][..], |(_, methods)| *methods);
    let Some(all) = lex(source) else {
        return vec![format!(
            "{relative}: does not tokenize as Rust, so the guard cannot read it"
        )];
    };
    let tokens = without_test_modules(&all);
    let mut problems = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let Token::Str { text, line } = token else {
            continue;
        };
        for method in GUARDED {
            if names(text, method) && !allowed.contains(method) {
                problems.push(format!(
                    "{relative}:{line}: names {method}, which only its ALLOWED file may"
                ));
            }
        }
        if composes_method(text) && !action_kind_label(relative, &tokens, index) {
            problems.push(format!(
                "{relative}:{line}: {text:?} composes a guarded CDP method name"
            ));
        }
    }
    for (text, line) in concatenations(&tokens) {
        for method in GUARDED {
            if names(&text, method) && !allowed.contains(method) {
                problems.push(format!(
                    "{relative}:{line}: a concat! of literals names {method}, which only its ALLOWED file may"
                ));
            }
        }
    }
    if relative == INIT_SCRIPTS {
        // A literal, an identifier or a `concat!` of literals that names a world, read as the
        // compiler reads them, so a computed key (`concat!("world", "Name")`) is one too.
        let names_world = |text: &str| text.to_ascii_lowercase().contains("world");
        for token in &tokens {
            let named = match token {
                Token::Str { text, .. } => names_world(text),
                Token::Ident { text, .. } => text == "world_name" || text == "worldName",
                Token::Punct { .. } => false,
            };
            if named {
                problems.push(format!(
                    "{relative}:{}: names a world; init scripts run in the page's own world only",
                    token.line()
                ));
            }
        }
        for (text, line) in concatenations(&tokens) {
            if names_world(&text) {
                problems.push(format!(
                    "{relative}:{line}: a concat! of literals names a world; init scripts run in the page's own world only"
                ));
            }
        }
    }
    let built = built_consts(source, &tokens);
    problems.extend(declarations(relative, &tokens, !allowed.is_empty(), &built));
    problems
}

/// Neo: the classifier's exact `Evaluate` match arm is a policy label. This exception grants
/// no CDP method or function declaration permission, even elsewhere in the same file.
fn action_kind_label(relative: &str, tokens: &[Token], index: usize) -> bool {
    let arm = ["Self", ":", ":", "Evaluate", "=", ">"];
    relative == "neo-runtime/src/actions.rs"
        && matches!(tokens.get(index), Some(Token::Str { text, .. }) if text == "evaluate")
        && index >= arm.len()
        && arm
            .iter()
            .enumerate()
            .all(|(offset, part)| tokens[index - arm.len() + offset].is(part))
        && tokens.get(index + 1).is_some_and(|token| token.is(","))
}

#[test]
fn tests_that_an_action_label_does_not_grant_page_script_access() {
    let file = "neo-runtime/src/actions.rs";
    let label = "impl ActionKind { fn as_str(self) -> &'static str { match self { Self::Evaluate => \"evaluate\", } } }";
    assert!(check(file, label).is_empty(), "an action's name is data");
    assert!(
        !check("neo-runtime/src/probe.rs", label).is_empty(),
        "the exception belongs only to the classifier"
    );
    for source in [
        "const METHOD: &str = \"evaluate\";",
        "let method = [domain, \"evaluate\"].join(\".\");",
        "self.send(\"Runtime.evaluate\", params)",
        "self.send(\"Runtime.callFunctionOn\", params)",
        "json!({\"functionDeclaration\": CODE})",
    ] {
        assert!(!check(file, source).is_empty(), "guarded: {source}");
    }
}

/// The literals of every `concat!(…)`, joined as the macro joins them, with its line.
fn concatenations(tokens: &[Token]) -> Vec<(String, usize)> {
    let mut joined = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let opens = token.is("concat")
            && tokens.get(index + 1).is_some_and(|token| token.is("!"))
            && tokens.get(index + 2).is_some_and(|token| token.is("("));
        if !opens {
            continue;
        }
        let mut text = String::new();
        let mut depth = 0usize;
        for token in &tokens[index + 2..] {
            if token.is("(") || token.is("[") || token.is("{") {
                depth += 1;
            } else if token.is(")") || token.is("]") || token.is("}") {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            } else if let Token::Str { text: part, .. } = token {
                text.push_str(part);
            }
        }
        joined.push((text, token.line()));
    }
    joined
}

/// Whether a literal names `method` as code would: anywhere but inside Markdown backticks, which
/// is prose (the code generator's doc text), never a method CDP would accept.
fn names(text: &str, method: &str) -> bool {
    text.match_indices(method).any(|(at, _)| {
        let quoted_before = text[..at].ends_with('`');
        let quoted_after = text[at + method.len()..].starts_with('`');
        !(quoted_before && quoted_after)
    })
}

/// A literal that builds a guarded method from parts: a domain followed by a placeholder or the
/// end of the literal, or a method's own name with or without its leading dot.
fn composes_method(text: &str) -> bool {
    GUARDED.iter().any(|method| {
        let (domain, name) = method.split_once('.').unwrap_or((method, ""));
        let prefix = format!("{domain}.");
        text == name
            || text == format!(".{name}")
            || text.ends_with(&prefix)
            || text.contains(&format!("{prefix}{{"))
    })
}

/// The `functionDeclaration` rule. Outside the two doors no file builds one; in them each value
/// is a built const, or a parameter every in-file call passes a built const.
fn declarations(relative: &str, tokens: &[Token], door: bool, built: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let is_key = matches!(token, Token::Str { text, .. } if text == "functionDeclaration")
            || matches!(token, Token::Ident { text, .. } if text == "function_declaration");
        if !is_key {
            continue;
        }
        let line = token.line();
        if !door {
            problems.push(format!(
                "{relative}:{line}: builds a functionDeclaration outside world.rs and main_world.rs"
            ));
            continue;
        }
        let (Some(Token::Punct { text: ':', .. }), Some(value)) =
            (tokens.get(index + 1), tokens.get(index + 2))
        else {
            problems.push(format!(
                "{relative}:{line}: a functionDeclaration whose value the guard cannot read"
            ));
            continue;
        };
        let after = tokens.get(index + 3);
        let ends = matches!(
            after,
            Some(Token::Punct {
                text: ',' | '}',
                ..
            }) | None
        );
        match value {
            Token::Ident { text, .. } if ends && built.iter().any(|name| name == text) => {}
            Token::Ident { text, .. } if ends && relayed(tokens, text, built) => {}
            _ => problems.push(format!(
                "{relative}:{line}: a functionDeclaration that is not a built neo-playwright-injected/dist/*.js const"
            )),
        }
    }
    problems
}

/// The top-level consts of `source` that are built artifacts ([`is_built_artifact`]), by name.
/// A name is trusted only when every top-level const of that name is one and nothing else in the
/// file binds the name again ([`bound_once`]); a file that does not parse has none.
fn built_consts(source: &str, tokens: &[Token]) -> Vec<String> {
    let Ok(file) = syn::parse_file(source) else {
        return Vec::new();
    };
    let mut built = Vec::new();
    let mut other = Vec::new();
    for item in &file.items {
        if let Item::Const(item) = item {
            let name = item.ident.to_string();
            let only_docs = item.attrs.iter().all(|attr| attr.path().is_ident("doc"));
            if only_docs && is_built_artifact(&item.expr) {
                built.push(name);
            } else {
                other.push(name);
            }
        }
    }
    built.retain(|name| !other.contains(name) && bound_once(tokens, name));
    built
}

/// Exactly `include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
/// "/../neo-playwright-injected/dist/<name>.js"))`: no block around it, no transformation of it, no
/// other path.
fn is_built_artifact(expr: &Expr) -> bool {
    let Expr::Macro(include) = expr else {
        return false;
    };
    if !include.attrs.is_empty() || !include.mac.path.is_ident("include_str") {
        return false;
    }
    let Ok(Expr::Macro(concat)) = include.mac.parse_body::<Expr>() else {
        return false;
    };
    if !concat.mac.path.is_ident("concat") {
        return false;
    }
    let Ok(parts) = concat
        .mac
        .parse_body_with(Punctuated::<Expr, SynToken![,]>::parse_terminated)
    else {
        return false;
    };
    if parts.len() != 2 || parts.trailing_punct() {
        return false;
    }
    let (Some(Expr::Macro(env)), Some(Expr::Lit(path))) = (parts.first(), parts.last()) else {
        return false;
    };
    let manifest_dir = env.attrs.is_empty()
        && env.mac.path.is_ident("env")
        && env
            .mac
            .parse_body::<syn::LitStr>()
            .is_ok_and(|name| name.value() == "CARGO_MANIFEST_DIR");
    let Lit::Str(path) = &path.lit else {
        return false;
    };
    manifest_dir && is_artifact_path(&path.value())
}

/// `/../neo-playwright-injected/dist/<name>.js`, `<name>` one plain file name.
fn is_artifact_path(path: &str) -> bool {
    path.strip_prefix(BUILT_PREFIX)
        .and_then(|rest| rest.strip_suffix(".js"))
        .is_some_and(|name| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

/// Whether `name` is declared by exactly one `const` and bound by nothing else in the tokens: no
/// `let`, parameter, closure argument, `static`, `fn`, `use … as`, or binding pattern of that name
/// that would shadow the const where a `functionDeclaration` reads it.
fn bound_once(tokens: &[Token], name: &str) -> bool {
    let mut consts = 0;
    for (index, token) in tokens.iter().enumerate() {
        if !token.is(name) {
            continue;
        }
        let before = index.checked_sub(1).and_then(|at| tokens.get(at));
        if before.is_some_and(|token| token.is("const")) {
            consts += 1;
            continue;
        }
        let rebinds = before.is_some_and(|token| {
            ["let", "mut", "ref", "static", "fn", "as", "|"]
                .iter()
                .any(|word| token.is(word))
        });
        let after = tokens.get(index + 1);
        let binds = after.is_some_and(|token| token.is("@"))
            || (after.is_some_and(|token| token.is(":"))
                && !tokens.get(index + 2).is_some_and(|token| token.is(":")));
        if rebinds || binds {
            return false;
        }
    }
    consts == 1
}

/// Whether `param` is a parameter of a function whose every call in this file passes a built
/// const at its position: `fn run(&self, context: i64, function: &str)` and
/// `self.run(context, ATTACH, …)`.
fn relayed(tokens: &[Token], param: &str, built: &[String]) -> bool {
    let Some((function, position)) = parameter_of(tokens, param) else {
        return false;
    };
    let calls = call_arguments(tokens, &function);
    !calls.is_empty()
        && calls.iter().all(|arguments| {
            matches!(arguments.get(position), Some(argument) if argument.len() == 1
                && matches!(&argument[0], Token::Ident { text, .. } if built.contains(text)))
        })
}

/// The function that declares `param` and its position, `self` not counted.
fn parameter_of(tokens: &[Token], param: &str) -> Option<(String, usize)> {
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(token, Token::Ident { text, .. } if text == "fn") {
            continue;
        }
        let Some(Token::Ident { text: name, .. }) = tokens.get(index + 1) else {
            continue;
        };
        let open = tokens[index..]
            .iter()
            .position(|token| matches!(token, Token::Punct { text: '(', .. }))?;
        let parameters = split_arguments(&tokens[index + open + 1..]);
        let mut position = 0;
        for parameter in parameters {
            let is_self = parameter
                .iter()
                .any(|token| matches!(token, Token::Ident { text, .. } if text == "self"));
            if is_self {
                continue;
            }
            if matches!(parameter.first(), Some(Token::Ident { text, .. }) if text == param) {
                return Some((name.clone(), position));
            }
            position += 1;
        }
    }
    None
}

/// The argument lists of every `.function(` / `function(` call that is not its declaration.
fn call_arguments(tokens: &[Token], function: &str) -> Vec<Vec<Vec<Token>>> {
    let mut calls = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(token, Token::Ident { text, .. } if text == function) {
            continue;
        }
        if index > 0 && matches!(&tokens[index - 1], Token::Ident { text, .. } if text == "fn") {
            continue;
        }
        if matches!(tokens.get(index + 1), Some(Token::Punct { text: '(', .. })) {
            calls.push(split_arguments(&tokens[index + 2..]));
        }
    }
    calls
}

/// Splits the tokens after an opening `(` into its comma-separated items, up to its `)`.
fn split_arguments(tokens: &[Token]) -> Vec<Vec<Token>> {
    let mut items = vec![Vec::new()];
    let mut depth = 0usize;
    for token in tokens {
        match token {
            Token::Punct {
                text: '(' | '[' | '{',
                ..
            } => depth += 1,
            Token::Punct {
                text: ')' | ']' | '}',
                ..
            } if depth == 0 => break,
            Token::Punct {
                text: ')' | ']' | '}',
                ..
            } => depth -= 1,
            Token::Punct { text: ',', .. } if depth == 0 => {
                items.push(Vec::new());
                continue;
            }
            _ => {}
        }
        if let Some(item) = items.last_mut() {
            item.push(token.clone());
        }
    }
    // A trailing comma leaves one empty item; any other position keeps its place.
    if items.last().is_some_and(Vec::is_empty) {
        items.pop();
    }
    items
}

/// The tokens without any `#[cfg(test)] mod name { … }` block, whose fakes name the methods;
/// code before and after such a block stays.
fn without_test_modules(tokens: &[Token]) -> Vec<Token> {
    let attribute = ["#", "[", "cfg", "(", "test", ")", "]", "mod"];
    let mut kept = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let matches_attribute = attribute
            .iter()
            .enumerate()
            .all(|(k, want)| tokens.get(i + k).is_some_and(|token| token.is(want)));
        let opens = tokens
            .get(i + attribute.len() + 1)
            .is_some_and(|token| token.is("{"));
        if matches_attribute && opens {
            let mut depth = 0usize;
            let mut j = i + attribute.len() + 1;
            while j < tokens.len() {
                if tokens[j].is("{") {
                    depth += 1;
                } else if tokens[j].is("}") {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            i = j + 1;
            continue;
        }
        kept.push(tokens[i].clone());
        i += 1;
    }
    kept
}

#[derive(Clone, Debug)]
enum Token {
    Str { text: String, line: usize },
    Ident { text: String, line: usize },
    Punct { text: char, line: usize },
}

impl Token {
    /// Whether this is the identifier or punctuation `text`.
    fn is(&self, text: &str) -> bool {
        match self {
            Self::Ident { text: ident, .. } => ident == text,
            Self::Punct { text: punct, .. } => text.chars().eq([*punct]),
            Self::Str { .. } => false,
        }
    }

    fn line(&self) -> usize {
        match self {
            Self::Str { line, .. } | Self::Ident { line, .. } | Self::Punct { line, .. } => *line,
        }
    }
}

/// The tokens of `source` as the compiler reads them: every string literal (plain, raw, byte or
/// C) by its decoded value, identifiers, and punctuation with each delimiter as its own token.
/// Comments and doc attributes (what `///` and `//!` become) are prose and dropped; other
/// literals carry no code. `None` when `source` does not tokenize as Rust.
fn lex(source: &str) -> Option<Vec<Token>> {
    let stream = TokenStream::from_str(source).ok()?;
    let mut tokens = Vec::new();
    flatten(stream, &mut tokens);
    Some(tokens)
}

fn flatten(stream: TokenStream, tokens: &mut Vec<Token>) {
    let trees: Vec<TokenTree> = stream.into_iter().collect();
    let mut index = 0;
    while index < trees.len() {
        if let Some(span) = doc_attribute(&trees[index..]) {
            index += span;
            continue;
        }
        match &trees[index] {
            TokenTree::Group(group) => {
                let delimiters = match group.delimiter() {
                    Delimiter::Parenthesis => Some(('(', ')')),
                    Delimiter::Brace => Some(('{', '}')),
                    Delimiter::Bracket => Some(('[', ']')),
                    Delimiter::None => None,
                };
                if let Some((open, _)) = delimiters {
                    tokens.push(Token::Punct {
                        text: open,
                        line: line_of(group.span_open()),
                    });
                }
                flatten(group.stream(), tokens);
                if let Some((_, close)) = delimiters {
                    tokens.push(Token::Punct {
                        text: close,
                        line: line_of(group.span_close()),
                    });
                }
            }
            TokenTree::Ident(ident) => tokens.push(Token::Ident {
                text: ident.to_string(),
                line: line_of(ident.span()),
            }),
            TokenTree::Punct(punct) => tokens.push(Token::Punct {
                text: punct.as_char(),
                line: line_of(punct.span()),
            }),
            TokenTree::Literal(literal) => {
                if let Some(text) = string_value(literal) {
                    tokens.push(Token::Str {
                        text,
                        line: line_of(literal.span()),
                    });
                }
            }
        }
        index += 1;
    }
}

/// How many trees a doc attribute at the start of `trees` spans (`#[doc …]` or `#![doc …]`).
fn doc_attribute(trees: &[TokenTree]) -> Option<usize> {
    let Some(TokenTree::Punct(hash)) = trees.first() else {
        return None;
    };
    if hash.as_char() != '#' {
        return None;
    }
    let bang = matches!(trees.get(1), Some(TokenTree::Punct(bang)) if bang.as_char() == '!');
    let at = if bang { 2 } else { 1 };
    let Some(TokenTree::Group(group)) = trees.get(at) else {
        return None;
    };
    let doc = group.delimiter() == Delimiter::Bracket
        && matches!(group.stream().into_iter().next(), Some(TokenTree::Ident(ident)) if ident == "doc");
    doc.then_some(at + 1)
}

/// A string literal's value as the compiler reads it; a literal `syn` does not classify is read
/// as written, so nothing unreadable passes. `None` for numbers, chars and bytes.
fn string_value(literal: &Literal) -> Option<String> {
    match Lit::new(literal.clone()) {
        Lit::Str(text) => Some(text.value()),
        Lit::ByteStr(bytes) => Some(String::from_utf8_lossy(&bytes.value()).into_owned()),
        Lit::CStr(text) => Some(text.value().to_string_lossy().into_owned()),
        Lit::Verbatim(other) => Some(other.to_string()),
        _ => None,
    }
}

fn line_of(span: Span) -> usize {
    span.start().line
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn tests_that_only_init_scripts_rs_adds_init_scripts_and_never_names_a_world() {
    for method in [
        "Page.addScriptToEvaluateOnNewDocument",
        "Page.removeScriptToEvaluateOnNewDocument",
    ] {
        let send = format!("cdp.send(\"{method}\", json!({{\"source\": source}}), Some(session))");
        assert!(
            check(INIT_SCRIPTS, &send).is_empty(),
            "{method} in init_scripts.rs"
        );
        assert!(
            !check("neo-runtime/src/browser_representations/page.rs", &send).is_empty(),
            "{method} elsewhere"
        );
        assert!(
            !check("neo-runtime/src/chromium_bridge/main_world.rs", &send).is_empty(),
            "{method} in main_world.rs"
        );
    }
    for world in [
        "json!({\"source\": source, \"worldName\": name})",
        "let request = Params { world_name: Some(name) };",
    ] {
        assert!(
            !check(INIT_SCRIPTS, world).is_empty(),
            "not refused: {world}"
        );
    }
    assert!(
        !check(INIT_SCRIPTS, "cdp.send(\"Runtime.evaluate\", params, None)").is_empty(),
        "init_scripts.rs may not evaluate"
    );
}

#[test]
fn tests_that_only_bindings_rs_adds_and_removes_the_binding_and_runs_no_script() {
    for method in ["Runtime.addBinding", "Runtime.removeBinding"] {
        let send = format!("cdp.send(\"{method}\", json!({{\"name\": NAME}}), Some(session))");
        assert!(check(BINDINGS, &send).is_empty(), "{method} in bindings.rs");
        for elsewhere in [
            INIT_SCRIPTS,
            "neo-runtime/src/browser_representations/page.rs",
            "neo-runtime/src/chromium_bridge/main_world.rs",
        ] {
            assert!(
                !check(elsewhere, &send).is_empty(),
                "{method} in {elsewhere}"
            );
        }
    }
    for script in [
        "cdp.send(\"Runtime.evaluate\", params, Some(session))",
        "cdp.send(\"Runtime.callFunctionOn\", params, Some(session))",
        "cdp.send(\"Page.addScriptToEvaluateOnNewDocument\", params, Some(session))",
    ] {
        assert!(
            !check(BINDINGS, script).is_empty(),
            "bindings.rs may not run script: {script}"
        );
    }
}

#[test]
fn tests_that_an_escaped_literal_is_read_as_its_value() {
    let source = r#"const EVAL: &str = "\x52untime.evaluate";"#;
    assert!(
        !check("neo-runtime/src/probe.rs", source).is_empty(),
        "the Rust literal evaluates to Runtime.evaluate"
    );
    for literal in [
        r#""\u{52}untime.evaluate""#,
        r##"r#"Runtime.evaluate"#"##,
        r#"b"Runtime.evaluate""#,
        r#"c"Runtime.evaluate""#,
        "\"Runtime.\\\n        evaluate\"",
        r#"concat!("Run", "time.evaluate")"#,
        r#"concat!("Runtime.eva", "luate")"#,
    ] {
        let source = format!("const EVAL: &str = {literal};");
        assert!(
            !check("neo-runtime/src/probe.rs", &source).is_empty(),
            "not refused: {source}"
        );
    }
}

#[test]
fn tests_that_only_an_exact_artifact_include_is_a_built_function() {
    let source = r#"const GLUE: &str = {
        let _ = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../neo-playwright-injected/dist/evaluate.js"));
        "function () { return globalThis.secret; }"
    };
    fn probe() { json!({"functionDeclaration": GLUE}); }"#;
    assert!(
        !check("neo-runtime/src/chromium_bridge/main_world.rs", source).is_empty(),
        "the const mentions an include_str but evaluates to another string"
    );
    let built = r#"include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../neo-playwright-injected/dist/evaluate.js"))"#;
    let accepted = format!(
        "const GLUE: &str = {built};\nfn probe() {{ json!({{\"functionDeclaration\": GLUE}}); }}"
    );
    assert!(check("neo-runtime/src/chromium_bridge/main_world.rs", &accepted).is_empty());
    for initializer in [
        r#"include_str!("/../neo-playwright-injected/dist/evaluate.js")"#,
        r#"include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../neo-playwright-injected/dist/../evil.js"))"#,
        r#"include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../neo-playwright-injected/dist/sub/x.js"))"#,
        r#"include_str!(concat!(env!("OUT_DIR"), "/../neo-playwright-injected/dist/evaluate.js"))"#,
        r#"include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../neo-playwright-injected/dist/evaluate.js", ""))"#,
        r#"include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../neo-playwright-injected/dist/evaluate.js")).trim_start()"#,
    ] {
        let source = format!(
            "const GLUE: &str = {initializer};\nfn probe() {{ json!({{\"functionDeclaration\": GLUE}}); }}"
        );
        assert!(
            !check("neo-runtime/src/chromium_bridge/main_world.rs", &source).is_empty(),
            "not refused: {initializer}"
        );
    }
    for shadow in [
        "fn probe() { let GLUE = \"() => 1\"; json!({\"functionDeclaration\": GLUE}); }",
        "fn probe(GLUE: &str) { json!({\"functionDeclaration\": GLUE}); }",
        "fn probe() { const GLUE: &str = \"() => 1\"; json!({\"functionDeclaration\": GLUE}); }",
        "#[cfg(any())]\nconst GLUE: &str = \"() => 1\";\nfn probe() { json!({\"functionDeclaration\": GLUE}); }",
    ] {
        let source = format!("const GLUE: &str = {built};\n{shadow}");
        assert!(
            !check("neo-runtime/src/chromium_bridge/main_world.rs", &source).is_empty(),
            "a re-bound name passed: {shadow}"
        );
    }
}

#[test]
fn a_computed_init_world_name_is_refused() {
    let source = r#"const KEY: &str = concat!("world", "Name");
        cdp.send("Page.addScriptToEvaluateOnNewDocument", json!({"source": source, (KEY): name}), Some(session));"#;
    let problems = check(INIT_SCRIPTS, source);
    assert!(
        !problems.is_empty(),
        "init scripts must not select a world: {problems:?}"
    );
}
