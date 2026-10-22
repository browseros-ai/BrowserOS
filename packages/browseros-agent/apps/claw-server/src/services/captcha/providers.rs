use std::future::Future;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{
    config::CaptchaConfig,
    types::{ProviderAnswer, ProviderError, ProviderErrorKind, SolveTask},
};

pub trait CaptchaBackend: Send + Sync {
    fn is_configured(&self) -> bool;
    fn solve(
        &self,
        task: &SolveTask,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> impl Future<Output = Result<ProviderAnswer, ProviderError>> + Send;
}

pub struct ProviderChain {
    configured: bool,
}

impl ProviderChain {
    pub fn new(config: &CaptchaConfig) -> Self {
        Self {
            configured: config.has_provider(),
        }
    }
    pub fn paid_tasks(&self) -> u32 {
        0
    }
}

impl CaptchaBackend for ProviderChain {
    fn is_configured(&self) -> bool {
        self.configured
    }

    async fn solve(
        &self,
        _task: &SolveTask,
        _deadline: Instant,
        _cancel: &CancellationToken,
    ) -> Result<ProviderAnswer, ProviderError> {
        Err(ProviderError {
            provider: None,
            kind: ProviderErrorKind::NotConfigured,
            message: "not implemented".into(),
            charged: false,
        })
    }
}
