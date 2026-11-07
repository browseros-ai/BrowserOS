//! Neo: the run's downloads, for `page.on('download')`, `context.on('download')` and the
//! `Download` objects they hand out (neo-sdk `download.ts`). While a run listens for downloads,
//! Neo asks the browser to report them on its connection and leaves the saving to the browser,
//! which puts each file where it puts the user's:
//!
//! ```text
//!  Runtime: Switch  holders 0 → 1: Browser.setDownloadBehavior {default, eventsEnabled: true}
//!                   holders 1 → 0: {default}: no more reports        shared by the Runtime's runs
//!  run: Tabs::watch_downloads ─▶ events receiver ─▶ hold ─▶ watch, until the run's events end
//!    Browser.downloadWillBegin {frameId, guid, url, suggestedFilename}
//!       ─▶ the run's tab of that frame ─▶ `download` on the context, naming the tab
//!    Browser.downloadProgress {guid, state: completed (filePath) | canceled}
//!       ─▶ the download's end, which `path()`, `failure()` and `delete()` wait for
//! ```
//!
//! Measured on BrowserOS neo 155: under `default` the browser reports every download it saves
//! (links, attachments, navigations to an attachment, `blob:` and `data:` URLs), each with
//! `downloadWillBegin`, repeated `inProgress` reports, then `completed` naming the file it wrote
//! in the profile's download directory, under the browser's own name. A download the browser
//! gives up on (a dropped connection, after 5 retries) ends `canceled`, as one `cancel()`
//! ended does. Reports go to the connection that turned them on, for every tab and context; the
//! behavior itself is per browser context, shared with every other client of the browser.
//!
//! A run's page calls wait until the reports are on ([`ALL_PAGES`]): any call may start a
//! download. The watcher registers with the hub's barrier, so a download a click began before
//! its reply reaches the listeners before that click's caller continues, and it keeps following
//! every download it handed out until the run's events end, also after the last listener
//! left: `await download.path()` comes after its `waitForEvent`.
//!
//! [`ALL_PAGES`]: crate::events::ALL_PAGES

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use browseros_cdp::CdpEvent;
use neo_runtime_spec::serve::{
    ArtifactCall, ArtifactFailureParams, ArtifactFailureResult, ArtifactPathAfterFinishedParams,
    ArtifactPathAfterFinishedResult,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::fs;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::watch;
use tracing::{debug, warn};

use crate::bridge;
use crate::discovery::{self, TargetInfo};
use crate::dispatch::encode_result;
use crate::events::{CONTEXT_GUID, Flushes};
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::switch::{Setting, Switches};
use crate::tabs::{self, Tabs};

/// Neo: the guid prefix of a run's downloads; the rest is the browser's download guid.
pub(crate) const DOWNLOAD_PREFIX: &str = "download@";

/// Deviation: `server/chromium/crBrowser.ts` `CRBrowserContext.initialize`: `behavior: this._options.acceptDownloads === 'accept' ? 'allowAndName' : 'deny',`
/// Playwright has the browser save each download of its context into Playwright's own directory
/// under a guid name, and removes that directory when it lets go of the browser. Neo's context is
/// the user's browser, so Neo leaves the saving to it (`default`: where, and under which name, the
/// user's settings say) and only turns the reports on. Off is `default` without reports: CDP has
/// no read of the behavior to restore, and `default` is the browser's own.
pub(crate) static DOWNLOAD_EVENTS: Setting = Setting {
    method: "Browser.setDownloadBehavior",
    params: download_behavior,
};

fn download_behavior(on: bool, context: Option<&str>) -> Value {
    let mut params = json!({ "behavior": "default" });
    if on {
        params["eventsEnabled"] = json!(true);
    }
    if let Some(context) = context {
        params["browserContextId"] = json!(context);
    }
    params
}

/// Neo: how many downloads a run keeps for their `Download` objects. Past it the oldest goes,
/// and a call on it throws [`DOWNLOAD_GONE`].
pub(crate) const MAX_KEPT_DOWNLOADS: usize = 1000;

/// Neo: what a call on a download past [`MAX_KEPT_DOWNLOADS`] throws.
const DOWNLOAD_GONE: &str = "Neo keeps a run's last 1000 downloads; this one is gone";

/// Playwright: `server/chromium/crBrowser.ts` `CRBrowser._onDownloadProgress`: `this.downloadFinished(payload.guid, this._closeReason || 'canceled');`
const CANCELED: &str = "canceled";

/// Neo: what `path()` throws when the browser reported a download complete without naming its
/// file (CDP marks `filePath` experimental; BrowserOS neo 155 named it every time, measured).
const NO_FILE_PATH: &str = "the browser reported the download complete without naming its file";

/// Neo: what `saveAs` throws. Neo writes a script's files only inside the artifact directory the
/// host gives the run, and this run has none.
const NO_ARTIFACT_DIRECTORY: &str = "path: this run has no artifact directory, so Neo cannot save the download; the host sets one per run";

/// CDP's `Browser.downloadWillBegin` params.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadWillBegin {
    frame_id: String,
    guid: String,
    url: String,
    suggested_filename: String,
}

/// CDP's `Browser.downloadProgress` params.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    guid: String,
    state: String,
    #[serde(default)]
    file_path: Option<String>,
}

/// How a download ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum End {
    /// `completed`, with the file the browser wrote when it named it.
    Completed(Option<String>),
    /// `canceled`: by `cancel()`, by the user, or a download the browser gave up on.
    Canceled,
}

/// The downloads the run's `download` events handed out, by the browser's guid.
#[derive(Default)]
pub(crate) struct Downloads(Mutex<Kept>);

#[derive(Default)]
struct Kept {
    /// Guids, oldest first, for [`MAX_KEPT_DOWNLOADS`].
    order: VecDeque<String>,
    downloads: HashMap<String, Download>,
}

struct Download {
    /// The browser context of the download's tab, which `Browser.cancelDownload` needs outside
    /// the default one (measured).
    context: Option<String>,
    /// `None` until the download ends.
    end: watch::Sender<Option<End>>,
    /// `delete()` removed its file: Playwright disposes the download then, so every later call
    /// on it fails as on a closed target.
    deleted: bool,
}

impl Downloads {
    /// Keeps a download the run hands out, in its tab's browser context.
    fn began(&self, guid: &str, context: Option<String>) {
        let Ok(mut kept) = self.0.lock() else {
            return;
        };
        if kept.downloads.contains_key(guid) {
            return;
        }
        let (end, _) = watch::channel(None);
        kept.downloads.insert(
            guid.to_string(),
            Download {
                context,
                end,
                deleted: false,
            },
        );
        kept.order.push_back(guid.to_string());
        while kept.order.len() > MAX_KEPT_DOWNLOADS {
            if let Some(oldest) = kept.order.pop_front() {
                kept.downloads.remove(&oldest);
            }
        }
    }

    /// Playwright: `server/chromium/crBrowser.ts` `CRBrowser._onDownloadProgress`: `if (payload.state === 'completed')`
    ///
    /// One progress report: the download ends at its first `completed` or `canceled`; the
    /// browser repeats `inProgress` many times, and nothing follows an end.
    fn progressed(&self, progress: DownloadProgress) {
        let end = match progress.state.as_str() {
            "completed" => End::Completed(progress.file_path),
            "canceled" => End::Canceled,
            _ => return,
        };
        let Ok(kept) = self.0.lock() else {
            return;
        };
        if let Some(download) = kept.downloads.get(&progress.guid) {
            download.end.send_if_modified(|current| {
                if current.is_some() {
                    return false;
                }
                *current = Some(end);
                true
            });
        }
    }

    /// The download's own record: its context and a receiver of its end. Fails for a download
    /// the run no longer keeps, and as on a closed target after `delete()`.
    fn get(
        &self,
        guid: &str,
    ) -> Result<(Option<String>, watch::Receiver<Option<End>>), PlaywrightError> {
        let kept = self.0.lock().map_err(|_| {
            PlaywrightError::error("neo-runtime: the run's downloads cannot be read")
        })?;
        let download = kept
            .downloads
            .get(guid)
            .ok_or_else(|| PlaywrightError::error(DOWNLOAD_GONE))?;
        if download.deleted {
            return Err(PlaywrightError::target_closed());
        }
        Ok((download.context.clone(), download.end.subscribe()))
    }

    /// Waits for the download to end.
    async fn end(&self, guid: &str) -> Result<End, PlaywrightError> {
        let (_, mut end) = self.get(guid)?;
        let ended = end
            .wait_for(Option::is_some)
            .await
            .map_err(|_| PlaywrightError::error(DOWNLOAD_GONE))?;
        ended
            .clone()
            .ok_or_else(|| PlaywrightError::error(DOWNLOAD_GONE))
    }

    fn deleted(&self, guid: &str) {
        if let Ok(mut kept) = self.0.lock()
            && let Some(download) = kept.downloads.get_mut(guid)
        {
            download.deleted = true;
        }
    }
}

/// Turns the browser's download reports on for the run and watches them until the run's events
/// end. Runs on the run's task tracker, so `Dispatcher::finish` waits for the holds' release.
///
/// The watcher also holds target discovery, for the tabs that open and close around a download
/// ([`Opened`]); without it the run still hears every other download.
pub(crate) async fn run(tabs: Tabs, switches: Switches) {
    let cdp = tabs.cdp().clone();
    // Subscribed first: every report from the switches on reaches this receiver.
    let events = cdp.events();
    let flushes = tabs.events().watcher();
    let discovery = switches.discovery.acquire(&cdp, None).await;
    if let Err(error) = &discovery {
        debug!(%error, "the download watcher cannot see new tabs; a new window's download is lost");
    }
    let hold = switches.downloads.acquire(&cdp, tabs.context_id()).await;
    tabs.downloads_ready();
    match hold {
        Ok(hold) => {
            watch(events, flushes, &tabs).await;
            hold.release().await;
        }
        Err(error) => {
            warn!(%error, "the run's download reports did not turn on; no download events");
            tabs.events().note(format!(
                "neo-runtime could not watch for downloads ({error}): no download events"
            ));
        }
    }
    if let Ok(discovery) = discovery {
        discovery.release().await;
    }
}

async fn watch(mut events: broadcast::Receiver<CdpEvent>, mut flushes: Flushes, tabs: &Tabs) {
    let ended = tabs.events().ended();
    let mut opened = Openings::default();
    loop {
        tokio::select! {
            biased;
            () = ended.cancelled() => return,
            event = events.recv() => match event {
                Ok(event) => on_event(&event, tabs, &mut opened).await,
                Err(RecvError::Lagged(skipped)) => lost(tabs, skipped),
                Err(RecvError::Closed) => return,
            },
            Some(ack) = flushes.recv() => {
                // A reply barrier (`events.rs`): every report already in the ring preceded the
                // reply that asked for it.
                loop {
                    match events.try_recv() {
                        Ok(event) => on_event(&event, tabs, &mut opened).await,
                        Err(TryRecvError::Empty | TryRecvError::Closed) => break,
                        Err(TryRecvError::Lagged(skipped)) => lost(tabs, skipped),
                    }
                }
                let _waiter = ack.send(());
            }
        }
    }
}

/// What target discovery reported of a tab: the tab that opened it, and whether its first
/// document committed (its url is `""` until then, measured).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Opened {
    pub(crate) opener: Option<String>,
    pub(crate) committed: bool,
}

/// Neo: how many tabs the download watcher remembers what discovery reported of. A tab that
/// opens only to download is gone before its download begins (measured), so the watcher keeps
/// its record past its end, the oldest first to go.
const MAX_OPENED: usize = 256;

/// The tabs discovery reported to the download watcher, by target id.
#[derive(Default)]
struct Openings {
    order: VecDeque<String>,
    tabs: HashMap<String, Opened>,
}

/// `Target.targetCreated` and `Target.targetInfoChanged` params.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetReport {
    target_info: TargetInfo,
}

impl Openings {
    /// One browser-level target report: a tab's opener, and its first commit.
    fn on(&mut self, event: &CdpEvent) {
        if !matches!(
            event.method.as_str(),
            "Target.targetCreated" | "Target.targetInfoChanged"
        ) {
            return;
        }
        let Ok(TargetReport { target_info: info }) =
            serde_json::from_value::<TargetReport>(event.params.clone())
        else {
            return;
        };
        if info.kind != "page" {
            return;
        }
        if !self.tabs.contains_key(&info.target_id) {
            self.order.push_back(info.target_id.clone());
            while self.order.len() > MAX_OPENED {
                if let Some(oldest) = self.order.pop_front() {
                    self.tabs.remove(&oldest);
                }
            }
        }
        let opened = self.tabs.entry(info.target_id).or_default();
        if info.opener_id.is_some() {
            opened.opener = info.opener_id;
        }
        if !discovery::is_initial_empty(&info.url) {
            opened.committed = true;
        }
    }

    fn get(&self, target: &str) -> Option<&Opened> {
        self.tabs.get(target)
    }
}

/// Lost browser events may hold a download's start or end; the run log says so.
fn lost(tabs: &Tabs, skipped: u64) {
    warn!(skipped, "the run's download watcher lagged");
    tabs.events().note(format!(
        "neo-runtime lost {skipped} browser events: a download's start or end may be missing"
    ));
}

async fn on_event(event: &CdpEvent, tabs: &Tabs, opened: &mut Openings) {
    if event.session_id.is_some() {
        return;
    }
    match event.method.as_str() {
        "Browser.downloadWillBegin" => {
            match serde_json::from_value::<DownloadWillBegin>(event.params.clone()) {
                Ok(began) => {
                    let opening = opened.get(&began.frame_id).cloned();
                    began_on(tabs, began, opening.as_ref()).await;
                }
                Err(error) => debug!(%error, "a download's start did not decode"),
            }
        }
        "Browser.downloadProgress" => {
            match serde_json::from_value::<DownloadProgress>(event.params.clone()) {
                Ok(progress) => tabs.downloads().progressed(progress),
                Err(error) => debug!(%error, "a download's progress did not decode"),
            }
        }
        _ => opened.on(event),
    }
}

/// Playwright: `server/chromium/crBrowser.ts` `CRBrowser._onDownloadWillBegin`: `const page = this._findOwningPage(payload.frameId);`
///
/// A download a frame began: Playwright's `download` event, on the tab and then the context,
/// while either listens. A frame of no tab of the run's context (an extension page, a developer
/// tools window) reports nothing, as in Playwright.
async fn began_on(tabs: &Tabs, began: DownloadWillBegin, opened: Option<&Opened>) {
    let Some(page) = tabs.download_page(&began.frame_id, opened).await else {
        debug!(frame = %began.frame_id, "a download began in no tab of the run's context");
        return;
    };
    let page_guid = bridge::page_guid(&page.id);
    let hub = tabs.events();
    if !hub.wants(&page_guid, "download", "download") {
        return;
    }
    tabs.downloads()
        .began(&began.guid, tabs.context_id().map(str::to_string));
    hub.emit(
        CONTEXT_GUID,
        "download",
        &json!({
            "page": { "guid": page_guid },
            "url": began.url,
            "suggestedFilename": began.suggested_filename,
            "artifact": format!("{DOWNLOAD_PREFIX}{}", began.guid),
        }),
    );
}

/// One `Artifact` call on the run's download `guid` (the browser's guid, without
/// [`DOWNLOAD_PREFIX`]).
pub(crate) async fn serve(
    tabs: &Tabs,
    guid: &str,
    call: ArtifactCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let downloads = tabs.downloads();
    match call {
        ArtifactCall::PathAfterFinished(_) => {
            match progress.race(downloads.end(guid)).await?? {
                End::Completed(Some(path)) => encode_result::<ArtifactPathAfterFinishedParams>(
                    &ArtifactPathAfterFinishedResult { value: path },
                ),
                End::Completed(None) => Err(PlaywrightError::error(NO_FILE_PATH).into()),
                End::Canceled => Err(PlaywrightError::error(CANCELED).into()),
            }
        }
        ArtifactCall::Failure(_) => {
            let error = match progress.race(downloads.end(guid)).await?? {
                End::Completed(_) => None,
                End::Canceled => Some(CANCELED.to_string()),
            };
            encode_result::<ArtifactFailureParams>(&ArtifactFailureResult { error })
        }
        ArtifactCall::Cancel(_) => {
            let (context, _) = downloads.get(guid)?;
            let mut params = json!({ "guid": guid });
            if let Some(context) = context {
                params["browserContextId"] = json!(context);
            }
            progress
                .race(tabs.cdp().send("Browser.cancelDownload", params, None))
                .await?
                .map_err(|error| tabs::cdp_error(&error))?;
            Ok(None)
        }
        ArtifactCall::Delete(_) => delete(tabs, guid, progress).await,
        ArtifactCall::SaveAs(_) => save_as(downloads, guid),
    }
}

/// Deviation: `server/dispatchers/artifactDispatcher.ts` `ArtifactDispatcher.saveAs`: `await fs.promises.copyFile(localPath, params.path);`
/// Playwright copies the file to any path on its host. Neo writes a script's files only inside the
/// artifact directory the host gives the run, and this run has none, so `saveAs` throws.
fn save_as(downloads: &Downloads, guid: &str) -> Result<Option<Value>, CallError> {
    downloads.get(guid)?;
    Err(PlaywrightError::error(NO_ARTIFACT_DIRECTORY).into())
}

/// Playwright: `server/artifact.ts` `Artifact._delete`: `const fileName = await this._localPathAfterFinished();`
///
/// Waits for the download to end and removes its file, which then fails a canceled download as
/// Playwright's does. The file is the one the browser wrote in the user's download directory;
/// Neo removes only it, and only when the script asks: nothing is removed at the run's end.
async fn delete(tabs: &Tabs, guid: &str, progress: &Progress) -> Result<Option<Value>, CallError> {
    let downloads = tabs.downloads();
    match progress.race(downloads.end(guid)).await?? {
        End::Canceled => Err(PlaywrightError::error(CANCELED).into()),
        End::Completed(path) => {
            if let Some(path) = path
                && let Err(error) = fs::remove_file(&path).await
            {
                debug!(%error, "a download's file could not be removed");
            }
            downloads.deleted(guid);
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(guid: &str, state: &str, file_path: Option<&str>) -> DownloadProgress {
        DownloadProgress {
            guid: guid.to_string(),
            state: state.to_string(),
            file_path: file_path.map(str::to_string),
        }
    }

    #[test]
    fn tests_that_reports_switch_on_with_events_and_off_to_the_browsers_own_behavior() {
        assert_eq!(
            download_behavior(true, Some("CONTEXT")),
            json!({ "behavior": "default", "eventsEnabled": true, "browserContextId": "CONTEXT" })
        );
        assert_eq!(
            download_behavior(false, None),
            json!({ "behavior": "default" })
        );
    }

    #[test]
    fn tests_that_the_reports_decode_what_browseros_sends() {
        let began: DownloadWillBegin = serde_json::from_value(json!({
            "frameId": "TAB", "guid": "G", "url": "http://a.test/x.txt",
            "suggestedFilename": "x.txt",
        }))
        .unwrap_or_else(|error| panic!("downloadWillBegin: {error}"));
        assert_eq!(
            (began.frame_id.as_str(), began.suggested_filename.as_str()),
            ("TAB", "x.txt")
        );
        let done: DownloadProgress = serde_json::from_value(json!({
            "guid": "G", "totalBytes": 11, "receivedBytes": 11, "state": "completed",
            "filePath": "/downloads/x.txt",
        }))
        .unwrap_or_else(|error| panic!("downloadProgress: {error}"));
        assert_eq!(done.file_path.as_deref(), Some("/downloads/x.txt"));
        let canceled: DownloadProgress = serde_json::from_value(json!({
            "guid": "G", "totalBytes": 2_000_000, "receivedBytes": 0, "state": "canceled",
        }))
        .unwrap_or_else(|error| panic!("downloadProgress: {error}"));
        assert_eq!(canceled.file_path, None);
    }

    #[tokio::test]
    async fn tests_that_a_download_ends_once_at_its_first_end() {
        let downloads = Downloads::default();
        downloads.began("G", Some("CONTEXT".to_string()));
        downloads.progressed(progress("G", "inProgress", None));
        let (context, end) = downloads
            .get("G")
            .unwrap_or_else(|error| panic!("kept: {error:?}"));
        assert_eq!(context.as_deref(), Some("CONTEXT"));
        assert_eq!(*end.borrow(), None, "inProgress ends nothing");
        downloads.progressed(progress("G", "completed", Some("/d/x.txt")));
        downloads.progressed(progress("G", "canceled", None));
        assert_eq!(
            downloads.end("G").await.ok(),
            Some(End::Completed(Some("/d/x.txt".to_string()))),
            "a cancel after completion changes nothing"
        );
        downloads.progressed(progress("UNKNOWN", "completed", None));
        assert!(
            downloads.get("UNKNOWN").is_err(),
            "a report of another download"
        );
    }

    fn target_event(
        method: &str,
        target: &str,
        kind: &str,
        url: &str,
        opener: Option<&str>,
    ) -> CdpEvent {
        CdpEvent {
            method: method.to_string(),
            params: json!({ "targetInfo": {
                "targetId": target, "type": kind, "url": url, "openerId": opener, "attached": false,
            }}),
            session_id: None,
        }
    }

    #[test]
    fn tests_that_the_watcher_remembers_each_tabs_opener_and_first_commit_past_its_end() {
        let mut opened = Openings::default();
        opened.on(&target_event(
            "Target.targetCreated",
            "NEW",
            "page",
            "",
            Some("RUN"),
        ));
        assert_eq!(
            opened.get("NEW"),
            Some(&Opened {
                opener: Some("RUN".to_string()),
                committed: false
            }),
            "a new window on its initial empty document"
        );
        opened.on(&CdpEvent {
            method: "Target.targetDestroyed".to_string(),
            params: json!({ "targetId": "NEW" }),
            session_id: None,
        });
        assert!(opened.get("NEW").is_some(), "kept past its end");
        opened.on(&target_event(
            "Target.targetCreated",
            "POPUP",
            "page",
            "",
            Some("RUN"),
        ));
        opened.on(&target_event(
            "Target.targetInfoChanged",
            "POPUP",
            "page",
            "http://a.test/",
            None,
        ));
        assert_eq!(
            opened.get("POPUP"),
            Some(&Opened {
                opener: Some("RUN".to_string()),
                committed: true
            }),
            "a commit keeps the opener"
        );
        opened.on(&target_event(
            "Target.targetCreated",
            "WORKER",
            "service_worker",
            "",
            None,
        ));
        assert!(opened.get("WORKER").is_none(), "no tab");
        for n in 0..MAX_OPENED {
            opened.on(&target_event(
                "Target.targetCreated",
                &format!("T{n}"),
                "page",
                "",
                None,
            ));
        }
        assert!(opened.get("NEW").is_none(), "the oldest goes past the cap");
    }

    #[test]
    fn tests_that_a_deleted_download_is_closed_and_an_old_one_gone() {
        let downloads = Downloads::default();
        downloads.began("DELETED", None);
        downloads.deleted("DELETED");
        assert!(
            downloads
                .get("DELETED")
                .is_err_and(|error| error.message == PlaywrightError::target_closed().message)
        );
        for n in 0..MAX_KEPT_DOWNLOADS {
            downloads.began(&format!("G{n}"), None);
        }
        assert!(
            downloads
                .get("DELETED")
                .is_err_and(|error| error.message == DOWNLOAD_GONE),
            "the oldest goes past the cap"
        );
        assert!(DOWNLOAD_GONE.contains(&MAX_KEPT_DOWNLOADS.to_string()));
        assert!(
            downloads
                .get(&format!("G{}", MAX_KEPT_DOWNLOADS - 1))
                .is_ok()
        );
    }
}
