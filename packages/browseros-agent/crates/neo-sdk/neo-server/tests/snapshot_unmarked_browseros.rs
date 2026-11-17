//! A missing clickable check is visible to the caller of a real snapshot. The malformed
//! page makes Chromium's DOM capture unreadable, while the probe's text exit still returns;
//! its whole-character twin proves that the script-only target normally receives a ref.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::io::{self, Write as _};
use std::{env, fs};

use browseros_rig::{CaptchaKey, Client, Rig, TestResult};
use serde_json::{Value, json};

async fn value(client: &Client, task: &str, code: &str) -> Result<Value, Box<dyn Error>> {
    let reply = browseros_rig::run(client, code, &json!({ "taskId": task })).await?;
    if browseros_rig::outcome(&reply)?["status"] != "ok" {
        return Err(format!("{task} did not return: {reply:?}").into());
    }
    reply
        .structured_content
        .and_then(|v| v.get("value").cloned())
        .ok_or_else(|| "run value missing".into())
}

const PAGE: &str = r#"
page.setDefaultTimeout(5000);
const html=`<meta charset=utf-8><button id=native>Native action</button>
<div id=listener style="display:inline-block;padding:8px">Script only</div><script>
document.getElementById('native').textContent='Before '+String.fromCodePoint(UNIT)+' After';
document.getElementById('listener').addEventListener('click',()=>{});
</script>`;
await page.goto('data:text/html,'+encodeURIComponent(html));
const snapshot=await page.ariaSnapshot();
const full=await page.ariaSnapshot({mode:'full'});
const locator=await page.locator('body').ariaSnapshot();
const upstream=await page.ariaSnapshot({mode:'default'});
const json=await page.ariaSnapshotJSON();
return {snapshot,full,locator,upstream,json,
 listener:snapshot.split('\n').find(l=>l.includes('Script only'))};
"#;

#[tokio::test(flavor = "multi_thread")]
async fn unchecked_root_explains_missing_script_only_refs_and_checked_twin_has_them() -> TestResult
{
    let Some(browser) = browseros_rig::browser_or_skip() else {
        return Ok(());
    };
    let rig = Rig::start(&browser, CaptchaKey::Absent)?;
    rig.until_ready().await?;
    let client = rig.client().await?;
    for (unit, unchecked) in [(0xd83d_u32, true), (0x1f600, false)] {
        let task = format!("snapshot-check-{unit}");
        let result = value(&client, &task, &PAGE.replace("UNIT", &unit.to_string())).await?;
        for form in ["snapshot", "full", "locator"] {
            let text = result[form].as_str().ok_or("snapshot text")?;
            assert_eq!(text.starts_with("# refs missing: the check for clickable elements did not finish on this page (timeout)."), unchecked, "{form}: {result}");
        }
        assert!(
            !result["upstream"]
                .as_str()
                .ok_or("upstream text")?
                .contains("# refs missing:")
        );
        assert!(!result["json"].to_string().contains("unmarked"));
        let listener = result["listener"]
            .as_str()
            .ok_or("script-only content remains listed")?;
        assert_eq!(listener.contains("[ref="), !unchecked, "{listener}");
        // A later run compares with the actual text the caller received, including its notice.
        // The notice describes that read and must not turn an unchanged page into a diff.
        let delta = value(
            &client,
            &task,
            "return await page.ariaSnapshot({delta:true});",
        )
        .await?;
        let text = delta.as_str().ok_or("delta text")?;
        assert_eq!(
            text.lines().next(),
            Some("# no change since your last snapshot of this tab")
        );
        assert_eq!(text.contains("# refs missing:"), unchecked, "{text}");
    }
    client.cancel().await?;
    Ok(())
}

/// One-off evidence for child-frame skips, which deliberately do not produce the root's
/// notice. The public URL list is supplied by the measurement job; no page content is saved
/// in the repository, and refused pages remain separate from checked pages in the report.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "one-off named-page measurement; needs SNAPSHOT_CHECK_PAGES and the pinned browser"]
async fn count_unchecked_roots_and_children_on_named_pages() -> TestResult {
    let Some(browser) = browseros_rig::browser_or_skip() else {
        return Ok(());
    };
    let pages = fs::read_to_string(env::var("SNAPSHOT_CHECK_PAGES")?)?;
    let rig = Rig::start_with(
        &browser,
        CaptchaKey::Absent,
        &[("CLAW_LOG", "info,neo_server=debug")],
    )?;
    rig.until_ready().await?;
    let client = rig.client().await?;
    for line in pages.lines().filter(|line| !line.trim().is_empty()) {
        let (name, url) = line.split_once(' ').ok_or("page list is name URL")?;
        let task = format!("snapshot-check-{name}");
        let code = format!(
            r"
await page.setViewportSize({{width:1280,height:800}});
page.setDefaultTimeout(60000);
let status=null,error=null;try{{const response=await page.goto({},{{waitUntil:'domcontentloaded',timeout:30000}});status=response?.status()??null}}catch(e){{error=String(e.message)}}
let title=null;try{{title=await page.title()}}catch(e){{title=String(e.message)}}return {{status,error,title,url:page.url()}};",
            json!(url)
        );
        let navigation = value(&client, &task, &code).await?;
        for phase in ["immediate", "settled"] {
            let offset = rig.neo_log()?.len();
            let code = format!(
                r"
{}const start=Date.now();try{{const text=await page.ariaSnapshot();return {{ok:true,ms:Date.now()-start,unmarked:text.includes('# refs missing:'),reason:/^# refs missing:.*?\(([^)]+)\)\./m.exec(text)?.[1]??null}}}}catch(e){{return {{ok:false,ms:Date.now()-start,error:String(e.message)}}}};",
                if phase == "settled" {
                    "await page.waitForTimeout(4000);"
                } else {
                    ""
                }
            );
            let result = value(&client, &task, &code).await?;
            let log = rig.neo_log()?;
            let notes = log.get(offset..).ok_or("append-only operation log")?;
            // Capture skips precede the root's completed walk; a root page-shape skip is
            // logged just after it. The root field identifies that one note, which must not
            // be counted as a descendant. With no targets a capture skip cannot also be a
            // page-shape skip (snapshot.ts marked() returns directly on an empty target list).
            let children =
                if let Some((_, after_root)) = notes.split_once("read the main frame's tree in") {
                    Some(
                        after_root
                            .matches("clickable marking skipped a frame")
                            .count()
                            .checked_sub(usize::from(result["reason"] == "page shape"))
                            .ok_or("the root page-shape note is missing from its operation log")?,
                    )
                } else {
                    None
                };
            writeln!(
                io::stderr(),
                "REF_CHECK {}",
                json!({
                    "page": name, "phase": phase, "navigation": navigation,
                    "snapshot": result, "childSkips": children,
                })
            )?;
        }
        value(&client, &task, "await page.close();return true;").await?;
    }
    client.cancel().await?;
    Ok(())
}
