//! Tag insertion through QuickJS, including same-process and out-of-process frame handles and
//! CSP rejection. Run ignored tests against a throwaway browser on `BROWSEROS_CDP_PORT`.

#[path = "support/fixture_server.rs"]
mod fixture_server;
#[path = "support/js_runner.rs"]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "tag scripts use the shared host without direct input helpers"
)]
mod live_run;

use std::sync::Arc;

use fixture_server::{FixtureServer, Page, nest_pages};
use js_runner::OpHost;
use live_run::{LiveRun, TestResult};
use serde_json::json;

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn tags_belong_to_their_frame_and_csp_errors_keep_browser_wording() -> TestResult {
    let mut pages = nest_pages();
    pages.insert(
        "/csp.html".to_owned(),
        Page::html("<html><head></head><body></body></html>").with_header(
            "Content-Security-Policy",
            "script-src 'none'; style-src 'none'",
        ),
    );
    let server = FixtureServer::start(pages).await?;
    let host: Arc<dyn OpHost> = Arc::new(LiveRun::connect().await?);
    let script = format!(
        r"
      try {{
        await page.goto({top});
        const values = [];
        for (const suffix of ['a1', 'b1', 'b3']) {{
          const frame = page.frame({{url: '**/' + suffix + '.html'}});
          const tag = await frame.addScriptTag({{content: 'window.tagValue = 37'}});
          const style = await frame.addStyleTag({{content: 'body {{ color: rgb(12, 34, 56) }}'}});
          values.push(await tag.evaluate(e => [e.tagName, e.ownerDocument.defaultView.tagValue]));
          values.push(await style.evaluate(e => [e.tagName, getComputedStyle(e.ownerDocument.body).color]));
        }}
        const untouched = await page.locator('body').evaluate(() => window.tagValue === undefined);
        await page.goto({csp});
        const failures = [];
        for (const method of ['addScriptTag', 'addStyleTag']) {{
          try {{
            await page[method]({{content: method === 'addScriptTag' ? 'window.blocked = true' : 'body {{ color: red }}'}});
            failures.push(false);
          }} catch (error) {{
            failures.push(/Content.Security.Policy/.test(error.message));
          }}
        }}
        return {{values, untouched, failures}};
      }} finally {{ await page.close(); }}
    ",
        top = json!(server.url("127.0.0.1", "/nest/top.html")),
        csp = json!(server.url("127.0.0.1", "/csp.html"))
    );
    let answer = js_runner::run_script(&host, &json!({"page": null, "pages": []}), &script).await?;
    assert_eq!(
        answer,
        json!({
            "values": [["SCRIPT",37],["STYLE","rgb(12, 34, 56)"],
                       ["SCRIPT",37],["STYLE","rgb(12, 34, 56)"],
                       ["SCRIPT",37],["STYLE","rgb(12, 34, 56)"]],
            "untouched": true, "failures": [true,true]
        }),
        "tags and handles must stay in the owning frame, and CSP blocks must reject"
    );
    Ok(())
}

/// A detached script still has an `ElementHandle` until the run ends, even after renderer GC.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn a_self_removing_tag_keeps_its_handle_after_browser_gc() -> TestResult {
    let run = LiveRun::connect().await?;
    let page = run.new_page().await?;
    let result =
        async {
            let inserted = run.invoke("page.addScriptTag", json!({
            "page": live_run::brand(&page), "content": "document.currentScript.remove()",
        })).await?;
            let target = inserted
                .get("element")
                .ok_or("insertion returns its element")?;
            let info = run
                .browser()
                .pages
                .get_info_by_tab(page.tab)
                .await
                .ok_or("the tab is open")?;
            let session = run.browser().pages.get_session(info.page_id).await?.session;
            session
                .send_value("HeapProfiler.collectGarbage", json!({}))
                .await?;
            let value = run
                .invoke(
                    "locator.evaluate",
                    json!({
                        "target": target, "strict": "strict",
                        "expression": "e => [e.tagName, e.isConnected]", "isFunction": true,
                        "arg": { "value": "{\"v\":\"undefined\"}", "handles": [] },
                    }),
                )
                .await?;
            assert_eq!(
                value,
                json!({"a":["SCRIPT",false],"id":1}),
                "the handle must retain its exact detached node across operation cleanup and GC"
            );
            Ok(())
        }
        .await;
    run.close_page(&page).await;
    result
}
