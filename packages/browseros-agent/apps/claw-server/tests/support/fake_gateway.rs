//! A stateful stand-in for the billing gateway on `127.0.0.1:0`, signing licences with a test
//! key. It keeps one licence key and its plan, logs every request so a test can check what was
//! sent, and can be scripted to answer one call with a given reply.
#![allow(dead_code)]

use axum::{
    Router,
    body::{Bytes, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use claw_server::services::billing::BillingConfig;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
};
use tokio::task::JoinHandle;

pub const TEST_KID: &str = "test-1";
/// The key the fake sells, in the form the email shows it.
pub const KEY: &str = "NEO-7K2QM-AAAAA-BBBBB-CCCCC-DDDDD";
/// The key after one reset.
pub const RESET_KEY: &str = "NEO-8M3RP-EEEEE-FFFFF-GGGGG-HHHHH";
pub const CAP: usize = 5;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[must_use]
pub fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

/// The bytes the fake signs, as base64url, with their signature.
#[must_use]
pub fn sign(key: &SigningKey, payload: &Value) -> (String, String) {
    let bytes = payload.to_string().into_bytes();
    let signature = key.sign(&bytes);
    (
        URL_SAFE_NO_PAD.encode(&bytes),
        URL_SAFE_NO_PAD.encode(signature.to_bytes()),
    )
}

#[must_use]
pub fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or_default(),
    )
    .unwrap_or_default()
}

#[derive(Clone, Debug)]
pub struct Logged {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub authorization: Option<String>,
    pub body: String,
}

#[derive(Clone, Debug)]
pub struct FakeInstall {
    pub id: String,
    pub secret: String,
    pub label: Option<String>,
    pub platform: String,
    pub app_version: String,
}

#[derive(Clone, Debug)]
pub struct FakePlan {
    pub status: &'static str,
    pub period_end: i64,
    pub ends_at: Option<i64>,
    pub grace_until: Option<i64>,
}

#[derive(Clone, Debug)]
struct Scripted {
    status: StatusCode,
    body: Value,
    retry_after: Option<u32>,
}

#[derive(Debug)]
pub struct FakeState {
    pub installs: Vec<FakeInstall>,
    /// Installs on the key, oldest first, with when each joined.
    pub on_key: Vec<(String, i64)>,
    pub key: String,
    pub plan: Option<FakePlan>,
    pub checkouts: HashMap<String, (String, &'static str)>,
    pub requests: Vec<Logged>,
    pub down: bool,
    scripted: HashMap<String, VecDeque<Scripted>>,
    signing: SigningKey,
}

#[derive(Clone)]
pub struct FakeGateway {
    pub url: String,
    state: Arc<Mutex<FakeState>>,
    _task: Arc<JoinHandle<()>>,
}

impl FakeGateway {
    pub async fn start() -> anyhow::Result<Self> {
        let state = Arc::new(Mutex::new(FakeState {
            installs: Vec::new(),
            on_key: Vec::new(),
            key: KEY.to_string(),
            plan: None,
            checkouts: HashMap::new(),
            requests: Vec::new(),
            down: false,
            scripted: HashMap::new(),
            signing: signing_key(),
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let router = Router::new().fallback(handle).with_state(state.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router.into_make_service()).await;
        });
        Ok(Self {
            url: format!("http://{address}"),
            state,
            _task: Arc::new(task),
        })
    }

    /// The config a test billing service uses to reach this fake and trust its key.
    pub fn config(&self) -> anyhow::Result<BillingConfig> {
        Ok(BillingConfig {
            gateway: url::Url::parse(&self.url)?,
            keys: vec![(TEST_KID.to_string(), signing_key().verifying_key())],
            release: false,
        })
    }

    pub fn state(&self) -> MutexGuard<'_, FakeState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The next `route` call (such as `"POST /v1/keys/activate"`) answers with this instead.
    pub fn script(&self, route: &str, status: u16, body: Value, retry_after: Option<u32>) {
        self.state()
            .scripted
            .entry(route.to_string())
            .or_default()
            .push_back(Scripted {
                status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                body,
                retry_after,
            });
    }

    pub fn requests(&self) -> Vec<Logged> {
        self.state().requests.clone()
    }

    pub fn count(&self, method: &str, path: &str) -> usize {
        self.state()
            .requests
            .iter()
            .filter(|logged| logged.method == method && logged.path == path)
            .count()
    }

    /// Pays a checkout, as Stripe's page and the webhook would: the plan starts and the
    /// computer that bought it goes on the key.
    pub fn complete_checkout(&self, checkout_id: &str) {
        let mut state = self.state();
        let Some((install_id, status)) = state.checkouts.get_mut(checkout_id) else {
            return;
        };
        *status = "complete";
        let install_id = install_id.clone();
        state.plan = Some(FakePlan {
            status: "active",
            period_end: now_ms() + 30 * DAY_MS,
            ends_at: None,
            grace_until: None,
        });
        if !state.on_key.iter().any(|(id, _)| *id == install_id) {
            state.on_key.push((install_id, now_ms()));
        }
    }

    pub fn set_plan(&self, plan: Option<FakePlan>) {
        self.state().plan = plan;
    }

    pub fn active_plan() -> FakePlan {
        FakePlan {
            status: "active",
            period_end: now_ms() + 30 * DAY_MS,
            ends_at: None,
            grace_until: None,
        }
    }

    pub fn set_down(&self, down: bool) {
        self.state().down = down;
    }

    /// Puts `count` other computers on the key, as if pasted elsewhere.
    pub fn add_computers(&self, count: usize) -> Vec<String> {
        let mut state = self.state();
        (0..count)
            .map(|index| {
                let id = uuid::Uuid::new_v4().to_string();
                state.installs.push(FakeInstall {
                    id: id.clone(),
                    secret: format!("other-secret-{index}-0123456789abcdef"),
                    label: Some(format!("Other computer {index}")),
                    platform: "windows".to_string(),
                    app_version: "0.0.1".to_string(),
                });
                state.on_key.push((id.clone(), now_ms()));
                id
            })
            .collect()
    }

    /// The gateway forgets every install, so the next authenticated call is a 401.
    pub fn forget_installs(&self) {
        let mut state = self.state();
        state.installs.clear();
        state.on_key.clear();
    }

    pub fn label_of(&self, install_id: &str) -> Option<String> {
        self.state()
            .installs
            .iter()
            .find(|install| install.id == install_id)
            .and_then(|install| install.label.clone())
    }
}

type Shared = Arc<Mutex<FakeState>>;

async fn handle(State(state): State<Shared>, request: Request) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let query = request.uri().query().map(str::to_string);
    let headers = request.headers().clone();
    let body = to_bytes(request.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    let mut state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.requests.push(Logged {
        method: method.to_string(),
        path: path.clone(),
        query,
        authorization: headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
        body: String::from_utf8_lossy(&body).into_owned(),
    });
    if state.down {
        return reply(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"code": "internal", "message": "down"}),
        );
    }
    let route = route_name(&method, &path);
    if let Some(scripted) = state.scripted.get_mut(&route).and_then(VecDeque::pop_front) {
        let mut response = reply(scripted.status, scripted.body);
        if let Some(seconds) = scripted.retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        }
        return response;
    }
    serve(&mut state, &method, &path, &headers, &body)
}

/// `POST /v1/keys/computers/:installId/label` and `GET /v1/checkouts/:id` are named by their
/// templates, so a test can script them without knowing the id.
fn route_name(method: &Method, path: &str) -> String {
    let template = if path.starts_with("/v1/checkouts/") {
        "/v1/checkouts/:id".to_string()
    } else if path.starts_with("/v1/keys/computers/") {
        "/v1/keys/computers/:installId/label".to_string()
    } else {
        path.to_string()
    };
    format!("{method} {template}")
}

fn serve(
    state: &mut FakeState,
    method: &Method,
    path: &str,
    headers: &HeaderMap,
    body: &Bytes,
) -> Response {
    let body: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    match (method.as_str(), path) {
        ("POST", "/v1/installs") => {
            let id = uuid::Uuid::new_v4().to_string();
            let secret = format!("{}{}", uuid::Uuid::new_v4().simple(), "secret");
            state.installs.push(FakeInstall {
                id: id.clone(),
                secret: secret.clone(),
                label: body["label"].as_str().map(str::to_string),
                platform: body["platform"].as_str().unwrap_or_default().to_string(),
                app_version: body["appVersion"].as_str().unwrap_or_default().to_string(),
            });
            return reply(
                StatusCode::CREATED,
                json!({"installId": id, "installSecret": secret}),
            );
        }
        ("GET", "/v1/pricing") => {
            return reply(
                StatusCode::OK,
                json!({"app": "claw", "plans": [{
                    "plan": "pro",
                    "name": "neo Pro",
                    "description": null,
                    "features": ["claw.ultrafast"],
                    "prices": [
                        {"interval": "month", "intervalCount": 1, "amount": 1200, "currency": "usd"},
                        {"interval": "year", "intervalCount": 1, "amount": 12000, "currency": "usd"},
                        {"interval": "fortnight", "intervalCount": 1, "amount": 600, "currency": "usd"}
                    ]
                }]}),
            );
        }
        _ => {}
    }

    let Some(caller) = authenticate(state, headers) else {
        return reply(
            StatusCode::UNAUTHORIZED,
            json!({"code": "unauthorized", "message": "This install is not registered. Register it again."}),
        );
    };
    let caller_on_key = state.on_key.iter().any(|(id, _)| *id == caller);
    let not_linked = || {
        reply(
            StatusCode::CONFLICT,
            json!({"code": "notLinked", "message": "This computer isn't using a licence key."}),
        )
    };
    let not_found = || {
        reply(
            StatusCode::NOT_FOUND,
            json!({"code": "notFound", "message": "That computer isn't using this key."}),
        )
    };

    if let Some(rest) = path.strip_prefix("/v1/checkouts/")
        && method == Method::GET
    {
        let Some((owner, status)) = state.checkouts.get(rest).cloned() else {
            return reply(
                StatusCode::NOT_FOUND,
                json!({"code": "notFound", "message": "There is no such checkout for this install."}),
            );
        };
        if owner != caller {
            return reply(
                StatusCode::NOT_FOUND,
                json!({"code": "notFound", "message": "There is no such checkout for this install."}),
            );
        }
        let licence = (status == "complete").then(|| licence_for(state, &caller));
        return reply(
            StatusCode::OK,
            json!({"status": status, "licence": licence}),
        );
    }
    if let Some(rest) = path.strip_prefix("/v1/keys/computers/")
        && let Some(target) = rest.strip_suffix("/label")
        && method == Method::POST
    {
        if !caller_on_key {
            return not_linked();
        }
        if !state.on_key.iter().any(|(id, _)| id == target) {
            return not_found();
        }
        let Some(label) = body["label"]
            .as_str()
            .map(str::trim)
            .filter(|label| !label.is_empty() && label.encode_utf16().count() <= 40)
        else {
            return reply(
                StatusCode::BAD_REQUEST,
                json!({"code": "invalidRequest", "message": "The request is not valid. Check label."}),
            );
        };
        let label = label.to_string();
        if let Some(install) = state
            .installs
            .iter_mut()
            .find(|install| install.id == target)
        {
            install.label = Some(label);
        }
        return reply(
            StatusCode::OK,
            json!({"current": current_key(state, &caller)}),
        );
    }

    match (method.as_str(), path) {
        ("POST", "/v1/checkouts") => {
            if caller_on_key && state.plan.is_some() {
                return reply(
                    StatusCode::CONFLICT,
                    json!({"code": "alreadySubscribed", "message": "This computer already has a plan. Change it from billing settings."}),
                );
            }
            if body["plan"] != "pro" {
                return reply(
                    StatusCode::BAD_REQUEST,
                    json!({"code": "unknownPlan", "message": "That plan or billing period is not on sale."}),
                );
            }
            let id = uuid::Uuid::new_v4().to_string();
            state.checkouts.insert(id.clone(), (caller, "open"));
            reply(
                StatusCode::CREATED,
                json!({"checkoutId": id, "url": "https://checkout.example.com/c/pay/cs_test_fake#fragment"}),
            )
        }
        ("GET", "/v1/keys/current") => {
            if !caller_on_key {
                return not_linked();
            }
            reply(StatusCode::OK, current_key(state, &caller))
        }
        ("POST", "/v1/keys/activate") => {
            let pasted = body["key"].as_str().unwrap_or_default();
            if normalise(pasted) != normalise(&state.key) {
                return reply(
                    StatusCode::BAD_REQUEST,
                    json!({"code": "keyInvalid", "message": "That key didn't work. Copy it again from your other computer and paste it here."}),
                );
            }
            if let Some(replace) = body["replace"].as_str() {
                state.on_key.retain(|(id, _)| id != replace);
            }
            if !caller_on_key {
                if state.on_key.len() >= CAP {
                    let computers = computers(state, &caller);
                    return reply(
                        StatusCode::CONFLICT,
                        json!({"code": "keyFull", "message": "This key is on 5 computers. Remove one to make room for this computer.", "computers": computers}),
                    );
                }
                state.on_key.push((caller.clone(), now_ms()));
            }
            key_change(state, &caller)
        }
        ("POST", "/v1/keys/deactivate") => {
            if !caller_on_key {
                return not_linked();
            }
            let target = body["installId"].as_str().unwrap_or_default().to_string();
            if !state.on_key.iter().any(|(id, _)| *id == target) {
                return not_found();
            }
            state.on_key.retain(|(id, _)| *id != target);
            key_change(state, &caller)
        }
        ("POST", "/v1/keys/reset") => {
            if !caller_on_key {
                return not_linked();
            }
            state.key = RESET_KEY.to_string();
            state.on_key.retain(|(id, _)| *id == caller);
            key_change(state, &caller)
        }
        ("POST", "/v1/licence/refresh") => reply(StatusCode::OK, licence_for(state, &caller)),
        ("POST", "/v1/subscription/cancel") => {
            if !caller_on_key {
                return not_linked();
            }
            let Some(plan) = state.plan.as_mut() else {
                return not_linked();
            };
            plan.status = "ending";
            plan.ends_at = Some(plan.period_end);
            reply(
                StatusCode::OK,
                json!({"licence": licence_for(state, &caller)}),
            )
        }
        ("POST", "/v1/subscription/resume") => {
            if !caller_on_key {
                return not_linked();
            }
            let Some(plan) = state.plan.as_mut() else {
                return not_linked();
            };
            plan.status = "active";
            plan.ends_at = None;
            reply(
                StatusCode::OK,
                json!({"licence": licence_for(state, &caller)}),
            )
        }
        ("POST", "/v1/portal") => {
            if !caller_on_key {
                return not_linked();
            }
            reply(
                StatusCode::OK,
                json!({"url": format!("https://billing.example.com/p/session/test_fake?flow={}", body["flow"].as_str().unwrap_or_default())}),
            )
        }
        _ => reply(
            StatusCode::NOT_FOUND,
            json!({"code": "notFound", "message": "There is nothing here."}),
        ),
    }
}

fn authenticate(state: &FakeState, headers: &HeaderMap) -> Option<String> {
    let secret = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?;
    state
        .installs
        .iter()
        .find(|install| install.secret == secret)
        .map(|install| install.id.clone())
}

fn normalise(key: &str) -> String {
    key.to_ascii_uppercase()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .trim_start_matches("NEO")
        .to_string()
}

fn key_change(state: &FakeState, caller: &str) -> Response {
    let on_key = state.on_key.iter().any(|(id, _)| id == caller);
    let current = on_key.then(|| current_key(state, caller));
    reply(
        StatusCode::OK,
        json!({"licence": licence_for(state, caller), "current": current}),
    )
}

fn current_key(state: &FakeState, caller: &str) -> Value {
    json!({
        "hint": normalise(&state.key).chars().take(5).collect::<String>(),
        "cap": CAP,
        "computers": computers(state, caller),
    })
}

fn computers(state: &FakeState, caller: &str) -> Vec<Value> {
    state
        .on_key
        .iter()
        .filter_map(|(id, activated_at)| {
            let install = state.installs.iter().find(|install| install.id == *id)?;
            Some(json!({
                "installId": install.id,
                "label": install.label,
                "platform": install.platform,
                "lastSeenAt": iso(now_ms()),
                "activatedAt": iso(*activated_at),
                "thisComputer": install.id == caller,
            }))
        })
        .collect()
}

fn iso(epoch_ms: i64) -> String {
    let nanos = i128::from(epoch_ms) * 1_000_000;
    time::OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .ok()
        .and_then(|time| {
            time.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default()
}

fn licence_for(state: &FakeState, install_id: &str) -> Value {
    let on_key = state.on_key.iter().any(|(id, _)| id == install_id);
    let plan = state.plan.as_ref().filter(|_| on_key);
    let issued_at = now_ms();
    let payload = json!({
        "v": 3,
        "kid": TEST_KID,
        "installId": install_id,
        "app": "claw",
        "plan": plan.map(|_| "pro"),
        "features": match plan {
            Some(plan) if plan.status != "unpaid" => vec!["claw.ultrafast"],
            _ => Vec::new(),
        },
        "status": plan.map_or("none", |plan| plan.status),
        "periodEnd": plan.map(|plan| plan.period_end),
        "endsAt": plan.and_then(|plan| plan.ends_at),
        "graceUntil": plan.and_then(|plan| plan.grace_until),
        "issuedAt": issued_at,
        "expiresAt": issued_at + 3 * DAY_MS,
    });
    let (licence, signature) = sign(&state.signing, &payload);
    json!({"licence": licence, "signature": signature, "kid": TEST_KID})
}

fn reply(status: StatusCode, body: Value) -> Response {
    (status, axum::Json(body)).into_response()
}
