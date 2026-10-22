//! Agent-facing credential contract. Native/core own all password semantics;
//! this adapter validates action combinations and formats metadata-only results.
#![allow(dead_code)] // Output-only schema types describe the wire contract.

use crate::{
    framework::{ArgIssue, ToolCtx, ToolError, ToolExecResult, ToolResult},
    response::ToolResponse,
};
use browseros_core::{
    PageId,
    credentials::{CredentialListOptions, CredentialSource},
};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DESCRIPTION: &str = "List, add, or fill password credentials in the selected browser profile. list with page returns site matches; omit page for all saved accounts in this profile (Default when unbound). add saves supplied username/password for the page's site, without needing a submitted form; identical adds are unchanged and different saved passwords conflict. fill accepts exactly one source: credentialId from list/add, or both username and password. Supplied filling does not save. Filling acknowledges fields, does not submit or confirm login, and supports recognized main-frame login forms. Passwords never appear in results. Use tabs to find page ids. In run, use browser.credentials(options) or browser.page(id).credentials(options).";

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Action {
    List,
    Add,
    Fill,
}
impl Action {
    fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Add => "add",
            Self::Fill => "fill",
        }
    }
}

// No Debug: error and tracing paths must not accidentally format supplied secrets.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Args {
    action: Action,
    /// Page id from tabs. Required for add/fill; omit for a profile-wide list.
    #[schemars(range(min = 1))]
    page: Option<u32>,
    /// Saved fill only. Cannot be combined with username/password.
    credential_id: Option<String>,
    /// Required for add/direct fill, even when empty. Never trimmed.
    username: Option<String>,
    /// Sensitive, nonempty input for add/direct fill. Never returned or trimmed.
    password: Option<String>,
    /// list only: case-insensitive literal substring of username or site.
    query: Option<String>,
    /// list only: 1-100 entries, default 50.
    #[schemars(range(min = 1, max = 100))]
    limit: Option<u32>,
    /// list only: nextCursor from the same profile, query and page/origin.
    cursor: Option<String>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Account {
    credential_id: String,
    username: String,
    site: String,
    stores: Vec<Store>,
}
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Store {
    Profile,
    Account,
}
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
enum AddStatus {
    Saved,
    Unchanged,
}
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FilledStatus {
    Filled,
}
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FilledField {
    Username,
    Password,
}
#[derive(JsonSchema)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "source"
)]
enum FillSource {
    #[serde(rename = "saved")]
    Saved { credential_id: String },
    #[serde(rename = "provided")]
    Provided,
}
#[derive(JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ErrorStatus {
    Error,
}
#[derive(JsonSchema)]
struct Failure {
    code: String,
    message: String,
}

#[derive(JsonSchema)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum OutputVariants {
    List {
        browser_profile_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        page: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        origin: Option<String>,
        credentials: Vec<Account>,
        total: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        next_cursor: Option<String>,
    },
    Add {
        status: AddStatus,
        browser_profile_id: String,
        page: u32,
        credential: Account,
    },
    Fill {
        status: FilledStatus,
        browser_profile_id: String,
        page: u32,
        origin: String,
        #[serde(flatten)]
        source: FillSource,
        filled_fields: Vec<FilledField>,
    },
    #[serde(untagged)]
    Error {
        action: Action,
        status: ErrorStatus,
        error: Failure,
    },
}

struct Output;
impl JsonSchema for Output {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "CredentialsOutput".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let mut schema = OutputVariants::json_schema(generator);
        // MCP requires an explicit object root, including for tagged unions.
        schema.insert("type".into(), json!("object"));
        schema
    }
}

/// Shared by the script bridge and its host audit adapter. Both structured and
/// raw CDP credential calls must establish sensitivity before any early error.
pub fn is_credential_primitive(method: &str, args: &[Value]) -> bool {
    if method == "tool:credentials" {
        return true;
    }
    let index = match method {
        "cdp" => 0,
        "cdpJsonForPage" => 1,
        _ => return false,
    };
    args.get(index).and_then(Value::as_str).is_some_and(|name| {
        name.starts_with("Autofill.") && name.to_ascii_lowercase().contains("credential")
    })
}

pub fn definition() -> crate::framework::ToolDef {
    super::def_with_output::<Args, Output>(
        "credentials",
        DESCRIPTION,
        Some(super::open_world_annotations()),
        handler,
    )
}

fn invalid(message: &str) -> ToolError {
    ToolError::InvalidArguments(vec![ArgIssue {
        path: String::new(),
        message: message.into(),
    }])
}
fn parse(raw: Value) -> ToolExecResult<Args> {
    // Serde diagnostics may quote unexpected scalar values/field names. All
    // malformed credential requests use fixed messages, including early errors.
    let obj = raw
        .as_object()
        .ok_or_else(|| invalid("Expected credential arguments."))?;
    if obj.values().any(Value::is_null) {
        return Err(invalid("Omit optional fields instead of passing null."));
    }
    let args: Args = serde_json::from_value(raw).map_err(|_| {
        invalid("Check action and credential field types; unknown fields are not accepted.")
    })?;
    if args.page == Some(0) {
        return Err(invalid("page must be a positive page id from tabs."));
    }
    let listing = args.query.is_some() || args.limit.is_some() || args.cursor.is_some();
    let supplied = args.username.is_some() || args.password.is_some();
    match args.action {
        Action::List => {
            if supplied || args.credential_id.is_some() {
                return Err(invalid("list accepts only page, query, limit and cursor."));
            }
            if args.limit.is_some_and(|n| !(1..=100).contains(&n))
                || args.cursor.as_ref().is_some_and(String::is_empty)
            {
                return Err(invalid("limit must be 1-100; cursor must be nonempty."));
            }
        }
        Action::Add => {
            if args.page.is_none()
                || args.username.is_none()
                || args.password.as_ref().is_none_or(String::is_empty)
                || args.credential_id.is_some()
                || listing
            {
                return Err(invalid(
                    "add requires page, username and a nonempty password; no listing fields or credentialId.",
                ));
            }
        }
        Action::Fill => {
            let saved = args.credential_id.as_ref().is_some_and(|id| !id.is_empty());
            let direct =
                args.username.is_some() && args.password.as_ref().is_some_and(|p| !p.is_empty());
            if args.page.is_none()
                || listing
                || (saved && supplied)
                || (!saved && args.credential_id.is_some())
                || saved == direct
            {
                return Err(invalid(
                    "fill requires page and exactly one source: credentialId, or username and a nonempty password.",
                ));
            }
        }
    }
    Ok(args)
}
fn escaped(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args = parse(raw)?;
        let action = args.action.name();
        let result = tokio::select! {
            biased;
            () = ctx.cancel.cancelled() => return Err(ToolError::Cancelled),
            result = async { match args.action {
            Action::List => {
                let inventory = ctx.browser.list_credentials(args.page.map(PageId), CredentialListOptions {
                    query: args.query, limit: args.limit, cursor: args.cursor }).await;
                inventory.map(|inventory| {
                    let text = if inventory.credentials.is_empty() { "No saved accounts found.".to_owned() }
                        else { std::iter::once(format!("{} saved account(s){}", inventory.total,
                            inventory.origin.as_ref().map(|s| format!(" for {s}")).unwrap_or_default()))
                            .chain(inventory.credentials.iter().map(|c| format!("{} — {} at {} [{}]", c.credential_id, escaped(&c.username), escaped(&c.site), c.stores.join(", ")))).collect::<Vec<_>>().join("\n") };
                    let mut data = serde_json::to_value(inventory).unwrap_or(json!({}));
                    data["action"] = json!(action);
                    if let Some(page) = args.page { data["page"] = json!(page); }
                    ToolResult::text(text, Some(data))
                })
            }
            Action::Add => ctx.browser.add_credential(PageId(args.page.unwrap_or_default()), args.username.unwrap_or_default(), args.password.unwrap_or_default()).await.map(|outcome| {
                let text = format!("{} {} for {} at {}.", if outcome.status == "saved" { "Saved" } else { "Already saved" }, outcome.credential.credential_id, escaped(&outcome.credential.username), escaped(&outcome.credential.site));
                let data = json!({"action":action,"status":outcome.status,"browserProfileId":outcome.browser_profile_id,"page":args.page,"credential":outcome.credential});
                ToolResult::text(text, Some(data))
            }),
            Action::Fill => {
                let saved = args.credential_id.clone();
                let source = match args.credential_id {
                    Some(credential_id) => CredentialSource::Saved { credential_id },
                    None => CredentialSource::Provided { username: args.username.unwrap_or_default(), password: args.password.unwrap_or_default() },
                };
                ctx.browser.fill_credential(PageId(args.page.unwrap_or_default()), source).await.map(|outcome| {
                    let text = format!("Filled {} on page {}. Form not submitted.", outcome.filled_fields.join(" and "), args.page.unwrap_or_default());
                    let mut data = json!({"action":action,"status":"filled","browserProfileId":outcome.browser_profile_id,"page":args.page,"origin":outcome.origin,"source":if saved.is_some(){"saved"}else{"provided"},"filledFields":outcome.filled_fields});
                    if let Some(id) = saved { data["credentialId"] = json!(id); }
                    ToolResult::text(text, Some(data))
                })
            }
            } } => result,
        };
        Ok(Some(match result {
            Ok(result) => result,
            Err(failure) => {
                let mut result = ToolResult::error(failure.message);
                result.structured_content =
                    Some(json!({"action":action,"status":"error","error":failure}));
                result
            }
        }))
    })
}
