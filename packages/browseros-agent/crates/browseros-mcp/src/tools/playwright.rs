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

const DESCRIPTION: &str = r#"The primary way to drive BrowserOS neo: standard Playwright JavaScript, no imports. Use top-level await to navigate, act, wait, assert, and extract in one call. Globals include browser, context, page, expect, neo, console, setTimeout, clearTimeout, and sleep(ms). console.log is captured; return a JSON value to read it back. Results are { ok, value?, logs, error? }; script exceptions come back as error results with their name and call log.

Do the whole task in as few calls as possible. Use Promise.all for independent pages; keep steps on the same page sequential. Each call has a default and hard cap of 30000 ms, including waits. Larger timeout values are clamped. Actions and navigation default to 10000 ms; expect retries for 5000 ms. Per-operation timeouts are clamped to the remaining call budget. Split longer work into bounded chunks. Keep to about 5 fresh pages per call unless the user asks for more. Wait for an element, URL, or assertion instead of looping with fixed pauses.

Pages and identity:
  context is your agent session, using the already signed-in profile.
  context.newPage() -> a new background Page, claimed and grouped before it resolves. It never steals focus.
  context.pages() -> your conversation's own tabs only.
  page -> lazy: your most recently used own tab that is still open, else a new background tab on first use. await page.goto(url) needs no setup.
  neo.pages({ ownership: 'all' }) -> [{ pageId, url, title, ownership, ownerLabel }] for everyone's tabs. Other scopes: 'mine', 'user', 'other-agent'.
  neo.page(pageId) -> a Page for any tab. Ownership is a label; use of another owner's tab is reported, never refused. Leave it as you found it unless the user asked you to change it.
  Carry the URL or page.pageId between calls; re-derive with context.pages() or neo.page(id). Confirm the tab is still open and is the intended page. Prefer your own tab for exploration. Leave useful pages open for the user.

COVERED
  browser: contexts newContext newPage close version
  context: pages newPage setDefaultTimeout setDefaultNavigationTimeout on('page', h) waitForEvent('page') close
  page: goto reload goBack goForward waitForLoadState waitForURL waitForFunction title url content evaluate screenshot pdf close frames
        waitForEvent('download'|'dialog'|'popup'|'response'|'request')
  keyboard: press type insertText; mouse: click move wheel
  locators: locator getByRole getByText getByLabel getByPlaceholder getByAltText getByTitle getByTestId
            filter({ hasText, has, hasNot, hasNotText }) nth first last and or chaining frameLocator
  actions: click dblclick hover fill type press check uncheck selectOption setInputFiles focus blur clear scrollIntoViewIfNeeded dragTo waitFor
  queries: count textContent innerText innerHTML inputValue getAttribute isVisible isHidden isEnabled isChecked isEditable boundingBox allTextContents allInnerTexts ariaSnapshot evaluate evaluateAll
  expect: toBeVisible, toBeHidden, toBeAttached, toBeEnabled, toBeDisabled, toBeChecked, toBeEditable, toBeEmpty, toBeFocused, toHaveText, toContainText, toHaveValue, toHaveCount, toHaveAttribute, toHaveClass, toHaveId, toHaveCSS, toHaveTitle, toHaveURL, .not
  value expect: toBe, toEqual, toContain, toBeTruthy, toBeGreaterThan, toMatch, .not
  neo: pages page snapshot read grep download cdp
       neo.snapshot(page) -> { text, refs }; neo.read(page) -> markdown; neo.grep(page, opts); neo.download(page, opts); neo.cdp(method, params?, page?).

DIFFERS
  context is the agent session. context.pages() is scoped to your tabs; neo.pages({ ownership: 'all' }) lists everyone's.
  browser.contexts() returns [context]. browser.newContext() returns context plus a warning: one signed-in profile, no isolation. browser.newPage() equals context.newPage().
  browser.close(), context.close(), and page.bringToFront() warn and do nothing. Use page.close() to close an individual tab. Agents never steal focus.
  page.evaluate(fn, arg) sends String(fn); arg and the result are JSON. DOM and site code run there, not in the script's server runtime.
  Actions/navigation default to 10 s, expect to 5 s, inside the 30 s call cap. context.setDefaultTimeout, context.setDefaultNavigationTimeout, and per-call timeout cannot extend that cap.
  test(name, fn), chromium.launch(), and chromium.connectOverCDP() use this same browser for pasted test bodies. Write directly against context and page; no launch or connection is needed.
  No saved helpers or helpersAvailable here. Use run for saved helpers and the legacy browser.cdp escape hatch.

NOT AVAILABLE
  route/unroute, request (APIRequestContext), cookies/addCookies/storageState ("you are already signed in"), addInitScript, setViewportSize, emulateMedia, tracing, video, exposeFunction, page.pause.
  These throw Error: not available in BrowserOS neo: <api>. <hint>.
  require, import, fetch, process, fs fail with a one-line hint. The runtime has no window or document; use page.evaluate(fn, arg) for page code or the site's API. Use the signed-in profile and normal site UI for browser flows. Use neo.snapshot, screenshots, console.log, and cockpit audit/replay to inspect progress.

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
                .is_some_and(|error| error.contains("not implemented yet: page.goto"))
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
                assert!(record.is_error);
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
            ("context.close", None),
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
            const errors = [];
            for (const method of {}) {{
                try {{ await __browserosCall(method, '[7]', false); }}
                catch (error) {{ errors.push(error.message); }}
            }}
            return errors;
        "#,
            serde_json::to_string(&names)?
        );
        let result = execute_tool(&definition(), json!({"code": code}), &ctx).await?;
        assert!(!result.is_error);
        let output = result
            .structured_content
            .ok_or_else(|| anyhow::anyhow!("missing output"))?;
        // Every leaf errors against the fake browser (stub or real), so the
        // value is one error message per method. The exact text belongs to the
        // leaf pieces and their own tests; this test pins routing and audit.
        let errors = output["value"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("errors are not an array"))?;
        assert_eq!(errors.len(), names.len());
        for (method, error) in names.iter().zip(errors) {
            let text = error.as_str().unwrap_or_default();
            assert!(!text.is_empty(), "{method} produced an empty error");
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
