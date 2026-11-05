//! The host API of hosted BrowserOS Neo (claw-sdk): the `/v1` routes of the `claw-sdk-api`
//! contract, the host auth in front of them, and the event stream behind `GET /v1/events`.
//!
//! zuzu (mirrord) is the only client. It reaches Neo over a Unix socket that the app binds;
//! the app builds one [`Service`] from values it already holds (the trace store and sink,
//! `Privacy`, the agent's browser session and viewports, and the writer's screenshot jobs) and
//! serves [`Service::router`] on that socket through [`host_service`]:
//!
//! ```text
//!  HostListener (app) ─ bind ─▶ axum::serve(listener, host_service(routes, auth))
//!                                  │ PeerInfo per connection; peer rule, then bearer token
//!                                  ▼
//!                                router ─ x-request-id ─▶ records, screenshots, tab privacy,
//!                                  │                       secrets
//!                                  │                       └▶ TraceStore, Privacy
//!                                  └▶ GET /v1/events ◀── EventLog ◀── TraceSink's events
//!
//!  trace writer ─ ShotJobs ─▶ camera (screenshot_taker) ─ agent's BrowserSession ─▶ JPEG file
//!       ▲                                  │
//!       └──────── ShotAnswer ◀─────────────┘
//! ```
//!
//! The app implements no trait for this crate.

#![deny(missing_docs)]
#![deny(
    clippy::wildcard_imports,
    clippy::missing_panics_doc,
    clippy::needless_for_each,
    clippy::fn_params_excessive_bools,
    clippy::string_add
)]

mod api_error;
mod event_log;
mod event_resume;
mod events_route;
mod host_auth;
mod host_service;
mod limits;
mod query;
mod record_routes;
mod response_id;
mod router;
mod screenshot_taker;
mod secret_routes;
mod service;
mod tab_routes;
mod wire;

pub use host_auth::{HostAuth, HostToken, PeerRule, TokenError};
pub use host_service::{PeerInfo, host_router, host_service};
pub use service::{Service, ServiceInputs};
