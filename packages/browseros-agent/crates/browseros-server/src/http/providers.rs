//! `/providers`: the LLM provider rows and the selected chat target. Reads never carry a secret
//! (`ProviderRow` has only `has_*` flags); a save takes credentials inward only. Chunk
//! `settings` fills the bodies.

use axum::extract::State;
use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::ApiError;
use super::dto;
use super::extract::{Body, PathParams};
use crate::app::App;

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
async fn list_providers(State(_app): State<App>) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
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
async fn get_default_provider(State(_app): State<App>) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
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
    State(_app): State<App>,
    Body(_body): Body<dto::SetDefaultProvider>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
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
    State(_app): State<App>,
    PathParams(_path): PathParams<dto::ProviderPath>,
    Body(body): Body<dto::ProviderInput>,
) -> Result<Response, ApiError> {
    let _edit = crate::store::ProviderEdit::try_from(body).map_err(ApiError::bad_request)?;
    Err(ApiError::not_implemented())
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
    State(_app): State<App>,
    PathParams(_path): PathParams<dto::ProviderPath>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
}
