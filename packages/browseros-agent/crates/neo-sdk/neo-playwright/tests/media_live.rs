//! Page media across field patches, renderer swaps, OOPIFs and run teardown.

#[path = "support/fixture_server.rs"]
mod fixture_server;
#[path = "support/js_runner.rs"]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "media tests use the shared host without input and capture helpers"
)]
mod live_run;

use std::sync::Arc;

use fixture_server::{FixtureServer, nest_pages};
use js_runner::OpHost;
use live_run::{LiveRun, TestResult, brand};
use serde_json::{Value, json};

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn media_reaches_existing_and_replacement_frame_renderers() -> TestResult {
    let mut pages = nest_pages();
    let startup = "<script>window.mediaStartup = [matchMedia('print').matches, matchMedia('(prefers-color-scheme: dark)').matches, matchMedia('(prefers-reduced-motion: reduce)').matches, matchMedia('(forced-colors: active)').matches, matchMedia('(prefers-contrast: more)').matches]</script>";
    for page in pages.values_mut() {
        if page.content_type.starts_with("text/html") {
            page.body = page.body.replacen("<head>", &format!("<head>{startup}"), 1);
        }
    }
    let server = FixtureServer::start(pages).await?;
    let host: Arc<dyn OpHost> = Arc::new(LiveRun::connect().await?);
    let script = format!(
        r"
      try {{
        await page.goto({first});
        await page.emulateMedia({{media:'print',colorScheme:'dark',reducedMotion:'reduce',forcedColors:'active',contrast:'more'}});
        const read = async () => {{
          const frames = page.frames();
          const seen = [];
          for (const frame of frames) {{
            seen.push(await frame.locator('body').evaluate(() => [
              matchMedia('print').matches, matchMedia('(prefers-color-scheme: dark)').matches,
              matchMedia('(prefers-reduced-motion: reduce)').matches,
              matchMedia('(forced-colors: active)').matches, matchMedia('(prefers-contrast: more)').matches
            ]));
          }}
          return seen;
        }};
        const before = await read();
        await page.goto({second});
        const after = await read();
        const startup = [];
        for (const frame of page.frames()) startup.push(await frame.locator('body').evaluate(() => window.mediaStartup));
        return {{before,after,startup}};
      }} finally {{ await page.close(); }}
    ",
        first = json!(server.url("127.0.0.1", "/nest/top.html")),
        second = json!(server.url("localhost", "/nest/top.html"))
    );
    let answer = js_runner::run_script(&host, &json!({"page":null,"pages":[]}), &script).await?;
    for phase in ["before", "after", "startup"] {
        let values = answer
            .get(phase)
            .and_then(Value::as_array)
            .ok_or("the phase returns every frame")?;
        assert!(
            values.len() >= 8,
            "nested fixtures must exercise OOPIFs and their descendants"
        );
        for frame in values {
            assert_eq!(
                frame,
                &json!([true, true, true, true, true]),
                "all renderers get all media features: {phase}"
            );
        }
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn media_is_page_state_across_runs_and_null_only_clears_its_field() -> TestResult {
    let mut run = LiveRun::connect().await?;
    let page = run.new_page().await?;
    let other = run.new_page().await?;
    let result = async {
        run.invoke("page.emulateMedia", json!({"page":brand(&page),"media":"print","colorScheme":"dark"})).await?;
        run.next_run();
        run.invoke("page.emulateMedia", json!({"page":brand(&page),"reducedMotion":"reduce"})).await?;
        let expression = "[matchMedia('print').matches,matchMedia('(prefers-color-scheme: dark)').matches,matchMedia('(prefers-reduced-motion: reduce)').matches]";
        assert_eq!(run.eval(&page, expression).await?, json!([true,true,true]), "later runs inherit page settings and field patches retain earlier overrides");
        assert_eq!(run.eval(&other, expression).await?, json!([false,false,false]), "another tab must retain its own native preferences");
        run.invoke("page.emulateMedia", json!({"page":brand(&page),"media":"no-override"})).await?;
        assert_eq!(run.eval(&page, expression).await?, json!([false,true,true]), "clearing media preserves both preference overrides");
        Ok(())
    }.await;
    run.close_page(&page).await;
    run.close_page(&other).await;
    result
}
