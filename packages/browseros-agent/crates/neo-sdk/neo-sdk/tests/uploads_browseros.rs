//! Uploads end to end on a real BrowserOS: `setInputFiles` over MCP `run` refuses Neo's own
//! secret files and hands the page every other file, a payload, and `[]`.
//!
//! ```text
//!  MCP run ─▶ browseros-claw-sdk ─ setInputFiles ─▶ the door ─▶ neo-playwright's upload
//!    ─ PrivacyHooks::secret_paths: the rig's token, CAPTCHA key, trace dir and browser profile
//!    ─ a secret, or a link to one: script / neo_secret_upload_refused, not_sent, no path named
//!    ─ any other file: Chromium hands the page its bytes, which the page lists back
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
use neo_trace::RunId;
use serde_json::{Value, json};
use tokio::time;

/// The whole drive; see `e2e_browseros.rs` for why the budget sits inside the test.
const TEST_BUDGET: Duration = Duration::from_secs(240);

/// The text of the agent's in-memory upload.
const PAYLOAD_TEXT: &str = "id,total\n7,12.50\n";

/// An agent uploading into a page whose file input reports what it holds: on each `change` the
/// page's own script lists every file's name, type and text, so the agent reads the upload back
/// through a locator. `SECRETS` (a name for each of Neo's secret files, and its path),
/// `ORDINARY` (a file of the agent's own) and `PAYLOAD_TEXT` are defined ahead of it.
const UPLOADS: &str = r#"
const html = '<title>Uploads</title><input type="file" id="files" multiple><ol id="seen"></ol>'
  + '<script>'
  + 'const input = document.getElementById("files");'
  + 'input.addEventListener("change", async () => {'
  + '  const files = await Promise.all(Array.from(input.files, async (file) =>'
  + '    ({ name: file.name, type: file.type, text: await file.text() })));'
  + '  const entry = document.createElement("li");'
  + '  entry.textContent = JSON.stringify(files);'
  + '  document.getElementById("seen").append(entry);'
  + '});'
  + '</script>';
await page.goto('data:text/html,' + encodeURIComponent(html));
const input = page.locator('#files');
const seen = page.locator('#seen li');
const refused = {};
for (const [name, path] of Object.entries(SECRETS)) {
  refused[name] = await input.setInputFiles(path).then(
    () => 'uploaded',
    ({ owner, code, delivery, message }) => ({ owner, code, delivery, message }),
  );
}
const changedByRefusals = await seen.count();
const uploads = [];
const payload = { name: 'payload.csv', mimeType: 'text/csv', buffer: Buffer.from(PAYLOAD_TEXT) };
for (const files of [ORDINARY, payload, []]) {
  await input.setInputFiles(files);
  await expect(seen).toHaveCount(uploads.length + 1);
  uploads.push(JSON.parse(await seen.nth(uploads.length).textContent()));
}
return { refused, changedByRefusals, uploads };
"#;

#[tokio::test(flavor = "multi_thread")]
async fn an_upload_of_neos_host_token_is_refused_and_an_ordinary_file_uploads() -> TestResult {
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
    let secrets = json!({
        "token": rig.token_path(),
        "captchaKey": rig.captcha_key_path(),
        "traceStore": rig.trace_path(),
        "localState": local_state,
        "linkToToken": link,
    });
    let code = format!(
        "const SECRETS = {secrets};\nconst ORDINARY = {};\nconst PAYLOAD_TEXT = {};\n{UPLOADS}",
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
    let value = result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| format!("the script returns what it saw: {result:?}"))?;

    let named = secrets.as_object().ok_or("the secrets are named")?;
    for (name, path) in named {
        let refused = value
            .pointer(&format!("/refused/{name}"))
            .ok_or_else(|| format!("the script tried {name}: {value}"))?;
        let field = |key: &str| refused.get(key).and_then(Value::as_str);
        assert_eq!(
            (field("owner"), field("code"), field("delivery")),
            (
                Some("script"),
                Some("neo_secret_upload_refused"),
                Some("not_sent")
            ),
            "{name} is one of Neo's own secret files: {value}"
        );
        let message = field("message").ok_or("a refusal has a message")?;
        let path = path.as_str().ok_or("a path is text")?;
        let file = Path::new(path)
            .file_name()
            .and_then(|file| file.to_str())
            .ok_or("a path names a file")?;
        assert!(
            !message.contains(path) && !message.contains(file),
            "{name}: the refusal names no path, so a script learns nothing of Neo's disk: {message}"
        );
    }
    assert_eq!(
        value.get("changedByRefusals"),
        Some(&json!(0)),
        "no refused file reached the page: {value}"
    );
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
