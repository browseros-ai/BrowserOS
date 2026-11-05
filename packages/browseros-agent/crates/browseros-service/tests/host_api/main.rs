//! The host API as zuzu sees it: requests through `Service::router` over a real trace store and a
//! scripted browser, and through `host_service` on a real Unix socket.

mod browser;
mod display;
mod errors;
mod events;
mod fits;
mod locks;
mod privacy;
mod records;
mod screenshots;
mod socket;
mod support;
mod sweep;
mod takeovers;
mod waiting;
