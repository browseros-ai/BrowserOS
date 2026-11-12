//! `page.localStorage` and `page.sessionStorage` (upstream's `WebStorage`, server/page.ts
//! `webStorage*` at v1.64.1). Each call is one probe request in the main frame's isolated world,
//! as upstream evaluates `localStorage.getItem(...)` and the rest in its utility world: the
//! world shares the document's Storage, and the page's own script cannot replace the methods.
//!
//! Two ops, so the dispatcher knows which calls change the page: `webStorage.read` (`items`,
//! `getItem`) and `webStorage.write` (`setItem`, `removeItem`, `clear`). A write rechecks the
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

pub(crate) const READ_SPEC: OpSpec = OpSpec::read(
    "webStorage.read",
    PAGE,
    "the main frame's localStorage or sessionStorage: every item, or one",
);

pub(crate) const WRITE_SPEC: OpSpec = OpSpec::write(
    "webStorage.write",
    PAGE,
    "set or remove an item of the main frame's localStorage or sessionStorage, or clear it",
);

/// The shared runner's decoded fields; the read and write routes own their separate inputs.
struct WebStorageInput {
    page: PageRef,
    kind: StorageKind,
    action: StorageAction,
    name: Option<String>,
    value: Option<String>,
}

/// Reads keep the existing action/name shape; only setItem's sibling accepts a value.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct StorageReadInput {
    page: PageRef,
    #[ts(type = "\"local\" | \"session\"")]
    kind: StorageKind,
    #[ts(type = "\"items\" | \"getItem\"")]
    action: StorageAction,
    #[serde(default)]
    #[ts(optional)]
    name: Option<String>,
}
options!(StorageReadInput);

/// The write side keeps optional name/value fields for removeItem and clear.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct StorageWriteInput {
    page: PageRef,
    #[ts(type = "\"local\" | \"session\"")]
    kind: StorageKind,
    #[ts(type = "\"setItem\" | \"removeItem\" | \"clear\"")]
    action: StorageAction,
    #[serde(default)]
    #[ts(optional)]
    name: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    value: Option<String>,
}
options!(StorageWriteInput);

/// The probe's storage results: item lists, a string, or null, without a wrapper object.
#[derive(Deserialize, Serialize, TS)]
#[serde(untagged)]
pub(crate) enum StorageValue {
    Items(Vec<StorageItem>),
    Item(Option<String>),
}

/// One name/value pair in `Storage.items()`, in the document's existing iteration order.
#[derive(Deserialize, Serialize, TS)]
pub(crate) struct StorageItem {
    name: String,
    value: String,
}

pub(crate) async fn read(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: StorageReadInput,
) -> Result<StorageValue, PlaywrightError> {
    let StorageReadInput {
        page,
        kind,
        action,
        name,
    } = input;
    call(
        pw,
        scope,
        WebStorageInput {
            page,
            kind,
            action,
            name,
            value: None,
        },
        false,
    )
    .await
}

pub(crate) async fn write_value(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: StorageWriteInput,
) -> Result<Nothing, PlaywrightError> {
    let StorageWriteInput {
        page,
        kind,
        action,
        name,
        value,
    } = input;
    let _reply = call(
        pw,
        scope,
        WebStorageInput {
            page,
            kind,
            action,
            name,
            value,
        },
        true,
    )
    .await?;
    Ok(())
}

/// Runs one Storage call. `writes` is the op's own side: a read op refuses a write action, and
/// the other way round, so the dispatcher's effect for the op always matches the call.
async fn call(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WebStorageInput,
    writes: bool,
) -> Result<StorageValue, PlaywrightError> {
    let WebStorageInput {
        page,
        kind,
        action,
        name,
        value,
    } = input;
    if action.writes() != writes {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            &format!("{action:?} is not a call of this op"),
        ));
    }
    let needs_name = !matches!(action, StorageAction::Items | StorageAction::Clear);
    if needs_name != name.is_some() || (action == StorageAction::SetItem) != value.is_some() {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            &format!("{action:?} takes a name, and setItem a value too"),
        ));
    }
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
    let reply = if writes {
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

    use super::StorageValue;
    use crate::Op;
    use crate::op_output::to_output;
    use crate::op_scope::OpScope;
    use crate::target::CallHeader;
    use crate::test_env::TestEnv;

    #[tokio::test]
    async fn storage_replies_keep_their_recorded_json_shapes() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let op = Op::from_name("webStorage.read").ok_or("storage op exists")?;
        let scope = OpScope::new(&env, op, CallHeader::default());
        for before in [r#"[{"name":"a","value":"b"}]"#, r#""b""#, "null", "[]"] {
            let typed: StorageValue = serde_json::from_str(before)?;
            assert_eq!(serde_json::to_string(&to_output(&scope, &typed)?)?, before);
        }
        Ok(())
    }
}
