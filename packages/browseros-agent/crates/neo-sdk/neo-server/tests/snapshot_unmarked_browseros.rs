//! A busy main thread costs a snapshot no refs: what Chromium reports clickable comes with the
//! APC tree (apc-engine-design 3.5), so the snapshot waits out the busy frame, prints no
//! `# refs missing:` line, and keeps the script-only element's ref; an idle read of the same DOM
//! prints the same, without relying on malformed page strings or on the shared protocol client
//! dropping them.

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

/// Prime both worlds before starting the hold. The busy evaluate stays in flight until the
/// snapshot returns: awaiting it first would consume the hold before the capture starts.
const BUSY_PAGE: &str = r#"
await page.goto('data:text/html,'+encodeURIComponent(`<meta charset=utf-8>
<button>Native action</button><div id=listener style="display:inline-block;padding:8px">Script only</div>
<script>document.getElementById('listener').addEventListener('click',()=>{});</script>`));
await page.ariaSnapshotJSON();
await page.evaluate(()=>0);
const busy=page.evaluate(()=>{
 const until=performance.now()+3000;while(performance.now()<until){}
});
await new Promise(resolve=>setTimeout(resolve,200));
const at=Date.now();const snapshot=await page.ariaSnapshot();const elapsed=Date.now()-at;
await busy;
return {snapshot,elapsed,listener:snapshot.split('\n').find(l=>l.includes('Script only'))};
"#;

#[tokio::test(flavor = "multi_thread")]
async fn busy_snapshot_waits_for_the_tree_and_keeps_script_only_refs() -> TestResult {
    let Some(browser) = browseros_rig::browser_or_skip() else {
        return Ok(());
    };
    // The rig runs hosted mode, which emits no console events, so this row cannot use them.
    // The bare script timer sends no renderer request; page.waitForTimeout refreshes frames.
    // No elapsed-time assertion depends on machine speed.
    let rig = Rig::start(&browser, CaptchaKey::Absent)?;
    rig.until_ready().await?;
    let client = rig.client().await?;
    let busy = value(&client, "snapshot-check-busy", BUSY_PAGE).await?;
    let snapshot = busy["snapshot"]
        .as_str()
        .ok_or("the busy snapshot returned text")?;
    assert!(!snapshot.contains("# refs missing:"), "{busy}");
    let listener = busy["listener"]
        .as_str()
        .ok_or("script-only content remains listed")?;
    assert!(listener.contains("[ref="), "{listener}");
    // The busy snapshot's tree needs the main thread: its return proves the loop has ended.
    // This idle read needs no further wait and observes the same DOM.
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
        "REFS_MISSING busy=returned notice=none listener_ref=present idle=returned idle_ref=present elapsed_ms={}",
        busy["elapsed"]
    )?;
    client.cancel().await?;
    Ok(())
}
