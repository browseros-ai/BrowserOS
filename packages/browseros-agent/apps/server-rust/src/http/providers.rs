//! Provider CRUD over both kinds in the unified table, plus the one default selection. The
//! wire projection is deliberately asymmetric: every read removes four secrets and adds
//! presence flags, while an upsert returns the raw row for compatibility with shipped clients.

use super::Json;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
};
use server_api::models as api;

use super::wire::crud::{self, CrudJson};
use crate::{AppState, db};

pub async fn list(State(state): State<AppState>) -> Response {
    match state.providers.list().await {
        // No sort: TS exposes SQLite row order here, including ACP providers.
        Ok(rows) => Json(api::ProviderList::new(
            rows.into_iter().map(crud::public_provider).collect(),
        ))
        .into_response(),
        Err(error) => crud::db_error(error),
    }
}

pub async fn get_default(State(state): State<AppState>) -> Response {
    match state.providers.default().await {
        Ok(row) => Json(api::DefaultProviderResponse::new(
            row.map(crud::public_provider),
        ))
        .into_response(),
        Err(error) => crud::db_error(error),
    }
}

pub async fn set_default(
    State(state): State<AppState>,
    CrudJson(input): CrudJson<api::SetDefaultProviderRequest>,
) -> Response {
    // The store clears the previous default before setting this one in one transaction;
    // doing this in two handler awaits would violate the partial unique index or lose state.
    match state.providers.set_default(&input.provider_id).await {
        Ok(Some(row)) => {
            Json(api::PublicProviderResponse::new(crud::public_provider(row))).into_response()
        }
        Ok(None) => crud::not_found("Unknown provider"),
        Err(error) => crud::db_error(error),
    }
}

pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.providers.get(&id).await {
        Ok(Some(row)) => {
            Json(api::PublicProviderResponse::new(crud::public_provider(row))).into_response()
        }
        Ok(None) => crud::not_found("Unknown provider"),
        Err(error) => crud::db_error(error),
    }
}

pub async fn upsert(
    State(state): State<AppState>,
    Path(id): Path<String>,
    CrudJson(input): CrudJson<api::UpsertProviderRequest>,
) -> Response {
    let input = db::UpsertProvider {
        provider_type: input.r#type,
        name: input.name,
        model_id: Some(input.model_id),
        base_url: input.base_url,
        headers: input
            .headers
            .map(|headers| headers.map(|headers| headers.into_iter().collect())),
        supports_images: input.supports_images,
        context_window: Some(crud::store_integer(input.context_window)),
        temperature: input.temperature,
        // Keep omission, null, and empty distinct until the write transaction. Pre-reading and
        // merging credentials here would lose concurrent updates and cannot express a clear.
        api_key: input.api_key,
        access_key_id: input.access_key_id,
        secret_access_key: input.secret_access_key,
        session_token: input.session_token,
        resource_name: input.resource_name,
        region: input.region,
        reasoning_effort: input.reasoning_effort,
        reasoning_summary: input.reasoning_summary,
        created_at: input.created_at.map(crud::store_integer),
    };
    match state.providers.upsert_llm(&id, input).await {
        Ok(row) => Json(api::ProviderResponse::new(crud::provider(row))).into_response(),
        Err(error) => crud::db_error(error),
    }
}

pub async fn delete(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    // This route intentionally deletes either kind and does not run the /agents session
    // callback. The database's SET NULL action owns scheduled-job references.
    crud::deleted(state.providers.delete(&id).await, "Unknown provider")
}
