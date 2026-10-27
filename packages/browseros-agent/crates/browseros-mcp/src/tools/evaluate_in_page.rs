//! `evaluate_in_page`: `evaluate` in the page's own main world, for the rare script that needs the
//! page's JavaScript state. A separate tool rather than a flag so clients can allow or deny it on
//! its own and its use stays countable; the runner is shared with `evaluate`.

use super::evaluate::{EvaluateArgs, World, evaluate_in, script_tool_usage};
use crate::framework::{ToolCtx, ToolExecResult, ToolResult};
use futures_util::future::BoxFuture;
use serde_json::Value;

const DESCRIPTION: &str = concat!(
    "Evaluate JavaScript in the page's own main world, where the page's scripts run and can watch it: hooks \
the page installed on built-ins see every call. Use this only when the user explicitly asks to run code in \
the page, or when the task needs the page's own JavaScript state (app globals, framework internals) that \
`evaluate` cannot see; use `evaluate` for everything else. ",
    script_tool_usage!()
);

pub fn definition() -> crate::framework::ToolDef {
    super::def::<EvaluateArgs>(
        World::Page.tool(),
        DESCRIPTION,
        Some(super::open_world_annotations()),
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(evaluate_in(World::Page, raw, ctx))
}
