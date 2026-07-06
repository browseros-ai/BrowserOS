pub mod agents;
pub mod audit;
pub mod browser;
pub mod harness;
pub mod replay;
pub mod screencast;
pub mod screenshots;
pub mod tab_activity;

pub(crate) fn now_epoch_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

pub(crate) fn now_iso() -> String {
    let now = time::OffsetDateTime::now_utc();
    match now.format(&time::format_description::well_known::Rfc3339) {
        Ok(value) => value,
        Err(_) => now.unix_timestamp().to_string(),
    }
}
