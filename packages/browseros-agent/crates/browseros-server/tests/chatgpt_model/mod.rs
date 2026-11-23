//! Drives the extension router and the actual loopback callback against a fake token service.
//! Session assertions live here because the session is internal to the model layer.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Barrier;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use axum::routing::post;
use axum::{Form, Json};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tower::ServiceExt;

use super::*;
use crate::app::{App, Launch, Parts, Shutdown, read_install_id};
use crate::browser::{Browser, BrowserConfig};
use crate::model::{ModelMessage, Models, StepRequest};
use crate::store::{Credentials, ProviderConfig, ProviderKind};
use crate::turn::{FinishReason, TurnEvent};

type FormFields = BTreeMap<String, String>;

const START: &str = "/oauth/chatgpt-pro/start";
const STATUS: &str = "/oauth/chatgpt-pro/status";
const SIGN_OUT: &str = "/oauth/chatgpt-pro";
const TEST_ORIGIN: &str = "http://localhost:5173";

struct FakeUpstream {
    base: Url,
    exchange: Arc<Mutex<FakeExchange>>,
    task: JoinHandle<std::io::Result<()>>,
}

#[derive(Default)]
struct FakeExchange {
    forms: Vec<FormFields>,
    responses: Vec<(HeaderMap, Value)>,
    replies: VecDeque<(StatusCode, Value)>,
    release_reply: Option<Arc<Notify>>,
}

impl FakeUpstream {
    async fn start(replies: impl IntoIterator<Item = (StatusCode, Value)>) -> anyhow::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let base = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
        let exchange = Arc::new(Mutex::new(FakeExchange {
            replies: replies.into_iter().collect(),
            ..Default::default()
        }));

        let router = Router::new()
            .route("/authorize", get(|| async { "Fake authorize page" }))
            .route("/token", post(fake_token))
            .route("/backend-api/codex/responses", post(fake_responses))
            .with_state(Arc::clone(&exchange));
        let task = tokio::spawn(async move { axum::serve(listener, router).await });
        Ok(Self {
            base,
            exchange,
            task,
        })
    }

    fn endpoints(&self) -> anyhow::Result<OAuthEndpoints> {
        Ok(OAuthEndpoints {
            authorize: self.base.join("authorize")?.into(),
            token: self.base.join("token")?.into(),
            callback_port: 0,
            ..Default::default()
        })
    }

    async fn forms(&self) -> Vec<FormFields> {
        self.exchange.lock().await.forms.clone()
    }
}

impl Drop for FakeUpstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn fake_token(
    State(exchange): State<Arc<Mutex<FakeExchange>>>,
    Form(form): Form<FormFields>,
) -> (StatusCode, Json<Value>) {
    let (reply, release) = {
        let mut exchange = exchange.lock().await;
        exchange.forms.push(form);
        (exchange.replies.pop_front(), exchange.release_reply.take())
    };
    if let Some(release) = release {
        release.notified().await;
    }
    let (status, body) = reply.unwrap_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({"error": "unexpected token exchange"}),
    ));
    (status, Json(body))
}

async fn fake_responses(
    State(exchange): State<Arc<Mutex<FakeExchange>>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    exchange.lock().await.responses.push((headers, body));
    (
        [(header::CONTENT_TYPE, "text/event-stream")],
        concat!(
            r#"data: {"type":"response.output_text.delta","delta":"Signed in"}"#,
            "\n\n",
            r#"data: {"type":"response.completed","response":{"status":"completed","output":[]}}"#,
            "\n\n",
        ),
    )
}

struct Fixture {
    dir: TempDir,
    app: App,
    router: Router,
}

impl Fixture {
    async fn new(endpoints: OAuthEndpoints, damaged_install: bool) -> anyhow::Result<Self> {
        let dir = tempfile::tempdir()?;
        if damaged_install {
            std::fs::write(dir.path().join("installation.json"), "{truncated")?;
        }
        let install_id = read_install_id(dir.path()).ok();
        assert_eq!(install_id.is_none(), damaged_install);
        let store = Store::open(dir.path()).await?;
        let mut models = Models::new(store.clone(), install_id.clone());
        models.sign_in = Arc::new(SignIn::with_endpoints(
            store.clone(),
            install_id.clone(),
            endpoints,
        ));
        let launch = Launch {
            server_port: 9200,
            cdp_port: 9100,
            resources: dir.path().join("resources"),
            execution: dir.path().join("execution"),
            state_dir: dir.path().into(),
            dev_mode: true,
            browseros_version: None,
            install_id,
            trusted_origins: vec![TEST_ORIGIN.into()],
        };
        let browser = Browser::start(
            BrowserConfig::Off {
                reason: "OAuth test",
            },
            0,
            CancellationToken::new(),
        )
        .await;
        let app = App::assemble(Parts {
            launch,
            store,
            models,
            browser,
            runner: None,
            shutdown: Shutdown::new(),
        });
        let router = crate::http::router(app.clone());
        Ok(Self { dir, app, router })
    }

    async fn send(&self, method: Method, path: &str) -> anyhow::Result<Response> {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::ORIGIN, TEST_ORIGIN)
            .header(header::HOST, "127.0.0.1:9200")
            .body(Body::empty())?;
        Ok(self.router.clone().oneshot(request).await?)
    }

    /// `GET` with a loopback `Host` and exactly these other headers.
    async fn get_with(&self, path: &str, headers: &[(&str, &str)]) -> anyhow::Result<Response> {
        let mut request = Request::get(path).header(header::HOST, "127.0.0.1:9200");
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        Ok(self
            .router
            .clone()
            .oneshot(request.body(Body::empty())?)
            .await?)
    }

    async fn listening(&self) -> bool {
        self.sign_in().inner.session.lock().await.is_some()
    }

    async fn start(&self) -> anyhow::Result<Url> {
        let response = self.send(Method::GET, START).await?;
        assert_eq!(response.status(), StatusCode::FOUND);
        Ok(Url::parse(response.headers()[header::LOCATION].to_str()?)?)
    }

    async fn status(&self) -> anyhow::Result<Value> {
        let response = self.send(Method::GET, STATUS).await?;
        assert_eq!(response.status(), StatusCode::OK);
        json_body(response).await
    }

    fn sign_in(&self) -> &SignIn {
        self.app.models.sign_in()
    }

    async fn seed_legacy(&self, expires_at: i64, account: Option<&str>) -> anyhow::Result<()> {
        // Seed the same SQL columns and install key that the TypeScript server writes.
        let db_path = self.dir.path().join("db/browseros.sqlite");
        let options = sea_orm::sqlx::sqlite::SqliteConnectOptions::new().filename(db_path);
        let pool = sea_orm::sqlx::SqlitePool::connect_with(options).await?;
        let install_id = self
            .app
            .launch
            .install_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("test install id missing"))?;
        sea_orm::sqlx::query(
            "INSERT INTO oauth_tokens
              (browseros_id, provider, access_token, refresh_token, expires_at, email, account_id, updated_at)
             VALUES (?, 'chatgpt-pro', 'legacy-access', 'legacy-refresh', ?, 'old@example.test', ?, 1)",
        )
        .bind(install_id).bind(expires_at).bind(account).execute(&pool).await?;
        pool.close().await;
        Ok(())
    }
}

async fn json_body(response: Response) -> anyhow::Result<Value> {
    Ok(serde_json::from_slice(
        &to_bytes(response.into_body(), 1024 * 1024).await?,
    )?)
}

fn local_http() -> anyhow::Result<Client> {
    Ok(Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

fn callback_url(authorize: &Url, state: &str) -> anyhow::Result<Url> {
    let mut callback = Url::parse(&parameter(authorize, "redirect_uri")?)?;
    callback.set_host(Some("127.0.0.1"))?;
    callback
        .query_pairs_mut()
        .append_pair("code", "code+&%")
        .append_pair("state", state);
    Ok(callback)
}

fn parameter(url: &Url, name: &str) -> anyhow::Result<String> {
    url.query_pairs()
        .find_map(|(key, val)| (key == name).then(|| val.into_owned()))
        .ok_or_else(|| anyhow::anyhow!("missing URL parameter {name}"))
}

async fn assert_port_released(port: u16) -> anyhow::Result<()> {
    timeout(Duration::from_secs(2), async {
        loop {
            if TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await.is_ok() {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    Ok(())
}

fn fake_bearer() -> String {
    let claims = json!({
        "https://api.openai.com/auth": {"chatgpt_account_id": "account-1"},
        "https://api.openai.com/profile": {"email": "signed-in@example.test"},
        "account_id": "wrong-fallback",
        "email": "wrong-fallback@example.test",
    });
    format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(claims.to_string())
    )
}

fn assert_session(session: &Session, bearer: &str, account: Option<&str>) {
    assert_eq!(
        session.url.as_str(),
        "https://chatgpt.com/backend-api/codex/responses"
    );
    assert_eq!(session.access.expose(), bearer);
    assert_eq!(session.account_id.as_deref(), account);
}

#[tokio::test]
async fn redirect_and_loopback_callback_commit_the_pkce_grant() -> anyhow::Result<()> {
    let bearer = fake_bearer();
    let upstream = FakeUpstream::start([(
        StatusCode::OK,
        json!({
            "access_token": bearer, "refresh_token": "refresh-1", "expires_in": 3600,
        }),
    )])
    .await?;
    let fixture = Fixture::new(upstream.endpoints()?, false).await?;
    assert_eq!(
        fixture.status().await?,
        json!({"authenticated": false, "provider": "chatgpt-pro"})
    );

    let authorize = fixture.start().await?;
    assert_eq!(authorize.path(), "/authorize");
    let http = local_http()?;
    assert_eq!(
        http.get(authorize.clone()).send().await?.status(),
        StatusCode::OK
    );
    assert_eq!(parameter(&authorize, "response_type")?, "code");
    assert_eq!(parameter(&authorize, "client_id")?, CLIENT_ID);
    assert_eq!(parameter(&authorize, "code_challenge_method")?, "S256");
    assert_eq!(
        parameter(&authorize, "scope")?,
        "openid profile email offline_access"
    );
    assert_eq!(parameter(&authorize, "id_token_add_organizations")?, "true");
    assert_eq!(parameter(&authorize, "codex_cli_simplified_flow")?, "true");
    assert_eq!(parameter(&authorize, "originator")?, "browseros");

    let state = parameter(&authorize, "state")?;
    assert_eq!(URL_SAFE_NO_PAD.decode(&state)?.len(), 16);
    let wrong = http.get(callback_url(&authorize, "wrong")?).send().await?;
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
    assert!(upstream.forms().await.is_empty());

    let callback = callback_url(&authorize, &state)?;
    let port = callback
        .port()
        .ok_or_else(|| anyhow::anyhow!("callback port missing"))?;
    let good = http.get(callback).send().await?;
    assert_eq!(good.status(), StatusCode::OK);
    assert!(good.text().await?.contains("Authentication Successful"));
    assert_port_released(port).await?;

    let forms = upstream.forms().await;
    assert_eq!(forms.len(), 1);
    let form = &forms[0];
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["client_id"], CLIENT_ID);
    assert_eq!(
        form["redirect_uri"],
        format!("http://localhost:{port}/auth/callback")
    );
    assert_eq!(form["code"], "code+&%");
    assert_eq!(URL_SAFE_NO_PAD.decode(&form["code_verifier"])?.len(), 32);
    assert_eq!(
        parameter(&authorize, "code_challenge")?,
        URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes()))
    );
    assert_eq!(
        fixture.status().await?,
        json!({
            "authenticated": true, "provider": "chatgpt-pro", "email": "signed-in@example.test"
        })
    );
    assert_session(
        &fixture.sign_in().session().await?,
        &bearer,
        Some("account-1"),
    );
    let delete = fixture.send(Method::DELETE, SIGN_OUT).await?;
    assert_eq!(delete.status(), StatusCode::OK);
    assert_eq!(json_body(delete).await?, json!({"success": true}));
    assert_eq!(fixture.status().await?["authenticated"], false);
    Ok(())
}

#[tokio::test]
async fn closing_the_callback_tab_does_not_lose_an_exchanged_grant() -> anyhow::Result<()> {
    let upstream = FakeUpstream::start([(
        StatusCode::OK,
        json!({
            "access_token": fake_bearer(), "refresh_token": "refresh-1", "expires_in": 3600,
        }),
    )])
    .await?;
    let release = Arc::new(Notify::new());
    upstream.exchange.lock().await.release_reply = Some(Arc::clone(&release));
    let fixture = Fixture::new(upstream.endpoints()?, false).await?;
    let authorize = fixture.start().await?;
    let callback = callback_url(&authorize, &parameter(&authorize, "state")?)?;
    let browser = tokio::spawn(local_http()?.get(callback).send());
    timeout(Duration::from_secs(2), async {
        while upstream.forms().await.is_empty() {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    browser.abort();
    assert!(browser.await.is_err_and(|error| error.is_cancelled()));
    // Let the server see the browser disconnect before giving it the token response.
    sleep(Duration::from_millis(20)).await;
    release.notify_one();
    timeout(Duration::from_secs(2), async {
        while fixture.status().await?["authenticated"] != true {
            sleep(Duration::from_millis(10)).await;
        }
        anyhow::Ok(())
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn a_signed_in_model_streams_a_turn_through_the_codex_dialect() -> anyhow::Result<()> {
    let bearer = fake_bearer();
    let upstream = FakeUpstream::start([(
        StatusCode::OK,
        json!({
            "access_token": bearer, "refresh_token": "refresh-1", "expires_in": 3600,
        }),
    )])
    .await?;
    let fixture = Fixture::new(
        OAuthEndpoints {
            responses: upstream.base.join("backend-api/codex/responses")?.into(),
            ..upstream.endpoints()?
        },
        false,
    )
    .await?;
    let authorize = fixture.start().await?;
    let callback = callback_url(&authorize, &parameter(&authorize, "state")?)?;
    assert_eq!(
        local_http()?.get(callback).send().await?.status(),
        StatusCode::OK
    );

    let model = fixture
        .app
        .models
        .resolve(
            ProviderConfig {
                id: PROVIDER.into(),
                kind: ProviderKind::ChatgptPro,
                model: "fixture-model".into(),
                base_url: None,
                headers: Vec::new(),
                context_window: 8192,
                supports_images: false,
                reasoning_effort: None,
                reasoning_summary: None,
                resource_name: None,
                credentials: Credentials::SignIn,
            },
            Uuid::new_v4(),
        )
        .await?;
    let mut events = Vec::new();
    let turn = model
        .step(
            &StepRequest {
                system: "",
                messages: &[ModelMessage::User {
                    text: "Hello".into(),
                    images: Vec::new(),
                }],
                tools: &[],
                effort: None,
            },
            &mut events,
        )
        .await?;
    assert_eq!(turn.finish(), FinishReason::Stop);
    assert_eq!(events, [TurnEvent::Text("Signed in".into())]);
    let exchange = upstream.exchange.lock().await;
    assert_eq!(exchange.responses.len(), 1);
    let (headers, body) = &exchange.responses[0];
    assert_eq!(headers["authorization"], format!("Bearer {bearer}"));
    assert_eq!(headers["chatgpt-account-id"], "account-1");
    assert_eq!(headers["originator"], "browseros");
    assert_eq!(headers["openai-beta"], "responses=experimental");
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["instructions"], "You are a helpful assistant.");
    Ok(())
}

#[tokio::test]
async fn a_legacy_row_refreshes_once_for_two_turns_and_keeps_missing_claims() -> anyhow::Result<()>
{
    let upstream = FakeUpstream::start([(StatusCode::OK, json!({
        "access_token": "refreshed-access", "refresh_token": "rotated-refresh", "expires_in": 3600,
    }))]).await?;
    let fixture = Fixture::new(upstream.endpoints()?, false).await?;
    fixture
        .seed_legacy(now_ms() + 60_000, Some("legacy-account"))
        .await?;
    assert_eq!(
        fixture.status().await?,
        json!({
            "authenticated": true, "provider": "chatgpt-pro", "email": "old@example.test"
        })
    );
    let (one, two) = tokio::join!(fixture.sign_in().session(), fixture.sign_in().session());
    assert_session(&one?, "refreshed-access", Some("legacy-account"));
    assert_session(&two?, "refreshed-access", Some("legacy-account"));

    let forms = upstream.forms().await;
    assert_eq!(
        forms,
        [FormFields::from([
            ("grant_type".into(), "refresh_token".into()),
            ("client_id".into(), CLIENT_ID.into()),
            ("refresh_token".into(), "legacy-refresh".into()),
        ])]
    );
    assert_eq!(fixture.status().await?["email"], "old@example.test");
    let install_id = fixture
        .sign_in()
        .inner
        .install_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("test install id missing"))?;
    let row = fixture
        .app
        .store
        .tokens()
        .get(install_id, PROVIDER)
        .await?
        .ok_or_else(|| anyhow::anyhow!("refreshed row missing"))?;
    assert_eq!(row.refresh.expose(), "rotated-refresh");
    Ok(())
}

#[tokio::test]
async fn refused_refresh_clears_the_row_and_returns_auth_failed() -> anyhow::Result<()> {
    let upstream = FakeUpstream::start([(
        StatusCode::BAD_REQUEST,
        json!({
            "error": "invalid_grant"
        }),
    )])
    .await?;
    let fixture = Fixture::new(upstream.endpoints()?, false).await?;
    fixture.seed_legacy(now_ms() - 1, None).await?;
    let result = fixture.sign_in().session().await;
    let Err(error) = result else {
        anyhow::bail!("the fake refused the refresh")
    };
    assert_eq!(error.code, ChatErrorCode::AuthFailed);
    assert_eq!(
        error.message,
        "ChatGPT session expired. Please sign in again."
    );
    assert_eq!(fixture.status().await?["authenticated"], false);
    assert_eq!(upstream.forms().await.len(), 1);
    Ok(())
}

#[tokio::test]
async fn busy_port_and_other_start_failures_have_distinct_statuses() -> anyhow::Result<()> {
    let busy = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let port = busy.local_addr()?.port();
    let busy = tokio::spawn(async move {
        let routes = Router::new().route("/cancel", get(|| async { StatusCode::NOT_FOUND }));
        axum::serve(busy, routes).await
    });
    let fixture = Fixture::new(
        OAuthEndpoints {
            callback_port: port,
            ..Default::default()
        },
        false,
    )
    .await?;
    let response = fixture.send(Method::GET, START).await;
    busy.abort();
    let response = response?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        json_body(response).await?["error"]
            .as_str()
            .is_some_and(|error| error.contains("in use"))
    );

    let broken = Fixture::new(
        OAuthEndpoints {
            authorize: "not an URL".into(),
            ..Default::default()
        },
        false,
    )
    .await?;
    let response = broken.send(Method::GET, START).await?;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(json_body(response).await?["error"].is_string());
    Ok(())
}

#[tokio::test]
async fn no_install_id_disables_all_sign_in_routes() -> anyhow::Result<()> {
    let fixture = Fixture::new(OAuthEndpoints::default(), true).await?;
    for (method, path) in [
        (Method::GET, START),
        (Method::GET, STATUS),
        (Method::DELETE, SIGN_OUT),
    ] {
        let response = fixture.send(method, path).await?;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            json_body(response).await?["error"]
                .as_str()
                .is_some_and(|error| error.contains("installation id"))
        );
    }
    assert_eq!(
        std::fs::read_to_string(fixture.dir.path().join("installation.json"))?,
        "{truncated"
    );
    let unknown = fixture.send(Method::GET, "/oauth/qwen-code/status").await?;
    assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn token_store_failures_return_500_for_status_and_sign_out() -> anyhow::Result<()> {
    let fixture = Fixture::new(OAuthEndpoints::default(), false).await?;
    let options = sea_orm::sqlx::sqlite::SqliteConnectOptions::new()
        .filename(fixture.dir.path().join("db/browseros.sqlite"));
    let pool = sea_orm::sqlx::SqlitePool::connect_with(options).await?;
    sea_orm::sqlx::query("DROP TABLE oauth_tokens")
        .execute(&pool)
        .await?;
    pool.close().await;

    for (method, path) in [(Method::GET, STATUS), (Method::DELETE, SIGN_OUT)] {
        let response = fixture.send(method, path).await?;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            json_body(response).await?,
            json!({
                "error": "Could not access the ChatGPT sign-in store."
            })
        );
    }
    Ok(())
}

#[tokio::test]
async fn the_private_listener_hands_off_and_expires_abandoned_sign_ins() -> anyhow::Result<()> {
    let first = Fixture::new(
        OAuthEndpoints {
            callback_port: 0,
            ..Default::default()
        },
        false,
    )
    .await?;
    let first_auth = first.start().await?;
    let port = Url::parse(&parameter(&first_auth, "redirect_uri")?)?
        .port()
        .ok_or_else(|| anyhow::anyhow!("callback port missing"))?;
    let second = Fixture::new(
        OAuthEndpoints {
            callback_port: port,
            flow_ttl: Duration::from_millis(50),
            ..Default::default()
        },
        false,
    )
    .await?;
    let second_auth = second.start().await?;
    assert_ne!(
        parameter(&first_auth, "state")?,
        parameter(&second_auth, "state")?
    );
    assert!(first.sign_in().inner.session.lock().await.is_none());
    assert_port_released(port).await?;
    Ok(())
}

#[tokio::test]
async fn a_page_cannot_begin_a_sign_in_and_the_extension_can() -> anyhow::Result<()> {
    let ephemeral = || OAuthEndpoints {
        callback_port: 0,
        ..Default::default()
    };
    let fixture = Fixture::new(ephemeral(), false).await?;
    // What BrowserOS 0.51 sends, measured at the server, for a page's image, no-cors fetch and
    // link: no `Origin` on any of them.
    let from_page: [&[(&str, &str)]; 3] = [
        &[
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-mode", "no-cors"),
            ("sec-fetch-dest", "image"),
        ],
        &[
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-mode", "no-cors"),
            ("sec-fetch-dest", "empty"),
        ],
        &[
            ("sec-fetch-site", "cross-site"),
            ("sec-fetch-mode", "navigate"),
            ("sec-fetch-dest", "document"),
        ],
    ];
    for headers in from_page {
        let response = fixture.get_with(START, headers).await?;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{headers:?}");
        assert!(
            !fixture.listening().await,
            "{headers:?} opened the listener"
        );
    }

    // The extension's settings page pressing Add ChatGPT, as measured: a CORS `fetch` with no
    // `Origin`.
    let extension = fixture
        .get_with(
            START,
            &[
                ("sec-fetch-site", "none"),
                ("sec-fetch-mode", "cors"),
                ("sec-fetch-dest", "empty"),
            ],
        )
        .await?;
    assert_eq!(extension.status(), StatusCode::FOUND);
    assert!(fixture.listening().await);

    // A native caller sends no fetch metadata at all.
    let native = Fixture::new(ephemeral(), false).await?;
    assert_eq!(
        native.get_with(START, &[]).await?.status(),
        StatusCode::FOUND
    );
    Ok(())
}

#[tokio::test]
async fn pending_sign_ins_are_capped_and_the_oldest_is_dropped() -> anyhow::Result<()> {
    let fixture = Fixture::new(
        OAuthEndpoints {
            callback_port: 0,
            ..Default::default()
        },
        false,
    )
    .await?;
    let mut states = Vec::new();
    for _ in 0..MAX_PENDING + 2 {
        states.push(parameter(&fixture.start().await?, "state")?);
    }
    let session = fixture.sign_in().inner.session.lock().await;
    let pending: Vec<&String> = session
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("the listener closed"))?
        .pending
        .iter()
        .map(|(state, _)| state)
        .collect();
    assert_eq!(pending, states.iter().skip(2).collect::<Vec<_>>());
    Ok(())
}

#[test]
fn concurrent_install_id_creation_agrees_on_the_key_the_old_server_reads() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let barrier = Barrier::new(2);
    let ids = std::thread::scope(|scope| {
        let publish = || {
            barrier.wait();
            read_install_id(dir.path())
        };
        let first = scope.spawn(publish);
        let second = publish();
        (first.join(), second)
    });
    let id = ids
        .0
        .map_err(|_| anyhow::anyhow!("install id publisher panicked"))?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(ids.1.map_err(anyhow::Error::msg)?, id);
    let json: Value = serde_json::from_str(&std::fs::read_to_string(
        dir.path().join("installation.json"),
    )?)?;
    assert_eq!(json["install_id"], id);
    Ok(())
}
