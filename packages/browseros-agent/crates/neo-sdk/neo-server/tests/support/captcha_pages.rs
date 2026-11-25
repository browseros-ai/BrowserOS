//! CAPTCHA-shaped pages for `neo.solveCaptcha` on the real browser: a loopback server whose top
//! documents load on `127.0.0.1` and whose `localhost` children are cross-site, so out of
//! process. Detection-isolation pages replace the main world's DOM methods after setup so a
//! detector reading the page's own world would see wrong answers; fresh-solve pages leave the main
//! world intact so the solver's flows can read and write response fields and invoke callbacks. The
//! `/provider` stub either refuses every request or mints sequential `FRESH-n` tokens using
//! `CapSolver`'s protocol.

use std::collections::HashMap;
use std::error::Error;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// Counts each call in `window.__poison` and answers wrongly.
const POISON: &str = r"(()=>{const P=window.__poison={};
const w=(o,n,key,wrong)=>{o[n]=function(...a){P[key]=(P[key]||0)+1;return wrong.apply(this,a);};};
w(Element.prototype,'querySelectorAll','Element.querySelectorAll',()=>[]);
w(Document.prototype,'querySelectorAll','Document.querySelectorAll',()=>[]);
w(Element.prototype,'querySelector','Element.querySelector',()=>null);
w(Document.prototype,'querySelector','Document.querySelector',()=>null);
w(Element.prototype,'matches','Element.matches',()=>false);
w(Element.prototype,'getBoundingClientRect','Element.getBoundingClientRect',()=>({x:0,y:0,top:0,left:0,width:0,height:0,right:0,bottom:0}));
w(Node.prototype,'contains','Node.contains',()=>false);
w(window,'getComputedStyle','window.getComputedStyle',()=>({display:'none',visibility:'hidden',opacity:'0'}));
})();";

/// The smallest PDF Chromium's viewer opens.
const PDF: &str = "%PDF-1.1\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";

/// Maximum bytes read for a single loopback request so a malformed client cannot grow memory.
const MAX_REQUEST_BYTES: usize = 65_536;

/// How the local `/provider` stub answers `CapSolver` requests.
#[derive(Clone, Copy, Debug)]
pub(super) enum ProviderMode {
    /// Refuses every request with `errorId: 1`, for tests checking which pages reach a provider.
    Reject,
    /// Answers `CapSolver`'s `createTask` and `getTaskResult` with sequential `FRESH-n` tokens.
    Mint,
}

#[derive(Default)]
struct MintState {
    next_id: usize,
    tasks: HashMap<String, MintedTask>,
}

struct MintedTask {
    solution_field: &'static str,
    token: String,
}

/// The loopback server and its provider stub.
pub(super) struct CaptchaPages {
    port: u16,
    provider_calls: Arc<AtomicUsize>,
    minted_tasks: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl CaptchaPages {
    pub(super) async fn start(mode: ProviderMode) -> Result<Self, Box<dyn Error>> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let minted_tasks = Arc::new(AtomicUsize::new(0));
        let mint_state = Arc::new(Mutex::new(MintState::default()));
        let calls = Arc::clone(&provider_calls);
        let minted = Arc::clone(&minted_tasks);
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let calls = Arc::clone(&calls);
                let minted = Arc::clone(&minted);
                let mint_state = Arc::clone(&mint_state);
                tokio::spawn(async move {
                    let Some((target, body)) = read_http_request(&mut socket).await else {
                        return;
                    };
                    let path = target.split('?').next().unwrap_or("/");
                    let (status, kind, response_body) = if path.starts_with("/provider") {
                        handle_provider(mode, path, &body, &calls, &minted, &mint_state)
                    } else if path == "/doc.pdf" {
                        ("200 OK", "application/pdf", PDF.to_owned())
                    } else if let Some(body) = page(path, port) {
                        ("200 OK", "text/html; charset=utf-8", body)
                    } else {
                        ("404 Not Found", "text/plain", "not found".to_owned())
                    };
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                        response_body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        Ok(Self {
            port,
            provider_calls,
            minted_tasks,
            task,
        })
    }

    /// A page's address on the top-level host.
    pub(super) fn url(&self, route: &str) -> String {
        format!("http://127.0.0.1:{}{route}", self.port)
    }

    /// The provider stub, for `BROWSEROS_CAPSOLVER_API_URL`.
    pub(super) fn provider(&self) -> String {
        self.url("/provider")
    }

    /// Total HTTP requests handled under `/provider`.
    pub(super) fn provider_calls(&self) -> usize {
        self.provider_calls.load(Ordering::SeqCst)
    }

    /// Number of tasks minted by `createTask` in [`ProviderMode::Mint`].
    pub(super) fn minted_tasks(&self) -> usize {
        self.minted_tasks.load(Ordering::SeqCst)
    }
}

impl Drop for CaptchaPages {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Reads an HTTP/1.1 request head and its full `Content-Length` body so split TCP writes from
/// `reqwest` do not truncate JSON payloads sent to `/provider`.
async fn read_http_request(socket: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut head_end = None;
    let mut content_length = 0_usize;

    loop {
        if let Some(split) = head_end {
            let needed = split + 4 + content_length;
            if buffer.len() >= needed {
                let head = String::from_utf8_lossy(buffer.get(..split)?);
                let target = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let body = buffer.get(split + 4..needed)?.to_vec();
                return Some((target, body));
            }
        }
        if buffer.len() >= MAX_REQUEST_BYTES {
            return None;
        }
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            let split = head_end?;
            let head = String::from_utf8_lossy(buffer.get(..split)?);
            let target = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let body = buffer.get(split + 4..)?.to_vec();
            return Some((target, body));
        }
        buffer.extend_from_slice(chunk.get(..read)?);
        if head_end.is_none()
            && let Some(split) = buffer.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let head = String::from_utf8_lossy(buffer.get(..split)?);
            content_length = head
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            head_end = Some(split);
        }
    }
}

/// Handles one `/provider` request under the configured stub mode.
fn handle_provider(
    mode: ProviderMode,
    path: &str,
    body: &[u8],
    calls: &AtomicUsize,
    minted: &AtomicUsize,
    state: &Mutex<MintState>,
) -> (&'static str, &'static str, String) {
    calls.fetch_add(1, Ordering::SeqCst);
    match mode {
        ProviderMode::Reject => (
            "200 OK",
            "application/json",
            r#"{"errorId":1,"errorCode":"TEST_PROVIDER_MUST_NOT_BE_CALLED"}"#.to_owned(),
        ),
        ProviderMode::Mint => handle_mint_provider(path, body, minted, state),
    }
}

/// Answers `CapSolver`'s `createTask` and `getTaskResult` with sequential `FRESH-n` tokens.
/// `CapsolverClient::normalize` expects `solution.token` for Turnstile and
/// `solution.gRecaptchaResponse` for reCAPTCHA.
fn handle_mint_provider(
    path: &str,
    body: &[u8],
    minted: &AtomicUsize,
    state: &Mutex<MintState>,
) -> (&'static str, &'static str, String) {
    let Ok(payload) = serde_json::from_slice::<Value>(body) else {
        return (
            "400 Bad Request",
            "application/json",
            json!({
                "errorId": 1,
                "errorCode": "INVALID_JSON",
                "errorDescription": "request body was not valid JSON",
            })
            .to_string(),
        );
    };
    if path == "/provider/createTask" {
        let task_type = payload.pointer("/task/type").and_then(Value::as_str);
        let solution_field = match task_type {
            Some("AntiTurnstileTaskProxyLess") => "token",
            Some("ReCaptchaV2TaskProxyLess" | "ReCaptchaV2EnterpriseTaskProxyLess") => {
                "gRecaptchaResponse"
            }
            _ => {
                return (
                    "200 OK",
                    "application/json",
                    json!({
                        "errorId": 1,
                        "errorCode": "UNSUPPORTED_STUB_TASK",
                        "errorDescription": format!("unexpected task type: {task_type:?}"),
                    })
                    .to_string(),
                );
            }
        };
        let task_id = {
            let mut guard = state.lock().unwrap_or_else(PoisonError::into_inner);
            guard.next_id += 1;
            let id = guard.next_id;
            let task_id = format!("stub-task-{id}");
            let token = format!("FRESH-{id}");
            guard.tasks.insert(
                task_id.clone(),
                MintedTask {
                    solution_field,
                    token,
                },
            );
            task_id
        };
        minted.fetch_add(1, Ordering::SeqCst);
        (
            "200 OK",
            "application/json",
            json!({ "errorId": 0, "taskId": task_id }).to_string(),
        )
    } else if path == "/provider/getTaskResult" {
        let task_id = payload.get("taskId").and_then(Value::as_str);
        let entry = task_id.and_then(|id| {
            let guard = state.lock().unwrap_or_else(PoisonError::into_inner);
            guard
                .tasks
                .get(id)
                .map(|task| (task.solution_field, task.token.clone()))
        });
        if let Some((field, token)) = entry {
            (
                "200 OK",
                "application/json",
                json!({
                    "errorId": 0,
                    "status": "ready",
                    "solution": { field: token },
                })
                .to_string(),
            )
        } else {
            (
                "200 OK",
                "application/json",
                json!({
                    "errorId": 1,
                    "errorCode": "ERROR_TASKID_INVALID",
                    "errorDescription": "unknown taskId",
                })
                .to_string(),
            )
        }
    } else {
        (
            "404 Not Found",
            "application/json",
            json!({ "errorId": 1, "errorCode": "UNKNOWN_ENDPOINT" }).to_string(),
        )
    }
}

/// neo-captcha's own `PerimeterX` pages, poisoned after their setup like every page here.
/// The poison goes before the document's last `</body>`.
fn testbed(page: &str) -> String {
    let end = page.rfind("</body>").unwrap_or(page.len());
    let (head, tail) = page.split_at(end);
    format!("{head}<script>{POISON}</script>{tail}")
}

fn html(title: &str, body: &str, setup: &str) -> String {
    format!(
        "<!doctype html><title>{title}</title><body>{body}<script>{setup}\n{POISON}</script></body>"
    )
}

/// Unpoisoned Google reCAPTCHA v2 test checkbox widget inside a guarded search form so
/// `e2e_browseros` can prove `neo.solveCaptcha` dismisses an open `bframe` challenge overlay
/// covering the Search button without hiding the form or widget.
const RECAPTCHA_OVERLAY_BODY: &str = r#"<form id="search-form">
  <div class="g-recaptcha" data-sitekey="6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI" data-callback="onRecaptchaToken"></div>
  <button id="search" type="submit" style="margin-top:16px;padding:10px 24px">Search</button>
</form>
<script>
window.__callbackTokens = [];
window.__acceptedSubmits = 0;
window.__submittedTokens = [];
window.onRecaptchaToken = function(token) {
  window.__callbackTokens.push(token);
};
document.getElementById('search-form').addEventListener('submit', function(event) {
  event.preventDefault();
  const field = document.querySelector('textarea[name="g-recaptcha-response"]');
  const token = field ? field.value.trim() : '';
  if (token.length > 0) {
    window.__acceptedSubmits += 1;
    window.__submittedTokens.push(token);
  }
});
</script>
<script src="https://www.google.com/recaptcha/api.js" async defer></script>"#;

fn unpoisoned_html(title: &str, body: &str) -> String {
    format!("<!doctype html><title>{title}</title><body>{body}</body>")
}

fn page(path: &str, port: u16) -> Option<String> {
    let cross = format!("http://localhost:{port}");
    Some(match path {
        "/plain-child" => html("child", "<p>An ordinary child document.</p>", ""),
        "/clear" => html(
            "clear",
            &format!(
                r#"<h1>No CAPTCHA here</h1>
<iframe src="/plain-child" style="width:300px;height:80px"></iframe>
<iframe src="{cross}/plain-child" style="width:300px;height:80px"></iframe>"#
            ),
            "",
        ),
        "/awkward" => html(
            "awkward frames",
            &format!(
                r#"<h1>No CAPTCHA here either</h1>
<iframe src="about:blank" style="width:200px;height:60px"></iframe>
<iframe srcdoc="<p>inline document</p>" style="width:200px;height:60px"></iframe>
<iframe sandbox src="{cross}/plain-child" style="width:200px;height:60px"></iframe>
<iframe src="data:text/html,%3Cp%3Edata%20document%3C%2Fp%3E" style="width:200px;height:60px"></iframe>
<embed src="/doc.pdf" type="application/pdf" style="width:200px;height:200px">"#
            ),
            "",
        ),
        "/aws-child" => html(
            "aws child",
            r#"<p>Verify you are human</p><div id="gate"></div>"#,
            r#"window.gokuProps = {key: 'child-aws-key', iv: 'iv', context: 'context'};
document.getElementById('gate').attachShadow({mode: 'closed'}).innerHTML =
  '<div id="captcha-container" style="width:300px;height:100px">AWS WAF<input name="captcha_voucher" value="already-passed"></div>';"#,
        ),
        "/aws-oopif" => html(
            "aws oopif",
            &format!(
                r#"<iframe src="{cross}/aws-child" style="width:360px;height:180px"></iframe>"#
            ),
            "",
        ),
        "/aws-same" => html(
            "aws same-process",
            r#"<iframe src="/aws-child" style="width:360px;height:180px"></iframe>"#,
            "",
        ),
        "/turnstile" => html(
            "turnstile",
            r#"<form><div class="cf-turnstile" data-sitekey="0x4AAAAAAAsynthetic" style="width:300px;height:65px">
<div id="widget" style="width:300px;height:65px"></div>
<input type="hidden" name="cf-turnstile-response" value="already-passed-token"></div></form>"#,
            &format!(
                r#"document.getElementById('widget').attachShadow({{mode: 'closed'}}).innerHTML =
  '<iframe src="{cross}/plain-child" style="width:300px;height:65px;border:0"></iframe>';"#
            ),
        ),
        "/hcaptcha" => html(
            "hcaptcha",
            r#"<div id="host"></div>"#,
            r#"document.getElementById('host').attachShadow({mode: 'open'}).innerHTML =
  '<div class="h-captcha" data-sitekey="10000000-ffff-ffff-ffff-000000000001" style="width:300px;height:100px">hCaptcha</div>';"#,
        ),
        // PerimeterX's press-and-hold check over the page, and its frame waiting hidden.
        "/perimeterx" => testbed(include_str!(
            "../../../neo-captcha/testbed/fixtures/pages/perimeterx-modal.html"
        )),
        "/perimeterx-waiting" => testbed(include_str!(
            "../../../neo-captcha/testbed/fixtures/pages/perimeterx-waiting.html"
        )),
        // Unpoisoned stale-token Turnstile widget without any vendor script: the hidden response
        // field starts with a stale token and a callback records every token Neo applies.
        "/turnstile-stale" => unpoisoned_html(
            "Turnstile stale token",
            r#"<p id="status">Verification failed</p>
<form>
  <div class="cf-turnstile" data-sitekey="0x4AAAAAAAsyntheticStale" data-callback="onTurnstileToken" style="width:300px;height:65px">
    <input type="hidden" name="cf-turnstile-response" value="STALE-TOKEN">
  </div>
</form>
<script>
window.__callbackTokens = [];
window.onTurnstileToken = function(token) {
  window.__callbackTokens.push(token);
};
</script>"#,
        ),
        // Unpoisoned Google reCAPTCHA v2 test checkbox widget (public always-pass test key) with a
        // callback recording every token passed to it.
        "/recaptcha-stale" => unpoisoned_html(
            "reCAPTCHA v2 stale token",
            r#"<p id="status">Verification failed</p>
<form>
  <div class="g-recaptcha" data-sitekey="6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI" data-callback="onRecaptchaToken"></div>
</form>
<script>
window.__callbackTokens = [];
window.onRecaptchaToken = function(token) {
  window.__callbackTokens.push(token);
};
</script>
<script src="https://www.google.com/recaptcha/api.js" async defer></script>"#,
        ),
        "/recaptcha-overlay" => unpoisoned_html("reCAPTCHA v2 overlay", RECAPTCHA_OVERLAY_BODY),
        _ => return None,
    })
}
