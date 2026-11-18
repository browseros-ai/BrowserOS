//! External drops preserve host-file MIME types, bytes and `DataTransfer` entry ordering.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::fs;
use std::time::Duration;

use browseros_rig::{CaptchaKey, Rig, TestResult, browser_or_skip, outcome, run};
use serde_json::json;
use tokio::time;

#[tokio::test(flavor = "multi_thread")]
async fn external_drop_preserves_files_and_mime_data() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let rig = Rig::start(&browser, CaptchaKey::Placeholder)?;
    time::timeout(Duration::from_secs(240), drop_data(&rig)).await??;
    Ok(())
}

async fn drop_data(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    let files = tempfile::tempdir()?;
    let xml = files.path().join("note.xml");
    let markdown = files.path().join("note.md");
    fs::write(&xml, "<note/>")?;
    fs::write(&markdown, "# note")?;
    // These extensions differ between common Rust MIME tables and Playwright's
    // table. The two text keys also alias one DataTransfer format, so order matters.
    let code = format!(
        r#"
await page.setContent(`<div id="zone" style="width:100px;height:100px"></div>
<script>
const zone = document.getElementById('zone');
zone.addEventListener('dragover', event => event.preventDefault());
zone.addEventListener('drop', async event => {{
  event.preventDefault();
  const files = await Promise.all(Array.from(event.dataTransfer.files, async file =>
    ({{ name: file.name, type: file.type, text: await file.text() }})));
  window.dropped = {{ files, text: event.dataTransfer.getData('text/plain') }};
}});
</script>`);
await page.locator('#zone').drop({{
  files: [{xml}, {markdown}], data: {{ 'text/plain': 'first', text: 'last' }}
}});
return await (await page.waitForFunction(() => window.dropped)).jsonValue();
"#,
        xml = json!(xml),
        markdown = json!(markdown),
    );
    let client = rig.client().await?;
    let result = run(&client, &code, &json!({ "taskId": "drop" })).await?;
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
            "files": [
                { "name": "note.xml", "type": "application/xml", "text": "<note/>" },
                { "name": "note.md", "type": "text/markdown", "text": "# note" }
            ],
            "text": "last"
        })),
        "{result:?}"
    );
    Ok(())
}
