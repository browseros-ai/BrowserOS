//! Playwright's methods: what each call a script makes (`click`, `fill`, `goto`, `textContent`, ...) does.
//!
//! `tab` owns the browser seam, `retry` the shared retry policy, and `aim` element preparation.
//! Typed pointer entry points live in `click`; check and drag compose them, while non-pointer
//! families use the neutral owners directly. The tab/input bridge shares plans and answers.

pub(crate) mod aim;
pub(crate) mod aria_yaml;
pub(crate) mod browser_methods;
pub(crate) mod capture;
pub(crate) mod check;
pub(crate) mod click;
pub(crate) mod content;
pub(crate) mod devices;
pub(crate) mod drag;
pub(crate) mod drop_on;
pub(crate) mod element_ops;
pub(crate) mod evaluate;
pub(crate) mod fill;
pub(crate) mod forms;
pub(crate) mod frame_element;
pub(crate) mod getters;
pub(crate) mod highlight;
pub(crate) mod history;
pub(crate) mod lists;
pub(crate) mod navigate;
pub(crate) mod network_idle;
pub(crate) mod page_and_context_methods;
pub(crate) mod pdf;
pub(crate) mod press;
pub(crate) mod reads;
pub(crate) mod retry;
pub(crate) mod screenshot;
pub(crate) mod set_input_files;
pub(crate) mod states;
pub(crate) mod tab;
pub(crate) mod waits;
