//! Replay and live are one cursor (DESIGN.md 5.4): a subscriber that attaches while frames are
//! being appended must see every frame of the run exactly once, in order, whenever it attached.
//! Twenty subscribers attach at staggered points of a 1,000-frame run on a multi-threaded
//! runtime, so appends and reads really race.

mod chat_support;
mod support;

use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use browseros_server::conversations::{Attach, RunStatus};
use browseros_server::turn::{FinishReason, TurnEnd, TurnEvent};
use chat_support::{Events, PATIENCE, Runner, Script, Step};
use futures_util::StreamExt as _;
use serde_json::json;
use support::{TestOptions, TestServer};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const CONVERSATION: &str = "1c2d3e4f-5061-4728-9a3b-4c5d6e7f8091";
const SUBSCRIBERS: usize = 20;
/// `start`, `start-step`, `text-start`, the deltas, `text-end`, `finish-step`, `finish`.
const FRAMES: usize = 1_000;
const DELTAS: usize = FRAMES - 6;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_subscriber_sees_each_frame_exactly_once() -> TestResult {
    let mut steps = vec![Step::Emit(TurnEvent::StepStart)];
    for index in 0..DELTAS {
        steps.push(Step::Emit(TurnEvent::Text(format!("{index} "))));
        steps.push(Step::Yield);
        // Slow enough that the attaching loop keeps up: most subscribers join a live run.
        if index % 50 == 49 {
            steps.push(Step::Pause(Duration::from_millis(2)));
        }
    }
    steps.push(Step::Emit(TurnEvent::StepEnd));
    let runner = Runner::new([Script {
        steps,
        end: TurnEnd::Done(FinishReason::Stop),
    }]);
    let server = TestServer::with(TestOptions {
        runner: Some(runner),
        ..TestOptions::default()
    })
    .await?;
    let id = CONVERSATION.parse()?;
    let started = Instant::now();
    let body = json!({ "conversationId": CONVERSATION, "target": { "type": "browseros" },
                       "message": "count", "persist": false });
    let response = server.send(server.json(Method::POST, "/chat", &body)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let poster = tokio::spawn(async move { Events::of(response).rest().await });

    // Attach the next subscriber each time the log has grown by another 1/20th, while the
    // run task keeps appending on another worker.
    let mut readers = Vec::new();
    let mut attached_live = 0;
    for index in 0..SUBSCRIBERS {
        let threshold = index * FRAMES / SUBSCRIBERS;
        let deadline = Instant::now() + PATIENCE;
        while server
            .app
            .conversations
            .state(id)
            .is_some_and(|state| state.frame_count < threshold)
        {
            assert!(Instant::now() < deadline, "the run stalled");
            tokio::task::yield_now().await;
        }
        let Attach::Run(subscription) = server.app.conversations.attach(id, None) else {
            panic!("subscriber {index} could not attach");
        };
        if server
            .app
            .conversations
            .state(id)
            .is_some_and(|state| state.status == RunStatus::Running)
        {
            attached_live += 1;
        }
        readers.push(tokio::spawn(async move {
            subscription
                .map(|frame| frame.data().to_owned())
                .collect::<Vec<String>>()
                .await
        }));
    }

    let mut seen = Vec::new();
    for reader in readers {
        seen.push(tokio::time::timeout(PATIENCE, reader).await??);
    }
    let elapsed = started.elapsed();
    let posted = poster.await?;

    let Attach::Run(replay) = server.app.conversations.attach(id, None) else {
        panic!("the finished run's log is gone");
    };
    let log: Vec<String> = replay.map(|frame| frame.data().to_owned()).collect().await;
    assert_eq!(log.len(), FRAMES);
    assert_eq!(
        posted.len(),
        FRAMES + 1,
        "the POST stream: every frame, then [DONE]"
    );
    assert_eq!(posted[..FRAMES], log[..]);
    for (index, frames) in seen.iter().enumerate() {
        assert_eq!(
            frames, &log,
            "subscriber {index}: a gap, a duplicate or a reordering"
        );
    }
    assert!(
        attached_live >= SUBSCRIBERS / 2,
        "only {attached_live} subscribers raced live appends"
    );

    #[expect(clippy::print_stderr, reason = "the numbers the pull request quotes")]
    {
        eprintln!(
            "{FRAMES} frames, {SUBSCRIBERS} subscribers ({attached_live} attached while the run \
             was appending), {} frames read, every subscriber equal to the log, in {elapsed:?}",
            seen.iter().map(Vec::len).sum::<usize>(),
        );
    }
    Ok(())
}
