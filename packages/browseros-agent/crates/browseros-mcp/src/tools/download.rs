use crate::{
    constants::DOWNLOAD_TIMEOUT,
    framework::{
        ToolCtx, ToolError, ToolExecResult, ToolResult, parse_args, pending_dialog_result,
        text_result,
    },
    output_file::{create_download_output_dir, record_browser_output_file},
};
use browseros_core::{PageId, Ref, SessionId};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

const DESCRIPTION: &str = "\
Click an element (by ref from the last snapshot) to trigger a file download, \
and save it to a BrowserOS output file. Returns the saved path and filename.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct DownloadArgs {
    /// Page id from `tabs`.
    page: u32,
    /// Ref of the element that triggers the download, e.g. "e12".
    r#ref: String,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<DownloadArgs>("download", DESCRIPTION, None, handler)
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: DownloadArgs = parse_args(raw)?;
        let page_id = PageId(args.page);
        if let Some(result) = pending_dialog_result(ctx, page_id.clone()) {
            return Ok(Some(result));
        }
        let page_session = ctx.session.pages.get_session(page_id.clone()).await?;
        let download_dir = create_download_output_dir().await?;
        page_session
            .session
            .send_value(
                "Page.setDownloadBehavior",
                json!({ "behavior": "allow", "downloadPath": download_dir }),
            )
            .await?;
        let capture = capture_download(
            ctx,
            page_id,
            page_session.session_id.clone(),
            &args.r#ref,
            &download_dir,
        )
        .await;
        let _ = page_session
            .session
            .send_value("Page.setDownloadBehavior", json!({ "behavior": "default" }))
            .await;
        let filename = capture?;
        let path = download_dir.join(&filename);
        record_browser_output_file(&ctx.output_files, path.clone()).await;
        Ok(Some(text_result(
            format!("Downloaded \"{filename}\" to: {}", path.display()),
            Some(json!({
                "page": args.page,
                "ref": args.r#ref,
                "path": path.to_string_lossy(),
                "filename": filename
            })),
        )))
    })
}

async fn capture_download(
    ctx: &ToolCtx,
    page_id: PageId,
    session_id: SessionId,
    ref_id: &str,
    _download_dir: &PathBuf,
) -> ToolExecResult<String> {
    // Subscribe before clicking so synchronous download events cannot outrun the receiver.
    let mut events = ctx.session.cdp_events();
    let input = ctx.session.input(page_id).await;
    input
        .click(&Ref(ref_id.to_string()), Default::default())
        .await?;
    let mut guid = String::new();
    let mut suggested_filename = String::new();
    let timeout = tokio::time::sleep(DOWNLOAD_TIMEOUT);
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            () = ctx.cancel.cancelled() => return Err(ToolError::Cancelled),
            () = &mut timeout => {
                return Err(ToolError::message(format!(
                    "Download timed out after {}ms",
                    DOWNLOAD_TIMEOUT.as_millis()
                )));
            }
            event = events.recv() => {
                let event = event.map_err(|err| ToolError::message(err.to_string()))?;
                if event.session_id.as_ref() != Some(&session_id) {
                    continue;
                }
                match event.method.as_str() {
                    "Page.downloadWillBegin" => {
                        guid = event.params.get("guid").and_then(Value::as_str).unwrap_or_default().to_string();
                        suggested_filename = event
                            .params
                            .get("suggestedFilename")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                    }
                    "Page.downloadProgress" => {
                        if event.params.get("guid").and_then(Value::as_str) != Some(guid.as_str()) {
                            continue;
                        }
                        match event.params.get("state").and_then(Value::as_str) {
                            Some("completed") => {
                                if suggested_filename.is_empty() {
                                    return Err(ToolError::message("Download completed without suggested filename"));
                                }
                                return Ok(suggested_filename);
                            }
                            Some("canceled") => return Err(ToolError::message("Download was canceled")),
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
