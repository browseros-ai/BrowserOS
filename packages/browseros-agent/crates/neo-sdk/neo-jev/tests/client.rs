//! The verbs over HTTP against a local stub Jev: request shape, long-context retry, the key file
//! re-read after a 401, fault mapping.

use std::error::Error;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use neo_jev::{FindOpts, FindOutcome, Jev, JevConfig, JevFault, Obstacle, PageText};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn Error>>;

const SHOP: &str = include_str!("fixtures/shop.txt");
const KEY: &str = "test-key-not-real";

/// One request the stub saw: its headers of interest and body.
#[derive(Debug, Clone)]
struct Seen {
    authorization: Option<String>,
    long_context: bool,
    body: Value,
}

type Reply = Box<dyn Fn(usize, &Value) -> (StatusCode, Value) + Send + Sync>;

#[derive(Clone)]
struct Stub {
    seen: Arc<Mutex<Vec<Seen>>>,
    reply: Arc<Reply>,
    /// When set, only `Bearer <this>` is answered; any other bearer gets a 401, as Jev does.
    accepts: Option<&'static str>,
}

async fn handle(
    State(stub): State<Stub>,
    headers: HeaderMap,
    body: String,
) -> (StatusCode, String) {
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let n = {
        let mut seen = stub
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        seen.push(Seen {
            authorization: header("authorization"),
            long_context: header("typesafe-danger-zone-long-context").as_deref() == Some("true"),
            body: body.clone(),
        });
        seen.len() - 1
    };
    if let Some(good) = stub.accepts
        && header("authorization") != Some(format!("Bearer {good}"))
    {
        let refusal = json!({ "detail": { "error_type": "authentication_error" } });
        return (StatusCode::UNAUTHORIZED, refusal.to_string());
    }
    let (status, reply) = (stub.reply)(n, &body);
    (status, reply.to_string())
}

/// Starts a stub Jev answering with `reply(request index, body)`; returns a client and the log.
async fn stub(reply: Reply) -> Result<(Jev, Arc<Mutex<Vec<Seen>>>), Box<dyn Error>> {
    let (url, seen) = serve(reply, None).await?;
    let jev = Jev::new(JevConfig::new(url, "jev-test".into(), KEY.into()));
    Ok((jev, seen))
}

/// Starts a stub Jev that answers only `accepts`' bearer when set; returns its URL and the log.
async fn serve(
    reply: Reply,
    accepts: Option<&'static str>,
) -> Result<(String, Arc<Mutex<Vec<Seen>>>), Box<dyn Error>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let state = Stub {
        seen: Arc::clone(&seen),
        reply: Arc::new(reply),
        accepts,
    };
    let app = Router::new()
        .route("/v1/systemone", post(handle))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/v1/systemone", listener.local_addr()?);
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok((url, seen))
}

fn seen(log: &Arc<Mutex<Vec<Seen>>>) -> Vec<Seen> {
    log.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn page(snapshot: &str) -> PageText {
    PageText {
        url: "https://shop.example/s?q=mug".into(),
        title: "Shop".into(),
        snapshot: snapshot.into(),
    }
}

/// Answers every Choice with `probs`, every Noul with `noul`, and the obstacle with `obstacle`.
fn answers(body: &Value, probs: &Value, noul: f64, obstacle: &str) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(questions) = body["questions"].as_object() {
        for (id, q) in questions {
            let answer = if id == "obstacle" {
                json!({ "type": "choice", "choice": obstacle, "probabilities": { obstacle: 1.0 } })
            } else if q["type"] == "noul" {
                json!({ "type": "noul", "noul": noul })
            } else {
                json!({ "type": "choice", "probabilities": probs })
            };
            out.insert(id.clone(), answer);
        }
    }
    json!({ "model": "jev-1.14.0", "answers": out, "usage": { "input_tokens": 1234, "output_tokens": 20 } })
}

#[tokio::test]
async fn find_sends_bare_refs_in_page_order_and_reads_the_verdict() -> TestResult {
    let (jev, log) = stub(Box::new(|_, body| {
        (
            StatusCode::OK,
            answers(
                body,
                &json!({ "e11": 0.9, "e10": 0.05, "none": 0.05 }),
                0.0,
                "cookie_banner",
            ),
        )
    }))
    .await?;
    let verdict = jev
        .find(&page(SHOP), "the blue mug", &FindOpts::default())
        .await?;
    assert_eq!(
        verdict.outcome,
        FindOutcome::Found,
        "0.95 summed over the mug's twins"
    );
    let pick = verdict.pick.ok_or("found carries a pick")?;
    assert_eq!(
        (pick.r#ref.as_str(), pick.p),
        ("e11", 0.95),
        "the hidden copy's 0.05 counts for its twin"
    );
    assert_eq!(
        verdict.obstacle,
        Obstacle::CookieBanner,
        "the obstacle rides along"
    );
    assert_eq!(
        (verdict.cost.calls, verdict.cost.input_tokens),
        (1, 1234),
        "one call, Jev's own token count"
    );

    let requests = seen(&log);
    let request = requests.first().ok_or("one request")?;
    assert_eq!(
        request.authorization.as_deref(),
        Some("Bearer test-key-not-real"),
        "Bearer prefix is required"
    );
    assert!(
        !request.long_context,
        "a small page needs no long-context header"
    );
    assert_eq!(request.body["model"], "jev-test", "the configured model");
    assert!(
        request.body["state"]["page"]
            .as_str()
            .is_some_and(|p| !p.contains("[aria-hidden]")),
        "merged hidden copies never reach Jev"
    );
    // The criteria's key order is the wire order Jev reads; serde_json would sort a map.
    let raw = request.body.to_string();
    let first = raw.find("\"e1\":null").ok_or("bare ref e1 offered")?;
    let later = raw.find("\"e50\":null").ok_or("bare ref e50 offered")?;
    let none = raw
        .find("\"none\":\"None of the offered")
        .ok_or("`none` offered")?;
    assert!(
        first < later && later < none,
        "options go out in page order, `none` last"
    );
    assert!(
        !raw.contains("\"e10\":null"),
        "the hidden copy is not an option"
    );
    Ok(())
}

#[test]
fn criteria_keep_page_order_even_where_sorting_would_differ() -> TestResult {
    // `e2` sorts after `e11` as a string; page order puts it first.
    let view = neo_jev::view::View::new(SHOP);
    let offered = view.offered(&view.candidates(None));
    let e2 = offered
        .iter()
        .position(|r| *r == "e2")
        .ok_or("e2 offered")?;
    let e11 = offered
        .iter()
        .position(|r| *r == "e11")
        .ok_or("e11 offered")?;
    assert!(e2 < e11, "page order, not string order");
    Ok(())
}

#[tokio::test]
async fn max_tokens_exceeded_is_retried_once_with_the_long_context_header() -> TestResult {
    let (jev, log) = stub(Box::new(|n, body| {
        if n == 0 {
            (
                StatusCode::BAD_REQUEST,
                json!({ "detail": { "error_type": "max_tokens_exceeded" } }),
            )
        } else {
            (
                StatusCode::OK,
                answers(body, &json!({ "e13": 0.97, "none": 0.03 }), 0.0, "none"),
            )
        }
    }))
    .await?;
    let verdict = jev
        .find(&page(SHOP), "the Add to cart button", &FindOpts::default())
        .await?;
    assert_eq!(
        verdict.cost.calls, 2,
        "the refused call and the retry both count"
    );
    assert!(
        verdict.saw.long_context,
        "the verdict says the header was needed"
    );
    let requests = seen(&log);
    assert_eq!(
        requests.iter().map(|r| r.long_context).collect::<Vec<_>>(),
        vec![false, true],
        "plain first, then once with the header"
    );
    Ok(())
}

#[tokio::test]
async fn a_big_page_sends_the_header_up_front_and_a_refusal_with_it_is_too_big() -> TestResult {
    let (jev, log) = stub(Box::new(|_, _| {
        (
            StatusCode::BAD_REQUEST,
            json!({ "detail": { "error_type": "max_tokens_exceeded" } }),
        )
    }))
    .await?;
    let mut big = String::from(SHOP);
    for i in 0..900 {
        big.push_str(&format!("- text: filler paragraph number {i} with enough words to weigh on the token estimate here\n"));
    }
    let fault = jev.check(&page(&big), "the page lists mugs").await;
    assert!(
        matches!(fault, Err(JevFault::TooBig { tokens }) if tokens > 30_000),
        "{fault:?}"
    );
    let requests = seen(&log);
    assert_eq!(requests.len(), 1, "no retry when the header was already on");
    assert!(
        requests.first().is_some_and(|r| r.long_context),
        "over 30k the header goes out at once"
    );
    Ok(())
}

#[tokio::test]
async fn server_errors_are_down_and_client_errors_are_refused() -> TestResult {
    let (jev, _) = stub(Box::new(|_, _| {
        (StatusCode::SERVICE_UNAVAILABLE, json!({ "detail": "busy" }))
    }))
    .await?;
    let fault = jev.check(&page(SHOP), "anything").await;
    assert!(
        matches!(&fault, Err(JevFault::Down(_))),
        "5xx is Jev down: {fault:?}"
    );

    let (jev, _) = stub(Box::new(|_, _| {
        (
            StatusCode::UNAUTHORIZED,
            json!({ "detail": { "error_type": "authentication_error", "message": "bad key" } }),
        )
    }))
    .await?;
    let fault = jev
        .find(&page(SHOP), "anything", &FindOpts::default())
        .await;
    assert_eq!(
        fault.as_ref().err(),
        Some(&JevFault::Refused { status: 401 }),
        "4xx is a refusal"
    );
    let text = fault.err().map(|f| f.to_string()).unwrap_or_default();
    assert!(!text.contains(KEY), "a fault never carries the key");

    let jev = Jev::new(JevConfig::new(
        "http://127.0.0.1:9/v1/systemone".into(),
        "m".into(),
        KEY.into(),
    ));
    let fault = jev.check(&page(SHOP), "anything").await;
    assert!(
        matches!(&fault, Err(JevFault::Down(_))),
        "no listener is Jev down: {fault:?}"
    );
    Ok(())
}

#[tokio::test]
async fn check_choose_and_find_all_round_trip() -> TestResult {
    let (jev, log) = stub(Box::new(|_, body| {
        (
            StatusCode::OK,
            answers(
                body,
                &json!({ "o0": 0.1, "o1": 0.85, "o2": 0.05 }),
                0.9,
                "none",
            ),
        )
    }))
    .await?;
    let check = jev.check(&page(SHOP), "the page lists mugs").await?;
    assert!(check.yes && !check.unsure, "0.9 is a clear yes");

    let options = vec![
        "sign in".to_owned(),
        "pick a size".to_owned(),
        "other".to_owned(),
    ];
    let choose = jev
        .choose(&page(SHOP), "what stops adding it", &options)
        .await?;
    assert_eq!(
        choose.choice, "pick a size",
        "the option string comes back, not its id"
    );
    assert_eq!(choose.ranked.len(), 3, "every option ranked");
    assert!(choose.clear, "0.85 over 0.10 is clear");

    let all = jev
        .find_all(
            &page(SHOP),
            "a product link",
            &FindOpts {
                role: Some("link".into()),
                ..FindOpts::default()
            },
        )
        .await?;
    let refs: Vec<&str> = all.picks.iter().map(|p| p.r#ref.as_str()).collect();
    assert_eq!(
        refs,
        vec!["e2", "e11", "e30", "e31"],
        "every link matched (stub says 0.9), page order"
    );
    assert!(all.unsure.is_empty(), "0.9 is outside the unsure band");
    let requests = seen(&log);
    let last = requests.last().ok_or("find_all sent a request")?;
    assert_eq!(
        last.body["questions"].as_object().map(|q| q.len()),
        Some(5),
        "one Noul per link group plus the obstacle"
    );
    Ok(())
}

#[test]
fn config_needs_a_readable_nonempty_key_file_and_never_prints_it() -> TestResult {
    let dir = std::env::temp_dir().join(format!("neo-jev-config-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let key_file = dir.join("key");
    std::fs::write(&key_file, format!("  {KEY}\n"))?;
    let path = key_file.to_string_lossy().into_owned();
    let lookup = |file: Option<String>| {
        move |name: &str| match name {
            "NEO_JEV_KEY_FILE" => file.clone(),
            _ => None,
        }
    };
    assert!(
        JevConfig::from_lookup(lookup(None)).is_none(),
        "unset: the verbs are unlisted"
    );
    assert!(
        JevConfig::from_lookup(lookup(Some("  ".into()))).is_none(),
        "empty: unlisted"
    );
    assert!(
        JevConfig::from_lookup(lookup(Some(
            dir.join("missing").to_string_lossy().into_owned()
        )))
        .is_none(),
        "unreadable: unlisted"
    );
    let config =
        JevConfig::from_lookup(lookup(Some(path))).ok_or("a readable key file configures Jev")?;
    let debug = format!("{config:?}");
    assert!(
        !debug.contains(KEY) && debug.contains("redacted"),
        "Debug hides the key: {debug}"
    );
    assert!(
        debug.contains("api.typesafe.ai") && debug.contains("jev-latest"),
        "defaults apply: {debug}"
    );
    std::fs::write(&key_file, "\n")?;
    assert!(
        JevConfig::from_lookup(lookup(Some(key_file.to_string_lossy().into_owned()))).is_none(),
        "a blank key file: unlisted"
    );
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

/// A key file in a fresh folder of its own, holding `key`.
fn key_file(name: &str, key: &str) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let dir = std::env::temp_dir().join(format!("neo-jev-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("key");
    std::fs::write(&file, format!("{key}\n"))?;
    Ok(file)
}

/// A client configured as a host would: from the key file, with the URL seam at the stub.
fn from_file(url: &str, file: &std::path::Path) -> Result<Jev, Box<dyn Error>> {
    let file = file.to_string_lossy().into_owned();
    let url = url.to_owned();
    let config = JevConfig::from_lookup(move |name| match name {
        "NEO_JEV_KEY_FILE" => Some(file.clone()),
        "NEO_JEV_URL" => Some(url.clone()),
        _ => None,
    })
    .ok_or("a readable key file configures Jev")?;
    Ok(Jev::new(config))
}

/// Answers every Noul 0.9 and every Choice with `o0` (the obstacle with `none`).
fn yes() -> Reply {
    Box::new(|_, body| {
        (
            StatusCode::OK,
            answers(body, &json!({ "o0": 1.0 }), 0.9, "none"),
        )
    })
}

fn bearers(log: &Arc<Mutex<Vec<Seen>>>) -> Vec<String> {
    seen(log)
        .into_iter()
        .map(|s| s.authorization.unwrap_or_default())
        .collect()
}

#[tokio::test]
async fn a_401_rereads_the_key_file_and_a_new_bearer_is_kept() -> TestResult {
    const OLD: &str = "test-key-old";
    const NEW: &str = "test-key-new";
    let (url, log) = serve(yes(), Some(NEW)).await?;
    let file = key_file("rotated", OLD)?;
    let jev = from_file(&url, &file)?;
    // The host rotates the file after start, as a licence refresher would.
    std::fs::write(&file, format!("{NEW}\n"))?;

    let check = jev.check(&page(SHOP), "the page lists mugs").await?;
    assert!(check.yes, "the retried request was answered");
    assert_eq!(
        check.cost.calls, 2,
        "the refused request and the retry both count"
    );
    let again = jev.check(&page(SHOP), "the page lists mugs").await?;
    assert_eq!(again.cost.calls, 1, "the new bearer is kept: no second 401");
    assert_eq!(
        bearers(&log),
        vec![
            format!("Bearer {OLD}"),
            format!("Bearer {NEW}"),
            format!("Bearer {NEW}")
        ],
        "old once, then the file's new bearer for the retry and every later request"
    );
    if let Some(dir) = file.parent() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

#[tokio::test]
async fn a_401_with_an_unchanged_or_missing_key_file_is_refused_without_a_retry() -> TestResult {
    let (url, log) = serve(yes(), Some("test-key-jev-wants")).await?;
    let file = key_file("unchanged", KEY)?;
    let jev = from_file(&url, &file)?;
    let fault = jev
        .find(&page(SHOP), "the blue mug", &FindOpts::default())
        .await;
    assert_eq!(
        fault.err(),
        Some(JevFault::Refused { status: 401 }),
        "the same bearer would be refused again"
    );
    assert_eq!(seen(&log).len(), 1, "an unchanged key is not sent twice");

    std::fs::remove_file(&file)?;
    let fault = jev.check(&page(SHOP), "anything").await;
    assert_eq!(
        fault.err(),
        Some(JevFault::Refused { status: 401 }),
        "a key file that went away leaves the 401 standing"
    );
    assert_eq!(seen(&log).len(), 2, "one request, no retry");

    std::fs::write(&file, "\n")?;
    let fault = jev.check(&page(SHOP), "anything").await;
    assert_eq!(
        fault.err(),
        Some(JevFault::Refused { status: 401 }),
        "a blank key file is no new bearer"
    );
    assert_eq!(seen(&log).len(), 3, "one request, no retry");
    if let Some(dir) = file.parent() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

#[tokio::test]
async fn parallel_requests_refused_by_the_same_stale_key_each_retry_once() -> TestResult {
    const NEW: &str = "test-key-new";
    let (url, log) = serve(yes(), Some(NEW)).await?;
    let file = key_file("parallel", "test-key-old")?;
    let jev = from_file(&url, &file)?;
    std::fs::write(&file, format!("{NEW}\n"))?;
    // 150 link groups: `find_all` sends two requests at once with the same stale bearer.
    let mut many = String::new();
    for i in 0..150 {
        many.push_str(&format!("- link \"Item {i}\" [ref=e{}]: /p/{i}\n", i + 1));
    }
    let all = jev
        .find_all(&page(&many), "an item link", &FindOpts::default())
        .await?;
    assert_eq!(all.saw.batches, 2, "two requests in parallel");
    assert_eq!(all.picks.len(), 150, "both answered after their retry");
    assert_eq!(
        all.cost.calls, 4,
        "each request was refused once and retried once"
    );
    let refused = bearers(&log)
        .iter()
        .filter(|b| *b != &format!("Bearer {NEW}"))
        .count();
    assert_eq!(refused, 2, "only the two first sends carried the stale key");
    if let Some(dir) = file.parent() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

#[tokio::test]
async fn a_reply_that_leaves_a_question_unanswered_is_unreadable_not_a_no() -> TestResult {
    let (jev, _) = stub(Box::new(|_, _| {
        (
            StatusCode::OK,
            json!({ "answers": {}, "usage": { "input_tokens": 1 } }),
        )
    }))
    .await?;
    let fault = jev.check(&page(SHOP), "the page lists mugs").await;
    assert!(
        matches!(&fault, Err(JevFault::Down(m)) if m.contains("`claim`")),
        "no answer at all is not a confident false: {fault:?}"
    );

    // Every answer but one link's Noul: `find_all` must not drop that element unseen.
    let (jev, _) = stub(Box::new(|_, body| {
        let mut reply = answers(body, &json!({}), 0.9, "none");
        if let Some(out) = reply["answers"].as_object_mut() {
            out.remove("e30");
        }
        (StatusCode::OK, reply)
    }))
    .await?;
    let fault = jev
        .find_all(
            &page(SHOP),
            "a product link",
            &FindOpts {
                role: Some("link".into()),
                ..FindOpts::default()
            },
        )
        .await;
    assert!(
        matches!(&fault, Err(JevFault::Down(m)) if m.contains("`e30`")),
        "one missing Noul makes the reply unreadable: {fault:?}"
    );

    // A Noul answered without its probability, and a Choice with no probabilities.
    let (jev, _) = stub(Box::new(|_, body| {
        let mut reply = answers(body, &json!({}), 0.9, "none");
        if let Some(claim) = reply["answers"]["claim"].as_object_mut() {
            claim.remove("noul");
        }
        (StatusCode::OK, reply)
    }))
    .await?;
    let fault = jev.check(&page(SHOP), "the page lists mugs").await;
    assert!(
        matches!(&fault, Err(JevFault::Down(_))),
        "a Noul without `noul`: {fault:?}"
    );
    let (jev, _) = stub(Box::new(|_, body| {
        (StatusCode::OK, answers(body, &json!({}), 0.9, "none"))
    }))
    .await?;
    let fault = jev
        .choose(&page(SHOP), "which", &["a".to_owned(), "b".to_owned()])
        .await;
    assert!(
        matches!(&fault, Err(JevFault::Down(m)) if m.contains("`choose`")),
        "a Choice with no probabilities: {fault:?}"
    );
    Ok(())
}
