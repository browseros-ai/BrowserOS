//! `BrowserTabs` over core's scripted connection: tab ids map to pages, and a tab the browser
//! does not list is not found.

use browseros_core::testing::{TestConnection, cdp_error};
use browseros_core::viewport::Viewports;
use browseros_core::{BrowserSession, BrowserSessionHooks, TabId};
use neo_takeover::{BrowserTabs, PartError};
use serde_json::{Value, json};
use std::sync::Arc;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn tab(id: i64) -> Value {
    json!({
        "tabId": id,
        "targetId": format!("target-{id}"),
        "windowId": 1,
        "url": format!("https://example.test/{id}"),
        "title": format!("Tab {id}"),
        "isActive": id == 3,
        "isLoading": false,
        "loadProgress": 1.0,
        "isPinned": false,
        "isHidden": false,
    })
}

fn browser(connection: &Arc<TestConnection>) -> BrowserTabs {
    BrowserTabs::new(
        BrowserSession::new(connection.clone(), BrowserSessionHooks::default()),
        Arc::new(Viewports::default()),
    )
}

#[tokio::test]
async fn a_tab_maps_to_its_page_and_an_unlisted_tab_is_not_found() -> TestResult {
    let connection = TestConnection::new([]);
    connection.always("Browser.getTabs", json!({ "tabs": [tab(3), tab(7)] }))?;
    let tabs = browser(&connection);

    let page = tabs.page(TabId(7)).await?;
    assert_eq!(page.tab_id, TabId(7));
    assert_eq!(page.target_id.as_str(), "target-7");
    assert_eq!(tabs.open_tabs().await?, [TabId(3), TabId(7)]);
    assert!(matches!(
        tabs.page(TabId(9)).await,
        Err(PartError::TabNotFound(TabId(9)))
    ));

    connection.always("Browser.getTabs", json!({ "tabs": [tab(3)] }))?;
    assert!(
        matches!(tabs.page(TabId(7)).await, Err(PartError::TabNotFound(_))),
        "a tab that closed since"
    );
    Ok(())
}

#[tokio::test]
async fn a_browser_that_does_not_answer_is_a_browser_error() -> TestResult {
    let connection = TestConnection::new([]);
    connection.always("Browser.getTabs", cdp_error("Browser closed"))?;
    let tabs = browser(&connection);
    assert!(matches!(
        tabs.page(TabId(3)).await,
        Err(PartError::Browser(_))
    ));
    assert!(matches!(tabs.open_tabs().await, Err(PartError::Browser(_))));
    Ok(())
}
