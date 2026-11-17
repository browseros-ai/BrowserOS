//! Read-only admission through the real builtins and door: no lazy opens or browser sends on
//! refusal, URL completion and owner protection, and the run-local close permission.

use std::error::Error;
use std::sync::Arc;

use neo_trace::{Delivery, ErrorOwner, RunAccess, RunEffects, RunId, TaskId};
use serde_json::{Value, json};
use tokio::task;
use tokio::time::Instant;

use super::{Harness, failure, goto_input, handle, page_of, value, within};
use crate::browser::{PageRef, TabsScope};
use crate::config::Defaults;
use crate::dispatch::{ErrorCode, RunCtx, RunGuard, RunStart};

fn read_only(
    harness: &Harness,
    task: Option<&TaskId>,
    scope: TabsScope,
) -> Result<(Arc<RunCtx>, RunGuard), Box<dyn Error>> {
    Ok(RunCtx::start(
        &harness.dispatcher,
        RunStart {
            id: RunId::new(),
            code: String::new(),
            request_id: None,
            task_id: task.cloned(),
            host_data: None,
            view: harness.link.view().ok_or("linked browser")?,
            deadline: Instant::now() + Defaults::RUN_TIMEOUT,
            default_page: None,
            tabs_scope: scope,
            access: Some(RunAccess::Read),
        },
    ))
}

async fn refused(
    harness: &Harness,
    run: &Arc<RunCtx>,
    name: &str,
    input: Value,
) -> Result<(), Box<dyn Error>> {
    let (operation, reply) = harness.submit_op(run, name, input).await;
    let error = failure(reply)?;
    assert_eq!(
        (error.error.owner(), error.error.code(), error.delivery),
        (
            ErrorOwner::Script,
            ErrorCode::READ_ONLY_RUN,
            Delivery::NotSent
        ),
        "{name}: {error:?}"
    );
    let row = harness.row(operation).await?;
    assert_eq!(row.delivery, Some(Delivery::NotSent));
    assert_eq!(
        row.tab, None,
        "refused access creates no tab trace link: {name}"
    );
    assert_eq!(row.page.tab_id, None);
    Ok(())
}

#[tokio::test]
async fn read_only_writes_and_chooser_interception_are_refused_before_even_a_lazy_open()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let (run, _guard) = read_only(&harness, None, TabsScope::Browser)?;
    let lazy = json!({"$neo":"page", "tab":null});
    // These are real operation names, with decoding deferred to their leaves. The access door
    // must win before target resolution, filesystem work, or a native chooser lease.
    for name in [
        "locator.click",
        "locator.fill",
        "locator.pressSequentially",
        "locator.press",
        "locator.hover",
        "locator.focus",
        "mouse.move",
        "mouse.click",
        "mouse.wheel",
        "keyboard.type",
        "keyboard.press",
        "page.evaluate",
        "locator.evaluate",
        "locator.evaluateAll",
        "page.addInitScript",
        "context.addInitScript",
        "page.setContent",
        "page.exposeBinding",
        "context.exposeBinding",
        "page.waitForFunction",
        "page.reload",
        "page.goBack",
        "page.goForward",
        "locator.setInputFiles",
        "dialog.accept",
        "dialog.dismiss",
        "download.saveAs",
        "page.saveScreenshot",
        "webStorage.setItem",
        "webStorage.removeItem",
        "webStorage.clear",
        "page.emulateMedia",
        "page.setViewportSize",
        "neo.solveCaptcha",
        "events.interceptFileChooser",
    ] {
        refused(&harness, &run, name, json!({"page":lazy})).await?;
    }
    refused(&harness, &run, "task.close", json!({"task":"not-opened"})).await?;
    assert!(
        harness.fake.tabs().is_empty(),
        "a forbidden call must not open its lazy page"
    );
    for method in [
        "Browser.createTab",
        "Page.navigate",
        "Runtime.evaluate",
        "Runtime.callFunctionOn",
        "Input.dispatchMouseEvent",
        "Input.dispatchKeyEvent",
        "Page.setInterceptFileChooserDialog",
    ] {
        assert_eq!(
            harness.fake.count(method),
            0,
            "no refused family sends {method}"
        );
    }
    assert_eq!(run.report().effects, RunEffects::None);
    let row = harness
        .store
        .run_summary(run.id())
        .await?
        .ok_or("recorded run")?;
    assert!(row.tabs.is_empty());
    Ok(())
}

#[tokio::test]
async fn read_only_goto_checks_completed_schemes_and_preserves_sent_write_effects()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let page = harness.owner_tab("https://agent.example/").await?;
    harness.tabs.opened_by_agent(&page, harness.tabs.seen());
    let (run, _guard) = read_only(&harness, None, TabsScope::Browser)?;
    for url in [
        "data:text/html,<p>no</p>",
        "about:blank",
        "javascript:void(0)",
        "file:///tmp/no.html",
        "blob:https://agent.example/id",
        "chrome://version",
        "ftp://agent.example/",
        "ws://agent.example/",
        "view-source:https://agent.example/",
        "agent.example/",
        " JavaScript:void(0)",
        "JavaScript:void(0)",
        " \u{1}java\tscript:void(0)",
        "java\tscript:void(0)",
        "\0\rfi\nle:///tmp/no.html",
        "chrome\r://version",
        "LOCALHOST:80",
        " LOCALHOST:3000/",
        "local\thost:3000/",
    ] {
        refused(&harness, &run, "page.goto", goto_input(&page, url)).await?;
    }
    assert_eq!(harness.fake.count("Page.navigate"), 0);
    harness.fake.answer(
        "Page.navigate",
        json!({"frameId":"frame", "errorText":"net::ERR_ABORTED"}),
    );
    let urls = [
        "https://agent.example/",
        " HTTP://agent.example/ ",
        "h\tt\nt\rps://agent.example/",
        "localhost:8080",
        "127.0.0.1:3000/",
        "localhost.example:3000/",
    ];
    for url in urls {
        let failure = failure(
            harness
                .submit(&run, "page.goto", goto_input(&page, url))
                .await,
        )?;
        assert_eq!(
            (failure.error.code(), failure.delivery),
            (ErrorCode::NAVIGATION_FAILED, Delivery::Done),
            "{url:?}: {failure:?}"
        );
    }
    assert_eq!(harness.fake.count("Page.navigate"), urls.len());
    let sent: Vec<_> = harness
        .fake
        .calls()
        .into_iter()
        .filter(|call| call.method == "Page.navigate")
        .map(|call| call.params["url"].clone())
        .collect();
    assert_eq!(
        sent,
        [
            "https://agent.example/",
            "http://agent.example/",
            "https://agent.example/",
            "http://localhost:8080",
            "http://127.0.0.1:3000/",
            "http://localhost.example:3000/"
        ]
        .map(Value::from)
    );
    assert_eq!(
        run.report().effects,
        RunEffects::Done,
        "access never relabels an allowed write"
    );
    Ok(())
}

#[tokio::test]
async fn read_only_goto_protects_owner_tabs_but_allows_found_tabs_held_after_restart()
-> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let owner = harness.owner_tab("https://owner.example/").await?;
    let held = harness.owner_tab("https://held.example/").await?;
    let task = harness.tabs.create_task(None)?;
    harness.tabs.touched(&held, harness.tabs.seen());
    harness.tabs.own(&held, &task.id)?;
    assert!(
        !harness.tabs.agent_tabs().contains(&held.tab),
        "a restored task tab still has Found origin"
    );
    let (run, _guard) = read_only(&harness, None, TabsScope::Browser)?;
    refused(
        &harness,
        &run,
        "page.goto",
        goto_input(&owner, "https://next.example/"),
    )
    .await?;
    assert_eq!(harness.fake.count("Page.navigate"), 0);
    // Reading an owner's tab is still allowed when task scope allows reaching it.
    value(
        harness
            .submit(&run, "page.opener", json!({"page":handle(&owner)}))
            .await,
    )?;
    harness.fake.answer(
        "Page.navigate",
        json!({"frameId":"frame", "errorText":"net::ERR_ABORTED"}),
    );
    let sent = failure(
        harness
            .submit(
                &run,
                "page.goto",
                goto_input(&held, "https://next.example/"),
            )
            .await,
    )?;
    assert_eq!(sent.delivery, Delivery::Done);
    assert_ne!(sent.error.code(), ErrorCode::READ_ONLY_RUN);
    assert_eq!(harness.fake.count("Page.navigate"), 1);
    Ok(())
}

#[tokio::test]
async fn read_only_close_accepts_own_explicit_and_lazy_opens_only() -> Result<(), Box<dyn Error>> {
    let harness = Harness::new().await?;
    let task = TaskId::parse("reading")?;
    let (run, _guard) = read_only(&harness, Some(&task), TabsScope::Task)?;
    let lazy = json!({"$neo":"page", "tab":null});
    refused(&harness, &run, "page.close", json!({"page":lazy})).await?;
    assert!(
        harness.fake.tabs().is_empty(),
        "refusing an unresolved close must not open a tab"
    );
    let own = page_of(&value(
        harness.submit(&run, "context.newPage", json!({})).await,
    )?)?;
    assert_eq!(
        harness.tabs.owner(own.tab),
        Some(task.clone()),
        "newPage joins the run's task"
    );
    let named = page_of(&value(
        harness
            .submit(&run, "task.newPage", json!({"task":task}))
            .await,
    )?)?;
    let popup = harness.fake.open_popup(own.tab, "https://popup.example/");
    within(async {
        while harness.tabs.owner(popup).is_none() {
            task::yield_now().await;
        }
    })
    .await?;
    refused(
        &harness,
        &run,
        "page.close",
        json!({"page":handle(&PageRef{tab:popup})}),
    )
    .await?;
    let (later, _later_guard) = read_only(&harness, Some(&task), TabsScope::Task)?;
    refused(&harness, &later, "page.close", json!({"page":handle(&own)})).await?;
    for page in [&own, &named] {
        value(
            harness
                .submit(&run, "page.close", json!({"page":handle(page)}))
                .await,
        )?;
        assert!(!harness.is_open(page.tab).await?);
    }
    assert!(harness.is_open(popup).await?);
    // An unbound read-only run uses the very same open ledger, rather than gaining close
    // permission only when task scope happens to be enabled.
    let (lazy_run, _lazy_guard) = read_only(&harness, None, TabsScope::Browser)?;
    let reply = harness
        .submit(&lazy_run, "page.opener", json!({"page":lazy}))
        .await;
    let opened = reply.default_page.clone().ok_or("lazy page resolved")?;
    value(reply)?;
    value(
        harness
            .submit(&lazy_run, "page.close", json!({"page":handle(&opened)}))
            .await,
    )?;
    assert!(!harness.is_open(opened.tab).await?);
    Ok(())
}

#[tokio::test]
async fn read_only_goto_accepts_its_new_tab_before_readiness_updates_registry_origin()
-> Result<(), Box<dyn Error>> {
    use crate::dispatch::Call;

    let harness = Harness::new().await?;
    let host = TaskId::parse("early-open")?;
    let (run, _guard) = read_only(&harness, Some(&host), TabsScope::Task)?;
    harness.fake.hold_next("Page.setLifecycleEventsEnabled");
    let opening = harness.dispatcher.submit(
        &run,
        Call::script("context.newPage".to_owned(), json!({}), None),
    );
    within(async {
        while harness.fake.count("Page.setLifecycleEventsEnabled") == 0 {
            task::yield_now().await;
        }
    })
    .await?;
    let listed = value(harness.submit(&run, "context.pages", json!({})).await)?;
    let tab = listed
        .get("pages")
        .and_then(Value::as_array)
        .and_then(|pages| pages.first())
        .and_then(|page| page.get("page"))
        .and_then(|page| page.get("tab"))
        .and_then(Value::as_i64)
        .ok_or("the new tab is already listed")?;
    let early = PageRef {
        tab: browseros_core::TabId(tab),
    };
    assert!(
        harness.tabs.adoptable(early.tab),
        "registry origin/holder are still pending readiness"
    );
    harness.fake.answer(
        "Page.navigate",
        json!({"frameId":"frame", "errorText":"net::ERR_ABORTED"}),
    );
    let navigating = harness.dispatcher.submit(
        &run,
        Call::script(
            "page.goto".to_owned(),
            goto_input(&early, "https://next.example/"),
            None,
        ),
    );
    harness.fake.release_held();
    value(within(opening.wait()).await?)?;
    let navigation = failure(within(navigating.wait()).await?)?;
    assert_eq!(
        (navigation.error.code(), navigation.delivery),
        (ErrorCode::NAVIGATION_FAILED, Delivery::Done),
        "the run's open ledger proves this is an agent tab before registry bookkeeping catches up"
    );
    Ok(())
}

#[tokio::test]
async fn read_only_can_close_its_physical_open_when_task_assignment_loses_a_race()
-> Result<(), Box<dyn Error>> {
    use crate::dispatch::Call;

    let harness = Harness::new().await?;
    let (run, _guard) = read_only(&harness, None, TabsScope::Task)?;
    let created = super::created_in(&harness, &run).await?;
    harness.fake.hold_next("Browser.createTab");
    let opening = harness.dispatcher.submit(
        &run,
        Call::script("task.newPage".to_owned(), json!({"task":created}), None),
    );
    within(async {
        while harness.fake.count("Browser.createTab") == 0 {
            task::yield_now().await;
        }
    })
    .await?;
    harness.tabs.ensure_task(&created)?;
    harness.fake.release_held();
    assert_eq!(
        failure(within(opening.wait()).await?)?.error.code(),
        ErrorCode::TASK_NOT_FOUND
    );
    let tab = *harness
        .fake
        .tabs()
        .first()
        .ok_or("the browser physically opened a tab")?;
    assert_eq!(harness.tabs.owner(tab), None, "task assignment failed");
    value(
        harness
            .submit(&run, "page.close", json!({"page":handle(&PageRef{tab})}))
            .await,
    )?;
    assert!(
        !harness.is_open(tab).await?,
        "physical creation grants same-run cleanup even when task assignment failed"
    );
    Ok(())
}
