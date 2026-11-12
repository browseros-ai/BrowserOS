//! `page.localStorage` and `page.sessionStorage` (upstream's `WebStorage`, server/page.ts
//! `webStorage*` at v1.64.1). Each call is one probe request in the main frame's isolated world,
//! as upstream evaluates `localStorage.getItem(...)` and the rest in its utility world: the
//! world shares the document's Storage, and the page's own script cannot replace the methods.
//!
//! Each upstream command has its own typed route; the dispatcher marks only `setItem`,
//! `removeItem` and `clear` as writes. A write rechecks the
//! host's policies and marks itself sent before each attempt, and is never sent twice into one
//! document, so a lost answer reads `unknown`, never `not_sent`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::op_scope::{OpScope, Sent};
use crate::op_spec::{OpSpec, PAGE};
use crate::page::OpPage;
use crate::page::evaluate_guard::{self, TreeFrame};
use crate::page::world_call::{self, unexpected, with_world};
use crate::probe::{
    self, ErrorClass, ProbeFailure, ProbeReply, ProbeRequest, Ran, StorageAction, StorageKind,
};
use crate::resolve::failed;
use crate::target::PageRef;
use crate::wire::{Nothing, options};
use crate::world::{Endpoint, IsolatedWorld, Place};
use crate::{ErrorCode, Playwright, PlaywrightError};

pub(crate) const ITEMS_SPEC: OpSpec = OpSpec::read(
    "webStorage.items",
    PAGE,
    "every item in the main frame's localStorage or sessionStorage",
);
pub(crate) const GET_ITEM_SPEC: OpSpec = OpSpec::read(
    "webStorage.getItem",
    PAGE,
    "one item in the main frame's localStorage or sessionStorage",
);
pub(crate) const SET_ITEM_SPEC: OpSpec = OpSpec::write(
    "webStorage.setItem",
    PAGE,
    "set one item in the main frame's localStorage or sessionStorage",
);
pub(crate) const REMOVE_ITEM_SPEC: OpSpec = OpSpec::write(
    "webStorage.removeItem",
    PAGE,
    "remove one item from the main frame's localStorage or sessionStorage",
);
pub(crate) const CLEAR_SPEC: OpSpec = OpSpec::write(
    "webStorage.clear",
    PAGE,
    "clear the main frame's localStorage or sessionStorage",
);

/// Only the host-to-probe boundary carries an action. Typed routes choose it, so callers
/// cannot ask a read route to mutate storage or omit a write's required name/value.
struct WebStorageInput {
    page: PageRef,
    kind: StorageKind,
    action: StorageAction,
    name: Option<String>,
    value: Option<String>,
}

/// Upstream's items and clear commands address a storage area without an item name.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct StorageInput {
    page: PageRef,
    #[ts(type = "\"local\" | \"session\"")]
    kind: StorageKind,
}
options!(StorageInput);

/// Upstream's getItem and removeItem commands require a name.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct StorageKeyInput {
    page: PageRef,
    #[ts(type = "\"local\" | \"session\"")]
    kind: StorageKind,
    name: String,
}
options!(StorageKeyInput);

/// Upstream's setItem command requires both strings; neither is an optional option.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct StorageSetInput {
    page: PageRef,
    #[ts(type = "\"local\" | \"session\"")]
    kind: StorageKind,
    name: String,
    value: String,
}
options!(StorageSetInput);

/// The probe's storage results: item lists, a string, or null, without a wrapper object.
#[derive(Deserialize)]
#[serde(untagged)]
enum StorageValue {
    Items(Vec<StorageItem>),
    Item(Option<String>),
}

/// One name/value pair in `Storage.items()`, in the document's existing iteration order.
#[derive(Deserialize, Serialize, TS)]
pub(crate) struct StorageItem {
    name: String,
    value: String,
}

pub(crate) async fn items(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: StorageInput,
) -> Result<Vec<StorageItem>, PlaywrightError> {
    let reply = call(
        pw,
        scope,
        WebStorageInput {
            page: input.page,
            kind: input.kind,
            action: StorageAction::Items,
            name: None,
            value: None,
        },
    )
    .await?;
    match reply {
        StorageValue::Items(items) => Ok(items),
        StorageValue::Item(_) => {
            Err(scope.fail(ErrorCode::Internal, "invalid storage items reply"))
        }
    }
}

pub(crate) async fn get_item(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: StorageKeyInput,
) -> Result<Option<String>, PlaywrightError> {
    let reply = call(
        pw,
        scope,
        WebStorageInput {
            page: input.page,
            kind: input.kind,
            action: StorageAction::GetItem,
            name: Some(input.name),
            value: None,
        },
    )
    .await?;
    match reply {
        StorageValue::Item(value) => Ok(value),
        StorageValue::Items(_) => {
            Err(scope.fail(ErrorCode::Internal, "invalid storage item reply"))
        }
    }
}

pub(crate) async fn set_item(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: StorageSetInput,
) -> Result<Nothing, PlaywrightError> {
    call(
        pw,
        scope,
        WebStorageInput {
            page: input.page,
            kind: input.kind,
            action: StorageAction::SetItem,
            name: Some(input.name),
            value: Some(input.value),
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn remove_item(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: StorageKeyInput,
) -> Result<Nothing, PlaywrightError> {
    call(
        pw,
        scope,
        WebStorageInput {
            page: input.page,
            kind: input.kind,
            action: StorageAction::RemoveItem,
            name: Some(input.name),
            value: None,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn clear(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: StorageInput,
) -> Result<Nothing, PlaywrightError> {
    call(
        pw,
        scope,
        WebStorageInput {
            page: input.page,
            kind: input.kind,
            action: StorageAction::Clear,
            name: None,
            value: None,
        },
    )
    .await?;
    Ok(())
}

/// Keep the probe protocol and its navigation retry rules independent of route packaging.
async fn call(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WebStorageInput,
) -> Result<StorageValue, PlaywrightError> {
    let WebStorageInput {
        page,
        kind,
        action,
        name,
        value,
    } = input;
    let page = OpPage::admit_handle(pw, scope, &page).await?;
    let world = world_call::frame_world(&page, scope, None).await?;
    let TreeFrame { endpoint, place } =
        evaluate_guard::frame_endpoint(&page, world.session(), world.frame().clone());
    let request = &ProbeRequest::WebStorage {
        kind,
        action,
        name,
        value,
    };
    let reply = if action.writes() {
        write(&page, scope, &endpoint, &place, request).await?
    } else {
        // A read changes nothing, so with_world may send it again in a replaced document.
        with_world(&page, &endpoint, &place, |world| async move {
            probe::call(&world, request).await
        })
        .await
        .map_err(|failure| failed(scope, failure))?
    };
    match reply {
        ProbeReply::Value { value } => {
            serde_json::from_value(value.into_value()).map_err(|error| {
                scope.fail(
                    ErrorCode::Internal,
                    &format!("invalid storage reply: {error}"),
                )
            })
        }
        // The document refused its storage (an opaque origin's `SecurityError`): the page's
        // error, as upstream's evaluation reports it.
        ProbeReply::Error {
            message,
            class: ErrorClass::Script,
            ..
        } => Err(scope.fail(ErrorCode::PageError, &message)),
        other => Err(failed(scope, unexpected("webStorage", other))),
    }
}

/// A write, sent at most once into each document. Each attempt re-runs the host's policies and
/// marks the op sent first (STYLE P6). A world whose document was replaced before the call
/// reached it (`Ran::No`) gets one more attempt in the new document; a call a navigation cut off
/// while pending may have changed the old document's storage, so it is never sent again and
/// the op fails with an ambiguous answer.
async fn write(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    endpoint: &Endpoint,
    place: &Place,
    request: &ProbeRequest,
) -> Result<ProbeReply, PlaywrightError> {
    let install = || async {
        page.world(endpoint, place.clone())
            .await
            .map_err(|failure| failed(scope, failure))
    };
    let mut world = install().await?;
    let mut retried = false;
    loop {
        let sent = scope.send_effect()?;
        match send(&world, request, &sent).await {
            Ok(reply) => return Ok(reply),
            Err(ProbeFailure::Gone(Ran::Maybe)) => {
                return Err(scope
                    .fail(
                        ErrorCode::PageError,
                        "the page navigated while the storage call ran, so it may have \
                         changed the old document's storage",
                    )
                    .after_input());
            }
            Err(ProbeFailure::Gone(Ran::No)) if !retried => {
                retried = true;
                page.frames().invalidate(&world);
                world = install().await?;
            }
            Err(failure) => return Err(failed(scope, failure)),
        }
    }
}

/// The probe's `webStorage` write, which only an op that marked itself sent may make.
async fn send(
    world: &IsolatedWorld,
    request: &ProbeRequest,
    _sent: &Sent<'_>,
) -> Result<ProbeReply, ProbeFailure> {
    probe::call(world, request).await
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::{StorageInput, StorageItem, StorageKeyInput, StorageSetInput};
    use crate::Op;
    use crate::op_output::to_output;
    use crate::op_scope::OpScope;
    use crate::target::CallHeader;
    use crate::test_env::TestEnv;

    #[test]
    fn storage_commands_require_only_their_upstream_fields() {
        let area = serde_json::json!({"page":{"$neo":"page","tab":1},"kind":"local"});
        assert!(serde_json::from_value::<StorageInput>(area.clone()).is_ok());
        assert!(serde_json::from_value::<StorageKeyInput>(area.clone()).is_err());
        let mut named = area.clone();
        named["name"] = serde_json::json!("key");
        assert!(serde_json::from_value::<StorageInput>(named.clone()).is_err());
        assert!(serde_json::from_value::<StorageKeyInput>(named.clone()).is_ok());
        assert!(serde_json::from_value::<StorageSetInput>(named.clone()).is_err());
        named["value"] = serde_json::json!("value");
        assert!(serde_json::from_value::<StorageSetInput>(named).is_ok());
        let mut legacy = area;
        legacy["action"] = serde_json::json!("clear");
        assert!(serde_json::from_value::<StorageInput>(legacy).is_err());
    }

    #[tokio::test]
    async fn storage_replies_keep_their_recorded_json_shapes() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let op = Op::from_name("webStorage.items").ok_or("storage op exists")?;
        let scope = OpScope::new(&env, op, CallHeader::default());
        for before in [r#"[{"name":"a","value":"b"}]"#, "[]"] {
            let typed: Vec<StorageItem> = serde_json::from_str(before)?;
            assert_eq!(serde_json::to_string(&to_output(&scope, &typed)?)?, before);
        }
        for before in [r#""b""#, "null"] {
            let typed: Option<String> = serde_json::from_str(before)?;
            assert_eq!(serde_json::to_string(&to_output(&scope, &typed)?)?, before);
        }
        assert_eq!(serde_json::to_string(&to_output(&scope, &())?)?, "null");
        Ok(())
    }
}
