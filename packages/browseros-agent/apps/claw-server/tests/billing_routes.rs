//! `/api/v1/plan*` against a fake billing gateway: what claw-app sees for each route, what
//! reaches the gateway, and that the licence key is never kept or echoed.

#[path = "support/fake_gateway.rs"]
mod fake_gateway;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use claw_server::{
    AppState, build_router,
    config::{Config, ServerMode},
    services::billing::{Billing, RefreshOutcome},
};
use fake_gateway::{FakeGateway, FakePlan, KEY, RESET_KEY, now_ms};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

const EXTENSION_ORIGIN: &str = "chrome-extension://pjimfkbpehlcllblajnpfamdfjhhlgkc";

struct Computer {
    router: Router,
    state: AppState,
    dir: TempDir,
}

impl Computer {
    fn data_dir(&self) -> PathBuf {
        self.dir.path().join("browserclaw")
    }

    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> anyhow::Result<Reply> {
        call(&self.router, method, uri, body, None).await
    }

    async fn plan(&self) -> anyhow::Result<Value> {
        Ok(self.call("GET", "/api/v1/plan", None).await?.body)
    }

    async fn activate(&self, key: &str) -> anyhow::Result<Reply> {
        self.call(
            "POST",
            "/api/v1/plan/key/activate",
            Some(json!({ "key": key })),
        )
        .await
    }

    async fn install_id(&self) -> anyhow::Result<String> {
        let raw = tokio::fs::read_to_string(self.data_dir().join("billing/install.json")).await?;
        let file: Value = serde_json::from_str(&raw)?;
        Ok(file["installId"].as_str().unwrap_or_default().to_string())
    }

    /// Restarts billing on the same data dir, as a server restart would.
    async fn reopen(&mut self, gateway: &FakeGateway) -> anyhow::Result<()> {
        self.state.billing = Arc::new(
            Billing::open(
                Some(gateway.config()?),
                self.data_dir(),
                CancellationToken::new(),
            )
            .await,
        );
        self.router = build_router(self.state.clone());
        Ok(())
    }
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
    raw: String,
}

fn config_for(dir: &Path) -> Arc<Config> {
    Arc::new(Config {
        server_port: 9200,
        cdp_port: 49337,
        proxy_port: None,
        resources_dir: dir.join("resources"),
        browserclaw_dir: dir.join("browserclaw"),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
        native: Default::default(),
        mode: ServerMode::Desktop,
        behavior: claw_server::config::Behavior::defaults(ServerMode::Desktop),
        behavior_ignored: Vec::new(),
    })
}

/// A server whose billing talks to `gateway`, set through the pub field the way other tests
/// swap a service, with no environment variable.
async fn computer(gateway: Option<&FakeGateway>) -> anyhow::Result<Computer> {
    let dir = tempfile::tempdir()?;
    let mut state =
        AppState::new_with_home(config_for(dir.path()), dir.path().join("home")).await?;
    if let Some(gateway) = gateway {
        state.billing = Arc::new(
            Billing::open(
                Some(gateway.config()?),
                dir.path().join("browserclaw"),
                CancellationToken::new(),
            )
            .await,
        );
    }
    Ok(Computer {
        router: build_router(state.clone()),
        state,
        dir,
    })
}

async fn call(
    router: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    origin: Option<&str>,
) -> anyhow::Result<Reply> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost");
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    let body = match body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = router.clone().oneshot(builder.body(body)?).await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let raw = String::from_utf8_lossy(&bytes).into_owned();
    Ok(Reply {
        status,
        headers,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        raw,
    })
}

/// Buys Pro on `computer` through the fake checkout.
async fn buy(computer: &Computer, gateway: &FakeGateway) -> anyhow::Result<Value> {
    let started = computer
        .call(
            "POST",
            "/api/v1/plan/checkout",
            Some(json!({"plan": "pro", "interval": "month"})),
        )
        .await?;
    assert_eq!(started.status, StatusCode::CREATED, "{}", started.raw);
    let id = started.body["checkoutId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    gateway.complete_checkout(&id);
    let polled = computer
        .call("GET", &format!("/api/v1/plan/checkout/{id}"), None)
        .await?;
    assert_eq!(polled.status, StatusCode::OK, "{}", polled.raw);
    Ok(polled.body)
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(files_under(&path));
        } else {
            files.push(path);
        }
    }
    files
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(
            &self
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
        .into_owned()
    }
}

#[tokio::test]
async fn the_plan_reads_free_before_anything_and_never_reaches_the_gateway() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let a = computer(Some(&gateway)).await?;
    let plan = a.plan().await?;
    assert_eq!(plan["available"], true);
    assert_eq!(plan["status"], "none");
    assert_eq!(plan["plan"], Value::Null);
    assert_eq!(plan["features"], json!([]));
    assert_eq!(plan["onKey"], false);
    assert_eq!(plan["stale"], false);
    assert!(gateway.requests().is_empty());
    assert!(!a.data_dir().join("billing").exists());
    Ok(())
}

#[tokio::test]
async fn pricing_is_cached_and_drops_periods_it_cannot_show() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let a = computer(Some(&gateway)).await?;
    let first = a.call("GET", "/api/v1/plan/pricing", None).await?;
    assert_eq!(first.status, StatusCode::OK, "{}", first.raw);
    let second = a.call("GET", "/api/v1/plan/pricing", None).await?;
    assert_eq!(second.body, first.body);
    assert_eq!(gateway.count("GET", "/v1/pricing"), 1);
    let logged = gateway.requests();
    assert_eq!(logged[0].query.as_deref(), Some("app=claw"));
    assert_eq!(logged[0].authorization, None);
    assert_eq!(first.body["plans"][0]["plan"], "pro");
    assert_eq!(first.body["plans"][0]["name"], "neo Pro");
    assert_eq!(
        first.body["plans"][0]["prices"],
        json!([
            {"interval": "month", "intervalCount": 1, "amount": 1200, "currency": "usd"},
            {"interval": "year", "intervalCount": 1, "amount": 12000, "currency": "usd"}
        ])
    );
    Ok(())
}

#[tokio::test]
async fn a_gateway_that_is_down_or_unreachable_answers_503() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    gateway.set_down(true);
    let a = computer(Some(&gateway)).await?;
    let pricing = a.call("GET", "/api/v1/plan/pricing", None).await?;
    assert_eq!(pricing.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(pricing.body["code"], "billing_unreachable");
    assert!(pricing.body["requestId"].is_string());

    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = closed.local_addr()?;
    drop(closed);
    let mut offline = computer(None).await?;
    let mut config = gateway.config()?;
    config.gateway = url::Url::parse(&format!("http://{address}"))?;
    offline.state.billing =
        Arc::new(Billing::open(Some(config), offline.data_dir(), CancellationToken::new()).await);
    offline.router = build_router(offline.state.clone());
    let checkout = offline
        .call(
            "POST",
            "/api/v1/plan/checkout",
            Some(json!({"plan": "pro", "interval": "month"})),
        )
        .await?;
    assert_eq!(checkout.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(checkout.body["code"], "billing_unreachable");
    Ok(())
}

#[tokio::test]
async fn buying_registers_this_computer_and_the_poll_saves_the_licence() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let a = computer(Some(&gateway)).await?;
    let started = a
        .call(
            "POST",
            "/api/v1/plan/checkout",
            Some(json!({"plan": "pro", "interval": "year"})),
        )
        .await?;
    assert_eq!(started.status, StatusCode::CREATED, "{}", started.raw);
    assert_eq!(
        started.body["url"],
        "https://checkout.example.com/c/pay/cs_test_fake#fragment"
    );

    let logged = gateway.requests();
    assert_eq!(logged[0].path, "/v1/installs");
    let registration: Value = serde_json::from_str(&logged[0].body)?;
    assert_eq!(registration["app"], "claw");
    assert_eq!(registration["appVersion"], claw_server::VERSION);
    assert_eq!(registration["platform"], std::env::consts::OS);
    let label = registration["label"].as_str().unwrap_or_default();
    assert!(!label.is_empty() && label.encode_utf16().count() <= 40);
    let checkout: Value = serde_json::from_str(&logged[1].body)?;
    assert_eq!(checkout, json!({"plan": "pro", "interval": "year"}));
    assert!(
        logged[1]
            .authorization
            .as_deref()
            .is_some_and(|value| value.starts_with("Bearer "))
    );

    let id = started.body["checkoutId"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let open = a
        .call("GET", &format!("/api/v1/plan/checkout/{id}"), None)
        .await?;
    assert_eq!(open.body, json!({"status": "open", "plan": null}));
    assert!(!a.data_dir().join("billing/licence.json").exists());

    gateway.complete_checkout(&id);
    let complete = a
        .call("GET", &format!("/api/v1/plan/checkout/{id}"), None)
        .await?;
    assert_eq!(complete.body["status"], "complete");
    assert_eq!(complete.body["plan"]["status"], "active");
    assert_eq!(complete.body["plan"]["onKey"], true);
    assert_eq!(complete.body["plan"]["planName"], "neo Pro");
    assert_eq!(complete.body["plan"]["features"], json!(["claw.ultrafast"]));
    assert!(!complete.raw.contains("NEO-"));

    let plan = a.plan().await?;
    assert_eq!(plan["status"], "active");
    assert!(plan["checkedAt"].as_i64().is_some());
    assert!(plan["periodEnd"].as_i64().is_some());

    #[cfg(unix)]
    for file in ["billing/install.json", "billing/licence.json"] {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(a.data_dir().join(file))?
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "{file}: {mode:o}");
    }

    let unknown = a
        .call(
            "GET",
            "/api/v1/plan/checkout/5d0c6a1e-3b7f-4c2a-9e81-2f64b0a7c913",
            None,
        )
        .await?;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.body["code"], "not_found");
    let not_an_id = a
        .call("GET", "/api/v1/plan/checkout/..%2Fkeys", None)
        .await?;
    assert_eq!(not_an_id.status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn checkout_refusals_keep_the_gateways_message() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let a = computer(Some(&gateway)).await?;
    buy(&a, &gateway).await?;
    let again = a
        .call(
            "POST",
            "/api/v1/plan/checkout",
            Some(json!({"plan": "pro", "interval": "month"})),
        )
        .await?;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.body["code"], "already_subscribed");
    assert_eq!(
        again.body["message"],
        "This computer already has a plan. Change it from billing settings."
    );

    let b = computer(Some(&gateway)).await?;
    b.call("GET", "/api/v1/plan/pricing", None).await?;
    assert_eq!(gateway.count("GET", "/v1/pricing"), 2);
    let unknown = b
        .call(
            "POST",
            "/api/v1/plan/checkout",
            Some(json!({"plan": "gold", "interval": "month"})),
        )
        .await?;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
    assert_eq!(unknown.body["code"], "unknown_plan");
    b.call("GET", "/api/v1/plan/pricing", None).await?;
    assert_eq!(
        gateway.count("GET", "/v1/pricing"),
        3,
        "pricing was dropped"
    );

    gateway.script(
        "POST /v1/checkouts",
        429,
        json!({"code": "rateLimited", "message": "Too many requests. Try again in a little while."}),
        Some(120),
    );
    let limited = b
        .call(
            "POST",
            "/api/v1/plan/checkout",
            Some(json!({"plan": "pro", "interval": "month"})),
        )
        .await?;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.body["code"], "rate_limited");
    assert_eq!(limited.body["retryAfterSeconds"], 120);
    assert_eq!(
        limited
            .headers
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
        Some("120")
    );

    let malformed = b
        .call(
            "POST",
            "/api/v1/plan/checkout",
            Some(json!({"plan": "pro"})),
        )
        .await?;
    assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
    assert_eq!(malformed.body["code"], "invalid_request");
    Ok(())
}

#[tokio::test]
async fn a_second_computer_pastes_the_key_and_both_see_the_computers() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let a = computer(Some(&gateway)).await?;
    let b = computer(Some(&gateway)).await?;

    let none = b.call("GET", "/api/v1/plan/key", None).await?;
    assert_eq!(none.status, StatusCode::CONFLICT);
    assert_eq!(none.body["code"], "not_linked");
    assert!(
        gateway.requests().is_empty(),
        "no install is registered just to ask"
    );

    buy(&a, &gateway).await?;
    let pasted = b.activate("neo-7k2qm-aaaaa-bbbbb-ccccc-ddddd").await?;
    assert_eq!(pasted.status, StatusCode::OK, "{}", pasted.raw);
    assert_eq!(pasted.body["plan"]["status"], "active");
    assert_eq!(pasted.body["plan"]["onKey"], true);
    assert_eq!(pasted.body["key"]["hint"], "7K2QM");
    assert!(!pasted.raw.to_ascii_uppercase().contains("7K2QM-AAAAA"));
    let activation: Value = serde_json::from_str(
        &gateway
            .requests()
            .iter()
            .rev()
            .find(|logged| logged.path == "/v1/keys/activate")
            .map(|logged| logged.body.clone())
            .unwrap_or_default(),
    )?;
    assert_eq!(
        activation,
        json!({"key": "neo-7k2qm-aaaaa-bbbbb-ccccc-ddddd"})
    );

    let key = a.call("GET", "/api/v1/plan/key", None).await?;
    assert_eq!(key.status, StatusCode::OK);
    assert_eq!(key.body["cap"], 5);
    let computers = key.body["computers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(computers.len(), 2);
    assert_eq!(computers[0]["installId"], a.install_id().await?);
    assert_eq!(computers[0]["thisComputer"], true);
    assert_eq!(computers[1]["installId"], b.install_id().await?);
    assert_eq!(computers[1]["thisComputer"], false);
    for computer in &computers {
        let activated = computer["activatedAt"].as_i64().unwrap_or_default();
        assert!((now_ms() - activated).abs() < 60_000, "{computer}");
        assert!(computer["lastSeenAt"].as_i64().is_some());
        assert!(computer["label"].is_string());
        assert_eq!(computer["platform"], std::env::consts::OS);
    }
    Ok(())
}

#[tokio::test]
async fn activation_refusals() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let a = computer(Some(&gateway)).await?;

    let wrong = a.activate("NEO-ZZZZZ-ZZZZZ-ZZZZZ-ZZZZZ-ZZZZZ").await?;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    assert_eq!(wrong.body["code"], "key_invalid");
    assert!(wrong.body["requestId"].is_string());

    let blank = a.activate("   ").await?;
    assert_eq!(blank.status, StatusCode::BAD_REQUEST);
    assert_eq!(blank.body["code"], "invalid_request");
    let long = a.activate(&"N".repeat(81)).await?;
    assert_eq!(long.body["code"], "invalid_request");

    gateway.script(
        "POST /v1/keys/activate",
        429,
        json!({"code": "tooManyTries", "message": "Too many tries from this computer. Wait a few minutes, then try again."}),
        None,
    );
    let tries = a.activate(KEY).await?;
    assert_eq!(tries.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(tries.body["code"], "too_many_tries");
    assert!(tries.body.get("retryAfterSeconds").is_none());

    gateway.script(
        "POST /v1/keys/activate",
        409,
        json!({"code": "alreadySubscribed", "message": "This computer already has its own neo Pro plan. Contact BrowserOS support to combine them."}),
        None,
    );
    let subscribed = a.activate(KEY).await?;
    assert_eq!(subscribed.status, StatusCode::CONFLICT);
    assert_eq!(subscribed.body["code"], "already_subscribed");
    assert!(subscribed.body.get("computers").is_none());

    gateway.set_plan(Some(FakeGateway::active_plan()));
    let others = gateway.add_computers(5);
    let full = a.activate(KEY).await?;
    assert_eq!(full.status, StatusCode::CONFLICT);
    assert_eq!(full.body["code"], "key_full");
    let computers = full.body["computers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(computers.len(), 5);
    assert_eq!(computers[0]["installId"], others[0]);
    assert_eq!(computers[0]["label"], "Other computer 0");
    assert!(computers[0]["activatedAt"].as_i64().is_some());
    assert_eq!(a.plan().await?["status"], "none");

    let replaced = a
        .call(
            "POST",
            "/api/v1/plan/key/activate",
            Some(json!({"key": KEY, "replace": others[0].to_uppercase()})),
        )
        .await?;
    assert_eq!(replaced.status, StatusCode::OK, "{}", replaced.raw);
    assert_eq!(replaced.body["plan"]["status"], "active");
    let sent: Value = serde_json::from_str(
        &gateway
            .requests()
            .iter()
            .rev()
            .find(|logged| logged.path == "/v1/keys/activate")
            .map(|logged| logged.body.clone())
            .unwrap_or_default(),
    )?;
    assert_eq!(
        sent["replace"], others[0],
        "sent as a plain lower-case UUID"
    );

    let bad_replace = a
        .call(
            "POST",
            "/api/v1/plan/key/activate",
            Some(json!({"key": KEY, "replace": "../x"})),
        )
        .await?;
    assert_eq!(bad_replace.status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn rename_remove_and_reset_across_two_computers() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let a = computer(Some(&gateway)).await?;
    let b = computer(Some(&gateway)).await?;
    buy(&a, &gateway).await?;
    b.activate(KEY).await?;
    let b_id = b.install_id().await?;

    let renamed = a
        .call(
            "POST",
            &format!("/api/v1/plan/key/computers/{b_id}/rename"),
            Some(json!({"name": "  Studio iMac  "})),
        )
        .await?;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.raw);
    let row = renamed.body["computers"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["installId"] == b_id.as_str()))
        .cloned()
        .unwrap_or_default();
    assert_eq!(row["label"], "Studio iMac");
    let sent = gateway
        .requests()
        .last()
        .map(|logged| (logged.path.clone(), logged.body.clone()))
        .unwrap_or_default();
    assert_eq!(sent.0, format!("/v1/keys/computers/{b_id}/label"));
    assert_eq!(
        serde_json::from_str::<Value>(&sent.1)?,
        json!({"label": "Studio iMac"})
    );

    let again = b.activate(KEY).await?;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(gateway.label_of(&b_id).as_deref(), Some("Studio iMac"));
    let sent: Value = serde_json::from_str(
        &gateway
            .requests()
            .last()
            .map(|logged| logged.body.clone())
            .unwrap_or_default(),
    )?;
    assert!(
        sent.get("label").is_none(),
        "pasting again never resends a name"
    );

    let too_long = "x".repeat(41);
    for (name, code) in [
        ("   ", "invalid_request"),
        (too_long.as_str(), "invalid_request"),
    ] {
        let refused = a
            .call(
                "POST",
                &format!("/api/v1/plan/key/computers/{b_id}/rename"),
                Some(json!({ "name": name })),
            )
            .await?;
        assert_eq!(refused.status, StatusCode::BAD_REQUEST);
        assert_eq!(refused.body["code"], code);
    }
    let stranger = a
        .call(
            "POST",
            "/api/v1/plan/key/computers/2a9b7c6d-1e0f-4a3b-8c5d-6e7f8091a2b3/rename",
            Some(json!({"name": "Nope"})),
        )
        .await?;
    assert_eq!(stranger.status, StatusCode::NOT_FOUND);
    assert_eq!(stranger.body["code"], "not_found");

    let removed = a
        .call(
            "POST",
            &format!("/api/v1/plan/key/computers/{b_id}/remove"),
            None,
        )
        .await?;
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.raw);
    assert_eq!(
        removed.body["key"]["computers"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(removed.body["plan"]["status"], "active");
    let gone = a
        .call(
            "POST",
            &format!("/api/v1/plan/key/computers/{b_id}/remove"),
            None,
        )
        .await?;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);

    let b_plan = b.call("POST", "/api/v1/plan/refresh", None).await?;
    assert_eq!(b_plan.body["status"], "none");
    assert_eq!(
        b_plan.body["onKey"], false,
        "asked the gateway after a planless licence"
    );
    let b_key = b.call("GET", "/api/v1/plan/key", None).await?;
    assert_eq!(b_key.body["code"], "not_linked");

    b.activate(KEY).await?;
    let reset = a.call("POST", "/api/v1/plan/key/reset", None).await?;
    assert_eq!(reset.status, StatusCode::OK, "{}", reset.raw);
    assert_eq!(reset.body["key"]["hint"], "8M3RP");
    assert_eq!(
        reset.body["key"]["computers"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(reset.body["plan"]["status"], "active");
    assert!(!reset.raw.contains("NEO-"));
    let old = b.activate(KEY).await?;
    assert_eq!(old.body["code"], "key_invalid");
    let new = b.activate(RESET_KEY).await?;
    assert_eq!(new.status, StatusCode::OK);

    gateway.script(
        "POST /v1/keys/reset",
        429,
        json!({"code": "rateLimited", "message": "You reset this key a few times today. Try again tomorrow."}),
        None,
    );
    let limited = a.call("POST", "/api/v1/plan/key/reset", None).await?;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.body["code"], "rate_limited");
    assert_eq!(
        limited.body["message"],
        "You reset this key a few times today. Try again tomorrow."
    );

    let a_id = a.install_id().await?;
    let itself = a
        .call(
            "POST",
            &format!("/api/v1/plan/key/computers/{a_id}/remove"),
            None,
        )
        .await?;
    assert_eq!(itself.status, StatusCode::OK);
    assert_eq!(itself.body["key"], Value::Null);
    assert_eq!(itself.body["plan"]["status"], "none");
    assert_eq!(itself.body["plan"]["onKey"], false);
    Ok(())
}

#[tokio::test]
async fn the_key_is_never_kept_logged_or_echoed() -> anyhow::Result<()> {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let gateway = FakeGateway::start().await?;
    let a = computer(Some(&gateway)).await?;
    let b = computer(Some(&gateway)).await?;
    let mut replies = Vec::new();
    buy(&a, &gateway).await?;
    replies.push(b.activate(KEY).await?.raw);
    replies.push(b.activate("NEO-ZZZZZ-ZZZZZ-ZZZZZ-ZZZZZ-ZZZZZ").await?.raw);
    replies.push(a.call("POST", "/api/v1/plan/key/reset", None).await?.raw);
    replies.push(b.activate(RESET_KEY).await?.raw);
    replies.push(b.call("GET", "/api/v1/plan/key", None).await?.raw);
    replies.push(b.call("POST", "/api/v1/plan/refresh", None).await?.raw);
    replies.push(
        a.call(
            "POST",
            "/api/v1/plan/key/activate",
            Some(json!({"key": 12345})),
        )
        .await?
        .raw,
    );

    let groups =
        |key: &str| -> Vec<String> { key.split('-').skip(2).map(str::to_string).collect() };
    let secrets: Vec<String> = gateway
        .state()
        .installs
        .iter()
        .map(|install| install.secret.clone())
        .collect();
    let logs = captured.text();
    for key in [KEY, RESET_KEY] {
        for group in groups(key) {
            for reply in &replies {
                assert!(
                    !reply.contains(&group),
                    "a reply has part of a key: {reply}"
                );
            }
            assert!(!logs.contains(&group), "a log line has part of a key");
            for dir in [a.data_dir(), b.data_dir()] {
                for file in files_under(&dir) {
                    let contents = std::fs::read(&file).unwrap_or_default();
                    let text = String::from_utf8_lossy(&contents);
                    assert!(
                        !text.contains(&group),
                        "{} has part of a key",
                        file.display()
                    );
                }
            }
        }
    }
    for secret in &secrets {
        assert!(
            !logs.contains(secret.as_str()),
            "a log line has the install secret"
        );
        for reply in &replies {
            assert!(!reply.contains(secret.as_str()));
        }
    }
    for dir in [a.data_dir(), b.data_dir()] {
        let holding: Vec<PathBuf> = files_under(&dir)
            .into_iter()
            .filter(|file| {
                let text =
                    String::from_utf8_lossy(&std::fs::read(file).unwrap_or_default()).into_owned();
                secrets.iter().any(|secret| text.contains(secret.as_str()))
            })
            .collect();
        assert_eq!(holding, [dir.join("billing/install.json")]);
    }
    Ok(())
}

#[tokio::test]
async fn cancel_resume_and_the_portal() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let a = computer(Some(&gateway)).await?;
    let none = a
        .call("POST", "/api/v1/plan/cancel", Some(json!({})))
        .await?;
    assert_eq!(none.status, StatusCode::CONFLICT);
    assert_eq!(none.body["code"], "not_linked");

    buy(&a, &gateway).await?;
    let cancelled = a
        .call(
            "POST",
            "/api/v1/plan/cancel",
            Some(json!({"feedback": "  Too pricey for now  "})),
        )
        .await?;
    assert_eq!(cancelled.status, StatusCode::OK, "{}", cancelled.raw);
    assert_eq!(cancelled.body["status"], "ending");
    assert!(cancelled.body["endsAt"].as_i64().is_some());
    assert_eq!(cancelled.body["features"], json!(["claw.ultrafast"]));
    let sent = gateway
        .requests()
        .last()
        .map(|logged| logged.body.clone())
        .unwrap_or_default();
    assert_eq!(
        serde_json::from_str::<Value>(&sent)?,
        json!({"feedback": "Too pricey for now"})
    );

    let long = a
        .call(
            "POST",
            "/api/v1/plan/cancel",
            Some(json!({"feedback": "x".repeat(1001)})),
        )
        .await?;
    assert_eq!(long.status, StatusCode::BAD_REQUEST);
    assert_eq!(long.body["code"], "invalid_request");

    let resumed = a.call("POST", "/api/v1/plan/resume", None).await?;
    assert_eq!(resumed.body["status"], "active");
    assert_eq!(resumed.body["endsAt"], Value::Null);

    let portal = a
        .call(
            "POST",
            "/api/v1/plan/portal",
            Some(json!({"flow": "updateCard"})),
        )
        .await?;
    assert_eq!(portal.status, StatusCode::OK);
    assert_eq!(
        portal.body["url"],
        "https://billing.example.com/p/session/test_fake?flow=updateCard"
    );
    let bad_flow = a
        .call("POST", "/api/v1/plan/portal", Some(json!({"flow": "nope"})))
        .await?;
    assert_eq!(bad_flow.status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn an_unknown_install_registers_again_and_reads_none() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let a = computer(Some(&gateway)).await?;
    buy(&a, &gateway).await?;
    let before = a.install_id().await?;
    gateway.forget_installs();

    let reset = a.call("POST", "/api/v1/plan/key/reset", None).await?;
    assert_eq!(reset.status, StatusCode::CONFLICT);
    assert_eq!(reset.body["code"], "not_linked");
    assert_eq!(gateway.count("POST", "/v1/installs"), 2);
    let after = a.install_id().await?;
    assert_ne!(after, before);
    assert!(!a.data_dir().join("billing/licence.json").exists());
    let plan = a.plan().await?;
    assert_eq!(plan["status"], "none");
    assert_eq!(plan["onKey"], false);

    let pasted = a.activate(KEY).await?;
    assert_eq!(pasted.status, StatusCode::OK, "{}", pasted.raw);
    Ok(())
}

#[tokio::test]
async fn a_failed_refresh_keeps_the_plan_and_marks_it_stale() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let a = computer(Some(&gateway)).await?;
    buy(&a, &gateway).await?;
    gateway.set_down(true);
    let refreshed = a.call("POST", "/api/v1/plan/refresh", None).await?;
    assert_eq!(refreshed.status, StatusCode::OK);
    assert_eq!(refreshed.body["status"], "active");
    assert_eq!(refreshed.body["stale"], true);
    assert_eq!(a.plan().await?["stale"], true);
    let key = a.call("GET", "/api/v1/plan/key", None).await?;
    assert_eq!(key.status, StatusCode::SERVICE_UNAVAILABLE);

    gateway.set_down(false);
    let calls = gateway.count("POST", "/v1/licence/refresh");
    let soon = a.call("POST", "/api/v1/plan/refresh", None).await?;
    assert_eq!(soon.body["stale"], true, "at most one refresh a minute");
    assert_eq!(gateway.count("POST", "/v1/licence/refresh"), calls);
    Ok(())
}

#[tokio::test]
async fn a_refresh_before_registering_does_not_hold_back_the_next_one() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let a = computer(Some(&gateway)).await?;
    let b = computer(Some(&gateway)).await?;
    assert_eq!(
        b.state.billing.scheduled_refresh().await,
        RefreshOutcome::Skipped
    );
    let early = b.call("POST", "/api/v1/plan/refresh", None).await?;
    assert_eq!(early.body["status"], "none");
    assert!(gateway.requests().is_empty());

    buy(&a, &gateway).await?;
    b.activate(KEY).await?;
    let b_id = b.install_id().await?;
    a.call(
        "POST",
        &format!("/api/v1/plan/key/computers/{b_id}/remove"),
        None,
    )
    .await?;
    let refreshed = b.call("POST", "/api/v1/plan/refresh", None).await?;
    assert_eq!(refreshed.body["status"], "none");
    assert_eq!(refreshed.body["onKey"], false);
    assert_eq!(gateway.count("POST", "/v1/licence/refresh"), 1);
    Ok(())
}

#[tokio::test]
async fn a_saved_licence_is_checked_again_on_start() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let mut a = computer(Some(&gateway)).await?;
    buy(&a, &gateway).await?;
    a.reopen(&gateway).await?;
    let plan = a.plan().await?;
    assert_eq!(plan["status"], "active");
    assert_eq!(plan["onKey"], true);

    let path = a.data_dir().join("billing/licence.json");
    let mut saved: Value = serde_json::from_str(&tokio::fs::read_to_string(&path).await?)?;
    let licence = saved["licence"].as_str().unwrap_or_default().to_string();
    let mut tampered = licence.into_bytes();
    if let Some(byte) = tampered.get_mut(10) {
        *byte = if *byte == b'A' { b'B' } else { b'A' };
    }
    saved["licence"] = json!(String::from_utf8(tampered)?);
    tokio::fs::write(&path, saved.to_string()).await?;
    a.reopen(&gateway).await?;
    assert_eq!(a.plan().await?["status"], "none");
    Ok(())
}

#[tokio::test]
async fn unpaid_keeps_the_key_working_with_no_features() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let a = computer(Some(&gateway)).await?;
    buy(&a, &gateway).await?;
    gateway.set_plan(Some(FakePlan {
        status: "unpaid",
        ..FakeGateway::active_plan()
    }));
    let plan = a.call("POST", "/api/v1/plan/refresh", None).await?.body;
    assert_eq!(plan["status"], "unpaid");
    assert_eq!(plan["features"], json!([]));
    assert!(!a.state.billing.has_feature("claw.ultrafast").await);
    let portal = a
        .call(
            "POST",
            "/api/v1/plan/portal",
            Some(json!({"flow": "billing"})),
        )
        .await?;
    assert_eq!(portal.status, StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn without_a_gateway_every_route_but_the_plan_is_unavailable() -> anyhow::Result<()> {
    let a = computer(None).await?;
    let plan = a.plan().await?;
    assert_eq!(plan["available"], false);
    assert_eq!(plan["status"], "none");
    let id = "5d0c6a1e-3b7f-4c2a-9e81-2f64b0a7c913";
    for (method, uri, body) in [
        ("GET", "/api/v1/plan/pricing".to_string(), None),
        (
            "POST",
            "/api/v1/plan/checkout".to_string(),
            Some(json!({"plan": "pro", "interval": "month"})),
        ),
        ("GET", format!("/api/v1/plan/checkout/{id}"), None),
        ("GET", "/api/v1/plan/key".to_string(), None),
        (
            "POST",
            "/api/v1/plan/key/activate".to_string(),
            Some(json!({"key": KEY})),
        ),
        (
            "POST",
            format!("/api/v1/plan/key/computers/{id}/remove"),
            None,
        ),
        (
            "POST",
            format!("/api/v1/plan/key/computers/{id}/rename"),
            Some(json!({"name": "Mac"})),
        ),
        ("POST", "/api/v1/plan/key/reset".to_string(), None),
        ("POST", "/api/v1/plan/cancel".to_string(), Some(json!({}))),
        ("POST", "/api/v1/plan/resume".to_string(), None),
        (
            "POST",
            "/api/v1/plan/portal".to_string(),
            Some(json!({"flow": "billing"})),
        ),
        ("POST", "/api/v1/plan/refresh".to_string(), None),
    ] {
        let reply = a.call(method, &uri, body).await?;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{method} {uri}");
        assert_eq!(reply.body["code"], "billing_unavailable", "{method} {uri}");
    }
    Ok(())
}

#[tokio::test]
async fn a_web_page_cannot_reach_the_plan_routes() -> anyhow::Result<()> {
    let gateway = FakeGateway::start().await?;
    let a = computer(Some(&gateway)).await?;
    for uri in ["/api/v1/plan", "/api/v1/plan/key", "/api/v1/plan/key/reset"] {
        let method = if uri.ends_with("reset") {
            "POST"
        } else {
            "GET"
        };
        let refused = call(&a.router, method, uri, None, Some("https://example.com")).await?;
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{uri}");
        assert_eq!(refused.body["code"], "forbidden");
        assert!(
            !refused
                .headers
                .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        );
    }
    assert!(gateway.requests().is_empty());
    let app = call(
        &a.router,
        "GET",
        "/api/v1/plan",
        None,
        Some(EXTENSION_ORIGIN),
    )
    .await?;
    assert_eq!(app.status, StatusCode::OK);
    let page = Request::builder()
        .uri("/api/v1/plan")
        .header(header::HOST, "localhost")
        .header("sec-fetch-site", "cross-site")
        .body(Body::empty())?;
    let response = a.router.clone().oneshot(page).await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    Ok(())
}
