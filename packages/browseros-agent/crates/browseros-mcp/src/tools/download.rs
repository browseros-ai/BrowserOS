use crate::{
    constants::DOWNLOAD_TIMEOUT,
    downloads_api::{newly_added, read_recent_downloads},
    framework::{
        ToolCtx, ToolError, ToolExecResult, ToolResult, parse_args, pending_dialog_result,
        text_result,
    },
    output_file::record_browser_output_file,
};
use browseros_core::{PageId, Ref, SessionId};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

const DESCRIPTION: &str = "\
Click an element (by ref from the last snapshot) to trigger a file download. \
The browser saves it where it saves its own downloads, renaming around an \
existing file of the same name, and it appears in the browser's downloads list. \
Returns the completed file's path on the machine running this browser, its name \
on disk, and its size.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
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
        // The download behaviour is deliberately left alone. Overriding it is the
        // only way to choose the destination, and it is also what skips the
        // browser's own target determination, which is the part that renames
        // around a collision rather than replacing. Letting the browser decide is
        // what keeps an existing file safe and its downloads list correct.
        let before = read_recent_downloads(&ctx.session)
            .await?
            .into_iter()
            .map(|record| record.id)
            .collect::<Vec<_>>();
        let page_session = ctx.session.pages.get_session(page_id.clone()).await?;
        let suggested =
            capture_download(ctx, page_id, page_session.session_id.clone(), &args.r#ref).await?;
        let after = read_recent_downloads(&ctx.session).await?;
        let record = newly_added(&before, &after, &suggested)?;
        let path = PathBuf::from(&record.filename);
        let filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| suggested.clone());
        record_browser_output_file(&ctx.output_files, path.clone()).await;
        Ok(Some(text_result(
            format!(
                "Downloaded \"{filename}\" ({} bytes) on the machine running this browser, to: {}",
                record.bytes,
                path.display()
            ),
            Some(json!({
                "page": args.page,
                "ref": args.r#ref,
                "path": record.filename,
                "filename": filename,
                "bytes": record.bytes
            })),
        )))
    })
}

async fn capture_download(
    ctx: &ToolCtx,
    page_id: PageId,
    session_id: SessionId,
    ref_id: &str,
) -> ToolExecResult<String> {
    // Subscribe before clicking so synchronous download events cannot outrun the receiver.
    let mut events = ctx.session.cdp_events();
    let input = ctx.session.input(page_id).await;
    input
        .click(&Ref(ref_id.to_string()), Default::default())
        .await?;
    let mut tracking = false;
    let mut guid = String::new();
    let mut suggested_filename = String::new();
    let timeout = tokio::time::sleep(DOWNLOAD_TIMEOUT);
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            () = ctx.cancel.cancelled() => return Err(ToolError::Cancelled),
            () = &mut timeout => {
                return Err(ToolError::message(format!(
                    "Download timed out after {}ms. If the browser is set to ask where to save each file, it is waiting on that dialog.",
                    DOWNLOAD_TIMEOUT.as_millis()
                )));
            }
            event = events.recv() => {
                let event = event.map_err(|err| ToolError::message(err.to_string()))?;
                if event.session_id.as_ref() != Some(&session_id) {
                    continue;
                }
                match event.method.as_str() {
                    // Only the first download of the click: a later one would
                    // otherwise take over the guid being tracked and the
                    // completion of this one would never be seen.
                    "Page.downloadWillBegin" if !tracking => {
                        tracking = true;
                        guid = event.params.get("guid").and_then(Value::as_str).unwrap_or_default().to_string();
                        suggested_filename = event
                            .params
                            .get("suggestedFilename")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                    }
                    "Page.downloadProgress" => {
                        if !tracking || event.params.get("guid").and_then(Value::as_str) != Some(guid.as_str()) {
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
