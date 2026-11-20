//! The Jev verbs through the real door against a fake Jev on a local port: listed only with a
//! key, `neo.stop` beside them, verdicts as values, transport faults as the deployment's errors,
//! and no key anywhere.

use std::error::Error;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use neo_jev::{Jev, JevConfig};
use neo_trace::{Delivery, ErrorOwner, Outcome};
use serde_json::{Map, Value, json};
use tokio::net::TcpListener;

use super::{Harness, failure, value};
use crate::dispatch::ErrorCode;

/// The fake's key. A test checks it never comes back in a reply or the trace.
const KEY: &str = "jev-test-key-0c0ffee";

/// The helpers the `run` description lists with a key: the four verbs and `neo.stop`.
const NAMES: [&str; 5] = [
    "neo.find",
    "neo.findAll",
    "neo.check",
    "neo.choose",
    "neo.stop",
];

/// The page every call shows: a search box, a button, two story links.
const SNAPSHOT: &str = "- searchbox \"Search\" [ref=e1]\n- button \"Go\" [ref=e2]\n\
    - link \"First story\" [ref=e3]\n- link \"Second story\" [ref=e4]";

/// What the fake answers, and what it was asked.
#[derive(Default)]
struct Fake {
    /// The ref every pick question chooses; `None`: every pick answers `none`.
    pick: Option<&'static str>,
    /// The refs `findAll`'s yes/no questions say yes to.
    all: Vec<&'static str>,
    /// `check`'s probability.
    claim: f32,
    /// The option id `choose` picks, e.g. `o1`.
    choose: &'static str,
    /// Each request's `Authorization` header and body, in order.
    seen: Mutex<Vec<(String, Value)>>,
}

/// One answer per question, as Jev shapes them: a Choice's probabilities or a Noul's value.
async fn answer(State(fake): State<Arc<Fake>>, headers: HeaderMap, body: String) -> String {
    let request: Value = serde_json::from_str(&body).unwrap_or_default();
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    fake.seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push((auth, request.clone()));
    let mut answers = Map::new();
    let questions = request["questions"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for (id, question) in questions {
        let reply = if id == "obstacle" {
            json!({ "probabilities": { "none": 1.0 } })
        } else if id.starts_with("pick") {
            let offered = question["criteria"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            match fake.pick.filter(|pick| offered.contains_key(*pick)) {
                Some(pick) => json!({ "probabilities": { pick: 0.97, "none": 0.03 } }),
                None => json!({ "probabilities": { "none": 1.0 } }),
            }
        } else if id == "claim" {
            json!({ "noul": fake.claim })
        } else if id == "choose" {
            json!({ "probabilities": { fake.choose: 0.9 } })
        } else {
            let yes = fake.all.contains(&id.as_str());
            json!({ "noul": if yes { 0.95 } else { 0.02 } })
        };
        answers.insert(id, reply);
    }
    json!({ "answers": answers, "usage": { "input_tokens": 321 } }).to_string()
}

/// The fake on a free local port, and a client pointed at it.
async fn jev_verbs(fake: Fake) -> Result<(Jev, Arc<Fake>), Box<dyn Error>> {
    let fake = Arc::new(fake);
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
    let address = listener.local_addr()?;
    let app = Router::new()
        .route("/v1/systemone", post(answer))
        .with_state(Arc::clone(&fake));
    tokio::spawn(async move { axum::serve(listener, app).await });
    let config = JevConfig::new(
        format!("http://{address}/v1/systemone"),
        "jev-latest".to_owned(),
        KEY.to_owned(),
    );
    Ok((Jev::new(config), fake))
}

fn page(extra: &Value) -> Value {
    let mut input = json!({
        "url": "https://shop.example/",
        "title": "Shop",
        "snapshot": SNAPSHOT,
    });
    if let (Some(input), Some(extra)) = (input.as_object_mut(), extra.as_object()) {
        input.extend(extra.clone());
    }
    input
}

#[tokio::test]
async fn without_a_key_the_verbs_are_unlisted_and_the_deployments_to_fix()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::with_jev(None).await?;
    let listed = harness.dispatcher.registry().listed();
    for name in NAMES {
        assert!(
            !listed.iter().any(|spec| spec.name == name),
            "{name}: left out of the run description, so no model is offered what cannot work"
        );
    }
    let (run, _guard) = harness.run()?;
    let (op, reply) = harness
        .submit_op(&run, "neo.find", page(&json!({ "ask": "the search box" })))
        .await;
    let refused = failure(reply)?;
    assert_eq!(
        (refused.error.owner(), refused.error.code()),
        (ErrorOwner::Host, ErrorCode::JEV_UNAVAILABLE),
        "the deployment adds a key, not the script: {}",
        refused.error
    );
    assert_eq!(harness.row(op).await?.delivery, Some(Delivery::NotSent));
    Ok(())
}

#[tokio::test]
async fn with_a_key_find_answers_its_pick_as_a_value() -> Result<(), Box<dyn Error>> {
    let (jev, fake) = jev_verbs(Fake {
        pick: Some("e1"),
        ..Fake::default()
    })
    .await?;
    let harness = Harness::with_jev(Some(jev)).await?;
    let listed = harness.dispatcher.registry().listed();
    for name in NAMES {
        assert!(
            listed.iter().any(|spec| spec.name == name),
            "{name}: with a key the description offers it"
        );
    }
    let (run, _guard) = harness.run()?;
    let input = page(&json!({
        "ask": "the search box",
        "picked": [{ "ref": "e2", "ask": "the search button" }],
    }));
    let (op, reply) = harness.submit_op(&run, "neo.find", input).await;
    let answer = value(reply)?;
    assert_eq!(answer["outcome"], "found", "{answer}");
    assert_eq!(answer["pick"]["ref"], "e1", "{answer}");
    assert_eq!(answer["obstacle"], "none");
    assert_eq!(answer["saw"]["elements"], 4, "{answer}");
    assert_eq!(answer["cost"]["input_tokens"], 321, "{answer}");
    let row = harness.row(op).await?;
    assert_eq!(
        (row.outcome, row.delivery),
        (Some(Outcome::Ok), Some(Delivery::NotSent)),
        "a read of snapshot text: nothing reached the page"
    );
    let seen = fake.seen.lock().unwrap_or_else(PoisonError::into_inner);
    let (auth, request) = seen.first().ok_or("Jev was asked")?;
    assert_eq!(
        auth,
        &format!("Bearer {KEY}"),
        "the key goes to Jev and only there"
    );
    assert_eq!(request["state"]["url"], "https://shop.example/");
    assert!(
        !answer.to_string().contains(KEY),
        "the key never comes back to the script"
    );
    Ok(())
}

#[tokio::test]
async fn a_miss_is_a_value_not_an_error() -> Result<(), Box<dyn Error>> {
    let (jev, _fake) = jev_verbs(Fake::default()).await?;
    let harness = Harness::with_jev(Some(jev)).await?;
    let (run, _guard) = harness.run()?;
    let reply = harness
        .submit(
            &run,
            "neo.find",
            page(&json!({ "ask": "the checkout button" })),
        )
        .await;
    let answer = value(reply)?;
    assert_eq!(
        answer["outcome"], "not_found",
        "the prelude turns a miss into a stop; Rust only reports it: {answer}"
    );
    assert_eq!(answer["pick"], Value::Null);
    Ok(())
}

#[tokio::test]
async fn find_all_check_and_choose_answer_values() -> Result<(), Box<dyn Error>> {
    let (jev, fake) = jev_verbs(Fake {
        all: vec!["e3", "e4"],
        claim: 0.92,
        choose: "o1",
        ..Fake::default()
    })
    .await?;
    let harness = Harness::with_jev(Some(jev)).await?;
    let (run, _guard) = harness.run()?;

    let all = value(
        harness
            .submit(&run, "neo.findAll", page(&json!({ "ask": "a story link" })))
            .await,
    )?;
    let refs: Vec<&str> = all["picks"]
        .as_array()
        .ok_or("picks is a list")?
        .iter()
        .filter_map(|pick| pick["ref"].as_str())
        .collect();
    assert_eq!(refs, ["e3", "e4"], "every match, in page order: {all}");

    let check = value(
        harness
            .submit(
                &run,
                "neo.check",
                page(&json!({ "claim": "the page lists stories" })),
            )
            .await,
    )?;
    assert_eq!(check["yes"], true, "{check}");

    let chose = value(
        harness
            .submit(
                &run,
                "neo.choose",
                page(&json!({
                    "question": "what blocks the cart",
                    "options": ["pick a size", "sign in", "out of stock"],
                })),
            )
            .await,
    )?;
    assert_eq!(
        chose["choice"], "sign in",
        "the script's own string: {chose}"
    );

    let bodies = fake.seen.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(
        bodies
            .iter()
            .all(|(_, body)| body["questions"].get("obstacle").is_some()),
        "every request carries the free obstacle question"
    );
    Ok(())
}

#[tokio::test]
async fn jev_unreachable_is_jev_down_and_names_no_key() -> Result<(), Box<dyn Error>> {
    // A closed local port: the connection is refused at once.
    let config = JevConfig::new(
        "http://127.0.0.1:9/v1/systemone".to_owned(),
        "jev-latest".to_owned(),
        KEY.to_owned(),
    );
    let harness = Harness::with_jev(Some(Jev::new(config))).await?;
    let (run, _guard) = harness.run()?;
    let (op, reply) = harness
        .submit_op(
            &run,
            "neo.check",
            page(&json!({ "claim": "the page loaded" })),
        )
        .await;
    let down = failure(reply)?;
    assert_eq!(
        (down.error.owner(), down.error.code()),
        (ErrorOwner::Host, ErrorCode::JEV_DOWN),
        "{}",
        down.error
    );
    assert_eq!(down.delivery, Delivery::NotSent);
    assert!(!down.error.to_string().contains(KEY), "{}", down.error);
    let row = harness.row(op).await?;
    assert!(
        !format!("{row:?}").contains(KEY),
        "the trace never holds the key"
    );
    Ok(())
}

#[tokio::test]
async fn an_unknown_field_or_no_options_is_the_scripts_mistake() -> Result<(), Box<dyn Error>> {
    let (jev, _fake) = jev_verbs(Fake::default()).await?;
    let harness = Harness::with_jev(Some(jev)).await?;
    let (run, _guard) = harness.run()?;
    let typo = failure(
        harness
            .submit(
                &run,
                "neo.find",
                page(&json!({ "ask": "x", "rolee": "link" })),
            )
            .await,
    )?;
    assert_eq!(typo.error.owner(), ErrorOwner::Script, "{}", typo.error);
    let empty = failure(
        harness
            .submit(
                &run,
                "neo.choose",
                page(&json!({ "question": "q", "options": [] })),
            )
            .await,
    )?;
    assert_eq!(
        (empty.error.owner(), empty.error.code()),
        (ErrorOwner::Script, ErrorCode::INVALID_INPUT),
        "{}",
        empty.error
    );
    Ok(())
}

/// `neo.stop` is listed, not run: the prelude's stop is a refusal brand the door turns down at
/// admission, so only a raw call reaches it, and that is the script's mistake with nothing sent.
#[tokio::test]
async fn a_raw_neo_stop_call_is_the_scripts_mistake() -> Result<(), Box<dyn Error>> {
    for jev in [None, Some(jev_verbs(Fake::default()).await?.0)] {
        let harness = Harness::with_jev(jev).await?;
        let (run, _guard) = harness.run()?;
        let (op, reply) = harness
            .submit_op(&run, "neo.stop", json!({ "why": "no results list" }))
            .await;
        let raw = failure(reply)?;
        assert_eq!(
            (raw.error.owner(), raw.error.code()),
            (ErrorOwner::Script, ErrorCode::INVALID_INPUT),
            "{}",
            raw.error
        );
        assert!(
            raw.error.to_string().contains("neo.stop(page, why)"),
            "the refusal names the call to make: {}",
            raw.error
        );
        assert_eq!(harness.row(op).await?.delivery, Some(Delivery::NotSent));
    }
    Ok(())
}
