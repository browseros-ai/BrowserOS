//! Shared creation defaults for the direct script and granular tab adapters.
//! Inherited grouping is a preference; explicit grouping remains core-validated
//! intent. Neither can widen the request's captured browser scope.

use crate::framework::ToolCtx;
use browseros_core::{CoreError, WindowId};
use serde_json::{Value, json};

pub(super) async fn tab_group_for_creation(
    ctx: &ToolCtx,
    explicit_group: Option<String>,
    requested_window: Option<WindowId>,
) -> Result<Option<String>, CoreError> {
    // Preserve explicit conflicts for core to reject, and preserve legacy
    // unbound grouping without adding a native lookup to that path.
    if explicit_group.is_some() {
        return Ok(explicit_group);
    }
    let Some(group) = ctx.defaults.default_tab_group_id.as_ref() else {
        return Ok(None);
    };
    let Some(scope) = ctx.browser.scope()? else {
        return Ok(Some(group.clone()));
    };
    let Some(destination) = requested_window.or(scope.placement_window) else {
        return Ok(None);
    };

    // The default group can move after capture, and an explicit destination can
    // differ from working placement. Consult its actual eligible native window
    // through the same captured view. If it does not match, leave grouping to
    // the host's destination-specific effect instead of restricting placement.
    // Core still revalidates a retained group immediately before creation.
    let result = ctx
        .browser
        .cdp("Browser.getTabGroups", json!({}), None)
        .await?;
    let compatible = result
        .get("groups")
        .and_then(Value::as_array)
        .is_some_and(|groups| {
            groups.iter().any(|candidate| {
                candidate.get("groupId").and_then(Value::as_str) == Some(group.as_str())
                    && candidate.get("windowId").and_then(Value::as_i64) == Some(destination.0)
            })
        });
    Ok(compatible.then(|| group.clone()))
}
