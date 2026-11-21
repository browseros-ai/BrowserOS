//! The module layering of DESIGN.md section 3, enforced by parsing `src/` with `syn`.
//!
//! A module may import only modules strictly below it in this order:
//!
//! ```text
//!   app > http > conversations > agent > {model, browser} > store > turn
//! ```
//!
//! plus the rules that keep one owner per piece of state and one home per wire format:
//!
//! - `http` may name `crate::app::App` and `crate::app::Launch` (the handlers' state), nothing
//!   else of `app`;
//! - `Store::conversations()` and `ConversationRows` only in `conversations` (its one writer);
//! - `Store::tokens()` and `Tokens` only in `model::chatgpt` (the sign-in's one writer);
//! - `utoipa` only in `http`; `sea_orm` only in `store`; `genai` and the AWS SDK only in
//!   `model`; `neo_server` and `rmcp` only in `browser`; `axum` only in `http`, `app`, `browser`
//!   and the private sign-in callback in `model::chatgpt`;
//! - `turn` is pure: no `tokio`, `axum`, `reqwest`, `sea_orm`, `std::fs` or `std::net`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use syn::visit::{self, Visit};

/// Rank in the layering: a module may import only lower ranks.
fn rank(module: &str) -> Option<u8> {
    Some(match module {
        "turn" => 0,
        "store" => 1,
        "model" | "browser" => 2,
        "agent" => 3,
        "conversations" => 4,
        "http" => 5,
        "app" => 6,
        _ => return None,
    })
}

/// External crates and the modules allowed to use them.
const CRATE_HOMES: &[(&str, &[&str])] = &[
    ("utoipa", &["http"]),
    ("utoipa_axum", &["http"]),
    ("sea_orm", &["store"]),
    ("sea_orm_migration", &["store"]),
    ("genai", &["model"]),
    ("aws_config", &["model"]),
    ("aws_sdk_bedrockruntime", &["model"]),
    ("aws_smithy_types", &["model"]),
    ("neo_server", &["browser"]),
    ("rmcp", &["browser"]),
    ("axum", &["http", "app", "browser"]),
];

/// What `turn` may not touch, by path prefix.
const IMPURE: &[&[&str]] = &[
    &["tokio"],
    &["axum"],
    &["reqwest"],
    &["sea_orm"],
    &["std", "fs"],
    &["std", "net"],
];

#[derive(Default)]
struct Uses {
    paths: BTreeSet<Vec<String>>,
    methods: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for Uses {
    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        collect_use_tree(&node.tree, &mut Vec::new(), &mut self.paths);
        visit::visit_item_use(self, node);
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        self.paths
            .insert(node.segments.iter().map(|s| s.ident.to_string()).collect());
        visit::visit_path(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        self.methods.insert(node.method.to_string());
        visit::visit_expr_method_call(self, node);
    }
}

fn collect_use_tree(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    out: &mut BTreeSet<Vec<String>>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_tree(&path.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let mut full = prefix.clone();
            full.push(name.ident.to_string());
            out.insert(full);
        }
        syn::UseTree::Rename(rename) => {
            let mut full = prefix.clone();
            full.push(rename.ident.to_string());
            out.insert(full);
        }
        syn::UseTree::Glob(_) => {
            out.insert(prefix.clone());
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_tree(item, prefix, out);
            }
        }
    }
}

/// The module path of a source file: `src/model/chatgpt.rs` is `["model", "chatgpt"]`.
fn module_path(relative: &Path) -> Vec<String> {
    let mut parts: Vec<String> = relative
        .iter()
        .map(|part| part.to_string_lossy().into_owned())
        .collect();
    if let Some(last) = parts.pop() {
        let stem = last.trim_end_matches(".rs");
        if stem != "mod" && stem != "lib" && stem != "main" {
            parts.push(stem.to_owned());
        }
    }
    parts
}

/// Resolves `crate::`, `super::` and `self::` against the file's module path; other paths are
/// returned as written (an external crate, a std path, or a local name).
fn resolve(path: &[String], module: &[String]) -> Vec<String> {
    match path.first().map(String::as_str) {
        Some("crate") => path[1..].to_vec(),
        Some("super") => {
            let mut base = module.to_vec();
            let mut rest = path;
            while rest.first().map(String::as_str) == Some("super") {
                base.pop();
                rest = &rest[1..];
            }
            base.extend_from_slice(rest);
            base
        }
        Some("self") => {
            let mut base = module.to_vec();
            base.extend_from_slice(&path[1..]);
            base
        }
        _ => path.to_vec(),
    }
}

fn violations(relative: &str, source: &str) -> Vec<String> {
    let module = module_path(Path::new(relative));
    let Some(top) = module.first().cloned() else {
        return Vec::new(); // lib.rs and main.rs: the crate root may name any module.
    };
    let Ok(file) = syn::parse_file(source) else {
        return vec![format!("{relative}: does not parse")];
    };
    let mut uses = Uses::default();
    uses.visit_file(&file);
    let mut found = Vec::new();
    let from = rank(&top);
    let in_chatgpt = relative == "model/chatgpt.rs";
    let local_modules = [
        "turn",
        "store",
        "model",
        "browser",
        "agent",
        "conversations",
        "http",
        "app",
    ];

    for raw in &uses.paths {
        let crate_relative = matches!(
            raw.first().map(String::as_str),
            Some("crate" | "super" | "self")
        );
        let path = resolve(raw, &module);
        let Some(first) = path.first() else { continue };

        if crate_relative && local_modules.contains(&first.as_str()) && *first != top {
            let allowed = match (from, rank(first)) {
                (Some(from), Some(to)) => to < from,
                // A crate-root module such as `test_support` may use anything.
                (None, _) => true,
                (_, None) => true,
            };
            let app_state = top == "http"
                && first == "app"
                && matches!(path.get(1).map(String::as_str), Some("App" | "Launch"));
            if !allowed && !app_state {
                found.push(format!(
                    "{relative}: `{top}` may not import `{}`",
                    path.join("::")
                ));
            }
        }
        if !crate_relative {
            // The private callback on 127.0.0.1:1455 stays in chatgpt.rs so its port,
            // client id, and tokens stay there too (DESIGN.md 3.6).
            if let Some((name, homes)) = CRATE_HOMES.iter().find(|(name, _)| first == name)
                && !homes.contains(&top.as_str())
                && !(*name == "axum" && in_chatgpt)
            {
                found.push(format!(
                    "{relative}: `{name}` belongs in {homes:?}, not `{top}`"
                ));
            }
            if top == "turn" {
                for impure in IMPURE {
                    if path.len() >= impure.len()
                        && path.iter().zip(impure.iter()).all(|(a, b)| a == b)
                    {
                        found.push(format!(
                            "{relative}: `turn` is pure; it may not use `{}`",
                            path.join("::")
                        ));
                    }
                }
            }
        }
        if path.iter().any(|segment| segment == "ConversationRows")
            && top != "conversations"
            && top != "store"
        {
            found.push(format!(
                "{relative}: only `conversations` writes conversation rows"
            ));
        }
        let in_chatgpt = module.len() >= 2 && module[0] == "model" && module[1] == "chatgpt";
        if path
            .iter()
            .any(|segment| segment == "Tokens" || segment == "TokenRow")
            && top != "store"
            && !in_chatgpt
        {
            found.push(format!(
                "{relative}: only `model::chatgpt` touches the token rows"
            ));
        }
    }
    if uses.methods.contains("conversations") && top != "conversations" && top != "store" {
        found.push(format!(
            "{relative}: only `conversations` may call `Store::conversations()`"
        ));
    }
    let in_chatgpt = module.len() >= 2 && module[0] == "model" && module[1] == "chatgpt";
    if uses.methods.contains("tokens") && top != "store" && !in_chatgpt {
        found.push(format!(
            "{relative}: only `model::chatgpt` may call `Store::tokens()`"
        ));
    }
    found
}

#[test]
fn only_chatgpt_can_host_a_private_callback_from_model() {
    let listener = "use axum::Router;";
    assert!(violations("model/chatgpt.rs", listener).is_empty());
    assert!(!violations("model/mod.rs", listener).is_empty());
    assert!(!violations("model/responses.rs", listener).is_empty());
    assert!(!violations("model/chatgpt.rs", "use crate::http::router;").is_empty());
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

#[test]
fn the_source_tree_respects_the_layering() -> Result<(), Box<dyn std::error::Error>> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files)?;
    files.sort();
    let mut found = Vec::new();
    for file in &files {
        let relative = file
            .strip_prefix(&src)?
            .to_string_lossy()
            .replace('\\', "/");
        found.extend(violations(&relative, &std::fs::read_to_string(file)?));
    }
    assert!(
        found.is_empty(),
        "layering violations:\n{}",
        found.join("\n")
    );
    assert!(files.len() > 30, "the walk found the source tree");
    Ok(())
}

#[test]
fn an_upward_import_is_caught() {
    assert!(!violations("store/providers.rs", "use crate::model::Models;").is_empty());
    assert!(!violations("turn/mod.rs", "use crate::store::Store;").is_empty());
    assert!(!violations("model/genai_route.rs", "use crate::browser::Browser;").is_empty());
    assert!(
        !violations(
            "agent/acpx.rs",
            "use super::super::conversations::Conversations;"
        )
        .is_empty()
    );
}

#[test]
fn downward_imports_and_the_app_state_edge_pass() {
    assert!(
        violations(
            "agent/own_loop.rs",
            "use crate::{browser::Browser, model::Models, store::Store, turn::TurnEvent};"
        )
        .is_empty()
    );
    assert!(violations("http/chat.rs", "use crate::app::App;").is_empty());
    assert!(!violations("http/chat.rs", "use crate::app::main;").is_empty());
}

#[test]
fn single_writers_and_wire_homes_are_enforced() {
    assert!(!violations("agent/mod.rs", "fn f(s: &Store) { s.conversations(); }").is_empty());
    assert!(
        violations(
            "conversations/mod.rs",
            "fn f(s: &Store) { s.conversations(); }"
        )
        .is_empty()
    );
    assert!(!violations("model/mod.rs", "fn f(s: &Store) { s.tokens(); }").is_empty());
    assert!(violations("model/chatgpt.rs", "fn f(s: &Store) { s.tokens(); }").is_empty());
    assert!(!violations("store/mod.rs", "use utoipa::ToSchema;").is_empty());
    assert!(!violations("agent/mod.rs", "use sea_orm::DatabaseConnection;").is_empty());
    assert!(!violations("turn/transcript.rs", "fn f() { std::fs::read(\"x\"); }").is_empty());
    assert!(!violations("turn/transcript.rs", "use tokio::sync::Mutex;").is_empty());
}
