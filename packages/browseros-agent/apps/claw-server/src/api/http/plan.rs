//! neo Pro: `/api/v1/plan*`, over `services/billing`.
//!
//! Every route here is behind the extension-origin gate in `request_context`, since these can
//! buy, cancel, reset a key and remove computers. `GET /api/v1/plan` never fails; with no
//! billing gateway in the build every other route answers 404 `billing_unavailable`.

use super::error;
use crate::{
    AppState,
    error::{CanonicalError, RequestId},
    services::billing::{
        BillingError, CheckoutPhase, Computer, Key, PlanSnapshot, PortalFlow, Pricing, RefusalCode,
        Secret, Status,
    },
};
use axum::{
    Extension, Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderValue, StatusCode, header::RETRY_AFTER},
    response::{IntoResponse, Response},
};
use claw_api::models::{
    ActivatePlanKeyRequest, CancelPlanRequest, Plan, PlanCheckout, PlanCheckoutPhase,
    PlanCheckoutStatus, PlanComputer, PlanKey, PlanKeyChange, PlanKeyFullError, PlanPortal,
    PlanPortalFlow, PlanPortalRequest, PlanPrice, PlanPricing, PlanRateLimitedError, PlanStatus,
    PriceInterval, PricedPlan, RenamePlanComputerRequest, StartPlanCheckoutRequest,
};

type PlanResult<T> = Result<Json<T>, Response>;

/// GET /api/v1/plan
pub(super) async fn plan(State(state): State<AppState>) -> Json<Plan> {
    Json(to_plan(state.billing.plan().await))
}

/// GET /api/v1/plan/pricing
pub(super) async fn pricing(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> PlanResult<PlanPricing> {
    let pricing = state
        .billing
        .pricing()
        .await
        .map_err(|source| fail(&request_id, source))?;
    Ok(Json(to_pricing(pricing)))
}

/// POST /api/v1/plan/checkout
pub(super) async fn start_checkout(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    payload: Result<Json<StartPlanCheckoutRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<PlanCheckout>), Response> {
    let Json(payload) = body(&request_id, payload, "expected a plan and a billing period")
        .map_err(IntoResponse::into_response)?;
    let interval = payload.interval.to_string();
    let started = state
        .billing
        .start_checkout(&payload.plan, &interval, payload.interval_count)
        .await
        .map_err(|source| fail(&request_id, source))?;
    Ok((
        StatusCode::CREATED,
        Json(PlanCheckout::new(started.checkout_id, started.url)),
    ))
}

/// GET /api/v1/plan/checkout/{checkoutId}
pub(super) async fn checkout(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(checkout_id): Path<String>,
) -> PlanResult<PlanCheckoutStatus> {
    let (phase, plan) = state
        .billing
        .checkout(&checkout_id)
        .await
        .map_err(|source| fail(&request_id, source))?;
    let phase = match phase {
        CheckoutPhase::Open => PlanCheckoutPhase::Open,
        CheckoutPhase::Complete => PlanCheckoutPhase::Complete,
        CheckoutPhase::Expired => PlanCheckoutPhase::Expired,
    };
    Ok(Json(PlanCheckoutStatus::new(phase, plan.map(to_plan))))
}

/// GET /api/v1/plan/key
pub(super) async fn key(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> PlanResult<PlanKey> {
    let key = state
        .billing
        .key()
        .await
        .map_err(|source| fail(&request_id, source))?;
    Ok(Json(to_key(key)))
}

/// POST /api/v1/plan/key/activate
pub(super) async fn activate(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    payload: Result<Json<ActivatePlanKeyRequest>, JsonRejection>,
) -> PlanResult<PlanKeyChange> {
    let Json(ActivatePlanKeyRequest { key, replace }) =
        body(&request_id, payload, "expected a licence key")
            .map_err(IntoResponse::into_response)?;
    let (plan, key) = state
        .billing
        .activate(Secret::new(key), replace)
        .await
        .map_err(|source| fail(&request_id, source))?;
    Ok(Json(PlanKeyChange::new(to_plan(plan), key.map(to_key))))
}

/// POST /api/v1/plan/key/computers/{installId}/remove
pub(super) async fn remove_computer(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(install_id): Path<String>,
) -> PlanResult<PlanKeyChange> {
    let (plan, key) = state
        .billing
        .remove(&install_id)
        .await
        .map_err(|source| fail(&request_id, source))?;
    Ok(Json(PlanKeyChange::new(to_plan(plan), key.map(to_key))))
}

/// POST /api/v1/plan/key/computers/{installId}/rename
pub(super) async fn rename_computer(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    Path(install_id): Path<String>,
    payload: Result<Json<RenamePlanComputerRequest>, JsonRejection>,
) -> PlanResult<PlanKey> {
    let Json(payload) =
        body(&request_id, payload, "expected a name").map_err(IntoResponse::into_response)?;
    let key = state
        .billing
        .rename(&install_id, &payload.name)
        .await
        .map_err(|source| fail(&request_id, source))?;
    Ok(Json(to_key(key)))
}

/// POST /api/v1/plan/key/reset
pub(super) async fn reset_key(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> PlanResult<PlanKeyChange> {
    let (plan, key) = state
        .billing
        .reset()
        .await
        .map_err(|source| fail(&request_id, source))?;
    Ok(Json(PlanKeyChange::new(to_plan(plan), key.map(to_key))))
}

/// POST /api/v1/plan/cancel
pub(super) async fn cancel(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    payload: Result<Json<CancelPlanRequest>, JsonRejection>,
) -> PlanResult<Plan> {
    let Json(payload) = body(&request_id, payload, "expected a cancellation")
        .map_err(IntoResponse::into_response)?;
    let plan = state
        .billing
        .cancel(payload.feedback)
        .await
        .map_err(|source| fail(&request_id, source))?;
    Ok(Json(to_plan(plan)))
}

/// POST /api/v1/plan/resume
pub(super) async fn resume(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> PlanResult<Plan> {
    let plan = state
        .billing
        .resume()
        .await
        .map_err(|source| fail(&request_id, source))?;
    Ok(Json(to_plan(plan)))
}

/// POST /api/v1/plan/portal
pub(super) async fn portal(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
    payload: Result<Json<PlanPortalRequest>, JsonRejection>,
) -> PlanResult<PlanPortal> {
    let Json(payload) = body(&request_id, payload, "expected a portal flow")
        .map_err(IntoResponse::into_response)?;
    let flow = match payload.flow {
        PlanPortalFlow::Billing => PortalFlow::Billing,
        PlanPortalFlow::UpdateCard => PortalFlow::UpdateCard,
    };
    let url = state
        .billing
        .portal(flow)
        .await
        .map_err(|source| fail(&request_id, source))?;
    Ok(Json(PlanPortal::new(url)))
}

/// POST /api/v1/plan/refresh
pub(super) async fn refresh(
    Extension(request_id): Extension<RequestId>,
    State(state): State<AppState>,
) -> PlanResult<Plan> {
    let plan = state
        .billing
        .refresh()
        .await
        .map_err(|source| fail(&request_id, source))?;
    Ok(Json(to_plan(plan)))
}

/// The rejection's own text is not returned: serde can quote the value it choked on, which
/// on activate is the pasted key.
fn body<T>(
    request_id: &RequestId,
    payload: Result<Json<T>, JsonRejection>,
    message: &str,
) -> Result<Json<T>, CanonicalError> {
    payload.map_err(|_| {
        error(
            request_id,
            StatusCode::BAD_REQUEST,
            "invalid_request",
            message,
        )
    })
}

fn fail(request_id: &RequestId, source: BillingError) -> Response {
    let refusal = match source {
        BillingError::Unavailable => {
            return error(
                request_id,
                StatusCode::NOT_FOUND,
                "billing_unavailable",
                "neo Pro isn't available in this version of BrowserOS neo.",
            )
            .into_response();
        }
        BillingError::Unreachable => {
            return error(
                request_id,
                StatusCode::SERVICE_UNAVAILABLE,
                "billing_unreachable",
                "neo can't reach billing right now. Check your connection and try again.",
            )
            .into_response();
        }
        BillingError::Unauthorized | BillingError::Internal(_) => {
            let detail = match &source {
                BillingError::Internal(detail) => detail.as_str(),
                _ => "unauthorized",
            };
            tracing::error!(request_id = %request_id.0, detail, "plan route failed");
            return error(
                request_id,
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal server error",
            )
            .into_response();
        }
        BillingError::Refused(refusal) => refusal,
    };
    let (status, code) = match refusal.code {
        RefusalCode::InvalidRequest => (StatusCode::BAD_REQUEST, "invalid_request"),
        RefusalCode::UnknownPlan => (StatusCode::BAD_REQUEST, "unknown_plan"),
        RefusalCode::KeyInvalid => (StatusCode::BAD_REQUEST, "key_invalid"),
        RefusalCode::AlreadySubscribed => (StatusCode::CONFLICT, "already_subscribed"),
        RefusalCode::NotLinked => (StatusCode::CONFLICT, "not_linked"),
        RefusalCode::KeyFull => (StatusCode::CONFLICT, "key_full"),
        RefusalCode::TooManyTries => (StatusCode::TOO_MANY_REQUESTS, "too_many_tries"),
        RefusalCode::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        RefusalCode::NotFound => (StatusCode::NOT_FOUND, "not_found"),
    };
    match refusal.code {
        RefusalCode::KeyFull => {
            let mut body = PlanKeyFullError::new(code.to_string(), refusal.message);
            body.request_id = Some(request_id.0.clone());
            body.computers = Some(refusal.computers.into_iter().map(to_computer).collect());
            (status, Json(body)).into_response()
        }
        RefusalCode::TooManyTries | RefusalCode::RateLimited => {
            let mut body = PlanRateLimitedError::new(code.to_string(), refusal.message);
            body.request_id = Some(request_id.0.clone());
            body.retry_after_seconds = refusal
                .retry_after_seconds
                .and_then(|seconds| i32::try_from(seconds).ok());
            let mut response = (status, Json(body)).into_response();
            if let Some(seconds) = refusal.retry_after_seconds {
                response
                    .headers_mut()
                    .insert(RETRY_AFTER, HeaderValue::from(seconds));
            }
            response
        }
        _ => error(request_id, status, code, &refusal.message).into_response(),
    }
}

fn to_plan(plan: PlanSnapshot) -> Plan {
    Plan::new(
        plan.available,
        plan.plan,
        plan.plan_name,
        plan.features,
        match plan.status {
            Status::None => PlanStatus::None,
            Status::Active => PlanStatus::Active,
            Status::Ending => PlanStatus::Ending,
            Status::PaymentFailed => PlanStatus::PaymentFailed,
            Status::Unpaid => PlanStatus::Unpaid,
        },
        plan.period_end,
        plan.ends_at,
        plan.grace_until,
        plan.on_key,
        plan.checked_at,
        plan.stale,
    )
}

fn to_key(key: Key) -> PlanKey {
    PlanKey::new(
        key.hint,
        key.cap,
        key.computers.into_iter().map(to_computer).collect(),
    )
}

fn to_computer(computer: Computer) -> PlanComputer {
    PlanComputer::new(
        computer.install_id,
        computer.label,
        computer.platform,
        computer.last_seen_at,
        computer.activated_at,
        computer.this_computer,
    )
}

/// A price on a period this build cannot show is left out rather than shown wrong.
fn to_pricing(pricing: Pricing) -> PlanPricing {
    PlanPricing::new(
        pricing
            .plans
            .into_iter()
            .map(|plan| {
                PricedPlan::new(
                    plan.plan,
                    plan.name,
                    plan.description,
                    plan.features,
                    plan.prices
                        .into_iter()
                        .filter_map(|price| {
                            let interval = match price.interval.as_str() {
                                "day" => PriceInterval::Day,
                                "week" => PriceInterval::Week,
                                "month" => PriceInterval::Month,
                                "year" => PriceInterval::Year,
                                _ => return None,
                            };
                            Some(PlanPrice::new(
                                interval,
                                price.interval_count,
                                price.amount,
                                price.currency,
                            ))
                        })
                        .collect(),
                )
            })
            .collect(),
    )
}
