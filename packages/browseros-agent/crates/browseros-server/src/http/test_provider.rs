//! `POST /test-provider`: one tiny model call to prove a provider works, through the same model
//! interface a turn uses.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::dto;
use super::extract::TestBody;
use crate::app::App;
use crate::store::{ProviderDraft, ProviderKind, Secret};

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
    State(app): State<App>,
    TestBody(body): TestBody<dto::TestProviderRequest>,
) -> Response {
    match test(&app, body).await {
        Ok(elapsed) => (
            StatusCode::OK,
            Json(dto::TestProviderResult {
                success: true,
                message: "Connection successful.".into(),
                response_time: u64::try_from(elapsed.as_millis()).ok(),
            }),
        ),
        Err(message) => (
            StatusCode::BAD_REQUEST,
            Json(dto::TestProviderResult {
                success: false,
                message,
                response_time: None,
            }),
        ),
    }
    .into_response()
}

async fn test(app: &App, body: dto::TestProviderRequest) -> Result<std::time::Duration, String> {
    if body.model.trim().is_empty() {
        return Err("Enter a model name.".into());
    }
    let kind = ProviderKind::from(body.provider);
    let saved = match non_blank(body.provider_id) {
        Some(id) => app
            .store
            .providers()
            .config(Some(&id))
            .await
            .map_err(|_| "Could not read the saved provider settings.")?,
        None => None,
    };
    // Keep credential borrowing and destination binding in the store's single merge rule.
    let draft = ProviderDraft {
        kind,
        model: body.model,
        base_url: non_blank(body.base_url),
        headers: body.headers.unwrap_or_default().into_iter().collect(),
        resource_name: non_blank(body.resource_name),
        region: non_blank(body.region),
        api_key: non_blank(body.api_key).map(Secret::new),
        access_key_id: non_blank(body.access_key_id).map(Secret::new),
        secret_access_key: non_blank(body.secret_access_key).map(Secret::new),
        session_token: non_blank(body.session_token).map(Secret::new),
    };
    let config = draft.resolve(saved).map_err(|error| error.to_string())?;
    app.models.test(config).await.map_err(|error| error.message)
}

fn non_blank(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}
