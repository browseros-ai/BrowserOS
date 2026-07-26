use crate::{
    constants::INLINE_PAGE_CONTENT_MAX_CHARS,
    framework::{
        ToolCtx, ToolExecResult, ToolResult, error_result, parse_args, pending_dialog_result,
        text_result,
    },
    output_file::write_temp_tool_output_file,
    trust_boundary::wrap_untrusted,
};
use browseros_core::{
    PageId,
    content_markdown::{ContentMarkdownOptions, build_content_markdown_expression},
};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DESCRIPTION: &str = "\
Extract page content as markdown (default), plain text, links, or console errors. \
For reading/scraping, not acting.";

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum ReadFormat {
    #[default]
    Markdown,
    Text,
    Links,
    Console,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ReadArgs {
    page: u32,
    #[serde(default)]
    format: ReadFormat,
    /// Restrict to a CSS subtree.
    selector: Option<String>,
    /// For markdown reads, include only visible viewport content.
    #[serde(rename = "viewportOnly")]
    viewport_only: Option<bool>,
    /// For markdown reads, render links as markdown links.
    #[serde(rename = "includeLinks")]
    include_links: Option<bool>,
    /// For markdown reads, include image references.
    #[serde(rename = "includeImages")]
    include_images: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvaluateResult {
    result: RemoteObject,
    exception_details: Option<ExceptionDetails>,
}

#[derive(Debug, Deserialize)]
struct RemoteObject {
    value: Option<Value>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExceptionDetails {
    text: String,
    exception: Option<RemoteObject>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<ReadArgs>(
        "read",
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
        let args: ReadArgs = parse_args(raw)?;
        let origin = ctx
            .session
            .pages
            .get_info(PageId(args.page))
            .await
            .map(|info| info.url)
            .unwrap_or_else(|| "unknown".to_string());
        if matches!(args.format, ReadFormat::Console) {
            let entries = ctx.session.page_signals.console_entries(&PageId(args.page));
            let text = if entries.is_empty() {
                "(no console errors or warnings)".to_string()
            } else {
                entries
                    .iter()
                    .map(browseros_core::ConsoleEntry::line)
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            return Ok(Some(text_result(
                wrap_untrusted(&text, &origin),
                Some(json!({
                    "page": args.page,
                    "format": format_name(&args.format),
                    "count": entries.len()
                })),
            )));
        }
        if let Some(result) = pending_dialog_result(ctx, PageId(args.page)) {
            return Ok(Some(result));
        }
        let page = ctx.session.pages.get_session(PageId(args.page)).await?;
        let expression = expression_for(&args)?;
        let result: EvaluateResult = page
            .session
            .send(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true }),
            )
            .await?;
        if let Some(exception) = result.exception_details {
            return Ok(Some(error_result(format!(
                "read: {}",
                exception_message(exception)
            ))));
        }
        let text = result
            .result
            .value
            .and_then(|value| value.as_str().map(ToString::to_string))
            .or(result.result.description)
            .unwrap_or_default();
        if text.len() <= INLINE_PAGE_CONTENT_MAX_CHARS {
            return Ok(Some(text_result(
                wrap_untrusted(if text.is_empty() { "(empty)" } else { &text }, &origin),
                Some(json!({
                    "page": args.page,
                    "format": format_name(&args.format),
                    "contentLength": text.len(),
                    "writtenToFile": false
                })),
            )));
        }
        let path = write_temp_tool_output_file(
            &ctx.output_files,
            "read",
            if matches!(args.format, ReadFormat::Markdown) {
                "md"
            } else {
                "txt"
            },
            &wrap_untrusted(&text, &origin),
        )
        .await?;
        let truncated = safe_prefix(&text, INLINE_PAGE_CONTENT_MAX_CHARS);
        Ok(Some(text_result(
            [
                wrap_untrusted(&truncated, &origin),
                format!(
                    "Content truncated at {INLINE_PAGE_CONTENT_MAX_CHARS} chars. Full content ({} chars) saved to: {}",
                    text.len(),
                    path.display()
                ),
            ]
            .join("\n\n"),
            Some(json!({
                "page": args.page,
                "format": format_name(&args.format),
                "path": path.to_string_lossy(),
                "contentLength": text.len(),
                "writtenToFile": true
            })),
        )))
    })
}

fn expression_for(args: &ReadArgs) -> ToolExecResult<String> {
    Ok(match args.format {
        ReadFormat::Markdown => build_content_markdown_expression(&ContentMarkdownOptions {
            selector: args.selector.clone(),
            viewport_only: args.viewport_only,
            include_links: args.include_links,
            include_images: args.include_images,
        }),
        ReadFormat::Text => {
            let root = root_expression(args.selector.as_deref())?;
            format!("(({root})?.innerText ?? '')")
        }
        ReadFormat::Links => {
            let root = root_expression(args.selector.as_deref())?;
            format!(
                "[...({root}?.querySelectorAll('a[href]') ?? [])].map(function(a){{return '[' + (a.textContent||'').trim() + '](' + a.href + ')'}}).join('\\n')"
            )
        }
        ReadFormat::Console => unreachable!("console reads are served from captured page signals"),
    })
}

fn root_expression(selector: Option<&str>) -> ToolExecResult<String> {
    Ok(match selector {
        Some(selector) => format!(
            "document.querySelector({})",
            serde_json::to_string(selector)?
        ),
        None => "document.body".to_string(),
    })
}

fn exception_message(exception: ExceptionDetails) -> String {
    exception
        .exception
        .and_then(|exception| exception.description)
        .unwrap_or(exception.text)
}

fn format_name(format: &ReadFormat) -> &'static str {
    match format {
        ReadFormat::Markdown => "markdown",
        ReadFormat::Text => "text",
        ReadFormat::Links => "links",
        ReadFormat::Console => "console",
    }
}

fn safe_prefix(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let mut end = max_chars;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text[..end].to_string()
}
