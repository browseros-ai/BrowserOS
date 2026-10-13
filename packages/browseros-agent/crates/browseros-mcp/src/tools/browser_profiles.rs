//! Native saved-profile discovery/loading, deliberately separate from logical
//! task binding and window creation. Loading never changes the admitted view.

use crate::framework::{ToolCtx, ToolError, ToolExecResult, ToolResult, parse_args, text_result};
use browseros_core::TargetError;
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DESCRIPTION: &str = "Discover existing regular browser profiles (list) or load one (load). load accepts at most one exact name or native browserProfileId; omit both to load this task's selected profile. Missing or ambiguous names fail without creating anything. Loading never binds, creates a profile, or opens a window; use browser_target set for task selection and separate windows create for a normal window. loaded=true means this call initiated loading; false means already loaded or joined another load. Native saved-profile handles differ from live CDP browserContextId and expire on browser restart.";

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ProfilesAction {
    #[default]
    List,
    Load,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ProfilesArgs {
    #[serde(default)]
    action: ProfilesAction,
    name: Option<String>,
    browser_profile_id: Option<String>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def_with_output::<ProfilesArgs, super::target_output_schema::ProfilesOutput>(
        "browser_profiles",
        DESCRIPTION,
        None,
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: ProfilesArgs = parse_args(raw)?;
        let store = super::browser_target::target_store(ctx)?;
        let binding = store.get().await.map_err(ToolError::message)?;
        let result = match args.action {
            ProfilesAction::List => {
                let profiles = ctx.browser.list_profiles().await?;
                let lines = profiles
                    .iter()
                    .map(|profile| {
                        format!(
                            "{}: {} ({})",
                            profile.browser_profile_id,
                            profile.name,
                            if profile.loaded {
                                "loaded"
                            } else {
                                "not loaded"
                            }
                        )
                    })
                    .collect::<Vec<_>>();
                let mut data = json!({ "action": "list", "profiles": profiles });
                if let Some(binding) = binding {
                    data["boundBrowserProfileId"] = json!(binding.intent.browser_profile_id);
                }
                text_result(
                    if lines.is_empty() {
                        "(no browser profiles)".to_string()
                    } else {
                        lines.join("\n")
                    },
                    Some(data),
                )
            }
            ProfilesAction::Load => {
                let selector = super::browser_target::selector(args.name, args.browser_profile_id)?;
                let intent = match selector {
                    Some(selector) => ctx.browser.resolve_intent(&selector, None).await?,
                    None => {
                        let binding = binding
                            .ok_or_else(|| ToolError::message(TargetError::NotBound.to_string()))?;
                        super::browser_target::resolve_bound_intent(ctx, &binding, None).await?
                    }
                };
                let outcome = ctx.browser.load_profile(&intent.browser_profile_id).await?;
                text_result(
                    format!(
                        "profile {}: {}",
                        outcome.profile.name,
                        if outcome.loaded {
                            "loaded now"
                        } else {
                            "already loaded"
                        }
                    ),
                    Some(
                        json!({ "action": "load", "profile": outcome.profile, "loaded": outcome.loaded }),
                    ),
                )
            }
        };
        Ok(Some(result))
    })
}
