//! `POST /v1/secrets`: the host's secret registration. The value joins the one process-wide
//! secret set (`Privacy::secrets`), which masks every record Neo writes and every result the
//! agent reads from then on. It joins as a host value, which no flood of page or agent values
//! pushes out (sdk-traces decision 53). A value is never logged or echoed, not even in an error.

use crate::api_error::ApiError;
use crate::service::ServiceState;
use crate::wire;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use neo_trace::{Registration, SecretSource};
use std::sync::Arc;

pub(crate) async fn register_secret(
    State(service): State<Arc<ServiceState>>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let (value, ttl) = wire::secret_request(&body)?;
    match service
        .privacy
        .secrets()
        .register_from(value, ttl, SecretSource::Host)
    {
        Registration::Registered | Registration::Refreshed => Ok(StatusCode::NO_CONTENT),
        Registration::TooShort => Err(ApiError::invalid(
            "the value is too short to mask without hiding ordinary text",
        )),
        // The set never refuses a host value. One it did would be masked nowhere, so the host
        // must not take a 204 for it.
        Registration::Full => Err(ApiError::internal("the secret set refused a host value")),
    }
}
