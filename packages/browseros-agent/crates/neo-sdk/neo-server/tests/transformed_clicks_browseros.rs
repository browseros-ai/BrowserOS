//! Transformed pointer geometry uses the original element and browser-mapped visible area.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::time::Duration;

use browseros_rig::{CaptchaKey, Rig, TestResult, browser_or_skip, outcome, run};
use serde_json::json;
use tokio::time;

#[tokio::test(flavor = "multi_thread")]
async fn transformed_clicks_use_the_original_position_and_mapped_area() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let rig = Rig::start(&browser, CaptchaKey::Placeholder)?;
    time::timeout(Duration::from_secs(240), transformed_clicks(&rig)).await??;
    Ok(())
}

// Each case separates local geometry from the point Chromium actually receives:
// a span retargets to its button for stability, clipping moves a centroid past an
// overlay, and scaling makes a subpixel local element occupy several root pixels.
const SCRIPT: &str = r#"
async function frame(content, style) {
  const escaped = content.replace(/&/g, '&amp;').replace(/"/g, '&quot;');
  await page.setContent(`<style>html,body{margin:0}</style><iframe style="border:0;transform-origin:0 0;${style}" srcdoc="${escaped}"></iframe><script>window.hits=[]</script>`);
  return page.frameLocator('iframe');
}
const inlineFrame = await frame(`
  <style>html,body{margin:0}</style>
  <button id="parent" style="position:relative;width:100px;height:60px;margin:0;padding:0;border:0" onclick="top.hits.push(event.target.id)">
    <span id="span" style="position:absolute;display:block;left:40px;top:10px;width:20px;height:20px">x</span>
  </button>`, 'width:200px;height:120px;transform:scale(2)');
await inlineFrame.locator('#span').click({position:{x:1,y:1}});
const inline = await page.evaluate(() => window.hits.slice());

const clippedFrame = await frame(`
  <style>html,body{margin:0}</style>
  <div id="target" style="width:1000px;height:40px" onclick="top.hits.push('target')"></div>
  <div style="position:absolute;left:490px;top:0;width:20px;height:40px;background:red"></div>
`, 'width:1000px;height:120px;transform:translateX(-400px)');
await clippedFrame.locator('#target').click();
const clipped = await page.evaluate(() => window.hits.slice());

const tinyFrame = await frame(`
  <style>html,body{margin:0}</style>
  <button id="small" style="width:0.5px;height:0.5px;border:0;padding:0" onclick="top.hits.push('small')"></button>
`, 'width:200px;height:80px;transform:scale(4)');
await tinyFrame.locator('#small').click();
await tinyFrame.locator('#small').click({force:true});
return {inline, clipped, tiny:await page.evaluate(() => window.hits.slice())};
"#;

async fn transformed_clicks(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let client = rig.client().await?;
    let result = run(&client, SCRIPT, &json!({ "taskId": "transformed-clicks" })).await?;
    client.cancel().await?;
    assert_eq!(
        outcome(&result)?.get("status"),
        Some(&json!("ok")),
        "{result:?}"
    );
    let value = result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"));
    assert_eq!(
        value,
        Some(&json!({
            "inline": ["span"], "clipped": ["target"], "tiny": ["small", "small"]
        })),
        "the actual trusted clicks must reach the mapped targets: {result:?}"
    );
    Ok(())
}
