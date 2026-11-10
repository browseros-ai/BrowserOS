//! Explicit opt-in and host adapter for Neo CAPTCHA. Claw owns this policy seam;
//! neither neo-runtime nor the shared solver knows hosted mode, environment or key paths.

use crate::{config::ServerMode, services::captcha::CaptchaService};
use ::neo_captcha::{SolveRefusal, TargetSolver, config, script, types::SolveOutcome};
use futures_util::future::BoxFuture;
use neo_runtime::{CaptchaHost, PageId, SolveReply};
use std::{path::Path, sync::Arc};
use tokio_util::sync::CancellationToken;

pub const FLAG: &str = "BROWSERCLAW_NEO_CAPTCHA";

pub enum Activation {
    On(Arc<CaptchaService>),
    Off(Off),
}
#[derive(Debug, PartialEq, Eq)]
pub enum Off {
    NotRequested,
    NotHosted,
    NoReadableKey,
    KeyInsideArtifacts,
}

impl Activation {
    /// Claw owns activation and provider state; the reusable MCP server supplies its one
    /// connected client and retains cancellation/drain/disconnect ownership.
    pub fn install(self, server: neo_sdk_server::NeoSdkServer) -> neo_sdk_server::NeoSdkServer {
        match self {
            Self::On(service) => {
                let solver = Arc::clone(service.solver());
                server.with_captcha(
                    move |cdp| -> Arc<dyn CaptchaHost> {
                        Arc::new(NeoCaptcha(TargetSolver::new(Arc::clone(&solver), cdp)))
                    },
                    script::RUN_INSTRUCTION,
                )
            }
            Self::Off(_) => server,
        }
    }
}

/// Reads FLAG first: off must not inspect even the key environment variables. Canonical
/// containment is checked before reading key contents, including symlinks into artifacts.
pub fn activation(
    env: impl Fn(&str) -> Option<String>,
    mode: ServerMode,
    artifacts: &Path,
) -> Activation {
    if env(FLAG).as_deref() != Some("1") {
        return Activation::Off(Off::NotRequested);
    }
    let off = if !mode.is_hosted() {
        Some(Off::NotHosted)
    } else {
        let artifacts = artifacts.canonicalize().ok();
        let inside = [
            config::CAPSOLVER_KEY_FILE_ENV,
            config::TWOCAPTCHA_KEY_FILE_ENV,
        ]
        .iter()
        .any(|name| {
            env(name)
                .and_then(|path| Path::new(path.trim()).canonicalize().ok())
                .zip(artifacts.as_ref())
                .is_some_and(|(path, artifacts)| path.starts_with(artifacts))
        });
        if inside {
            Some(Off::KeyInsideArtifacts)
        } else {
            None
        }
    };
    let off = if let Some(off) = off {
        off
    } else {
        if let Some(service) =
            CaptchaService::from_config(config::CaptchaConfig::from_lookup(env), mode)
        {
            // Never log file paths or contents. This guard is defense in depth, not isolation.
            tracing::warn!(target: "captcha", "Neo CAPTCHA enabled: provider key files are readable by any file: page already open in this browser context; Neo refuses script file: navigation; isolation requires key files the browser process cannot read");
            return Activation::On(service);
        }
        Off::NoReadableKey
    };
    tracing::info!(target: "captcha", reason = ?off, "Neo CAPTCHA disabled");
    Activation::Off(off)
}

/// Owns the shared solver's target driver while the runtime owns admission and input turns.
pub(super) struct NeoCaptcha(pub TargetSolver);
impl CaptchaHost for NeoCaptcha {
    fn solve(&self, page: PageId, stop: CancellationToken) -> BoxFuture<'static, SolveReply> {
        let solver = self.0.clone();
        Box::pin(async move { solve_reply(solver.solve(page.as_str(), stop).await) })
    }
}

// This mapping is intentionally independent of SolveReason: a soft flow timeout whose
// click later acknowledges is settled; any earlier failed click remains unknown after success.
fn solve_reply(outcome: Result<SolveOutcome, SolveRefusal>) -> SolveReply {
    match outcome {
        Ok(outcome) => match script::ScriptOutcome::from_outcome(&outcome) {
            Some(value) => SolveReply::Value {
                value: serde_json::json!(value),
                log: outcome.events.iter().map(script::call_log_line).collect(),
                // C1/I6: success later in the flow cannot erase failed delivery.
                input_settled: !outcome.any_click_failed,
            },
            None => SolveReply::Stopped,
        },
        Err(refusal) => {
            let (closed, message) = script::refusal(&refusal);
            SolveReply::Refused { closed, message }
        }
    }
}

#[cfg(test)]
#[expect(
    dead_code,
    reason = "The shared CDP fixture supports other runtime suites"
)]
#[allow(
    unfulfilled_lint_expectations,
    reason = "Neo's shared fixture expects CDP-client restrictions that Claw's host config permits"
)]
#[path = "../../../../../crates/neo-sdk/neo-runtime/tests/support/fake_browser.rs"]
mod fake_browser;

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "Missing fixture state or failed synchronization must stop this regression"
)]
mod preflight_tests {
    use super::*;
    use fake_browser::{FakeBrowser, Replies, Reply};
    use neo_runtime::{Delivery, RunStatus};
    use std::time::Duration;
    use tokio::time;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn captcha_cancelled_target_preflight_releases_claim_and_input() -> anyhow::Result<()> {
        // Both connections identify the same page. Only the solver's tab-list read stalls;
        // runtime teardown and follow-up input must stay observable without releasing it.
        let browser = FakeBrowser::start(Replies::default()).await;
        let solver_browser = FakeBrowser::start(Replies {
            get_tabs: Reply {
                answers: 0,
                ..Reply::default()
            },
            ..Replies::default()
        })
        .await;
        let keys = tempfile::tempdir()?;
        let key = keys.path().join("sentinel.key");
        std::fs::write(&key, "NEO-CAPTCHA-PREFLIGHT-SENTINEL")?;
        let service = CaptchaService::from_config(
            config::CaptchaConfig::from_lookup(|name| match name {
                config::CAPSOLVER_KEY_FILE_ENV => Some(key.display().to_string()),
                config::PAID_TASK_LIMIT_ENV => Some("0".to_owned()),
                config::CAPSOLVER_URL_ENV | config::TWOCAPTCHA_URL_ENV => {
                    Some("http://127.0.0.1:1".to_owned())
                }
                _ => None,
            }),
            ServerMode::Hosted,
        )
        .expect("local-only sentinel provider");
        let host = Arc::new(NeoCaptcha(TargetSolver::new(
            Arc::clone(service.solver()),
            solver_browser.cdp().await,
        )));
        let runtime = browser.runtime().await.with_captcha(host);
        let cancel = CancellationToken::new();
        let running = {
            let runtime = runtime.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                runtime
                    .run(fake_browser::run_request(
                        "return await neo.solveCaptcha(page)",
                        Duration::from_secs(30),
                        cancel,
                    ))
                    .await
            })
        };
        time::timeout(Duration::from_secs(5), async {
            while !solver_browser
                .exchanges()
                .iter()
                .any(|exchange| exchange.method() == "Browser.getTabs")
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("solver reached the held preflight read");
        cancel.cancel();
        let outcome = time::timeout(Duration::from_secs(5), running)
            .await
            .expect("cancelled run finishes")?;
        assert_eq!(outcome.status, RunStatus::Cancelled, "{outcome:?}");
        assert_eq!(outcome.calls[0].delivery, Some(Delivery::Unknown));
        time::timeout(Duration::from_secs(5), runtime.drain())
            .await
            .expect("cancelled target-list preflight must finish host ownership without a reply");
        let next = runtime
            .run(fake_browser::run_request(
                "await page.click('#go'); return true",
                Duration::from_secs(5),
                CancellationToken::new(),
            ))
            .await;
        assert_eq!(next.status, RunStatus::Ok, "{next:?}");
        assert_eq!(next.calls[0].delivery, Some(Delivery::Done));
        let exchanges = solver_browser.exchanges();
        assert_eq!(
            exchanges.len(),
            1,
            "preflight must not attach or write input"
        );
        assert_eq!(exchanges[0].method(), "Browser.getTabs");
        assert!(
            exchanges[0].reply.is_none(),
            "no stalled reply was released"
        );
        runtime.drain().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap};

    #[test]
    fn captcha_off_reads_only_its_opt_in_flag() {
        for flag in [None, Some("0"), Some("true"), Some(" 1")] {
            let read = RefCell::new(Vec::new());
            let result = activation(
                |name| {
                    read.borrow_mut().push(name.to_owned());
                    assert_eq!(name, FLAG, "off must not even read key variables");
                    flag.map(str::to_owned)
                },
                ServerMode::Hosted,
                Path::new("/unused"),
            );
            assert!(matches!(result, Activation::Off(Off::NotRequested)));
            assert_eq!(*read.borrow(), vec![FLAG]);
        }
    }

    #[test]
    fn captcha_activation_rejects_desktop_before_reading_keys() {
        assert!(matches!(
            activation(
                |name| {
                    assert_eq!(name, FLAG);
                    Some("1".into())
                },
                ServerMode::Desktop,
                Path::new("/unused")
            ),
            Activation::Off(Off::NotHosted)
        ));
    }

    #[test]
    fn captcha_activation_containment_and_readable_provider_table() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let artifacts = dir.path().join("artifacts");
        std::fs::create_dir(&artifacts)?;
        let outside = dir.path().join("key");
        std::fs::write(&outside, "NEO-CAPTCHA-SENTINEL-TEST")?;
        // This directory cannot be read as a key file; containment must still win before
        // provider loading, proving the decision never depends on reading its contents.
        let inside = artifacts.join("unreadable-key");
        std::fs::create_dir(&inside)?;
        for variable in [
            config::CAPSOLVER_KEY_FILE_ENV,
            config::TWOCAPTCHA_KEY_FILE_ENV,
        ] {
            for (path, expected) in [
                (Some(&inside), Some(Off::KeyInsideArtifacts)),
                (None, Some(Off::NoReadableKey)),
                (Some(&outside), None),
            ] {
                let mut env = HashMap::from([
                    (FLAG, "1".to_owned()),
                    (config::PAID_TASK_LIMIT_ENV, "0".to_owned()),
                    (config::CAPSOLVER_URL_ENV, "http://127.0.0.1:1".to_owned()),
                    (config::TWOCAPTCHA_URL_ENV, "http://127.0.0.1:1".to_owned()),
                ]);
                if let Some(path) = path {
                    env.insert(variable, path.display().to_string());
                }
                match (
                    activation(
                        |name| env.get(name).cloned(),
                        ServerMode::Hosted,
                        &artifacts,
                    ),
                    expected,
                ) {
                    (Activation::On(_), None) => {}
                    (Activation::Off(actual), Some(expected)) => assert_eq!(actual, expected),
                    _ => panic!("wrong activation decision"),
                }
            }
        }
        #[cfg(unix)]
        {
            let link = dir.path().join("key-link");
            std::os::unix::fs::symlink(&inside, &link)?;
            assert!(matches!(
                activation(
                    |name| match name {
                        FLAG => Some("1".into()),
                        config::CAPSOLVER_KEY_FILE_ENV => Some(link.display().to_string()),
                        _ => None,
                    },
                    ServerMode::Hosted,
                    &artifacts
                ),
                Activation::Off(Off::KeyInsideArtifacts)
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod log_tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Clone, Default)]
    struct Writer(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .map_err(|_| std::io::Error::other("log writer poisoned"))?
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn captcha_all_activation_logs_omit_sentinel_and_key_path() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let key = dir.path().join("provider-key-with-private-path");
        let artifacts = dir.path().join("artifacts");
        std::fs::create_dir(&artifacts)?;
        std::fs::write(&key, "NEO-CAPTCHA-LOG-SENTINEL")?;
        let inside = artifacts.join("provider-key");
        std::fs::write(&inside, "NEO-CAPTCHA-LOG-SENTINEL")?;
        let writer = Writer::default();
        let output = writer.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            for (requested, mode, file) in [
                (false, ServerMode::Hosted, Some(&key)),
                (true, ServerMode::Desktop, Some(&key)),
                (true, ServerMode::Hosted, None),
                (true, ServerMode::Hosted, Some(&inside)),
                (true, ServerMode::Hosted, Some(&key)),
            ] {
                let _activation = activation(
                    |name| match name {
                        FLAG if requested => Some("1".to_owned()),
                        config::CAPSOLVER_KEY_FILE_ENV => file.map(|p| p.display().to_string()),
                        config::CAPSOLVER_URL_ENV | config::TWOCAPTCHA_URL_ENV => {
                            Some("http://127.0.0.1:1".to_owned())
                        }
                        config::PAID_TASK_LIMIT_ENV => Some("0".to_owned()),
                        _ => None,
                    },
                    mode,
                    &artifacts,
                );
            }
        });
        let logs = String::from_utf8(
            output
                .0
                .lock()
                .map_err(|_| anyhow::anyhow!("log writer poisoned"))?
                .clone(),
        )?;
        assert!(!logs.contains("NEO-CAPTCHA-LOG-SENTINEL"));
        assert!(!logs.contains(&dir.path().display().to_string()));
        for reason in ["NotHosted", "NoReadableKey", "KeyInsideArtifacts"] {
            assert!(logs.contains(reason), "missing {reason}: {logs}");
        }
        assert!(logs.contains("browser process cannot read"));
        Ok(())
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "These fixture assertions report malformed test setup"
)]
mod reply_tests {
    use super::*;
    use neo_captcha::types::{SolveOutcome, SolveReason, SolveStatus};
    use rmcp::ServerHandler;

    fn outcome() -> SolveOutcome {
        serde_json::from_value(serde_json::json!({"status":"failed","type":null,"method":"none","reason":"browser_error","requestId":"fixture","page":1,"durationMs":12,"attempts":{"checkbox":0,"tileRounds":0,"token":0},"paidTasks":0,"costUsd":0.0,"message":"fixture","events":[]})).expect("legacy outcome fixture")
    }

    #[test]
    fn captcha_host_delivery_uses_sticky_input_proof_instead_of_coarse_reason() {
        for (failed, status, reason) in [
            (false, SolveStatus::Failed, SolveReason::BrowserError),
            (true, SolveStatus::AlreadyClear, SolveReason::NoCaptcha),
        ] {
            let mut outcome = outcome();
            outcome.any_click_failed = failed;
            outcome.status = status;
            outcome.reason = reason;
            match solve_reply(Ok(outcome)) {
                SolveReply::Value { input_settled, .. } => assert_eq!(input_settled, !failed),
                _ => panic!("finished outcome is a value"),
            }
        }
        let mut outcome = outcome();
        outcome.reason = SolveReason::CallerCancelled;
        assert!(matches!(solve_reply(Ok(outcome)), SolveReply::Stopped));
    }

    #[test]
    fn captcha_instructions_are_exposed_only_when_the_host_installs_its_solver()
    -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let service = neo_sdk_server::NeoSdkServer::new(
            neo_sdk_server::Config::new(1, dir.path().to_path_buf()),
            CancellationToken::new(),
        );
        assert!(
            !service
                .get_info()
                .instructions
                .expect("instructions")
                .contains(script::RUN_INSTRUCTION)
        );
        assert!(
            !service
                .get_tool("run")
                .expect("tool")
                .description
                .expect("description")
                .contains(script::RUN_INSTRUCTION)
        );
        let key = dir.path().join("sentinel.key");
        std::fs::write(&key, "NEO-CAPTCHA-DESCRIPTION-SENTINEL")?;
        let captcha = CaptchaService::from_config(
            config::CaptchaConfig::from_lookup(|name| match name {
                config::CAPSOLVER_KEY_FILE_ENV => Some(key.display().to_string()),
                config::PAID_TASK_LIMIT_ENV => Some("0".to_owned()),
                config::CAPSOLVER_URL_ENV | config::TWOCAPTCHA_URL_ENV => {
                    Some("http://127.0.0.1:1".to_owned())
                }
                _ => None,
            }),
            ServerMode::Hosted,
        )
        .expect("configured hosted service");
        let service = Activation::On(captcha).install(service);
        assert!(
            service
                .get_info()
                .instructions
                .expect("instructions")
                .contains(script::RUN_INSTRUCTION)
        );
        assert!(
            service
                .get_tool("run")
                .expect("tool")
                .description
                .expect("description")
                .contains(script::RUN_INSTRUCTION)
        );
        Ok(())
    }
}
