//! `POST /test-provider`: one tiny model call to prove a provider works, through the same model
//! interface a turn uses. Chunk `models` fills the body.

use axum::extract::State;
use axum::response::Response;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::ApiError;
use super::dto;
use super::extract::TestBody;
use crate::app::App;

pub(super) fn routes() -> OpenApiRouter<App> {
    OpenApiRouter::new().routes(routes!(test_provider))
}

/// One tiny model call to prove a provider works.
///
/// A blank credential is filled from the stored row named by `providerId`, and only from a row
/// of the same provider type; when one is borrowed, the stored base URL, resource name and
/// region are forced too, so a saved secret is never sent to an address the caller chose. Every
/// `200` and `400` of this route is `{success, message, responseTime?}`, a body that does not
/// parse included, because the extension reads that one shape at any status.
#[utoipa::path(
    post,
    path = "/test-provider",
    tag = "providers",
    operation_id = "testProvider",
    request_body = dto::TestProviderRequest,
    responses(
        (status = 200, description = "The call succeeded.", body = dto::TestProviderResult),
        (status = 400, description = "The call failed, or the request could not be tested; `message` says why.", body = dto::TestProviderResult),
    )
)]
async fn test_provider(
    State(_app): State<App>,
    TestBody(_body): TestBody<dto::TestProviderRequest>,
) -> Result<Response, ApiError> {
    Err(ApiError::not_implemented())
}
