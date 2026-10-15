//! PostHog metrics (`http_server.started`, chat turn start/finish, `tool_executed` 30-min
//! rollup); disabled without an install id or a build-time key. Implemented by P13.

#[derive(Debug, Clone, Default)]
pub struct Metrics {
    _private: (),
}

impl Metrics {
    /// S0 stub: a disabled sink.
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn capture(&self, event: &str, properties: serde_json::Value) {
        let _ = (event, properties);
    }
}
