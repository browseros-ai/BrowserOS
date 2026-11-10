//! Rust stand-ins for the browser things a script holds: pages, tabs, dialogs, downloads, routes, requests, storage, clock.

pub(crate) mod bindings;
pub(crate) mod clock;
pub(crate) mod console;
pub(crate) mod cookies_and_storage;
pub(crate) mod dialogs;
pub(crate) mod downloads;
pub(crate) mod emulation;
pub(crate) mod init_scripts;
pub(crate) mod locator_handlers;
pub(crate) mod network;
pub(crate) mod page;
pub(crate) mod responses;
pub(crate) mod routes;
pub(crate) mod tabs;
pub(crate) mod web_storage;
