//! `setInputFiles(paths)`: upstream's `_setInputFiles` (server/dom.ts at v1.64.1), with the
//! files set by core's file transfers, whose rules every file Neo hands a page follows.
//!
//! ```text
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

use crate::action::subject::Subject;
use crate::backoff::Backoff;
use crate::op_scope::{OpScope, Sent};
use crate::page::OpPage;
use crate::probe::{ProbeFailure, ProbeReply, ProbeRequest, Read};
use crate::read::backend_node;
use crate::resolve::{self, Resolved};
use crate::target::{Strictness, Target};
use crate::world::{BackendNodeId, Pin};
use crate::{ErrorCode, PlaywrightError};

/// A `fileInput` read's value (protocol.ts `FileInput`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileInput {
    pin: Pin,
}

/// Sets `files` on `target`'s file input, as upstream's `setInputFiles`.
pub(crate) async fn set_input_files(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    target: &Target,
    strictness: Strictness,
    files: &[String],
) -> Result<(), PlaywrightError> {
    let subject = Subject::new(page, scope, target, strictness);
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
        let found = scope.within(input_node(scope, &subject, &located, strictness, files));
        let Ok(found) = found.await else {
            return Err(timed_out());
        };
        let Some(node) = found? else {
            continue;
        };
        let sent = scope.send_effect()?;
        return upload(&sent, scope, page, &located, node, files).await;
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
