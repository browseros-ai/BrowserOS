//! Hosted lifecycle through the production stack and the existing fake CDP seam.

use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;
use std::{fs, future};

use neo_trace::{RunId, RunStatus, TraceStore};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::time;

use super::hosted_stack::HostedStack;
use super::{HostedOptions, HostedRuntime};
use crate::browser::BrowserLink;
use crate::config::HostedMode;
use crate::test_rig::ManualClock;
use crate::test_support::FakeCdp;

async fn connect(dir: &TempDir, fake: Arc<FakeCdp>) -> Result<HostedRuntime, Box<dyn Error>> {
    Ok(HostedRuntime::start(
        HostedOptions {
            state_dir: dir.path().join("state"),
            read_dirs: None,
            execution_dir: Some(dir.path().join("profile/.browseros")),
            browseros_version: Some("0.52.1.1".to_owned()),
            hosted_mode: false,
        },
        move |native, tabs| {
            BrowserLink::start_with(native, tabs, move || future::ready(fake.connect()))
        },
    )
    .await?)
}

/// Uses the production stack on the caller's clock; other lifecycle tests retain the owned
/// executor. These tests must freeze the run deadline while observing an unpolled call.
async fn connect_stack(
    dir: &TempDir,
    fake: Arc<FakeCdp>,
) -> Result<Arc<HostedStack>, Box<dyn Error>> {
    Ok(HostedStack::start(
        HostedOptions {
            state_dir: dir.path().join("state"),
            read_dirs: None,
            execution_dir: Some(dir.path().join("profile/.browseros")),
            browseros_version: Some("0.52.1.1".to_owned()),
            hosted_mode: false,
        },
        move |native, tabs| {
            BrowserLink::start_with(native, tabs, move || future::ready(fake.connect()))
        },
    )
    .await?)
}

#[tokio::test]
async fn open_invoke_finish_records_the_real_run() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let fake = FakeCdp::new();
    fake.open_tab("about:blank", "Launch tab");
    let runtime = connect(&dir, fake).await?;
    let run = runtime.open_run(Duration::from_secs(10)).await?;
    let init: Value = serde_json::from_str(run.init())?;
    assert!(
        init["page"].is_null(),
        "no previously touched tab, so the default page is lazy"
    );
    assert_eq!(
        init["pages"].as_array().map(Vec::len),
        Some(1),
        "init includes the owner's launch tab"
    );
    assert!(
        init["observed"]["seq"].is_u64(),
        "init carries the observation that orders its listing against later replies: {init}"
    );
    let opened: Value = serde_json::from_str(
        &run.invoke("context.newPage".to_owned(), "{}", "run:1:1")
            .await,
    )?;
    let page = opened["ok"]["page"].clone();
    assert_eq!(page["$neo"], "page", "the product returns a page brand");
    assert!(
        page["tab"].is_u64() && page.get("id").is_none(),
        "a page brand names its tab only: {page}"
    );
    let input = json!({ "page": page }).to_string();
    let closed: Value =
        serde_json::from_str(&run.invoke("page.close".to_owned(), &input, "").await)?;
    assert!(
        closed.get("ok").is_some(),
        "close uses the real registered method: {closed}"
    );
    run.log("info", "hosted log".to_owned());
    let reply: Value = serde_json::from_str(&run.finish().await?)?;
    assert_eq!(reply["status"], "ok");
    assert_eq!(reply["id"], run.id());
    runtime.close().await?;
    let store = TraceStore::open(&dir.path().join("state/claw-sdk/trace.db")).await?;
    let record = store
        .run(RunId::from_str(&run.id())?)
        .await?
        .ok_or("missing run record")?;
    assert_eq!(
        record.summary.status,
        RunStatus::Ok,
        "finish flushes a completed run"
    );
    Ok(())
}

#[tokio::test]
async fn concurrent_finish_and_close_are_idempotent() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let runtime = connect(&dir, FakeCdp::new()).await?;
    let run = runtime.open_run(Duration::from_secs(10)).await?;
    let (first, second) = tokio::join!(run.finish(), run.finish());
    assert_eq!(first?, second?, "every waiter receives the same end");
    let (first, second) = tokio::join!(runtime.close(), runtime.close());
    first?;
    second?;
    assert!(
        runtime.open_run(Duration::from_secs(1)).await.is_err(),
        "closed hosts admit no runs"
    );
    Ok(())
}

#[tokio::test]
async fn deadline_stops_a_pending_call_and_sleep() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let fake = FakeCdp::new();
    fake.hang("Browser.createTab");
    let runtime = connect(&dir, fake).await?;
    let run = runtime.open_run(Duration::from_millis(100)).await?;
    let call = run.invoke("context.newPage".to_owned(), "{}", "");
    let (envelope, ()) = time::timeout(Duration::from_secs(5), async {
        tokio::join!(call, run.sleep(60_000.0))
    })
    .await?;
    let envelope: Value = serde_json::from_str(&envelope)?;
    assert_eq!(envelope["err"]["code"], "run_timeout");
    let reply: Value = serde_json::from_str(&run.finish().await?)?;
    assert_eq!(
        reply["status"], "timeout",
        "finish cannot overwrite the deadline's cause"
    );
    runtime.close().await?;
    Ok(())
}

#[tokio::test]
async fn admission_precedes_poll_and_finish_drains_a_call_in_flight() -> Result<(), Box<dyn Error>>
{
    let dir = tempfile::tempdir()?;
    let fake = FakeCdp::new();
    fake.hang("Browser.createTab");
    let stack = connect_stack(&dir, Arc::clone(&fake)).await?;
    let clock = ManualClock::pause();
    let run = stack.open_run(Duration::from_secs(10)).await?;
    // Do not poll the returned future: the bridge must have already admitted and spawned it.
    let mut call = Box::pin(run.invoke("context.newPage".to_owned(), "{}", "run:1:1"));
    fake.received("Browser.createTab").await?;
    let reply: Value = serde_json::from_str(&run.finish().await?)?;
    let Poll::Ready(answer) = futures_util::poll!(call.as_mut()) else {
        return Err("finish returned before the call's answer settled".into());
    };
    let envelope: Value = serde_json::from_str(&answer)?;
    assert_eq!(envelope["err"]["code"], "not_awaited");
    assert_eq!(
        reply["notAwaited"].as_array().map(Vec::len),
        Some(1),
        "the pending call is accounted for before the end"
    );
    drop(clock);
    stack.close().await?;
    Ok(())
}

#[tokio::test]
async fn dropping_the_run_cancels_its_pending_call() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let fake = FakeCdp::new();
    fake.hang("Browser.createTab");
    let stack = connect_stack(&dir, Arc::clone(&fake)).await?;
    let clock = ManualClock::pause();
    let run = stack.open_run(Duration::from_secs(10)).await?;
    let call = run.invoke("context.newPage".to_owned(), "{}", "");
    fake.received("Browser.createTab").await?;
    drop(run);
    let envelope: Value = serde_json::from_str(&call.await)?;
    assert_eq!(envelope["err"]["code"], "cancelled");
    drop(clock);
    stack.close().await?;
    Ok(())
}

/// A host states the browser's launch mode: the spec runner launches BrowserOS without hosted
/// mode unless asked, and then Neo must wait for console and page errors as it does there.
#[test]
fn tests_that_hosted_mode_follows_the_hosts_launch_fact() -> Result<(), Box<dyn Error>> {
    let dir = TempDir::new()?;
    let options = |hosted_mode| HostedOptions {
        state_dir: dir.path().join("state"),
        hosted_mode,
        ..HostedOptions::default()
    };
    assert_eq!(
        options(false).config()?.env.hosted_mode,
        HostedMode::Off { value: None }
    );
    assert_eq!(options(true).config()?.env.hosted_mode, HostedMode::On);
    Ok(())
}

#[tokio::test]
async fn hosted_runs_start_unowned_and_attach_tabs_only_after_picking_a_task()
-> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let runtime = connect(&dir, FakeCdp::new()).await?;
    let first = runtime.open_run(Duration::from_secs(10)).await?;
    let opened: Value =
        serde_json::from_str(&first.invoke("context.newPage".to_owned(), "{}", "").await)?;
    assert_eq!(opened["pages"][0]["taskId"], Value::Null);
    let first_reply: Value = serde_json::from_str(&first.finish().await?)?;
    assert_eq!(first_reply["tasks"], json!([]));
    let second = runtime.open_run(Duration::from_secs(10)).await?;
    let init: Value = serde_json::from_str(second.init())?;
    assert_eq!(
        init["page"],
        Value::Null,
        "a new hosted run cannot inherit the first one's default"
    );
    assert_eq!(init["pages"][0]["taskId"], Value::Null);
    let picked: Value =
        serde_json::from_str(&second.invoke("neo.createTask".to_owned(), "{}", "").await)?;
    let owned: Value =
        serde_json::from_str(&second.invoke("context.newPage".to_owned(), "{}", "").await)?;
    assert_ne!(opened["ok"]["page"], owned["ok"]["page"]);
    assert_eq!(owned["pages"][0]["taskId"], picked["ok"]["id"]);
    let second_reply: Value = serde_json::from_str(&second.finish().await?)?;
    assert_eq!(second_reply["tasks"][0]["kind"], "created");
    assert_eq!(
        second_reply["tasks"][0]["tabs"][0]["tabId"],
        owned["ok"]["page"]["tab"]
    );
    runtime.close().await?;
    Ok(())
}

#[tokio::test]
async fn hosted_file_reads_use_default_explicit_and_empty_lists_and_protect_state()
-> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let fixture = dir.path().join("fixture.js");
    fs::write(&fixture, "globalThis.fixture = 1;")?;
    for (name, read_dirs, allowed) in [
        ("default", None, false),
        ("explicit", Some(vec![dir.path().to_owned()]), true),
        ("empty", Some(Vec::new()), false),
    ] {
        let fake = FakeCdp::new();
        let state = dir.path().join(name);
        let runtime = HostedRuntime::start(
            HostedOptions {
                state_dir: state.clone(),
                read_dirs,
                ..HostedOptions::default()
            },
            move |native, tabs| {
                BrowserLink::start_with(native, tabs, move || future::ready(fake.connect()))
            },
        )
        .await?;
        let run = runtime.open_run(Duration::from_secs(10)).await?;
        let answer: Value = serde_json::from_str(
            &run.invoke(
                "context.addInitScript".to_owned(),
                &json!({"path": fixture}).to_string(),
                "",
            )
            .await,
        )?;
        if allowed {
            assert!(answer.get("ok").is_some(), "{name}: {answer}");
        } else {
            assert_eq!(answer["err"]["code"], "invalid_input", "{name}: {answer}");
            assert_eq!(answer["err"]["delivery"], "not_sent");
            assert_eq!(answer["err"]["owner"], "script");
        }
        let protected = state.join("nested/new.png");
        let answer: Value = serde_json::from_str(
            &run.invoke(
                "page.saveScreenshot".to_owned(),
                &json!({"path": protected, "base64": "cGljdHVyZQ=="}).to_string(),
                "",
            )
            .await,
        )?;
        assert_eq!(answer["err"]["code"], "invalid_input", "{answer}");
        assert_eq!(answer["err"]["delivery"], "not_sent");
        assert!(!protected.exists());
        run.finish().await?;
        runtime.close().await?;
    }
    let fake = FakeCdp::new();
    let invalid = HostedRuntime::start(
        HostedOptions {
            state_dir: dir.path().join("invalid"),
            read_dirs: Some(vec!["relative".into()]),
            ..HostedOptions::default()
        },
        move |native, tabs| {
            BrowserLink::start_with(native, tabs, move || future::ready(fake.connect()))
        },
    )
    .await;
    assert!(
        invalid.is_err(),
        "hosted options use the host settings validation"
    );
    Ok(())
}
