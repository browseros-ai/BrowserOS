//! The router serves exactly the (method, path) set of C1's OpenAPI contract
//! (`contracts/server-api/openapi.yaml` + `paths/*.yaml`), parsed at test time so a contract
//! change without a matching `http::ROUTES` change fails here. Both directions are checked: the
//! static table against the spec, and the mounted router by sending each spec request through it.

use axum::body::Body;
use http::{Method, Request, StatusCode};
use serde_json::Value;
use server_rust::http::{ROUTES, UnmatchedRoute};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use tower::ServiceExt as _;

const HTTP_METHODS: &[&str] = &[
    "get", "put", "post", "delete", "patch", "head", "options", "trace",
];

type RouteSet = BTreeSet<(String, String)>;

fn contract_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/server-api")
}

fn read_yaml(path: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(serde_saphyr::from_str(&text)?)
}

/// Normalizes `:param` (Express/Hono style) to `{param}` (OpenAPI and axum 0.8 style).
fn normalize(path: &str) -> String {
    path.split('/')
        .map(|segment| match segment.strip_prefix(':') {
            Some(name) => format!("{{{name}}}"),
            None => segment.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Resolves each `paths:` entry (inline or `$ref: ./paths/<file>.yaml#/<key>`) to its methods.
fn spec_routes() -> Result<RouteSet, Box<dyn std::error::Error>> {
    let dir = contract_dir();
    let root = read_yaml(&dir.join("openapi.yaml"))?;
    let paths = root
        .get("paths")
        .and_then(Value::as_object)
        .ok_or("openapi.yaml has no paths")?;
    let mut routes = RouteSet::new();
    for (path, item) in paths {
        let item = match item.get("$ref").and_then(Value::as_str) {
            Some(reference) => {
                let (file, pointer) = reference.split_once('#').ok_or("ref without #")?;
                let document = read_yaml(&dir.join(file))?;
                document
                    .pointer(pointer)
                    .cloned()
                    .ok_or_else(|| format!("unresolved {reference}"))?
            }
            None => item.clone(),
        };
        let item = item.as_object().ok_or("path item is not a map")?;
        for method in item
            .keys()
            .filter(|key| HTTP_METHODS.contains(&key.as_str()))
        {
            routes.insert((method.to_uppercase(), normalize(path)));
        }
    }
    Ok(routes)
}

fn table_routes() -> RouteSet {
    ROUTES
        .iter()
        .map(|route| (route.method.as_str().to_owned(), normalize(route.path)))
        .collect()
}

#[test]
fn route_table_equals_contract() -> Result<(), Box<dyn std::error::Error>> {
    let spec = spec_routes()?;
    let table = table_routes();
    assert_eq!(table.len(), ROUTES.len(), "ROUTES contains duplicates");
    let missing: Vec<_> = spec.difference(&table).collect();
    let extra: Vec<_> = table.difference(&spec).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "route table drifted from the contract\n  in spec, not in ROUTES: {missing:?}\n  in ROUTES, not in spec: {extra:?}"
    );
    Ok(())
}

/// Fills `{param}` with a value every param schema accepts (a zod-valid uuid also satisfies the
/// free-string params).
fn concrete(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.starts_with('{') {
                "00000000-0000-4000-8000-000000000000"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Every spec route reaches a handler rather than the router fallback. Status codes are not
/// asserted: they change as pieces replace the skeleton's 501s. Once P3 adds guards, requests
/// here must carry whatever headers pass them, or a guard's 403 would mask a missing route.
#[tokio::test]
async fn mounted_router_serves_every_contract_route() -> Result<(), Box<dyn std::error::Error>> {
    let config_dir = tempfile::tempdir()?;
    let state = test_state(config_dir.path()).await?;
    let router = server_rust::build_router(state);

    for (method, path) in spec_routes()? {
        let uri = concrete(&path);
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::from_bytes(method.as_bytes())?)
                    .uri(&uri)
                    .body(Body::empty())?,
            )
            .await?;
        assert!(
            response.extensions().get::<UnmatchedRoute>().is_none(),
            "{method} {uri} fell through to the router fallback"
        );
    }

    let response = router
        .oneshot(Request::get("/not-in-the-contract").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(response.extensions().get::<UnmatchedRoute>().is_some());
    Ok(())
}

async fn test_state(dir: &Path) -> Result<server_rust::AppState, Box<dyn std::error::Error>> {
    let config_path = dir.join("config.json");
    std::fs::write(
        &config_path,
        r#"{"ports":{"server":0,"cdp":0},"directories":{"resources":".","execution":"."}}"#,
    )?;
    let env = server_rust::config::EnvConfig {
        browseros_dir: Some(dir.join("browseros")),
        ..Default::default()
    };
    let config = server_rust::config::Config::load(&config_path, &env)?;
    Ok(server_rust::AppState::start(config).await?)
}
