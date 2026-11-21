//! `/providers`: the LLM provider rows and the selected chat target. Reads never carry a secret
//! (`ProviderRow` has only `has_*` flags); a save takes credentials inward only.

use axum::Json;
use axum::extract::State;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::ApiError;
use super::dto;
use super::extract::{Body, PathParams};
use crate::app::App;
use crate::store::{ProviderEdit, Providers};

pub(super) fn routes() -> OpenApiRouter<App> {
    OpenApiRouter::new()
        .routes(routes!(list_providers))
        .routes(routes!(get_default_provider, set_default_provider))
        .routes(routes!(save_provider, delete_provider))
}

/// Every chat target the server can run, LLM providers and coding agents.
///
/// Rows of a type this server no longer runs are left in the database and not listed.
#[utoipa::path(
    get,
    path = "/providers",
    tag = "providers",
    operation_id = "listProviders",
    responses(
        (status = 200, description = "Rows without secrets.", body = dto::ProviderList),
    )
)]
async fn list_providers(State(app): State<App>) -> Result<Json<dto::ProviderList>, ApiError> {
    let providers = app.store.providers().list().await?;
    Ok(Json(dto::ProviderList {
        providers: providers.into_iter().map(dto::ProviderBody::from).collect(),
    }))
}

/// The selected chat target.
#[utoipa::path(
    get,
    path = "/providers/default",
    tag = "providers",
    operation_id = "getDefaultProvider",
    responses(
        (status = 200, description = "The selected row, or null when none is selected.", body = dto::DefaultProvider),
    )
)]
async fn get_default_provider(
    State(app): State<App>,
) -> Result<Json<dto::DefaultProvider>, ApiError> {
    let provider = app.store.providers().default().await?;
    Ok(Json(dto::DefaultProvider {
        provider: provider.map(dto::ProviderBody::from),
    }))
}

/// Select a chat target.
#[utoipa::path(
    put,
    path = "/providers/default",
    tag = "providers",
    operation_id = "setDefaultProvider",
    request_body = dto::SetDefaultProvider,
    responses(
        (status = 200, description = "The newly selected row.", body = dto::ProviderResponse),
        (status = 400, description = "The body did not parse.", body = dto::ApiErrorBody),
        (status = 404, description = "No such row.", body = dto::ApiErrorBody),
    )
)]
async fn set_default_provider(
    State(app): State<App>,
    Body(body): Body<dto::SetDefaultProvider>,
) -> Result<Json<dto::ProviderResponse>, ApiError> {
    if body.provider_id.is_empty() {
        return Err(ApiError::bad_request("providerId must not be empty"));
    }
    let provider = app
        .store
        .providers()
        .set_default(&body.provider_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Unknown provider"))?;
    Ok(Json(dto::ProviderResponse {
        provider: provider.into(),
    }))
}

/// Create or update an LLM provider. The id is the client's.
///
/// A credential that is absent or an empty string keeps the stored value; `null` clears it. A
/// save never moves the selection and never rewrites `createdAt`.
#[utoipa::path(
    put,
    path = "/providers/{providerId}",
    tag = "providers",
    operation_id = "saveProvider",
    params(dto::ProviderPath),
    request_body = dto::ProviderInput,
    responses(
        (status = 200, description = "The saved row, without secrets.", body = dto::ProviderResponse),
        (status = 400, description = "The body is not a provider.", body = dto::ApiErrorBody),
    )
)]
async fn save_provider(
    State(app): State<App>,
    PathParams(path): PathParams<dto::ProviderPath>,
    Body(body): Body<dto::ProviderInput>,
) -> Result<Json<dto::ProviderResponse>, ApiError> {
    let edit = ProviderEdit::try_from(body).map_err(ApiError::bad_request)?;
    let provider = app
        .store
        .providers()
        .save(&path.provider_id, edit)
        .await
        .map_err(|error| {
            if Providers::is_id_conflict(&error) {
                ApiError::bad_request("Provider id belongs to another target")
            } else {
                error.into()
            }
        })?;
    Ok(Json(dto::ProviderResponse {
        provider: provider.into(),
    }))
}

/// Delete a provider row.
#[utoipa::path(
    delete,
    path = "/providers/{providerId}",
    tag = "providers",
    operation_id = "deleteProvider",
    params(dto::ProviderPath),
    responses(
        (status = 200, description = "Deleted.", body = dto::Success),
        (status = 404, description = "No such row.", body = dto::ApiErrorBody),
    )
)]
async fn delete_provider(
    State(app): State<App>,
    PathParams(path): PathParams<dto::ProviderPath>,
) -> Result<Json<dto::Success>, ApiError> {
    if !app.store.providers().delete(&path.provider_id).await? {
        return Err(ApiError::not_found("Unknown provider"));
    }
    Ok(Json(dto::Success { success: true }))
}
