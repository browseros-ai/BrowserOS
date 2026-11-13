//! Scaled fits keep logical device metrics while their shared physical framebuffer changes.

use super::{HOME, TestResult, harness, natural, phone};
use browseros_core::TabId;
use browseros_core::viewport::{Size, ViewportMode, ViewportRequest};
use neo_takeover::{FitMode, PartError, ScreenSize, Viewer, ViewerScale};
use std::time::Duration;

const PHONE: ScreenSize = ScreenSize {
    width: 366,
    height: 700,
};
const PHYSICAL: ScreenSize = ScreenSize {
    width: 732,
    height: 1400,
};

#[tokio::test]
async fn a_scaled_phone_keeps_its_device_and_restores_the_agents_preview() -> TestResult {
    let h = harness(&[3]).await?;
    let original = ViewportRequest::new(ViewportMode::Mobile, Some(150.0))?.with_size(Size {
        width: 400,
        height: 906,
    })?;
    h.agent_sets_viewport(3, original).await?;
    let fit = h
        .fits
        .fit(TabId(3), Viewer::new(PHONE, Some(ViewerScale::Two))?)
        .await?;
    assert_eq!(fit.screen, PHYSICAL);
    assert_eq!(fit.viewer.size, PHONE);
    assert_eq!(fit.viewer.scale, ViewerScale::Two);
    assert_eq!(h.browser.metrics(3), phone(PHONE));
    assert_eq!(h.browser.window(), "fullscreen");
    let metrics = h
        .browser
        .calls()
        .into_iter()
        .rev()
        .find(|call| call.method == "Emulation.setDeviceMetricsOverride")
        .ok_or("a mobile request")?
        .params;
    assert_eq!(metrics["scale"], 2.0);
    assert_eq!(metrics["deviceScaleFactor"], 2.0);
    let restored = h.fits.restore(fit.id).await?;
    assert_eq!(h.xvnc.size(), HOME);
    assert_eq!(h.browser.window(), "maximized");
    assert_eq!(
        h.browser.metrics(3),
        phone(ScreenSize {
            width: 400,
            height: 906
        })
    );
    let metrics = h
        .browser
        .calls()
        .into_iter()
        .rev()
        .find(|call| call.method == "Emulation.setDeviceMetricsOverride")
        .ok_or("the restored mobile request")?
        .params;
    assert_eq!(
        metrics["scale"], 1.5,
        "the fit must not overwrite the agent's saved preview"
    );
    assert_eq!(h.fits.restore(fit.id).await?, restored);
    Ok(())
}

#[tokio::test]
async fn refits_inherit_scale_for_keyboard_height_and_rotation_until_explicit_one() -> TestResult {
    let h = harness(&[3]).await?;
    let fit = h
        .fits
        .fit(TabId(3), Viewer::new(PHONE, Some(ViewerScale::Two))?)
        .await?;
    for (logical, physical) in [
        (
            ScreenSize {
                width: 366,
                height: 500,
            },
            ScreenSize {
                width: 732,
                height: 1000,
            },
        ),
        (
            ScreenSize {
                width: 500,
                height: 366,
            },
            ScreenSize {
                width: 1000,
                height: 732,
            },
        ),
    ] {
        let refit = h.fits.fit(TabId(3), Viewer::new(logical, None)?).await?;
        assert_eq!(refit.id, fit.id);
        assert_eq!(refit.screen, physical);
        assert_eq!(refit.viewer.scale, ViewerScale::Two);
        assert_eq!(h.browser.metrics(3), phone(logical));
    }
    let desktop = ScreenSize {
        width: 700,
        height: 366,
    };
    let calls = h.browser.calls().len();
    let resizes = h.xvnc.resizes().len();
    let refused = h.fits.fit(TabId(3), Viewer::new(desktop, None)?).await;
    assert!(matches!(refused, Err(PartError::InvalidViewer(_))));
    assert_eq!(
        h.browser.calls().len(),
        calls,
        "reject inherited desktop 2 before browser work"
    );
    assert_eq!(h.xvnc.resizes().len(), resizes);
    let refit = h
        .fits
        .fit(TabId(3), Viewer::new(desktop, Some(ViewerScale::One))?)
        .await?;
    assert_eq!(refit.viewer.scale, ViewerScale::One);
    assert_eq!(refit.mode, FitMode::Desktop);
    assert_eq!(refit.screen, desktop);
    h.fits.restore(fit.id).await?;
    Ok(())
}

#[tokio::test]
async fn external_resizes_use_physical_pixels_and_forget_two_x_on_desktop() -> TestResult {
    let h = harness(&[3]).await?;
    let fit = h
        .fits
        .fit(TabId(3), Viewer::new(PHONE, Some(ViewerScale::Two))?)
        .await?;
    let odd = ScreenSize {
        width: 733,
        height: 1401,
    };
    h.xvnc.resize_from_viewer(odd);
    let read = h.fit_when(fit.id, |fit| fit.screen == odd).await?;
    assert_eq!(read.viewer.size, PHONE);
    assert_eq!(read.viewer.scale, ViewerScale::Two);
    assert_eq!(h.browser.metrics(3), phone(PHONE));
    let desktop = ScreenSize {
        width: 1202,
        height: 1400,
    };
    h.xvnc.resize_from_viewer(desktop);
    let read = h.fit_when(fit.id, |fit| fit.screen == desktop).await?;
    assert_eq!(read.mode, FitMode::Desktop);
    assert_eq!(read.viewer.scale, ViewerScale::One);
    h.xvnc.resize_from_viewer(PHYSICAL);
    let read = h.fit_when(fit.id, |fit| fit.screen == PHYSICAL).await?;
    assert_eq!(read.viewer.scale, ViewerScale::One);
    assert_eq!(
        read.mode,
        FitMode::Desktop,
        "the old phone scale is not suspended"
    );
    h.xvnc.resize_from_viewer(PHONE);
    let read = h.fit_when(fit.id, |fit| fit.screen == PHONE).await?;
    assert_eq!(read.viewer.scale, ViewerScale::One);
    assert_eq!(read.mode, FitMode::Mobile);
    h.fits.restore(fit.id).await?;
    Ok(())
}

#[tokio::test]
async fn multiple_fits_share_the_latest_scale_in_both_release_orders() -> TestResult {
    for first_releases_first in [true, false] {
        let h = harness(&[3, 4]).await?;
        let first = h
            .fits
            .fit(TabId(3), Viewer::new(PHONE, Some(ViewerScale::Two))?)
            .await?;
        let second = h.fits.fit(TabId(4), Viewer::new(PHONE, None)?).await?;
        assert_eq!(
            second.viewer.scale,
            ViewerScale::One,
            "a new omitted-scale fit defaults to 1"
        );
        assert_eq!(
            h.fits.view(first.id).ok_or("first fit")?.viewer.scale,
            ViewerScale::One
        );
        h.fits
            .fit(TabId(4), Viewer::new(PHONE, Some(ViewerScale::Two))?)
            .await?;
        let inherited = h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;
        assert_eq!(
            inherited.viewer.scale,
            ViewerScale::Two,
            "refits inherit the shared layout"
        );
        let (early, last, remaining) = if first_releases_first {
            (first.id, second.id, 4)
        } else {
            (second.id, first.id, 3)
        };
        h.fits.restore(early).await?;
        assert_eq!(h.xvnc.size(), PHYSICAL);
        assert_eq!(h.browser.metrics(remaining), phone(PHONE));
        h.fits.restore(last).await?;
        assert_eq!(h.xvnc.size(), HOME);
        assert_eq!(h.browser.window(), "maximized");
        assert_eq!(h.browser.metrics(remaining), natural(HOME));
    }
    Ok(())
}

#[tokio::test]
async fn a_failed_refit_keeps_the_previous_shared_scale() -> TestResult {
    let h = harness(&[3]).await?;
    let fit = h
        .fits
        .fit(TabId(3), Viewer::new(PHONE, Some(ViewerScale::Two))?)
        .await?;
    h.xvnc.refuse_next(3);
    let refused = h
        .fits
        .fit(TabId(3), Viewer::new(PHONE, Some(ViewerScale::One))?)
        .await;
    assert!(matches!(refused, Err(PartError::Screen(_))));
    assert_eq!(h.xvnc.size(), PHYSICAL);
    let unchanged = h.fits.view(fit.id).ok_or("the previous fit")?;
    assert_eq!(unchanged.screen, fit.screen);
    assert_eq!(unchanged.viewer, fit.viewer);
    assert_eq!(unchanged.mode, fit.mode);
    assert_eq!(h.browser.metrics(3), phone(PHONE));
    let refit = h.fits.fit(TabId(3), Viewer::new(PHONE, None)?).await?;
    assert_eq!(refit.viewer.scale, ViewerScale::Two);
    h.fits.restore(fit.id).await?;
    Ok(())
}

#[tokio::test]
async fn keyboard_boxes_are_physical_and_equal_screen_refits_discard_old_readings() -> TestResult {
    let h = harness(&[3]).await?;
    let logical = ScreenSize {
        width: 300,
        height: 500,
    };
    let physical = ScreenSize {
        width: 600,
        height: 1000,
    };
    h.browser.focus_fields(&[[16.0, 40.0, 200.0, 30.0]]);
    let fit = h
        .fits
        .fit(TabId(3), Viewer::new(logical, Some(ViewerScale::Two))?)
        .await?;
    let read = h.fit_when(fit.id, |fit| fit.keyboard.is_some()).await?;
    let keyboard = read.keyboard.ok_or("scaled boxes")?;
    assert_eq!(keyboard.screen, physical);
    assert_eq!(keyboard.boxes, [[32.0, 80.0, 400.0, 60.0]]);

    h.browser.hold("owner-view-reply");
    wait_until(|| h.browser.held("owner-view-reply") > 0).await?;
    h.fits
        .fit(TabId(3), Viewer::new(physical, Some(ViewerScale::One))?)
        .await?;
    h.browser.focus_fields(&[[9.0, 12.0, 40.0, 50.0]]);
    // Prevent a fresh round from hiding an incorrectly accepted old reply. The sampled
    // round already listed its pages, so it can finish while the next round waits here.
    h.browser.hold("Browser.getTabs");
    h.browser.let_go("owner-view-reply");
    wait_until(|| h.browser.held("Browser.getTabs") > 0).await?;
    assert_eq!(h.fits.view(fit.id).ok_or("the fit")?.keyboard, None);
    h.browser.let_go("Browser.getTabs");
    let read = h.fit_when(fit.id, |fit| fit.keyboard.is_some()).await?;
    assert_eq!(
        read.keyboard.ok_or("fresh boxes")?.boxes,
        [[9.0, 12.0, 40.0, 50.0]]
    );
    h.fits.restore(fit.id).await?;
    Ok(())
}

async fn wait_until(done: impl Fn() -> bool) -> TestResult {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    Ok(())
}
