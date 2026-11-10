//! claw-server's door to the shared solver in `neo-captcha`.
//!
//! The solver itself has no server mode. This wrapper owns claw-server's one policy on top of
//! it: `solve_captcha` is offered only in hosted mode. It keeps the name and signatures the MCP
//! layer (`api/mcp/captcha.rs`, the busy guard, the snapshot status effect, the script hook) was
//! built against, so none of those callers changed when the solver moved out.

use std::sync::Arc;

use browseros_core::{BrowserView, PageId};
use neo_captcha::{CaptchaSolver, HumanHelp, types::SolveOutcome};
use tokio_util::sync::CancellationToken;

pub use neo_captcha::{SolveRefusal, config, reservation};

use crate::config::ServerMode;

pub struct CaptchaService {
    solver: CaptchaSolver,
}

impl CaptchaService {
    /// The provider check runs before the mode check, so a desktop server without a key stays
    /// silent and only a keyed desktop server logs why solving is off.
    pub fn from_config(config: config::CaptchaConfig, mode: ServerMode) -> Option<Arc<Self>> {
        if !config.has_provider() {
            return None;
        }
        if !mode.is_hosted() {
            tracing::info!(target: "captcha", "solve_captcha disabled: desktop mode");
            return None;
        }
        CaptchaSolver::new(config).map(|solver| Arc::new(Self { solver }))
    }

    pub fn from_env(mode: ServerMode) -> Option<Arc<Self>> {
        Self::from_config(config::CaptchaConfig::from_env(), mode)
    }

    pub fn reservations(&self) -> &Arc<reservation::TabReservations> {
        self.solver.reservations()
    }

    pub async fn solve(
        &self,
        view: BrowserView,
        page: PageId,
        cancel: CancellationToken,
        human_help_available: bool,
    ) -> Result<SolveOutcome, SolveRefusal> {
        let human_help = if human_help_available {
            HumanHelp::Available
        } else {
            HumanHelp::Unavailable
        };
        self.solver.solve(view, page, cancel, human_help).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct LogWriter(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LogWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn tests_that_service_is_disabled_without_a_provider() {
        for mode in [ServerMode::Desktop, ServerMode::Hosted] {
            assert!(CaptchaService::from_config(config::CaptchaConfig::default(), mode).is_none());
        }
    }

    #[test]
    fn tests_that_service_requires_hosted_mode_with_either_provider() -> anyhow::Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), "test-key")?;
        for key_env in [
            config::CAPSOLVER_KEY_FILE_ENV,
            config::TWOCAPTCHA_KEY_FILE_ENV,
        ] {
            let config = config::CaptchaConfig::from_lookup(|name| {
                (name == key_env).then(|| file.path().to_string_lossy().into_owned())
            });
            for mode in [ServerMode::Desktop, ServerMode::Hosted] {
                assert_eq!(
                    CaptchaService::from_config(config.clone(), mode).is_some(),
                    mode.is_hosted(),
                    "{key_env} in {mode:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn tests_that_desktop_logs_one_info_line_without_the_key() -> anyhow::Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), "private-test-key")?;
        let config = config::CaptchaConfig::from_lookup(|name| {
            (name == config::CAPSOLVER_KEY_FILE_ENV)
                .then(|| file.path().to_string_lossy().into_owned())
        });
        let writer = LogWriter::default();
        let output = writer.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            assert!(CaptchaService::from_config(config, ServerMode::Desktop).is_none());
        });
        let bytes = output
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let logs = String::from_utf8_lossy(&bytes);
        assert_eq!(logs.lines().count(), 1, "{logs}");
        assert!(logs.contains("INFO"), "{logs}");
        assert!(
            logs.contains("solve_captcha disabled: desktop mode"),
            "{logs}"
        );
        assert!(!logs.contains("private-test-key"));
        Ok(())
    }
}
