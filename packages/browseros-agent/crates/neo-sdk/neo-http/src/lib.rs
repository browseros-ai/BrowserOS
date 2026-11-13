//! The host API of hosted BrowserOS Neo (claw-sdk): the `/v1` routes of the `neo-sdk-api`
//! contract, the host auth in front of them, and the event stream behind `GET /v1/events`.
//!
//! zuzu (mirrord) is the only client. It reaches Neo over a Unix socket that the app binds;
//! the app builds one [`Service`] from values it already holds (the trace store and sink, the
//! host's [`Settings`],
//! the agent's browser session and viewports, the writer's screenshot jobs, Xvnc's
//! address and home size, and Chromium's CDP port) and serves [`Service::router`] on that socket
//! through [`host_service`]:
//!
//! ```text
//!  HostListener (app) ─ bind ─▶ axum::serve(listener, host_service(routes, auth))
//!                                  │ PeerInfo per connection; peer rule, then bearer token
//!                                  ▼
//!                                router ─ x-request-id ─▶ records, screenshots, locks, fits
//!                                  │                       └▶ TraceStore, Locks, Fits
//!                                  ├▶ /v1/takeovers ─▶ Takeovers ─ the flow over the parts above
//!                                  ├▶ GET /v1/events ◀── EventLog ◀── TraceSink's events,
//!                                  │                     lock and takeover changes, tab.closed
//!                                  ├▶ /v1/tabs/{id}/screencast, /video ◀── CaptureHub ◀── Chromium,
//!                                  │                                             on its own CDP socket
//!                                  ├▶ /v1/tabs/{id}/fill, /fill/check ─▶ the app's FillDoor
//!                                  │    (armed by the app: the host's private typing)
//!                                  ├▶ GET /v1/display ─ WebSocket, RFB ─▶ Displays ─▶ Xvnc
//!                                  │    release=<takeover> binds a control display to it
//!                                  └▶ /v1/settings ─▶ Settings ─ saved beside trace.db; the app's
//!                                       janitor and retention sweep read it on every pass
//!
//!  trace writer ─ ShotJobs ─▶ camera (screenshot_taker) ─ agent's BrowserSession ─▶ JPEG file
//!       ▲                                  │
//!       └──────── ShotAnswer ◀─────────────┘
//!
//!  Fits ─ RFB ─▶ Xvnc (the screen's size)    Fits ─ agent's BrowserSession ─▶ window, viewport
//!
//!  dispatcher lock policy (app) ─▶ Service::locks().check, synchronous
//!  dispatcher door, page.bringToFront (app) ─▶ Service::front(): check (synchronous), and
//!                                              bring_to_front through the front turn
//!  browser link tab events (app) ─▶ Service::tab_closed, retain_tabs
//!       a reconcile's fence (app) ─▶ Service::tab_fence, before its listing; the routes that
//!                                    change a tab stamp it first (tab_stamps)
//!  idle-tab janitor (app) ─▶ Service::tab_hold: a lock or a fit on the tab, now or until when
//! ```
//!
//! The app implements no trait for this crate; it arms the fill routes with a closure
//! ([`Service::arm_fill`]), since what a fill does lives beside its dispatcher. The takeover parts themselves live in
//! `neo-takeover`, which knows nothing of HTTP; this crate builds them once and serves
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
mod fill_route;
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
mod service;
mod setting;
mod settings;
mod settings_routes;
mod tab_routes;
mod tab_stamps;
mod takeover_routes;
mod wire;

pub use fill_route::{FillAsk, FillDoor, FillRefusal, FillReply, FillVerb};
pub use host_auth::{HostAuth, HostToken, PeerRule, TokenError};
pub use host_service::{PeerInfo, host_router, host_service};
pub use service::{Service, ServiceInputs, TabOwner};
pub use settings::{Settings, SettingsDefaults};
pub use tab_stamps::TabFence;
