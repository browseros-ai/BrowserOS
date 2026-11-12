//! CAPTCHA-shaped pages for `neo.solveCaptcha` on the real browser: a loopback server whose top
//! documents load on `127.0.0.1` and whose `localhost` children are cross-site, so out of
//! process. Every page replaces the main world's DOM methods after its setup, so a detector that
//! scanned the page's own world would read wrong answers. A provider stub counts every request.

use std::error::Error;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
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

/// The loopback server and its provider stub.
pub(super) struct CaptchaPages {
    port: u16,
    provider_calls: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl CaptchaPages {
    pub(super) async fn start() -> Result<Self, Box<dyn Error>> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&provider_calls);
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let calls = Arc::clone(&calls);
                tokio::spawn(async move {
                    let mut request = vec![0; 16384];
                    let Ok(read) = socket.read(&mut request).await else {
                        return;
                    };
                    let text = String::from_utf8_lossy(request.get(..read).unwrap_or_default());
                    let target = text.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    let path = target.split('?').next().unwrap_or("/");
                    let (status, kind, body) = if path.starts_with("/provider") {
                        calls.fetch_add(1, Ordering::SeqCst);
                        (
                            "200 OK",
                            "application/json",
                            r#"{"errorId":1,"errorCode":"TEST_PROVIDER_MUST_NOT_BE_CALLED"}"#
                                .to_owned(),
                        )
                    } else if path == "/doc.pdf" {
                        ("200 OK", "application/pdf", PDF.to_owned())
                    } else if let Some(body) = page(path, port) {
                        ("200 OK", "text/html; charset=utf-8", body)
                    } else {
                        ("404 Not Found", "text/plain", "not found".to_owned())
                    };
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        Ok(Self {
            port,
            provider_calls,
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

    pub(super) fn provider_calls(&self) -> usize {
        self.provider_calls.load(Ordering::SeqCst)
    }
}

impl Drop for CaptchaPages {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn html(title: &str, body: &str, setup: &str) -> String {
    format!(
        "<!doctype html><title>{title}</title><body>{body}<script>{setup}\n{POISON}</script></body>"
    )
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
        _ => return None,
    })
}
