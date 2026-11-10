//! Playwright: `client/webStorage.ts` `WebStorage`
//! Playwright: `server/page.ts` `Page.webStorageItems`
//!
//! `page.localStorage` and `page.sessionStorage`: each method is one Page call on the main
//! frame's storage. Playwright's server evaluates each step as an expression in the main frame's
//! utility world; Neo runs the same steps there as typed glue (`neo-playwright-injected` `glue.ts`,
//! `webStorage*`), never as an evaluation it sends itself.
//!
//! ```text
//!  JS page.localStorage.setItem('k', 'v')
//!    {"guid":"page@T","method":"webStorageSetItem","params":{"kind":"local","name":"k","value":"v"}}
//!    └─ page_and_context_methods::page → serve → the main frame's glue `webStorageSetItem` → {}
//! ```

use std::sync::Arc;

use neo_runtime_spec::injected::{
    InjectedWebStorageClearParams, InjectedWebStorageGetItemParams, InjectedWebStorageItemsParams,
    InjectedWebStorageRemoveItemParams, InjectedWebStorageSetItemParams, WebStorageKind,
};
use neo_runtime_spec::serve::{
    NameValue, PageCall, PageWebStorageClearParams, PageWebStorageClearParamsKind,
    PageWebStorageClearResult, PageWebStorageGetItemParams, PageWebStorageGetItemParamsKind,
    PageWebStorageGetItemResult, PageWebStorageItemsParams, PageWebStorageItemsParamsKind,
    PageWebStorageItemsResult, PageWebStorageRemoveItemParams, PageWebStorageRemoveItemParamsKind,
    PageWebStorageRemoveItemResult, PageWebStorageSetItemParams, PageWebStorageSetItemParamsKind,
    PageWebStorageSetItemResult,
};
use serde_json::Value;

use crate::browser_representations::page::{FrameTab, PageSession};
use crate::chromium_bridge::frames::FrameScope;
use crate::method_router::encode_result;
use crate::playwright_methods::navigate::page_error;
use crate::progress::{CallError, PlaywrightError, Progress};

/// Neo: what a non-storage `PageCall` reaching [`serve`] answers; `page_and_context_methods::page` routes only
/// the five `webStorage*` calls here.
const NOT_A_STORAGE_CALL: &str = "neo-runtime: not a WebStorage call";

/// Serves a `Page.webStorage*` call on the main frame of `page`, a prepared session.
/// Playwright: `server/page.ts` `Page.webStorageItems`: `return await this.mainFrame().evaluateExpression(progress, `
pub(crate) async fn serve(
    page: Arc<PageSession>,
    call: PageCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let main = FrameTab {
        page,
        scope: FrameScope::Main,
    };
    let frame = main.context().await.map_err(page_error)?;
    match call {
        PageCall::WebStorageItems(params) => {
            let kind = storage(params.kind == PageWebStorageItemsParamsKind::Local);
            let read = progress
                .race(frame.call(None, &InjectedWebStorageItemsParams { kind }))
                .await?
                .map_err(page_error)?;
            let items = read
                .items
                .into_iter()
                .map(|item| NameValue {
                    name: item.name,
                    value: item.value,
                })
                .collect();
            encode_result::<PageWebStorageItemsParams>(&PageWebStorageItemsResult { items })
        }
        PageCall::WebStorageGetItem(params) => {
            let kind = storage(params.kind == PageWebStorageGetItemParamsKind::Local);
            let read = progress
                .race(frame.call(
                    None,
                    &InjectedWebStorageGetItemParams {
                        kind,
                        name: params.name,
                    },
                ))
                .await?
                .map_err(page_error)?;
            encode_result::<PageWebStorageGetItemParams>(&PageWebStorageGetItemResult {
                value: read.value,
            })
        }
        PageCall::WebStorageSetItem(params) => {
            let kind = storage(params.kind == PageWebStorageSetItemParamsKind::Local);
            progress
                .race(frame.call(
                    None,
                    &InjectedWebStorageSetItemParams {
                        kind,
                        name: params.name,
                        value: params.value,
                    },
                ))
                .await?
                .map_err(page_error)?;
            encode_result::<PageWebStorageSetItemParams>(&PageWebStorageSetItemResult {})
        }
        PageCall::WebStorageRemoveItem(params) => {
            let kind = storage(params.kind == PageWebStorageRemoveItemParamsKind::Local);
            progress
                .race(frame.call(
                    None,
                    &InjectedWebStorageRemoveItemParams {
                        kind,
                        name: params.name,
                    },
                ))
                .await?
                .map_err(page_error)?;
            encode_result::<PageWebStorageRemoveItemParams>(&PageWebStorageRemoveItemResult {})
        }
        PageCall::WebStorageClear(params) => {
            let kind = storage(params.kind == PageWebStorageClearParamsKind::Local);
            progress
                .race(frame.call(None, &InjectedWebStorageClearParams { kind }))
                .await?
                .map_err(page_error)?;
            encode_result::<PageWebStorageClearParams>(&PageWebStorageClearResult {})
        }
        _ => Err(PlaywrightError::error(NOT_A_STORAGE_CALL).into()),
    }
}

/// Neo: the glue's storage for a call's `kind`, as each generated params type names its own
/// `kind` enum.
fn storage(local: bool) -> WebStorageKind {
    if local {
        WebStorageKind::Local
    } else {
        WebStorageKind::Session
    }
}
