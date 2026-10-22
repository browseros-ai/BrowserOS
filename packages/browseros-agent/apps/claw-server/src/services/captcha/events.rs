use tokio::sync::broadcast;

use super::types::{CaptchaEvent, CaptchaEventName, ProviderName, SolveOutcome};

#[derive(Clone)]
pub struct CaptchaEventBus {
    sender: broadcast::Sender<CaptchaEvent>,
}

impl Default for CaptchaEventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptchaEventBus {
    pub fn new() -> Self {
        Self {
            sender: broadcast::channel(64).0,
        }
    }
    pub fn subscribe(&self) -> broadcast::Receiver<CaptchaEvent> {
        self.sender.subscribe()
    }
    pub fn emit(&self, event: CaptchaEvent) {
        let _ = self.sender.send(event);
    }
}

pub struct EventLog {
    _private: (),
}

impl EventLog {
    pub fn new(_bus: CaptchaEventBus, _request_id: String, _page: u32) -> Self {
        Self { _private: () }
    }
    pub fn emit(&self, _event: CaptchaEventName, _data: serde_json::Value) {}
    pub fn events(&self) -> Vec<CaptchaEvent> {
        Vec::new()
    }
}

pub fn log_outcome(_outcome: &SolveOutcome, _host: Option<&str>, _provider: Option<ProviderName>) {}
