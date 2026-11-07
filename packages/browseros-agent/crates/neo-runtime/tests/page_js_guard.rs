//! Neo's page JavaScript guard: which Neo file may reach which page world, and with what code.
//!
//! The owner's constraint: page JavaScript never bypasses one narrow, deliberate path. In Neo
//! that path is two files of `neo-runtime`:
//!
//! - `src/world.rs`, Neo's isolated (utility) world: `Page.createIsolatedWorld` and
//!   `Runtime.callFunctionOn`, for every read and action.
//! - `src/main_world.rs`, the page's own world, for Playwright's evaluations: `Runtime.callFunctionOn`
//!   only, on the `UtilityScript` it builds there.
//!
//! In both, every `functionDeclaration` is a `const` that is `include_str!` of a built
//! `neo-runtime-injected/dist/*.js` artifact (or a parameter every caller in the file passes one
//! of those consts), so agent code can reach a page only as an argument value.
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

/// The methods that run script or reach a world of the page.
const GUARDED: &[&str] = &[
    "Runtime.evaluate",
    "Runtime.callFunctionOn",
    "Runtime.compileScript",
    "Runtime.runScript",
    "Runtime.addBinding",
    "Page.addScriptToEvaluateOnNewDocument",
    "Page.createIsolatedWorld",
    "Debugger.evaluateOnCallFrame",
];

/// The files that may name guarded methods, each with the methods it may name. Paths are
/// relative to `crates/`.
const ALLOWED: &[(&str, &[&str])] = &[
    (
        "neo-runtime/src/world.rs",
        &["Runtime.callFunctionOn", "Page.createIsolatedWorld"],
    ),
    ("neo-runtime/src/main_world.rs", &["Runtime.callFunctionOn"]),
];

/// Where a built artifact lives: the only source a `functionDeclaration` may have.
const BUILT_PREFIX: &str = "/../neo-runtime-injected/dist/";

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
        "neo-runtime/src/dispatch.rs",
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
        let problems = check("neo-runtime/src/page.rs", line);
        assert!(!problems.is_empty(), "not refused: {line}");
    }
}

#[test]
fn tests_that_a_method_is_refused_outside_its_file() {
    let send = "self.send(\"Runtime.callFunctionOn\", request).await";
    assert!(check("neo-runtime/src/main_world.rs", send).is_empty());
    assert!(check("neo-runtime/src/world.rs", send).is_empty());
    assert!(!check("neo-runtime/src/evaluate.rs", send).is_empty());
    assert!(!check("neo-runtime/src/page.rs", send).is_empty());
    assert!(!check("neo-sdk/src/x.rs", send).is_empty());
    let world = "self.send(\"Page.createIsolatedWorld\", params)";
    assert!(check("neo-runtime/src/world.rs", world).is_empty());
    assert!(
        !check("neo-runtime/src/main_world.rs", world).is_empty(),
        "main_world.rs reaches the page's own world only"
    );
    for method in ["Runtime.evaluate", "Runtime.addBinding", "Runtime.runScript"] {
        let line = format!("self.send(\"{method}\", params)");
        assert!(
            !check("neo-runtime/src/main_world.rs", &line).is_empty(),
            "{method} in main_world.rs"
        );
        assert!(
            !check("neo-runtime/src/world.rs", &line).is_empty(),
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
    assert!(check("neo-runtime/src/page.rs", source).is_empty());
    let after = format!("{source}const Y: &str = \"Runtime.evaluate\";\n");
    assert_eq!(
        check("neo-runtime/src/page.rs", &after).len(),
        1,
        "code after a test module is still code"
    );
}

#[test]
fn tests_that_a_function_declaration_must_be_a_built_artifact() {
    let built = "const GLUE: &str = include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../neo-runtime-injected/dist/call.js\"));\n";
    let ok = format!("{built}fn f() {{ json!({{\"functionDeclaration\": GLUE}}); }}\n");
    assert!(check("neo-runtime/src/main_world.rs", &ok).is_empty());

    let literal = format!(
        "{built}fn f() {{ json!({{\"functionDeclaration\": \"() => document.title\"}}); }}\n"
    );
    assert!(!check("neo-runtime/src/main_world.rs", &literal).is_empty());

    let other_const = "const JS: &str = \"() => 1\";\nfn f() { json!({\"functionDeclaration\": JS}); }\n";
    assert!(!check("neo-runtime/src/world.rs", other_const).is_empty());

    let wrong_dir = "const JS: &str = include_str!(\"../payload.js\");\nfn f() { json!({\"functionDeclaration\": JS}); }\n";
    assert!(!check("neo-runtime/src/world.rs", wrong_dir).is_empty());

    let relay_ok = format!(
        "{built}impl W {{ async fn run(&self, context: i64, function: &str) {{ json!({{\"functionDeclaration\": function}}); }}\n\
         async fn a(&self) {{ self.run(1, GLUE).await; }} }}\n"
    );
    assert!(check("neo-runtime/src/world.rs", &relay_ok).is_empty());
    let relay_bad = format!(
        "{built}impl W {{ async fn run(&self, context: i64, function: &str) {{ json!({{\"functionDeclaration\": function}}); }}\n\
         async fn a(&self, js: &str) {{ self.run(1, js).await; }} }}\n"
    );
    assert!(!check("neo-runtime/src/world.rs", &relay_bad).is_empty());

    let elsewhere = "fn f() { json!({\"functionDeclaration\": X}); }\n";
    assert!(!check("neo-runtime/src/evaluate.rs", elsewhere).is_empty());
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
    let tokens = without_test_modules(lex(source));
    let mut problems = Vec::new();
    for token in &tokens {
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
        if composes_method(text) {
            problems.push(format!(
                "{relative}:{line}: {text:?} composes a guarded CDP method name"
            ));
        }
    }
    problems.extend(declarations(relative, &tokens, !allowed.is_empty()));
    problems
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
fn declarations(relative: &str, tokens: &[Token], door: bool) -> Vec<String> {
    let mut problems = Vec::new();
    let built = built_consts(tokens);
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
        let value = match (tokens.get(index + 1), tokens.get(index + 2)) {
            (Some(Token::Punct { text: ':', .. }), Some(value)) => value,
            _ => {
                problems.push(format!(
                    "{relative}:{line}: a functionDeclaration whose value the guard cannot read"
                ));
                continue;
            }
        };
        let after = tokens.get(index + 3);
        let ends = matches!(after, Some(Token::Punct { text: ',' | '}', .. }) | None);
        match value {
            Token::Ident { text, .. } if ends && built.iter().any(|name| name == text) => {}
            Token::Ident { text, .. } if ends && relayed(tokens, text, &built) => {}
            _ => problems.push(format!(
                "{relative}:{line}: a functionDeclaration that is not a built neo-runtime-injected/dist/*.js const"
            )),
        }
    }
    problems
}

/// `const NAME: &str = include_str!(…"/../neo-runtime-injected/dist/<file>.js"…);`, by name.
fn built_consts(tokens: &[Token]) -> Vec<String> {
    let mut names = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(token, Token::Ident { text, .. } if text == "const") {
            continue;
        }
        let Some(Token::Ident { text: name, .. }) = tokens.get(index + 1) else {
            continue;
        };
        let Some(end) = tokens[index..]
            .iter()
            .position(|token| matches!(token, Token::Punct { text: ';', .. }))
        else {
            continue;
        };
        let body = &tokens[index..index + end];
        let includes = body
            .iter()
            .any(|token| matches!(token, Token::Ident { text, .. } if text == "include_str"));
        let path = body.iter().find_map(|token| match token {
            Token::Str { text, .. } if text.ends_with(".js") => Some(text.as_str()),
            _ => None,
        });
        if includes && path.is_some_and(|path| path.starts_with(BUILT_PREFIX)) {
            names.push(name.clone());
        }
    }
    names
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
            Token::Punct { text: '(' | '[' | '{', .. } => depth += 1,
            Token::Punct { text: ')' | ']' | '}', .. } if depth == 0 => break,
            Token::Punct { text: ')' | ']' | '}', .. } => depth -= 1,
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
fn without_test_modules(tokens: Vec<Token>) -> Vec<Token> {
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

/// A small Rust lexer: string literals (escaped, raw, byte), identifiers and punctuation, with
/// comments, char literals, lifetimes and numbers skipped.
fn lex(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut line = 1;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if c == '"' || (c == 'b' && chars.get(i + 1) == Some(&'"')) {
            let start_line = line;
            i += if c == 'b' { 2 } else { 1 };
            let mut text = String::new();
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    match chars.get(i + 1) {
                        Some('n') => text.push('\n'),
                        Some('t') => text.push('\t'),
                        Some('"') => text.push('"'),
                        Some('\\') => text.push('\\'),
                        Some('\n') => {
                            line += 1;
                            i += 2;
                            while i < chars.len() && chars[i].is_whitespace() {
                                if chars[i] == '\n' {
                                    line += 1;
                                }
                                i += 1;
                            }
                            continue;
                        }
                        Some(other) => text.push(*other),
                        None => {}
                    }
                    i += 2;
                } else {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    text.push(chars[i]);
                    i += 1;
                }
            }
            i += 1;
            tokens.push(Token::Str {
                text,
                line: start_line,
            });
        } else if (c == 'r' || c == 'b')
            && raw_string_start(&chars, i).is_some()
        {
            let (hashes, body) = raw_string_start(&chars, i).unwrap_or((0, i));
            let start_line = line;
            let mut j = body;
            let mut text = String::new();
            loop {
                if j >= chars.len() {
                    break;
                }
                if chars[j] == '"'
                    && (0..hashes).all(|k| chars.get(j + 1 + k) == Some(&'#'))
                {
                    j += 1 + hashes;
                    break;
                }
                if chars[j] == '\n' {
                    line += 1;
                }
                text.push(chars[j]);
                j += 1;
            }
            i = j;
            tokens.push(Token::Str {
                text,
                line: start_line,
            });
        } else if c == '\'' {
            // A char literal (`'a'`, `'\n'`, `'\''`) or a lifetime (`'a`): skip either.
            if chars.get(i + 1) == Some(&'\\') {
                i += 2;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if chars.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                i += 1;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
            }
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Ident {
                text: chars[start..i].iter().collect(),
                line,
            });
        } else if c.is_ascii_digit() {
            // A number is one argument token, as an identifier would be.
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                i += 1;
            }
            tokens.push(Token::Ident {
                text: chars[start..i].iter().collect(),
                line,
            });
        } else {
            tokens.push(Token::Punct { text: c, line });
            i += 1;
        }
    }
    tokens
}

/// `r"`, `r#"`, `br"`, `br#"` at `i`: the number of `#`s and where the body starts.
fn raw_string_start(chars: &[char], i: usize) -> Option<(usize, usize)> {
    let mut j = i;
    if chars.get(j) == Some(&'b') {
        j += 1;
    }
    if chars.get(j) != Some(&'r') {
        return None;
    }
    if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
        return None;
    }
    j += 1;
    let mut hashes = 0;
    while chars.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    (chars.get(j) == Some(&'"')).then_some((hashes, j + 1))
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
