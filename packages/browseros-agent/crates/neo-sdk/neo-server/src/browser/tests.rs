//! `BrowserLink` and `tab_watch` against the fake browser: the connect loop, the one session,
//! closes noticed without anyone listing tabs, the tab events the host API learns from, and what
//! tab telemetry's ledger hears from the watch.

use std::error::Error;
use std::future;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use browseros_cdp::CdpError;
use browseros_core::{NativeCapabilities, TabId, page_js};
use neo_telemetry::{Telemetry, TelemetryConfig};
use neo_trace::TaskId;
use serde_json::json;
use tokio::sync::broadcast;
use tokio::time;

use crate::browser::{
    BrowserLink, LinkStatus, PageRef, RunScope, Sight, TabEvent, TabListing, TabRegistry, Watched,
};
use crate::telemetry_rig::Scrape;
use crate::test_support::FakeCdp;

#[derive(Default)]
struct Marker;

fn start(fake: &Arc<FakeCdp>, tabs: &Arc<TabRegistry>) -> Arc<BrowserLink> {
    let fake = Arc::clone(fake);
    BrowserLink::start_with(NativeCapabilities::default(), Arc::clone(tabs), move || {
        future::ready(fake.connect())
    })
}

/// Waits for `done`, failing after `limit`. Under paused time the sleeps advance the clock, so
/// the background loops' own timers fire.
async fn eventually(limit: Duration, mut done: impl FnMut() -> bool) -> Result<(), Box<dyn Error>> {
    let poll = async {
        while !done() {
            time::sleep(Duration::from_millis(10)).await;
        }
    };
    time::timeout(limit, poll)
        .await
        .map_err(|error| format!("not within {limit:?}: {error}").into())
}

/// The page core assigned to the fake's `tab`, from a fresh listing.
async fn page_for(link: &BrowserLink, tab: TabId) -> Result<PageRef, Box<dyn Error>> {
    let run = link.view().ok_or("the link is up")?;
    let listing = run.view.list_pages().await?;
    let info = listing
        .pages
        .iter()
        .find(|page| page.tab_id == tab)
        .ok_or("core lists the fake's tab")?;
    Ok(PageRef::from(info))
}

#[tokio::test(start_paused = true)]
async fn the_first_connect_is_retried_until_the_browser_answers() -> Result<(), Box<dyn Error>> {
    let fake = FakeCdp::new();
    fake.set_reachable(false);
    let tabs = Arc::new(TabRegistry::new());
    let link = start(&fake, &tabs);
    eventually(Duration::from_secs(5), || {
        link.status().last_error.is_some()
    })
    .await?;
    assert!(
        !link.is_ready(),
        "not ready while the browser refuses connections"
    );
    assert!(
        link.view().is_none(),
        "no run can capture a view before the first connect"
    );

    fake.set_reachable(true);
    eventually(Duration::from_secs(60), || link.is_ready()).await?;
    let attempts = fake.connect_attempts();
    assert!(attempts >= 2, "the failed first connect was retried");
    time::sleep(Duration::from_secs(120)).await;
    assert_eq!(
        fake.connect_attempts(),
        attempts,
        "once linked the loop ends: one session per process, and the client reconnects itself"
    );

    assert_eq!(
        link.status(),
        LinkStatus {
            connected: true,
            epoch: Some(1),
            last_error: None,
        },
        "linked on the fake's first socket, and the last error is cleared"
    );
    link.session().ok_or("linked")?;
    assert!(
        page_js::isolated_world_enabled(),
        "core's DOM scripts stay out of the page world"
    );
    Ok(())
}

#[tokio::test]
async fn start_reports_why_the_cdp_port_does_not_answer() -> Result<(), Box<dyn Error>> {
    // A port nothing listens on: bound, read, released.
    let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    let link = BrowserLink::start(
        port,
        NativeCapabilities::default(),
        Arc::new(TabRegistry::new()),
    );
    eventually(Duration::from_secs(10), || {
        link.status().last_error.is_some()
    })
    .await?;
    assert!(!link.is_ready(), "nothing answers on the port");
    assert_eq!(link.status().epoch, None, "never linked");
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_tab_closed_by_someone_else_drops_its_slots() -> Result<(), Box<dyn Error>> {
    let fake = FakeCdp::new();
    let owners_tab = fake.open_tab("https://example.com/", "Example");
    let tabs = Arc::new(TabRegistry::new());
    let link = start(&fake, &tabs);
    eventually(Duration::from_secs(5), || {
        fake.count("Target.setDiscoverTargets") == 1
    })
    .await?;
    let page = page_for(&link, owners_tab).await?;
    tabs.touched(&page, tabs.seen());
    let slot = tabs
        .slot::<Marker>(&page)
        .ok_or("a touched tab has slots")?;

    assert!(fake.close_tab(owners_tab), "the fake had the tab");
    eventually(Duration::from_secs(5), || !tabs.is_open(page.tab)).await?;
    assert_eq!(
        Arc::strong_count(&slot),
        1,
        "the registry dropped the closed tab's slot"
    );
    assert!(
        link.page(page.tab).await.is_none(),
        "core forgot the page too"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_reconnect_renews_discovery_and_catches_tabs_closed_meanwhile()
-> Result<(), Box<dyn Error>> {
    let fake = FakeCdp::new();
    let tab = fake.open_tab("https://example.com/", "Example");
    let tabs = Arc::new(TabRegistry::new());
    let link = start(&fake, &tabs);
    eventually(Duration::from_secs(5), || {
        fake.count("Target.setDiscoverTargets") == 1
    })
    .await?;
    let first = link.view().ok_or("linked")?.epoch;
    let page = page_for(&link, tab).await?;
    tabs.touched(&page, tabs.seen());

    fake.drop_connection();
    assert!(!link.is_ready(), "a dropped socket is not ready");
    assert!(link.view().is_none(), "no run starts on a dropped socket");
    assert!(
        fake.close_tab(tab),
        "closed while the socket is down, so no event reaches Neo"
    );
    let second = fake.reconnect();

    assert!(link.is_ready(), "ready again on the new socket");
    assert!(
        !link.is_current(first),
        "a run captured before the reconnect is stale"
    );
    assert!(link.is_current(second), "a run captured now is current");
    assert_eq!(
        link.status().epoch,
        Some(second),
        "the link reports the new epoch"
    );
    eventually(Duration::from_secs(5), || !tabs.is_open(page.tab)).await?;
    let discovered: Vec<u64> = fake
        .calls()
        .iter()
        .filter(|call| call.method == "Target.setDiscoverTargets")
        .map(|call| call.epoch)
        .collect();
    assert_eq!(
        discovered,
        vec![first, second],
        "discovery is asked for once per socket"
    );
    Ok(())
}

/// The next tab event, within 5 s of fake time.
async fn next_event(
    events: &mut broadcast::Receiver<TabEvent>,
) -> Result<TabEvent, Box<dyn Error>> {
    Ok(time::timeout(Duration::from_secs(5), events.recv()).await??)
}

/// One listing that finds a tab gone and another new reports the open first: a takeover released
/// by its tab's close must have noted a tab the owner opened in the same moment.
#[tokio::test(start_paused = true)]
async fn a_listing_reports_its_opens_before_its_closes() -> Result<(), Box<dyn Error>> {
    let fake = FakeCdp::new();
    let first = fake.open_tab("https://example.com/", "Example");
    let link = start(&fake, &Arc::new(TabRegistry::new()));
    let mut events = link.tab_events();
    assert!(matches!(
        next_event(&mut events).await?,
        TabEvent::Watching(_)
    ));
    assert!(fake.close_tab(first), "the fake had the tab");
    let second = fake.open_tab("https://example.org/", "Other");
    assert_eq!(
        (
            next_event(&mut events).await?,
            next_event(&mut events).await?
        ),
        (
            TabEvent::Opened {
                epoch: 1,
                tab: second,
            },
            TabEvent::Closed {
                epoch: 1,
                tab: first,
            },
        )
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn every_open_and_close_is_reported_in_order_under_its_socket() -> Result<(), Box<dyn Error>>
{
    let fake = FakeCdp::new();
    let first = fake.open_tab("https://example.com/", "Example");
    let link = start(&fake, &Arc::new(TabRegistry::new()));
    let mut events = link.tab_events();
    let watched = Watched {
        epoch: 1,
        generation: 1,
    };
    assert_eq!(
        next_event(&mut events).await?,
        TabEvent::Watching(watched),
        "the first socket starts the watch, and the tabs it already had never count as opened"
    );
    assert_eq!(
        link.watched_listing().await,
        TabListing {
            watched,
            open: vec![first],
        },
        "a listing under the watch has what is already open"
    );

    let second = fake.open_tab("https://example.org/", "Other");
    assert_eq!(
        next_event(&mut events).await?,
        TabEvent::Opened {
            epoch: 1,
            tab: second,
        }
    );
    assert!(fake.close_tab(first), "the fake had the tab");
    assert_eq!(
        next_event(&mut events).await?,
        TabEvent::Closed {
            epoch: 1,
            tab: first,
        }
    );

    fake.drop_connection();
    let epoch = fake.reconnect();
    assert_eq!(
        next_event(&mut events).await?,
        TabEvent::Watching(Watched {
            epoch,
            generation: 2,
        }),
        "a new socket starts the watch afresh, under its own epoch"
    );
    assert_eq!(link.watched_listing().await.open, vec![second]);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn only_a_listing_without_the_tab_turns_a_destroyed_target_into_a_close()
-> Result<(), Box<dyn Error>> {
    let fake = FakeCdp::new();
    let kept = fake.open_tab("https://example.com/", "Kept");
    let marker = fake.open_tab("https://example.org/", "Marker");
    let link = start(&fake, &Arc::new(TabRegistry::new()));
    let mut events = link.tab_events();
    let started = next_event(&mut events).await?;
    assert!(
        matches!(started, TabEvent::Watching(_)),
        "the watch starts first: {started:?}"
    );

    assert!(
        fake.replace_target(kept),
        "the tab swaps its page target and stays open: a destroyed target, not a close"
    );
    // Opened and closed inside one burst of events: no listing ever has this tab.
    fake.hold_events();
    let brief = fake.open_tab("https://example.net/", "Brief");
    assert!(fake.close_tab(brief), "the fake had the tab");
    fake.release_events();
    assert_eq!(
        next_event(&mut events).await?,
        TabEvent::Closed {
            epoch: 1,
            tab: brief,
        },
        "its destroy was noted before the burst's one listing, which lacks it"
    );
    assert!(fake.close_tab(marker), "the fake had the tab");
    assert_eq!(
        next_event(&mut events).await?,
        TabEvent::Closed {
            epoch: 1,
            tab: marker,
        },
        "nothing was reported for the tab that only changed target"
    );
    assert!(
        link.watched_listing().await.open.contains(&kept),
        "and it is still open"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_popup_is_its_openers_family_from_its_announcement_before_any_listing()
-> Result<(), Box<dyn Error>> {
    let fake = FakeCdp::new();
    let opener_tab = fake.open_tab("https://example.com/", "Example");
    let tabs = Arc::new(TabRegistry::new());
    let link = start(&fake, &tabs);
    let mut events = link.tab_events();
    assert!(matches!(
        next_event(&mut events).await?,
        TabEvent::Watching(_)
    ));
    // The watch reads one thing at a time: once it reports this tab, its listings are done and
    // it waits on the next target event, so the listing hung below is the popup's own.
    let marker = fake.open_tab("https://example.org/", "Other");
    assert!(matches!(
        next_event(&mut events).await?,
        TabEvent::Opened { tab, .. } if tab == marker
    ));
    let opener = page_for(&link, opener_tab).await?;
    let task = TaskId::parse("host-s")?;
    tabs.ensure_task(&task)?;
    tabs.touched(&opener, tabs.seen());
    tabs.own(&opener, &task)?;

    fake.hang("Browser.getTabs");
    let popup = fake.open_popup(opener_tab, "https://example.net/");
    let family = RunScope::Family(task);
    eventually(Duration::from_secs(5), || {
        tabs.sight(&family, popup) == Sight::Shown
    })
    .await?;
    assert_eq!(
        tabs.owner(popup),
        None,
        "no listing registered the popup: its announcement alone made it the family's"
    );
    assert!(
        fake.count("Browser.getTabs") > 0,
        "the watch's listing hangs"
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn a_popup_that_opened_and_closed_between_two_listings_leaves_no_announcement()
-> Result<(), Box<dyn Error>> {
    let fake = FakeCdp::new();
    let opener_tab = fake.open_tab("https://example.com/", "Example");
    let tabs = Arc::new(TabRegistry::new());
    let link = start(&fake, &tabs);
    let mut events = link.tab_events();
    assert!(matches!(
        next_event(&mut events).await?,
        TabEvent::Watching(_)
    ));
    let opener = page_for(&link, opener_tab).await?;
    let task = TaskId::parse("host-s")?;
    tabs.ensure_task(&task)?;
    tabs.touched(&opener, tabs.seen());
    tabs.own(&opener, &task)?;

    // One burst: no listing between the popup's announcement and its close, so core never
    // caches it and its close never reaches the registry's removal.
    fake.hold_events();
    let popup = fake.open_popup(opener_tab, "https://example.net/");
    assert!(fake.close_tab(popup), "the fake had the popup");
    fake.release_events();
    assert!(matches!(
        next_event(&mut events).await?,
        TabEvent::Closed { tab, .. } if tab == popup
    ));
    assert_eq!(
        tabs.sight(&RunScope::Family(task), popup),
        Sight::Foreign,
        "the watch's confirmed close dropped the announcement"
    );
    Ok(())
}

/// The tab metrics `telemetry` serves, as `name{labels} value` lines (the gauge left out).
fn tab_counts(telemetry: &Telemetry) -> Result<Vec<String>, Box<dyn Error>> {
    Ok(Scrape::of(telemetry)?.lines_of(&[
        "neo_tab_opened_total",
        "neo_tab_closed_total",
        "neo_renderer_crashes_total",
    ]))
}

/// The tab watch is the only writer of browser facts to tab telemetry: a tab open before Neo
/// attached, the owner's tab and its popup are each born once; a renderer crash on an idle tab,
/// no op on it, counts once and names its close; a reconnect to the same browser counts no tab
/// again and closes the one that went meanwhile; a browser that comes back as another one closes
/// every old tab as `shutdown`, even under a tab id it reused.
#[tokio::test(start_paused = true)]
async fn the_tab_watch_tells_tab_telemetry_what_the_browser_did() -> Result<(), Box<dyn Error>> {
    let fake = FakeCdp::new();
    let before = fake.open_tab("https://example.com/", "Example");
    let (telemetry, _probe) = Telemetry::in_memory(&TelemetryConfig::default());
    let tabs = Arc::new(TabRegistry::new_with_telemetry(telemetry.clone()));
    let _link = start(&fake, &tabs);
    let found = |count: u64| {
        let telemetry = telemetry.clone();
        move || Scrape::of(&telemetry).is_ok_and(|scrape| scrape.open_tabs() == (0, count, 0))
    };
    eventually(Duration::from_secs(5), found(1)).await?;
    let owners = fake.open_tab("https://example.org/", "Other");
    let popup = fake.open_popup(owners, "https://example.net/");
    eventually(Duration::from_secs(5), found(3)).await?;

    assert!(
        fake.page_event(
            owners,
            "Target.targetCrashed",
            json!({ "targetId": format!("target-{}", owners.0), "status": "crashed", "errorCode": 11 }),
        ),
        "the fake had the tab"
    );
    assert!(fake.close_tab(owners), "the fake had the tab");
    eventually(Duration::from_secs(5), found(2)).await?;

    fake.drop_connection();
    assert!(fake.close_tab(popup), "closed while no socket hears of it");
    fake.reconnect();
    eventually(Duration::from_secs(5), found(1)).await?;
    // A close nobody asked for waits two seconds; the socket stays up meanwhile,
    // so it is the user's. A drop within the wait would have made it the shutdown's.
    let user = r#"neo_tab_closed_total{neo_tab_close_reason="user",neo_tab_origin="found"} 1"#;
    eventually(Duration::from_secs(5), || {
        tab_counts(&telemetry).is_ok_and(|counts| counts.iter().any(|line| line == user))
    })
    .await?;

    // Restarted while Neo was away: every target is new, and tab number `before` came back.
    fake.drop_connection();
    assert!(fake.replace_target(before), "the fake had the tab");
    fake.reconnect();
    let shutdown =
        r#"neo_tab_closed_total{neo_tab_close_reason="shutdown",neo_tab_origin="found"} 1"#;
    eventually(Duration::from_secs(5), || {
        tab_counts(&telemetry).is_ok_and(|counts| counts.iter().any(|line| line == shutdown))
    })
    .await?;
    eventually(Duration::from_secs(5), found(1)).await?;

    assert_eq!(
        tab_counts(&telemetry)?,
        [
            "neo_renderer_crashes_total 1",
            r#"neo_tab_closed_total{neo_tab_close_reason="crash",neo_tab_origin="found"} 1"#,
            shutdown,
            r#"neo_tab_closed_total{neo_tab_close_reason="user",neo_tab_origin="found"} 1"#,
            r#"neo_tab_opened_total{neo_tab_origin="found"} 4"#,
        ],
        "three tabs found, each closed once (the crashed one as crash, the one gone while Neo \
         was away as the user's, the restarted browser's as shutdown), the reborn tab found \
         once more, and the crash counted once"
    );
    Ok(())
}

/// Review F7: after core's event channel dropped events, the watch rebuilds its map of live page
/// targets with one `Target.getTargets` before the next listing. A tab sitting on an excluded page
/// is neither closed by the lag nor counted again when it comes back, and the watch is not left
/// without that map for the rest of the socket.
#[tokio::test(start_paused = true)]
async fn after_a_lag_the_watch_still_knows_a_tab_on_an_excluded_page() -> Result<(), Box<dyn Error>>
{
    let fake = FakeCdp::new();
    let shown = fake.open_tab("https://example.com/", "Example");
    let parked = fake.open_tab("https://example.org/", "Parked");
    let (telemetry, _probe) = Telemetry::in_memory(&TelemetryConfig::default());
    let tabs = Arc::new(TabRegistry::new_with_telemetry(telemetry.clone()));
    let _link = start(&fake, &tabs);
    let found = |count: u64| {
        let telemetry = telemetry.clone();
        move || Scrape::of(&telemetry).is_ok_and(|scrape| scrape.open_tabs() == (0, count, 0))
    };
    eventually(Duration::from_secs(5), found(2)).await?;
    // The parked tab moves to an extension page: core's listing leaves it out from now on.
    assert!(
        fake.navigate_tab(parked, "chrome-extension://abc/page.html", "Extension"),
        "the fake had the tab"
    );
    let page = |tab: TabId, url: &str| json!({ "targetId": format!("target-{}", tab.0), "type": "page", "tabId": tab.0, "url": url });

    // Core's channel overflows while the watch waits on a listing.
    let listings = fake.count("Browser.getTabs");
    let target_listings = fake.count("Target.getTargets");
    fake.hold_next("Browser.getTabs");
    let lagger = fake.open_tab("https://example.net/", "Lagger");
    fake.answer(
        "Target.getTargets",
        json!({ "targetInfos": [
            page(shown, "https://example.com/"),
            page(parked, "chrome-extension://abc/page.html"),
            page(lagger, "https://example.net/"),
        ] }),
    );
    eventually(Duration::from_secs(5), || {
        fake.count("Browser.getTabs") > listings
    })
    .await?;
    let changed =
        json!({ "targetInfo": { "targetId": format!("target-{}", shown.0), "type": "page" } });
    for _ in 0..1100 {
        assert!(
            fake.page_event(shown, "Target.targetInfoChanged", changed.clone()),
            "the fake had the tab"
        );
    }
    fake.release_held();
    // Read after the lag: the watch rebuilt its map, then listed.
    let first = fake.open_tab("https://example.edu/", "First");
    eventually(Duration::from_secs(5), found(4)).await?;
    assert!(
        fake.count("Target.getTargets") > target_listings,
        "the watch listed the targets after the lag"
    );

    // Back on a listed page: the same tab, not a new one.
    assert!(
        fake.navigate_tab(parked, "https://example.org/", "Back"),
        "the fake had the tab"
    );
    let second = fake.open_tab("https://example.edu/two", "Second");
    eventually(Duration::from_secs(5), found(5)).await?;
    assert!(
        fake.close_tab(first) && fake.close_tab(second),
        "the fake had both tabs"
    );
    eventually(Duration::from_secs(5), found(3)).await?;
    // The owner's two closes count once their wait is over.
    let expected = [
        r#"neo_tab_closed_total{neo_tab_close_reason="user",neo_tab_origin="found"} 2"#,
        r#"neo_tab_opened_total{neo_tab_origin="found"} 5"#,
    ];
    eventually(Duration::from_secs(5), || {
        tab_counts(&telemetry).is_ok_and(|counts| counts == expected)
    })
    .await?;
    assert_eq!(
        tab_counts(&telemetry)?,
        expected,
        "five tabs born once each, and only the two the owner closed closed: the parked tab was \
         never closed nor born twice"
    );
    Ok(())
}

/// After a lag, a failed `Target.getTargets` leaves the watch without its live targets, so the
/// ledger hears no listing until a later rebuild succeeds: a tab on an excluded page is never
/// closed for it, and nothing is missed once the rebuild works.
#[tokio::test(start_paused = true)]
async fn a_failed_rebuild_after_a_lag_closes_no_tab() -> Result<(), Box<dyn Error>> {
    let fake = FakeCdp::new();
    let shown = fake.open_tab("https://example.com/", "Example");
    let parked = fake.open_tab("https://example.org/", "Parked");
    let (telemetry, _probe) = Telemetry::in_memory(&TelemetryConfig::default());
    let tabs = Arc::new(TabRegistry::new_with_telemetry(telemetry.clone()));
    let _link = start(&fake, &tabs);
    let found = |count: u64| {
        let telemetry = telemetry.clone();
        move || Scrape::of(&telemetry).is_ok_and(|scrape| scrape.open_tabs() == (0, count, 0))
    };
    eventually(Duration::from_secs(5), found(2)).await?;
    assert!(
        fake.navigate_tab(parked, "chrome-extension://abc/page.html", "Extension"),
        "the fake had the tab"
    );
    let page = |tab: TabId, url: &str| json!({ "targetId": format!("target-{}", tab.0), "type": "page", "tabId": tab.0, "url": url });

    let listings = fake.count("Browser.getTabs");
    let target_listings = fake.count("Target.getTargets");
    fake.fail(
        "Target.getTargets",
        CdpError::Protocol {
            code: -32000,
            message: "target listing unavailable".to_owned(),
        },
    );
    fake.hold_next("Browser.getTabs");
    let lagger = fake.open_tab("https://example.net/", "Lagger");
    eventually(Duration::from_secs(5), || {
        fake.count("Browser.getTabs") > listings
    })
    .await?;
    let changed =
        json!({ "targetInfo": { "targetId": format!("target-{}", shown.0), "type": "page" } });
    for _ in 0..1100 {
        assert!(
            fake.page_event(shown, "Target.targetInfoChanged", changed.clone()),
            "the fake had the tab"
        );
    }
    fake.release_held();
    // Several failed rebuilds, each followed by a listing the ledger does not hear, and past
    // the two seconds a close nobody asked for would wait.
    eventually(Duration::from_secs(10), || {
        fake.count("Target.getTargets") > target_listings + 2
    })
    .await?;
    time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        tab_counts(&telemetry)?,
        [r#"neo_tab_opened_total{neo_tab_origin="found"} 3"#],
        "the parked tab is not closed while the live targets are unknown"
    );

    let first = fake.open_tab("https://example.edu/", "First");
    fake.answer(
        "Target.getTargets",
        json!({ "targetInfos": [
            page(shown, "https://example.com/"),
            page(parked, "chrome-extension://abc/page.html"),
            page(lagger, "https://example.net/"),
            page(first, "https://example.edu/"),
        ] }),
    );
    fake.unfail("Target.getTargets");
    eventually(Duration::from_secs(5), found(4)).await?;
    time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        tab_counts(&telemetry)?,
        [r#"neo_tab_opened_total{neo_tab_origin="found"} 4"#],
        "once a rebuild works the tab opened meanwhile counts, and the parked tab is still open"
    );
    Ok(())
}

/// Through the real watch: tabs that close and then lose their socket within the
/// two-second wait were the browser exiting (`docker stop` quits the browser before Neo hears of
/// it), so they count as `shutdown`. The gauge drops at once all the same.
#[tokio::test(start_paused = true)]
async fn closes_followed_by_a_socket_drop_count_as_shutdown() -> Result<(), Box<dyn Error>> {
    let fake = FakeCdp::new();
    let first = fake.open_tab("https://example.com/", "Example");
    let second = fake.open_tab("https://example.org/", "Other");
    let (telemetry, _probe) = Telemetry::in_memory(&TelemetryConfig::default());
    let tabs = Arc::new(TabRegistry::new_with_telemetry(telemetry.clone()));
    let _link = start(&fake, &tabs);
    let found = |count: u64| {
        let telemetry = telemetry.clone();
        move || Scrape::of(&telemetry).is_ok_and(|scrape| scrape.open_tabs() == (0, count, 0))
    };
    eventually(Duration::from_secs(5), found(2)).await?;
    assert!(
        fake.close_tab(first) && fake.close_tab(second),
        "the fake had both tabs"
    );
    eventually(Duration::from_secs(5), found(0)).await?;
    assert_eq!(
        tab_counts(&telemetry)?,
        [r#"neo_tab_opened_total{neo_tab_origin="found"} 2"#],
        "the closes wait for their reason"
    );
    fake.drop_connection();
    let expected = [
        r#"neo_tab_closed_total{neo_tab_close_reason="shutdown",neo_tab_origin="found"} 2"#,
        r#"neo_tab_opened_total{neo_tab_origin="found"} 2"#,
    ];
    eventually(Duration::from_secs(5), || {
        tab_counts(&telemetry).is_ok_and(|counts| counts == expected)
    })
    .await?;
    Ok(())
}
