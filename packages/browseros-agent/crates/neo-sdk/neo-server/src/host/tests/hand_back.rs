//! The takeover's hand-back as a host meets it: `openedTabs` on the real host API, fed by the
//! real tab watch over the fake browser. Each tab the watch sees open reaches every active
//! takeover as the watch sees it, with the opener the browser announced, never through the tab
//! feed, which waits out each close's release before it reads on.

use std::error::Error;

use browseros_core::TabId;
use serde_json::{Value, json};
use tokio::sync::broadcast;

use super::{Host, Reply, TOKEN, TestResult, request, within};
use crate::browser::TabEvent;

/// `POST /v1/takeovers` on `tab`, without a viewer: its id.
async fn take_over(host: &Host, tab: TabId) -> Result<String, Box<dyn Error>> {
    let started = request(
        &host.socket,
        "POST",
        "/v1/takeovers",
        Some(TOKEN),
        Some(&json!({ "tabId": tab.0 })),
    )
    .await?;
    assert_eq!(started.status, 200, "{started:?}");
    Ok(started.body["id"]
        .as_str()
        .ok_or("the takeover's id")?
        .to_owned())
}

/// The takeover's hand-back: the takeover as `GET /v1/takeovers/{id}?wait=10` answers once it
/// is released, its `openedTabs` included.
async fn hand_back(host: &Host, id: &str) -> Result<Value, Box<dyn Error>> {
    let Reply { status, body } = request(
        &host.socket,
        "GET",
        &format!("/v1/takeovers/{id}?wait=10"),
        Some(TOKEN),
        None,
    )
    .await?;
    assert_eq!(
        (status, &body["state"]),
        (200, &json!("released")),
        "{body}"
    );
    Ok(body)
}

/// Waits until the tab watch reported `tab` opened; its open sink ran just before.
async fn until_opened(events: &mut broadcast::Receiver<TabEvent>, tab: TabId) -> TestResult {
    within(async {
        loop {
            if let Ok(TabEvent::Opened { tab: opened, .. }) = events.recv().await
                && opened == tab
            {
                return;
            }
        }
    })
    .await
}

/// The tab ids `openedTabs` lists.
fn listed(takeover: &Value) -> Vec<i64> {
    takeover["openedTabs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tab| tab["tabId"].as_i64())
        .collect()
}

/// The takeover's own tab opens a popup and closes at once, before the watch lists again: its
/// close releases the takeover (`tab_closed`), and the popup is listed with that tab as its
/// opener, not as a tab opened by hand, though the browser and the registry forgot the link
/// with the opener.
#[tokio::test]
async fn a_popup_whose_opener_closes_at_once_is_listed_with_its_opener() -> TestResult {
    let host = Host::start(Some(TOKEN)).await?;
    let shared = host.fake.open_tab("https://shop.example/", "Shop");
    host.until_serving().await?;
    let id = take_over(&host, shared).await?;
    host.fake.hold_events();
    let popup = host.fake.open_popup(shared, "https://pay.example/");
    assert!(host.fake.close_tab(shared), "the fake had the tab");
    host.fake.release_events();
    let takeover = hand_back(&host, &id).await?;
    assert_eq!(takeover["releaseReason"], "tab_closed", "{takeover}");
    assert_eq!(
        takeover["openedTabs"],
        json!([{
            "tabId": popup.0,
            "url": "https://pay.example/",
            "inFront": false,
            "taskId": null,
            "openerTabId": shared.0,
        }]),
        "{takeover}"
    );
    Ok(())
}

/// Two takeovers in flight. The first one's tab closes, and its release waits on the browser,
/// with the tab feed behind it. The owner opens a tab meanwhile, and the host releases the
/// second: the second lists the tab, which opened while it was active; the first, already
/// releasing when the tab opened, does not.
#[tokio::test]
async fn an_open_while_a_closing_tabs_release_waits_is_credited_to_the_takeover_active_then()
-> TestResult {
    let host = Host::start(Some(TOKEN)).await?;
    let closing = host.fake.open_tab("https://closing.example/", "Closing");
    let staying = host.fake.open_tab("https://staying.example/", "Staying");
    host.until_serving().await?;
    let first = take_over(&host, closing).await?;
    let second = take_over(&host, staying).await?;
    let mut events = host.link.tab_events();
    // A tab both noted, so the first's release lists the browser's tabs.
    let both = host.fake.open_tab("https://both.example/", "Both");
    until_opened(&mut events, both).await?;
    // The close's own listing answers; the first's release listing after it waits.
    let before = host.fake.count("Browser.getTabs");
    host.fake.hold_after("Browser.getTabs", 1);
    assert!(host.fake.close_tab(closing), "the fake had the tab");
    host.until_called("Browser.getTabs", before + 1).await?;
    let late = host.fake.open_tab("https://late.example/", "Late");
    until_opened(&mut events, late).await?;
    let released_second = request(
        &host.socket,
        "POST",
        &format!("/v1/takeovers/{second}/release"),
        Some(TOKEN),
        None,
    )
    .await?;
    assert_eq!(released_second.status, 200, "{released_second:?}");
    assert_eq!(
        listed(&released_second.body),
        vec![both.0, late.0],
        "{}",
        released_second.body
    );
    host.fake.release_held();
    let released_first = hand_back(&host, &first).await?;
    assert_eq!(released_first["releaseReason"], "tab_closed");
    assert_eq!(
        listed(&released_first),
        vec![both.0],
        "the late tab opened once the first was releasing: {released_first}"
    );
    Ok(())
}
