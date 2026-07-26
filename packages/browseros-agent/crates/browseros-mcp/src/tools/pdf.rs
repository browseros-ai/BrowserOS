use crate::{
    framework::{ToolCtx, ToolExecResult, ToolResult, parse_args, text_result},
    output_file::write_temp_tool_output_binary_file,
};
use base64::Engine;
use browseros_core::PageId;
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DESCRIPTION: &str = "\
Print the page to a PDF and save it to a BrowserOS output file, returning the path. \
Use for archiving or reading a page as a document; prefer read for extracting text.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct PdfArgs {
    /// Page id from `tabs`.
    page: u32,
    /// Use landscape orientation.
    landscape: Option<bool>,
    /// Compatibility alias for printBackground.
    background: Option<bool>,
    /// Print background graphics.
    #[serde(rename = "printBackground")]
    print_background: Option<bool>,
    /// Use CSS page size when the page defines one.
    #[serde(default)]
    #[serde(rename = "preferCSSPageSize")]
    prefer_css_page_size: bool,
}

#[derive(Debug, Deserialize)]
struct PrintToPdfResult {
    data: String,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<PdfArgs>(
        "pdf",
        DESCRIPTION,
        Some(super::read_only_annotations()),
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: PdfArgs = parse_args(raw)?;
        let page = ctx.session.pages.get_session(PageId(args.page)).await?;
        let result: PrintToPdfResult = page
            .session
            .send(
                "Page.printToPDF",
                json!({
                    "landscape": args.landscape.unwrap_or(false),
                    "printBackground": args.print_background.or(args.background).unwrap_or(true),
                    "preferCSSPageSize": args.prefer_css_page_size
                }),
            )
            .await?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(result.data)
            .map_err(|err| crate::framework::ToolError::message(err.to_string()))?;
        let path =
            write_temp_tool_output_binary_file(&ctx.output_files, "pdf", "pdf", &bytes).await?;
        Ok(Some(text_result(
            format!(
                "Saved page {} as PDF ({} bytes) to: {}",
                args.page,
                bytes.len(),
                path.display()
            ),
            Some(
                json!({ "page": args.page, "path": path.to_string_lossy(), "bytes": bytes.len() }),
            ),
        )))
    })
}
