use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use browseros_core::testing::TestConnection;
use browseros_core::{BrowserView, TabId};
use neo_trace::ErrorOwner;
use serde_json::{Value, json};
use tokio::time;

use crate::op_scope::OpScope;
use crate::target::CallHeader;
use crate::test_env::{EnvCall, TestEnv};
use crate::{Op, OpEnv, Playwright, Refusal, test_support};

type TestResult = Result<(), Box<dyn Error>>;

const ADD: &str = "Page.addScriptToEvaluateOnNewDocument";
const REMOVE: &str = "Page.removeScriptToEvaluateOnNewDocument";

/// A browser with tab 7 whose session answers every add with identifier `"1"`.
async fn browser(
    connection: &Arc<TestConnection>,
) -> Result<Arc<browseros_core::BrowserSession>, Box<dyn Error>> {
    let test_support::OneTab { browser } = test_support::one_tab(connection).await?;
    connection.always(ADD, json!({ "identifier": "1" }))?;
    connection.always(REMOVE, json!({}))?;
    connection.always(
        "Target.getTargetInfo",
        json!({ "targetInfo": { "targetId": "target-7" } }),
    )?;
    Ok(browser)
}

async fn invoke(
    playwright: &Playwright,
    env: &TestEnv,
    name: &str,
    input: Value,
) -> Result<Value, Box<dyn Error>> {
    let op = Op::from_name(name).ok_or(format!("{name} is an op"))?;
    Ok(playwright.invoke(env, op, input).await?)
}

/// How many times `method` was sent, and with which sources for an add.
fn sent(connection: &TestConnection, method: &str) -> Result<Vec<Value>, Box<dyn Error>> {
    Ok(connection
        .calls()?
        .into_iter()
        .filter(|call| call.method == method)
        .map(|call| call.params)
        .collect())
}

fn page_input(source: &str) -> Value {
    json!({ "call": {}, "page": { "$neo": "page", "tab": 7 }, "source": source })
}

/// A context script reaches a tab at the run's first write to it, once however many writes
/// follow, and never at a read. Disposing a page script removes it at once; the run's end
/// removes everything else, and dropping the run state removes anything registered after that.
#[tokio::test]
async fn tests_that_a_context_script_reaches_a_written_tab_once_and_leaves_with_the_run()
-> TestResult {
    let connection = TestConnection::new([]);
    let browser = browser(&connection).await?;
    let playwright = Playwright::new(Arc::clone(&browser));
    let env = TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None));

    let context = invoke(
        &playwright,
        &env,
        "context.addInitScript",
        json!({ "call": {}, "source": "window.c = 1" }),
    )
    .await?;
    assert_eq!(
        context,
        json!(1),
        "the id, upstream's one result field unwrapped"
    );
    assert!(sent(&connection, ADD)?.is_empty(), "no tab is touched yet");

    invoke(
        &playwright,
        &env,
        "page.opener",
        json!({ "call": {}, "page": { "$neo": "page", "tab": 7 } }),
    )
    .await?;
    assert!(sent(&connection, ADD)?.is_empty(), "a read adds nothing");

    let first = invoke(
        &playwright,
        &env,
        "page.addInitScript",
        page_input("window.p = 1"),
    )
    .await?;
    let second = invoke(
        &playwright,
        &env,
        "page.addInitScript",
        page_input("window.q = 1"),
    )
    .await?;
    assert_eq!((first, second), (json!(2), json!(3)));
    assert_eq!(
        sent(&connection, ADD)?,
        vec![
            json!({ "source": "window.c = 1" }),
            json!({ "source": "window.p = 1" }),
            json!({ "source": "window.q = 1" }),
        ],
        "the context's script first, at the first write's admission, and only once"
    );

    invoke(
        &playwright,
        &env,
        "disposable.dispose",
        json!({ "call": {}, "id": 2 }),
    )
    .await?;
    assert_eq!(
        sent(&connection, REMOVE)?.len(),
        1,
        "dispose removes its own script, before it returns"
    );

    env.run_state().remove_init_scripts().await;
    assert_eq!(
        sent(&connection, REMOVE)?.len(),
        3,
        "the run's end removes the two scripts left, before it returns"
    );

    // A script a straggling op registers after that is removed when the run state drops.
    invoke(
        &playwright,
        &env,
        "page.addInitScript",
        page_input("window.r = 1"),
    )
    .await?;
    drop(env);
    time::timeout(Duration::from_secs(5), async {
        while sent(&connection, REMOVE).map_or(0, |removed| removed.len()) < 4 {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_elapsed| "dropping the run state removes what is left")?;
    Ok(())
}

/// The door's rules on the tab refuse a registration, which adds nothing: the page's own, and
/// the context's at a write's admission.
#[tokio::test]
async fn tests_that_a_refused_tab_gets_no_script() -> TestResult {
    let connection = TestConnection::new([]);
    let browser = browser(&connection).await?;
    let playwright = Playwright::new(Arc::clone(&browser));
    let refusal = Refusal {
        owner: ErrorOwner::Host,
        code: "owner_has_control".to_owned(),
        message: "The owner has control of this tab".to_owned(),
        host_code: None,
    };
    let env = TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None)).with_refusal(refusal);
    let context = invoke(
        &playwright,
        &env,
        "context.addInitScript",
        json!({ "call": {}, "source": "window.c = 1" }),
    )
    .await;
    assert!(context.is_err(), "the run's own recheck refuses it too");
    let op = Op::from_name("page.addInitScript").ok_or("page.addInitScript is an op")?;
    let error = playwright
        .invoke(&env, op, page_input("window.p = 1"))
        .await
        .err()
        .ok_or("a refused tab takes no script")?;
    assert_eq!(
        (error.owner(), error.code()),
        (ErrorOwner::Host, "owner_has_control"),
        "the policy's own refusal: {error}"
    );
    assert!(sent(&connection, ADD)?.is_empty(), "nothing was added");
    assert!(
        !env.calls().contains(&EnvCall::MarkSent),
        "nothing was marked sent"
    );
    Ok(())
}

/// A write that took its snapshot of two context scripts, and adds the second only after a
/// dispose of it returned, adds the first alone: the dispose found the second's cell, and the add
/// sees it gone (Greptile, PR 3348).
#[tokio::test]
async fn tests_that_a_script_disposed_after_a_writes_snapshot_is_not_added() -> TestResult {
    let connection = TestConnection::new([]);
    let browser = browser(&connection).await?;
    let playwright = Playwright::new(Arc::clone(&browser));
    let env = TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None));
    for source in ["window.one = 1", "window.two = 2"] {
        invoke(
            &playwright,
            &env,
            "context.addInitScript",
            json!({ "call": {}, "source": source }),
        )
        .await?;
    }
    // A read's scope admits the tab without adding anything, standing in for a write that
    // took its snapshot and has not reached its adds yet.
    let op = Op::from_name("page.opener").ok_or("page.opener is an op")?;
    let scope = OpScope::new(&env, op, CallHeader::default());
    let admitted = scope.admit(&browser, TabId(7)).await?;
    let scripts = &env.run_state().init_scripts;
    let snapshot = scripts.missing(admitted);
    assert_eq!(snapshot.len(), 2, "both scripts are missing from the tab");

    invoke(
        &playwright,
        &env,
        "disposable.dispose",
        json!({ "call": {}, "id": 2 }),
    )
    .await?;
    scripts
        .add_missing(&scope, &browser, admitted, snapshot)
        .await?;
    assert_eq!(
        sent(&connection, ADD)?,
        vec![json!({ "source": "window.one = 1" })],
        "the disposed script never goes out"
    );
    Ok(())
}

/// How many times the crate told the host an op is about to send main-world script.
fn main_world_reports(env: &TestEnv) -> usize {
    env.calls()
        .iter()
        .filter(|call| **call == EnvCall::MainWorld)
        .count()
}

/// A write that declares no `MainWorld` (`page.requestGC` stands in for a `goto` or a click)
/// reports the main world when its admission adds the context's script to its tab, which then
/// runs there, and only then: not before the context has one, not on a read, and not once the
/// tab has it. `context.addInitScript` itself passes the main-world gateway, so it reports too.
#[tokio::test]
async fn tests_that_a_write_reports_the_main_world_when_it_adds_a_context_script() -> TestResult {
    let connection = TestConnection::new([]);
    let browser = browser(&connection).await?;
    connection.always("HeapProfiler.collectGarbage", json!({}))?;
    let playwright = Playwright::new(Arc::clone(&browser));
    let env = TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None));
    let page = json!({ "call": {}, "page": { "$neo": "page", "tab": 7 } });

    invoke(&playwright, &env, "page.requestGC", page.clone()).await?;
    assert_eq!(main_world_reports(&env), 0, "nothing to add yet");

    invoke(
        &playwright,
        &env,
        "context.addInitScript",
        json!({ "call": {}, "source": "window.c = 1" }),
    )
    .await?;
    assert_eq!(main_world_reports(&env), 1, "the gateway op reports itself");

    invoke(&playwright, &env, "page.opener", page.clone()).await?;
    assert_eq!(main_world_reports(&env), 1, "a read adds nothing");

    invoke(&playwright, &env, "page.requestGC", page.clone()).await?;
    assert_eq!(
        sent(&connection, ADD)?,
        vec![json!({ "source": "window.c = 1" })]
    );
    assert_eq!(
        main_world_reports(&env),
        2,
        "the write that added the context's script reports it"
    );

    invoke(&playwright, &env, "page.requestGC", page).await?;
    assert_eq!(
        main_world_reports(&env),
        2,
        "a tab that has the script gets nothing more"
    );
    Ok(())
}

/// A browser like [`browser`]'s that also answers what a binding's plumbing and its runtime
/// lease send, and the write ops these tests admit.
async fn binding_browser(
    connection: &Arc<TestConnection>,
) -> Result<Arc<browseros_core::BrowserSession>, Box<dyn Error>> {
    let browser = browser(connection).await?;
    for method in [
        "Runtime.addBinding",
        "Runtime.disable",
        "HeapProfiler.collectGarbage",
    ] {
        connection.always(method, json!({}))?;
    }
    Ok(browser)
}

/// A write admitted on a tab where another live run holds a context binding's name adds nothing
/// there, so it reports no main world, though its recheck passed. A third run's binding of
/// another name, on the same tab, is added at its write and reported.
#[tokio::test]
async fn tests_that_a_context_binding_another_run_holds_reports_no_main_world() -> TestResult {
    let connection = TestConnection::new([]);
    let browser = binding_browser(&connection).await?;
    let playwright = Playwright::new(Arc::clone(&browser));
    let run = || TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None));
    let page = json!({ "$neo": "page", "tab": 7 });
    let write = json!({ "call": {}, "page": page });

    let holder = run();
    invoke(
        &playwright,
        &holder,
        "page.exposeBinding",
        json!({ "call": {}, "page": page, "name": "f" }),
    )
    .await?;

    let blocked = run();
    invoke(
        &playwright,
        &blocked,
        "context.exposeBinding",
        json!({ "call": {}, "name": "f" }),
    )
    .await?;
    let adds = sent(&connection, ADD)?.len();
    let before = main_world_reports(&blocked);
    invoke(&playwright, &blocked, "page.requestGC", write.clone()).await?;
    assert_eq!(
        sent(&connection, ADD)?.len(),
        adds,
        "the holder's binding stays on the tab, and nothing is added"
    );
    assert_eq!(
        main_world_reports(&blocked),
        before,
        "a write that added nothing reports no main world"
    );

    let other = run();
    invoke(
        &playwright,
        &other,
        "context.exposeBinding",
        json!({ "call": {}, "name": "g" }),
    )
    .await?;
    let before = main_world_reports(&other);
    invoke(&playwright, &other, "page.requestGC", write).await?;
    assert!(
        main_world_reports(&other) > before,
        "a write that added its run's binding reports it"
    );
    Ok(())
}

/// A binding's dispose runs its cleanup (`removeBinding`) in the main world of the documents
/// loaded now, so it reports the main world. An init script's dispose only unregisters it and
/// runs nothing in the page, so it reports nothing.
#[tokio::test]
async fn tests_that_disposing_a_binding_reports_the_main_world() -> TestResult {
    let connection = TestConnection::new([]);
    let browser = binding_browser(&connection).await?;
    let playwright = Playwright::new(Arc::clone(&browser));
    let env = TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None));
    let script = invoke(
        &playwright,
        &env,
        "page.addInitScript",
        page_input("window.p = 1"),
    )
    .await?;
    let binding = invoke(
        &playwright,
        &env,
        "page.exposeBinding",
        json!({ "call": {}, "page": { "$neo": "page", "tab": 7 }, "name": "f" }),
    )
    .await?;

    let before = main_world_reports(&env);
    invoke(
        &playwright,
        &env,
        "disposable.dispose",
        json!({ "call": {}, "id": script }),
    )
    .await?;
    assert_eq!(
        main_world_reports(&env),
        before,
        "an init script's dispose runs nothing in the page"
    );

    invoke(
        &playwright,
        &env,
        "disposable.dispose",
        json!({ "call": {}, "id": binding }),
    )
    .await?;
    let cleanup = sent(&connection, ADD)?
        .into_iter()
        .last()
        .ok_or("the dispose added the cleanup")?;
    assert_eq!(cleanup["runImmediately"], json!(true));
    assert!(
        cleanup["source"]
            .as_str()
            .is_some_and(|source| source.contains("removeBinding")),
        "the cleanup removes the binding from the loaded documents: {cleanup}"
    );
    assert!(
        main_world_reports(&env) > before,
        "the binding's dispose reports the main world its cleanup ran in"
    );
    Ok(())
}

/// A binding's dispose whose script removal never answers is cut off before its cleanup goes
/// out, so it reports no main world: the report comes right before the cleanup's send.
#[tokio::test]
async fn tests_that_a_dispose_stalled_before_its_cleanup_reports_nothing() -> TestResult {
    let connection = TestConnection::new([]);
    let browser = binding_browser(&connection).await?;
    let playwright = Playwright::new(Arc::clone(&browser));
    let env = TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None));
    let binding = invoke(
        &playwright,
        &env,
        "page.exposeBinding",
        json!({ "call": {}, "page": { "$neo": "page", "tab": 7 }, "name": "f" }),
    )
    .await?;
    connection.always(REMOVE, browseros_core::testing::cdp_pending())?;
    let adds_before = sent(&connection, ADD)?.len();
    let reports_before = main_world_reports(&env);

    let dispose = invoke(
        &playwright,
        &env,
        "disposable.dispose",
        json!({ "call": {}, "id": binding }),
    );
    assert!(
        time::timeout(Duration::from_millis(200), dispose)
            .await
            .is_err(),
        "the removal never answers, so the dispose is cut off"
    );
    assert_eq!(
        sent(&connection, ADD)?.len(),
        adds_before,
        "no cleanup went out"
    );
    assert_eq!(
        main_world_reports(&env),
        reports_before,
        "a cleanup that never went out reports no main world"
    );
    Ok(())
}
