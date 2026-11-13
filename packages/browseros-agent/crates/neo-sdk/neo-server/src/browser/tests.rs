//! `BrowserLink` and `tab_watch` against the fake browser: the connect loop, the one session,
//! closes noticed without anyone listing tabs, and the tab events the host API learns from.

use std::error::Error;
use std::future;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use browseros_core::{NativeCapabilities, TabId, page_js};
use neo_trace::TaskId;
use tokio::sync::broadcast;
use tokio::time;

use crate::browser::{
    BrowserLink, LinkStatus, PageRef, RunScope, Sight, TabEvent, TabListing, TabRegistry, Watched,
};
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
