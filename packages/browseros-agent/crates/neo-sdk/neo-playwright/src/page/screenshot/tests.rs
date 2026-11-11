//! A document replacement can strand a native screenshot reply while its session stays alive.
//! The operation must restore its rendering state and read pixels from the replacement document.

use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use browseros_core::SessionId;
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio::{task, time};

use crate::Op;
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::page::document_watch::{DocumentWatch, Expect};
use crate::target::CallHeader;
use crate::test_support::{ProbeAnswer, SCRIPTED_MAIN, ScriptedRun, ScriptedTab, TabScript};

#[derive(Default)]
struct LostCapture {
    captures: AtomicUsize,
    started: Notify,
    document: AtomicUsize,
    prepared: AtomicUsize,
    fonts: AtomicUsize,
}

impl TabScript for LostCapture {
    fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
        ProbeAnswer::unexpected()
    }

    fn outlasts_client(&self, method: &str) -> bool {
        if method != "Page.captureScreenshot" {
            return false;
        }
        let first = self.captures.fetch_add(1, Ordering::SeqCst) == 0;
        if first {
            self.started.notify_one();
        }
        first
    }

    fn cdp_reply(&self, method: &str, _params: &Value) -> Option<Value> {
        let document = self.document.load(Ordering::SeqCst);
        match method {
            "Runtime.evaluate" => Some(json!({ "result": {
                "type": "object", "objectId": format!("g{document}"),
            } })),
            "Page.getFrameTree" => Some(json!({ "frameTree": { "frame": {
                "id": SCRIPTED_MAIN, "loaderId": format!("loader-{document}"),
                "url": "https://a.test/",
            } } })),
            "Page.captureScreenshot" => {
                let prepared = self.prepared.load(Ordering::SeqCst) == document
                    && self.fonts.load(Ordering::SeqCst) == document;
                Some(json!({ "data": if prepared { "AQID" } else { "BAUG" } }))
            }
            _ => None,
        }
    }

    fn main_world(&self, object: &str, function: &str) -> Option<Result<Value, &'static str>> {
        let document = self.document.load(Ordering::SeqCst);
        if object.starts_with('g') && object != format!("g{document}") {
            return Some(Err("Cannot find context with specified id"));
        }
        if function == include_str!("../../../assets/screenshot-prepare.js") {
            self.prepared.store(document, Ordering::SeqCst);
            Some(Ok(json!({ "result": {} })))
        } else if function.contains("document.fonts.ready") {
            self.fonts.store(document, Ordering::SeqCst);
            Some(Ok(json!({ "result": {} })))
        } else {
            None
        }
    }
}

#[tokio::test(start_paused = true)]
async fn capture_restarts_after_document_change_without_waiting_for_a_lost_reply()
-> Result<(), Box<dyn Error>> {
    let tab = ScriptedTab::new(LostCapture::default());
    let run = ScriptedRun::over(&tab).await?;
    let op = Op::from_name("page.screenshot").ok_or("screenshot is registered")?;
    let (session, mut reload_watch) = {
        let scope = OpScope::new(&run.env, op, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let session = page
            .root_session()
            .session_id()
            .cloned()
            .ok_or("the admitted page has a session")?;
        let endpoint = page.main_endpoint();
        let watch = DocumentWatch::new(
            page.browser(),
            endpoint.session,
            session.clone(),
            endpoint.frame,
        );
        (session, watch)
    };
    let shot = task::spawn(async move {
        run.playwright
            .invoke(
                &run.env,
                op,
                json!({
                    "page": run.page, "style": "body { background: blue !important }",
                    "animations": "disabled",
                }),
            )
            .await
    });
    tab.script().started.notified().await;
    let navigation = |frame: &str| {
        json!({ "frame": {
        "id": frame, "loaderId": "loader-1", "url": "https://a.test/next",
    } })
    };
    tab.emit_on(
        "Page.frameNavigated",
        navigation("unrelated-child"),
        Some(session.clone()),
    );
    tab.emit_on(
        "Page.frameNavigated",
        navigation(SCRIPTED_MAIN),
        Some(SessionId::from("another-session")),
    );
    for _turn in 0..3 {
        task::yield_now().await;
    }
    assert!(
        !shot.is_finished(),
        "another frame or session must not cancel this page's pending capture"
    );
    tab.script().document.store(1, Ordering::SeqCst);
    tab.emit_on(
        "Page.frameNavigated",
        navigation(SCRIPTED_MAIN),
        Some(session),
    );
    let data = time::timeout(Duration::from_secs(1), shot)
        .await
        .map_err(
            |_elapsed| "screenshot still waits for the old document's lost native reply",
        )???;
    assert_eq!(
        data["$neo"], "bytes",
        "the replacement capture returns image bytes to the original caller"
    );
    assert_eq!(
        data["base64"], "AQID",
        "the replacement document is prepared and its fonts are ready before pixels return"
    );
    assert_eq!(
        tab.script().captures.load(Ordering::SeqCst),
        2,
        "only the admitted document change restarts the abandoned read"
    );
    time::timeout(
        Duration::from_secs(1),
        reload_watch.arrived(Expect::NewDocument),
    )
    .await
    .map_err(|_elapsed| "capture observation stole the commit from the concurrent reload")?
    .map_err(|error| format!("concurrent reload observation failed: {error:?}"))?;
    Ok(())
}
