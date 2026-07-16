pub mod agents;
pub mod audit;
pub mod browser;
pub mod harness;
pub mod replay;
pub mod replay_tabs;
pub mod screencast;
pub mod screenshots;
pub mod tab_activity;
pub mod telemetry;

pub(crate) fn now_epoch_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}
