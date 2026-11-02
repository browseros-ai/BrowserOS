//! Reads immutable, session-owned outputs without reaching the browser or replaying actions.
use crate::{
    framework::{ToolCtx, ToolExecResult, ToolResult, parse_args, text_result},
    output_store::{MAX_READ_BYTES, OutputStore},
    trust_boundary::wrap_untrusted,
};
use base64::Engine;
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Encoding {
    Text,
    Base64,
    Image,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ReadOutputArgs {
    output_id: String,
    /// Omit to list the saved call's parts. Use a name returned in output.parts.
    part: Option<String>,
    /// UTF-8 byte offset from the previous response's nextOffset (default 0).
    #[serde(default)]
    offset: u64,
    /// Max bytes per chunk; default 16384, clamped to 4..=65536.
    limit: Option<usize>,
    /// Defaults to text for text/JSON, image for screenshots, base64 for other files.
    /// Use base64 to transfer exact binary chunks to an importer. image is bounded to 2 MiB.
    encoding: Option<Encoding>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<ReadOutputArgs>(
        "read_output",
        "Read a saved Neo tool result by outputId in the same logical session. Omit part to list parts; read text/JSON in bounded chunks using nextOffset. Screenshots return inline images; encoding=base64 returns exact file bytes for saving/sending. Outputs expire after 24 hours. Reading never repeats the browser action and accepts no filesystem paths.",
        Some(super::read_only_annotations()),
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move { read(raw, &ctx.output_files).await.map(Some) })
}

/// Also used by hosts before browser admission: saved outputs remain readable while
/// the browser is disconnected, provided the logical session is still authorized.
pub async fn read(raw: Value, store: &OutputStore) -> ToolExecResult<ToolResult> {
    let args: ReadOutputArgs = parse_args(raw)?;
    let output = store.describe(&args.output_id).await?;
    let Some(name) = args.part else {
        let value = json!({ "output": output });
        return Ok(text_result(
            serde_json::to_string_pretty(&value)?,
            Some(value),
        ));
    };
    let part = output
        .parts
        .iter()
        .find(|part| part.name == name)
        .ok_or_else(|| crate::framework::ToolError::message("Unknown output part"))?;
    let encoding = args.encoding.unwrap_or_else(|| {
        if part.mime_type.starts_with("image/") {
            Encoding::Image
        } else if part.mime_type.starts_with("text/") || part.mime_type == "application/json" {
            Encoding::Text
        } else {
            Encoding::Base64
        }
    });
    if matches!(encoding, Encoding::Image) {
        if !part.mime_type.starts_with("image/")
            || part.bytes > 2 * 1024 * 1024
            || args.offset != 0
            || args.limit.is_some()
        {
            return Err(crate::framework::ToolError::message(
                "Inline images require an image part up to 2 MiB and no offset/limit; use encoding=base64 for larger files",
            ));
        }
        let mut bytes = Vec::with_capacity(part.bytes as usize);
        let mut offset = 0;
        loop {
            let chunk = store
                .read(&output.id, &name, offset, MAX_READ_BYTES)
                .await?;
            bytes.extend(chunk.bytes);
            match chunk.next_offset {
                Some(next) => offset = next,
                None => break,
            }
        }
        return Ok(ToolResult::image(
            base64::engine::general_purpose::STANDARD.encode(bytes),
            &part.mime_type,
            json!({ "outputId": output.id, "part": name, "mimeType": part.mime_type, "bytes": part.bytes }),
        ));
    }
    let limit = args.limit.unwrap_or(16 * 1024);
    let chunk = if matches!(encoding, Encoding::Base64) {
        store
            .read_bytes(&output.id, &name, args.offset, limit)
            .await?
    } else {
        store.read(&output.id, &name, args.offset, limit).await?
    };
    let mut structured = json!({ "outputId": output.id, "part": name, "mimeType": chunk.part.mime_type, "offset": args.offset, "bytes": chunk.bytes.len(), "totalBytes": chunk.part.bytes, "nextOffset": chunk.next_offset, "eof": chunk.next_offset.is_none() });
    let text = match encoding {
        Encoding::Text => {
            let text = String::from_utf8(chunk.bytes).map_err(|_| {
                crate::framework::ToolError::message("Binary output requires encoding=base64")
            })?;
            structured["text"] = json!(text);
            wrap_untrusted(&text, "saved Neo output")
        }
        Encoding::Base64 => {
            structured["data"] =
                json!(base64::engine::general_purpose::STANDARD.encode(&chunk.bytes));
            format!(
                "Binary output chunk: {} bytes; base64 bytes are in structuredContent.data. nextOffset={}",
                chunk.bytes.len(),
                chunk
                    .next_offset
                    .map_or_else(|| "end".to_string(), |next| next.to_string())
            )
        }
        Encoding::Image => unreachable!(),
    };
    Ok(text_result(text, Some(structured)))
}
