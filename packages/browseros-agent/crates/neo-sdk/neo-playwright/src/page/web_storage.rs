//! `page.localStorage` and `page.sessionStorage` (upstream's `WebStorage`, server/page.ts
//! `webStorage*` at v1.64.1). Each call is one probe request in the main frame's isolated world,
//! as upstream evaluates `localStorage.getItem(...)` and the rest in its utility world: the
//! world shares the document's Storage, and the page's own script cannot replace the methods.
//!
//! Two ops, so the dispatcher knows which calls change the page: `webStorage.read` (`items`,
//! `getItem`) and `webStorage.write` (`setItem`, `removeItem`, `clear`), which marks itself
//! sent before its probe call, so a lost answer reads `unknown`, never `not_sent`.

use serde::Deserialize;
use serde_json::Value;

use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, PAGE};
use crate::page::OpPage;
use crate::page::evaluate_guard::{self, TreeFrame};
use crate::page::world_call::{self, unexpected, with_world};
use crate::probe::{self, ErrorClass, ProbeReply, ProbeRequest, StorageAction, StorageKind};
use crate::resolve::failed;
use crate::target::PageRef;
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

/// `{ page, kind, action, name?, value? }`, as the prelude's `WebStorage` sends it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct WebStorageInput {
    page: PageRef,
    kind: StorageKind,
    action: StorageAction,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    value: Option<String>,
}

/// Runs one Storage call. `writes` is the op's own side: a read op refuses a write action, and
/// the other way round, so the dispatcher's effect for the op always matches the call.
pub(crate) async fn call(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WebStorageInput,
    writes: bool,
) -> Result<Value, PlaywrightError> {
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
    let _sent = if writes {
        Some(scope.send_effect()?)
    } else {
        None
    };
    let reply = with_world(&page, &endpoint, &place, |world| async move {
        match probe::call(&world, request).await? {
            reply @ (ProbeReply::Value { .. } | ProbeReply::Error { .. }) => Ok(reply),
            other => Err(unexpected("webStorage", other)),
        }
    })
    .await
    .map_err(|failure| failed(scope, failure))?;
    match reply {
        ProbeReply::Value { value } => Ok(value.into_value()),
        // The document refused its storage (an opaque origin's `SecurityError`): the page's
        // error, as upstream's evaluation reports it.
        ProbeReply::Error {
            message,
            class: ErrorClass::Script,
        } => Err(scope.fail(ErrorCode::PageError, &message)),
        other => Err(scope.fail(
            ErrorCode::Internal,
            &format!("the probe answered `webStorage` with `{}`", other.kind()),
        )),
    }
}
