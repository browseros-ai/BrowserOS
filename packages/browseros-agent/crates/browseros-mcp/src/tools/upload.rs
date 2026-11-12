use crate::framework::{
    ToolCtx, ToolError, ToolExecResult, ToolResult, error_result, parse_args,
    pending_dialog_result, text_result,
};
use browseros_core::{PageId, Ref};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DESCRIPTION: &str = "\
Select local files on a file input or upload button using a snapshot ref. \
Omit ref to finish an already-pending file chooser. files=[] clears the selection. \
Files must exist on the browser/server filesystem. This selects files; the website handles submission.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UploadArgs {
    /// Page id from `tabs`.
    page: u32,
    /// Ref of the file input or upload button; omit for the pending chooser.
    r#ref: Option<String>,
    /// Single local file path to upload.
    file: Option<String>,
    /// Local file paths to upload.
    files: Option<Vec<String>>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<UploadArgs>("upload", DESCRIPTION, None, handler)
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: UploadArgs = parse_args(raw)?;
        if let Some(result) = pending_dialog_result(ctx, PageId(args.page)) {
            return Ok(Some(result));
        }
        if args.file.is_none() && args.files.is_none() {
            return Ok(Some(error_result("upload: provide file or files[].")));
        }
        let files = args
            .files
            .unwrap_or_else(|| args.file.clone().into_iter().collect::<Vec<_>>());
        let transfer = ctx.browser.files();
        let reference = args.r#ref.clone().map(Ref);
        tokio::select! {
            () = ctx.cancel.cancelled() => return Err(ToolError::Cancelled),
            selected = transfer.upload(PageId(args.page), reference.as_ref(), &files) => selected?,
        }
        Ok(Some(text_result(
            format!("Selected {} file(s) on page {}", files.len(), args.page),
            Some(json!({
                "page": args.page,
                "ref": args.r#ref,
                "files": files,
                "uploaded": files.len()
            })),
        )))
    })
}
