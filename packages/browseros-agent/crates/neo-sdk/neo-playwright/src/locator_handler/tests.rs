use std::error::Error;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{task, time};

use super::{AddInput, LocatorHandlers, ResolveInput};
use crate::op::Op;
use crate::op_input::OpInput;
use crate::op_scope::OpScope;
use crate::target::LocatorTarget;
use crate::test_env::{self, TestEnv};
use crate::{ErrorCode, Playwright};

/// The JS oracle records reply order: a pending pull can finish before remove's reply, even
/// though remove already unregistered it. Reproduce that registry state before each pull.
#[tokio::test]
async fn tests_that_handler_routes_keep_the_recorded_input_and_reply_bytes()
-> Result<(), Box<dyn Error>> {
    #[derive(Deserialize)]
    struct Fixture {
        scenarios: Vec<Scenario>,
    }
    #[derive(Deserialize)]
    struct Scenario {
        exchanges: Vec<Exchange>,
    }
    #[derive(Deserialize)]
    struct Exchange {
        op: String,
        args: String,
        reply: String,
    }
    let fixture: Fixture =
        serde_json::from_str(include_str!("../../tests/data/locator-handler-wire.json"))?;
    for scenario in fixture.scenarios {
        let env = TestEnv::new();
        let pw = Playwright::new(test_env::offline_session());
        for exchange in scenario.exchanges {
            let op = Op::from_name(&exchange.op).ok_or("recorded op is registered")?;
            let OpInput { call, args } =
                OpInput::decode(op, serde_json::from_str(&exchange.args)?)?;
            let scope = OpScope::new(&env, op, call);
            let expected: Value = serde_json::from_str(&exchange.reply)?;
            let expected = &expected["ok"];
            let handlers = &scope.run_state().locator_handlers;
            if exchange.op == "page.locatorHandlerTriggered" {
                if let Some(uid) = expected["uid"].as_u64() {
                    let _running = handlers.trigger(uid).ok_or("registered handler")?;
                } else {
                    handlers.unregister(1);
                }
            }
            let actual = super::invoke(
                &pw,
                &scope,
                match op.kind() {
                    crate::op::OpKind::LocatorHandler(op) => op,
                    _ => return Err("fixture must contain only handler ops".into()),
                },
                args,
            )
            .await?;
            assert_eq!(
                serde_json::to_string(&actual)?,
                serde_json::to_string(expected)?
            );
        }
    }
    Ok(())
}

#[test]
fn tests_that_defaulted_flags_keep_rejecting_explicit_null() {
    let target = json!({
        "$neo": "locator", "page": { "$neo": "page", "tab": 1 },
        "chunks": [{ "parts": [{ "name": "css", "body": "#overlay", "source": "#overlay" }] }],
        "display": "locator('#overlay')"
    });
    assert!(serde_json::from_value::<AddInput>(json!({ "target": target })).is_ok());
    assert!(
        serde_json::from_value::<AddInput>(json!({
            "target": target, "noWaitAfter": null
        }))
        .is_err()
    );
    assert!(serde_json::from_value::<ResolveInput>(json!({ "uid": 1 })).is_ok());
    assert!(serde_json::from_value::<ResolveInput>(json!({ "uid": 1, "remove": null })).is_err());
}

#[tokio::test]
async fn tests_that_add_keeps_its_element_handle_refusal() -> Result<(), Box<dyn Error>> {
    let env = TestEnv::new();
    let pw = Playwright::new(test_env::offline_session());
    let op = Op::from_name("page.addLocatorHandler").ok_or("registered")?;
    let OpInput { call, args } = OpInput::decode(
        op,
        json!({
            "target": { "$neo": "element", "page": { "$neo": "page", "tab": 1 },
                "seq": 1, "node": 1, "display": "<div>" }
        }),
    )?;
    let scope = OpScope::new(&env, op, call);
    let error = super::invoke(&pw, &scope, super::LocatorHandlerOp::Add, args)
        .await
        .err()
        .ok_or("handles must fail")?;
    assert_eq!(error.error_code(), Some(ErrorCode::InvalidInput));
    assert_eq!(
        error.message(),
        "page.addLocatorHandler: target: a locator handler takes a locator, not an element handle"
    );
    Ok(())
}

/// `page.locator('#overlay')` on tab `tab`, as the prelude sends it.
fn overlay(tab: i64) -> Result<LocatorTarget, serde_json::Error> {
    serde_json::from_value(json!({
        "page": { "$neo": "page", "tab": tab },
        "chunks": [{ "parts": [{ "name": "css", "body": "#overlay", "source": "#overlay" }] }],
        "display": "locator('#overlay')",
    }))
}

fn uids(handlers: &LocatorHandlers, tab: i64) -> Option<Vec<u64>> {
    let tab = overlay(tab).ok()?.page().tab();
    handlers
        .ready_on(tab)
        .map(|ready| ready.iter().map(|handler| handler.uid).collect())
}

#[tokio::test]
async fn tests_that_the_pull_answers_each_trigger_once_and_none_once_no_handler_is_left()
-> Result<(), Box<dyn Error>> {
    let handlers = LocatorHandlers::default();
    assert_eq!(
        handlers.next_trigger().await,
        None,
        "no handler: the pull ends"
    );
    let uid = handlers.register(overlay(1)?, false);
    let pulled = time::timeout(Duration::from_millis(20), handlers.next_trigger()).await;
    assert!(pulled.is_err(), "a handler and no trigger: the pull waits");
    assert!(handlers.trigger(uid).is_some());
    assert!(
        handlers.trigger(uid).is_some(),
        "a second checkpoint joins it"
    );
    assert_eq!(handlers.next_trigger().await, Some(uid));
    let pulled = time::timeout(Duration::from_millis(20), handlers.next_trigger()).await;
    assert!(pulled.is_err(), "a running handler is queued once");
    handlers.unregister(uid);
    assert_eq!(handlers.next_trigger().await, None);
    Ok(())
}

#[tokio::test]
async fn tests_that_removing_the_last_handler_wakes_a_waiting_pull() -> Result<(), Box<dyn Error>> {
    let handlers = LocatorHandlers::default();
    let uid = handlers.register(overlay(1)?, false);
    let (pulled, ()) = tokio::join!(handlers.next_trigger(), async {
        task::yield_now().await;
        handlers.unregister(uid);
    });
    assert_eq!(pulled, None);
    Ok(())
}

#[tokio::test]
async fn tests_that_a_checkpoint_waits_until_the_script_resolves_or_removes_the_handler()
-> Result<(), Box<dyn Error>> {
    let handlers = LocatorHandlers::default();
    let resolved = handlers.register(overlay(1)?, false);
    let removed = handlers.register(overlay(1)?, true);

    let mut waiting = handlers.trigger(resolved).ok_or("registered")?;
    let early = time::timeout(Duration::from_millis(20), waiting.changed()).await;
    assert!(early.is_err(), "running until the script resolves it");
    handlers.resolve(resolved, false);
    assert!(
        waiting.changed().await.is_err(),
        "resolved: the sender went"
    );
    assert!(!handlers.is_running(resolved));

    let mut waiting = handlers.trigger(removed).ok_or("registered")?;
    handlers.resolve(removed, true);
    assert!(waiting.changed().await.is_err(), "times ran out: removed");
    assert!(handlers.trigger(removed).is_none());
    Ok(())
}

#[tokio::test]
async fn tests_that_a_tab_passes_its_checkpoint_while_one_of_its_handlers_runs()
-> Result<(), Box<dyn Error>> {
    let handlers = LocatorHandlers::default();
    let first = handlers.register(overlay(1)?, false);
    let second = handlers.register(overlay(1)?, true);
    let other = handlers.register(overlay(2)?, false);
    assert_eq!(uids(&handlers, 1), Some(vec![first, second]), "in order");
    assert_eq!(uids(&handlers, 3), None, "a tab without handlers");

    let _running = handlers.trigger(first).ok_or("registered")?;
    assert_eq!(uids(&handlers, 1), None, "the handler's own actions pass");
    assert_eq!(
        uids(&handlers, 2),
        Some(vec![other]),
        "other tabs check theirs"
    );
    handlers.resolve(first, false);
    assert_eq!(uids(&handlers, 1), Some(vec![first, second]));

    let tab = overlay(1)?.page().tab();
    let waiting = handlers.wait_on(tab);
    assert_eq!(uids(&handlers, 1), None, "a checkpoint of the tab waits");
    drop(waiting);
    assert_eq!(
        uids(&handlers, 1),
        Some(vec![first, second]),
        "its timeout ends it"
    );
    Ok(())
}
