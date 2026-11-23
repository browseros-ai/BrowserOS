//! `/chat` through the in-process router with a scripted backend: the stream, `409`, stop,
//! state and replay, the checkpoint writes, and the lifecycle races of DESIGN.md section 9,
//! chunk `runs`. Each test reads the stored row from a real `browseros.sqlite` in its own
//! temporary state folder.

mod chat_support;
mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Method, StatusCode, header};
use browseros_server::turn::{
    ChatError, ChatErrorCode, ConversationId, FinishReason, ToolCallId, ToolOwner, TurnEnd,
    TurnEvent,
};
use chat_support::{Events, Runner, Script, Step, chunk, chunk_types};
use sea_orm::sqlx::{SqlitePool, query, query_scalar, sqlite::SqliteConnectOptions};
use serde_json::{Value, json};
use support::{TestOptions, TestServer, body_json};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const CONVERSATION: &str = "0b1d2c3e-4f50-4617-8899-aabbccddeeff";

fn conversation() -> ConversationId {
    CONVERSATION.parse().unwrap_or_default()
}

async fn server_with(runner: Arc<Runner>) -> Result<TestServer, Box<dyn std::error::Error>> {
    let server = TestServer::with(TestOptions {
        runner: Some(runner.clone()),
        browser_replies: Some(Vec::new()),
        ..TestOptions::default()
    })
    .await?;
    runner.observe(server.app.store.clone());
    if let Some(browser) = &server.browser {
        runner.with_browser(browser.clone());
    }
    Ok(server)
}

fn ask(message: &str) -> Value {
    json!({
        "conversationId": CONVERSATION,
        "target": { "type": "browseros" },
        "message": message,
    })
}

async fn post(server: &TestServer, body: &Value) -> axum::http::Response<Body> {
    server.send(server.json(Method::POST, "/chat", body)).await
}

async fn stored(server: &TestServer) -> Option<Value> {
    server
        .app
        .store
        .conversations()
        .load(conversation())
        .await
        .ok()
        .flatten()
        .and_then(|row| serde_json::to_value(row.messages).ok())
}

async fn wait_for_backend(runner: &Runner) -> TestResult {
    // The backend records its input before taking any scripted step. A test can observe
    // admission without having the backend emit a stream event or checkpoint first.
    tokio::time::timeout(chat_support::PATIENCE, async {
        while runner.inputs().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    Ok(())
}

async fn seed_unreadable_transcript(server: &TestServer, payload: &str) -> TestResult<SqlitePool> {
    let db = SqlitePool::connect_with(
        SqliteConnectOptions::new().filename(server.state_dir().join("db/browseros.sqlite")),
    )
    .await?;
    query(
        "INSERT INTO conversations (id, messages, target_type, last_messaged_at, created_at, updated_at)
         VALUES (?, ?, 'browseros', 110, 90, 110)",
    )
    .bind(CONVERSATION)
    .bind(payload)
    .execute(&db)
    .await?;
    Ok(db)
}

fn step_start() -> Step {
    Step::Emit(TurnEvent::StepStart)
}

fn step_end() -> Step {
    Step::Emit(TurnEvent::StepEnd)
}

fn text(delta: &str) -> Step {
    Step::Emit(TurnEvent::Text(delta.into()))
}

fn started(id: &str) -> Step {
    Step::Emit(TurnEvent::ToolStarted {
        id: ToolCallId(id.into()),
        name: "run".into(),
        owner: ToolOwner::Server,
    })
}

fn input(id: &str) -> Step {
    Step::Emit(TurnEvent::ToolInput {
        id: ToolCallId(id.into()),
        name: "run".into(),
        input: json!({ "code": "return document.title" }),
        owner: ToolOwner::Server,
    })
}

fn done(id: &str) -> Step {
    Step::Emit(TurnEvent::ToolDone {
        id: ToolCallId(id.into()),
        result: Ok(json!("Example Domain")),
    })
}

fn answer(steps: Vec<Step>) -> Script {
    Script {
        steps,
        end: TurnEnd::Done(FinishReason::Stop),
    }
}

/// One step that says `words`, then stops.
fn says(words: &str) -> Script {
    answer(vec![step_start(), text(words), step_end()])
}

#[tokio::test]
async fn a_turn_streams_with_the_ui_stream_headers_and_ends_with_done() -> TestResult {
    let runner = Runner::new([answer(vec![
        step_start(),
        text("Checking."),
        started("call_1"),
        input("call_1"),
        done("call_1"),
        step_end(),
        step_start(),
        text("It is Example Domain."),
        step_end(),
    ])]);
    let server = server_with(runner).await?;

    let response = post(&server, &ask("What is the title?")).await;
    assert_eq!(response.status(), StatusCode::OK);
    for (name, value) in browseros_server::turn::STREAM_HEADERS {
        assert_eq!(
            response.headers().get(name).and_then(|v| v.to_str().ok()),
            Some(value),
            "{name}"
        );
        assert_eq!(
            response.headers().get_all(name).iter().count(),
            1,
            "{name} once"
        );
    }
    let events = Events::of(response).rest().await;
    assert_eq!(
        chunk_types(&events),
        [
            "start",
            "start-step",
            "text-start",
            "text-delta",
            "text-end",
            "tool-input-start",
            "tool-input-available",
            "tool-output-available",
            "finish-step",
            "start-step",
            "text-start",
            "text-delta",
            "text-end",
            "finish-step",
            "finish",
            "[DONE]",
        ]
    );
    let message_id = chunk(&events[0])["messageId"].clone();
    assert!(
        message_id
            .as_str()
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
    );

    // The stored answer has the id the stream announced, so a reload replaces in place.
    let row = stored(&server).await.unwrap_or_default();
    assert_eq!(row[0]["role"], json!("user"));
    assert_eq!(
        row[0]["parts"],
        json!([{ "type": "text", "text": "What is the title?" }])
    );
    assert_eq!(row[1]["id"], message_id);
    assert_eq!(row[1]["parts"][2]["state"], json!("output-available"));
    assert_eq!(row[1]["parts"][4]["text"], json!("It is Example Domain."));
    Ok(())
}

#[tokio::test]
async fn a_second_post_while_a_turn_runs_is_409_and_status_says_busy() -> TestResult {
    let runner = Runner::new([
        answer(vec![step_start(), text("Working"), Step::Gate, step_end()]),
        says("Second."),
    ]);
    let server = server_with(runner.clone()).await?;

    let mut first = Events::of(post(&server, &ask("one")).await);
    first.until("text-delta").await;
    let status = body_json(server.send(server.get("/status")).await).await;
    assert_eq!(
        status["can_update"],
        json!(false),
        "a live run blocks updates"
    );

    let second = post(&server, &ask("two")).await;
    assert_eq!(second.status(), StatusCode::CONFLICT);
    let envelope = body_json(second).await;
    assert!(
        envelope["error"]["message"].is_string(),
        "the chat error envelope"
    );

    runner.open_gate();
    assert_eq!(
        first.rest().await.last().map(String::as_str),
        Some("[DONE]")
    );
    let status = body_json(server.send(server.get("/status")).await).await;
    assert_eq!(status["can_update"], json!(true));
    let third = post(&server, &ask("two")).await;
    assert_eq!(
        third.status(),
        StatusCode::OK,
        "free again once the run ended"
    );
    Events::of(third).rest().await;
    Ok(())
}

#[tokio::test]
async fn stop_ends_the_stream_with_abort_and_stores_the_call_as_stopped() -> TestResult {
    let runner = Runner::new([answer(vec![
        step_start(),
        text("Clicking"),
        started("call_1"),
        started("call_2"),
        input("call_1"),
        Step::Gate,
    ])]);
    let server = server_with(runner).await?;
    let mut stream = Events::of(post(&server, &ask("click twice")).await);
    stream.until("tool-input-available").await;

    let stop = format!("/chat/{CONVERSATION}/stop");
    let stopped = server
        .send(server.request(Method::POST, &stop).body(Body::empty())?)
        .await;
    assert_eq!(body_json(stopped).await, json!({ "stopped": true }));

    let rest = stream.rest().await;
    assert_eq!(
        chunk_types(&rest),
        [
            "tool-output-error",
            "tool-input-available",
            "tool-output-error",
            "abort",
            "[DONE]"
        ]
    );
    assert_eq!(
        chunk(&rest[0])["errorText"],
        json!("Stopped"),
        "the call in flight"
    );
    assert_eq!(
        chunk(&rest[1]),
        json!({
            "type": "tool-input-available",
            "toolCallId": "call_2",
            "toolName": "run",
            "input": {},
        }),
        "the queued call gets an empty input before its error"
    );
    assert_eq!(
        chunk(&rest[2])["errorText"],
        json!("Not run"),
        "the call queued behind it"
    );

    let row = stored(&server).await.unwrap_or_default();
    let parts = &row[1]["parts"];
    assert_eq!(parts[2]["state"], json!("output-error"));
    assert_eq!(parts[2]["errorText"], json!("Stopped"));
    assert_eq!(
        parts[2]["input"],
        json!({ "code": "return document.title" })
    );
    assert_eq!(parts[3]["state"], json!("output-error"));
    assert_eq!(parts[3]["errorText"], json!("Not run"));
    assert_eq!(parts[3]["input"], json!({}));

    let state = body_json(
        server
            .send(server.get(&format!("/chat/{CONVERSATION}/state")))
            .await,
    )
    .await;
    assert_eq!(state["status"], json!("aborted"));
    let again = server
        .send(server.request(Method::POST, &stop).body(Body::empty())?)
        .await;
    assert_eq!(body_json(again).await, json!({ "stopped": false }));
    Ok(())
}

#[tokio::test]
async fn a_second_panel_gets_state_then_replay_then_live() -> TestResult {
    let runner = Runner::new([answer(vec![
        step_start(),
        text("Hello"),
        Step::Gate,
        text(" world"),
        step_end(),
    ])]);
    let server = server_with(runner.clone()).await?;
    let mut first = Events::of(post(&server, &ask("greet")).await);
    let mut seen = first.until("text-delta").await;

    let state_path = format!("/chat/{CONVERSATION}/state");
    let state = server.send(server.get(&state_path)).await;
    assert_eq!(
        state
            .headers()
            .get(header::CACHE_CONTROL)
            .map(|v| v.as_bytes()),
        Some(&b"no-store"[..])
    );
    let state = body_json(state).await;
    assert_eq!(state["status"], json!("running"));
    assert_eq!(state["conversationId"], json!(CONVERSATION));
    let messages = state["messages"].as_array().cloned().unwrap_or_default();
    assert_eq!(messages.len(), 1, "the question, not the answer in flight");
    assert_eq!(messages[0]["role"], json!("user"));
    let run_id = state["runId"].as_str().unwrap_or_default().to_owned();

    let stream_path = format!("/chat/{CONVERSATION}/stream?runId={run_id}");
    let mirror = server.send(server.get(&stream_path)).await;
    assert_eq!(mirror.status(), StatusCode::OK);
    let mut mirror = Events::of(mirror);
    let replayed = mirror.until("text-delta").await;
    assert_eq!(
        replayed, seen,
        "replay starts at `start` with the same messageId"
    );

    let other_run = uuid::Uuid::new_v4();
    let changed = server
        .send(server.get(&format!("/chat/{CONVERSATION}/stream?runId={other_run}")))
        .await;
    assert_eq!(changed.status(), StatusCode::CONFLICT);
    let current = server
        .send(server.get(&format!("/chat/{CONVERSATION}/stream")))
        .await;
    assert_eq!(
        current.status(),
        StatusCode::OK,
        "no runId attaches to the current run"
    );
    let unknown = uuid::Uuid::new_v4();
    let none = server
        .send(server.get(&format!("/chat/{unknown}/stream")))
        .await;
    assert_eq!(none.status(), StatusCode::NO_CONTENT);
    let missing = server
        .send(server.get(&format!("/chat/{unknown}/state")))
        .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    runner.open_gate();
    seen.extend(first.rest().await);
    let mirrored: Vec<String> = replayed.into_iter().chain(mirror.rest().await).collect();
    assert_eq!(mirrored, seen, "the mirror saw every frame, once, in order");
    assert_eq!(Events::of(current).rest().await, seen);

    // After the end: `/state` holds the answer, and the log still replays whole.
    let state = body_json(server.send(server.get(&state_path)).await).await;
    assert_eq!(state["status"], json!("completed"));
    assert_eq!(
        state["messages"][1]["parts"][1]["text"],
        json!("Hello world")
    );
    assert_eq!(state["chunkCount"], json!(seen.len() - 1));
    let late = Events::of(server.send(server.get(&stream_path)).await)
        .rest()
        .await;
    assert_eq!(late, seen);
    Ok(())
}

#[tokio::test]
async fn checkpoints_write_the_row_before_and_after_each_tool() -> TestResult {
    let runner = Runner::new([answer(vec![
        step_start(),
        text("Checking"),
        started("call_1"),
        input("call_1"),
        done("call_1"),
        step_end(),
        step_start(),
        text("Done."),
        step_end(),
    ])]);
    let server = server_with(runner.clone()).await?;
    Events::of(post(&server, &ask("check")).await).rest().await;

    let observed = runner.take_observed();
    assert_eq!(observed.len(), 2);
    assert_eq!(observed[0].after, "ToolInput");
    let before = observed[0].row.clone().unwrap_or_default();
    assert_eq!(before[0]["role"], json!("user"));
    assert_eq!(
        before[1]["parts"][2],
        json!({ "type": "tool-run", "toolCallId": "call_1", "state": "output-error",
                "input": { "code": "return document.title" }, "errorText": "Interrupted" }),
        "written before the tool runs, as if cut off right there"
    );
    assert_eq!(before[1]["parts"][1]["state"], json!("done"));
    assert_eq!(observed[1].after, "ToolDone");
    let after = observed[1].row.clone().unwrap_or_default();
    assert_eq!(after[1]["parts"][2]["state"], json!("output-available"));
    assert_eq!(after[1]["parts"][2]["output"], json!("Example Domain"));
    assert_eq!(after[1]["parts"].as_array().map(Vec::len), Some(3));

    let last = stored(&server).await.unwrap_or_default();
    assert_eq!(last[1]["parts"][4]["text"], json!("Done."));
    Ok(())
}

#[tokio::test]
async fn the_question_is_stored_while_the_backend_is_waiting_to_answer() -> TestResult {
    let runner = Runner::new([answer(vec![Step::Gate])]);
    let server = server_with(runner.clone()).await?;
    let mut body = ask("Will this survive a restart?");
    body["userMessageId"] = json!("question-1");
    let mut stream = Events::of(post(&server, &body).await);

    wait_for_backend(&runner).await?;
    let admitted = stored(&server).await;
    runner.open_gate();
    stream.rest().await;

    assert_eq!(
        admitted,
        Some(json!([{
            "id": "question-1", "role": "user",
            "parts": [{ "type": "text", "text": "Will this survive a restart?" }]
        }]))
    );
    Ok(())
}

#[tokio::test]
async fn a_turn_that_fails_before_saying_anything_writes_the_question_alone() -> TestResult {
    let runner = Runner::new([Script {
        steps: vec![step_start()],
        end: TurnEnd::Failed(ChatError::new(
            ChatErrorCode::ProviderConfig,
            "No provider selected",
            "Choose a provider in settings.",
        )),
    }]);
    let server = server_with(runner).await?;
    let events = Events::of(post(&server, &ask("hello")).await).rest().await;
    assert_eq!(
        chunk_types(&events),
        ["start", "start-step", "error", "[DONE]"]
    );
    let envelope: Value =
        serde_json::from_str(chunk(&events[2])["errorText"].as_str().unwrap_or_default())?;
    assert_eq!(envelope["error"]["code"], json!("provider_config"));
    let row = stored(&server).await.unwrap_or_default();
    assert_eq!(row.as_array().map(Vec::len), Some(1));
    assert_eq!(row[0]["role"], "user");
    assert_eq!(row[0]["parts"][0]["text"], "hello");
    let state = body_json(
        server
            .send(server.get(&format!("/chat/{CONVERSATION}/state")))
            .await,
    )
    .await;
    assert_eq!(state["status"], json!("failed"));
    Ok(())
}

#[tokio::test]
async fn an_incognito_turn_writes_nothing_and_answers_from_the_request() -> TestResult {
    let runner = Runner::new([answer(vec![
        step_start(),
        Step::Gate,
        text("Sure."),
        step_end(),
    ])]);
    let server = server_with(runner.clone()).await?;
    let body = json!({
        "conversationId": CONVERSATION,
        "target": { "type": "codex", "agentId": "8a2f2c1e-0000-4000-8000-000000000001" },
        "message": "and now?",
        "persist": false,
        "previousConversation": [
            { "role": "user", "content": "earlier" },
            { "role": "assistant", "content": "an answer" }
        ]
    });
    let mut stream = Events::of(post(&server, &body).await);
    assert_eq!(
        chunk_types(&stream.until("start-step").await),
        ["start", "start-step"]
    );
    assert_eq!(stored(&server).await, None, "at admission");
    runner.open_gate();
    assert_eq!(
        stream.rest().await.last().map(String::as_str),
        Some("[DONE]")
    );
    assert_eq!(stored(&server).await, None);
    let inputs = runner.inputs();
    let (_, input) = &inputs[0];
    assert!(input.ephemeral);
    let history = serde_json::to_value(&input.history[..])?;
    assert_eq!(history.as_array().map(Vec::len), Some(3));
    assert_eq!(history[0]["parts"][0]["text"], json!("earlier"));
    assert_eq!(history[2]["parts"][0]["text"], json!("and now?"));
    Ok(())
}

#[tokio::test]
async fn an_unreadable_conversation_is_left_untouched() -> TestResult {
    let runner = Runner::new([says("A backend should never see this.")]);
    let server = server_with(runner.clone()).await?;
    let unreadable = "not a transcript";
    let db = seed_unreadable_transcript(&server, unreadable).await?;

    let events = Events::of(post(&server, &ask("Could this replace the row?")).await)
        .rest()
        .await;
    assert_eq!(chunk_types(&events), ["start", "error", "[DONE]"]);
    assert!(runner.inputs().is_empty());
    let messages: String = query_scalar("SELECT messages FROM conversations WHERE id = ?")
        .bind(CONVERSATION)
        .fetch_one(&db)
        .await?;
    assert_eq!(messages, unreadable);
    Ok(())
}

#[tokio::test]
async fn the_same_user_message_id_twice_stores_the_question_once() -> TestResult {
    let runner = Runner::new([says("First try."), says("Second try."), says("Next.")]);
    let server = server_with(runner.clone()).await?;
    let mut retry = ask("Where is Paris?");
    retry["userMessageId"] = json!("u-1");
    Events::of(post(&server, &retry).await).rest().await;
    Events::of(post(&server, &retry).await).rest().await;

    let row = stored(&server).await.unwrap_or_default();
    let rows = row.as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 2, "one question, one answer: {row}");
    assert_eq!(rows[0]["id"], json!("u-1"));
    assert_eq!(rows[1]["parts"][1]["text"], json!("Second try."));
    assert_eq!(
        runner.inputs()[1].1.history.len(),
        1,
        "the retry answers the question alone"
    );

    let mut next = ask("And Rome?");
    next["userMessageId"] = json!("u-2");
    Events::of(post(&server, &next).await).rest().await;
    let row = stored(&server).await.unwrap_or_default();
    let ids: Vec<&Value> = row
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| &m["id"])
        .collect();
    assert_eq!(ids.len(), 4);
    assert_eq!((ids[0], ids[2]), (&json!("u-1"), &json!("u-2")));
    Ok(())
}

#[tokio::test]
async fn history_is_continued_only_for_the_target_that_wrote_it() -> TestResult {
    let runner = Runner::new([says("From the provider."), says("From codex.")]);
    let server = server_with(runner.clone()).await?;
    Events::of(post(&server, &ask("one")).await).rest().await;
    let codex = json!({
        "conversationId": CONVERSATION,
        "target": { "type": "codex", "agentId": "8a2f2c1e-0000-4000-8000-000000000001" },
        "message": "two",
    });
    Events::of(post(&server, &codex).await).rest().await;
    let inputs = runner.inputs();
    assert_eq!(inputs[0].1.history.len(), 1);
    assert_eq!(
        inputs[1].1.history.len(),
        1,
        "the provider's history is not codex's"
    );
    Ok(())
}

#[tokio::test]
async fn two_posts_racing_for_one_conversation_give_one_run() -> TestResult {
    let runner = Runner::new([says("Only once.")]);
    let server = server_with(runner.clone()).await?;
    let (first, second) = (ask("a"), ask("b"));
    let (a, b) = tokio::join!(post(&server, &first), post(&server, &second));
    let mut statuses = [a.status(), b.status()];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT]);
    let winner = if a.status() == StatusCode::OK { a } else { b };
    Events::of(winner).rest().await;
    assert_eq!(runner.inputs().len(), 1);
    Ok(())
}

#[tokio::test]
async fn a_post_racing_a_delete_does_not_bring_the_row_back() -> TestResult {
    let runner = Runner::new([
        says("Kept until deleted."),
        // Slow to unwind: the delete has to wait, and a POST in that window must be refused.
        answer(vec![
            step_start(),
            text("Writing"),
            started("call_1"),
            input("call_1"),
            Step::AfterStop(Duration::from_millis(300)),
            done("call_1"),
        ]),
        says("A fresh start."),
    ]);
    let server = server_with(runner.clone()).await?;
    Events::of(post(&server, &ask("one")).await).rest().await;
    assert!(stored(&server).await.is_some());

    let mut live = Events::of(post(&server, &ask("two")).await);
    live.until("tool-input-available").await;
    let conversations = server.app.conversations.clone();
    let deleting = tokio::spawn(async move { conversations.delete(conversation()).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let during = post(&server, &ask("three")).await;
    assert_eq!(
        during.status(),
        StatusCode::CONFLICT,
        "refused while the delete runs"
    );

    assert!(deleting.await??, "the row existed");
    assert_eq!(
        stored(&server).await,
        None,
        "the late checkpoint did not bring it back"
    );
    assert_eq!(
        runner.ended_sessions(),
        [conversation()],
        "the agent's session is ended"
    );
    assert_eq!(
        chunk_types(&live.rest().await).last().map(String::as_str),
        Some("[DONE]")
    );

    Events::of(post(&server, &ask("four")).await).rest().await;
    let row = stored(&server).await.unwrap_or_default();
    assert_eq!(
        row.as_array().map(Vec::len),
        Some(2),
        "a new row, without the deleted turns"
    );
    Ok(())
}

#[tokio::test]
async fn an_older_run_unwinding_late_never_overwrites_the_newer_one() -> TestResult {
    let runner = Runner::new([
        answer(vec![
            step_start(),
            started("call_1"),
            input("call_1"),
            Step::AfterStop(Duration::from_millis(300)),
            done("call_1"),
        ]),
        says("The newer answer."),
    ]);
    let server = server_with(runner).await?;
    let mut old = Events::of(post(&server, &ask("old")).await);
    old.until("tool-input-available").await;
    let stop = format!("/chat/{CONVERSATION}/stop");
    server
        .send(server.request(Method::POST, &stop).body(Body::empty())?)
        .await;
    let early = post(&server, &ask("new")).await;
    assert_eq!(
        early.status(),
        StatusCode::CONFLICT,
        "the old run has not ended yet"
    );
    old.rest().await;

    Events::of(post(&server, &ask("new")).await).rest().await;
    let row = stored(&server).await.unwrap_or_default();
    let last = row
        .as_array()
        .and_then(|rows| rows.last())
        .cloned()
        .unwrap_or_default();
    assert_eq!(last["parts"][1]["text"], json!("The newer answer."));
    let old_answer = &row[1]["parts"][1];
    assert_eq!(
        old_answer["output"],
        json!("Example Domain"),
        "its late result, before"
    );
    Ok(())
}

#[tokio::test]
async fn release_stops_the_run_and_forgets_it_but_keeps_the_row() -> TestResult {
    let runner = Runner::new([answer(vec![step_start(), text("Long answer"), Step::Gate])]);
    let server = server_with(runner).await?;
    let mut body = ask("long");
    body["browserContext"] = json!({ "activeTab": { "id": 7 } });
    let mut stream = Events::of(post(&server, &body).await);
    stream.until("text-delta").await;

    let release = format!("/chat/{CONVERSATION}");
    for _ in 0..2 {
        let response = server
            .send(
                server
                    .request(Method::DELETE, &release)
                    .body(Body::empty())?,
            )
            .await;
        assert_eq!(body_json(response).await, json!({ "success": true }));
    }
    assert_eq!(
        chunk_types(&stream.rest().await),
        ["text-end", "abort", "[DONE]"]
    );
    let state = server
        .send(server.get(&format!("/chat/{CONVERSATION}/state")))
        .await;
    assert_eq!(state.status(), StatusCode::NOT_FOUND);
    assert_eq!(server.app.conversations.current_panels().assignments, []);
    assert!(
        stored(&server).await.is_some(),
        "the row is history's to delete"
    );
    Ok(())
}

#[tokio::test]
async fn shutdown_also_stops_a_run_a_release_is_waiting_for() -> TestResult {
    let runner = Runner::new([answer(vec![
        step_start(),
        started("call_1"),
        input("call_1"),
        Step::Hang,
    ])]);
    let server = server_with(runner).await?;
    let mut stream = Events::of(post(&server, &ask("long tool")).await);
    stream.until("tool-input-available").await;

    let release = server.app.conversations.clone();
    let releasing = tokio::spawn(async move { release.release(conversation()).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        server.app.conversations.busy(),
        "a run still unwinding is busy"
    );

    let started = Instant::now();
    server
        .app
        .conversations
        .shutdown(Duration::from_millis(2_500))
        .await;
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(1_400) && took < Duration::from_millis(2_500),
        "shutdown waited for the released run, then dropped it: {took:?}"
    );
    let row = stored(&server).await.unwrap_or_default();
    assert_eq!(row[1]["parts"][1]["errorText"], json!("Stopped"));
    tokio::time::timeout(Duration::from_secs(1), releasing).await??;
    assert_eq!(
        chunk_types(&stream.rest().await),
        ["tool-output-error", "abort", "[DONE]"]
    );
    Ok(())
}

#[tokio::test]
async fn a_tab_the_agent_opens_is_in_the_panel_table_while_the_run_is_running() -> TestResult {
    let runner = Runner::new([answer(vec![
        step_start(),
        started("call_1"),
        input("call_1"),
        Step::OpenTab(42),
        done("call_1"),
        Step::Gate,
        step_end(),
    ])]);
    let server = server_with(runner.clone()).await?;
    let mut panels = Events::of(server.send(server.get("/chat/panels")).await);
    assert_eq!(
        panels.next().await.map(|data| chunk(&data)),
        Some(json!({ "assignments": [] }))
    );

    let mut body = ask("open a tab");
    body["browserContext"] = json!({ "activeTab": { "id": 7, "url": "https://example.com" } });
    let mut stream = Events::of(post(&server, &body).await);
    stream.until("tool-output-available").await;

    let running = loop {
        let table = chunk(&panels.next().await.unwrap_or_default());
        if table["assignments"].as_array().map(Vec::len) == Some(2) {
            break table;
        }
    };
    let assignments = &running["assignments"];
    assert_eq!(assignments[0]["tabId"], json!(7));
    assert!(
        assignments[0].get("autoOpenId").is_none(),
        "the tab that asked"
    );
    assert_eq!(assignments[1]["tabId"], json!(42));
    assert_eq!(assignments[1]["status"], json!("running"));
    assert!(
        assignments[1]["autoOpenId"].is_string(),
        "the tab the agent opened"
    );

    runner.open_gate();
    stream.rest().await;
    let finished = loop {
        let table = chunk(&panels.next().await.unwrap_or_default());
        if table["assignments"][1]["status"] == json!("completed") {
            break table;
        }
    };
    assert_eq!(
        finished["assignments"][1]["autoOpenId"],
        assignments[1]["autoOpenId"]
    );

    let forget = server
        .send(
            server
                .request(Method::DELETE, "/chat/panels/7")
                .body(Body::empty())?,
        )
        .await;
    assert_eq!(body_json(forget).await, json!({ "success": true }));
    let table = chunk(&panels.next().await.unwrap_or_default());
    assert_eq!(table["assignments"].as_array().map(Vec::len), Some(1));
    Ok(())
}

#[tokio::test]
async fn a_run_that_opens_tabs_and_ends_at_once_still_shows_them_running() -> TestResult {
    // Tab 42 is found after the tool returns, tab 43 only by the take before the end. With no
    // database writes (incognito) the end follows at once, before the panel reader polls.
    let runner = Runner::new([answer(vec![
        step_start(),
        started("call_1"),
        input("call_1"),
        Step::OpenTab(42),
        done("call_1"),
        Step::OpenTab(43),
        step_end(),
    ])]);
    let server = server_with(runner).await?;
    let mut panels = Events::of(server.send(server.get("/chat/panels")).await);
    assert_eq!(
        panels.next().await.map(|data| chunk(&data)),
        Some(json!({ "assignments": [] }))
    );

    let mut body = ask("open two tabs");
    body["persist"] = json!(false);
    Events::of(post(&server, &body).await).rest().await;

    // Only now does the reader poll: every table the run published is still there, in order.
    let mut opened = std::collections::BTreeMap::new();
    loop {
        let table = chunk(&panels.next().await.unwrap_or_default());
        let assignments = table["assignments"].as_array().cloned().unwrap_or_default();
        for assignment in &assignments {
            if assignment["status"] == json!("running")
                && assignment["autoOpenId"].is_string()
                && let Some(tab) = assignment["tabId"].as_i64()
            {
                opened.insert(tab, assignment["autoOpenId"].clone());
            }
        }
        if !assignments.is_empty()
            && assignments
                .iter()
                .all(|a| a["status"] == json!("completed"))
        {
            break;
        }
    }
    assert_eq!(
        opened.keys().copied().collect::<Vec<_>>(),
        [42, 43],
        "each opened tab was seen running with its autoOpenId before the terminal table"
    );
    Ok(())
}

#[tokio::test]
async fn the_panel_table_is_sent_again_every_five_seconds() -> TestResult {
    let server = server_with(Runner::new([])).await?;
    let mut panels = Events::of(server.send(server.get("/chat/panels")).await);
    let first = panels.next().await;
    let started = Instant::now();
    let heartbeat = panels.next().await;
    let waited = started.elapsed();
    assert_eq!(first, heartbeat);
    assert!(
        waited <= Duration::from_millis(5_500),
        "heartbeat after {waited:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_backend_that_ignores_stop_is_dropped_after_the_grace() -> TestResult {
    let runner = Runner::new([answer(vec![step_start(), text("Stuck"), Step::Hang])]);
    let server = server_with(runner).await?;
    let mut body = ask("hang");
    body["persist"] = json!(false);
    let mut stream = Events::of(post(&server, &body).await);
    stream.until("text-delta").await;
    let stop = format!("/chat/{CONVERSATION}/stop");
    let started = Instant::now();
    server
        .send(server.request(Method::POST, &stop).body(Body::empty())?)
        .await;
    let rest = stream.rest().await;
    let waited = started.elapsed();
    assert_eq!(chunk_types(&rest), ["text-end", "abort", "[DONE]"]);
    assert!(
        waited >= Duration::from_secs(5) && waited < Duration::from_secs(7),
        "dropped after {waited:?}"
    );
    Ok(())
}

#[tokio::test]
async fn shutdown_seals_a_turn_with_a_tool_in_flight_within_its_budget() -> TestResult {
    let runner = Runner::new([answer(vec![
        step_start(),
        started("call_1"),
        input("call_1"),
        // A tool that does not notice Stop: shutdown must drop it, not wait 5 s.
        Step::Hang,
    ])]);
    let server = server_with(runner).await?;
    let mut stream = Events::of(post(&server, &ask("long tool")).await);
    stream.until("tool-input-available").await;

    let started = Instant::now();
    server
        .app
        .conversations
        .shutdown(Duration::from_millis(2_500))
        .await;
    let took = started.elapsed();
    assert!(
        took < Duration::from_millis(2_500),
        "shutdown took {took:?}"
    );
    let row = stored(&server).await.unwrap_or_default();
    assert_eq!(row[1]["parts"][1]["errorText"], json!("Stopped"));
    let rest = stream.rest().await;
    assert_eq!(chunk_types(&rest), ["tool-output-error", "abort", "[DONE]"]);
    let refused = post(&server, &ask("after")).await;
    assert_eq!(
        refused.status(),
        StatusCode::CONFLICT,
        "no turn starts after shutdown"
    );
    Ok(())
}
