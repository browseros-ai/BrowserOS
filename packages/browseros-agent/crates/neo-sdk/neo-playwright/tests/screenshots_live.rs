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

/// Pixel alpha proves the override reaches the encoder; the next image proves it is reset.
#[tokio::test]
#[ignore = "needs an owned BrowserOS on BROWSEROS_CDP_PORT"]
async fn encodings_preserve_alpha_and_restore_the_default_background() -> TestResult {
    let run = Arc::new(LiveRun::connect().await?);
    let page = run.new_page().await?;
    let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
    let measured = js_runner::run_script(&host, &json!({ "page": brand(&page) }), r#"
        await page.setContent('<style>html,body{margin:0;background:transparent}</style><div id=hidden style="display:none">hidden</div>');
        const transparent = await page.screenshot({ omitBackground: true });
        const opaque = await page.screenshot();
        const jpeg = await page.screenshot({ type: 'jpeg', omitBackground: true });
        const webp = await page.screenshot({ type: 'webp' });
        const webpTransparent = await page.screenshot({ type: 'webp', omitBackground: true });
        const webpLossy = await page.screenshot({ type: 'webp', quality: 80 });
        let cancelled = false;
        try { await page.locator('#hidden').screenshot({ omitBackground: true, timeout: 100 }); }
        catch { cancelled = true; }
        const afterTimeout = await page.screenshot();
        const images = { transparent, opaque, jpeg, webpTransparent, afterTimeout };
        const urls = {};
        for (const [name, bytes] of Object.entries(images))
            urls[name] = 'data:image/' + (name === 'jpeg' ? 'jpeg' : name === 'webpTransparent' ? 'webp' : 'png') + ';base64,' + Buffer.from(bytes).toString('base64');
        const pixels = await page.evaluate(async urls => {
            const pixels = {};
            for (const [name, url] of Object.entries(urls)) {
                const image = new Image();
                await new Promise((resolve, reject) => { image.onload = resolve; image.onerror = reject; image.src = url; });
                const canvas = document.createElement('canvas');
                canvas.width = canvas.height = 1;
                const context = canvas.getContext('2d');
                context.drawImage(image, 0, 0);
                pixels[name] = Array.from(context.getImageData(0, 0, 1, 1).data);
            }
            return pixels;
        }, urls);
        // Chromium may put VP8X and a color profile before the actual image chunk.
        function lossless(bytes) {
            for (let offset = 12; offset + 8 <= bytes.length;) {
                const kind = String.fromCharCode(...bytes.slice(offset, offset + 4));
                if (kind === 'VP8L') return true;
                if (kind === 'VP8 ') return false;
                const size = bytes[offset + 4] + bytes[offset + 5] * 256 + bytes[offset + 6] * 65536 + bytes[offset + 7] * 16777216;
                offset += 8 + size + size % 2;
            }
            throw new Error('WebP image chunk missing');
        }
        return { pixels, cancelled, defaultWebp: lossless(webp), lossyWebp: lossless(webpLossy) };
    "#).await;
    run.close_page(&page).await;
    assert_eq!(
        measured?,
        json!({
            "pixels": { "transparent": [0,0,0,0], "opaque": [255,255,255,255], "jpeg": [255,255,255,255],
                "webpTransparent": [0,0,0,0], "afterTimeout": [255,255,255,255] },
            "cancelled": true, "defaultWebp": true, "lossyWebp": false,
        })
    );
    Ok(())
}
