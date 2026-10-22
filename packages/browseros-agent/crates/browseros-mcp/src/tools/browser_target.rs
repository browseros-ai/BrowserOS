//! Logical task selection. Native discovery validates intent, while the host
//! owns storage; none of these operations load profiles or create windows.
//! A stored binding is an explicit override. Without one, calls use the browser's
//! original Default profile, resolved by core capture and never stored here.

use crate::framework::{
    ArgIssue, TargetStore, ToolCtx, ToolError, ToolExecResult, ToolResult, parse_args, text_result,
};
use browseros_core::{
    BrowserProfileId, CoreError, DEFAULT_PROFILE_DIRECTORY, ProfileSelector, TargetBinding,
    TargetError, TargetIntent, TargetSource, TargetStatus, WindowId, WindowSelection,
};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

const DESCRIPTION: &str = "Select a browser profile for this task: get, set (exact name or browserProfileId), set_window (windowId; strict=true pins), clear_window, or clear. With no selection, calls use the browser's original Default profile (directory \"Default\"), whatever window is focused; set overrides it for this task and clear returns to Default. get reports the selection, or the Default profile this call resolved. set only records intent and clears window selection; it never loads or creates anything. Use browser_profiles load and separate windows create when needed. Selection affects later calls; mutating selection inside run returns target_change_in_run. Calls include only regular profiles' normal windows and pages: OAuth/window.open popups return page_out_of_scope, even in the same profile. get, clear, and reselection remain available after profile_gone, pinned_window_closed or default_profile_unavailable.";

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum TargetAction {
    #[default]
    Get,
    Set,
    SetWindow,
    ClearWindow,
    Clear,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct TargetArgs {
    #[serde(default)]
    action: TargetAction,
    /// Exact case-sensitive profile name for set; mutually exclusive with browserProfileId.
    name: Option<String>,
    /// Native saved-profile handle for set, valid for one browser process lifetime.
    browser_profile_id: Option<String>,
    /// Open normal window for set_window; must belong to the selected profile.
    window_id: Option<i64>,
    /// false selects placement only; true restricts every call to this window.
    strict: Option<bool>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def_with_output::<TargetArgs, super::target_output_schema::TargetOutput>(
        "browser_target",
        DESCRIPTION,
        None,
        handler,
    )
}

pub(super) fn target_store(ctx: &ToolCtx) -> ToolExecResult<&Arc<dyn TargetStore>> {
    ctx.target
        .as_ref()
        .ok_or_else(|| ToolError::message("targeting unavailable here"))
}

pub(super) fn selector(
    name: Option<String>,
    id: Option<String>,
) -> ToolExecResult<Option<ProfileSelector>> {
    match (name, id) {
        (Some(name), None) => Ok(Some(ProfileSelector::Name(name))),
        (None, Some(id)) => Ok(Some(ProfileSelector::Id(BrowserProfileId(id)))),
        (None, None) => Ok(None),
        (Some(_), Some(_)) => Err(ToolError::InvalidArguments(vec![ArgIssue {
            path: "name/browserProfileId".to_string(),
            message: "provide at most one profile selector".to_string(),
        }])),
    }
}

/// Validate a retained identity without treating a missing/recycled handle as
/// a new selection. Management uses this independently of stale view admission.
pub(super) async fn resolve_bound_intent(
    ctx: &ToolCtx,
    binding: &TargetBinding,
    window: Option<WindowSelection>,
) -> ToolExecResult<TargetIntent> {
    let id = &binding.intent.browser_profile_id;
    let intent = ctx
        .browser
        .resolve_intent(&ProfileSelector::Id(id.clone()), window)
        .await
        .map_err(|err| match err {
            CoreError::Target(TargetError::ProfileNotFound(_)) => {
                ToolError::message(TargetError::ProfileGone(id.clone()).to_string())
            }
            err => ToolError::from(err),
        })?;
    if intent.directory != binding.intent.directory {
        return Err(ToolError::message(
            TargetError::ProfileGone(id.clone()).to_string(),
        ));
    }
    Ok(intent)
}

/// No stored override: report the Default profile this very call resolved (core capture owns
/// that resolution), so agents never read "unbound" as browser-wide on a capable native.
async fn default_target_result(ctx: &ToolCtx) -> ToolResult {
    let status = match (ctx.browser.target_source(), ctx.browser.binding()) {
        (Some(TargetSource::Default), Some(effective)) => ctx.browser.status(effective).await,
        _ => TargetStatus {
            profile: None,
            windows: Vec::new(),
            placement_window_id: None,
            error: Some(ctx.browser.admission_error().map_or_else(
                || "the Default profile was not resolved for this call".to_string(),
                ToString::to_string,
            )),
        },
    };
    let summary = match (&status.profile, &status.error) {
        (_, Some(err)) => {
            format!("no browser target is set; the default profile is unavailable: {err}")
        }
        (Some(profile), None) => format!(
            "no browser target is set; calls use the default profile {} (directory {DEFAULT_PROFILE_DIRECTORY})",
            profile.name
        ),
        (None, None) => format!(
            "no browser target is set; calls use the default profile (directory {DEFAULT_PROFILE_DIRECTORY})"
        ),
    };
    text_result(
        summary,
        Some(json!({
            "action": "get",
            "bound": false,
            "default": { "directory": DEFAULT_PROFILE_DIRECTORY, "status": status },
        })),
    )
}

fn binding_result(
    action: &str,
    binding: TargetBinding,
    previous: Option<TargetBinding>,
) -> ToolResult {
    let mut data = json!({ "action": action, "binding": binding });
    if let Some(previous) = previous {
        data["previous"] = json!(previous);
    }
    text_result(
        format!(
            "{action}: selected profile {}",
            binding.intent.browser_profile_id
        ),
        Some(data),
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: TargetArgs = parse_args(raw)?;
        let store = target_store(ctx)?;
        let previous = store.get().await.map_err(ToolError::message)?;
        let result = match args.action {
            TargetAction::Get => match previous {
                Some(binding) => {
                    let status = ctx.browser.status(&binding).await;
                    text_result(
                        format!(
                            "selected profile {}{}",
                            binding.intent.browser_profile_id,
                            status
                                .error
                                .as_ref()
                                .map(|err| format!(": {err}"))
                                .unwrap_or_default()
                        ),
                        Some(
                            json!({ "action": "get", "bound": true, "binding": binding, "status": status }),
                        ),
                    )
                }
                None => default_target_result(ctx).await,
            },
            TargetAction::Set => {
                let selector = selector(args.name, args.browser_profile_id)?.ok_or_else(|| {
                    ToolError::InvalidArguments(vec![ArgIssue {
                        path: "name/browserProfileId".to_string(),
                        message: "provide exactly one profile selector".to_string(),
                    }])
                })?;
                // Resolve before writing, so missing/ambiguous names preserve the
                // old binding and never trigger native profile or window creation.
                let intent = ctx.browser.resolve_intent(&selector, None).await?;
                let binding = store.set(intent).await.map_err(ToolError::message)?;
                binding_result("set", binding, previous)
            }
            TargetAction::SetWindow => {
                let window_id = args.window_id.ok_or_else(|| {
                    ToolError::InvalidArguments(vec![ArgIssue {
                        path: "windowId".to_string(),
                        message: "windowId is required for set_window".to_string(),
                    }])
                })?;
                let window = if args.strict.unwrap_or(false) {
                    WindowSelection::Pinned {
                        window_id: WindowId(window_id),
                    }
                } else {
                    WindowSelection::Working {
                        window_id: WindowId(window_id),
                    }
                };
                let intent = if let Some(binding) = &previous {
                    // set_window changes placement, not saved-profile identity.
                    // A recycled native handle must require explicit profile reselection.
                    resolve_bound_intent(ctx, binding, Some(window)).await?
                } else {
                    ctx.browser.intent_for_window(window).await?
                };
                let binding = store.set(intent).await.map_err(ToolError::message)?;
                binding_result("set_window", binding, previous)
            }
            TargetAction::ClearWindow => {
                let mut intent = previous
                    .as_ref()
                    .ok_or_else(|| ToolError::message(TargetError::NotBound.to_string()))?
                    .intent
                    .clone();
                // Recovery is local intent mutation: it must work after a strict
                // pin closes, without successfully capturing that stale pin first.
                intent.window = None;
                let binding = store.set(intent).await.map_err(ToolError::message)?;
                binding_result("clear_window", binding, previous)
            }
            TargetAction::Clear => {
                store.clear().await.map_err(ToolError::message)?;
                text_result(
                    format!(
                        "browser target cleared; later calls use the default profile (directory {DEFAULT_PROFILE_DIRECTORY})"
                    ),
                    Some(json!({ "action": "clear", "bound": false })),
                )
            }
        };
        Ok(Some(result))
    })
}
