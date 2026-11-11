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
    json!({ "call": {}, "page": { "$neo": "page", "tab": 7 }, "script": { "source": source } })
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
        json!({ "call": {}, "script": { "source": "window.c = 1" } }),
    )
    .await?;
    assert_eq!(context, json!({ "id": 1 }));
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
    assert_eq!((first, second), (json!({ "id": 2 }), json!({ "id": 3 })));
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
        json!({ "call": {}, "script": { "source": "window.c = 1" } }),
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
            json!({ "call": {}, "script": { "source": source } }),
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
