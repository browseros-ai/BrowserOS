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

use browseros_core::ProtocolSession;
use js_runner::OpHost;
use live_run::{LiveRun, TestResult, brand};
use serde_json::{Value, json};

/// Browser fixture: compare joined rows with small native captures of a changing row pattern.
const TALL_PNG_PROGRAM: &str = r#"
                await page.setContent('<style>html,body{margin:0;background:transparent}</style><div style="height:10000px;background:red"></div><div style="width:2px;height:12000px;background:blue"></div><div style="position:absolute;left:200px;top:0;width:2px;height:22000px;background:repeating-linear-gradient(red 0px,blue 20px,red 40px)"></div>');
                const viewport = await page.evaluate(() => [innerWidth,innerHeight,devicePixelRatio,scrollX,scrollY]);
                async function sample(scale, ys, joins) {
                    const bytes = await page.screenshot({ fullPage: true, omitBackground: true, scale, timeout: 60000 });
                    const url = 'data:image/png;base64,' + Buffer.from(bytes).toString('base64');
                    const density = scale === 'css' ? 1 : viewport[2];
                    const native = [];
                    for (const join of joins) {
                        // Keep the reference on an integer CSS/raster grid and sample inside
                        // its bounds; fractional clip origins can change gradient rasterization.
                        const y=Math.floor((join-3)/(density*4))*4;
                        const bytes = await page.screenshot({fullPage:true,omitBackground:true,scale,clip:{x:200,y,width:4,height:16},timeout:60000});
                        native.push({join,startRow:Math.round(y*density),url:'data:image/png;base64,'+Buffer.from(bytes).toString('base64')});
                    }
                    return await page.evaluate(async ({url, ys, native, density}) => {
                        const image = new Image();
                        await new Promise((resolve, reject) => { image.onload = resolve; image.onerror = reject; image.src = url; });
                        const canvas = document.createElement('canvas');
                        canvas.width = image.width; canvas.height = 1;
                        const context = canvas.getContext('2d');
                        const pixels = ys.map(y => {
                            context.clearRect(0,0,canvas.width,1);
                            context.drawImage(image,0,y,image.width,1,0,0,image.width,1);
                            return [Array.from(context.getImageData(0,0,1,1).data), Array.from(context.getImageData(10,0,1,1).data)];
                        });
                        const point = (source,x,y) => {
                            context.clearRect(0,0,canvas.width,1);
                            context.drawImage(source,x,y,1,1,0,0,1,1);
                            return Array.from(context.getImageData(0,0,1,1).data);
                        };
                        let checkedRows=0, joinsMatch=true; const mismatches=[];
                        for (const {url,join,startRow} of native) {
                            const reference = new Image();
                            await new Promise((resolve,reject) => { reference.onload=resolve;reference.onerror=reject;reference.src=url; });
                            for (let row=0;row<6;row++) {
                                const joined=point(image,Math.round(200*density)+1,join-3+row);
                                const expected=point(reference,1,join-3+row-startRow);
                                // Independent paints can round a gradient channel by one level. Adjacent
                                // rows in this ramp differ by at least six levels at these joins.
                                const same=joined.every((value,index) => index===3 ? value===expected[index] : Math.abs(value-expected[index])<=1);
                                if (!same && mismatches.length<6) mismatches.push({join,row,joined,expected});
                                joinsMatch &&= same;
                                checkedRows++;
                            }
                        }
                        return { size: [image.width,image.height], pixels, checkedRows, joinsMatch, mismatches };
                    }, {url,ys,native,density});
                }
                const device = await sample('device', [3123,3124,10415,10416,14999,15000,19999,20000,22000*viewport[2]-1], viewport[2] === 1.5 ? [10374,20748,31122] : [3124,6248,9372]);
                const css = await sample('css', [9999,10000,15624,15625,21999], viewport[2] === 1.5 ? [15564] : [6250,12500,18750]);
                return { device,css,viewport,after: await page.evaluate(() => [innerWidth,innerHeight,devicePixelRatio,scrollX,scrollY]) };
            "#;

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

/// Tall background tabs must retain their density, transparency and host foreground state.
#[tokio::test]
#[ignore = "needs an owned BrowserOS on BROWSEROS_CDP_PORT"]
async fn tall_pngs_preserve_rows_without_activating_or_resizing_the_tab() -> TestResult {
    let run = Arc::new(LiveRun::connect().await?);
    let page = run.new_page().await?;
    let session = run
        .browser()
        .pages
        .get_session(run.core_page(&page).await?)
        .await?
        .session;
    let root = ProtocolSession::root(run.browser().connection());
    let active: Value = root.send("Browser.getActiveTab", json!({})).await?;
    let measured = async {
        let mut results = Vec::new();
        for (width, density) in [(514, 1.5), (1280, 2.0)] {
            session.send::<_, Value>("Emulation.setDeviceMetricsOverride", json!({
                "width": width, "height": 600, "deviceScaleFactor": density, "mobile": false,
            })).await?;
            let host: Arc<dyn OpHost> = Arc::clone(&run) as Arc<dyn OpHost>;
            let value =
                js_runner::run_script(&host, &json!({ "page": brand(&page) }), TALL_PNG_PROGRAM)
                    .await?;
            results.push(value);
        }
        Ok::<_, Box<dyn Error>>(results)
    }
    .await;
    let after_active: Value = root.send("Browser.getActiveTab", json!({})).await?;
    let _reset = session
        .send::<_, Value>("Emulation.clearDeviceMetricsOverride", json!({}))
        .await;
    run.close_page(&page).await;
    let red = json!([[255, 0, 0, 255], [255, 0, 0, 255]]);
    let blue = json!([[0, 0, 255, 255], [0, 0, 0, 0]]);
    let expected = [(514, 1.5, 771, 33000), (1280, 2.0, 2560, 44000)].map(
        |(width, density, device_width, device_height)| {
            let pixels: Vec<_> = [
                3123.0,
                3124.0,
                10415.0,
                10416.0,
                14999.0,
                15000.0,
                19999.0,
                20000.0,
                22000.0 * density - 1.0,
            ]
            .into_iter()
            .map(|y| {
                if y < 10000.0 * density {
                    red.clone()
                } else {
                    blue.clone()
                }
            })
            .collect();
            let density_value = if density < 2.0 { json!(1.5) } else { json!(2) };
            json!({
                "device": { "size": [device_width,device_height], "pixels": pixels, "checkedRows": 18, "joinsMatch": true, "mismatches": [] },
                "css": { "size": [width,22000], "pixels": [red,blue,blue,blue,blue], "checkedRows": if density < 2.0 {6} else {18}, "joinsMatch": true, "mismatches": [] },
                "viewport": [width,600,density_value,0,0], "after": [width,600,density_value,0,0],
            })
        },
    );
    assert_eq!(measured?, expected);
    assert_eq!(
        active, after_active,
        "captures must not activate their native tab"
    );
    Ok(())
}
