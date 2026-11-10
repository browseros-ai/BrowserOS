//! What a process starts with (design D2): Chromium's launch JSON, a handful of env vars, and
//! named constants. There is no config file. Choices about one run arrive in its `_meta`, and
//! choices about one takeover arrive in its request body.
//!
//! ```text
//!  --config <file> ─▶ LaunchConfig::read ────────┐
//!  process env     ─▶ EnvConfig::from_lookup ────┤
//!                  ─▶ CaptchaConfig::from_lookup ┼▶ AppConfig: parsed once in main, trusted after
//!  both, and the cwd ─▶ SecretFiles::locate ─────┘
//!  CaptchaConfig: neo-captcha's provider key files, read once
//!  SecretFiles: where Neo's own secrets are on disk, which no upload hands a page
//!  Defaults: fixed values; changing one is a code change, reviewed like one
//! ```
//!
//! Everything is validated here, at the process boundary, so the rest of the crate reads typed
//! fields and never an env var or a JSON key.

mod app_config;
mod defaults;
mod env_config;
mod launch_config;
mod secret_files;

pub use app_config::{AppConfig, ConfigError};
pub(crate) use defaults::Defaults;
pub(crate) use env_config::{EnvConfig, HostApiConfig, HostedMode};
pub(crate) use launch_config::LaunchConfig;
pub(crate) use secret_files::SecretFiles;
