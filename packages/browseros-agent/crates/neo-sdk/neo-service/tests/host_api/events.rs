//! `GET /v1/events` over a real store: numbered live events, the filters, and what
//! `Last-Event-ID` resumes or resets.

use crate::support::{
    EventStream, Op, Sse, Start, TestResult, end_run, harness, op_ended, op_started, start_run,
};
use axum::http::StatusCode;
use neo_trace::{Delivery, OperationId};
use serde_json::json;

fn seqs(events: &[Sse]) -> Vec<u64> {
    events.iter().filter_map(Sse::seq).collect()
}

#[tokio::test]
async fn live_events_arrive_in_order_with_ids() -> TestResult {
    let h = harness().await?;
    let mut stream = EventStream::open(&h, "/v1/events", None).await?;
    let run = start_run(
        &h.sink,
        Start {
            task: Some("checkout"),
            ..Start::default()
        },
    );
    let click = OperationId::new(run, 1);
    op_started(&h.sink, click, Op::write(4, 1));
    op_ended(&h.sink, click, Delivery::Done, None);
    end_run(&h.sink, run, None);

    let events = stream.take(4).await?;
    let kinds = events
        .iter()
        .map(|event| event.kind.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            "run.started",
            "operation.started",
            "operation.ended",
            "run.ended"
        ]
    );
    assert_eq!(seqs(&events), [1, 2, 3, 4]);
    let boot = events[0].boot();
    assert!(boot.is_some_and(|boot| !boot.is_empty()));
    assert!(
        events.iter().all(|event| event.boot() == boot),
        "one process, one boot"
    );
    for event in &events {
        assert_eq!(
            event.data["type"],
            event.kind.as_str(),
            "the SSE type is the Event's"
        );
        assert_eq!(event.data["taskId"], "checkout");
    }
    assert_eq!(events[0].data["run"]["id"], run.to_string());
    assert!(
        events[0].data.get("tabId").is_none(),
        "a run is about no single tab"
    );
    assert_eq!(events[1].data["operation"]["id"], click.to_string());
    assert_eq!(events[1].data["tabId"], 4);
    assert_eq!(events[3].data["run"]["status"], "ok");
    Ok(())
}

#[tokio::test]
async fn an_operations_end_event_carries_its_notes() -> TestResult {
    let h = harness().await?;
    let mut stream = EventStream::open(&h, "/v1/events", None).await?;
    let run = start_run(&h.sink, Start::default());
    let click = OperationId::new(run, 1);
    op_started(&h.sink, click, Op::write(4, 1));
    h.sink
        .op_notes(click, vec!["attempting click action".to_string()]);
    op_ended(&h.sink, click, Delivery::Done, None);

    let events = stream.take(3).await?;
    assert_eq!(events[2].kind, "operation.ended", "notes publish no event");
    assert_eq!(
        events[2].data["operation"]["notes"],
        json!(["attempting click action"])
    );
    Ok(())
}

#[tokio::test]
async fn filters_keep_the_events_about_their_task_and_tab() -> TestResult {
    let h = harness().await?;
    let mut all = EventStream::open(&h, "/v1/events", None).await?;
    let mut task = EventStream::open(&h, "/v1/events?taskId=a", None).await?;
    let mut tab = EventStream::open(&h, "/v1/events?tab=4", None).await?;
    let mut both = EventStream::open(&h, "/v1/events?taskId=a&tab=4", None).await?;

    let a = start_run(
        &h.sink,
        Start {
            task: Some("a"),
            ..Start::default()
        },
    );
    op_started(&h.sink, OperationId::new(a, 1), Op::write(3, 1));
    op_ended(&h.sink, OperationId::new(a, 1), Delivery::Done, None);
    let b = start_run(
        &h.sink,
        Start {
            task: Some("b"),
            ..Start::default()
        },
    );
    op_started(&h.sink, OperationId::new(b, 1), Op::write(4, 2));
    op_ended(&h.sink, OperationId::new(b, 1), Delivery::Done, None);
    end_run(&h.sink, b, None);
    h.browser.open_tab(4);
    assert_eq!(
        h.post("/v1/tabs/4/private", "").await?.status,
        StatusCode::OK
    );
    op_started(&h.sink, OperationId::new(a, 2), Op::write(4, 3));
    op_ended(&h.sink, OperationId::new(a, 2), Delivery::Done, None);

    let everything = all.take(10).await?;
    assert_eq!(seqs(&everything), (1..=10).collect::<Vec<_>>());
    assert_eq!(everything[7].kind, "tab.private");
    // Event 10 matches every filter, so an event a filter let through by mistake would show
    // up among these, and one it dropped by mistake would leave a stream waiting.
    assert_eq!(seqs(&task.take(5).await?), [1, 2, 3, 9, 10]);
    assert_eq!(seqs(&tab.take(5).await?), [5, 6, 8, 9, 10]);
    assert_eq!(seqs(&both.take(2).await?), [9, 10]);
    Ok(())
}

#[tokio::test]
async fn a_client_resumes_after_its_last_event_id() -> TestResult {
    let h = harness().await?;
    let mut first = EventStream::open(&h, "/v1/events", None).await?;
    let run = start_run(&h.sink, Start::default());
    for seq in 1..=2 {
        op_started(
            &h.sink,
            OperationId::new(run, seq),
            Op::write(3, i64::from(seq)),
        );
    }
    let seen = first.take(3).await?;

    let mut resumed = EventStream::open(&h, "/v1/events", Some(&seen[0].id)).await?;
    let replayed = resumed.take(2).await?;
    assert_eq!(
        replayed.iter().map(|event| &event.id).collect::<Vec<_>>(),
        seen[1..].iter().map(|event| &event.id).collect::<Vec<_>>(),
        "only what came after its id, with no reset"
    );
    end_run(&h.sink, run, None);
    let live = resumed.next().await?;
    assert_eq!((live.kind.as_str(), live.seq()), ("run.ended", Some(4)));
    Ok(())
}

#[tokio::test]
async fn an_id_from_another_process_or_never_sent_starts_with_a_reset() -> TestResult {
    let h = harness().await?;
    let mut live = EventStream::open(&h, "/v1/events", None).await?;
    let run = start_run(&h.sink, Start::default());
    let first = live.next().await?;
    let boot = first.boot().ok_or("an id without a boot")?.to_string();

    let mut streams = Vec::new();
    for last in [
        "01JA0000000000000000000000-1".to_string(),
        "not-an-id".to_string(),
        format!("{boot}-99"),
    ] {
        streams.push(EventStream::open(&h, "/v1/events", Some(&last)).await?);
    }
    for stream in &mut streams {
        let reset = stream.next().await?;
        assert_eq!(reset.kind, "stream.reset");
        assert_eq!(reset.data["type"], "stream.reset");
        assert_eq!(
            reset.id,
            format!("{boot}-1"),
            "an id to resume from after re-reading"
        );
    }
    end_run(&h.sink, run, None);
    for stream in &mut streams {
        let next = stream.next().await?;
        assert_eq!(
            (next.kind.as_str(), next.seq()),
            ("run.ended", Some(2)),
            "live events, with no replay"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_client_the_ring_left_behind_gets_a_reset() -> TestResult {
    let h = harness().await?;
    let mut reader = EventStream::open(&h, "/v1/events", None).await?;
    let mut idle = EventStream::open(&h, "/v1/events", None).await?;
    let run = start_run(&h.sink, Start::default());
    let boot = reader.next().await?.boot().ok_or("no boot")?.to_string();
    // 1100 operations after the run's start: more than the 1024 the ring keeps. They go in
    // chunks the reader drains, so the numbering task never falls a whole broadcast behind.
    let (chunks, chunk) = (5, 220);
    for start in (0..chunks).map(|index| index * chunk) {
        for seq in start + 1..=start + chunk {
            op_started(
                &h.sink,
                OperationId::new(run, seq),
                Op::write(3, i64::from(seq)),
            );
        }
        reader.take(usize::try_from(chunk)?).await?;
    }
    let newest = 1 + u64::from(chunks * chunk);

    let mut stale = EventStream::open(&h, "/v1/events", Some(&format!("{boot}-2"))).await?;
    for stream in [&mut idle, &mut stale] {
        let reset = stream.next().await?;
        assert_eq!(reset.kind, "stream.reset");
        assert_eq!(reset.id, format!("{boot}-{newest}"));
    }
    end_run(&h.sink, run, None);
    for stream in [&mut idle, &mut stale] {
        let next = stream.next().await?;
        assert_eq!(
            (next.kind.as_str(), next.seq()),
            ("run.ended", Some(newest + 1))
        );
    }
    Ok(())
}
