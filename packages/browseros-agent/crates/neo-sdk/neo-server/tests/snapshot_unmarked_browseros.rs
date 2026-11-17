//! A bounded busy frame forces the clickable check to miss its budget while the snapshot
//! still returns. The subsequent idle read of the same DOM shows its script-only ref again,
//! without relying on malformed page strings or on the shared protocol client dropping them.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::io::{self, Write as _};

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

/// Prime the isolated world before holding its renderer busy: an install queued behind the
/// loop would consume the hold before the clickable capture's own budget even begins.
const BUSY_PAGE: &str = r#"
await page.goto('data:text/html,'+encodeURIComponent(`<meta charset=utf-8>
<button>Native action</button><div id=listener style="display:inline-block;padding:8px">Script only</div>
<script>document.getElementById('listener').addEventListener('click',()=>{});</script>`));
await page.ariaSnapshotJSON();
const started=page.waitForEvent('console',{predicate:m=>m.text()==='snapshot-check-loop-start'});
const ended=page.waitForEvent('console',{predicate:m=>m.text()==='snapshot-check-loop-end'});
await page.evaluate(()=>{setTimeout(()=>{
 console.log('snapshot-check-loop-start');
 const until=performance.now()+3000;while(performance.now()<until){}
 console.log('snapshot-check-loop-end');
},0)});
await started;
const at=Date.now();const snapshot=await page.ariaSnapshot();const elapsed=Date.now()-at;
await ended;
return {snapshot,elapsed,listener:snapshot.split('\n').find(l=>l.includes('Script only'))};
"#;

#[tokio::test(flavor = "multi_thread")]
async fn busy_snapshot_check_explains_missing_script_only_refs_and_recovers() -> TestResult {
    let Some(browser) = browseros_rig::browser_or_skip() else {
        return Ok(());
    };
    // This rig starts the ordinary local server, which receives console events. Hosted mode
    // intentionally does not; using this synchronization there must fail, never skip a row.
    let rig = Rig::start(&browser, CaptchaKey::Absent)?;
    rig.until_ready().await?;
    let client = rig.client().await?;
    let busy = value(&client, "snapshot-check-busy", BUSY_PAGE).await?;
    let snapshot = busy["snapshot"]
        .as_str()
        .ok_or("the busy snapshot returned text")?;
    assert!(snapshot.starts_with("# refs missing: the check for clickable elements did not finish on this page (timeout)."), "{busy}");
    let listener = busy["listener"]
        .as_str()
        .ok_or("script-only content remains listed")?;
    assert!(!listener.contains("[ref="), "{listener}");
    // The first run awaited the loop's own end event, so this read observes recovery rather
    // than relying on a sleep or on the test machine's speed. Its DOM has not changed.
    let idle = value(
        &client,
        "snapshot-check-busy",
        r"
const snapshot=await page.ariaSnapshot();
return {snapshot,listener:snapshot.split('\n').find(l=>l.includes('Script only')),
 full:await page.ariaSnapshot({mode:'full'}), locator:await page.locator('body').ariaSnapshot()};
",
    )
    .await?;
    for form in ["snapshot", "full", "locator"] {
        assert!(
            !idle[form]
                .as_str()
                .ok_or("idle snapshot text")?
                .contains("# refs missing:"),
            "{idle}"
        );
    }
    assert!(
        idle["listener"]
            .as_str()
            .is_some_and(|line| line.contains("[ref=")),
        "{idle}"
    );
    writeln!(
        io::stderr(),
        "REFS_MISSING busy=returned notice=timeout listener_ref=absent idle=returned idle_ref=present elapsed_ms={}",
        busy["elapsed"]
    )?;
    client.cancel().await?;
    Ok(())
}
