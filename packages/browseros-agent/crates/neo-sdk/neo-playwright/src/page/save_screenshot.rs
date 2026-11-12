//! Screenshot persistence is a client step after capture, implemented on Neo's disk for
//! QuickJS. Its targetless write declaration records filesystem effects without blocking
//! screenshot reads on a tab held by takeover.

use std::io;
use std::path::{self, Path};

use crate::wire::{Nothing, options};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use tokio::fs;
use ts_rs::TS;

use crate::op_scope::{OpScope, Sent};
use crate::op_spec::{OpSpec, OpTarget};
use crate::{ErrorCode, PlaywrightError, file_save};

/// The filesystem step has no tab target; capture already passed its own page policies.
pub(crate) const SPEC: OpSpec = OpSpec::write(
    "page.saveScreenshot",
    OpTarget::None,
    "save captured screenshot bytes on Neo's disk (the screenshot client's path step)",
);

/// The bytes already returned to the client, with its chosen destination.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct SaveScreenshotInput {
    path: String,
    base64: String,
}

options!(SaveScreenshotInput);

/// Save the captured bytes, making parent directories as Playwright's client does.
pub(crate) async fn save(
    scope: &OpScope<'_>,
    input: SaveScreenshotInput,
) -> Result<Nothing, PlaywrightError> {
    let bytes = STANDARD.decode(&input.base64).map_err(|error| {
        scope.fail(
            ErrorCode::InvalidInput,
            &format!("invalid screenshot bytes: {error}"),
        )
    })?;
    let target = path::absolute(&input.path).map_err(|error| {
        scope.fail(
            ErrorCode::InvalidInput,
            &format!("cannot save screenshot to \"{}\": {error}", input.path),
        )
    })?;
    let sent = scope.send_effect()?;
    // Tokio's filesystem work can finish after its future is cancelled. Mark the effect
    // before mkdir/write so a deadline reports unknown delivery rather than claiming no write.
    scope
        .within(write_to(&sent, &target, &bytes))
        .await
        .map_err(|_expired| scope.timed_out(ErrorCode::WaitTimeout))?
        .map_err(|error| {
            scope.fail(
                file_save::error_code(error.kind()),
                &format!(
                    "cannot save screenshot to \"{}\": {error}",
                    target.display()
                ),
            )
        })?;
    Ok(())
}

async fn write_to(_sent: &Sent<'_>, target: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(target, bytes).await
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::error::Error;
    use std::path::Path;

    use neo_trace::{Effect, ErrorOwner};
    use tokio::fs;

    use super::{SPEC, SaveScreenshotInput, save};
    use crate::op_scope::OpScope;
    use crate::op_spec::OpTarget;
    use crate::target::CallHeader;
    use crate::test_env::{EnvCall, TestEnv};
    use crate::{ErrorCode, Op, Refusal};

    fn input(path: &Path) -> SaveScreenshotInput {
        SaveScreenshotInput {
            path: path.to_string_lossy().into_owned(),
            base64: "iVBORw0KGgo=".to_owned(),
        }
    }

    #[tokio::test]
    async fn saves_exact_bytes_creates_parents_and_overwrites_after_marking_the_write()
    -> Result<(), Box<dyn Error>> {
        let own = tempfile::tempdir()?;
        let target = own.path().join("nested/screenshot.png");
        let env = TestEnv::new();
        let op = Op::from_name("page.saveScreenshot").ok_or("save op exists")?;
        let scope = OpScope::new(&env, op, CallHeader::default());
        for _ in 0..2 {
            save(&scope, input(&target)).await?;
            assert_eq!(fs::read(&target).await?, b"\x89PNG\r\n\x1a\n");
            fs::write(&target, b"previous file contents").await?;
        }
        assert_eq!(
            env.calls(),
            [
                EnvCall::Recheck,
                EnvCall::MarkSent,
                EnvCall::Recheck,
                EnvCall::MarkSent
            ]
        );
        assert_eq!(SPEC.effect, Effect::Write);
        assert_eq!(SPEC.target, OpTarget::None);
        assert_eq!(
            Op::from_name("page.screenshot")
                .ok_or("capture op exists")?
                .spec()
                .effect,
            Effect::Read
        );
        Ok(())
    }

    #[tokio::test]
    async fn relative_paths_use_neos_current_directory_without_changing_it()
    -> Result<(), Box<dyn Error>> {
        let cwd = env::current_dir()?;
        let own = tempfile::Builder::new()
            .prefix(".screenshot-test-")
            .tempdir_in(&cwd)?;
        let target = own.path().join("nested/relative.png");
        let env = TestEnv::new();
        let scope = OpScope::new(
            &env,
            Op::from_name("page.saveScreenshot").ok_or("save op exists")?,
            CallHeader::default(),
        );
        save(&scope, input(target.strip_prefix(&cwd)?)).await?;
        assert_eq!(fs::read(target).await?, b"\x89PNG\r\n\x1a\n");
        Ok(())
    }

    #[tokio::test]
    async fn invalid_bytes_and_a_refused_write_leave_no_directories_or_files()
    -> Result<(), Box<dyn Error>> {
        let own = tempfile::tempdir()?;
        let target = own.path().join("absent/screenshot.png");
        let op = Op::from_name("page.saveScreenshot").ok_or("save op exists")?;
        let env = TestEnv::new();
        let scope = OpScope::new(&env, op, CallHeader::default());
        let mut invalid = input(&target);
        invalid.base64 = "not base64!".to_owned();
        let error = save(&scope, invalid)
            .await
            .err()
            .ok_or("bad bytes must fail")?;
        assert_eq!(error.error_code(), Some(ErrorCode::InvalidInput));
        assert!(env.calls().is_empty());
        let env = TestEnv::refusing(Refusal {
            owner: ErrorOwner::Host,
            code: "write_refused".to_owned(),
            message: "no filesystem write".to_owned(),
            host_code: None,
        });
        let scope = OpScope::new(&env, op, CallHeader::default());
        assert!(save(&scope, input(&target)).await.is_err());
        assert_eq!(env.calls(), [EnvCall::Recheck]);
        assert!(!own.path().join("absent").exists());
        Ok(())
    }

    #[tokio::test]
    async fn a_directory_destination_reports_an_error_instead_of_success()
    -> Result<(), Box<dyn Error>> {
        let own = tempfile::tempdir()?;
        let env = TestEnv::new();
        let scope = OpScope::new(
            &env,
            Op::from_name("page.saveScreenshot").ok_or("save op exists")?,
            CallHeader::default(),
        );
        let error = save(&scope, input(own.path()))
            .await
            .err()
            .ok_or("cannot overwrite a directory")?;
        assert_eq!(error.error_code(), Some(ErrorCode::InvalidInput));
        assert_eq!(error.owner(), ErrorOwner::Script);
        assert!(error.message().contains("cannot save screenshot"));
        Ok(())
    }
}
