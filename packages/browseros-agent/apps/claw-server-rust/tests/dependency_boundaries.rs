use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use syn::{
    ItemMod, ItemUse, UseTree,
    visit::{self, Visit},
};

const DELETED_ROOT_MODULES: &[&str] = &[
    "capture",
    "tabs",
    "routes",
    "mcp",
    "sessions",
    "browser",
    "live_sessions",
    "telemetry",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Layer {
    ApiHttp,
    ApiMcp,
    Service(String),
    Db,
    Identity,
    Support,
    Composition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    ApiHttp,
    ApiMcp,
    AppState,
    Service(String),
    Db,
    Identity,
    Support,
    Composition,
}

#[derive(Default)]
struct DependencyVisitor {
    paths: BTreeSet<Vec<String>>,
    modules: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for DependencyVisitor {
    fn visit_item_use(&mut self, node: &'ast ItemUse) {
        collect_use_paths(&node.tree, &mut Vec::new(), &mut self.paths);
        visit::visit_item_use(self, node);
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        self.paths.insert(
            node.segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect(),
        );
        visit::visit_path(self, node);
    }

    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        self.modules.insert(node.ident.to_string());
        visit::visit_item_mod(self, node);
    }
}

#[test]
fn source_tree_respects_dependency_boundaries() -> Result<(), Box<dyn std::error::Error>> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rust_files(&src, &mut files)?;
    files.sort();

    let mut failures = Vec::new();
    for file in files {
        let relative = file.strip_prefix(&src)?;
        let source = fs::read_to_string(&file)?;
        if let Err(errors) = check_source(relative, &source) {
            failures.extend(errors);
        }
    }

    assert!(
        failures.is_empty(),
        "dependency boundary violations:\n{}",
        failures.join("\n")
    );
    Ok(())
}

#[test]
fn browser_to_sessions_is_an_allowed_service_edge() {
    assert!(
        check_source(
            Path::new("services/browser/example.rs"),
            "use crate::services::sessions::Sessions;",
        )
        .is_ok()
    );
}

#[test]
fn services_cannot_depend_on_api() {
    let errors = violations("services/browser/example.rs", "use crate::api::http;");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("services/browser"))
    );
}

#[test]
fn services_cannot_depend_on_app_state() {
    let errors = violations("services/cockpit/example.rs", "use crate::AppState;");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("services/cockpit -> AppState"))
    );
}

#[test]
fn db_cannot_depend_on_services() {
    let errors = violations("db/example.rs", "use crate::services::sessions::Sessions;");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("db -> services/sessions"))
    );
}

#[test]
fn sea_orm_is_rejected_outside_db() {
    let errors = violations(
        "services/recordings/example.rs",
        "use sea_orm::EntityTrait;",
    );
    assert!(errors.iter().any(|error| error.contains("sea_orm")));
}

#[test]
fn analytics_catalog_and_sdk_have_single_source_boundaries()
-> Result<(), Box<dyn std::error::Error>> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rust_files(&src, &mut files)?;
    let wire_names = [
        "server_started",
        "agent_session_started",
        "agent_session_ended",
        "harness_connected",
        "harness_disconnected",
    ];
    let mut wire_locations = wire_names
        .iter()
        .map(|name| (*name, Vec::new()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut sdk_locations = Vec::new();

    for file in files {
        let relative = file
            .strip_prefix(&src)?
            .to_string_lossy()
            .replace('\\', "/");
        let source = fs::read_to_string(&file)?;
        for name in wire_names {
            if source.contains(&format!("\"{name}\"")) {
                wire_locations
                    .entry(name)
                    .or_default()
                    .push(relative.clone());
            }
        }
        if source.contains("posthog_rs") {
            sdk_locations.push(relative);
        }
    }

    for (name, locations) in wire_locations {
        assert_eq!(
            locations,
            ["analytics/events.rs"],
            "analytics wire name {name} escaped the catalog"
        );
    }
    assert_eq!(sdk_locations, ["analytics/service.rs"]);
    assert_eq!(claw_server_rust::analytics::events::ALL.len(), 5);
    Ok(())
}

fn violations(relative: &str, source: &str) -> Vec<String> {
    check_source(Path::new(relative), source)
        .err()
        .unwrap_or_default()
}

fn check_source(relative: &Path, source: &str) -> Result<(), Vec<String>> {
    let display = relative.to_string_lossy().replace('\\', "/");
    let layer = classify(relative).map_err(|error| vec![format!("{display}: {error}")])?;
    let syntax = syn::parse_file(source)
        .map_err(|error| vec![format!("{display}: failed to parse source: {error}")])?;
    let mut visitor = DependencyVisitor::default();
    visitor.visit_file(&syntax);
    let mut failures = Vec::new();

    if display == "lib.rs" {
        for module in visitor.modules.intersection(
            &DELETED_ROOT_MODULES
                .iter()
                .map(|module| (*module).to_string())
                .collect(),
        ) {
            failures.push(format!(
                "{display}: deleted root module `{module}` is declared"
            ));
        }
    }

    for path in visitor.paths {
        if path
            .first()
            .is_some_and(|root| root == "sea_orm" || root == "sea_orm_migration")
            && layer != Layer::Db
        {
            failures.push(format!(
                "{display}: `{}` is restricted to src/db",
                path.join("::")
            ));
            continue;
        }
        let Some(target) = crate_target(&path, &mut failures, &display) else {
            continue;
        };
        if !edge_allowed(&layer, &target) {
            failures.push(format!(
                "{display}: {} -> {} is not allowed",
                layer_name(&layer),
                target_name(&target)
            ));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

fn classify(relative: &Path) -> Result<Layer, String> {
    let parts = relative
        .iter()
        .map(|part| part.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    match parts.as_slice() {
        [api, surface, ..] if api == "api" && surface == "http" => Ok(Layer::ApiHttp),
        [api, surface, ..] if api == "api" && surface == "mcp" => Ok(Layer::ApiMcp),
        [services, name, ..] if services == "services" => Ok(Layer::Service(
            name.strip_suffix(".rs").unwrap_or(name).to_string(),
        )),
        [db, ..] if db == "db" => Ok(Layer::Db),
        [identity, ..] if identity == "identity" => Ok(Layer::Identity),
        [analytics, ..] if analytics == "analytics" => Ok(Layer::Support),
        [file] if matches!(file.as_str(), "app.rs" | "runtime.rs" | "main.rs") => {
            Ok(Layer::Composition)
        }
        [api, file] if api == "api" && file == "mod.rs" => Ok(Layer::Support),
        [file]
            if matches!(
                file.as_str(),
                "clock.rs" | "config.rs" | "error.rs" | "ids.rs" | "lib.rs" | "storage.rs"
            ) =>
        {
            Ok(Layer::Support)
        }
        _ => Err("file is outside the approved architecture".to_string()),
    }
}

fn crate_target(path: &[String], failures: &mut Vec<String>, display: &str) -> Option<Target> {
    if path.first().is_none_or(|root| root != "crate") || path.len() < 2 {
        return None;
    }
    let root = path[1].as_str();
    if DELETED_ROOT_MODULES.contains(&root) {
        failures.push(format!(
            "{display}: deleted root module is imported as `{}`",
            path.join("::")
        ));
        return None;
    }
    match root {
        "api" => match path.get(2).map(String::as_str) {
            Some("http") => Some(Target::ApiHttp),
            Some("mcp") => Some(Target::ApiMcp),
            _ => Some(Target::Support),
        },
        "services" => Some(Target::Service(
            path.get(2)
                .map(|name| name.strip_suffix(".rs").unwrap_or(name))
                .unwrap_or("root")
                .to_string(),
        )),
        "db" => Some(Target::Db),
        "identity" => Some(Target::Identity),
        "app" | "runtime" => Some(Target::Composition),
        "AppState" => Some(Target::AppState),
        "analytics" | "clock" | "config" | "error" | "ids" | "storage" | "AppResult" => {
            Some(Target::Support)
        }
        _ => None,
    }
}

fn edge_allowed(source: &Layer, target: &Target) -> bool {
    match source {
        Layer::Composition => true,
        Layer::ApiHttp => matches!(
            target,
            Target::ApiHttp
                | Target::AppState
                | Target::Service(_)
                | Target::Db
                | Target::Identity
                | Target::Support
        ),
        Layer::ApiMcp => matches!(
            target,
            Target::ApiMcp
                | Target::AppState
                | Target::Service(_)
                | Target::Db
                | Target::Identity
                | Target::Support
        ),
        Layer::Service(source) => match target {
            Target::Service(target) if source == target => true,
            Target::Service(target) => allowed_service_edge(source, target),
            Target::Db | Target::Identity | Target::Support => true,
            Target::ApiHttp | Target::ApiMcp | Target::AppState | Target::Composition => false,
        },
        Layer::Db => matches!(target, Target::Db | Target::Support),
        Layer::Identity | Layer::Support => matches!(target, Target::Identity | Target::Support),
    }
}

fn allowed_service_edge(source: &str, target: &str) -> bool {
    matches!(
        (source, target),
        ("browser", "sessions")
            | ("cockpit", "browser" | "sessions" | "profiles")
            | ("recordings", "browser")
            | ("replay", "recordings")
    )
}

fn layer_name(layer: &Layer) -> String {
    match layer {
        Layer::ApiHttp => "api/http".to_string(),
        Layer::ApiMcp => "api/mcp".to_string(),
        Layer::Service(name) => format!("services/{name}"),
        Layer::Db => "db".to_string(),
        Layer::Identity => "identity".to_string(),
        Layer::Support => "support".to_string(),
        Layer::Composition => "composition".to_string(),
    }
}

fn target_name(target: &Target) -> String {
    match target {
        Target::ApiHttp => "api/http".to_string(),
        Target::ApiMcp => "api/mcp".to_string(),
        Target::AppState => "AppState".to_string(),
        Target::Service(name) => format!("services/{name}"),
        Target::Db => "db".to_string(),
        Target::Identity => "identity".to_string(),
        Target::Support => "support".to_string(),
        Target::Composition => "composition".to_string(),
    }
}

fn collect_use_paths(tree: &UseTree, prefix: &mut Vec<String>, paths: &mut BTreeSet<Vec<String>>) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_paths(&path.tree, prefix, paths);
            prefix.pop();
        }
        UseTree::Name(name) => {
            let mut path = prefix.clone();
            path.push(name.ident.to_string());
            paths.insert(path);
        }
        UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            path.push(rename.ident.to_string());
            paths.insert(path);
        }
        UseTree::Glob(_) => {
            paths.insert(prefix.clone());
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_paths(item, prefix, paths);
            }
        }
    }
}

fn collect_rust_files(directory: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_rust_files(&path, files)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    Ok(())
}
