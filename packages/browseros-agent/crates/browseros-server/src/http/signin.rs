//! Extension-facing routes for ChatGPT sign-in. The model owns the callback and token storage.

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::ApiError;
use super::dto;
use super::extract::PathParams;
use crate::app::App;
use crate::model::SignInError;

pub(super) fn routes() -> OpenApiRouter<App> {
    OpenApiRouter::new()
        .routes(routes!(start_sign_in))
        .routes(routes!(get_sign_in_status))
        .routes(routes!(sign_out))
}

/// Begin "Sign in with ChatGPT".
///
/// Opens the callback listener on `127.0.0.1:1455` for as long as a sign-in is pending and
/// redirects to OpenAI's authorize page. The extension follows the redirect and opens the final
/// URL in a tab.
#[utoipa::path(
    get,
    path = "/oauth/{provider}/start",
    tag = "signin",
    operation_id = "startSignIn",
    params(dto::SignInPath),
    responses(
        (status = 302, description = "To the authorize URL.",
            headers(("location" = String, description = "OpenAI's authorize URL."))),
        (status = 400, description = "Not a sign-in provider.", body = dto::ApiErrorBody),
        (status = 500, description = "The sign-in could not start for another reason.", body = dto::ApiErrorBody),
        (status = 503, description = "Port 1455 is held by another program, or the install id is unreadable.", body = dto::ApiErrorBody),
    )
)]
async fn start_sign_in(
    State(app): State<App>,
    PathParams(_path): PathParams<dto::SignInPath>,
) -> Result<Response, ApiError> {
    let authorize = app.models.sign_in().start().await?;
    Ok((StatusCode::FOUND, [(header::LOCATION, authorize.as_str())]).into_response())
}

/// Whether tokens are stored for this install.
#[utoipa::path(
    get,
    path = "/oauth/{provider}/status",
    tag = "signin",
    operation_id = "getSignInStatus",
    params(dto::SignInPath),
    responses(
        (status = 200, description = "Whether tokens are stored for this install.", body = dto::SignInStatusBody),
        (status = 400, description = "Not a sign-in provider.", body = dto::ApiErrorBody),
        (status = 500, description = "The sign-in store could not be read.", body = dto::ApiErrorBody),
        (status = 503, description = "The install id is unreadable, so no sign-in can be stored.", body = dto::ApiErrorBody),
    )
)]
async fn get_sign_in_status(
    State(app): State<App>,
    PathParams(path): PathParams<dto::SignInPath>,
) -> Result<Json<dto::SignInStatusBody>, ApiError> {
    let status = app.models.sign_in().status().await?;
    Ok(Json(dto::SignInStatusBody::new(path.provider, status)))
}

/// Remove the stored tokens.
#[utoipa::path(
    delete,
    path = "/oauth/{provider}",
    tag = "signin",
    operation_id = "signOut",
    params(dto::SignInPath),
    responses(
        (status = 200, description = "Tokens removed.", body = dto::Success),
        (status = 400, description = "Not a sign-in provider.", body = dto::ApiErrorBody),
        (status = 500, description = "The sign-in could not be removed from the store.", body = dto::ApiErrorBody),
        (status = 503, description = "The install id is unreadable, so no sign-in can be stored.", body = dto::ApiErrorBody),
    )
)]
async fn sign_out(
    State(app): State<App>,
    PathParams(_path): PathParams<dto::SignInPath>,
) -> Result<Json<dto::Success>, ApiError> {
    app.models.sign_in().sign_out().await?;
    Ok(Json(dto::Success { success: true }))
}

impl From<SignInError> for ApiError {
    fn from(error: SignInError) -> Self {
        match error {
            SignInError::NoInstallId => {
                Self::unavailable("Could not read the BrowserOS installation id.")
            }
            SignInError::PortBusy => Self::unavailable(
                "The ChatGPT callback port is in use. Close the other sign-in and try again.",
            ),
            SignInError::Failed(message) => Self::new(StatusCode::INTERNAL_SERVER_ERROR, message),
        }
    }
}
