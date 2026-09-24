//! Playwright scripts enter the same bounded QuickJS runtime as the run SDK.

use super::run::{
    RunError, RunOutcome, RunOutput, ScriptSpec, default_timeout, execute_script,
    normalized_timeout_ms,
};
use crate::{
    framework::{ToolCtx, ToolError, ToolExecResult, ToolResult, parse_args},
    pw,
};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

const DESCRIPTION: &str = r#"Drive BrowserOS neo with standard Playwright JavaScript, no imports. Use top-level await and return a JSON value. Globals: browser, context, page, expect, neo, console, setTimeout, clearTimeout, sleep(ms); test and chromium support pasted test bodies. Results are { ok, value?, logs, error? }; console.log is captured and errors include the failing locator and call log.

Pages and identity:
  context is your session in the user's signed-in browser. context.newPage() opens and groups your background tab; context.pages() lists your tabs.
  page is lazy: your most recently used own open tab, or a new tab on first use. No browser launch or connection is needed.
  neo.pages({ ownership: 'all' }) lists { pageId, url, title, ownership, ownerLabel } for everyone's tabs; scopes also include 'mine', 'user', 'other-agent'. neo.page(pageId) selects a tab.
  Carry URLs or page.pageId between calls and confirm the intended tab. Leave other people's tabs as found unless asked to change them. Prefer your own tabs for exploration and leave useful pages open.
  Tabs never steal focus. browser.newContext() returns the same context with a warning; browser.close(), context.close(), and page.bringToFront() warn and do nothing. Use page.close() for one tab.

Use standard locators (getByRole, getByLabel, locator, filter, nth, frameLocator), actions (click, fill, press, selectOption, setInputFiles, dragTo), and expect assertions including .not. Navigation, keyboard/mouse, dialog/popup/download events, waitForRequest/waitForResponse, JSON response bodies, evaluate/evaluateAll, and screenshot bytes are supported.
page.evaluate(fn, arg) sends the function's source to the page with JSON arguments/results. The script itself has no DOM or Node globals.
Extras: neo.read(page) -> markdown; neo.grep(page, opts); neo.snapshot(page) -> { text, refs }; neo.download(page, opts); neo.cdp(method, params?, page?).

One bounded chunk per call, with a default and hard cap of 30000 ms including waits. Actions/navigation default to 10000 ms; expect retries for 5000 ms. All timeouts are clamped to the remaining call budget. Wait on elements, URLs, or assertions instead of the clock. Use Promise.all for independent pages (about 5 at a time); keep each page's steps sequential.

NOT AVAILABLE
  route/unroute, request (APIRequestContext), cookies/addCookies/storageState, addInitScript, setViewportSize, emulateMedia, tracing, video, exposeFunction, page.pause.
  Unsupported methods throw Error: not available in BrowserOS neo: <api>. <hint>.
  No require, import, fetch, process, fs, window or document in the script. Use page.evaluate(fn, arg) for page code or site APIs and the signed-in profile for browser flows. Inspect progress through screenshots, console.log and the cockpit.

Examples (code bodies, no imports):
1. Navigate, act, assert, extract:
  await page.goto('https://news.ycombinator.com');
  await page.getByRole('link', { name: 'new', exact: true }).click();
  await expect(page).toHaveURL(/newest/);
  return (await page.locator('.titleline > a').allInnerTexts()).slice(0, 10);

2. Fan out over independent pages:
  const urls = ['https://example.com', 'https://example.org'];
  return await Promise.all(urls.map(async url => {
    const p = await context.newPage();
    await p.goto(url, { waitUntil: 'domcontentloaded' });
    await expect(p.getByRole('heading', { name: 'Example Domain' })).toBeVisible();
    return { pageId: p.pageId, url: await p.url(), text: await neo.read(p) };
  }));

3. A missing button fails with the locator and call log:
  await page.goto('https://example.com');
  await page.getByRole('button', { name: 'Submit' }).click({ timeout: 3000 });
  Result shape:
  { "ok": false, "logs": [], "error": "TimeoutError: locator.click: Timeout 3000ms exceeded.\nCall log:\n  - waiting for getByRole('button', { name: 'Submit' })" }
  Read the error and logs. Fix the locator or page state before retrying. A call that reaches 30 s needs smaller chunks, not a larger timeout."#;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlaywrightArgs {
    /// Standard Playwright JavaScript body, no imports. Top-level await; `return` a JSON value.
    code: String,
    /// Max run time in ms. Default and hard cap 30000; larger values are clamped.
    #[serde(default = "default_timeout")]
    timeout: f64,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def_with_output::<PlaywrightArgs, RunOutput>(
        "playwright",
        DESCRIPTION,
        Some(super::open_world_annotations()),
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: PlaywrightArgs = parse_args(raw)?;
        let outcome = match execute_script(
            ScriptSpec {
                bootstrap_js: pw::FACADE_JS,
                code: args.code,
                timeout_ms: normalized_timeout_ms(args.timeout),
                helpers: false,
            },
            ctx,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(RunError::Syntax(message)) => {
                RunOutcome::failure(format!("playwright: syntax error - {message}"), Vec::new())
            }
            Err(RunError::Cancelled) => return Err(ToolError::Cancelled),
            Err(RunError::Engine(message)) => return Err(ToolError::message(message)),
        };
        Ok(Some(outcome.into_tool_result()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{framework::execute_tool, tools::run::tests::test_ctx};
    use serde_json::json;

    #[tokio::test]
    async fn playwright_exposes_globals_and_reports_stub_operations() -> anyhow::Result<()> {
        let ctx = test_ctx();
        let result = execute_tool(
            &definition(),
            json!({"code": "return typeof page + typeof expect"}),
            &ctx,
        )
        .await?;
        assert!(!result.is_error);
        assert_eq!(
            result.structured_content,
            Some(json!({"ok": true, "value": "objectfunction", "logs": []}))
        );

        let result = execute_tool(
            &definition(),
            json!({"code": "await page.goto('https://example.com')"}),
            &ctx,
        )
        .await?;
        assert!(result.is_error);
        let output = result
            .structured_content
            .ok_or_else(|| anyhow::anyhow!("missing script output"))?;
        assert_eq!(output["ok"], false);
        assert!(
            output["error"]
                .as_str()
                .is_some_and(|error| error.contains("page.goto"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn playwright_does_not_load_legacy_helpers() -> anyhow::Result<()> {
        let mut ctx = test_ctx();
        ctx.preloaded_helpers.push(crate::framework::HelperSource {
            name: "legacy".to_string(),
            source: "(() => { throw new Error('legacy helper evaluated'); })()".to_string(),
        });
        let result = execute_tool(
            &definition(),
            json!({"code": "return typeof helpers"}),
            &ctx,
        )
        .await?;
        assert_eq!(
            result.structured_content,
            Some(json!({"ok": true, "value": "undefined", "logs": []}))
        );
        Ok(())
    }

    #[derive(Default)]
    struct RecordingHook {
        authorized: std::sync::Mutex<Vec<Option<u32>>>,
        recorded: std::sync::Mutex<Vec<(String, Option<u32>, Value)>>,
    }

    impl crate::framework::InnerCallHook for RecordingHook {
        fn authorize<'a>(&'a self, page: Option<u32>) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async move {
                self.authorized
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(page);
                Ok(())
            })
        }

        fn record<'a>(
            &'a self,
            record: crate::framework::InnerCallRecord<'a>,
        ) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                // Leaves may succeed or fail against the fake browser; only the
                // routing and attribution are pinned here.
                assert!(!record.from_helper);
                assert!(record.secrets.is_empty());
                self.recorded
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push((record.method.to_string(), record.page, record.args.clone()));
            })
        }

        fn on_page_created<'a>(&'a self, _page_id: u32) -> BoxFuture<'a, ()> {
            // Leaf implementations may claim a page against the fake browser;
            // this test only checks routing and attribution, not page effects.
            Box::pin(async {})
        }
    }

    #[tokio::test]
    async fn playwright_bridge_routes_and_audits_page_first_methods() -> anyhow::Result<()> {
        let methods = [
            ("page.goto", Some(7)),
            ("locator.click", Some(7)),
            ("expect.toBeVisible", Some(7)),
            ("keyboard.press", Some(7)),
            ("mouse.click", Some(7)),
            ("frame.info", Some(7)),
            ("neo.snapshot", Some(7)),
            ("neo.read", Some(7)),
            ("neo.grep", Some(7)),
            ("neo.download", Some(7)),
            ("neo.page", Some(7)),
            ("context.newPage", None),
            ("context.pages", None),
            ("context.close", Some(7)),
            ("neo.pages", None),
            ("neo.cdp", None),
        ];
        let hook = std::sync::Arc::new(RecordingHook::default());
        let mut ctx = test_ctx();
        ctx.inner_call_hook = Some(hook.clone());
        let names = methods
            .iter()
            .map(|(method, _)| *method)
            .collect::<Vec<_>>();
        let code = format!(
            r#"
            const outcomes = [];
            for (const method of {}) {{
                try {{ await __browserosCall(method, '[7]', false); outcomes.push('ok'); }}
                catch (error) {{ outcomes.push(error.message); }}
            }}
            return outcomes;
        "#,
            serde_json::to_string(&names)?
        );
        let result = execute_tool(&definition(), json!({"code": code}), &ctx).await?;
        assert!(!result.is_error);
        let output = result
            .structured_content
            .ok_or_else(|| anyhow::anyhow!("missing output"))?;
        // One outcome per method: 'ok' or the leaf's error text. The exact
        // text belongs to the leaf pieces and their own tests; this test pins
        // routing and audit attribution.
        let outcomes = output["value"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("outcomes are not an array"))?;
        assert_eq!(outcomes.len(), names.len());
        for (method, outcome) in names.iter().zip(outcomes) {
            let text = outcome.as_str().unwrap_or_default();
            assert!(!text.is_empty(), "{method} produced an empty outcome");
        }
        assert_eq!(
            *hook
                .authorized
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            methods.iter().map(|(_, page)| *page).collect::<Vec<_>>()
        );
        assert_eq!(
            *hook
                .recorded
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            methods
                .iter()
                .map(|(method, page)| ((*method).to_string(), *page, json!([7])))
                .collect::<Vec<_>>()
        );
        Ok(())
    }
}
