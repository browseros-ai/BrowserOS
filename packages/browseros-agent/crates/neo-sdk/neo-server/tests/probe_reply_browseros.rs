//! Probe values cross the real MCP/server/CDP path as text, so malformed page UTF-16 and
//! deep ARIA trees cannot disappear in the shared protocol reader. Each row owns a fresh
//! task document; a failed call is bounded and never retried as an uncertain click.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::time::Duration;

use browseros_rig::{CaptchaKey, Client, Rig, TestResult};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};
use tokio::time;

async fn returned(client: &Client, task: &str, code: &str) -> Result<(Value, CallToolResult), Box<dyn Error>> {
    let reply = browseros_rig::run(client, code, &json!({"taskId": task})).await?;
    if browseros_rig::outcome(&reply)?.get("status") != Some(&json!("ok")) {
        return Err(format!("{task} did not return: {reply:?}").into());
    }
    let value = reply.structured_content.as_ref().and_then(|v| v.get("value"))
        .cloned().ok_or("a run returns its observed value")?;
    Ok((value, reply))
}

/// Generate the malformed unit inside the page: the fixture's source and the navigation
/// argument remain valid Unicode, so the row isolates the reply rather than the input bridge.
const HALF_PAGE: &str = r#"
page.setDefaultTimeout(3000);
const html = `<meta charset=utf-8><button id=target>Target</button><script>
const target=document.getElementById('target');
const bad='Before '+String.fromCharCode(HALF)+' After';
if(PLACE==='input'||PLACE==='placeholder'){
 const field=document.createElement('input');field.id='target';target.replaceWith(field);
 if(PLACE==='input'){field.value=bad;field.setAttribute('value',bad)}
 else field.setAttribute('placeholder',bad);
}else if(PLACE==='text')target.textContent=bad;
else if(PLACE==='aria')target.setAttribute('aria-label',bad);
else {target.textContent='';target.setAttribute('title',bad)}
document.body.dataset.clicks='0';document.getElementById('target').addEventListener('click',()=>document.body.dataset.clicks=String(Number(document.body.dataset.clicks)+1));
</script>`;
await page.goto('data:text/html,'+encodeURIComponent(html));
const snapshot=await page.ariaSnapshot();
const ref=snapshot.split('\n').filter(l=>/\b(button|textbox)\b/.test(l)).map(l=>/\[ref=([^\]]+)\]/.exec(l)?.[1]).find(Boolean);
if(!ref)throw new Error('target ref missing: '+snapshot);
await page.locator('#target').click();
const target=page.locator('#target');
const attribute=PLACE==='aria'?'aria-label':PLACE==='title'?'title':PLACE==='placeholder'?'placeholder':'value';
let failed='';try{await expect(target).toHaveText('Never matches',{timeout:20})}catch(e){failed=String(e.message)}
return {snapshot,ref,text:await target.textContent(),inner:await target.innerText(),
 value:PLACE==='input'?await target.inputValue():null,
 attribute:await target.getAttribute(attribute),
 prefix:PLACE==='text'?await page.getByText('Before').count():null,
 replacement:await page.getByText('Before \ufffd After',{exact:true}).count(),
 failed, clicks:await page.evaluate(()=>Number(document.body.dataset.clicks))};
"#;

#[tokio::test(flavor = "multi_thread")]
async fn malformed_utf16_page_text_and_locator_actions_return_through_mcp() -> TestResult {
    let Some(browser) = browseros_rig::browser_or_skip() else { return Ok(()) };
    let rig = Rig::start_with(&browser, CaptchaKey::Absent, &[("CLAW_LOG", "info,browseros_cdp=debug")])?;
    let result = time::timeout(Duration::from_secs(240), async {
        rig.until_ready().await?;
        let client = rig.client().await?;
        for half in [0xd83d_u32, 0xde00] {
            for place in ["text", "aria", "input", "title", "placeholder"] {
                let task = format!("probe-half-{half}-{place}");
                let code = HALF_PAGE.replace("HALF", &half.to_string()).replace("PLACE", &json!(place).to_string());
                let (value, reply) = returned(&client, &task, &code).await?;
                let snapshot = value["snapshot"].as_str().ok_or("snapshot text")?;
                assert!(snapshot.contains('\u{fffd}'), "{task}: {value}");
                assert_eq!(value["clicks"], 1, "{task}: selector click");
                assert_eq!(value["replacement"], 0, "replacement text is not the DOM's text");
                assert!(value["failed"].as_str().is_some_and(|s| s.contains("Never matches")), "{task}: failed expect replies");
                if place == "text" {
                    assert_eq!(value["text"], "Before \u{fffd} After");
                    assert_eq!(value["inner"], "Before \u{fffd} After");
                    assert_eq!(value["prefix"], 1);
                } else {
                    assert_eq!(value["attribute"], "Before \u{fffd} After", "{task}");
                }
                if place == "input" { assert_eq!(value["value"], "Before \u{fffd} After"); }
                let id = browseros_rig::outcome(&reply)?["id"].as_str().ok_or("run id")?;
                let trace = rig.host("GET", &format!("/v1/runs/{id}?wait=30"), Some(browseros_rig::HOST_TOKEN), None).await?;
                assert_eq!(trace.status, 200);
                assert!(trace.text().contains("clickable marking skipped"), "the shared DOMSnapshot limitation stays explicit: {}", trace.text());
                let ref_code = format!("await page.getByRef({}).click();return await page.evaluate(()=>Number(document.body.dataset.clicks));", value["ref"]);
                let (clicks, _) = returned(&client, &task, &ref_code).await?;
                assert_eq!(clicks, 2, "{task}: the printed ref resolves in a later run");
                eprintln!("PROBE_REPLY half={half} place={place} snapshot=ok selector=1 ref=2");
            }
        }
        client.cancel().await?;
        Ok::<(), Box<dyn Error>>(())
    }).await;
    match result {
        Ok(Ok(())) => Ok(()),
        other => Err(format!("{other:?}\n{}", rig.logs()).into()),
    }
}

const DEEP_PAGE: &str = r#"
page.setDefaultTimeout(3000);
const nested='<div role="group" aria-label="Level">'.repeat(DEPTH)+'<button>Deep target</button>'+'</div>'.repeat(DEPTH);
await page.goto('data:text/html,'+encodeURIComponent(nested));
const forms={
 default:()=>page.ariaSnapshot(), full:()=>page.ariaSnapshot({mode:'full'}),
 delta:()=>page.ariaSnapshot({delta:true}), json:()=>page.ariaSnapshotJSON(),
 locator:()=>page.locator('body').ariaSnapshot()
};
const values={};for(const [name,read] of Object.entries(forms)){const start=Date.now();try{
 const value=await read();values[name]={ok:true,ms:Date.now()-start,found:JSON.stringify(value).includes('Deep target')};
}catch(e){values[name]={ok:false,ms:Date.now()-start,code:e.code,message:String(e.message)}}}
return values;
"#;

#[tokio::test(flavor = "multi_thread")]
async fn deep_aria_trees_return_or_refuse_promptly_in_every_snapshot_form() -> TestResult {
    let Some(browser) = browseros_rig::browser_or_skip() else { return Ok(()) };
    let rig = Rig::start(&browser, CaptchaKey::Absent)?;
    rig.until_ready().await?;
    let client = rig.client().await?;
    for depth in [70, 130, 147, 148] {
        let (value, _) = returned(&client, &format!("probe-depth-{depth}"), &DEEP_PAGE.replace("DEPTH", &depth.to_string())).await?;
        for form in ["default", "full", "delta", "json", "locator"] {
            let observed = &value[form];
            assert_eq!(observed["ok"], depth <= 147, "depth {depth}, {form}: {observed}");
            if depth <= 147 { assert_eq!(observed["found"], true, "{observed}"); }
            else {
                let message = observed["message"].as_str().ok_or("depth error")?;
                assert_eq!(observed["code"], "unsupported", "{observed}");
                assert!(message.contains("depth") && message.contains("{ depth: n }") && message.contains("locator"), "{message}");
                assert!(observed["ms"].as_u64().is_some_and(|ms| ms < 2500), "a refusal is not the 3000 ms timeout: {observed}");
            }
            eprintln!("PROBE_REPLY depth={depth} form={form} {observed}");
        }
    }
    client.cancel().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_child_tree_that_would_exceed_the_stitched_cap_is_partial() -> TestResult {
    let Some(browser) = browseros_rig::browser_or_skip() else { return Ok(()) };
    let rig = Rig::start(&browser, CaptchaKey::Absent)?;
    rig.until_ready().await?;
    let client = rig.client().await?;
    for depth in [70, 130, 147, 148] {
        let code = format!(r#"
page.setDefaultTimeout(3000);
const child='<div role="group" aria-label="Level">'.repeat({depth})+'<button>Child target</button>'+'</div>'.repeat({depth});
await page.goto('data:text/html,'+encodeURIComponent('<button>Outside</button><iframe></iframe>'));
await page.evaluate(html=>document.querySelector('iframe').srcdoc=html,child);
await page.frameLocator('iframe').getByRole('button').waitFor();
return await page.ariaSnapshot();"#);
        let (value, _) = returned(&client, &format!("probe-child-depth-{depth}"), &code).await?;
        let text = value.as_str().ok_or("page text")?;
        assert!(text.contains("Outside"));
        if depth <= 130 { assert!(text.contains("Child target"), "{depth}: {text}"); }
        else { assert!(text.contains("# partial:") && text.contains("depth"), "{depth}: {text}"); }
        eprintln!("PROBE_REPLY child_depth={depth} partial={}", depth > 130);
    }
    client.cancel().await?;
    Ok(())
}
