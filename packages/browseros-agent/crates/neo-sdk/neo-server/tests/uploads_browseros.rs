//! Uploads end to end on a real BrowserOS: `setInputFiles` over MCP `run` hands the page
//! host-owned paths, an ordinary file, a payload, and `[]`.
//!
//! ```text
//!  MCP run ─▶ browseros-claw-sdk ─ setInputFiles ─▶ the door ─▶ neo-playwright's upload
//!    ─ host-owned files and a symlink: the page reports the uploaded file's name and size
//!    ─ an ordinary file and payload: the page reports their bytes; [] clears the input
//! ```
//!
//! Like `e2e_browseros.rs`, it runs only when `CLAW_SDK_E2E_BROWSER` names a BrowserOS binary.

#[path = "support/browseros_rig.rs"]
pub mod browseros_rig;

use std::error::Error;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::time::Duration;

use browseros_rig::{CaptchaKey, PATIENCE, Rig, TestResult, browser_or_skip, outcome, run};
use neo_server::test_support;
use neo_trace::RunId;
use serde_json::{Value, json};
use tokio::time;

/// The whole drive; see `e2e_browseros.rs` for why the budget sits inside the test.
const TEST_BUDGET: Duration = Duration::from_secs(240);

/// The text of the agent's in-memory upload.
const PAYLOAD_TEXT: &str = "id,total\n7,12.50\n";

/// An agent uploading into a page whose file input reports what it holds: on each `change` the
/// page's own script lists every file's name, type and text, so the agent reads the upload back
/// through a locator. Host-owned files use a separate input that reports only names and sizes,
/// so the test's result stays independent of changing trace/profile bytes. `HOST_FILES`,
/// `ORDINARY` (a file of the agent's own) and `PAYLOAD_TEXT` are defined ahead of it.
const UPLOADS: &str = r#"
const html = '<title>Uploads</title>'
  + '<input type="file" id="host-files" multiple><ol id="host-seen"></ol>'
  + '<input type="file" id="files" multiple><ol id="seen"></ol>'
  + '<script>'
  + 'for (const [inputId, listId] of [["host-files", "host-seen"], ["files", "seen"]]) {'
  + 'const input = document.getElementById(inputId);'
  + 'input.addEventListener("change", async () => {'
  + '  const files = await Promise.all(Array.from(input.files, async (file) =>'
  + '    inputId === "host-files" ? { name: file.name, size: file.size }'
  + '      : { name: file.name, type: file.type, text: await file.text() }));'
  + '  const entry = document.createElement("li");'
  + '  entry.textContent = JSON.stringify(files);'
  + '  document.getElementById(listId).append(entry);'
  + '});'
  + '}'
  + '</script>';
await page.goto('data:text/html,' + encodeURIComponent(html));
const hostInput = page.locator('#host-files');
const hostSeen = page.locator('#host-seen li');
const hostUploads = {};
let hostCount = 0;
for (const [name, path] of Object.entries(HOST_FILES)) {
  await hostInput.setInputFiles(path);
  await expect(hostSeen).toHaveCount(++hostCount);
  hostUploads[name] = JSON.parse(await hostSeen.nth(hostCount - 1).textContent());
}
const input = page.locator('#files');
const seen = page.locator('#seen li');
const uploads = [];
const payload = { name: 'payload.csv', mimeType: 'text/csv', buffer: Buffer.from(PAYLOAD_TEXT) };
for (const files of [ORDINARY, payload, []]) {
  await input.setInputFiles(files);
  await expect(seen).toHaveCount(uploads.length + 1);
  uploads.push(JSON.parse(await seen.nth(uploads.length).textContent()));
}
return { hostUploads, uploads };
"#;

#[tokio::test(flavor = "multi_thread")]
async fn host_owned_paths_and_payloads_upload_and_an_empty_list_clears_the_input() -> TestResult {
    let Some(browser) = browser_or_skip() else {
        return Ok(());
    };
    let rig = Rig::start(&browser, CaptchaKey::Placeholder)?;
    let failure = match time::timeout(TEST_BUDGET, uploads(&rig)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(_elapsed) => format!("the drive did not finish within {TEST_BUDGET:?}"),
    };
    Err(format!("{failure}\n{}", rig.logs()).into())
}

async fn uploads(rig: &Rig) -> TestResult {
    rig.until_ready().await?;
    // The agent's files, in a directory of their own: the rig's root holds its CAPTCHA key.
    let own = tempfile::tempdir()?;
    let ordinary = own.path().join("ordinary.txt");
    let marker = format!("an ordinary file, {}", RunId::new());
    fs::write(&ordinary, &marker)?;
    let link = own.path().join("token-link");
    symlink(rig.token_path(), &link)?;
    let local_state = rig.profile_dir().join("Local State");
    until_written(&local_state).await?;
    let host_files = json!({
        "token": rig.token_path(),
        "captchaKey": rig.captcha_key_path(),
        "traceStore": rig.trace_path(),
        "localState": local_state,
        "linkToToken": link,
    });
    let code = format!(
        "const HOST_FILES = {host_files};\nconst ORDINARY = {};\nconst PAYLOAD_TEXT = {};\n{UPLOADS}",
        json!(ordinary),
        json!(PAYLOAD_TEXT),
    );
    let client = rig.client().await?;
    let result = run(&client, &code, &json!({ "taskId": "uploads" })).await?;
    client.cancel().await?;
    assert_eq!(
        outcome(&result)?.get("status"),
        Some(&json!("ok")),
        "{result:?}"
    );
    let value = test_support::run_value(&result)?;

    let named = host_files.as_object().ok_or("the host files are named")?;
    for (name, path) in named {
        let uploaded = value
            .pointer(&format!("/hostUploads/{name}"))
            .and_then(Value::as_array)
            .ok_or_else(|| format!("the script uploaded {name}: {value}"))?;
        assert_eq!(uploaded.len(), 1, "{name} uploads one file: {value}");
        let uploaded = uploaded.first().ok_or("the input holds its file")?;
        let path = path.as_str().ok_or("a path is text")?;
        let file = Path::new(path)
            .file_name()
            .and_then(|file| file.to_str())
            .ok_or("a path names a file")?;
        assert_eq!(
            uploaded.get("name"),
            Some(&json!(file)),
            "{name} reaches the page under its supplied name: {value}"
        );
        let size = uploaded
            .get("size")
            .and_then(Value::as_u64)
            .ok_or("the uploaded file has a size")?;
        assert!(
            size > 0,
            "{name} reaches the page with its contents: {value}"
        );
    }
    assert_eq!(
        value.get("uploads"),
        Some(&json!([
            [{ "name": "ordinary.txt", "type": "text/plain", "text": marker }],
            [{ "name": "payload.csv", "type": "text/csv", "text": PAYLOAD_TEXT }],
            [],
        ])),
        "the agent's own file and its payload reach the page whole, and [] clears the input: \
         {value}"
    );
    Ok(())
}

/// Returns once `path` exists. Chromium writes a fresh profile's `Local State` on its own timer,
/// after the browser link is up.
async fn until_written(path: &Path) -> Result<(), Box<dyn Error>> {
    time::timeout(PATIENCE, async {
        while !path.exists() {
            time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_elapsed| format!("{} was not written within {PATIENCE:?}", path.display()).into())
}
