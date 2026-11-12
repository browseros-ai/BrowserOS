//! `setInputFiles` on every form (`locator`, `page`, `frame`, `elementHandle`): upstream's
//! client and server rules in one module (`convertInputFiles` in client/elementHandle.ts and
//! `_setInputFiles` in server/dom.ts, at v1.64.1). It is the crate's only code that hands a page
//! files from Neo's disk, and so the gateway of `Capability::HostFiles`.
//!
//! ```text
//!  require(HostFiles)   an op that does not declare it: neo / internal, nothing admitted
//!  prelude   files.ts: paths mixed with payloads, 50 MiB, payload fields, MIME, base64 text
//!  classify  each path in order: resolve it as Node's `path.resolve` does and stat the
//!            path as given. Stat errors use Node's text with no API prefix.
//!            ─▶ Paths(absolute) │ Directory(absolute) │ Payloads
//!  admit the page, lease its focus
//!  outer loop (locator: re-resolve │ handle: once)
//!    ① read { fileInput }   upstream's four checks, `<api>: Error: …`, never retried
//!    paths    access(F_OK) each ─ `<api>: ENOENT: … access '<abs>'`  ┐ a locator retries,
//!             not type=file    ─ Chromium's Protocol error          ┘ a handle fails
//!             the input's remote object, in the op's object group
//!    directory: armInput on the element named, a isolated-world listener the page cannot see
//!    send_effect()                                                            Sent
//!    paths, directory   DOM.setFileInputFiles { objectId, files }: Chromium reads the files,
//!                       and the page sees trusted `input` and `change`
//!    directory          inputArrived, polled on Rust's clock until `input` or the timeout
//!    payloads, []       edit { setFiles }: upstream's injected setInputFiles, isolated world
//! ```
//!
//! No relative path reaches CDP: `DOM.setFileInputFiles` crashes the target on one, and turns
//! a missing path into an empty file (spike S3), so every path is resolved to an absolute one
//! and stat'ed here first. Core's `FileTransfers::upload_node` is not used: its rules for those
//! cases, and for `[]`, are MCP's, not Playwright's.

use std::fmt::Write as _;
use std::time::Duration;
use std::{env, io, slice};

use browseros_cdp::CdpError;
use browseros_core::CoreError;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::fs;
use ts_rs::TS;

use crate::action::focus_lease;
use crate::action::subject::Subject;
use crate::backoff::Backoff;
use crate::locator_handler::Prechecks;
use crate::op_scope::{OpScope, Sent};
use crate::op_spec::Capability;
use crate::page::OpPage;
use crate::probe::{self, Edit, FilePayload, ProbeFailure, ProbeReply, ProbeRequest, Read};
use crate::resolve::{self, Resolved};
use crate::target::{Strictness, Target};
use crate::wire::options;
use crate::world::Pin;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// `locator.setInputFiles` and its page, frame and handle forms, as the prelude sends them.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetInputFilesInput {
    target: Target,
    strict: Strictness,
    files: FilesInput,
}
options!(SetInputFilesInput);

/// The files after upstream's client checks that need no disk (the prelude's files.ts).
#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum FilesInput {
    /// The strings as the script wrote them, in its order.
    Paths(Vec<String>),
    /// In-memory files, `[]` among them, ready for the page.
    Payloads(#[ts(type = "Array<import('../../../probe/protocol').FilePayload>")] Vec<FilePayload>),
}

/// Drop paths carry MIME types from Playwright's vendored table in the prelude.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum DropFiles {
    Paths(Vec<DropPath>),
    Payloads(#[ts(type = "Array<import('../../../probe/protocol').FilePayload>")] Vec<FilePayload>),
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DropPath {
    path: String,
    mime_type: String,
}

/// Reads drop paths on Neo's host after the same client classification as uploads.
/// Directories are refused before page work; files retain their MIME type and mtime.
pub(crate) async fn drop_payloads(files: DropFiles) -> Result<Vec<FilePayload>, PlaywrightError> {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use std::path::Path;
    use std::time::UNIX_EPOCH;

    let paths = match files {
        DropFiles::Payloads(payloads) => return Ok(payloads),
        DropFiles::Paths(paths) => paths,
    };
    let (paths, mime_types): (Vec<_>, Vec<_>) = paths
        .into_iter()
        .map(|path| (path.path, path.mime_type))
        .unzip();
    let files = FilesInput::Paths(paths);
    match InputFiles::classify(files).await? {
        InputFiles::Payloads(payloads) => Ok(payloads),
        InputFiles::Directory(_) => Err(client_error(
            "Dropping a directory is not supported — pass individual files.".to_owned(),
        )),
        InputFiles::Paths(paths) => {
            let mut payloads = Vec::with_capacity(paths.len());
            for (HostPath(path), mime_type) in paths.into_iter().zip(mime_types) {
                let bytes = fs::read(&path)
                    .await
                    .map_err(|error| client_error(node_error(&error, "open", &path)))?;
                let metadata = fs::metadata(&path)
                    .await
                    .map_err(|error| client_error(node_error(&error, "stat", &path)))?;
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok());
                payloads.push(FilePayload {
                    name: Path::new(&path)
                        .file_name()
                        .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
                    mime_type,
                    buffer: STANDARD.encode(bytes),
                    last_modified_ms: modified.map(|time| time.as_secs_f64() * 1000.0),
                });
            }
            Ok(payloads)
        }
    }
}

/// What the input gets, classified on Neo's disk before any page work.
enum InputFiles {
    /// Set natively, as on upstream's co-located path, so the page sees trusted events.
    Paths(Vec<HostPath>),
    /// One directory, for a `webkitdirectory` input.
    Directory(HostPath),
    /// Built in the isolated world by upstream's injected script.
    Payloads(Vec<FilePayload>),
}

/// A path that upstream's client stat found, resolved as Node's `path.resolve` resolves it
/// against Neo's working directory (Neo's process is the Playwright client). Only
/// [`InputFiles::classify`] builds one, so a relative path cannot reach `DOM.setFileInputFiles`.
struct HostPath(String);

/// A `fileInput` read's value (protocol.ts `FileInput`).
#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FileInput {
    /// The `<input>` the files go to: the element, or the control its label names.
    pin: Pin,
    /// The element the locator or handle named, which a directory's `input` wait listens on.
    target: Pin,
    /// `type=file`, without which Chromium refuses paths.
    takes_paths: bool,
}

/// A `setFiles` value other than null (protocol.ts `FilesSet`): what upstream's injected
/// `setInputFiles` threw, as CDP describes it.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct Threw {
    threw: String,
}

/// The probe's setFiles value: null on success, or the injected script's failure. Naming
/// this transparent wrapper lets the decoder and generated contract share its nullability.
#[derive(Deserialize, TS)]
#[serde(transparent)]
pub(crate) struct FilesSet(Option<Threw>);

/// One pass's checked files, with what their effect needs.
enum Ready<'f> {
    /// Paths for `DOM.setFileInputFiles` on the input's remote object. `waits_on` is a
    /// directory's: the element whose armed `input` listener ends the op.
    Native {
        paths: &'f [HostPath],
        object: String,
        waits_on: Option<Pin>,
    },
    /// Payloads for upstream's injected script, on the input's pin.
    Payloads(&'f [FilePayload]),
}

/// How a pass ended once it was marked sent.
enum Set {
    Done,
    /// A locator goes round again, as upstream's loop does after a CDP error.
    Again,
}

/// How often a directory upload asks whether its `input` came.
const INPUT_POLL: Duration = Duration::from_millis(50);

/// What Chromium's `DOM.setFileInputFiles` answers for paths into an input that is not
/// `type=file`, as upstream reports it. Neo checks first, and so never sends those paths.
const NOT_FILE_INPUT: &str =
    "Protocol error (DOM.setFileInputFiles): Node is not a file input element";

/// Sets `input`'s files on its target's input, as upstream's `setInputFiles`. Every step before
/// `send_effect` can only refuse, so a bad path or a wrong input is `not_sent`.
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
    let files = InputFiles::classify(files).await?;
    let page = OpPage::admit_handle(pw, scope, target.page()).await?;
    let _lease = focus_lease::lease(scope, &page).await?;
    let subject = Subject::new(&page, scope, &target, strict);
    subject.log_waiting();
    let backoff = Backoff::outer(scope.timeout());
    // A locator that matched an input and still ran out of time met one that never took the
    // files: not a file input, or a file that went away after the client's stat.
    let mut matched = false;
    for pass in 0.. {
        if scope.sleep(backoff.wait(pass)).await.is_err() {
            break;
        }
        let Ok(located) = scope
            .within(subject.locate(pass, Prechecks::LocatorPasses))
            .await
        else {
            break;
        };
        let Some(located) = located? else {
            continue;
        };
        let read = file_input(scope, &subject, &located, strict, &files);
        let Ok(input) = scope.within(read).await else {
            break;
        };
        let Some(input) = input? else {
            continue;
        };
        matched = true;
        let ready = files.ready(scope, &subject, &located, &input);
        let Ok(ready) = scope.within(ready).await else {
            break;
        };
        let Some(ready) = ready? else {
            continue;
        };
        let sent = scope.send_effect()?;
        match ready
            .set(&sent, scope, &subject, &located, input.pin)
            .await?
        {
            Set::Done => return Ok(()),
            Set::Again => {}
        }
    }
    Err(scope.timed_out(if matched {
        ErrorCode::NotActionable
    } else {
        ErrorCode::NoMatch
    }))
}

impl InputFiles {
    /// Upstream's client `resolvePathsAndDirectoryForInputFiles` on Neo's disk: each path
    /// stat'ed as given, in order (relative to Neo's working directory, as the OS resolves
    /// it); a second directory fails at once; files and a directory fail after the loop. Its
    /// errors are the client's, thrown before any channel call: unprefixed, nothing admitted.
    async fn classify(files: FilesInput) -> Result<Self, PlaywrightError> {
        let paths = match files {
            FilesInput::Payloads(payloads) => return Ok(Self::Payloads(payloads)),
            FilesInput::Paths(paths) => paths,
        };
        let mut cwd = None;
        let mut found = Vec::new();
        let mut directory = None;
        for path in paths {
            // Node's `path.resolve` reads the working directory only for a relative path.
            let resolved = if path.starts_with('/') {
                resolve_like_node("/", &path)
            } else {
                let cwd = match &cwd {
                    Some(cwd) => cwd,
                    None => cwd.insert(working_directory()?),
                };
                resolve_like_node(cwd, &path)
            };
            let is_dir = client_stat(&path).await?;
            if !is_dir {
                found.push(HostPath(resolved));
            } else if directory.is_some() {
                return Err(client_error(
                    "Multiple directories are not supported".to_owned(),
                ));
            } else {
                directory = Some(HostPath(resolved));
            }
        }
        match directory {
            Some(_) if !found.is_empty() => Err(client_error(
                "File paths must be all files or a single directory".to_owned(),
            )),
            Some(directory) => Ok(Self::Directory(directory)),
            None => Ok(Self::Paths(found)),
        }
    }

    /// The read upstream's checks run in: whether several files go in (a directory counts as
    /// one), and whether the input must take a directory.
    fn read(&self) -> Read {
        let (multiple, directory) = match self {
            Self::Paths(paths) => (paths.len() > 1, false),
            Self::Directory(_) => (false, true),
            Self::Payloads(payloads) => (payloads.len() > 1, false),
        };
        Read::FileInput {
            multiple,
            directory,
        }
    }

    /// Upstream's server checks after the element's own, in its order, and what the effect
    /// needs, for one pass. `None` sends nothing this pass: a locator goes round again.
    ///
    /// Recheck each path Chromium will open: Node's resolution folds `..` without following
    /// symlinks, and a file can disappear while the locator waits. Then check `type=file`
    /// before sending to Chromium. A locator retries either error, while a handle fails
    /// immediately. A directory's `input` listener is armed in the isolated world, where the page
    /// cannot see it.
    async fn ready(
        &self,
        scope: &OpScope<'_>,
        subject: &Subject<'_>,
        located: &Resolved,
        input: &FileInput,
    ) -> Result<Option<Ready<'_>>, PlaywrightError> {
        let (paths, directory) = match self {
            Self::Payloads(payloads) => return Ok(Some(Ready::Payloads(payloads))),
            Self::Paths(paths) => (paths.as_slice(), false),
            Self::Directory(directory) => (slice::from_ref(directory), true),
        };
        if let Some(gone) = recheck(paths).await {
            subject.retry_unless_handle(|| scope.fail(ErrorCode::InvalidInput, &gone))?;
            return Ok(None);
        }
        if !input.takes_paths {
            subject.retry_unless_handle(|| scope.fail(ErrorCode::InvalidInput, NOT_FILE_INPUT))?;
            return Ok(None);
        }
        let object = match probe::pinned_object(scope, located.world(), input.pin).await {
            Ok(Some(object)) => object,
            Ok(None) => return subject.detached().map(|()| None),
            Err(ProbeFailure::Gone(_)) => return subject.lost(located).map(|()| None),
            Err(failure) => return Err(resolve::failed(scope, failure)),
        };
        if directory {
            let arm = ProbeRequest::ArmInput { pin: input.target };
            match subject.call(located, &arm).await {
                Ok(ProbeReply::Value { .. }) => {}
                Ok(ProbeReply::Detached) => return subject.detached().map(|()| None),
                Ok(other) => return Err(subject.refused(other)),
                Err(ProbeFailure::Gone(_)) => return subject.lost(located).map(|()| None),
                Err(failure) => return Err(resolve::failed(scope, failure)),
            }
        }
        Ok(Some(Ready::Native {
            paths,
            object,
            waits_on: directory.then_some(input.target),
        }))
    }
}

/// Mirrors upstream's `access(F_OK)` check in path order. Returns the first access error.
async fn recheck(paths: &[HostPath]) -> Option<String> {
    for path in paths {
        if let Err(error) = fs::metadata(&path.0).await {
            return Some(node_error(&error, "access", &path.0));
        }
    }
    None
}

impl Ready<'_> {
    /// The effect upstream's `_setInputFiles` ends with, on the input pinned as `input`.
    async fn set(
        self,
        _sent: &Sent<'_>,
        scope: &OpScope<'_>,
        subject: &Subject<'_>,
        located: &Resolved,
        input: Pin,
    ) -> Result<Set, PlaywrightError> {
        match self {
            Self::Payloads(payloads) => {
                set_payloads(scope, subject, located, input, payloads).await
            }
            Self::Native {
                paths,
                object,
                waits_on,
            } => {
                let files: Vec<&str> = paths.iter().map(|path| path.0.as_str()).collect();
                let params = json!({ "objectId": object, "files": files });
                let setting = located
                    .world()
                    .session()
                    .send::<_, Value>("DOM.setFileInputFiles", params);
                let Ok(set) = scope.within(setting).await else {
                    return Err(scope.timed_out(ErrorCode::NotActionable));
                };
                if let Err(error) = set {
                    return match ProbeFailure::from_core(error) {
                        ProbeFailure::Gone(_) => subject.lost(located).map(|()| Set::Again),
                        ProbeFailure::Cdp(CoreError::Cdp(CdpError::Protocol {
                            message, ..
                        })) => {
                            let message =
                                format!("Protocol error (DOM.setFileInputFiles): {message}");
                            subject.retry_unless_handle(|| {
                                scope.fail(ErrorCode::ElementDetached, &message)
                            })?;
                            Ok(Set::Again)
                        }
                        failure => Err(resolve::failed(scope, failure)),
                    };
                }
                match waits_on {
                    Some(target) => wait_for_input(scope, subject, located, target).await,
                    None => Ok(Set::Done),
                }
            }
        }
    }
}

/// Upstream's injected `setInputFiles(node, payloads)` from the isolated world. What it threw
/// (a payload's text that `atob` refuses) is upstream's `<api>: DOMException`.
async fn set_payloads(
    scope: &OpScope<'_>,
    subject: &Subject<'_>,
    located: &Resolved,
    input: Pin,
    payloads: &[FilePayload],
) -> Result<Set, PlaywrightError> {
    let request = ProbeRequest::Edit {
        pin: input,
        edit: Edit::SetFiles {
            payloads: payloads.to_vec(),
        },
    };
    let Ok(edited) = scope.within(subject.call(located, &request)).await else {
        return Err(scope.timed_out(ErrorCode::NotActionable));
    };
    match edited {
        Ok(ProbeReply::Value { value }) => {
            match serde_json::from_value::<FilesSet>(value.into_value()) {
                Ok(FilesSet(None)) => Ok(Set::Done),
                Ok(FilesSet(Some(Threw { threw }))) => {
                    Err(scope.fail(ErrorCode::InvalidInput, &threw))
                }
                Err(error) => Err(scope.fail(
                    ErrorCode::Internal,
                    &format!(
                        "the probe answered setFiles with a value this build cannot read: {error}"
                    ),
                )),
            }
        }
        Ok(ProbeReply::Detached) => subject.detached().map(|()| Set::Again),
        Ok(other) => Err(subject.refused(other)),
        Err(ProbeFailure::Gone(_)) => subject.lost(located).map(|()| Set::Again),
        Err(failure) => Err(resolve::failed(scope, failure)),
    }
}

/// Upstream's wait for a directory's `input`, bounded by the op's timeout, polled on Rust's
/// clock. A document that went away ends the wait as done: upstream's wait swallows the
/// rejection it gets then (`.catch(() => {})`).
async fn wait_for_input(
    scope: &OpScope<'_>,
    subject: &Subject<'_>,
    located: &Resolved,
    target: Pin,
) -> Result<Set, PlaywrightError> {
    let request = ProbeRequest::InputArrived { pin: target };
    loop {
        let Ok(arrived) = scope.within(subject.call(located, &request)).await else {
            return Err(scope.timed_out(ErrorCode::WaitTimeout));
        };
        match arrived {
            Ok(ProbeReply::Value { value }) => {
                if value.into_value() == Value::Bool(true) {
                    return Ok(Set::Done);
                }
            }
            // An evicted pin never comes back, so the op waits out its timeout, as upstream's
            // listener on an element that never sees `input` does.
            Ok(ProbeReply::Detached) => {}
            Ok(other) => return Err(subject.refused(other)),
            Err(ProbeFailure::Gone(_)) => return Ok(Set::Done),
            Err(failure) => return Err(resolve::failed(scope, failure)),
        }
        if scope.sleep(INPUT_POLL).await.is_err() {
            return Err(scope.timed_out(ErrorCode::WaitTimeout));
        }
    }
}

/// ① for one pass: the input after upstream's checks, or `None` while nothing matches yet or
/// the element went away (a handle fails then instead).
async fn file_input(
    scope: &OpScope<'_>,
    subject: &Subject<'_>,
    located: &Resolved,
    strictness: Strictness,
    files: &InputFiles,
) -> Result<Option<FileInput>, PlaywrightError> {
    let request = ProbeRequest::Read {
        target: subject.first_target(),
        strictness,
        frame_visible: true,
        read: files.read(),
    };
    let reply = subject.call(located, &request).await;
    // What the pass's locator resolved to, as upstream logs it before the input's own answer.
    if let Some(resolved) = reply.as_ref().ok().and_then(ProbeReply::resolution) {
        scope.log(resolved.log_line());
    }
    match reply {
        Ok(ProbeReply::Read { value, .. }) => serde_json::from_value(value.into_value())
            .map(Some)
            .map_err(|error| {
                scope.fail(
                    ErrorCode::Internal,
                    &format!("the probe answered a file input this build cannot read: {error}"),
                )
            }),
        Ok(ProbeReply::None { stale_ref: None }) => Ok(None),
        Ok(ProbeReply::Detached) => subject.detached().map(|()| None),
        Ok(other) => Err(subject.refused(other)),
        Err(ProbeFailure::Gone(_)) => subject.lost(located).map(|()| None),
        Err(failure) => Err(resolve::failed(scope, failure)),
    }
}

/// An error upstream's client throws before any channel call: no `<api>:` prefix and no call
/// log, and `not_sent`, since nothing was. The prelude keeps its message unprefixed (files.ts),
/// where it starts every other op error with the method the script called.
fn client_error(message: String) -> PlaywrightError {
    PlaywrightError::new(ErrorCode::InvalidInput, message)
}

/// upstream's client `fs.promises.stat(path)`, on the path as the script wrote it: whether it
/// names a directory, or Node's error.
async fn client_stat(path: &str) -> Result<bool, PlaywrightError> {
    refuse_nul(path)?;
    match fs::metadata(path).await {
        Ok(metadata) => Ok(metadata.is_dir()),
        Err(error) => Err(client_error(node_error(&error, "stat", path))),
    }
}

/// `addInitScript({ path })`'s source, as upstream's client builds it (`evaluationScript`):
/// `fs.promises.readFile(path, 'utf8')` on the path as the script wrote it, which the system
/// resolves against Neo's working directory as it does Node's (no lexical normalization, so
/// `a.txt/../b.js` fails `ENOTDIR` as it does there), and then `addSourceUrlToScript`'s
/// `//# sourceURL=` line naming that path. A file that is not UTF-8 reads with replacement
/// characters, as Node's decoder does. A failure is Node's error, unprefixed, with nothing sent.
pub(crate) async fn read_script_file(path: &str) -> Result<String, PlaywrightError> {
    refuse_nul(path)?;
    match fs::read(path).await {
        Ok(bytes) => Ok(format!(
            "{}\n//# sourceURL={}",
            String::from_utf8_lossy(&bytes),
            path.replace('\n', "")
        )),
        Err(error) => Err(client_error(node_error(&error, "open", path))),
    }
}

/// Node refuses a NUL byte in a path before the system call.
fn refuse_nul(path: &str) -> Result<(), PlaywrightError> {
    if path.contains('\0') {
        return Err(client_error(format!(
            "The argument 'path' must be a string, Uint8Array, or URL without null bytes. \
             Received {}",
            inspect(path)
        )));
    }
    Ok(())
}

/// Neo's working directory, which Node's `process.cwd()` gives the client's `path.resolve`.
fn working_directory() -> Result<String, PlaywrightError> {
    let cwd = env::current_dir().map_err(|error| {
        PlaywrightError::new(
            ErrorCode::Internal,
            format!("Neo cannot read its working directory: {error}"),
        )
    })?;
    cwd.into_os_string().into_string().map_err(|raw| {
        PlaywrightError::new(
            ErrorCode::Internal,
            format!(
                "Neo's working directory {} is not UTF-8",
                raw.to_string_lossy()
            ),
        )
    })
}

/// Node's `path.resolve(path)` on POSIX, against `cwd` (absolute): `path` joined onto `cwd`
/// unless it is absolute itself, then normalized without reading the disk. Empty and `.`
/// segments go, `..` takes the segment before it off and never goes past `/`, and no trailing
/// slash is left. A symlink is not followed, as Node does not follow one.
fn resolve_like_node(cwd: &str, path: &str) -> String {
    let joined = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("{cwd}/{path}")
    };
    let mut kept: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                kept.pop();
            }
            name => kept.push(name),
        }
    }
    format!("/{}", kept.join("/"))
}

/// Node's error for a failed file system call: libuv's code and text for the errno, the
/// syscall, and the path as it was passed (`ENOENT: no such file or directory, stat 'a.txt'`).
fn node_error(error: &io::Error, syscall: &str, path: &str) -> String {
    let (code, text) = errno_of(error)
        .and_then(|errno| {
            ERRNO_TEXT
                .iter()
                .find(|(number, ..)| *number == errno)
                .map(|&(_, code, text)| (code, text))
        })
        .unwrap_or(("UNKNOWN", "unknown error"));
    format!("{code}: {text}, {syscall} '{path}'")
}

/// libuv's name and text (uv.h `UV_ERRNO_MAP`) for each errno `stat` and `access` can return,
/// by this platform's number: the words Node puts in its file system errors.
const ERRNO_TEXT: &[(i32, &str, &str)] = &[
    (1, "EPERM", "operation not permitted"),
    (2, "ENOENT", "no such file or directory"),
    (4, "EINTR", "interrupted system call"),
    (5, "EIO", "i/o error"),
    (6, "ENXIO", "no such device or address"),
    (9, "EBADF", "bad file descriptor"),
    (12, "ENOMEM", "not enough memory"),
    (13, "EACCES", "permission denied"),
    (14, "EFAULT", "bad address in system call argument"),
    (16, "EBUSY", "resource busy or locked"),
    (19, "ENODEV", "no such device"),
    (20, "ENOTDIR", "not a directory"),
    (22, "EINVAL", "invalid argument"),
    (errno::ELOOP, "ELOOP", "too many symbolic links encountered"),
    (errno::ENAMETOOLONG, "ENAMETOOLONG", "name too long"),
    (
        errno::EOVERFLOW,
        "EOVERFLOW",
        "value too large for defined data type",
    ),
    (errno::ETIMEDOUT, "ETIMEDOUT", "connection timed out"),
];

/// The errnos whose numbers differ between Linux and the BSDs, macOS among them. The first
/// twenty-odd numbers are the same on both.
#[cfg(any(target_os = "linux", target_os = "android"))]
mod errno {
    pub(super) const ELOOP: i32 = 40;
    pub(super) const ENAMETOOLONG: i32 = 36;
    pub(super) const EOVERFLOW: i32 = 75;
    pub(super) const ETIMEDOUT: i32 = 110;
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
mod errno {
    pub(super) const ELOOP: i32 = 62;
    pub(super) const ENAMETOOLONG: i32 = 63;
    pub(super) const EOVERFLOW: i32 = 84;
    pub(super) const ETIMEDOUT: i32 = 60;
}

/// The POSIX errno behind a failed call; none elsewhere, where Node reports `UNKNOWN` here.
#[cfg(unix)]
fn errno_of(error: &io::Error) -> Option<i32> {
    error.raw_os_error()
}

#[cfg(not(unix))]
fn errno_of(_error: &io::Error) -> Option<i32> {
    None
}

/// Node's `util.inspect` of a string, as its argument errors quote one: single quotes unless
/// the text holds one (then double quotes, then backticks), with the quote, backslashes and
/// control characters escaped (`'a\x00b'`).
fn inspect(text: &str) -> String {
    let quote = if !text.contains('\'') {
        '\''
    } else if !text.contains('"') {
        '"'
    } else if !text.contains('`') && !text.contains("${") {
        '`'
    } else {
        '\''
    };
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push(quote);
    for character in text.chars() {
        match character {
            '\u{8}' => quoted.push_str("\\b"),
            '\t' => quoted.push_str("\\t"),
            '\n' => quoted.push_str("\\n"),
            '\u{c}' => quoted.push_str("\\f"),
            '\r' => quoted.push_str("\\r"),
            '\\' => quoted.push_str("\\\\"),
            control if u32::from(control) < 0x20 || control == '\u{7f}' => {
                let _ = write!(quoted, "\\x{:02X}", u32::from(control));
            }
            same if same == quote => {
                quoted.push('\\');
                quoted.push(same);
            }
            other => quoted.push(other),
        }
    }
    quoted.push(quote);
    quoted
}

#[cfg(test)]
mod tests;
