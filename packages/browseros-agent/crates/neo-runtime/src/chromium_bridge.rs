//! Talks to Chromium: calls into each page's JS worlds, finds frames, writes mouse and keyboard input, and turns Chromium's events into the run's events.

pub(crate) mod contexts;
pub(crate) mod discovery;
pub(crate) mod frame_events;
pub(crate) mod frame_selector;
pub(crate) mod frames;
pub(crate) mod input_devices;
pub(crate) mod keyboard;
pub(crate) mod main_world;
pub(crate) mod network_events;
pub(crate) mod page_events;
pub(crate) mod switch;
pub(crate) mod world;
