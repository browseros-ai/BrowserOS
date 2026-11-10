//! STYLE.md rules that no compiler lint checks, enforced as a test over claw-sdk's own source.
//!
//! Copy to `crates/neo-sdk/neo-server/tests/style_boundaries.rs`. Dev-dependencies: `syn` (features `full`,
//! `visit`) and `toml`.
//!
//! The checks parse `src/` with `syn`, without type information, so every rule is phrased in what
//! a parser can see: paths a file names, the file layout, and attributes. Code inside macro bodies
//! (`json!`, `format!`, `tokio::select!`) is not parsed and not checked. The dispatcher's "one front
//! door" needs types, so it is enforced by visibility in `dispatch/` instead (STYLE.md, rule P2);
//! this test only keeps the entry points away from the browser crates, and the door from naming
//! the modules whose methods and rules it runs.

use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::path::{Component, Path, PathBuf};

use syn::visit::{self, Visit};
use syn::{AttrStyle, Attribute, Fields, File, Item, ItemEnum, ItemStruct, ItemUse, UseTree};
use toml::{Table, Value};

/// A crate that only one subtree of `src/` may name.
struct CrateHome {
    krate: &'static str,
    subtree: &'static str,
    why: &'static str,
}

/// STYLE.md P1: only `runtime/` touches the JS engine, so it stays swappable (skeleton D7, D8).
const CRATE_HOMES: &[CrateHome] = &[CrateHome {
    krate: "rquickjs",
    subtree: "runtime",
    why: "only runtime/ imports the JS engine (STYLE.md P1)",
}];

/// STYLE.md P2: the entry points reach the browser only through `Dispatcher::submit`, so they
/// never name a crate that drives it.
const ENTRY_POINTS: &[&str] = &["mcp", "http", "runtime"];
const BROWSER_CRATES: &[&str] = &[
    "browseros_cdp",
    "browseros_core",
    "neo_playwright",
    "neo_captcha",
];

/// STYLE.md P2: the door runs every method as a registered `Method` and every rule as a `Policy`,
/// so `dispatch/` never names the modules that implement them. They depend on the door, never the
/// other way round: a door that named a built-in could reach it around its own registry.
const DOOR: &str = "dispatch";
const BEHIND_THE_DOOR: &[&str] = &["builtins", "host"];

/// STYLE.md P4: types that hold a secret or a raw field value. A type with such a field must not
/// derive `Debug` or `Serialize`; it writes a `Debug` that prints a fixed mask.
const SECRET_TYPES: &[&str] = &["Zeroizing", "SecretValue", "FieldValues", "SecretString"];
const LEAKING_DERIVES: &[&str] = &["Debug", "Serialize"];

/// Leaf files whose primary item is legitimately not named after the file, with the reason.
/// Google allows exceptions; each one is written down here instead of being argued in review.
const LEAF_NAME_EXCEPTIONS: &[(&str, &str)] = &[];

/// Non-leaf files allowed to hold a small amount of "ties things together" code (Google allows
/// it), with the reason.
const NON_LEAF_CODE_EXCEPTIONS: &[(&str, &str)] = &[];

#[test]
fn source_follows_style_boundaries() -> Result<(), Box<dyn Error>> {
    let violations = check_crate(Path::new(env!("CARGO_MANIFEST_DIR")))?;
    assert!(
        violations.is_empty(),
        "STYLE.md violations (see crates/neo-sdk/STYLE.md):\n{}",
        violations.join("\n")
    );
    Ok(())
}

#[test]
fn lint_table_matches_workspace_and_twin() -> Result<(), Box<dyn Error>> {
    let violations = check_lint_tables(Path::new(env!("CARGO_MANIFEST_DIR")))?;
    assert!(
        violations.is_empty(),
        "lint table drift:\n{}",
        violations.join("\n")
    );
    Ok(())
}

/// Runs every source rule over `<manifest_dir>/src` and returns one line per violation.
pub(crate) fn check_crate(manifest_dir: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let src = manifest_dir.join("src");
    let mut files = Vec::new();
    collect_rust_files(&src, &mut files)?;
    files.sort();

    let mut violations = Vec::new();
    for path in &files {
        let relative = path.strip_prefix(&src)?;
        let shown = format!("src/{}", relative.display());
        let file = syn::parse_file(&fs::read_to_string(path)?)?;
        let module = ModuleFile::classify(&src, relative);

        let mut names = NamedPaths::default();
        names.visit_file(&file);
        check_crate_homes(&shown, &module, &names, &mut violations);
        check_entry_points(&shown, &module, &names, &mut violations);
        check_door(&shown, &module, &names, &mut violations);
        check_module_doc(&shown, &module, &file, &mut violations);
        check_non_leaf_items(&shown, &module, &file, &mut violations);
        check_leaf_name(&shown, &module, &file, &mut violations);

        let mut secrets = SecretDerives {
            file: &shown,
            violations: &mut violations,
        };
        secrets.visit_file(&file);
    }
    Ok(violations)
}

/// Where a file sits in the module tree, which decides the rules that apply to it.
struct ModuleFile {
    /// First path component without `.rs`: `runtime` for `runtime.rs` and `runtime/bridge.rs`.
    subtree: String,
    /// The module's own name: the file stem, or the directory name for a `mod.rs`.
    name: String,
    kind: ModuleKind,
}

#[derive(PartialEq, Eq)]
enum ModuleKind {
    /// `lib.rs`: a non-leaf that is also the crate root.
    LibRoot,
    /// `main.rs`: the binary's entry point. Code, not structure.
    BinRoot,
    /// A module with submodules: `dispatch.rs` beside `dispatch/`.
    NonLeaf,
    /// A module without submodules, named after its primary item.
    Leaf,
    /// `tests.rs` / `test.rs`, a `#[cfg(test)] mod tests;` file (Google allows these in leaves).
    Tests,
}

impl ModuleFile {
    fn classify(src: &Path, relative: &Path) -> Self {
        let parts: Vec<String> = relative
            .components()
            .filter_map(|part| match part {
                Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        let file_name = parts.last().cloned().unwrap_or_default();
        let stem = file_name.trim_end_matches(".rs").to_owned();
        let subtree = parts.first().map_or_else(String::new, |first| {
            first.trim_end_matches(".rs").to_owned()
        });
        let parent_dir = relative
            .parent()
            .map_or_else(|| src.to_path_buf(), |parent| src.join(parent));

        let (name, kind) = match (parts.len(), stem.as_str()) {
            (1, "lib") => (stem, ModuleKind::LibRoot),
            (1, "main") => (stem, ModuleKind::BinRoot),
            (_, "tests" | "test") => (stem, ModuleKind::Tests),
            (_, "mod") => {
                let dir_name = parts.iter().rev().nth(1).cloned().unwrap_or_default();
                let has_children = fs::read_dir(&parent_dir)
                    .is_ok_and(|entries| entries.filter_map(Result::ok).count() > 1);
                let kind = if has_children {
                    ModuleKind::NonLeaf
                } else {
                    ModuleKind::Leaf
                };
                (dir_name, kind)
            }
            _ => {
                let kind = if parent_dir.join(&stem).is_dir() {
                    ModuleKind::NonLeaf
                } else {
                    ModuleKind::Leaf
                };
                (stem, kind)
            }
        };
        Self {
            subtree,
            name,
            kind,
        }
    }
}

/// STYLE.md P1: a crate with a home may be named only inside that subtree.
fn check_crate_homes(shown: &str, module: &ModuleFile, names: &NamedPaths, out: &mut Vec<String>) {
    for home in CRATE_HOMES {
        if module.subtree == home.subtree {
            continue;
        }
        for path in names.with_root(home.krate) {
            out.push(format!("{shown}: names `{path}`; {}", home.why));
        }
    }
}

/// STYLE.md P2: entry points never name a browser-driving crate.
fn check_entry_points(shown: &str, module: &ModuleFile, names: &NamedPaths, out: &mut Vec<String>) {
    if !ENTRY_POINTS.contains(&module.subtree.as_str()) {
        return;
    }
    for krate in BROWSER_CRATES {
        for path in names.with_root(krate) {
            out.push(format!(
                "{shown}: names `{path}`; {}/ reaches the browser only through Dispatcher::submit (STYLE.md P2)",
                module.subtree
            ));
        }
    }
}

/// STYLE.md P2: the door names neither the built-ins nor the host. It reads paths as written, as
/// every rule here does, so it sees the `crate::` form this crate's cross-subtree paths take; a
/// `super::super::host` path would slip past it.
fn check_door(shown: &str, module: &ModuleFile, names: &NamedPaths, out: &mut Vec<String>) {
    if module.subtree != DOOR {
        return;
    }
    for path in names.with_root("crate") {
        let named = path.split("::").nth(1);
        if named.is_some_and(|named| BEHIND_THE_DOOR.contains(&named)) {
            out.push(format!(
                "{shown}: names `{path}`; {DOOR}/ runs methods and rules only as `Method` and `Policy`, and never names builtins/ or host/ (STYLE.md P2)"
            ));
        }
    }
}

/// STYLE.md D2: the crate roots and every non-leaf module open with a `//!` doc, which is where a
/// subsystem's purpose and data flow are written down.
fn check_module_doc(shown: &str, module: &ModuleFile, file: &File, out: &mut Vec<String>) {
    let needs_doc = matches!(
        module.kind,
        ModuleKind::LibRoot | ModuleKind::BinRoot | ModuleKind::NonLeaf
    );
    if needs_doc && !file.attrs.iter().any(is_inner_doc) {
        out.push(format!("{shown}: a non-leaf module starts with a `//!` doc saying what the subsystem owns (STYLE.md D2)"));
    }
}

/// STYLE.md G7 (Google): non-leaf modules hold only `mod` declarations and re-exports.
fn check_non_leaf_items(shown: &str, module: &ModuleFile, file: &File, out: &mut Vec<String>) {
    if !matches!(module.kind, ModuleKind::LibRoot | ModuleKind::NonLeaf)
        || is_excepted(NON_LEAF_CODE_EXCEPTIONS, shown)
    {
        return;
    }
    for item in &file.items {
        if !matches!(item, Item::Mod(_) | Item::Use(_)) {
            out.push(format!(
                "{shown}: a non-leaf module holds only `mod` and `use`; move this {} into a leaf (STYLE.md G7)",
                item_kind(item)
            ));
        }
    }
}

/// STYLE.md G7 (Google): a leaf module is named after its primary item: `operation_guard.rs`
/// defines `OperationGuard`, `system_routes.rs` defines `fn system_routes`.
fn check_leaf_name(shown: &str, module: &ModuleFile, file: &File, out: &mut Vec<String>) {
    if module.kind != ModuleKind::Leaf || is_excepted(LEAF_NAME_EXCEPTIONS, shown) {
        return;
    }
    let camel = upper_camel(&module.name);
    let screaming = module.name.to_uppercase();
    let named = file.items.iter().any(|item| match item {
        Item::Struct(item) => item.ident == camel,
        Item::Enum(item) => item.ident == camel,
        Item::Trait(item) => item.ident == camel,
        Item::Type(item) => item.ident == camel,
        Item::Union(item) => item.ident == camel,
        Item::Fn(item) => item.sig.ident == module.name,
        Item::Const(item) => item.ident == screaming,
        Item::Static(item) => item.ident == screaming,
        Item::Macro(item) => item
            .ident
            .as_ref()
            .is_some_and(|ident| *ident == module.name),
        _ => false,
    });
    if !named {
        out.push(format!(
            "{shown}: a leaf module defines an item named after it (`{camel}` or `fn {}`), or is listed in LEAF_NAME_EXCEPTIONS with a reason (STYLE.md G7)",
            module.name
        ));
    }
}

fn is_excepted(exceptions: &[(&str, &str)], shown: &str) -> bool {
    exceptions.iter().any(|(path, _reason)| *path == shown)
}

fn is_inner_doc(attr: &Attribute) -> bool {
    matches!(attr.style, AttrStyle::Inner(_)) && attr.path().is_ident("doc")
}

fn item_kind(item: &Item) -> &'static str {
    match item {
        Item::Fn(_) => "fn",
        Item::Struct(_) => "struct",
        Item::Enum(_) => "enum",
        Item::Impl(_) => "impl",
        Item::Trait(_) => "trait",
        Item::Const(_) | Item::Static(_) => "constant",
        Item::Macro(_) => "macro",
        _ => "item",
    }
}

/// `operation_guard` -> `OperationGuard`.
fn upper_camel(snake: &str) -> String {
    snake
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect()
}

/// Every path a file names, joined with `::`: `use` trees, types, expressions, attributes and
/// macro invocations (not their bodies).
#[derive(Default)]
struct NamedPaths {
    paths: BTreeSet<String>,
}

impl NamedPaths {
    fn with_root<'a>(&'a self, root: &'a str) -> impl Iterator<Item = &'a String> + 'a {
        self.paths
            .iter()
            .filter(move |path| path.split("::").next() == Some(root))
    }
}

impl<'ast> Visit<'ast> for NamedPaths {
    fn visit_item_use(&mut self, node: &'ast ItemUse) {
        collect_use_paths(&node.tree, &mut Vec::new(), &mut self.paths);
        visit::visit_item_use(self, node);
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        let joined: Vec<String> = node
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        self.paths.insert(joined.join("::"));
        visit::visit_path(self, node);
    }
}

fn collect_use_paths(tree: &UseTree, prefix: &mut Vec<String>, out: &mut BTreeSet<String>) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_paths(&path.tree, prefix, out);
            prefix.pop();
        }
        UseTree::Name(name) => {
            out.insert(joined_with(prefix, &name.ident.to_string()));
        }
        UseTree::Rename(rename) => {
            out.insert(joined_with(prefix, &rename.ident.to_string()));
        }
        UseTree::Glob(_) => {
            out.insert(joined_with(prefix, "*"));
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_paths(item, prefix, out);
            }
        }
    }
}

fn joined_with(prefix: &[String], last: &str) -> String {
    if prefix.is_empty() {
        last.to_owned()
    } else {
        format!("{}::{last}", prefix.join("::"))
    }
}

/// STYLE.md P4: finds structs and enums that hold a secret type and derive `Debug` or `Serialize`.
struct SecretDerives<'a> {
    file: &'a str,
    violations: &'a mut Vec<String>,
}

impl SecretDerives<'_> {
    fn check(&mut self, ident: &syn::Ident, attrs: &[Attribute], fields: &[&Fields]) {
        let holds_secret = fields.iter().any(|fields| {
            fields.iter().any(|field| {
                let mut segments = SegmentNames::default();
                segments.visit_type(&field.ty);
                SECRET_TYPES
                    .iter()
                    .any(|secret| segments.names.contains(*secret))
            })
        });
        if !holds_secret {
            return;
        }
        let derives = match derived_names(attrs) {
            Ok(derives) => derives,
            Err(error) => {
                self.violations.push(format!(
                    "{}: `{ident}`: derive list did not parse: {error}",
                    self.file
                ));
                return;
            }
        };
        for derive in derives {
            if LEAKING_DERIVES.contains(&derive.as_str()) {
                self.violations.push(format!(
                    "{}: `{ident}` holds a secret and derives `{derive}`; write a Debug that prints a fixed mask, and never Serialize it (STYLE.md P4)",
                    self.file
                ));
            }
        }
    }
}

impl<'ast> Visit<'ast> for SecretDerives<'_> {
    fn visit_item_struct(&mut self, node: &'ast ItemStruct) {
        self.check(&node.ident, &node.attrs, &[&node.fields]);
        visit::visit_item_struct(self, node);
    }

    fn visit_item_enum(&mut self, node: &'ast ItemEnum) {
        let fields: Vec<&Fields> = node
            .variants
            .iter()
            .map(|variant| &variant.fields)
            .collect();
        self.check(&node.ident, &node.attrs, &fields);
        visit::visit_item_enum(self, node);
    }
}

/// The last segment of every path inside a type: `Zeroizing<String>` gives `Zeroizing`, `String`.
#[derive(Default)]
struct SegmentNames {
    names: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for SegmentNames {
    fn visit_path_segment(&mut self, node: &'ast syn::PathSegment) {
        self.names.insert(node.ident.to_string());
        visit::visit_path_segment(self, node);
    }
}

/// Names in every `#[derive(...)]` on an item, by their last segment (`serde::Serialize` -> `Serialize`).
fn derived_names(attrs: &[Attribute]) -> syn::Result<Vec<String>> {
    let mut names = Vec::new();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("derive")) {
        attr.parse_nested_meta(|meta| {
            if let Some(last) = meta.path.segments.last() {
                names.push(last.ident.to_string());
            }
            Ok(())
        })?;
    }
    Ok(names)
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_rust_files(&path, out)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Checks that this crate restates every workspace lint at least as strictly, and that the twin
/// crate on the same lint set (crates/neo-sdk/neo-playwright) has an identical `[lints]` table and
/// clippy.toml. Cargo cannot share a lint table between two crates without the workspace's.
pub(crate) fn check_lint_tables(manifest_dir: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut violations = Vec::new();
    let manifest = read_toml(&manifest_dir.join("Cargo.toml"))?;
    let lints = manifest
        .get("lints")
        .and_then(Value::as_table)
        .cloned()
        .unwrap_or_default();

    let workspace_root = find_workspace_root(manifest_dir)?;
    let workspace = read_toml(&workspace_root.join("Cargo.toml"))?;
    let workspace_lints = workspace
        .get("workspace")
        .and_then(|table| table.get("lints"))
        .and_then(Value::as_table)
        .cloned()
        .unwrap_or_default();
    for (tool, table) in &workspace_lints {
        for (lint, level) in table.as_table().into_iter().flatten() {
            let ours = lints.get(tool).and_then(|tool_table| tool_table.get(lint));
            let required = level_rank(Some(level));
            if level_rank(ours) < required {
                violations.push(format!(
                    "Cargo.toml [lints.{tool}] must restate workspace lint `{lint}` at level {} or stricter",
                    level_name(level)
                ));
            }
        }
    }

    let twin = workspace_root.join("crates/neo-sdk/neo-playwright");
    if twin.join("Cargo.toml").is_file() {
        let twin_lints = read_toml(&twin.join("Cargo.toml"))?.get("lints").cloned();
        if twin_lints.as_ref().and_then(Value::as_table) != Some(&lints) {
            violations.push(
                "crates/neo-sdk/neo-playwright/Cargo.toml [lints] differs from claw-sdk's"
                    .to_owned(),
            );
        }
        if fs::read_to_string(twin.join("clippy.toml")).ok()
            != fs::read_to_string(manifest_dir.join("clippy.toml")).ok()
        {
            violations.push(
                "crates/neo-sdk/neo-playwright/clippy.toml differs from claw-sdk's".to_owned(),
            );
        }
    }
    Ok(violations)
}

fn read_toml(path: &Path) -> Result<Table, Box<dyn Error>> {
    Ok(fs::read_to_string(path)?.parse::<Table>()?)
}

fn find_workspace_root(manifest_dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    for dir in manifest_dir.ancestors() {
        let candidate = dir.join("Cargo.toml");
        if candidate.is_file() && read_toml(&candidate)?.contains_key("workspace") {
            return Ok(dir.to_path_buf());
        }
    }
    Err("no workspace Cargo.toml above the crate".into())
}

/// `allow` < `warn` < `deny` < `forbid`; a missing lint ranks below `allow`.
fn level_rank(level: Option<&Value>) -> u8 {
    match level.map(level_name).as_deref() {
        Some("allow") => 1,
        Some("warn") => 2,
        Some("deny") => 3,
        Some("forbid") => 4,
        _ => 0,
    }
}

/// A lint entry is either `"deny"` or `{ level = "deny", priority = -1 }`.
fn level_name(value: &Value) -> String {
    match value {
        Value::String(level) => level.clone(),
        Value::Table(table) => table
            .get("level")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        _ => String::new(),
    }
}
