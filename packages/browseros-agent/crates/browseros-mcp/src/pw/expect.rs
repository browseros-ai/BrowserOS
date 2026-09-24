//! Web-first assertions; P6 replaces this dispatch stub.

use super::PwCallOutcome;
use crate::tools::run::BrowserBridge;
use serde_json::Value;

pub(crate) async fn dispatch(
    _bridge: &BrowserBridge,
    method: &str,
    _args: &[Value],
) -> Result<PwCallOutcome, String> {
    Err(format!("not implemented yet: {method}"))
}
