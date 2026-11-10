//! Playwright: `server/frames.ts` `Frame.setInputFiles`
//! Neo: `Locator/Page/Frame.setInputFiles`: Playwright's file preparation
//! (`server/fileUploadUtils.ts` `prepareFilesForUpload`, plus its client's path checks, which
//! need the file system a script lacks), then loop A (`click.rs`) around the per-element action
//! [`SetInputFiles`]. `drop_on.rs` shares the preparation.
//!
//! ```text
//!  set_input_files(tab, params, progress)
//!   prepare: payloads typed by name | files inside the artifact directory | one directory
//!   loop A (no actionability, `waitForFrameVisible: false`; the action pre-checks)
//!     SetInputFiles::act(element)
//!       fileInput: the element retargeted to its input; Playwright's checks throw (the Error)
//!       payloads  → setInputFilePayloads (the input's `input` and `change` events)
//!       files     → DOM.setFileInputFiles
//!       directory → inputEventWaiter, DOM.setFileInputFiles, waitForInputEvent
//! ```
//!
//! [`SetInputFiles`] also runs once on a fixed element (`ElementHandle.setInputFiles`), whose
//! caller reads its `NotConnected` as Playwright's "Element is not attached to the DOM".

use std::future::Future;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use browseros_cdp::CdpError;
use neo_playwright_protocol::injected::{
    InjectedFileInputParams, InjectedInputEventWaiterParams, InjectedSetInputFilePayloadsParams,
    InjectedWaitForInputEventParams, InputEventWaiter, UploadFile,
};
use neo_playwright_protocol::serve::FrameSetInputFilesParams;
use neo_playwright_protocol::{Binary, Element, Handle};
use tokio::sync::oneshot;

use crate::artifacts::{Artifacts, ExistingPath};
use crate::browser_representations::page::{FrameTab, GroupGuard, ResolvedElement};
use crate::chromium_bridge::cdp;
use crate::chromium_bridge::frames::FrameContext;
use crate::mime;

use crate::playwright_methods::retry::{self, ActionResult, ElementAction, PreChecks};
use crate::playwright_methods::tab::{PageError, Stop, Tab};
use crate::progress::{CallError, DeliveryCell, InputTicket, PlaywrightError, Progress};

/// Playwright: `server/fileUploadUtils.ts` `prepareFilesForUpload`: `mimeType: payload.mimeType || mime.getType(payload.name) || 'application/octet-stream',`
const OCTET_STREAM: &str = "application/octet-stream";

/// Playwright: `server/fileUploadUtils.ts` `prepareFilesForUpload`: `throw new Error('Exactly one of payloads, localPaths and streams must be provided');`
pub(crate) const EXACTLY_ONE: &str =
    "Exactly one of payloads, localPaths and streams must be provided";

/// Neo: dispatch gives every input method a delivery cell; a call without one is a broken route.
const NO_DELIVERY_CELL: &str = "neo-runtime: Frame.setInputFiles has no delivery cell";

/// Neo: Neo's SDK names a directory among `localPaths` and never sends streams, so a wire with
/// only those is not Neo's.
const NOT_NEOS_WIRE: &str =
    "neo-runtime: Neo's SDK names a directory in localPaths and sends no streams";

/// Neo: a delivery cell begins once; a second set would repeat input the page may have seen.
const SET_AGAIN: &str = "neo-runtime: a setInputFiles tried to set its files again";

/// Neo: a transport failure after the files were sent proves nothing, so the call ends rather
/// than set them again.
const MAYBE_SET: &str =
    "neo-runtime: setting the input's files failed after they may have been set";

/// Neo: the option a script's file paths are named by in errors, as Playwright's first
/// argument (`setInputFiles(files)`, `drop({ files })`).
pub(crate) const FILES_OPTION: &str = "files";

/// Playwright: `server/fileUploadUtils.ts` `prepareFilesForUpload`: `return { localPaths, localDirectory, filePayloads };`
///
/// What `setInputFiles` sets, prepared once before loop A: Playwright's `InputFilesItems`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InputFiles {
    /// Buffers, each typed by its name when the script gave no `mimeType`.
    Payloads(Vec<UploadFile>),
    /// Absolute paths of files inside the run's artifact directory.
    Paths(Vec<String>),
    /// The absolute path of one directory inside it, for a `webkitdirectory` input.
    Directory(String),
}

impl InputFiles {
    /// Playwright: `server/dom.ts` `ElementHandle._setInputFiles`: `const multiple = filePayloads && filePayloads.length > 1 || localPaths && localPaths.length > 1;`
    fn multiple(&self) -> bool {
        match self {
            Self::Payloads(payloads) => payloads.len() > 1,
            Self::Paths(paths) => paths.len() > 1,
            Self::Directory(_) => false,
        }
    }
}

/// Neo: one payload as the wire carries it, from `setInputFiles` or `drop`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WirePayload {
    pub(crate) name: String,
    pub(crate) mime_type: Option<String>,
    pub(crate) buffer: Binary,
}

/// Neo: what a script's file paths name once Rust has looked at them (Playwright's client does
/// with `fs.promises.stat`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LocalFiles {
    Files(Vec<ExistingPath>),
    Directory(ExistingPath),
}

/// Playwright: `server/fileUploadUtils.ts` `prepareFilesForUpload`: `if ([payloads, localPaths, localDirectory, streams, directoryStream].filter(Boolean).length !== 1)`
///
/// The wire's payloads or paths, of which exactly one is given. `unsupported` counts what
/// Neo's SDK never sends (`localDirectory`, streams): Rust learns a directory from the paths.
///
/// # Errors
///
/// Playwright's preparation and path errors, and a path Neo will not read.
pub(crate) async fn prepare_files(
    payloads: Option<Vec<WirePayload>>,
    local_paths: Option<Vec<String>>,
    unsupported: usize,
    artifacts: &Artifacts,
) -> Result<InputFiles, PlaywrightError> {
    let given = usize::from(payloads.is_some()) + usize::from(local_paths.is_some()) + unsupported;
    if given != 1 {
        return Err(PlaywrightError::error(EXACTLY_ONE));
    }
    if let Some(payloads) = payloads {
        return Ok(InputFiles::Payloads(
            payloads.into_iter().map(upload_payload).collect(),
        ));
    }
    let Some(local_paths) = local_paths else {
        return Err(PlaywrightError::error(NOT_NEOS_WIRE));
    };
    Ok(match resolve_paths(&local_paths, artifacts).await? {
        LocalFiles::Files(files) => {
            InputFiles::Paths(files.into_iter().map(|file| file.path).collect())
        }
        LocalFiles::Directory(directory) => InputFiles::Directory(directory.path),
    })
}

/// Playwright: `client/elementHandle.ts` `resolvePathsAndDirectoryForInputFiles`: `throw new Error('File paths must be all files or a single directory');`
///
/// Each path as Playwright's client sorts them, in order: a directory, at most one, or files.
///
/// # Errors
///
/// Playwright's two errors, and a path Neo will not read ([`Artifacts::existing`]).
pub(crate) async fn resolve_paths(
    items: &[String],
    artifacts: &Artifacts,
) -> Result<LocalFiles, PlaywrightError> {
    let mut files = Vec::new();
    let mut directory = None;
    for item in items {
        let found = artifacts.existing(FILES_OPTION, item).await?;
        if found.is_dir {
            if directory.is_some() {
                // Playwright: `client/elementHandle.ts` `resolvePathsAndDirectoryForInputFiles`: `throw new Error('Multiple directories are not supported');`
                return Err(PlaywrightError::error(
                    "Multiple directories are not supported",
                ));
            }
            directory = Some(found);
        } else {
            files.push(found);
        }
    }
    match directory {
        Some(_) if !files.is_empty() => Err(PlaywrightError::error(
            "File paths must be all files or a single directory",
        )),
        Some(directory) => Ok(LocalFiles::Directory(directory)),
        None => Ok(LocalFiles::Files(files)),
    }
}

/// Playwright: `server/fileUploadUtils.ts` `prepareFilesForUpload`: `buffer: payload.buffer.toString('base64'),`
///
/// One payload as the page takes it: base64 bytes, typed by its name without a `mimeType`.
pub(crate) fn upload_payload(payload: WirePayload) -> UploadFile {
    let mime_type = payload
        .mime_type
        .filter(|mime_type| !mime_type.is_empty())
        .or_else(|| mime::get_type(&payload.name).map(str::to_string))
        .unwrap_or_else(|| OCTET_STREAM.to_string());
    UploadFile {
        buffer: STANDARD.encode(payload.buffer.as_bytes()),
        name: payload.name,
        mime_type,
        last_modified_ms: None,
    }
}

/// Serves `Frame.setInputFiles`. `Ok(())` is Playwright's `{}` reply; the call log and delivery
/// are in `progress`.
pub(crate) async fn set_input_files<T: FilesTab>(
    tab: &T,
    params: FrameSetInputFilesParams,
    progress: &Progress,
) -> Result<(), CallError> {
    // `setInputFiles` is an input method in the spec, so dispatch always gives it a cell.
    let Some(delivery) = progress.delivery() else {
        return Err(PlaywrightError::error(NO_DELIVERY_CELL).into());
    };
    let FrameSetInputFilesParams {
        selector,
        strict,
        payloads,
        local_directory,
        directory_stream,
        local_paths,
        streams,
    } = params;
    let unsupported = [
        local_directory.is_some(),
        directory_stream.is_some(),
        streams.is_some(),
    ]
    .into_iter()
    .filter(|given| *given)
    .count();
    let payloads = payloads.map(|items| {
        items
            .into_iter()
            .map(|item| WirePayload {
                name: item.name,
                mime_type: item.mime_type,
                buffer: item.buffer,
            })
            .collect()
    });
    // Playwright: `server/frames.ts` `Frame.setInputFiles`: `const inputFileItems = await progress.race(prepareFilesForUpload(this, params));`
    let files = progress
        .race(prepare_files(
            payloads,
            local_paths,
            unsupported,
            progress.artifacts(),
        ))
        .await??;
    // Playwright: `server/frames.ts` `Frame.setInputFiles`: `return dom.assertDone(await this._retryWithProgressIfNotConnected(progress, target, { waitForFrameVisible: false }, (progress, handle, box) => handle._setInputFiles(progress, inputFileItems, box)));`
    retry::retry_with_progress_if_not_connected(
        tab,
        &selector,
        strict.unwrap_or(false),
        PreChecks::Run,
        progress,
        &SetInputFiles {
            files: &files,
            delivery,
        },
    )
    .await
}

/// Playwright: `server/dom.ts` `ElementHandle._setInputFiles`
///
/// The per-element `setInputFiles`, for loop A over a selector and for one fixed element. No
/// actionability check and no input turn: Playwright's path writes no device input. `files`
/// is [`prepare_files`]' answer; the delivery cell records whether the files were set.
pub(crate) struct SetInputFiles<'a> {
    pub(crate) files: &'a InputFiles,
    pub(crate) delivery: &'a Arc<DeliveryCell>,
}

impl<T: FilesTab> ElementAction<T> for SetInputFiles<'_> {
    type Output = ();

    async fn act(
        &self,
        tab: &T,
        element: &T::Element,
        progress: &Progress,
    ) -> Result<ActionResult<()>, Stop> {
        let directory = matches!(self.files, InputFiles::Directory(_));
        // Playwright: `server/dom.ts` `ElementHandle._setInputFiles`: `if (result === 'error:notconnected' || !result.asElement())`
        let Some(input) = progress
            .race(tab.file_input(element, self.files.multiple(), directory))
            .await??
        else {
            return Ok(ActionResult::NotConnected);
        };
        // Playwright: `server/dom.ts` `ElementHandle._setInputFiles`: `// Install the listener on the input itself before setting the files, so that the event is not missed.`
        let waiter = if directory {
            progress.race(tab.input_event_waiter(&input)).await?
        } else {
            None
        };
        // The last chance to stop before the input's files change.
        progress.check()?;
        let ticket = self
            .delivery
            .begin()
            .map_err(|_| Stop::from(PlaywrightError::error(SET_AGAIN)))?;
        let set = match self.files {
            InputFiles::Payloads(payloads) => {
                progress
                    .race(tab.set_file_payloads(&input, payloads.clone(), ticket))
                    .await?
            }
            InputFiles::Paths(paths) => {
                progress
                    .race(tab.set_file_paths(&input, paths.clone(), ticket))
                    .await?
            }
            InputFiles::Directory(directory) => {
                progress
                    .race(tab.set_file_paths(&input, vec![directory.clone()], ticket))
                    .await?
            }
        };
        if set.is_ok()
            && let Some(waiter) = waiter
        {
            progress.race(tab.wait_for_input_event(waiter)).await?;
        }
        match set {
            Ok(()) => Ok(ActionResult::Done(())),
            // Chromium refused the paths (no file input), so nothing was set: Playwright's loop A
            // retries every error but a page script's.
            Err(FilesError::Refused) => Err(Stop::Retry),
            Err(FilesError::Page(PageError::NotConnected)) => {
                Err(PlaywrightError::error(MAYBE_SET).into())
            }
            Err(FilesError::Page(error)) => Err(error.into()),
        }
    }
}

/// How a step that sets files failed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FilesError {
    /// The browser answered that it set nothing (`DOM.setFileInputFiles` on a node that is no
    /// file input): the ticket is back to `NotSent`.
    Refused,
    /// The page's error; after a transport failure the ticket stays `Unknown`.
    Page(PageError),
}

/// What [`SetInputFiles`] needs from a tab beyond loop A's steps. A step that sets files runs on
/// the page's own task once sent, holding the input and its ticket until the page answers, so
/// a call that stops waiting never cuts it short and its delivery is always marked (`forms.rs`).
pub(crate) trait FilesTab: Tab {
    /// The input `file_input` answered, alive until it drops.
    type FileInput: Send + Sync;
    /// The promise of an input's next `input` event.
    type Waiter: Send;

    /// The `fileInput` glue: the element retargeted to its input and Playwright's checks, which
    /// throw (`PageError::Threw`) for a node that cannot take these files. `None` when the
    /// node is no longer connected.
    fn file_input(
        &self,
        element: &Self::Element,
        multiple: bool,
        directory_upload: bool,
    ) -> impl Future<Output = Result<Option<Self::FileInput>, PageError>> + Send;

    /// `InjectedScript.setInputFiles` with `payloads`, then the input's events.
    fn set_file_payloads(
        &self,
        input: &Self::FileInput,
        payloads: Vec<UploadFile>,
        ticket: InputTicket,
    ) -> impl Future<Output = Result<(), FilesError>> + Send;

    /// CDP `DOM.setFileInputFiles` with absolute `paths` (Playwright's `setInputFilePaths`).
    fn set_file_paths(
        &self,
        input: &Self::FileInput,
        paths: Vec<String>,
        ticket: InputTicket,
    ) -> impl Future<Output = Result<(), FilesError>> + Send;

    /// Arms the input's `input` event promise; `None` when the page refused (Playwright's
    /// `.catch(() => undefined)`).
    fn input_event_waiter(
        &self,
        input: &Self::FileInput,
    ) -> impl Future<Output = Option<Self::Waiter>> + Send;

    /// Waits for the armed event; the page's errors end the wait (Playwright's `.catch(() => {})`).
    fn wait_for_input_event(&self, waiter: Self::Waiter) -> impl Future<Output = ()> + Send;
}

/// `FilesTab::FileInput` for the real tab: the input in the object group of its `fileInput`
/// call, released when the last copy drops.
#[derive(Clone)]
pub(crate) struct FileInputElement {
    handle: Handle<Element>,
    frame: Arc<FrameContext>,
    group: Arc<GroupGuard>,
}

/// `FilesTab::Waiter` for the real tab; it lives in its input's group.
pub(crate) struct InputWaiter {
    handle: Handle<InputEventWaiter>,
    input: FileInputElement,
}

impl FilesTab for FrameTab {
    type FileInput = FileInputElement;
    type Waiter = InputWaiter;

    async fn file_input(
        &self,
        element: &ResolvedElement,
        multiple: bool,
        directory_upload: bool,
    ) -> Result<Option<FileInputElement>, PageError> {
        let group = self.group(&element.frame, "set-input-files");
        let checked = element
            .frame
            .call(
                Some(group.group()),
                &InjectedFileInputParams {
                    element: element.handle().clone(),
                    multiple,
                    directory_upload,
                },
            )
            .await?;
        Ok(checked.input.map(|handle| FileInputElement {
            handle,
            frame: Arc::clone(&element.frame),
            group: Arc::new(group),
        }))
    }

    fn set_file_payloads(
        &self,
        input: &FileInputElement,
        payloads: Vec<UploadFile>,
        ticket: InputTicket,
    ) -> impl Future<Output = Result<(), FilesError>> + Send {
        let input = input.clone();
        files_step(self, async move {
            let set = input
                .frame
                .call(
                    None,
                    &InjectedSetInputFilePayloadsParams {
                        element: input.handle.clone(),
                        payloads,
                    },
                )
                .await;
            // Keep the group until the page has answered.
            drop(input);
            match set {
                Ok(_) => {
                    ticket.acked().delivered();
                    Ok(())
                }
                Err(error) => {
                    // A script error precedes the files' change; a transport failure proves
                    // nothing and leaves the dropped ticket `Unknown`.
                    if matches!(error, PageError::Threw(_)) {
                        ticket.unsent();
                    }
                    Err(FilesError::Page(error))
                }
            }
        })
    }

    fn set_file_paths(
        &self,
        input: &FileInputElement,
        paths: Vec<String>,
        ticket: InputTicket,
    ) -> impl Future<Output = Result<(), FilesError>> + Send {
        let input = input.clone();
        files_step(self, async move {
            // Playwright: `server/chromium/crPage.ts` `CRPage.setInputFilePaths`: `await progress.race(parentSession._client.send('DOM.setFileInputFiles', {`
            let set = input
                .frame
                .host
                .send_command(cdp::dom::SetFileInputFiles {
                    files: paths,
                    object_id: Some(input.handle.id().object_id().to_string()),
                    ..cdp::dom::SetFileInputFiles::default()
                })
                .await;
            let outcome = match set {
                Ok(_) => {
                    ticket.acked().delivered();
                    Ok(())
                }
                // Chromium answered: it set nothing (a node that is no file input, or gone).
                Err(CdpError::Protocol { .. }) => {
                    ticket.unsent();
                    Err(FilesError::Refused)
                }
                Err(error) => Err(FilesError::Page(input.frame.host.classify(&error))),
            };
            // Keep the group until the browser has answered.
            drop(input);
            outcome
        })
    }

    async fn input_event_waiter(&self, input: &FileInputElement) -> Option<InputWaiter> {
        let armed = input
            .frame
            .call(
                Some(input.group.group()),
                &InjectedInputEventWaiterParams {
                    element: input.handle.clone(),
                },
            )
            .await
            .ok()?;
        Some(InputWaiter {
            handle: armed.waiter,
            input: input.clone(),
        })
    }

    async fn wait_for_input_event(&self, waiter: InputWaiter) {
        let _event = waiter
            .input
            .frame
            .call(
                None,
                &InjectedWaitForInputEventParams {
                    waiter: waiter.handle,
                },
            )
            .await;
    }
}

/// Runs a step that sets files on the page's task, so it finishes, and marks its ticket, however
/// the call ends; the caller's future only waits for the answer.
fn files_step(
    tab: &FrameTab,
    work: impl Future<Output = Result<(), FilesError>> + Send + 'static,
) -> impl Future<Output = Result<(), FilesError>> + Send {
    let (sender, receiver) = oneshot::channel();
    tab.spawn(async move {
        let _waiting = sender.send(work.await);
    });
    async move {
        receiver
            .await
            .unwrap_or(Err(FilesError::Page(PageError::Closed)))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::future;

    use super::*;
    use crate::playwright_methods::tab::fake::FakeTab;
    use crate::progress::ErrorName;
    use crate::run::Delivery;

    thread_local! {
        /// What the fake's file steps answer in this test (a paused tokio test runs on one
        /// thread).
        static FILES: RefCell<FakeFiles> = RefCell::new(FakeFiles::default());
    }

    #[derive(Default)]
    struct FakeFiles {
        gone: bool,
        set: Option<Result<(), FilesError>>,
    }

    /// The fake's answer to a step that sets files, marking its ticket as the real one does.
    fn answer(ticket: InputTicket) -> Result<(), FilesError> {
        let set = FILES
            .with(|files| files.borrow_mut().set.take())
            .unwrap_or(Ok(()));
        match &set {
            Ok(()) => ticket.acked().delivered(),
            Err(FilesError::Refused) => ticket.unsent(),
            Err(FilesError::Page(_)) => drop(ticket),
        }
        set
    }

    impl FilesTab for FakeTab {
        type FileInput = u32;
        type Waiter = ();

        fn file_input(
            &self,
            element: &u32,
            multiple: bool,
            directory_upload: bool,
        ) -> impl Future<Output = Result<Option<u32>, PageError>> + Send {
            self.record(format!(
                "fileInput multiple={multiple} directory={directory_upload}"
            ));
            let gone = FILES.with(|files| files.borrow().gone);
            future::ready(Ok((!gone).then_some(*element)))
        }

        fn set_file_payloads(
            &self,
            _input: &u32,
            payloads: Vec<UploadFile>,
            ticket: InputTicket,
        ) -> impl Future<Output = Result<(), FilesError>> + Send {
            let names: Vec<String> = payloads.into_iter().map(|file| file.name).collect();
            self.record(format!("payloads {}", names.join(",")));
            future::ready(answer(ticket))
        }

        fn set_file_paths(
            &self,
            _input: &u32,
            paths: Vec<String>,
            ticket: InputTicket,
        ) -> impl Future<Output = Result<(), FilesError>> + Send {
            self.record(format!("paths {}", paths.join(",")));
            future::ready(answer(ticket))
        }

        fn input_event_waiter(&self, _input: &u32) -> impl Future<Output = Option<()>> + Send {
            self.record("waiter");
            future::ready(Some(()))
        }

        fn wait_for_input_event(&self, (): ()) -> impl Future<Output = ()> + Send {
            self.record("input event");
            future::ready(())
        }
    }

    fn payload(name: &str, mime_type: Option<&str>) -> WirePayload {
        WirePayload {
            name: name.to_string(),
            mime_type: mime_type.map(str::to_string),
            buffer: Binary(b"bytes".to_vec()),
        }
    }

    async fn run(files: InputFiles) -> (Result<(), CallError>, Vec<String>, Delivery) {
        let tab = FakeTab::ready();
        let progress = Progress::for_test(2_000.0);
        let delivery = progress.delivery().expect("an input call");
        let result = retry::retry_with_progress_if_not_connected(
            &tab,
            "#file",
            true,
            PreChecks::Run,
            &progress,
            &SetInputFiles {
                files: &files,
                delivery,
            },
        )
        .await;
        let calls = tab
            .calls()
            .into_iter()
            .filter(|call| !matches!(call.as_str(), "describe" | "resolve"))
            .collect();
        (result, calls, delivery.delivery())
    }

    /// Playwright's preparation: exactly one kind of files; payloads keep their type or take
    /// their name's, `application/octet-stream` when neither says.
    #[tokio::test]
    async fn tests_that_files_are_prepared_as_playwright_prepares_them() {
        let artifacts = Artifacts::new(None);
        for (payloads, paths, unsupported) in [
            (None, None, 0),
            (Some(vec![]), Some(vec![]), 0),
            (Some(vec![]), None, 1),
        ] {
            let error = prepare_files(payloads, paths, unsupported, &artifacts)
                .await
                .expect_err("not exactly one");
            assert_eq!(error.message, EXACTLY_ONE);
        }
        let prepared = prepare_files(
            Some(vec![
                payload("a.json", None),
                payload("b.txt", Some("text/x-custom")),
                payload("c.unknownext", Some("")),
            ]),
            None,
            0,
            &artifacts,
        )
        .await
        .expect("payloads");
        let InputFiles::Payloads(files) = prepared else {
            panic!("payloads: {prepared:?}");
        };
        assert_eq!(
            files
                .iter()
                .map(|file| (file.mime_type.as_str(), file.buffer.as_str()))
                .collect::<Vec<_>>(),
            [
                ("application/json", "Ynl0ZXM="),
                ("text/x-custom", "Ynl0ZXM="),
                (OCTET_STREAM, "Ynl0ZXM="),
            ]
        );
    }

    /// Paths resolve in the artifact directory to files or to one directory, with Playwright's
    /// client errors for two directories and for a directory with files.
    #[tokio::test]
    async fn tests_that_paths_are_files_or_one_directory() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("run");
        for name in ["tree/one.txt", "other/two.txt", "a.txt"] {
            let file = root.join(name);
            fs::create_dir_all(file.parent().expect("a parent")).expect("parents");
            fs::write(&file, name).expect("a file");
        }
        let artifacts = Artifacts::new(Some(root));
        let paths = |items: &[&str]| Some(items.iter().map(|item| (*item).to_string()).collect());
        let files = prepare_files(None, paths(&["a.txt", "tree/one.txt"]), 0, &artifacts)
            .await
            .expect("files");
        assert!(
            matches!(&files, InputFiles::Paths(found) if found.len() == 2 && found[0].ends_with("run/a.txt")),
            "{files:?}"
        );
        let folder = prepare_files(None, paths(&["tree"]), 0, &artifacts)
            .await
            .expect("a folder");
        assert!(
            matches!(&folder, InputFiles::Directory(found) if found.ends_with("run/tree")),
            "{folder:?}"
        );
        for (items, message) in [
            (
                &["tree", "other"][..],
                "Multiple directories are not supported",
            ),
            (
                &["tree", "a.txt"][..],
                "File paths must be all files or a single directory",
            ),
        ] {
            let error = prepare_files(None, paths(items), 0, &artifacts)
                .await
                .expect_err("refused");
            assert_eq!(error.message, message);
        }
    }

    /// Payloads go to the retargeted input once, `Done`; a node gone retries loop A until the
    /// deadline with nothing set.
    #[tokio::test(start_paused = true)]
    async fn tests_that_payloads_are_set_once_on_the_input() {
        let files = InputFiles::Payloads(vec![upload_payload(payload("a.txt", None))]);
        let (result, calls, delivery) = run(files.clone()).await;
        assert_eq!(result, Ok(()));
        assert_eq!(
            calls,
            ["fileInput multiple=false directory=false", "payloads a.txt"]
        );
        assert_eq!(delivery, Delivery::Done);

        FILES.with(|state| state.borrow_mut().gone = true);
        let (result, calls, delivery) = run(files).await;
        FILES.with(|state| state.borrow_mut().gone = false);
        assert!(
            matches!(&result, Err(CallError::Playwright(error)) if error.name == ErrorName::TimeoutError),
            "{result:?}"
        );
        assert!(
            calls.iter().all(|call| call.starts_with("fileInput")),
            "{calls:?}"
        );
        assert_eq!(delivery, Delivery::NotSent);
    }

    /// Chromium refusing the paths (no file input) sets nothing, so loop A tries again, as
    /// Playwright's retries every error but a page script's.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_refused_path_retries_with_nothing_set() {
        FILES.with(|state| state.borrow_mut().set = Some(Err(FilesError::Refused)));
        let (result, calls, delivery) = run(InputFiles::Paths(vec![
            "/run/a.txt".to_string(),
            "/run/b.txt".to_string(),
        ]))
        .await;
        assert_eq!(result, Ok(()), "the second attempt sets them");
        assert_eq!(
            calls,
            [
                "fileInput multiple=true directory=false",
                "paths /run/a.txt,/run/b.txt",
                "fileInput multiple=true directory=false",
                "paths /run/a.txt,/run/b.txt",
            ]
        );
        assert_eq!(delivery, Delivery::Done);
    }

    /// A directory: the `input` event listener is armed before the path is set and awaited
    /// after it, as Playwright's `_setInputFiles` does.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_directory_waits_for_the_input_event() {
        let (result, calls, delivery) = run(InputFiles::Directory("/run/tree".to_string())).await;
        assert_eq!(result, Ok(()));
        assert_eq!(
            calls,
            [
                "fileInput multiple=false directory=true",
                "waiter",
                "paths /run/tree",
                "input event",
            ]
        );
        assert_eq!(delivery, Delivery::Done);
    }
}
