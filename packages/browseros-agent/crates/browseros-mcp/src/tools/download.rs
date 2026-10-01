use crate::{
    constants::DOWNLOAD_TIMEOUT,
    framework::{
        ToolCtx, ToolError, ToolExecResult, ToolResult, parse_args, pending_dialog_result,
        text_result,
    },
    output_file::{
        create_download_staging_dir, ensure_download_dir_inside_home, free_name_in,
        get_user_download_dir, prepare_download_dir, read_download_dir_names,
        record_browser_output_file, validate_download_dir,
    },
};
use browseros_core::{PageId, Ref, SessionId};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

const DESCRIPTION: &str = "\
Click an element (by ref from the last snapshot) to trigger a file download. \
Saves to the browser's download folder, so it also appears in the browser's \
downloads list, and an existing file of the same name is renamed around rather \
than replaced. Returns the completed file's path on the machine running this \
browser, its name on disk, and its size.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DownloadArgs {
    /// Page id from `tabs`.
    page: u32,
    /// Ref of the element that triggers the download, e.g. "e12".
    r#ref: String,
    /// Absolute directory to save into, for an agent that needs the file inside
    /// its own working directory. Must be inside your home directory.
    /// Defaults to the browser's download folder.
    dir: Option<String>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<DownloadArgs>("download", DESCRIPTION, None, handler)
}

/// One lock per page, so two downloads on a page cannot cross.
///
/// `Page.setDownloadBehavior` is page-wide state that a call sets, clicks on,
/// waits for, then restores. Two overlapping calls on one page would otherwise
/// interfere both ways: the second's override redirects the first's download,
/// and the second's restore drops the override while the first is still waiting.
/// Keyed by page because the override is per page, and held here rather than on
/// the session because the page ids come from the one browser a server drives.
static PAGE_DOWNLOAD_LOCKS: LazyLock<Mutex<HashMap<PageId, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn page_download_lock(page: &PageId) -> Arc<tokio::sync::Mutex<()>> {
    let mut locks = PAGE_DOWNLOAD_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(locks.entry(page.clone()).or_default())
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: DownloadArgs = parse_args(raw)?;
        let page_id = PageId(args.page);
        if let Some(result) = pending_dialog_result(ctx, page_id.clone()) {
            return Ok(Some(result));
        }
        let requested = match args.dir.as_deref() {
            Some(dir) => validate_download_dir(dir).map_err(ToolError::message)?,
            None => get_user_download_dir().await?,
        };
        // Before creating anything, so a refused destination leaves no
        // directories behind.
        if args.dir.is_some() {
            ensure_download_dir_inside_home(&requested)
                .await
                .map_err(ToolError::message)?;
        }
        let download_dir = prepare_download_dir(requested).await?;
        // Again on the created path: a symlink could have appeared in between.
        if args.dir.is_some() {
            ensure_download_dir_inside_home(&download_dir)
                .await
                .map_err(ToolError::message)?;
        }
        // Held across the override, the click and the restore.
        let page_lock = page_download_lock(&page_id);
        let _page_guard = page_lock.lock().await;
        let staging = create_download_staging_dir(&download_dir).await?;
        let page_session = ctx.session.pages.get_session(page_id.clone()).await?;
        page_session
            .session
            .send_value(
                "Page.setDownloadBehavior",
                json!({ "behavior": "allow", "downloadPath": staging }),
            )
            .await?;
        let capture =
            capture_download(ctx, page_id, page_session.session_id.clone(), &args.r#ref).await;
        let _ = page_session
            .session
            .send_value("Page.setDownloadBehavior", json!({ "behavior": "default" }))
            .await;
        let settled = match capture {
            Ok(suggested) => settle_download(&staging, &download_dir, &suggested).await,
            Err(err) => Err(err),
        };
        // The staging directory goes either way, so a failure leaves nothing behind.
        let _ = tokio::fs::remove_dir_all(&staging).await;
        let (filename, path, bytes) = settled?;
        record_browser_output_file(&ctx.output_files, path.clone()).await;
        Ok(Some(text_result(
            format!(
                "Downloaded \"{filename}\" ({bytes} bytes) on the machine running this browser, to: {}",
                path.display()
            ),
            Some(json!({
                "page": args.page,
                "ref": args.r#ref,
                "path": path.to_string_lossy(),
                "filename": filename,
                "bytes": bytes,
                "dir": download_dir.to_string_lossy()
            })),
        )))
    })
}

/// Moves the staged download into its destination and reports what landed.
///
/// The staging directory holds exactly one file, so the name is observed rather
/// than reconstructed from the suggestion, and the destination name is chosen to
/// be free so an existing file is never replaced.
async fn settle_download(
    staging: &Path,
    destination: &Path,
    suggested: &str,
) -> ToolExecResult<(String, PathBuf, u64)> {
    let staged = staged_file(staging, suggested).await?;
    let filename = free_name_in(destination, &staged).await?;
    let path = destination.join(&filename);
    tokio::fs::rename(staging.join(&staged), &path).await?;
    let bytes = tokio::fs::metadata(&path).await?.len();
    Ok((filename, path, bytes))
}

/// The single file the download wrote into its own directory.
///
/// Partial downloads are skipped so a `.crdownload` seen mid-write is never
/// mistaken for the finished file.
async fn staged_file(staging: &Path, suggested: &str) -> ToolExecResult<String> {
    let names = read_download_dir_names(staging).await?;
    let mut names = names.into_iter().collect::<Vec<_>>();
    names.sort_unstable();
    match names.as_slice() {
        [only] => Ok(only.clone()),
        // Nothing, or a click that started more than one download. Either way the
        // file this call should report cannot be named, and naming the wrong one
        // is worse than saying so.
        _ => Err(ToolError::message(format!(
            "Download of \"{suggested}\" completed, but {} files landed instead of one",
            names.len()
        ))),
    }
}

async fn capture_download(
    ctx: &ToolCtx,
    page_id: PageId,
    session_id: SessionId,
    ref_id: &str,
) -> ToolExecResult<String> {
    // Subscribe before clicking so synchronous download events cannot outrun the receiver.
    let mut events = ctx.session.cdp_events();
    let input = ctx.session.input(page_id).await;
    input
        .click(&Ref(ref_id.to_string()), Default::default())
        .await?;
    let mut tracking = false;
    let mut guid = String::new();
    let mut suggested_filename = String::new();
    let timeout = tokio::time::sleep(DOWNLOAD_TIMEOUT);
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            () = ctx.cancel.cancelled() => return Err(ToolError::Cancelled),
            () = &mut timeout => {
                return Err(ToolError::message(format!(
                    "Download timed out after {}ms",
                    DOWNLOAD_TIMEOUT.as_millis()
                )));
            }
            event = events.recv() => {
                let event = event.map_err(|err| ToolError::message(err.to_string()))?;
                if event.session_id.as_ref() != Some(&session_id) {
                    continue;
                }
                match event.method.as_str() {
                    // Only the first download of the click: a later one would
                    // otherwise take over the guid being tracked and the
                    // completion of this one would never be seen.
                    "Page.downloadWillBegin" if !tracking => {
                        tracking = true;
                        guid = event.params.get("guid").and_then(Value::as_str).unwrap_or_default().to_string();
                        suggested_filename = event
                            .params
                            .get("suggestedFilename")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                    }
                    "Page.downloadProgress" => {
                        if !tracking || event.params.get("guid").and_then(Value::as_str) != Some(guid.as_str()) {
                            continue;
                        }
                        match event.params.get("state").and_then(Value::as_str) {
                            Some("completed") => {
                                if suggested_filename.is_empty() {
                                    return Err(ToolError::message("Download completed without suggested filename"));
                                }
                                return Ok(suggested_filename);
                            }
                            Some("canceled") => return Err(ToolError::message("Download was canceled")),
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("browseros-download-{}", uuid::Uuid::new_v4()));
        if let Err(err) = std::fs::create_dir_all(&dir) {
            panic!("test scratch directory should be creatable: {err}");
        }
        dir
    }

    async fn write_file(dir: &Path, name: &str, body: &str) {
        if let Err(err) = tokio::fs::write(dir.join(name), body).await {
            panic!("test file {name} should be writable: {err}");
        }
    }

    async fn read_file(path: &Path) -> String {
        match tokio::fs::read_to_string(path).await {
            Ok(body) => body,
            Err(err) => panic!("{} should be readable: {err}", path.display()),
        }
    }

    #[tokio::test]
    async fn moves_the_staged_download_into_place() {
        let destination = scratch_dir();
        let staging = destination.join("staging");
        if let Err(err) = tokio::fs::create_dir(&staging).await {
            panic!("staging directory should be creatable: {err}");
        }
        write_file(&staging, "report.pdf", "the download").await;

        let (filename, path, bytes) = settle_download(&staging, &destination, "report.pdf")
            .await
            .unwrap_or_else(|err| panic!("the staged file should settle: {err}"));

        assert_eq!(filename, "report.pdf");
        assert_eq!(path, destination.join("report.pdf"));
        assert_eq!(bytes, "the download".len() as u64);
        assert_eq!(read_file(&path).await, "the download");
    }

    #[tokio::test]
    async fn renames_around_an_existing_file_instead_of_replacing_it() {
        // The whole reason the download goes through a staging directory: writing
        // straight into the destination replaces what is already there.
        let destination = scratch_dir();
        write_file(&destination, "report.pdf", "the user's own file").await;
        let staging = destination.join("staging");
        if let Err(err) = tokio::fs::create_dir(&staging).await {
            panic!("staging directory should be creatable: {err}");
        }
        write_file(&staging, "report.pdf", "the download").await;

        let (filename, path, _) = settle_download(&staging, &destination, "report.pdf")
            .await
            .unwrap_or_else(|err| panic!("the staged file should settle: {err}"));

        assert_eq!(filename, "report (1).pdf");
        assert_eq!(read_file(&path).await, "the download");
        assert_eq!(
            read_file(&destination.join("report.pdf")).await,
            "the user's own file",
            "the file that was already there must survive"
        );
    }

    #[tokio::test]
    async fn names_the_single_staged_file() {
        let staging = scratch_dir();
        write_file(&staging, "report.pdf", "body").await;

        let name = staged_file(&staging, "report.pdf")
            .await
            .unwrap_or_else(|err| panic!("the one staged file should be named: {err}"));

        assert_eq!(name, "report.pdf");
    }

    #[tokio::test]
    async fn ignores_a_partial_download_still_being_written() {
        let staging = scratch_dir();
        write_file(&staging, "report.pdf.crdownload", "partial").await;
        write_file(&staging, "report.pdf", "done").await;

        let name = staged_file(&staging, "report.pdf")
            .await
            .unwrap_or_else(|err| panic!("the finished file should be named: {err}"));

        assert_eq!(name, "report.pdf");
    }

    #[tokio::test]
    async fn refuses_when_nothing_was_staged() {
        let staging = scratch_dir();

        let Err(error) = staged_file(&staging, "report.pdf").await else {
            panic!("an empty staging directory cannot name a download");
        };

        assert!(error.to_string().contains("0 files landed"));
    }

    #[tokio::test]
    async fn refuses_when_one_click_staged_several_files() {
        // Naming one of them would report an artifact the caller did not ask for.
        let staging = scratch_dir();
        write_file(&staging, "one.pdf", "a").await;
        write_file(&staging, "two.pdf", "b").await;

        let Err(error) = staged_file(&staging, "one.pdf").await else {
            panic!("two staged files cannot identify the download");
        };

        assert!(error.to_string().contains("2 files landed"));
    }
}
