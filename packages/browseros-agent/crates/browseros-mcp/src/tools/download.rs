use crate::{
    framework::{
        ToolCtx, ToolError, ToolExecResult, ToolResult, parse_args, pending_dialog_result,
        text_result,
    },
    output_file::record_browser_output_file,
};
use browseros_core::{PageId, Ref};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

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
        let transfer = ctx.browser.files();
        let reference = Ref(args.r#ref.clone());
        let record = tokio::select! {
            () = ctx.cancel.cancelled() => return Err(ToolError::Cancelled),
            record = transfer.download(page_id, &reference) => record?,
        };
        let path = record.path;
        let filename = record.filename;
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
                "path": path.to_string_lossy(),
                "filename": filename,
                "bytes": record.bytes
            })),
        )))
    })
}
