pub mod config;
pub mod detect;
pub mod events;
pub mod page;
pub mod providers;
pub mod recaptcha;
pub mod reservation;
pub mod turnstile;
pub mod twocaptcha;
pub mod types;

use std::sync::Arc;

use browseros_core::{BrowserView, PageId};
use tokio_util::sync::CancellationToken;

use self::{
    events::{CaptchaEventBus, EventLog},
    page::CaptchaPage,
    providers::{CaptchaBackend, ProviderChain},
    reservation::{ActiveSolve, SolveLease, TabReservations},
    types::{Attempts, DocumentId, SolveMethod, SolveOutcome, SolveReason, SolveStatus},
};

pub struct SolveCtx<'a, P: CaptchaPage, B: CaptchaBackend> {
    pub page: &'a P,
    pub backend: &'a B,
    pub events: &'a EventLog,
    pub lease: &'a SolveLease,
    pub cancel: &'a CancellationToken,
    pub deadline: tokio::time::Instant,
    pub page_url: String,
    pub document: DocumentId,
    pub user_agent: Option<String>,
}

pub struct CaptchaService {
    pub providers: ProviderChain,
    reservations: Arc<TabReservations>,
    events: CaptchaEventBus,
}

#[derive(Debug)]
pub enum SolveRefusal {
    InvalidPage(String),
    Busy(ActiveSolve),
    BrowserUnavailable,
}

impl CaptchaService {
    pub fn from_config(config: config::CaptchaConfig) -> Option<Arc<Self>> {
        if !config.has_provider() {
            return None;
        }
        Some(Arc::new(Self {
            providers: ProviderChain::new(&config),
            reservations: Arc::default(),
            events: CaptchaEventBus::new(),
        }))
    }

    pub fn from_env() -> Option<Arc<Self>> {
        Self::from_config(config::CaptchaConfig::from_env())
    }
    pub fn reservations(&self) -> &Arc<TabReservations> {
        &self.reservations
    }
    pub fn events(&self) -> &CaptchaEventBus {
        &self.events
    }

    pub async fn solve(
        &self,
        _view: BrowserView,
        page: PageId,
        _cancel: CancellationToken,
        _human_help_available: bool,
    ) -> Result<SolveOutcome, SolveRefusal> {
        Ok(SolveOutcome {
            status: SolveStatus::Failed,
            kind: None,
            method: SolveMethod::None,
            reason: SolveReason::BrowserError,
            request_id: format!("cap_{}", ulid::Ulid::new()),
            page: page.0,
            duration_ms: 0,
            attempts: Attempts::default(),
            paid_tasks: 0,
            cost_usd: 0.0,
            message: "solve_captcha is not implemented yet".into(),
            events: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_that_coordination_stubs_use_stable_constructors() -> anyhow::Result<()> {
        let reservations = Arc::<TabReservations>::default();
        let lease = reservations
            .try_reserve(42, "cap_test")
            .map_err(|_| anyhow::anyhow!("unexpected busy page"))?;
        let log = EventLog::new(CaptchaEventBus::new(), "cap_test".into(), 42);
        assert_eq!(lease.page(), 42);
        assert_eq!(lease.request_id(), "cap_test");
        assert!(log.events().is_empty());
        Ok(())
    }

    #[test]
    fn tests_that_service_is_disabled_without_a_provider() {
        assert!(CaptchaService::from_config(config::CaptchaConfig::default()).is_none());
    }

    #[test]
    fn tests_that_service_is_enabled_with_a_provider() -> anyhow::Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), "test-key")?;
        let config = config::CaptchaConfig::from_lookup(|name| {
            (name == config::CAPSOLVER_KEY_FILE_ENV)
                .then(|| file.path().to_string_lossy().into_owned())
        });
        assert!(CaptchaService::from_config(config).is_some());
        Ok(())
    }
}
