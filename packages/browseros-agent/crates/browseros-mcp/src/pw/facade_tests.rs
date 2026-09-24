//! Exercise the facade through the real tool/runtime/audit seam. Scripted
//! replies supply host values beyond the fake CDP connection, while each call
//! still crosses authorization and the production audit policy.
use crate::{
    framework::{BrowserToolDefaults, InnerCallHook, InnerCallRecord, ToolCtx, execute_tool},
    tools::{
        playwright,
        run::tests::{RunFakeConnection, test_ctx_for},
    },
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
struct Call {
    method: String,
    page: Option<u32>,
    args: Value,
}

#[derive(Default)]
struct MockHook(Mutex<Vec<Call>>);

impl InnerCallHook for MockHook {
    fn authorize<'a>(&'a self, _page: Option<u32>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }

    fn record<'a>(&'a self, record: InnerCallRecord<'a>) -> BoxFuture<'a, ()> {
        assert!(!record.from_helper);
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Call {
                method: record.method.to_owned(),
                page: record.page,
                args: record.args.clone(),
            });
        Box::pin(async {})
    }

    fn on_page_created<'a>(&'a self, _page_id: u32) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

fn ctx_with_hook(hook: Arc<MockHook>) -> ToolCtx {
    let mut ctx = test_ctx_for(
        Arc::new(RunFakeConnection::new()),
        BrowserToolDefaults::default(),
    );
    ctx.inner_call_hook = Some(hook);
    ctx
}

async fn run(code: &str, replies: Value) -> anyhow::Result<(Value, Vec<Call>)> {
    let hook = Arc::new(MockHook::default());
    let ctx = ctx_with_hook(hook.clone());
    // Replies replace fake-browser results after authorization/audit, never the
    // bridge itself. Missing replies retain the real dispatch outcome.
    let code = format!(
        r#"
        const nativeCall = __browserosCall;
        const replies = {replies};
        globalThis.__browserosCall = async (method, args, helper) => {{
            let value, failure;
            try {{ value = await nativeCall(method, args, helper); }} catch (error) {{ failure = error; }}
            if (Object.prototype.hasOwnProperty.call(replies, method)) {{
                const reply = replies[method];
                if (reply && reply.throw) throw new Error(reply.throw);
                return reply;
            }}
            if (method === 'neo.page') return {{pageId:JSON.parse(args)[0]}};
            if (failure) throw failure;
            return value;
        }};
        {code}
    "#
    );
    let result = execute_tool(&playwright::definition(), json!({"code": code}), &ctx).await?;
    let output = result
        .structured_content
        .ok_or_else(|| anyhow::anyhow!("missing script output"))?;
    let calls = hook
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    Ok((output, calls))
}

fn leaf(calls: &[Call], method: &str, page: Option<u32>, args: Value) {
    assert_eq!(
        calls.last(),
        Some(&Call {
            method: method.to_owned(),
            page,
            args
        })
    );
}

fn success(output: &Value) {
    assert_eq!(output["ok"], true, "{output}");
}

#[tokio::test]
async fn facade_selectors_reach_audited_leaf_with_playwright_bytes() -> anyhow::Result<()> {
    let cases = [
        (
            "p.getByRole('button', { name: 'Submit', exact: true }).click()",
            "locator.click",
            json!([7, "internal:role=button[name=\"Submit\"s]", {"timeout":10000}]),
        ),
        (
            "p.getByText('Hello').first().fill('x')",
            "locator.fill",
            json!([7, "internal:text=\"Hello\"i >> nth=0", "[redacted]", {"timeout":10000}]),
        ),
        (
            "p.locator('.item').filter({hasText:'two'}).nth(1).click()",
            "locator.click",
            json!([7, ".item >> internal:has-text=\"two\"i >> nth=1", {"timeout":10000}]),
        ),
        (
            "p.frameLocator('#f').getByLabel('Name').fill('Ada')",
            "locator.fill",
            json!([7, "#f >> internal:control=enter-frame >> internal:label=\"Name\"i", "[redacted]", {"timeout":10000}]),
        ),
        (
            "p.getByPlaceholder('Search', {exact:true}).fill('x')",
            "locator.fill",
            json!([7, "internal:attr=[placeholder=\"Search\"s]", "[redacted]", {"timeout":10000}]),
        ),
        (
            "p.getByAltText('Photo').click()",
            "locator.click",
            json!([7, "internal:attr=[alt=\"Photo\"i]", {"timeout":10000}]),
        ),
        (
            "p.getByTitle('Details').click()",
            "locator.click",
            json!([7, "internal:attr=[title=\"Details\"i]", {"timeout":10000}]),
        ),
        (
            "p.getByTestId('submit').click()",
            "locator.click",
            json!([7, "internal:testid=[data-testid=\"submit\"s]", {"timeout":10000}]),
        ),
    ];
    for (expression, method, args) in cases {
        let (output, calls) = run(
            &format!("const p = await neo.page(7); await {expression};"),
            json!({}),
        )
        .await?;
        assert_eq!(output["ok"], false, "{expression}: {output}");
        assert!(
            output["error"]
                .as_str()
                .is_some_and(|error| !error.is_empty())
        );
        leaf(&calls, method, Some(7), args);
    }
    Ok(())
}

#[tokio::test]
async fn facade_selector_composition_preserves_nested_locators_and_frames() -> anyhow::Result<()> {
    let (output, calls) = run(r#"
        const p = await neo.page(7);
        const item = p.locator('.item');
        const composed = item.filter({hasText:/two/i, hasNotText:'gone', has:p.getByText('child'), hasNot:p.locator('.removed'), visible:false})
            .and(p.getByTitle('title')).or(p.getByRole('button')).last();
        try { await composed.click({timeout:23}); } catch (_) {}
        try { await p.locator('#outer').contentFrame().frameLocator('#inner').nth(2).getByLabel('Name').click(); } catch (_) {}
        try { await p.locator('.a').locator(p.locator('.b')).click(); } catch (_) {}
        return {selector:item.selector, frozen:Object.isFrozen(item), page:item.pageId};
    "#, json!({})).await?;
    success(&output);
    assert_eq!(
        output["value"],
        json!({"selector":".item", "frozen":true, "page":7})
    );
    assert_eq!(calls.len(), 4);
    assert_eq!(
        calls[1].args,
        json!([7, ".item >> internal:has-text=/two/i >> internal:has-not-text=\"gone\"i >> internal:has=\"internal:text=\\\"child\\\"i\" >> internal:has-not=\".removed\" >> visible=false >> internal:and=\"internal:attr=[title=\\\"title\\\"i]\" >> internal:or=\"internal:role=button\" >> nth=-1", {"timeout":23}])
    );
    assert_eq!(
        calls[2].args[1],
        "#outer >> internal:control=enter-frame >> #inner >> nth=2 >> internal:control=enter-frame >> internal:label=\"Name\"i"
    );
    assert_eq!(calls[3].args[1], ".a >> internal:chain=\".b\"");
    Ok(())
}

#[tokio::test]
async fn facade_escapes_text_attributes_and_regex_like_playwright() -> anyhow::Result<()> {
    let (output, _) = run(r#"
        const p = await neo.page(7);
        return [
            p.getByText('a"\\\nb', {exact:true}).selector,
            p.getByTitle('a"\\\nb').selector,
            p.getByRole('button', {checked:false, disabled:true, selected:false, expanded:true, includeHidden:true, level:2, name:/a"'`>>\\b/gi, pressed:false}).selector,
            p.getByText(/a"'`>>/u).selector,
            p.getByTestId(/id.*/i).selector,
            p.locator('.x').filter({hasText:'',hasNotText:''}).selector
        ];
    "#, json!({})).await?;
    success(&output);
    assert_eq!(
        output["value"],
        json!([
            "internal:text=\"a\\\"\\\\\\nb\"s",
            "internal:attr=[title=\"a\\\"\\\\\nb\"i]",
            "internal:role=button[checked=false][disabled=true][selected=false][expanded=true][include-hidden=true][level=2][name=/a\\\"'\\`\\>\\>\\\\b/gi][pressed=false]",
            "internal:text=/a\"'`>>/u",
            "internal:testid=[data-testid=/id.*/i]",
            ".x"
        ])
    );
    Ok(())
}

#[tokio::test]
async fn facade_locators_reject_cross_page_composition() -> anyhow::Result<()> {
    let (output, calls) = run(r#"
        const a = (await neo.page(1)).locator('a'), b = (await neo.page(2)).locator('b');
        const errors = [];
        for (const op of [() => a.and(b), () => a.or(b), () => a.locator(b), () => a.filter({has:b}), () => a.dragTo(b)]) {
            try { op(); } catch (error) { errors.push(error.message); }
        }
        return errors;
    "#, json!({})).await?;
    success(&output);
    assert_eq!(output["value"].as_array().map(Vec::len), Some(5));
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|call| call.method == "neo.page"));
    Ok(())
}

#[tokio::test]
async fn facade_expect_forwards_matcher_negation_regex_and_options() -> anyhow::Result<()> {
    let cases = [
        (
            "expect(p.getByText('Done')).toBeVisible()",
            "expect.toBeVisible",
            json!([7,"internal:text=\"Done\"i",true,{"timeout":5000,"isNot":false}]),
        ),
        (
            "expect(p.getByText('Done')).not.toBeVisible({timeout:13})",
            "expect.toBeVisible",
            json!([7,"internal:text=\"Done\"i",true,{"timeout":13,"isNot":true}]),
        ),
        (
            "expect(p).toHaveURL(/done$/i)",
            "expect.toHaveURL",
            json!([7,null,{"regexSource":"done$","regexFlags":"i"},{"timeout":5000,"isNot":false}]),
        ),
        (
            "expect(p.locator('a')).toHaveAttribute('href', /docs/i, {timeout:40})",
            "expect.toHaveAttribute",
            json!([7,"a",{"regexSource":"docs","regexFlags":"i"},{"timeout":40,"isNot":false,"name":"href"}]),
        ),
        (
            "expect.configure({timeout:70})(p.locator('li')).toHaveText(['one', /two/i])",
            "expect.toHaveText",
            json!([7,"li",["one",{"regexSource":"two","regexFlags":"i"}],{"timeout":70,"isNot":false}]),
        ),
        (
            "expect(p.locator('a')).toHaveCSS('color', 'red')",
            "expect.toHaveCSS",
            json!([7,"a","red",{"name":"color","timeout":5000,"isNot":false}]),
        ),
    ];
    for (expression, method, args) in cases {
        let (_, calls) = run(
            &format!("const p = await neo.page(7); await {expression};"),
            json!({}),
        )
        .await?;
        leaf(&calls, method, Some(7), args);
    }
    Ok(())
}

#[tokio::test]
async fn facade_expect_formats_failure_and_preserves_timeout_class() -> anyhow::Result<()> {
    let (output, _) = run(
        "await expect((await neo.page(7)).locator('#done')).toBeVisible({timeout:12});",
        json!({
            "expect.toBeVisible":{"matches":false,"received":"hidden","log":["waiting for #done"]}
        }),
    )
    .await?;
    let error = output["error"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing error: {output}"))?;
    for line in [
        "expect(locator).toBeVisible() failed",
        "Locator: #done",
        "Expected: true",
        "Received: \"hidden\"",
        "Timeout: 12ms",
        "Call log:",
    ] {
        assert!(error.contains(line), "{error}");
    }
    let (output, _) = run(r#"
        try { await (await neo.page(7)).locator('missing').click(); }
        catch (error) { return {name:error.name, message:error.message, string:String(error)}; }
    "#, json!({"locator.click":{"throw":"TimeoutError: locator.click: Timeout 10ms exceeded.\nCall log:\n  - waiting"}})).await?;
    success(&output);
    assert_eq!(output["value"]["name"], "TimeoutError");
    assert_eq!(
        output["value"]["message"],
        "locator.click: Timeout 10ms exceeded.\nCall log:\n  - waiting"
    );
    assert_eq!(
        output["value"]["string"],
        "TimeoutError: locator.click: Timeout 10ms exceeded.\nCall log:\n  - waiting"
    );
    Ok(())
}

#[tokio::test]
async fn facade_lazy_page_creates_once_for_concurrent_first_use() -> anyhow::Result<()> {
    let (output, calls) = run(
        r#"
        await Promise.all([page.goto('https://example.com'), page.getByText('Hello').click()]);
        return {id:page.pageId,url:page.url(),title:await page.title()};
    "#,
        json!({
            "context.lastPage":null, "context.newPage":9,
            "page.goto":{"value":null,"url":"https://example.com","title":"Example"},
            "page.info":{"url":"https://example.com","title":"Example"},
            "page.title":"Example",
            "locator.click":null
        }),
    )
    .await?;
    success(&output);
    assert_eq!(
        output["value"],
        json!({"id":9,"url":"https://example.com","title":"Example"})
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.method == "context.newPage")
            .count(),
        1
    );
    assert_eq!(calls[0].method, "context.newPage");
    assert_eq!(calls[0].args, json!([null,{"timeout":10000}]));
    assert!(
        !calls
            .iter()
            .any(|call| super::is_silent_method(&call.method))
    );
    assert!(calls.iter().any(|call| call.method == "page.goto"
        && call.page == Some(9)
        && call.args == json!([9,"https://example.com",{"timeout":10000}])));
    Ok(())
}

#[tokio::test]
async fn facade_lazy_page_uses_host_recency_and_metadata_from_each_leaf() -> anyhow::Result<()> {
    let (output, calls) = run(
        r#"
        const locator = page.getByRole('button');
        await locator.click();
        return {id:page.pageId,url:page.url(),title:await page.title()};
    "#,
        json!({
            "context.lastPage":{"pageId":4,"url":"https://before.test","title":"Before"},
            "locator.click":{"value":null,"url":"https://after.test","title":"After"},
            "page.title":"After"
        }),
    )
    .await?;
    success(&output);
    assert_eq!(
        output["value"],
        json!({"id":4,"url":"https://after.test","title":"After"})
    );
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].method, "locator.click");
    assert_eq!(
        calls[0].args,
        json!([4,"internal:role=button",{"timeout":10000}])
    );
    leaf(&calls, "page.title", Some(4), json!([4]));
    Ok(())
}

#[tokio::test]
async fn facade_new_page_defaults_and_query_argument_positions() -> anyhow::Result<()> {
    let (output, calls) = run(
        r#"
        context.setDefaultTimeout(31); context.setDefaultNavigationTimeout(41);
        const p = await context.newPage(); p.setDefaultTimeout(51);
        try { await p.goto('https://example.test'); } catch (_) {}
        try { await p.locator('input').fill('secret', {timeout:0,force:true}); } catch (_) {}
        try { await p.locator('input').getAttribute('type'); } catch (_) {}
        try { await p.keyboard.press('Enter'); } catch (_) {}
        return p.pageId;
    "#,
        json!({"context.newPage":7}),
    )
    .await?;
    success(&output);
    assert_eq!(calls[0].args, json!([null,{"timeout":31}]));
    assert_eq!(
        calls[1].args,
        json!([7,"https://example.test",{"timeout":51}])
    );
    assert_eq!(
        calls[2].args,
        json!([7,"input","[redacted]",{"timeout":0,"force":true}])
    );
    assert_eq!(
        calls[3].args,
        json!([7,"input","getAttribute","type",{"timeout":51}])
    );
    assert_eq!(calls[4].args, json!([7,"Enter",{"timeout":51}]));
    Ok(())
}

#[tokio::test]
async fn facade_evaluate_sends_source_and_keeps_json_result_envelope_separate() -> anyhow::Result<()>
{
    let (output, calls) = run(r#"
        const p = await neo.page(7);
        const fn = () => 1 + 1;
        const value = await p.evaluate(fn);
        return {source:String(fn), value, url:p.url(), title:await p.title()};
    "#, json!({"page.evaluate":{"value":{"value":2,"url":"user data","title":"user title"},"url":"https://example.test","title":"Page title"},"page.title":"Page title"})).await?;
    success(&output);
    assert_eq!(calls[1].args[1], output["value"]["source"]);
    assert!(
        calls[1].args[1]
            .as_str()
            .is_some_and(|source| source.contains("1 + 1"))
    );
    assert_eq!(
        calls[1].args,
        json!([7,output["value"]["source"],null,{"timeout":10000}])
    );
    assert_eq!(
        output["value"]["value"],
        json!({"value":2,"url":"user data","title":"user title"})
    );
    assert_eq!(output["value"]["url"], "https://example.test");
    Ok(())
}

#[tokio::test]
async fn facade_actions_and_queries_use_table_methods() -> anyhow::Result<()> {
    let (output, calls) = run(
        r#"
        const p = await neo.page(7), l = p.locator('input');
        for (const op of [
            () => l.uncheck(), () => l.setChecked(true), () => l.selectOption(['a','b']),
            () => l.setInputFiles('/file'), () => l.dragTo(p.locator('.target')),
            () => l.waitFor({state:'hidden',timeout:5}), () => l.allInnerTexts(),
            () => l.evaluate((el, arg) => el.textContent + arg, '!'),
            () => p.waitForURL(/done$/), () => p.waitForFunction(() => true, null, {timeout:8}),
            () => p.mouse.click(12,34,{button:'right'}), () => p.mouse.wheel(5,6)
        ]) { try { await op(); } catch (_) {} }
        return true;
    "#,
        json!({}),
    )
    .await?;
    success(&output);
    assert_eq!(calls[1].args, json!([7,"input",false,{"timeout":10000}]));
    assert_eq!(calls[2].args, json!([7,"input",true,{"timeout":10000}]));
    assert_eq!(
        calls[3].args,
        json!([7,"input",["a","b"],{"timeout":10000}])
    );
    assert_eq!(calls[4].args, json!([7,"input","/file",{"timeout":10000}]));
    assert_eq!(
        calls[5].args,
        json!([7,"input",".target",{"timeout":10000}])
    );
    assert_eq!(
        calls[6].args,
        json!([7,"input","hidden",{"state":"hidden","timeout":5}])
    );
    assert_eq!(
        calls[7].args,
        json!([7,"input","allInnerTexts",null,{"timeout":10000}])
    );
    assert_eq!(calls[8].method, "locator.evaluate");
    assert_eq!(
        calls[9].args,
        json!([7,{"source":"done$","flags":""},{"timeout":10000}])
    );
    assert_eq!(calls[10].method, "page.waitForFunction");
    assert_eq!(
        calls[11].args,
        json!([7,12,34,{"button":"right","timeout":10000}])
    );
    assert_eq!(calls[12].args, json!([7,5,6,{"timeout":10000}]));
    Ok(())
}

#[tokio::test]
async fn facade_events_arm_before_click_and_adapt_dialogs_downloads_popups() -> anyhow::Result<()> {
    let (output, calls) = run(r#"
        const p = await neo.page(7);
        const seen = [];
        p.once('dialog', async dialog => { seen.push([dialog.type(),dialog.message()]); await dialog.accept('yes'); });
        await p.locator('button').click();
        await sleep(10);
        const download = await p.waitForEvent('download');
        return {seen,name:download.suggestedFilename(),path:await download.path()};
    "#, json!({
        "page.waitForEvent":{"type":"confirm","message":"Delete?","suggestedFilename":"notes.txt","path":"/downloads/notes.txt"},
        "locator.click":null,"page.dialog":null
    })).await?;
    success(&output);
    assert_eq!(
        output["value"],
        json!({"seen":[["confirm","Delete?"]],"name":"notes.txt","path":"/downloads/notes.txt"})
    );
    assert_eq!(calls[1].method, "page.waitForEvent");
    assert_eq!(calls[1].args, json!([7,"dialog",null,{"timeout":0}]));
    // Leaves complete asynchronously, so audit rows are in completion order;
    // pin only that the dialog wait was armed before the click was issued.
    let click_index = calls
        .iter()
        .position(|call| call.method == "locator.click")
        .ok_or_else(|| anyhow::anyhow!("locator.click was never recorded"))?;
    assert!(click_index > 1, "click must follow the armed dialog wait");
    assert!(
        calls
            .iter()
            .any(|call| call.method == "page.dialog" && call.args == json!([7, true, "yes"]))
    );
    let (output, calls) = run(r#"
        const p = await context.waitForEvent('page', {timeout:13});
        const popup = await p.waitForEvent('popup');
        return [p.pageId,popup.pageId,popup.url()];
    "#, json!({"context.waitForEvent":{"pageId":9},"page.waitForEvent":{"pageId":10,"url":"https://popup.test","title":"Popup"}})).await?;
    success(&output);
    assert_eq!(output["value"], json!([9, 10, "https://popup.test"]));
    assert_eq!(calls[0].args, json!(["page",null,{"timeout":13}]));
    Ok(())
}

#[tokio::test]
async fn facade_unavailable_apis_have_actionable_exact_errors() -> anyhow::Result<()> {
    let (output, calls) = run(r#"
        const errors = [];
        for (const op of [() => page.route('**',()=>{}), () => context.cookies(), () => page.evaluateHandle(()=>1), () => page.request, () => context.tracing, () => require('fs'), () => fetch('https://example.com'), () => process.env, () => fs]) {
            try { op(); } catch (error) { errors.push(String(error)); }
        }
        return errors;
    "#, json!({})).await?;
    success(&output);
    assert!(calls.is_empty());
    let hint = "Use page.evaluate() or neo.cdp() for supported browser operations.";
    let node_hint = "No Node.js or modules here; use the provided browser, context, page, expect and neo globals.";
    assert_eq!(
        output["value"],
        json!([
            format!("Error: not available in BrowserOS neo: page.route. {hint}"),
            "Error: not available in BrowserOS neo: context.cookies. you are already signed in",
            "Error: not available in BrowserOS neo: page.evaluateHandle. Use page.evaluate() to return JSON.",
            format!("Error: not available in BrowserOS neo: page.request. {hint}"),
            format!("Error: not available in BrowserOS neo: context.tracing. {hint}"),
            format!("Error: not available in BrowserOS neo: require. {node_hint}"),
            format!("Error: not available in BrowserOS neo: fetch. {node_hint}"),
            format!("Error: not available in BrowserOS neo: process. {node_hint}"),
            format!("Error: not available in BrowserOS neo: fs. {node_hint}")
        ])
    );
    let (output, _) = run("await import('fs');", json!({})).await?;
    assert!(
        output["error"]
            .as_str()
            .is_some_and(|error| error.contains("not available in BrowserOS neo: import().")),
        "{output}"
    );
    Ok(())
}

#[tokio::test]
async fn facade_pure_assertions_scaffolding_and_pasted_tests_work() -> anyhow::Result<()> {
    let (output, calls) = run(r#"
        expect(NaN).toBe(NaN); expect({a:[1,2],b:3}).toEqual({b:3,a:[1,2]});
        expect('hello').toContain('ell'); expect([1,2]).toContain(2);
        expect(false).not.toBeTruthy(); expect(4).toBeGreaterThan(3); expect('Hi').toMatch(/hi/i);
        expect(new Set([1,2])).toEqual(new Set([2,1]));
        expect(new Map([['a',1]])).not.toEqual(new Map([['a',2]]));
        expect(new Array(2)).not.toEqual([]);
        let failed = false; try { expect(1).not.toBe(1); } catch (_) { failed = true; }
        expect(failed).toBeTruthy();
        expect(await chromium.launch()).toBe(browser); expect(await chromium.connectOverCDP()).toBe(browser);
        expect(browser.contexts()).toEqual([context]); expect(context.browser()).toBe(browser);
        test('pasted test', async ({page}) => { await sleep(2); console.log('finished', typeof page.goto); });
        await browser.newContext(); await browser.close(); await context.close(); await page.bringToFront();
        console.log('value', {a:1});
        return {n:BigInt(4),fn:()=>1};
    "#, json!({})).await?;
    success(&output);
    assert!(calls.is_empty());
    assert_eq!(output["value"]["n"], "4");
    assert!(
        output["logs"]
            .as_array()
            .is_some_and(|logs| logs.contains(&json!("finished function")))
    );
    Ok(())
}

#[tokio::test]
async fn facade_negated_assertions_use_raw_match_and_timeout_outcomes() -> anyhow::Result<()> {
    for (is_not, matches, timed_out, ok) in [
        (false, true, false, true),
        (false, false, false, false),
        (true, false, false, true),
        (true, true, false, false),
        (true, false, true, false),
        (false, true, true, false),
    ] {
        let not = if is_not { ".not" } else { "" };
        let (output, _) = run(
            &format!("await expect((await neo.page(7)).locator('div')){not}.toBeVisible();"),
            json!({"expect.toBeVisible":{"matches":matches,"timedOut":timed_out,"received":"hidden"}}),
        ).await?;
        assert_eq!(output["ok"], ok, "{output}");
    }
    Ok(())
}

#[tokio::test]
async fn facade_attribute_existence_and_checked_options_reach_expect() -> anyhow::Result<()> {
    let (output, calls) = run(r#"
        const p = await neo.page(7);
        try { await expect(p.locator('a')).toHaveAttribute('disabled', {timeout:9}); } catch (_) {}
        try { await expect(p.locator('input')).not.toBeChecked({checked:false,timeout:8}); } catch (_) {}
    "#, json!({})).await?;
    success(&output);
    assert_eq!(
        calls[1].args,
        json!([7,"a",null,{"name":"disabled","timeout":9,"isNot":false}])
    );
    assert_eq!(
        calls[2].args,
        json!([7,"input",true,{"checked":false,"timeout":8,"isNot":true}])
    );
    Ok(())
}

#[tokio::test]
async fn facade_page_lifecycle_reuses_identity_and_replaces_closed_lazy_page() -> anyhow::Result<()>
{
    let (output, calls) = run(
        r#"
        const p = await context.newPage();
        const listed = await context.pages();
        expect(listed[0]).toBe(p);
        await page.locator('a').click();
        expect(page.pageId).toBe(7);
        await p.close(); expect(p.isClosed()).toBe(true);
        await page.locator('b').click();
        return {old:p.pageId,current:page.pageId,closed:p.isClosed()};
    "#,
        json!({
            "context.newPage":7,
            "context.pages":[{"pageId":7,"url":"https://owned.test","title":"Owned"}],
            "context.lastPage":{"pageId":8,"url":"https://recent.test","title":"Recent"},
            "locator.click":null,"context.close":null
        }),
    )
    .await?;
    success(&output);
    assert_eq!(output["value"], json!({"old":7,"current":8,"closed":true}));
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.method == "context.newPage")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.method == "context.lastPage")
            .count(),
        0
    );
    leaf(
        &calls,
        "locator.click",
        Some(8),
        json!([8,"b",{"timeout":10000}]),
    );
    Ok(())
}

#[tokio::test]
async fn facade_neo_rehydrates_metadata_and_routes_all_extras() -> anyhow::Result<()> {
    let (output, calls) = run(
        r#"
        const p = await neo.page(7);
        const before = [p.pageId,p.url(),await p.title()];
        for (const op of [
            () => neo.pages({ownership:'other-agent'}), () => neo.snapshot(p),
            () => neo.read(p,{format:'markdown'}), () => neo.grep(p,{pattern:'hello'}),
            () => neo.download(p,{url:'https://example.test/file'}),
            () => neo.cdp('Runtime.evaluate',{expression:'2'},p)
        ]) { try { await op(); } catch (_) {} }
        return before;
    "#,
        json!({"neo.page":{"pageId":7,"url":"https://example.test","title":"Example"},"page.title":"Example"}),
    )
    .await?;
    success(&output);
    assert_eq!(
        output["value"],
        json!([7, "https://example.test", "Example"])
    );
    assert_eq!(calls[0].args, json!([7]));
    assert_eq!(calls[0].page, Some(7));
    assert_eq!(calls[1].method, "page.title");
    assert_eq!(calls[1].args, json!([7]));
    assert_eq!(calls[2].args, json!([{"ownership":"other-agent"}]));
    assert_eq!(calls[3].args, json!([7]));
    assert_eq!(calls[4].args, json!([7,{"format":"markdown"}]));
    assert_eq!(calls[5].args, json!([7,{"pattern":"hello"}]));
    assert_eq!(
        calls[6].args,
        json!([7,{"url":"https://example.test/file"}])
    );
    assert_eq!(
        calls[7].args,
        json!(["Runtime.evaluate",{"expression":"2"},7])
    );
    Ok(())
}

#[tokio::test]
async fn facade_frames_keep_explicit_parentage_and_metadata() -> anyhow::Result<()> {
    let (output, calls) = run(r#"
        const p = await neo.page(7);
        const frames = await p.frames();
        return frames.map(frame => ({
            url:frame.url(), name:frame.name(), page:frame.page().pageId,
            parent:frame.parentFrame()?.frameId || null, children:frame.childFrames().map(child => child.frameId)
        }));
    "#, json!({"page.frames":{"frameTree":{
        "frame":{"id":"main","url":"https://example.test","name":""},
        "childFrames":[{"frame":{"id":"child","url":"https://child.test","name":"inner"}}]
    }}})).await?;
    success(&output);
    assert_eq!(
        output["value"],
        json!([
            {"url":"https://example.test","name":"","page":7,"parent":null,"children":["child"]},
            {"url":"https://child.test","name":"inner","page":7,"parent":"main","children":[]}
        ])
    );
    leaf(&calls, "page.frames", Some(7), json!([7]));
    Ok(())
}

#[tokio::test]
async fn facade_syntax_failure_precedes_all_bridge_effects() -> anyhow::Result<()> {
    let (output, calls) = run("const page = await context.newPage(;", json!({})).await?;
    assert_eq!(output["ok"], false);
    assert!(
        output["error"]
            .as_str()
            .is_some_and(|error| error.contains("syntax error"))
    );
    assert!(calls.is_empty());
    Ok(())
}

#[tokio::test]
async fn facade_lazy_stub_failure_identifies_requested_api_and_dependency() -> anyhow::Result<()> {
    let (output, calls) = run(
        "await page.goto('https://example.test');",
        json!({"context.lastPage":{"throw":"not implemented yet: context.lastPage"}}),
    )
    .await?;
    assert_eq!(output["ok"], false);
    let error = output["error"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing error"))?;
    assert!(error.contains("not implemented yet: page.goto"), "{error}");
    assert!(error.contains("context.lastPage"), "{error}");
    assert!(calls.is_empty(), "internal discovery must remain silent");
    Ok(())
}

#[tokio::test]
async fn facade_lazy_first_action_inherits_the_selected_pages_timeout() -> anyhow::Result<()> {
    let (output, calls) = run(
        r#"
        const p = await context.newPage();
        p.setDefaultTimeout(71); p.setDefaultNavigationTimeout(81);
        try { await page.locator('button').click(); } catch (_) {}
        try { await page.goto('https://example.test'); } catch (_) {}
        try { await page.locator('button').click({timeout:0}); } catch (_) {}
    "#,
        json!({"context.newPage":7,"context.pages":[{"pageId":7}]}),
    )
    .await?;
    success(&output);
    let actions: Vec<_> = calls
        .iter()
        .filter(|call| call.method == "locator.click" || call.method == "page.goto")
        .collect();
    assert_eq!(actions[0].args, json!([7,"button",{"timeout":71}]));
    assert_eq!(
        actions[1].args,
        json!([7,"https://example.test",{"timeout":81}])
    );
    assert_eq!(actions[2].args, json!([7,"button",{"timeout":0}]));
    Ok(())
}

#[tokio::test]
async fn facade_corpus_declarations_can_shadow_entry_globals() -> anyhow::Result<()> {
    let (output, calls) = run(r#"
        const browser = await chromium.launch();
        const context = await browser.newContext();
        const page = await context.newPage();
        await page.goto('https://example.test');
        return page.pageId;
    "#, json!({"context.newPage":7,"page.goto":null,"page.info":{"url":"https://example.test","title":"Example"}})).await?;
    success(&output);
    assert_eq!(output["value"], 7);
    assert_eq!(calls[0].method, "context.newPage");
    assert_eq!(calls[1].method, "page.goto");
    Ok(())
}

#[tokio::test]
async fn facade_response_bodies_are_correlated_decoded_and_fetched_once() -> anyhow::Result<()> {
    // Return metadata on the first event call and body bytes on the correlated
    // second call. Both still reach the real Rust hook before substitution.
    let (output, calls) = run(r#"
        const intercepted = __browserosCall;
        globalThis.__browserosCall = async (method, args, helper) => {
            const value = await intercepted(method,args,helper);
            if (method === 'page.waitForEvent') {
                if (JSON.parse(args)[3].body) return {body:'eyJ2Ijoi8J+miiJ9',base64Encoded:true};
                return {url:'https://example.test/data',status:200,headers:{'content-type':'application/json'},requestId:'request-1'};
            }
            return value;
        };
        const p = await neo.page(7);
        const response = await p.waitForEvent('response', r => r.status() === 200);
        const [bytes,text,json] = await Promise.all([response.body(),response.text(),response.json()]);
        return {text,json,bytes:Array.from(bytes),string:bytes.toString(),url:response.request().url()};
    "#, json!({"page.waitForEvent":null})).await?;
    success(&output);
    assert_eq!(output["value"]["text"], "{\"v\":\"🦊\"}");
    assert_eq!(output["value"]["string"], output["value"]["text"]);
    assert_eq!(output["value"]["json"], json!({"v":"🦊"}));
    assert_eq!(
        output["value"]["bytes"],
        json!([123, 34, 118, 34, 58, 34, 240, 159, 166, 138, 34, 125])
    );
    assert_eq!(output["value"]["url"], "https://example.test/data");
    assert_eq!(calls.len(), 3);
    leaf(
        &calls,
        "page.waitForEvent",
        Some(7),
        json!([7,"response",{"requestId":"request-1"},{"body":true,"timeout":10000}]),
    );
    Ok(())
}

#[tokio::test]
async fn facade_p5_numeric_popups_and_flat_frames_keep_their_identity() -> anyhow::Result<()> {
    let (output, calls) = run(r#"
        const p = await neo.page(7);
        const popup = await p.waitForEvent('popup');
        const frames = await popup.frames();
        return {popup:popup.pageId,child:frames[0].parentFrame().frameId,parentChildren:frames[1].childFrames().map(f=>f.frameId)};
    "#, json!({"page.waitForEvent":9,"page.frames":[
        {"frameId":"child","parentFrameId":"main","url":"https://child.test","name":"inner"},
        {"frameId":"main","parentFrameId":null,"url":"https://example.test","name":""}
    ]})).await?;
    success(&output);
    assert_eq!(
        output["value"],
        json!({"popup":9,"child":"main","parentChildren":["child"]})
    );
    leaf(&calls, "page.frames", Some(9), json!([9]));
    Ok(())
}

#[tokio::test]
async fn facade_does_not_require_post_es2020_object_builtins() -> anyhow::Result<()> {
    let (output, calls) = run(
        r#"
        Object.hasOwn = undefined;
        const p = await neo.page(7);
        expect({value:1}).toEqual({value:1});
        await expect(p.locator('button')).toBeVisible();
        return await p.title();
    "#,
        json!({"neo.page":{"pageId":7,"title":"ES2020"},"page.title":"ES2020","expect.toBeVisible":{"matches":true}}),
    )
    .await?;
    success(&output);
    assert_eq!(output["value"], "ES2020");
    assert_eq!(calls[calls.len() - 2].method, "expect.toBeVisible");
    leaf(&calls, "page.title", Some(7), json!([7]));
    Ok(())
}

#[tokio::test]
async fn facade_network_waits_transport_url_filters_and_expose_response_bodies()
-> anyhow::Result<()> {
    for (expression, filter) in [
        ("'**/api/items'", json!("**/api/items")),
        (
            "/api\\/items$/i",
            json!({"source":"api\\/items$","flags":"i"}),
        ),
        (
            "r => r.status() === 200",
            json!({"predicate":"r => r.status() === 200"}),
        ),
    ] {
        let (output, calls) = run(
            &format!("const p = await neo.page(7); const response = await p.waitForResponse({expression}, {{timeout:40}}); return {{url:response.url(),status:response.status(),data:await response.json()}};"),
            json!({"page.waitForEvent":{"url":"https://example.test/api/items","status":200,"body":"{\"items\":[1]}","base64Encoded":false}}),
        ).await?;
        success(&output);
        assert_eq!(
            output["value"],
            json!({"url":"https://example.test/api/items","status":200,"data":{"items":[1]}})
        );
        leaf(
            &calls,
            "page.waitForEvent",
            Some(7),
            json!([7,"response",filter,{"timeout":40}]),
        );
    }
    let (output, calls) = run(
        "const p = await neo.page(7); const request = await p.waitForRequest('**/api/items'); return request.method();",
        json!({"page.waitForEvent":{"url":"https://example.test/api/items","method":"POST"}}),
    ).await?;
    success(&output);
    assert_eq!(output["value"], "POST");
    leaf(
        &calls,
        "page.waitForEvent",
        Some(7),
        json!([7,"request","**/api/items",{"timeout":10000}]),
    );
    Ok(())
}
