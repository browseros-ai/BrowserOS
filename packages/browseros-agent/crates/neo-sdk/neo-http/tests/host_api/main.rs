//! The host API as zuzu sees it: requests through `Service::router` over a real trace store and a
//! scripted browser, and through `host_service` on a real Unix socket.

mod browser;
mod display;
mod downloads;
mod errors;
mod events;
mod fill;
mod fits;
mod listener_compatibility;
mod live;
mod locks;
mod reconcile;
mod records;
mod screenshots;
mod settings;
mod socket;
mod support;
mod tabs;
mod takeovers;
mod trace_views;
mod waiting;
