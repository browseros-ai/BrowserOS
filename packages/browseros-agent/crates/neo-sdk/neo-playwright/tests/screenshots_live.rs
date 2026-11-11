//! Encoded screenshot dimensions through the real prelude and a live browser, including a
//! high-density viewport and an element larger than it. Emulation and capture share core's
//! attached CDP session, so the test does not mix density settings from different sessions.

#[path = "support/js_runner.rs"]
#[expect(
    dead_code,
    reason = "the shared runner also serves selector and event tests"
)]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "other live suites use the shared action and policy helpers"
)]
mod live_run;

use std::error::Error;
use std::sync::Arc;

use js_runner::OpHost;
use live_run::{LiveRun, TestResult, brand};
use serde_json::{Value, json};

#[tokio::test]
#[ignore = "needs an owned BrowserOS on BROWSEROS_CDP_PORT"]
async fn encoded_dimensions_follow_element_geometry_and_css_scale() -> TestResult {
    let run = Arc::new(LiveRun::connect().await?);
    let page = run.new_page().await?;
    let session = run
        .browser()
        .pages
        .get_session(run.core_page(&page).await?)
        .await?
        .session;
    let measured = async {
        session.send::<_, Value>("Emulation.setDeviceMetricsOverride", json!({
            "width": 800, "height": 600, "deviceScaleFactor": 2, "mobile": false,
        })).await?;
        let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
        let value = js_runner::run_script(&host, &json!({ "page": brand(&page) }), r#"
            function size(bytes) {
                const n = i => (bytes[i] * 0x1000000 + bytes[i + 1] * 0x10000 + bytes[i + 2] * 0x100 + bytes[i + 3]);
                return [n(16), n(20)];
            }
            await page.setContent('<style>body{margin:0}</style><div id=box style="width:1200px;height:1000px;background:red"></div>');
            const box = page.locator('#box');
            const largeCss = size(await box.screenshot({ scale: 'css' }));
            const largeDevice = size(await box.screenshot());
            const viewport = size(await page.screenshot());
            const clipped = size(await page.screenshot({ fullPage: true, scale: 'css', clip: { x: 200, y: 300, width: 500, height: 400 } }));
            await page.setContent('<style>body{margin:0}</style><div id=box style="width:48.51px;height:19.8px;border:1px solid black"></div>');
            const fractional = size(await page.locator('#box').screenshot({ scale: 'css' }));
            return { largeCss, largeDevice, viewport, clipped, fractional };
        "#).await?;
        Ok::<_, Box<dyn Error>>(value)
    }.await;
    // The fixture owns this tab. Reset its emulation and close it even if a screenshot fails.
    let _reset = session
        .send::<_, Value>("Emulation.clearDeviceMetricsOverride", json!({}))
        .await;
    run.close_page(&page).await;
    assert_eq!(
        measured?,
        json!({
            "largeCss": [1200, 1000], "largeDevice": [2400, 2000],
            "viewport": [1600, 1200], "clipped": [500, 400], "fractional": [51, 22],
        }),
        "encoded dimensions must follow CSS geometry independently of device density"
    );
    Ok(())
}
