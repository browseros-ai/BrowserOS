//! The host API of hosted BrowserOS Neo (claw-sdk): the `/v1` routes of the `claw-sdk-api`
//! contract, the host auth in front of them, and the event stream behind `GET /v1/events`.
//!
//! zuzu (mirrord) is the only client. It reaches Neo over a Unix socket that the app binds;
//! the app builds one [`Service`] from values it already holds (the trace store and sink,
//! `Privacy`, the agent's browser session and viewports, the writer's screenshot jobs, Xvnc's
//! address and home size, and Chromium's CDP port) and serves [`Service::router`] on that socket
//! through [`host_service`]:
//!
//! ```text
//!  HostListener (app) ─ bind ─▶ axum::serve(listener, host_service(routes, auth))
//!                                  │ PeerInfo per connection; peer rule, then bearer token
//!                                  ▼
//!                                router ─ x-request-id ─▶ records, screenshots, tab privacy,
//!                                  │                       sweep, secrets, locks, fits
//!                                  │                       └▶ TraceStore, Privacy, Locks, Fits
//!                                  ├▶ /v1/takeovers ─▶ Takeovers ─ the flow over the parts above
//!                                  ├▶ GET /v1/events ◀── EventLog ◀── TraceSink's events,
//!                                  │                     lock and takeover changes, tab.closed
//!                                  ├▶ /v1/tabs/{id}/screencast, /video ◀── CaptureHub ◀── Chromium,
//!                                  │  each frame admitted by a CaptureGuard     on its own CDP socket
//!                                  └▶ GET /v1/display ─ WebSocket, RFB ─▶ Displays ─▶ Xvnc
//!                                       release=<takeover> binds a control display to it
//!
//!  trace writer ─ ShotJobs ─▶ camera (screenshot_taker) ─ agent's BrowserSession ─▶ JPEG file
//!       ▲                                  │
//!       └──────── ShotAnswer ◀─────────────┘
//!
//!  Fits ─ RFB ─▶ Xvnc (the screen's size)    Fits ─ agent's BrowserSession ─▶ window, viewport
//!
//!  dispatcher lock policy (app) ─▶ Service::locks().check, synchronous
//!  browser link tab events (app) ─▶ Service::tab_opened, tab_closed, retain_tabs
//!       a reconcile's fence (app) ─▶ Service::tab_fence, before its listing; the routes that
//!                                    change a tab stamp it first (tab_stamps)
//! ```
//!
//! The app implements no trait for this crate. The takeover parts themselves live in
//! `browseros-takeover`, which knows nothing of HTTP; this crate builds them once and serves
//! their routes.

#![deny(missing_docs)]
#![deny(
    clippy::wildcard_imports,
    clippy::missing_panics_doc,
    clippy::needless_for_each,
    clippy::fn_params_excessive_bools,
    clippy::string_add
)]

mod api_error;
mod display_route;
mod event_log;
mod event_resume;
mod events_route;
mod fit_routes;
mod host_auth;
mod host_service;
mod limits;
mod live_tabs;
mod part_routes;
mod query;
mod record_routes;
mod response_id;
mod router;
mod screenshot_taker;
mod secret_routes;
mod service;
mod tab_routes;
mod tab_stamps;
mod takeover_routes;
mod wire;

pub use host_auth::{HostAuth, HostToken, PeerRule, TokenError};
pub use host_service::{PeerInfo, host_router, host_service};
pub use service::{Service, ServiceInputs};
pub use tab_stamps::TabFence;
