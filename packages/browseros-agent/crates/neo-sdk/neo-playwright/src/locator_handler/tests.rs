use std::error::Error;
use std::time::Duration;

use serde_json::json;
use tokio::{task, time};

use super::LocatorHandlers;
use crate::target::LocatorTarget;

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
