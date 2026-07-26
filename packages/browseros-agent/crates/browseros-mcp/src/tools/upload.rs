use crate::framework::{
    ToolCtx, ToolExecResult, ToolResult, error_result, parse_args, pending_dialog_result,
    text_result,
};
use browseros_core::{PageId, Ref};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DESCRIPTION: &str = "\
Set local file path(s) on a file input using a ref from the last snapshot. \
Use for <input type=\"file\"> upload flows; files must exist on the server filesystem.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct UploadArgs {
    /// Page id from `tabs`.
    page: u32,
    /// Ref of the <input type="file"> element, e.g. "e12".
    r#ref: String,
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
        let files = args
            .files
            .unwrap_or_else(|| args.file.clone().into_iter().collect::<Vec<_>>());
        if files.is_empty() {
            return Ok(Some(error_result("upload: provide file or files[].")));
        }
        ctx.session
            .input(PageId(args.page))
            .await
            .upload_file_by_ref(&Ref(args.r#ref.clone()), files.clone())
            .await?;
        Ok(Some(text_result(
            format!("Uploaded {} file(s) to {}", files.len(), args.r#ref),
            Some(json!({
                "page": args.page,
                "ref": args.r#ref,
                "files": files,
                "uploaded": files.len()
            })),
        )))
    })
}
