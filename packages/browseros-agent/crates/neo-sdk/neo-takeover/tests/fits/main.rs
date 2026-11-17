//! `Fits` as a host sees it, over a fake Chromium and a fake Xvnc: what the screen, the window
//! and the page show after each call, and the order the fit changed them in.
//!
//! The fits loop and the screen watcher run on real TCP and real time, so these tests wait
//! for what the loop does with short deadlines rather than pausing the clock.

#[allow(
    dead_code,
    reason = "the shared fake also exposes gates and probes for the takeover flow's tests"
)]
mod browser;
mod scale;

use browser::{CHROME_HEIGHT, FakeBrowser, Metrics};
use browseros_core::viewport::{Size, ViewportMode, ViewportRequest, Viewports};
use browseros_core::{BrowserSession, BrowserSessionHooks, BrowserView, TabId, TargetId};
use neo_takeover::testing::FakeXvnc;
use neo_takeover::{
    BarAction, BarOutcome, BrowserBar, BrowserTabs, Fit, FitId, FitMode, FitState, Fits, Fitted,
    PartError, ScreenSize, Viewer,
};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

const HOME: ScreenSize = ScreenSize {
    width: 1600,
    height: 1000,
};
const PHONE: ScreenSize = ScreenSize {
    width: 390,
    height: 844,
};
const PHONE_TURNED: ScreenSize = ScreenSize {
    width: 412,
    height: 915,
};
const LAPTOP: ScreenSize = ScreenSize {
    width: 1280,
    height: 800,
};

struct Harness {
    xvnc: Arc<FakeXvnc>,
    browser: Arc<FakeBrowser>,
    session: Arc<BrowserSession>,
    viewports: Arc<Viewports>,
    fits: Arc<Fits>,
}

async fn harness(tabs: &[i64]) -> Result<Harness, Box<dyn std::error::Error + Send + Sync>> {
    let xvnc = Arc::new(FakeXvnc::start(HOME).await?);
    let browser = FakeBrowser::new(Arc::clone(&xvnc), tabs);
    let session = BrowserSession::new(browser.clone(), BrowserSessionHooks::default());
    let viewports = Arc::new(Viewports::default());
    let fits = Fits::new(
        BrowserTabs::new(Arc::clone(&session), Arc::clone(&viewports)),
        xvnc.xvnc(HOME),
    );
    Ok(Harness {
        xvnc,
        browser,
        session,
        viewports,
        fits,
    })
}

/// What a phone of `size` shows: mobile emulation at its size, DPR 2, touch.
fn phone(size: ScreenSize) -> Metrics {
    Metrics {
        width: size.width,
        height: size.height,
        device_scale: 2.0,
        touch: true,
    }
}

/// What a page with no emulation shows on Neo's screen in a maximized window.
fn natural(screen: ScreenSize) -> Metrics {
    Metrics {
        width: screen.width,
        height: screen.height - CHROME_HEIGHT,
        device_scale: 1.0,
        touch: false,
    }
}

impl Harness {
    /// Sets tab `tab`'s viewport the way the agent's own `setViewport` does.
    async fn agent_sets_viewport(&self, tab: i64, request: ViewportRequest) -> TestResult {
        let pages = self.session.pages.resolve_tab_ids(&[TabId(tab)]).await?;
        let page = pages.get(&TabId(tab)).cloned().ok_or("the tab is open")?;
        BrowserView::unbound(Arc::clone(&self.session), None)
            .with_viewports(Arc::clone(&self.viewports))
            .set_viewport(page, request, &CancellationToken::new())
            .await?;
        Ok(())
    }

    /// Waits up to 3 s for fit `id` to satisfy `done`, and returns it.
    async fn fit_when(&self, id: FitId, done: impl Fn(&Fit) -> bool) -> Result<Fit, String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let fit = self.fits.view(id).ok_or("no such fit")?;
            if done(&fit) {
                return Ok(fit);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("the fit never got there: {fit:?}"));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Waits for fit `id`'s first owner view read, after which a page its tab opens is pulled in.
    async fn first_read(&self, id: FitId) -> Result<Fit, String> {
        self.fit_when(id, |fit| fit.bar.is_some()).await
    }
}

/// The browser's navigations and tab closes so far, in order.
fn moves(browser: &FakeBrowser) -> Vec<String> {
    browser
        .changes()
        .into_iter()
        .filter(|change| {
            change.starts_with("Page.navigate") || change.starts_with("Browser.closeTab")
        })
        .collect()
}

#[tokio::test]
async fn a_phone_viewer_gets_a_fullscreen_mobile_page_in_chromiums_order() -> TestResult {
    let h = harness(&[3]).await?;
    let fit = h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;
    assert_eq!(
        (fit.tab, fit.screen, fit.mode, fit.fitted, fit.state),
        (
            TabId(3),
            PHONE,
            FitMode::Mobile,
            Fitted::Full,
            FitState::Active
        )
    );
    assert_eq!(h.fits.view(fit.id), Some(fit.clone()));
    assert_eq!(h.xvnc.size(), PHONE);
    assert_eq!(h.browser.window(), "fullscreen");
    assert_eq!(h.browser.metrics(3), phone(PHONE));
    // The screen before the window, the window before the viewport, Ctrl+0 before mobile
    // emulation; and the tab in front before any of them.
    assert_eq!(
        h.browser.changes(),
        [
            "Browser.activateTab tab 3 after 0",
            "Input.dispatchKeyEvent rawKeyDown Digit0 after 1",
            "Input.dispatchKeyEvent keyUp Digit0 after 1",
            "Browser.setWindowBounds fullscreen after 1",
            "Emulation.setDeviceMetricsOverride 390x844 after 1",
        ]
    );
    let keys = h.browser.calls_of("Input.dispatchKeyEvent");
    assert!(
        keys.iter().all(|key| key.params["modifiers"] == 2
            && key.params["key"] == "0"
            && key.params["windowsVirtualKeyCode"] == 48
            && key.params["nativeVirtualKeyCode"] == 48),
        "Ctrl+0 as a raw key with its native code: {keys:?}"
    );
    Ok(())
}

#[tokio::test]
async fn restore_puts_back_the_agents_own_viewport_the_window_and_the_screen() -> TestResult {
    let h = harness(&[3]).await?;
    let agents = ViewportRequest::new(ViewportMode::Mobile, None)?.with_size(Size {
        width: 400,
        height: 906,
    })?;
    h.agent_sets_viewport(3, agents).await?;
    let fit = h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;
    assert_eq!(h.browser.metrics(3), phone(PHONE));

    let restored = h.fits.restore(fit.id).await?;
    assert!(
        matches!(restored.state, FitState::Restored(_)),
        "{restored:?}"
    );
    assert_eq!(
        h.browser.metrics(3),
        phone(ScreenSize {
            width: 400,
            height: 906
        }),
        "the agent's own emulation, as it was"
    );
    assert_eq!(h.browser.window(), "maximized");
    assert_eq!(h.xvnc.size(), HOME);
    assert_eq!(
        h.xvnc.resizes(),
        ["390x844 id 7 flags 0", "1600x1000 id 7 flags 0"]
    );

    assert_eq!(
        h.fits.restore(fit.id).await?,
        restored,
        "restoring again changes nothing"
    );
    assert_eq!(h.xvnc.resizes().len(), 2);
    assert!(matches!(
        h.fits.navigate(fit.id, BarAction::Reload).await,
        Err(PartError::FitRestored)
    ));
    assert!(matches!(
        h.fits.restore(FitId::new()).await,
        Err(PartError::NotFound)
    ));
    Ok(())
}

#[tokio::test]
async fn a_busy_viewport_times_out_without_stranding_the_surface_or_claiming_a_fit() -> TestResult {
    let h = harness(&[3, 4]).await?;
    let view =
        BrowserView::unbound(Arc::clone(&h.session), None).with_viewports(Arc::clone(&h.viewports));
    let pages = h.session.pages.resolve_tab_ids(&[TabId(3)]).await?;
    let page = pages.get(&TabId(3)).ok_or("the busy tab is open")?;
    let _busy = view
        .prepare_playwright_viewport(
            page.clone(),
            ViewportRequest::playwright(Size {
                width: 800,
                height: 600,
            })?,
        )
        .await?;
    let failed = tokio::time::timeout(
        Duration::from_secs(7),
        h.fits.fit(TabId(3), Viewer::new(PHONE, None)?),
    )
    .await?;
    assert!(
        matches!(failed, Err(PartError::TimedOut(_))),
        "a fit must not wait for an unbounded browser acknowledgement"
    );
    assert_eq!(
        h.xvnc.size(),
        HOME,
        "a timed-out snapshot never changes the screen"
    );
    assert!(
        !h.browser
            .changes()
            .iter()
            .any(|change| change.starts_with("Browser.activateTab")),
        "no outdated restore target was committed"
    );
    // Tab 3 remains busy while another tab successfully claims and releases the surface.
    let other = tokio::time::timeout(
        Duration::from_secs(7),
        h.fits.fit(TabId(4), Viewer::new(PHONE, None)?),
    )
    .await??;
    h.fits.restore(other.id).await?;
    assert_eq!(h.xvnc.size(), HOME);
    Ok(())
}

#[tokio::test]
async fn a_fit_waits_for_an_inflight_resize_before_saving_what_it_will_restore() -> TestResult {
    let h = harness(&[3]).await?;
    h.agent_sets_viewport(
        3,
        ViewportRequest::playwright(Size {
            width: 640,
            height: 480,
        })?,
    )
    .await?;
    let view =
        BrowserView::unbound(Arc::clone(&h.session), None).with_viewports(Arc::clone(&h.viewports));
    let pages = h.session.pages.resolve_tab_ids(&[TabId(3)]).await?;
    let page = pages.get(&TabId(3)).ok_or("the script's tab is open")?;
    let requested = ViewportRequest::playwright(Size {
        width: 800,
        height: 600,
    })?;
    let change = view
        .prepare_playwright_viewport(page.clone(), requested)
        .await?;
    let reset = "Emulation.setTouchEmulationEnabled";
    h.browser.hold(reset);
    let applying = tokio::spawn(change.apply());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while h.browser.held(reset) == 0 {
        if tokio::time::Instant::now() > deadline {
            return Err("resize never reached its held acknowledgement".into());
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // A takeover's bounded settle can end while this acknowledged-metrics change still
    // waits. Its fit must snapshot the configuration after that in-flight change commits.
    let fits = Arc::clone(&h.fits);
    let fitting = tokio::spawn(async move { fits.fit(TabId(3), Viewer::new(PHONE, None)?).await });
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(!fitting.is_finished(), "the fit waits behind the resize");
    h.browser.let_go(reset);
    applying.await??;
    let fit = fitting.await??;
    h.fits.restore(fit.id).await?;
    assert_eq!(
        view.current_viewport(page),
        Some(requested),
        "the fit must save the last completed resize, not the configuration seen before waiting"
    );
    assert_eq!(
        h.browser.metrics(3),
        Metrics {
            width: 800,
            height: 600,
            device_scale: 1.0,
            touch: false
        }
    );
    Ok(())
}

#[tokio::test]
async fn a_refitted_owner_view_restores_the_scripts_viewport_and_desktop_context() -> TestResult {
    let h = harness(&[3, 4]).await?;
    let size = Size {
        width: 55,
        height: 5000,
    };
    let request = ViewportRequest::playwright(size)?;
    h.agent_sets_viewport(3, request).await?;
    let fit = h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;
    assert_eq!(h.browser.metrics(3), phone(PHONE));
    let refit = h.fits.fit(TabId(3), Viewer::new(LAPTOP, None)?).await?;
    assert_eq!(
        refit.id, fit.id,
        "a refit keeps the original saved viewport"
    );
    h.fits.restore(fit.id).await?;
    assert_eq!(
        h.browser.metrics(3),
        Metrics {
            width: 55,
            height: 5000,
            device_scale: 1.0,
            touch: false
        }
    );
    let view =
        BrowserView::unbound(Arc::clone(&h.session), None).with_viewports(Arc::clone(&h.viewports));
    let pages = h
        .session
        .pages
        .resolve_tab_ids(&[TabId(3), TabId(4)])
        .await?;
    let page = pages.get(&TabId(3)).ok_or("script's tab is open")?;
    let other = pages.get(&TabId(4)).ok_or("other tab is open")?;
    assert_eq!(
        view.current_viewport(page),
        Some(request),
        "restore retains the request profile, not just its dimensions"
    );
    assert_eq!(view.playwright_viewport(page), Some(size));
    assert_eq!(
        view.playwright_viewport(other),
        None,
        "configuration belongs to one tab"
    );
    assert_eq!(h.xvnc.size(), HOME);
    Ok(())
}

#[tokio::test]
async fn a_desktop_viewer_gets_the_default_viewport_in_a_maximized_window() -> TestResult {
    let h = harness(&[3]).await?;
    let fit = h.fits.fit(TabId(3), Viewer::new(LAPTOP, None)?).await?;
    assert_eq!((fit.mode, fit.fitted), (FitMode::Desktop, Fitted::Full));
    assert_eq!(h.xvnc.size(), LAPTOP);
    assert_eq!(h.browser.window(), "maximized");
    assert_eq!(
        h.browser.metrics(3),
        natural(LAPTOP),
        "no emulation: the page is the window's content area"
    );
    assert!(
        h.browser.calls_of("Browser.setWindowBounds").is_empty(),
        "a maximized window is left alone"
    );
    h.fits.restore(fit.id).await?;
    assert_eq!(h.xvnc.size(), HOME);
    Ok(())
}

#[tokio::test]
async fn two_fits_share_one_screen_and_the_last_one_puts_it_home() -> TestResult {
    let h = harness(&[3, 4]).await?;
    let first = h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;
    let second = h
        .fits
        .fit(TabId(4), Viewer::new(PHONE_TURNED, None)?)
        .await?;
    assert_ne!(first.id, second.id);
    assert_eq!(
        h.fits
            .fit(TabId(3), Viewer::new(PHONE_TURNED, None)?)
            .await?
            .id,
        first.id,
        "a tab with an active fit is refitted, not fitted twice"
    );
    assert_eq!(h.browser.metrics(3), phone(PHONE_TURNED));
    assert_eq!(h.browser.metrics(4), phone(PHONE_TURNED));

    h.fits.restore(first.id).await?;
    assert_eq!(
        h.xvnc.size(),
        PHONE_TURNED,
        "the other fit keeps the screen"
    );
    assert_eq!(h.browser.window(), "fullscreen");
    assert_eq!(
        h.browser.metrics(3).device_scale,
        1.0,
        "back to no emulation"
    );
    assert_eq!(h.browser.metrics(4), phone(PHONE_TURNED));

    h.fits.restore(second.id).await?;
    assert_eq!(h.xvnc.size(), HOME);
    assert_eq!(
        h.browser.window(),
        "maximized",
        "the window's home is what the first fit found, not the fullscreen the second found"
    );
    assert_eq!(h.browser.metrics(4), natural(HOME));
    Ok(())
}

#[tokio::test]
async fn the_fit_follows_the_owners_own_resizes() -> TestResult {
    let h = harness(&[3]).await?;
    let fit = h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;
    // The owner's noVNC resizes the screen to the phone turned sideways, then a laptop.
    h.xvnc.resize_from_viewer(PHONE_TURNED);
    h.fit_when(fit.id, |fit| fit.screen == PHONE_TURNED).await?;
    assert_eq!(h.browser.metrics(3), phone(PHONE_TURNED));
    h.xvnc.resize_from_viewer(LAPTOP);
    let followed = h
        .fit_when(fit.id, |fit| fit.mode == FitMode::Desktop)
        .await?;
    assert_eq!(followed.screen, LAPTOP);
    assert_eq!(h.browser.window(), "maximized");
    assert_eq!(h.browser.metrics(3), natural(LAPTOP));
    assert_eq!(
        h.xvnc.resizes(),
        ["390x844 id 7 flags 0"],
        "following never resizes the screen itself"
    );
    Ok(())
}

#[tokio::test]
async fn a_tab_that_closes_gives_the_screen_back() -> TestResult {
    let h = harness(&[3]).await?;
    let fit = h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;
    h.browser.close_tab(3);
    h.fits.tab_closed(TabId(3)).await;
    assert!(matches!(
        h.fits.view(fit.id).map(|fit| fit.state),
        Some(FitState::Restored(_))
    ));
    assert_eq!(h.xvnc.size(), HOME);
    assert_eq!(h.browser.window(), "maximized");
    Ok(())
}

#[tokio::test]
async fn a_screen_that_refuses_the_size_fails_the_fit_and_leaves_nothing_claimed() -> TestResult {
    let h = harness(&[3]).await?;
    h.xvnc.refuse_next(3);
    assert!(matches!(
        h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await,
        Err(PartError::Screen(_))
    ));
    assert_eq!(h.browser.window(), "maximized");
    assert_eq!(h.browser.metrics(3), natural(HOME));
    // The failed claim left no window home behind: the next fit records its own.
    let fit = h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;
    h.fits.restore(fit.id).await?;
    assert_eq!(h.browser.window(), "maximized");
    assert_eq!(h.xvnc.size(), HOME);
    Ok(())
}

#[tokio::test]
async fn unusable_viewers_and_closed_tabs_fit_nothing() -> TestResult {
    let h = harness(&[3]).await?;
    let tiny = ScreenSize {
        width: 99,
        height: 844,
    };
    assert!(matches!(
        Viewer::new(tiny, None),
        Err(PartError::UnusableViewer(size)) if size == tiny
    ));
    assert!(matches!(
        h.fits.fit(TabId(9), Viewer::new(PHONE, None)?).await,
        Err(PartError::TabNotFound(TabId(9)))
    ));
    assert!(h.xvnc.resizes().is_empty());
    Ok(())
}

#[tokio::test]
async fn the_owner_view_reads_keyboard_boxes_and_the_current_browser_bar() -> TestResult {
    let h = harness(&[3]).await?;
    h.browser.navigate(
        3,
        "https://shop.example/search?q=typed-value",
        "form_submit",
    );
    h.browser.focus_fields(&[[16.0, 40.0, 200.0, 30.0]]);
    let fit = h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;
    let read = h.fit_when(fit.id, |fit| fit.keyboard.is_some()).await?;
    let keyboard = read.keyboard.ok_or("boxes")?;
    assert_eq!(keyboard.screen, PHONE);
    assert_eq!(keyboard.boxes, [[16.0, 40.0, 200.0, 30.0]]);
    assert_eq!(
        read.bar,
        Some(BrowserBar {
            url: "https://shop.example/search?q=typed-value".to_string(),
            back: true,
            forward: false,
        })
    );

    // On a desktop the browser shows its own toolbar: no bar.
    h.fits.fit(TabId(3), Viewer::new(LAPTOP, None)?).await?;
    let desktop = h
        .fit_when(fit.id, |fit| {
            fit.mode == FitMode::Desktop && fit.keyboard.is_some()
        })
        .await?;
    assert_eq!(desktop.bar, None);
    assert_eq!(
        desktop.keyboard.map(|keyboard| keyboard.screen),
        Some(LAPTOP)
    );
    Ok(())
}

#[tokio::test]
async fn on_a_phone_a_page_the_tab_opens_moves_into_it_unless_it_cannot_load_again() -> TestResult {
    let h = harness(&[3]).await?;
    // Open before the owner arrived: it stays where it is.
    h.browser.open_popup(3, 5, "https://kept.example/", "link");
    let fit = h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;
    let first = h.fit_when(fit.id, |fit| !fit.popups.is_empty()).await?;
    assert_eq!(first.popups, [TargetId("target-5".into())]);

    h.browser.open_popup(3, 6, "https://link.example/", "link");
    h.browser
        .open_popup(3, 7, "https://form.example/", "form_submit");
    h.fit_when(fit.id, |fit| {
        fit.popups == [TargetId("target-5".into()), TargetId("target-7".into())]
    })
    .await?;
    assert!(!h.browser.is_open(6), "the link's page closed");
    assert_eq!(
        h.browser.url(3).as_deref(),
        Some("https://link.example/"),
        "and loaded in the fitted tab"
    );
    assert_eq!(
        moves(&h.browser),
        [
            "Page.navigate https://link.example/ after 1",
            "Browser.closeTab tab 6 after 1"
        ],
        "the fitted tab loads the page before its popup closes"
    );
    assert!(h.browser.is_open(7), "a form's response stays");
    Ok(())
}

#[tokio::test]
async fn a_page_the_fitted_tab_cannot_load_stays_where_it_is() -> TestResult {
    let h = harness(&[3]).await?;
    h.browser.unreachable("https://down.example/");
    let fit = h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;
    h.first_read(fit.id).await?;

    h.browser.open_popup(3, 6, "https://down.example/", "link");
    h.fit_when(fit.id, |fit| fit.popups == [TargetId("target-6".into())])
        .await?;
    assert!(h.browser.is_open(6), "the owner keeps the page");
    assert_eq!(
        h.browser.url(3).as_deref(),
        Some("https://example.test/3"),
        "the fitted tab stays where it was"
    );
    assert_eq!(
        moves(&h.browser),
        ["Page.navigate https://down.example/ after 1"]
    );
    Ok(())
}

/// An owner view read can close popups and move the fitted tab; once restore returns, the tab
/// is the agent's again, and nothing of the owner's may move it.
#[tokio::test]
async fn restore_lets_a_running_owner_view_land_first() -> TestResult {
    let h = harness(&[3]).await?;
    let fit = h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;
    h.first_read(fit.id).await?;
    h.browser.hold_navigations();
    h.browser.open_popup(3, 6, "https://link.example/", "link");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while h.browser.held_navigations() == 0 {
        if tokio::time::Instant::now() >= deadline {
            return Err("the owner view never pulled the page in".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let restoring = tokio::spawn({
        let (fits, browser) = (Arc::clone(&h.fits), Arc::clone(&h.browser));
        async move {
            let restored = fits.restore(fit.id).await;
            (restored, browser.url(3))
        }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    h.browser.release_navigations();
    let (restored, at_restore) = tokio::time::timeout(Duration::from_secs(10), restoring).await??;
    assert!(matches!(restored?.state, FitState::Restored(_)));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        h.browser.url(3),
        at_restore,
        "nothing moved the tab after restore returned"
    );
    Ok(())
}

#[tokio::test]
async fn the_owners_browser_bar_moves_the_fitted_tab() -> TestResult {
    let h = harness(&[3]).await?;
    let fit = h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;
    assert_eq!(
        h.fits
            .navigate(fit.id, BarAction::Open("example.org".into()))
            .await?,
        BarOutcome::Completed
    );
    assert_eq!(h.browser.url(3).as_deref(), Some("https://example.org"));
    let refused = h
        .fits
        .navigate(fit.id, BarAction::Open("javascript:alert(1)".into()))
        .await?;
    assert_eq!(refused, BarOutcome::Refused);
    assert!(refused.notice().is_some());
    assert_eq!(
        h.fits.navigate(fit.id, BarAction::Back).await?,
        BarOutcome::Completed
    );
    assert_eq!(h.browser.url(3).as_deref(), Some("https://example.test/3"));
    assert_eq!(
        h.fits.navigate(fit.id, BarAction::Back).await?,
        BarOutcome::Failed,
        "no history to go back to"
    );
    assert!(matches!(
        h.fits.navigate(FitId::new(), BarAction::Reload).await,
        Err(PartError::NotFound)
    ));
    Ok(())
}
