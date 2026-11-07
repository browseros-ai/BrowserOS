//! Playwright: `server/frames.ts` `Frame.drop`
//! Neo: `Locator.drop`: files and data dropped on an element at its point. The files are
//! `set_input_files.rs`'s preparation, local files read into payloads; then loop A
//! (`click.rs`) around loop B for the `drop` pointer action, whose input is Playwright's page
//! function (a `DataTransfer` and three drag events), not device input.
//!
//! ```text
//!  drop_on(tab, params, progress)
//!   neither files nor data → Error; a directory → Error
//!   payloads typed by name; local files read (name, type, bytes, mtime)
//!   loop A (the action pre-checks)
//!     loop B "drop": visible and stable, scroll, the point (`position`), the hit check at it
//!       (the `mouse` interceptor), then `dropOn` at the point with the input turn held
//!       not accepted → Error (Playwright's NonRecoverableDOMError)
//! ```

use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use neo_runtime_spec::injected::{
    DropData, DropOutcome, ElementState, HitTargetAction, InjectedDropOnParams,
    InjectedDropOnResult, Point, UploadFile,
};
use neo_runtime_spec::serve::FrameDropParams;
use tokio::fs;
use tokio::sync::oneshot;

use crate::artifacts::{Artifacts, ExistingPath};
use crate::click::{
    self, ActionResult, Arming, ElementAction, NotReady, PageError, StatesCheck, Stop, Tab, Verdict,
};
use crate::mime;
use crate::page::{FrameTab, InputPermit, ResolvedElement};
use crate::progress::{CallError, DeliveryCell, InputTicket, PlaywrightError, Progress};
use crate::set_input_files::{self, LocalFiles, WirePayload};

/// Playwright: `server/frames.ts` `Frame.drop`: `throw new Error('At least one of "files" or "data" must be provided.');`
const NOTHING_TO_DROP: &str = "At least one of \"files\" or \"data\" must be provided.";

/// Playwright: `client/frame.ts` `Frame._drop`: `throw new Error('Dropping a directory is not supported — pass individual files.');`
const DIRECTORY_DROP: &str = "Dropping a directory is not supported — pass individual files.";

/// Playwright: `server/dom.ts` `ElementHandle._drop`: `throw new NonRecoverableDOMError('Drop target did not accept the drop — its dragover handler did not call preventDefault()');`
const NOT_ACCEPTED: &str =
    "Drop target did not accept the drop — its dragover handler did not call preventDefault()";

/// Deviation: `server/dom.ts` `ElementHandle._drop`: `buffer: (await fs.promises.readFile(p)).toString('base64'),` Playwright reads local files of any size into the page call. Neo holds them in memory and hands them to the page in one CDP message, so it caps their total at 50 MiB, Playwright's own cap for a browser that does not share its files (`fileUploadUtils.ts`), and refuses more with an Error naming the size.
const MAX_DROP_FILE_BYTES: u64 = 50 * 1024 * 1024;

/// Neo: dispatch gives every input method a delivery cell; a call without one is a broken route.
const NO_DELIVERY_CELL: &str = "neo-runtime: Frame.drop has no delivery cell";

/// Neo: Neo's SDK sends dropped files as payloads or paths, never as streams.
const NOT_NEOS_WIRE: &str =
    "neo-runtime: Neo's SDK sends files as payloads or localPaths, not streams";

/// Neo: a delivery cell begins once; a second drop would repeat events the page has seen.
const DROP_AGAIN: &str = "neo-runtime: a drop tried to drop again";

/// Neo: a transport failure after the drop was sent proves nothing, so the call ends rather
/// than drop again.
const MAYBE_DROPPED: &str =
    "neo-runtime: the drop's page call failed after the page may have seen it";

/// What a drop carries to the page, prepared once before loop A.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DropItems {
    pub(crate) payloads: Vec<UploadFile>,
    pub(crate) data: Vec<DropData>,
}

/// Serves `Frame.drop`. `Ok(())` is Playwright's `{}` reply; the call log and delivery are in
/// `progress`.
pub(crate) async fn drop_on<T: DropTab>(
    tab: &T,
    params: FrameDropParams,
    progress: &Progress,
) -> Result<(), CallError> {
    // `drop` is an input method in the spec, so dispatch always gives it a cell.
    let Some(delivery) = progress.delivery() else {
        return Err(PlaywrightError::error(NO_DELIVERY_CELL).into());
    };
    let FrameDropParams {
        selector,
        strict,
        position,
        payloads,
        local_paths,
        streams,
        data,
    } = params;
    // Playwright: `server/frames.ts` `Frame.drop`: `const hasFiles = !!(params.payloads?.length || params.localPaths?.length || params.streams?.length);`
    let has_files = payloads.as_ref().is_some_and(|items| !items.is_empty())
        || local_paths.as_ref().is_some_and(|items| !items.is_empty())
        || streams.as_ref().is_some_and(|items| !items.is_empty());
    let data: Vec<DropData> = data
        .unwrap_or_default()
        .into_iter()
        .map(|entry| DropData {
            mime_type: entry.mime_type,
            value: entry.value,
        })
        .collect();
    if !has_files && data.is_empty() {
        return Err(PlaywrightError::error(NOTHING_TO_DROP).into());
    }
    let payloads = if has_files {
        if streams.is_some() {
            return Err(PlaywrightError::error(NOT_NEOS_WIRE).into());
        }
        let payloads = payloads.map(|items| {
            items
                .into_iter()
                .map(|item| WirePayload {
                    name: item.name,
                    mime_type: item.mime_type,
                    buffer: item.buffer,
                })
                .collect::<Vec<_>>()
        });
        progress
            .race(drop_files(payloads, local_paths, progress.artifacts()))
            .await??
    } else {
        Vec::new()
    };
    let items = Arc::new(DropItems { payloads, data });
    let position = position.as_ref().map(click::point);
    // Playwright: `server/frames.ts` `Frame.drop`: `dom.assertDone(await this._retryWithProgressIfNotConnected(progress, target, { waitForFrameVisible: true }, (progress, handle, box, frameVisible) => handle._drop(progress, inputFileItems, data, params, frameVisible)));`
    click::retry_with_progress_if_not_connected(
        tab,
        &selector,
        strict.unwrap_or(false),
        false,
        progress,
        &DropAction {
            items: &items,
            position: position.as_ref(),
            delivery,
        },
    )
    .await
}

/// Playwright: `server/frames.ts` `Frame.drop`: `const inputFileItems = hasFiles ? await progress.race(prepareFilesForUpload(this, params)) : { filePayloads: undefined, localPaths: undefined };`
///
/// The files as the page takes them: payloads typed by name, local files read whole.
async fn drop_files(
    payloads: Option<Vec<WirePayload>>,
    local_paths: Option<Vec<String>>,
    artifacts: &Artifacts,
) -> Result<Vec<UploadFile>, PlaywrightError> {
    if payloads.is_some() && local_paths.is_some() {
        return Err(PlaywrightError::error(set_input_files::EXACTLY_ONE));
    }
    if let Some(payloads) = payloads {
        return Ok(payloads
            .into_iter()
            .map(set_input_files::upload_payload)
            .collect());
    }
    let local_paths = local_paths.unwrap_or_default();
    let files = match set_input_files::resolve_paths(&local_paths, artifacts).await? {
        LocalFiles::Directory(_) => return Err(PlaywrightError::error(DIRECTORY_DROP)),
        LocalFiles::Files(files) => files,
    };
    let total = files.iter().map(|file| file.size).sum::<u64>();
    if total > MAX_DROP_FILE_BYTES {
        return Err(too_large(total));
    }
    let mut uploads = Vec::with_capacity(files.len());
    for file in files {
        uploads.push(read_local(file).await?);
    }
    Ok(uploads)
}

/// Playwright: `server/dom.ts` `ElementHandle._drop`: `mimeType: mime.getType(p) || 'application/octet-stream',`
///
/// One local file as a payload: its base name, its type by its path, its bytes, its mtime.
async fn read_local(file: ExistingPath) -> Result<UploadFile, PlaywrightError> {
    let bytes = fs::read(&file.path)
        .await
        .map_err(|error| unreadable(&file, &error))?;
    let name = Path::new(&file.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&file.display)
        .to_string();
    Ok(UploadFile {
        mime_type: mime::get_type(&file.path)
            .unwrap_or("application/octet-stream")
            .to_string(),
        buffer: STANDARD.encode(&bytes),
        name,
        last_modified_ms: file.modified_ms,
    })
}

/// Neo: the refusal of [`MAX_DROP_FILE_BYTES`], naming the total.
fn too_large(total: u64) -> PlaywrightError {
    PlaywrightError::error(format!(
        "files: the files to drop total {total} bytes, over the {MAX_DROP_FILE_BYTES} bytes Neo \
         hands a page at once"
    ))
}

/// Neo: a local file found in the artifact directory that cannot be read now.
fn unreadable(file: &ExistingPath, error: &io::Error) -> PlaywrightError {
    PlaywrightError::error(format!(
        "files: \"{}\" cannot be read in the run's artifact directory: {error}",
        file.display
    ))
}

/// Playwright: `server/dom.ts` `ElementHandle._drop`: `return this._retryPointerAction(progress, { name: 'drop', waitForEnabled: false, waitAfter: 'disabled', perform }, options, frameVisible);`
///
/// Loop B for the drop (`_retryAction` with `_retryPointerAction`): the same element until the
/// drop is done, the element is detached, or the call stops.
struct DropAction<'a> {
    items: &'a Arc<DropItems>,
    position: Option<&'a Point>,
    delivery: &'a Arc<DeliveryCell>,
}

/// Playwright: `server/dom.ts` `ElementHandle._retryAction`: `if (!options.skipActionPreChecks && !options.force)`
impl<T: DropTab> ElementAction<T> for DropAction<'_> {
    type Output = ();

    async fn act(
        &self,
        tab: &T,
        element: &T::Element,
        progress: &Progress,
    ) -> Result<ActionResult<()>, Stop> {
        let mut retry = 0;
        loop {
            if retry == 0 {
                progress.log("attempting drop action");
            } else {
                progress.log("retrying drop action");
                let wait = click::loop_b_wait(retry);
                if !wait.is_zero() {
                    progress.log(format!("  waiting {}ms", wait.as_millis()));
                    progress.wait(wait).await?;
                }
            }
            click::wait_out_pending_navigation(tab, progress).await?;
            let reason = match self.attempt(tab, element, retry, progress).await? {
                None => return Ok(ActionResult::Done(())),
                Some(NotReady::NotConnected) => return Ok(ActionResult::NotConnected),
                Some(reason) => reason,
            };
            retry += 1;
            if let Some(line) = click::not_ready_line(&reason) {
                progress.log(line);
            }
        }
    }
}

impl DropAction<'_> {
    /// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`
    ///
    /// One attempt, retry index `retry`: visible and stable, the scroll, the point, the hit
    /// check, then the drop. `Some` is why the drop is not done yet.
    async fn attempt<T: DropTab>(
        &self,
        tab: &T,
        element: &T::Element,
        retry: usize,
        progress: &Progress,
    ) -> Result<Option<NotReady>, Stop> {
        progress.log("  waiting for element to be visible and stable");
        match progress
            .race(tab.check_states(element, &DROP_STATES, true))
            .await??
        {
            StatesCheck::Ready => {}
            StatesCheck::NotConnected => return Ok(Some(NotReady::NotConnected)),
            StatesCheck::Missing(state) => return Ok(Some(NotReady::Missing(state))),
        }
        progress.log("  element is visible and stable");
        progress.log("  scrolling into view if needed");
        let scroll = click::SCROLLS[retry % click::SCROLLS.len()];
        if let Some(reason) = progress
            .race(tab.scroll(element, scroll, self.position))
            .await??
        {
            return Ok(Some(reason));
        }
        progress.log("  done scrolling");
        let geometry = progress
            .race(tab.geometry(element, self.position, false))
            .await??;
        let point = match click::click_point(&geometry, self.position) {
            Ok(point) => click::round_point(&point),
            Err(reason) => return Ok(Some(reason)),
        };
        // The hit check and the drop run inside the tab's input turn, as every pointer action's
        // input does, so another call's interceptor never judges this drop.
        let input = progress.race(tab.input()).await?;
        // Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `const actionType = action.name === 'move and up' ? 'drag' : ((action.name === 'hover' || action.name === 'tap') ? action.name : 'mouse');`
        let armed = match progress
            .race(tab.arm(
                element,
                point.clone(),
                HitTargetAction::Mouse,
                input.clone(),
            ))
            .await??
        {
            Arming::Armed(armed) => armed,
            Arming::NotConnected => return Ok(Some(NotReady::NotConnected)),
            Arming::Intercepted(target) => return Ok(Some(NotReady::Intercepted(target))),
        };
        // The last chance to stop before the page sees the drop.
        progress.check()?;
        let ticket = self
            .delivery
            .begin()
            .map_err(|_| Stop::from(PlaywrightError::error(DROP_AGAIN)))?;
        progress.log("  performing drop action");
        let dropped = progress
            .race(tab.drop_on(element, self.items, point, ticket, input.clone()))
            .await?;
        let outcome = match dropped {
            Ok(outcome) => outcome,
            Err(PageError::NotConnected) => {
                return Err(PlaywrightError::error(MAYBE_DROPPED).into());
            }
            Err(error) => return Err(error.into()),
        };
        if outcome == DropOutcome::NotAccepted {
            // Dropping the interceptor disarms it, as Playwright's `finally` does.
            drop(armed);
            return Err(PlaywrightError::error(NOT_ACCEPTED).into());
        }
        // Deviation: `server/dom.ts` `ElementHandle._performPointerAction`: `if (hitTargetResult !== 'done')` The drop's own events are untrusted, which the interceptor lets through, so only a trusted press during the drop can block; Playwright then drops again, while Neo, which never delivers input twice, ends the drop done.
        let _verdict: Verdict = progress.race(tab.verdict(armed)).await?;
        drop(input);
        // Playwright's perform ignores a node gone by then: the action is done.
        progress.log("  drop action done");
        progress.log("  waiting for scheduled navigations to finish");
        progress.log("  navigations have finished");
        Ok(None)
    }
}

/// Playwright: `server/dom.ts` `ElementHandle._performPointerAction`: `['visible', 'stable']`
/// (the drop's `waitForEnabled: false`).
const DROP_STATES: [ElementState; 2] = click::HOVER_STATES;

/// What a drop needs from a tab beyond loop B's pointer steps.
pub(crate) trait DropTab: Tab {
    /// The `dropOn` glue on `element` at `point`, on the page's own task once sent: it holds
    /// the element, the input turn and the ticket until the page answers, and marks the ticket
    /// (`Done` once the page saw the events, `NotSent` when the node was gone).
    fn drop_on(
        &self,
        element: &Self::Element,
        items: &Arc<DropItems>,
        point: Point,
        ticket: InputTicket,
        input: Self::Input,
    ) -> impl Future<Output = Result<DropOutcome, PageError>> + Send;
}

impl DropTab for FrameTab {
    fn drop_on(
        &self,
        element: &ResolvedElement,
        items: &Arc<DropItems>,
        point: Point,
        ticket: InputTicket,
        input: InputPermit,
    ) -> impl Future<Output = Result<DropOutcome, PageError>> + Send {
        let element = element.clone();
        let items = Arc::clone(items);
        let (sender, receiver) = oneshot::channel();
        self.spawn(async move {
            let _input = input;
            // Deviation: `server/dom.ts` `ElementHandle._drop`: `const mainContext = await progress.race(this._frame.mainContext());` Playwright builds the DataTransfer and dispatches the events in the page's main world, because Firefox strips the files of one that crosses from the utility world. Neo runs Playwright's same page function in its utility world, where Chromium hands the page's listeners the same DataTransfer and files; only a page that replaced DataTransfer, File, DragEvent or dispatchEvent sees the difference.
            let result = element
                .frame
                .call(
                    None,
                    &InjectedDropOnParams {
                        element: element.handle().clone(),
                        payloads: items.payloads.clone(),
                        data: items.data.clone(),
                        point,
                    },
                )
                .await;
            // Keep the group until the page has answered.
            drop(element);
            let outcome = match result {
                Ok(InjectedDropOnResult {
                    outcome: DropOutcome::ErrorNotconnected,
                }) => {
                    ticket.unsent();
                    Ok(DropOutcome::ErrorNotconnected)
                }
                Ok(InjectedDropOnResult { outcome }) => {
                    ticket.acked().delivered();
                    Ok(outcome)
                }
                Err(error) => {
                    // A script error precedes the events; a transport failure proves nothing
                    // and leaves the dropped ticket `Unknown`.
                    if matches!(error, PageError::Threw(_)) {
                        ticket.unsent();
                    }
                    Err(error)
                }
            };
            let _waiting = sender.send(outcome);
        });
        async move { receiver.await.unwrap_or(Err(PageError::Closed)) }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::future;

    use neo_runtime_spec::Binary;
    use serde_json::json;

    use super::*;
    use crate::click::tests::FakeTab;
    use crate::run::Delivery;

    thread_local! {
        /// What the fake's `dropOn` answers in this test (a paused tokio test runs on one thread).
        static OUTCOME: RefCell<DropOutcome> = const { RefCell::new(DropOutcome::Accepted) };
    }

    impl DropTab for FakeTab {
        fn drop_on(
            &self,
            _element: &u32,
            items: &Arc<DropItems>,
            point: Point,
            ticket: InputTicket,
            _input: Self::Input,
        ) -> impl Future<Output = Result<DropOutcome, PageError>> + Send {
            let names: Vec<&str> = items
                .payloads
                .iter()
                .map(|file| file.name.as_str())
                .collect();
            self.record(format!(
                "dropOn {} {} at ({}, {})",
                names.join(","),
                items.data.len(),
                point.x,
                point.y
            ));
            let outcome = OUTCOME.with(|outcome| *outcome.borrow());
            if outcome == DropOutcome::ErrorNotconnected {
                ticket.unsent();
            } else {
                ticket.acked().delivered();
            }
            future::ready(Ok(outcome))
        }
    }

    fn params(value: &serde_json::Value) -> FrameDropParams {
        serde_json::from_value(value.clone()).expect("drop params")
    }

    async fn run(value: serde_json::Value) -> (Result<(), CallError>, Vec<String>, Delivery) {
        let tab = FakeTab::ready();
        let progress = Progress::for_test(2_000.0);
        let result = drop_on(&tab, params(&value), &progress).await;
        let delivery = progress.delivery().expect("an input call").delivery();
        (result, tab.calls(), delivery)
    }

    /// Data is dropped once at the element's point, after the hit check, inside the turn.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_drop_runs_the_page_function_once_at_the_point() {
        let (result, calls, delivery) = run(json!({
            "selector": "#zone", "strict": true,
            "data": [{ "mimeType": "text/plain", "value": "hi" }],
        }))
        .await;
        assert_eq!(result, Ok(()));
        let arm = calls.iter().position(|call| call.starts_with("arm Mouse"));
        let dropped = calls.iter().position(|call| call.starts_with("dropOn"));
        assert!(arm.is_some() && arm < dropped, "{calls:?}");
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.starts_with("dropOn"))
                .count(),
            1
        );
        assert_eq!(delivery, Delivery::Done);
    }

    /// Playwright's two refusals: a target whose `dragover` was not prevented (the page saw
    /// the events), and a drop with neither files nor data (nothing sent).
    #[tokio::test(start_paused = true)]
    async fn tests_that_refused_drops_fail_as_playwright_fails_them() {
        OUTCOME.with(|outcome| *outcome.borrow_mut() = DropOutcome::NotAccepted);
        let (result, _, delivery) = run(json!({
            "selector": "#zone",
            "data": [{ "mimeType": "text/plain", "value": "hi" }],
        }))
        .await;
        OUTCOME.with(|outcome| *outcome.borrow_mut() = DropOutcome::Accepted);
        assert_eq!(result, Err(PlaywrightError::error(NOT_ACCEPTED).into()));
        assert_eq!(
            delivery,
            Delivery::Done,
            "dragenter and dragover reached the page"
        );

        for empty in [
            json!({ "selector": "#zone" }),
            json!({ "selector": "#zone", "payloads": [], "data": [] }),
        ] {
            let (result, calls, delivery) = run(empty).await;
            assert_eq!(result, Err(PlaywrightError::error(NOTHING_TO_DROP).into()));
            assert!(calls.is_empty(), "{calls:?}");
            assert_eq!(delivery, Delivery::NotSent);
        }
    }

    /// A node gone by the time the page function runs drops nothing, and the drop is done, as
    /// Playwright's perform ignores it.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_node_gone_at_the_drop_is_done_with_nothing_sent() {
        OUTCOME.with(|outcome| *outcome.borrow_mut() = DropOutcome::ErrorNotconnected);
        let (result, _, delivery) = run(json!({
            "selector": "#zone",
            "payloads": [{ "name": "a.txt", "buffer": "aGk=" }],
        }))
        .await;
        OUTCOME.with(|outcome| *outcome.borrow_mut() = DropOutcome::Accepted);
        assert_eq!(result, Ok(()));
        assert_eq!(delivery, Delivery::NotSent);
    }

    /// Local files become payloads (base name, type by path, bytes, mtime); a directory is
    /// refused as Playwright's client refuses it; payloads are typed by name.
    #[tokio::test]
    async fn tests_that_files_to_drop_are_read_as_playwright_reads_them() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let root = dir.path().join("run");
        fs::create_dir_all(root.join("tree")).expect("a folder");
        fs::write(root.join("note.md"), "# hi").expect("a file");
        let artifacts = Artifacts::new(Some(root));
        let read = drop_files(None, Some(vec!["note.md".to_string()]), &artifacts)
            .await
            .expect("read");
        assert_eq!(read.len(), 1);
        assert_eq!(
            (
                read[0].name.as_str(),
                read[0].mime_type.as_str(),
                read[0].buffer.as_str()
            ),
            ("note.md", "text/markdown", "IyBoaQ==")
        );
        assert!(read[0].last_modified_ms.is_some_and(|ms| ms > 0.0));
        let error = drop_files(None, Some(vec!["tree".to_string()]), &artifacts)
            .await
            .expect_err("a directory");
        assert_eq!(error.message, DIRECTORY_DROP);
        let typed = drop_files(
            Some(vec![WirePayload {
                name: "page.html".to_string(),
                mime_type: None,
                buffer: Binary(b"<p>".to_vec()),
            }]),
            None,
            &artifacts,
        )
        .await
        .expect("payloads");
        assert_eq!(typed[0].mime_type, "text/html");
    }
}
