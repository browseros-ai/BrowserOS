//! `/oauth/{provider}`: "Sign in with ChatGPT". The handlers see only `model::SignIn`'s three
//! calls; ports, client ids and tokens stay inside `model::chatgpt`. Chunk `chatgpt` fills the
//! bodies.

use axum::extract::State;
use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::ApiError;
use super::dto;
use super::extract::PathParams;
use crate::app::App;

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
    State(_app): State<App>,
    PathParams(_path): PathParams<dto::SignInPath>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
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
        (status = 503, description = "The install id is unreadable, so no sign-in can be stored.", body = dto::ApiErrorBody),
    )
)]
async fn get_sign_in_status(
    State(_app): State<App>,
    PathParams(_path): PathParams<dto::SignInPath>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
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
        (status = 503, description = "The install id is unreadable, so no sign-in can be stored.", body = dto::ApiErrorBody),
    )
)]
async fn sign_out(
    State(_app): State<App>,
    PathParams(_path): PathParams<dto::SignInPath>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
}
