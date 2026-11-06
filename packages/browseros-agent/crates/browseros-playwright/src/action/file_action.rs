//! `setInputFiles(paths)`: upstream's `_setInputFiles` (server/dom.ts at v1.64.1), with the
//! files set by core's file transfers, whose rules every file Neo hands a page follows. The
//! crate's only code that hands a page files from Neo's disk, and so the gateway of
//! `Capability::HostFiles`.
//!
//! ```text
//!  an op whose spec does not declare HostFiles is refused, with nothing admitted
//!  admit the page, lease its focus
//!  outer loop (locator: re-resolve │ handle: once); the element only has to be attached
//!    ① read { fileInput { multiple } }  ─ the input (or the label's control), pinned, after
//!                                          upstream's checks and messages
//!    the pin's backend node id          ─ DOM.describeNode on the world's session
//!    send_effect()                      ─ Sent
//!    core FileTransfers::upload_node    ─ admission, every path a readable local file, a file
//!                                          input, DOM.setFileInputFiles (empty: cleared)
//! ```
//!
//! Only host paths are accepted; the prelude refuses Playwright's in-memory payloads.

use browseros_core::CoreError;
use serde::Deserialize;

use crate::action::focus_lease::lease;
use crate::action::subject::Subject;
use crate::backoff::Backoff;
use crate::op_scope::{OpScope, Sent};
use crate::op_spec::Capability;
use crate::page::OpPage;
use crate::probe::{ProbeFailure, ProbeReply, ProbeRequest, Read};
use crate::read::backend_node;
use crate::resolve::{self, Resolved};
use crate::target::{Strictness, Target};
use crate::world::{BackendNodeId, Pin};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// `locator.setInputFiles(paths)`: host paths, which core checks.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetInputFilesInput {
    target: Target,
    strict: Strictness,
    files: Vec<String>,
}

/// A `fileInput` read's value (protocol.ts `FileInput`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileInput {
    pin: Pin,
}

/// Sets `input`'s files on its target's file input, as upstream's `setInputFiles`. It admits
/// its own page so that the op's declaration is checked first: a host serves an op unheld
/// unless its spec declares `HostFiles`, and such an op must not reach a page here.
pub(crate) async fn set_input_files(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: SetInputFilesInput,
) -> Result<(), PlaywrightError> {
    scope.require(Capability::HostFiles)?;
    let SetInputFilesInput {
        target,
        strict,
        files,
    } = input;
    let page = OpPage::admit_handle(pw, scope, target.page()).await?;
    let _lease = lease(scope, &page).await?;
    let subject = Subject::new(&page, scope, &target, strict);
    subject.log_waiting();
    let backoff = Backoff::outer(scope.timeout());
    // Nothing is waited for once the input is found, so a timeout means it never was.
    let timed_out = || scope.timed_out(ErrorCode::NoMatch);
    for pass in 0.. {
        if scope.sleep(backoff.wait(pass)).await.is_err() {
            return Err(timed_out());
        }
        let Ok(located) = scope.within(subject.locate(pass)).await else {
            return Err(timed_out());
        };
        let Some(located) = located? else {
            continue;
        };
        let found = scope.within(input_node(scope, &subject, &located, strict, &files));
        let Ok(found) = found.await else {
            return Err(timed_out());
        };
        let Some(node) = found? else {
            continue;
        };
        let sent = scope.send_effect()?;
        return upload(&sent, scope, &page, &located, node, &files).await;
    }
    Err(timed_out())
}

/// ① and the pin's backend node: the file input's node on the world's session, or `None`
/// when nothing matches yet or the element went away.
async fn input_node(
    scope: &OpScope<'_>,
    subject: &Subject<'_>,
    located: &Resolved,
    strictness: Strictness,
    files: &[String],
) -> Result<Option<BackendNodeId>, PlaywrightError> {
    let request = ProbeRequest::Read {
        target: subject.first_target(),
        strictness,
        frame_visible: true,
        read: Read::FileInput {
            multiple: files.len() > 1,
        },
    };
    let pin = match subject.call(located, &request).await? {
        Ok(ProbeReply::Read { value, .. }) => {
            let input: FileInput = serde_json::from_value(value.textless()).map_err(|error| {
                scope.fail(
                    ErrorCode::Internal,
                    &format!("the probe answered a file input this build cannot read: {error}"),
                )
            })?;
            input.pin
        }
        Ok(ProbeReply::None { stale_ref: None }) => return Ok(None),
        Ok(ProbeReply::Detached) => return subject.detached().map(|()| None),
        Ok(other) => return Err(subject.refused(located, other).await),
        Err(ProbeFailure::Gone) => return subject.lost(located).map(|()| None),
        Err(failure) => return Err(resolve::failed(scope, failure)),
    };
    match backend_node(scope, located.world(), pin).await? {
        Some(node) => Ok(Some(node)),
        None => subject.detached().map(|()| None),
    }
}

/// Core sets the files. Its own refusals (a path that is not a readable file, an input that is
/// not `type=file`, several files for a single-file input) are about the script's arguments;
/// it makes them before it changes anything.
async fn upload(
    _sent: &Sent<'_>,
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    located: &Resolved,
    node: BackendNodeId,
    files: &[String],
) -> Result<(), PlaywrightError> {
    let transfers = scope.view().files();
    let uploaded =
        transfers.upload_node(page.id().clone(), located.world().session(), node.0, files);
    match scope.within(uploaded).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(CoreError::Message(message))) => Err(scope.fail(ErrorCode::InvalidInput, &message)),
        Ok(Err(error)) => Err(scope.core_error(&error)),
        Err(_expired) => Err(scope.timed_out(ErrorCode::NotActionable)),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use browseros_trace::ErrorOwner;
    use serde_json::json;

    use super::{SetInputFilesInput, set_input_files};
    use crate::op_scope::OpScope;
    use crate::target::CallHeader;
    use crate::test_env::{TestEnv, offline_session};
    use crate::test_support::RecordingHooks;
    use crate::{ErrorCode, Op, Playwright};

    /// A host serves an op unheld unless its spec declares `HostFiles`, so the file action
    /// refuses any other op before it admits a page, while `locator.setInputFiles` goes on to
    /// admit its page, which fails here for want of a browser. Paused: core waits out its own
    /// attach timeout before it gives up on the page.
    #[tokio::test(start_paused = true)]
    async fn tests_that_only_an_op_declaring_host_files_reaches_the_page()
    -> Result<(), Box<dyn Error>> {
        let playwright = Playwright::new(offline_session(), Arc::new(RecordingHooks::default()));
        let env = TestEnv::new();
        let input = || {
            serde_json::from_value::<SetInputFilesInput>(json!({
                "target": { "$neo": "locator", "page": { "$neo": "page", "id": 7, "tab": 1007 },
                            "display": "getByRef('f7e3')",
                            "chunks": [{ "parts": [{ "name": "aria-ref", "body": "f7e3",
                                                     "source": "f7e3" }] }] },
                "strict": "strict",
                "files": ["/tmp/upload.txt"],
            }))
        };
        let click = Op::from_name("locator.click").ok_or("locator.click is an op")?;
        let undeclared = OpScope::new(&env, click, CallHeader::default());
        let refused = set_input_files(&playwright, &undeclared, input()?)
            .await
            .err()
            .ok_or("an op that does not declare HostFiles must not reach the page")?;
        assert_eq!(
            (refused.owner(), refused.error_code()),
            (ErrorOwner::Neo, Some(ErrorCode::Internal)),
            "{refused}"
        );
        let upload = Op::from_name("locator.setInputFiles").ok_or("setInputFiles is an op")?;
        let declared = OpScope::new(&env, upload, CallHeader::default());
        let admitting = set_input_files(&playwright, &declared, input()?)
            .await
            .err()
            .ok_or("no browser, no page")?;
        assert_eq!(
            admitting.error_code(),
            Some(ErrorCode::BrowserDisconnected),
            "the declared op goes on to admit its page: {admitting}"
        );
        assert!(
            env.calls().is_empty(),
            "neither op noted, rechecked or marked anything sent"
        );
        Ok(())
    }
}
