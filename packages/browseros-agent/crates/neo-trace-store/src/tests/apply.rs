//! Neo: Replay, reordering, storage budgets and caps are checked at the transactional boundary.

use std::sync::Arc;

use neo_runtime::{Delivery, GetterResult, RunEvent};
use sea_orm::{EntityTrait, PaginatorTrait};

use super::support::{after, apply, before, ended, event, getter, inspect, scratch, started};
use crate::db::entities::{trace_calls, trace_getters, trace_runs};
use crate::db::kinds;
use crate::db::migration::{Migrator, run_pending};
use crate::ingest::Command;
use crate::{CallQuery, CallState, Detail, StoreConfig, TraceStore, apply, writer};

#[tokio::test]
async fn replay_and_out_of_order_ends_preserve_rows_delivery_and_absolute_losses() {
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let conn = inspect(&path).await;
    let start = started("s");
    let run = start.run.id;
    let commands = [
        Command::Started(start),
        event(after(run, 1)),
        event(before(run, 1)),
        event(RunEvent::DeliverySettled {
            run,
            call: 1,
            delivery: Delivery::Done,
        }),
        event(ended(run, 1)),
        Command::Event {
            event: RunEvent::RunSettled { run },
            stub: false,
            loss: (3, 4),
        },
    ];
    apply(&conn, &commands).await;
    let first_run = store.run(&run).await.unwrap().unwrap();
    let first_calls = store
        .calls(
            &run,
            CallQuery {
                after: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    apply(&conn, &commands).await;
    apply(&conn, &[event(after(run, 1))]).await;
    assert_eq!(
        store.run(&run).await.unwrap().unwrap(),
        first_run,
        "replay is durable-state idempotent"
    );
    assert_eq!(
        store
            .calls(
                &run,
                CallQuery {
                    after: None,
                    limit: 10
                }
            )
            .await
            .unwrap(),
        first_calls,
        "old end cannot regress final delivery"
    );
    assert_eq!(
        first_calls.items[0].delivery,
        Some(Delivery::Done),
        "final delivery wins"
    );
    assert_eq!(first_run.summary.loss.stubbed, 3, "absolute stubs");
    assert_eq!(first_run.summary.loss.dropped, 4, "absolute loss");
    assert_eq!(
        first_calls.items[0].selector.as_deref(),
        Some("#button"),
        "late hint enriches end row"
    );
}

#[tokio::test]
async fn any_event_can_create_a_placeholder_and_settlement_marks_missing_ends_lost() {
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let conn = inspect(&path).await;
    let start = started("s");
    let run = start.run.id;
    apply(
        &conn,
        &[
            event(before(run, 1)),
            event(ended(run, 2)),
            event(RunEvent::RunSettled { run }),
        ],
    )
    .await;
    let row = store.run(&run).await.unwrap().unwrap();
    assert_eq!(
        row.summary.start_page, None,
        "placeholder has no invented page"
    );
    assert_eq!(
        row.summary.session.as_str(),
        run.to_string(),
        "unlabelled fallback session"
    );
    assert_eq!(
        row.summary.started_at_ms,
        ulid::Ulid::from_string(&run.to_string())
            .unwrap()
            .timestamp_ms(),
        "ULID time"
    );
    assert_eq!(
        store
            .calls(
                &run,
                CallQuery {
                    after: None,
                    limit: 10
                }
            )
            .await
            .unwrap()
            .items[0]
            .state,
        CallState::Lost,
        "missing end is explicit"
    );
}

#[tokio::test]
async fn detail_budget_stubs_getters_calls_and_large_run_ends_without_losing_kinds() {
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let conn = inspect(&path).await;
    let start = started("s");
    let run = start.run.id;
    let mut big = getter(run, 1);
    if let RunEvent::Getter { result, .. } = &mut big {
        *result = GetterResult::Ok {
            summary: "x".repeat(16 << 20).into(),
        };
    }
    let mut big_end = ended(run, 1);
    if let RunEvent::RunEnded { script, .. } = &mut big_end {
        *script = Some(Arc::from("x".repeat((16 << 20) + 1)));
    }
    apply(
        &conn,
        &[Command::Started(start), event(big), event(big_end)],
    )
    .await;
    assert_eq!(
        store.getters(&run).await.unwrap()[0].detail,
        Detail::Stub,
        "oversized getter stub"
    );
    assert!(
        store.run(&run).await.unwrap().unwrap().script_truncated,
        "oversized end stub"
    );
    // Exhaust the actual budget with multiple accepted rows, then inspect the next call.
    let mut batch = Vec::new();
    for no in 2..=260 {
        let mut item = getter(run, no);
        if let RunEvent::Getter { result, .. } = &mut item {
            *result = GetterResult::Ok {
                summary: "x".repeat(64 << 10).into(),
            };
        }
        batch.push(event(item));
    }
    let mut last = after(run, 1);
    if let RunEvent::AfterCall { record, .. } = &mut last {
        Arc::make_mut(record).params = "x".repeat(70 << 10).into();
    }
    batch.push(event(last));
    apply(&conn, &batch).await;
    let run_row = trace_runs::Entity::find_by_id(run.to_string())
        .one(&conn)
        .await
        .unwrap()
        .unwrap();
    assert!(run_row.detail_bytes <= 16 << 20, "detail budget is hard");
    let calls = store
        .calls(
            &run,
            CallQuery {
                after: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        calls.items[0].detail,
        Detail::Stub,
        "later calls keep only their kind and timing"
    );
    assert_eq!(
        calls.items[0].state,
        CallState::Ok,
        "call outcome survives stubbing"
    );
}

#[tokio::test]
async fn caps_keep_fifty_thousand_calls_and_ten_thousand_getters_and_count_replay_once() {
    let (_dir, path) = scratch();
    let conn = inspect(&path).await;
    run_pending::<Migrator>(&conn, None).await.unwrap();
    let start = started("caps");
    let run = start.run.id;
    apply(&conn, &[Command::Started(start)]).await;
    for range in 0..120 {
        let events: Vec<_> = (range * 500 + 1..=range * 500 + 500)
            .map(|no| {
                if no <= 50_000 {
                    event(after(run, no))
                } else {
                    event(getter(run, no - 50_000))
                }
            })
            .collect();
        apply(&conn, &events).await;
    }
    let excess = [
        event(after(run, 50_001)),
        event(getter(run, 10_001)),
        event(RunEvent::DeliverySettled {
            run,
            call: 50_002,
            delivery: Delivery::Unknown,
        }),
    ];
    apply(&conn, &excess).await;
    apply(&conn, &excess).await;
    assert_eq!(
        trace_calls::Entity::find().count(&conn).await.unwrap(),
        50_000,
        "call row cap"
    );
    assert_eq!(
        trace_getters::Entity::find().count(&conn).await.unwrap(),
        10_000,
        "getter row cap"
    );
    let row = trace_runs::Entity::find_by_id(run.to_string())
        .one(&conn)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (row.calls_over_cap, row.getters_over_cap),
        (2, 1),
        "absolute excess counts"
    );
    assert_eq!(
        row.over_cap_delivery,
        Some(kinds::Delivery::Unknown),
        "worst excess delivery"
    );
}

#[tokio::test]
async fn final_delivery_before_end_and_page_close_before_open_are_order_tolerant() {
    use neo_runtime::{PageId, PageRef};
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let conn = inspect(&path).await;
    let start = started("order");
    let run = start.run.id;
    apply(
        &conn,
        &[
            event(RunEvent::DeliverySettled {
                run,
                call: 1,
                delivery: Delivery::Done,
            }),
            event(after(run, 1)),
            event(RunEvent::PageClosed {
                run,
                page: PageId::new("p".into()),
                offset_us: 20,
            }),
            event(RunEvent::PageOpened {
                run,
                page: PageRef {
                    target: PageId::new("p".into()),
                    tab_id: Some(7),
                },
                created: true,
                by_call: Some(1),
                offset_us: 10,
            }),
            Command::Started(start),
        ],
    )
    .await;
    let call = store
        .calls(
            &run,
            CallQuery {
                after: None,
                limit: 10,
            },
        )
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(
        call.delivery,
        Some(Delivery::Done),
        "older end respects final delivery guard"
    );
    let page = store.run(&run).await.unwrap().unwrap().pages.remove(0);
    assert_eq!(
        (page.opened_offset_us, page.closed_offset_us, page.by_call),
        (Some(10), Some(20), Some(1)),
        "page facts merge"
    );
    assert_eq!(
        store
            .session(&neo_runtime::SessionKey::new("order"), crate::Timeline::All)
            .await
            .unwrap()
            .unwrap()
            .session
            .runs,
        1,
        "placeholder transfers to labelled session"
    );
}

#[tokio::test]
async fn settle_sweep_times_out_ended_runs_and_marks_them_incomplete() {
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let conn = inspect(&path).await;
    let mut start = started("timeout");
    let run = start.run.id;
    start.started_at_ms = 1000;
    apply(
        &conn,
        &[
            Command::Started(start),
            event(before(run, 1)),
            event(ended(run, 1)),
        ],
    )
    .await;
    apply::sweep(&conn, 122_000).await.unwrap();
    let row = store.run(&run).await.unwrap().unwrap();
    assert_eq!(
        row.summary.settled.unwrap().by,
        crate::SettledBy::Timeout,
        "sweep settles missing last event"
    );
    assert!(
        row.summary.loss.incomplete,
        "unknown final loss counts flagged"
    );
    assert_eq!(
        store
            .calls(
                &run,
                CallQuery {
                    after: None,
                    limit: 10
                }
            )
            .await
            .unwrap()
            .items[0]
            .state,
        CallState::Lost,
        "running call's record was lost"
    );
}

#[tokio::test]
async fn late_start_metadata_is_merged_once_and_a_shed_run_end_is_not_left_running() {
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let conn = inspect(&path).await;
    let start = started("late-hint");
    let run = start.run.id;
    let mut hint = before(run, 1);
    if let RunEvent::BeforeCall {
        selector,
        offset_us,
        ..
    } = &mut hint
    {
        *selector = Some("#hint-only".into());
        *offset_us = 7;
    }
    let commands = [
        Command::Started(start),
        event(after(run, 1)),
        event(hint),
        event(RunEvent::RunSettled { run }),
    ];
    apply(&conn, &commands).await;
    let first = trace_runs::Entity::find_by_id(run.to_string())
        .one(&conn)
        .await
        .unwrap()
        .unwrap();
    apply(&conn, &commands).await;
    let second = trace_runs::Entity::find_by_id(run.to_string())
        .one(&conn)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        first.detail_bytes, second.detail_bytes,
        "late metadata is charged only once"
    );
    let row = store
        .calls(
            &run,
            CallQuery {
                after: None,
                limit: 10,
            },
        )
        .await
        .unwrap()
        .items
        .remove(0);
    assert_eq!(
        (row.selector.as_deref(), row.offset_us, row.state),
        (Some("#hint-only"), 7, CallState::Ok),
        "hint enriches end without regressing it"
    );
    assert_eq!(
        store.run(&run).await.unwrap().unwrap().summary.state,
        crate::RunState::Interrupted,
        "a shed run end cannot remain running after settlement"
    );
}

#[tokio::test]
async fn a_shed_end_and_settle_use_the_known_runtime_budget_as_a_backstop() {
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let conn = inspect(&path).await;
    let mut start = started("shed-end");
    let run = start.run.id;
    let base = writer::now();
    start.started_at_ms = base;
    start.timeout_ms = 1000;
    apply(&conn, &[Command::Started(start), event(before(run, 1))]).await;
    apply::sweep(&conn, base + 120_999).await.unwrap();
    assert!(
        store
            .run(&run)
            .await
            .unwrap()
            .unwrap()
            .summary
            .settled
            .is_none(),
        "runtime budget plus settlement grace retained"
    );
    apply::sweep(&conn, base + 121_001).await.unwrap();
    let row = store.run(&run).await.unwrap().unwrap().summary;
    assert_eq!(
        (row.state, row.settled.unwrap().by),
        (crate::RunState::Interrupted, crate::SettledBy::Timeout),
        "missing lifecycle events get a bounded incomplete end"
    );
    assert!(
        row.loss.incomplete,
        "missing terminal counts remain unknown"
    );
}

#[tokio::test]
async fn every_runtime_getter_name_round_trips_without_a_second_store_enum() {
    use neo_runtime::GetterField;
    use std::time::Duration;
    let (_dir, path) = scratch();
    let store = TraceStore::open(StoreConfig::new(&path)).await.unwrap();
    let conn = inspect(&path).await;
    let start = started("getter-vocabulary");
    let run = start.run.id;
    let fields = [
        GetterField::Url,
        GetterField::IsClosed,
        GetterField::Pages,
        GetterField::Initializer,
        GetterField::Chain,
        GetterField::RedirectedTo,
        GetterField::ExistingResponse,
        GetterField::Timing,
        GetterField::Failure,
        GetterField::Finished,
        GetterField::Frames,
        GetterField::Name,
        GetterField::ParentFrame,
        GetterField::ChildFrames,
        GetterField::IsDetached,
        GetterField::Version,
        GetterField::IsConnected,
        GetterField::PausedDetails,
        GetterField::Debugger,
        GetterField::Opener,
        GetterField::Args,
        GetterField::Element,
        GetterField::MatchesUrl,
        GetterField::Devices,
        GetterField::ParseUrl,
        GetterField::SetUrl,
        GetterField::RandomValues,
    ];
    let mut commands = vec![Command::Started(start)];
    for (index, field) in fields.iter().enumerate() {
        let mut read = getter(run, index as u64 + 1);
        if let RunEvent::Getter { field: saved, .. } = &mut read {
            *saved = *field;
        }
        commands.push(event(read));
    }
    apply(&conn, &commands).await;
    let rows = store.getters(&run).await.unwrap();
    assert_eq!(rows.iter().map(|row| row.field).collect::<Vec<_>>(), fields);
    store.close(Duration::from_secs(10)).await;
}
