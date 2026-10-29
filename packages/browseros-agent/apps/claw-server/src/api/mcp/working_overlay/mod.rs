//! Holds a page under the working overlay while a page-changing call acts on it.
//!
//! Reading tools never start a hold, so snapshot, read and grep never touch the page's DOM.

use crate::{
    api::mcp::dispatch::{ToolCall, extract_page_id},
    services::page_hold::labels,
};
use browseros_core::{PageId, Ref};

/// Granular tools that change a page. `run` holds per primitive through its script hook.
const HOLDING_TOOLS: [&str; 5] = ["act", "navigate", "upload", "download", "evaluate_in_page"];

fn held_page(call: &ToolCall) -> Option<PageId> {
    if !HOLDING_TOOLS.contains(&call.tool().name) {
        return None;
    }
    extract_page_id(call).map(PageId)
}

/// Draws the overlay before the call runs. An error is the refusal to return instead of running
/// it: the person took the page over.
pub(super) async fn begin(call: &ToolCall) -> Result<(), String> {
    let page_hold = &call.state.page_hold;
    if !page_hold.enabled() {
        return Ok(());
    }
    let (Some(page), Some(identity), Some(browser)) =
        (held_page(call), &call.identity, &call.browser_session)
    else {
        return Ok(());
    };
    let hint = match (call.tool().name, call.raw_args["ref"].as_str(), &call.view) {
        ("act", Some(ref_id), Some(view)) => match view.input(page.clone()).await {
            Ok(input) => input.target_hint(&Ref(ref_id.to_string())).await,
            Err(_) => None,
        },
        _ => None,
    };
    let label = labels::for_tool(
        call.tool().name,
        &call.raw_args,
        hint.as_ref().map(|hint| hint.name.as_str()),
    );
    page_hold
        .begin_action(
            browser,
            page,
            identity.session.id(),
            &call.dispatch_id,
            label,
            hint.and_then(|hint| hint.rect),
        )
        .await
}

/// Moves the page to "between actions" once the call is done, whatever its outcome.
pub(super) async fn end(call: &ToolCall) {
    if let (Some(page), Some(browser)) = (held_page(call), &call.browser_session) {
        call.state
            .page_hold
            .end_action(browser, page, &call.dispatch_id)
            .await;
    }
}

#[cfg(test)]
mod tests;
