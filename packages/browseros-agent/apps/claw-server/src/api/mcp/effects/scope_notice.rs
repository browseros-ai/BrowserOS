//! Tells an agent that its saved profile target was not applied to this call.
//!
//! A binding saved while a newer browser was connected can meet an older native browser,
//! for example after a downgrade or when correlation restores it. Capture then runs the
//! call with the unbound, browser-wide routing those natives always had, so ordinary work
//! keeps going (see `BrowserView::capture`). This effect only reports that: like
//! `page_ownership_notice`, it is a label, never a permission, and it must not turn a
//! result into an error or gate a dispatch.
//!
//! It annotates the outer MCP result only. Primitives inside a `run` script never pass
//! through dispatch effects, so their text stays exactly what the page returned.

use crate::api::mcp::dispatch::{ToolEffect, ToolEffectContext};
use browseros_mcp::{ToolResult, unapplied_target_note};
use futures_util::future::BoxFuture;

pub fn apply(context: ToolEffectContext<'_>) -> BoxFuture<'_, anyhow::Result<Option<ToolResult>>> {
    Box::pin(async move {
        let Some(note) = context.call.view.as_ref().and_then(unapplied_target_note) else {
            return Ok(None);
        };
        let mut annotated = context.result.clone();
        annotated
            .content
            .push(rmcp::model::ContentBlock::text(note));
        Ok(Some(annotated))
    })
}

const _: ToolEffect = apply;
