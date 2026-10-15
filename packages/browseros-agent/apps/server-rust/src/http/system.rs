//! Liveness remains 200 during browser reconnects; OTA readiness independently reads
//! active work. Diagnostics share one process-stable OS lookup and shutdown only signals
//! the process owner after yielding the response to the HTTP runtime.

use crate::{AppState, VERSION};
use axum::{
    Json,
    extract::State,
    response::{IntoResponse, Response},
};
use server_api::models::{DiagnosticsResponse, HealthResponse, ShutdownResponse, StatusResponse};
use tokio::sync::OnceCell;

pub async fn health(State(state): State<AppState>) -> Response {
    Json(HealthResponse {
        cdp_connected: Some(state.cdp_connected().await),
        ..Default::default()
    })
    .into_response()
}
pub async fn status(State(state): State<AppState>) -> Response {
    Json(StatusResponse {
        cdp_connected: Some(state.cdp_connected().await),
        can_update: state.activity.can_update(),
        ..Default::default()
    })
    .into_response()
}
pub async fn diagnostics() -> Response {
    static DETAILS: OnceCell<DiagnosticsResponse> = OnceCell::const_new();
    Json(DETAILS.get_or_init(system_details).await.clone()).into_response()
}
pub async fn shutdown(State(state): State<AppState>) -> Response {
    tokio::spawn(async move {
        // Like TS setImmediate: return the response first. The process owner then
        // drains HTTP before stopping OAuth/Klavis/ACP and removing discovery.
        tokio::task::yield_now().await;
        state.shutdown.trigger();
    });
    Json(ShutdownResponse::default()).into_response()
}

async fn system_details() -> DiagnosticsResponse {
    let (os, os_version) = if cfg!(target_os = "macos") {
        let mut command = tokio::process::Command::new("/usr/bin/sw_vers");
        command.arg("-productVersion").kill_on_drop(true);
        let version = tokio::time::timeout(std::time::Duration::from_secs(1), command.output())
            .await
            .ok()
            .and_then(Result::ok)
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|value| !value.is_empty());
        ("macOS".to_owned(), version)
    } else if cfg!(target_os = "linux") {
        parse_os_release(
            &tokio::fs::read_to_string("/etc/os-release")
                .await
                .unwrap_or_default(),
        )
    } else {
        let version = tokio::process::Command::new("cmd")
            .args(["/C", "ver"])
            .kill_on_drop(true)
            .output()
            .await
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
        (
            if cfg!(windows) {
                "Windows"
            } else {
                std::env::consts::OS
            }
            .to_owned(),
            version,
        )
    };
    DiagnosticsResponse::new(VERSION.to_owned(), os, os_version)
}

pub fn parse_os_release(contents: &str) -> (String, Option<String>) {
    let mut name = None;
    let mut version = None;
    for line in contents.lines() {
        if let Some((key, value)) = line.split_once('=') {
            let value = if value.len() >= 2
                && ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\'')))
            {
                &value[1..value.len() - 1]
            } else {
                value
            };
            let value: String = value.chars().take(200).collect();
            let value = (!value.is_empty()).then_some(value);
            match key {
                "NAME" => name = value,
                "VERSION_ID" => version = value,
                _ => {}
            }
        }
    }
    (name.unwrap_or_else(|| "Linux".into()), version)
}
