//! Request extractors whose refusals answer in the contract's shapes instead of axum's plain
//! text: `400 {"error": "<what was wrong>"}` for most routes, the chat-error envelope on
//! `POST /chat`, and the connection-test result on `POST /test-provider` (the extension reads
//! that one shape at any status). A handler takes one of these in place of axum's `Json`,
//! `Path` or `Query`, so validation failures are uniform without each handler mapping them.

use axum::Json;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Path, Query, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

use super::ApiError;
use super::dto;
use crate::turn::{ChatError, ChatErrorCode};

/// A JSON body; a body that does not parse is `400 {"error": …}`.
pub struct Body<T>(pub T);

impl<S, T> FromRequest<S> for Body<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Body(value))
            .map_err(|rejection: JsonRejection| ApiError::bad_request(rejection.body_text()))
    }
}

/// `POST /chat`'s body; a body that does not parse is `400` with the chat-error envelope, which
/// the panel renders with the same card as an `error` chunk.
pub struct ChatBody<T>(pub T);

impl<S, T> FromRequest<S> for ChatBody<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| ChatBody(value))
            .map_err(|rejection: JsonRejection| {
                let mut error = ChatError::new(
                    ChatErrorCode::Unknown,
                    "Invalid request",
                    rejection.body_text(),
                );
                error.retryable = false;
                ApiError::chat(StatusCode::BAD_REQUEST, error)
            })
    }
}

/// `POST /test-provider`'s body; a body that does not parse is `400` with
/// `{success: false, message}`.
pub struct TestBody<T>(pub T);

/// The refusal of `TestBody`, in the route's one result shape.
pub struct TestRejection(String);

impl IntoResponse for TestRejection {
    fn into_response(self) -> Response {
        let body = dto::TestProviderResult {
            success: false,
            message: self.0,
            response_time: None,
        };
        (StatusCode::BAD_REQUEST, Json(body)).into_response()
    }
}

impl<S, T> FromRequest<S> for TestBody<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = TestRejection;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| TestBody(value))
            .map_err(|rejection: JsonRejection| TestRejection(rejection.body_text()))
    }
}

/// Path parameters; a segment that does not parse (a conversation id that is not a UUID) is
/// `400 {"error": …}`.
pub struct PathParams<T>(pub T);

impl<S, T> FromRequestParts<S> for PathParams<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| PathParams(value))
            .map_err(|rejection: PathRejection| ApiError::bad_request(rejection.body_text()))
    }
}

/// Query parameters; a value that does not parse is `400 {"error": …}`.
pub struct QueryParams<T>(pub T);

impl<S, T> FromRequestParts<S> for QueryParams<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| QueryParams(value))
            .map_err(|rejection: QueryRejection| ApiError::bad_request(rejection.body_text()))
    }
}
