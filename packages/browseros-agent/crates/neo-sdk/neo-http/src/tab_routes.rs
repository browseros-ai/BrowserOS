//! The host's live native tab inventory joined with committed trace task associations. The
//! browser supplies identities/presentation; retained runs supply history, never ownership. The
//! one tab Neo keeps for itself (its blank tab) is listed too, marked `owner: neo`.

use crate::api_error::ApiError;
use crate::service::ServiceState;
use axum::Json;
use axum::extract::State;
use browseros_cdp::browser;
use browseros_core::TabId;
use neo_sdk_api as api;
use neo_takeover::PartError;
use serde_json::json;
use std::sync::Arc;

pub(crate) async fn list_tabs(
    State(service): State<Arc<ServiceState>>,
) -> Result<Json<api::TabList>, ApiError> {
    // One live native listing, not the agent's filtered page cache. No extra CDP connection or
    // page attachment is needed, and a failed listing must not look like an empty browser.
    let native: browser::GetTabsResult = service
        .tab_inventory
        .send("Browser.getTabs", json!({}))
        .await
        .map_err(PartError::Browser)?;
    let ids: Vec<TabId> = native.tabs.iter().map(|tab| TabId(tab.tab_id)).collect();
    let mut tasks = service.store.tasks_for_tabs(&ids).await?;
    let items: Vec<api::Tab> = native
        .tabs
        .into_iter()
        .map(|tab| api::Tab {
            tab_id: tab.tab_id,
            target_id: tab.target_id,
            url: tab.url,
            title: tab.title,
            task_ids: tasks
                .remove(&TabId(tab.tab_id))
                .unwrap_or_default()
                .into_iter()
                .map(|task| task.to_string())
                .collect(),
            owner: (service.tab_owner)(TabId(tab.tab_id)).map(str::to_owned),
        })
        .collect();
    Ok(Json(api::TabList::new(items)))
}
