use crate::{
    constants::DOWNLOAD_TIMEOUT,
    framework::{
        ToolCtx, ToolError, ToolExecResult, ToolResult, parse_args, pending_dialog_result,
        text_result,
    },
    output_file::{
        ensure_download_dir_inside_home, get_user_download_dir, prepare_download_dir,
        read_download_dir_names, record_browser_output_file, validate_download_dir,
    },
};
use browseros_core::{PageId, Ref, SessionId};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, LazyLock, Mutex},
};

const DESCRIPTION: &str = "\
Click an element (by ref from the last snapshot) to trigger a file download. \
Saves to the browser's download folder, so it also appears in the browser's \
downloads list. Returns the completed file's path on the machine running this \
browser, its name on disk, and its size.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DownloadArgs {
    /// Page id from `tabs`.
    page: u32,
    /// Ref of the element that triggers the download, e.g. "e12".
    r#ref: String,
    /// Absolute directory to save into, for an agent that needs the file inside
    /// its own working directory. Defaults to the browser's download folder.
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
        // Held across the whole override, click and restore.
        let page_lock = page_download_lock(&page_id);
        let _page_guard = page_lock.lock().await;
        // Taken before the click, and inside the lock, so the entry the download
        // adds can be told apart from what was already there.
        let before = read_download_dir_names(&download_dir).await?;
        let page_session = ctx.session.pages.get_session(page_id.clone()).await?;
        // Still managed rather than left to the browser's own behaviour: with
        // "ask where to save each file" turned on, an unmanaged download opens a
        // save dialog and blocks behind a modal nobody is there to answer.
        page_session
            .session
            .send_value(
                "Page.setDownloadBehavior",
                json!({ "behavior": "allow", "downloadPath": download_dir }),
            )
            .await?;
        let capture =
            capture_download(ctx, page_id, page_session.session_id.clone(), &args.r#ref).await;
        let _ = page_session
            .session
            .send_value("Page.setDownloadBehavior", json!({ "behavior": "default" }))
            .await;
        let suggested = capture?;
        let filename = resolve_downloaded_name(&download_dir, &before, &suggested).await?;
        let path = download_dir.join(&filename);
        let bytes = tokio::fs::metadata(&path).await?.len();
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

/// The name the download actually landed under.
///
/// Not the suggested name: sharing a directory with the user's own files means
/// Chromium's uniquifier is routinely in play, so the file on disk is `name.ext`
/// or `name (1).ext` and reconstructing the path from the suggestion names
/// whichever file was already there.
async fn resolve_downloaded_name(
    dir: &Path,
    before: &HashSet<String>,
    suggested: &str,
) -> ToolExecResult<String> {
    let after = read_download_dir_names(dir).await?;
    let added = after
        .difference(before)
        .map(String::as_str)
        .collect::<Vec<_>>();
    // Every new name this download could plausibly have taken. The suggested
    // name is not treated as proof on its own: an earlier download of the same
    // name finishing during this call also appears as new, because a partial
    // file is skipped by the scan above, and it would have taken the plain name
    // while this one took the rename.
    let candidates = added
        .iter()
        .filter(|name| **name == suggested || is_uniquified(name, suggested))
        .collect::<Vec<_>>();
    if let [name] = candidates.as_slice() {
        return Ok((**name).to_string());
    }
    if candidates.is_empty()
        && let [name] = added.as_slice()
    {
        return Ok((*name).to_string());
    }
    // Deliberately an error rather than a guess. Several candidates means
    // something else wrote here in the same window, and naming the wrong file is
    // worse than saying the name could not be established.
    Err(ToolError::message(format!(
        "Download of \"{suggested}\" completed, but its name in {} could not be established ({} new files, {} of them possible matches)",
        dir.display(),
        added.len(),
        candidates.len()
    )))
}

/// Whether `name` is Chromium's collision rename of `suggested`, e.g. "a (1).pdf".
///
/// Chromium inserts " (N)" before the extension, and what counts as the
/// extension is its own rule: "archive.tar.gz" becomes "archive (1).tar.gz", not
/// "archive.tar (1).gz". Rather than model that, take the counter back out and
/// see whether the suggested name is what remains.
fn is_uniquified(name: &str, suggested: &str) -> bool {
    let Some(open) = name.rfind(" (") else {
        return false;
    };
    let after_open = &name[open + 2..];
    let Some(close) = after_open.find(')') else {
        return false;
    };
    let counter = &after_open[..close];
    if counter.is_empty() || !counter.chars().all(|char| char.is_ascii_digit()) {
        return false;
    }
    let mut without_counter = String::with_capacity(name.len());
    without_counter.push_str(&name[..open]);
    without_counter.push_str(&after_open[close + 1..]);
    without_counter == suggested
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
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn names(values: &[&str]) -> HashSet<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("browseros-download-{}", uuid::Uuid::new_v4()));
        if let Err(err) = std::fs::create_dir_all(&dir) {
            panic!("test scratch directory should be creatable: {err}");
        }
        dir
    }

    async fn write_file(dir: &Path, name: &str) {
        if let Err(err) = tokio::fs::write(dir.join(name), b"body").await {
            panic!("test file {name} should be writable: {err}");
        }
    }

    async fn names_on_disk(dir: &Path) -> HashSet<String> {
        read_download_dir_names(dir)
            .await
            .unwrap_or_else(|err| panic!("scratch directory should be readable: {err}"))
    }

    async fn resolve(dir: &Path, before: &HashSet<String>, suggested: &str) -> String {
        resolve_downloaded_name(dir, before, suggested)
            .await
            .unwrap_or_else(|err| panic!("download name should resolve: {err}"))
    }

    #[test]
    fn recognises_chromiums_collision_rename() {
        assert!(is_uniquified("proposal (1).pdf", "proposal.pdf"));
        assert!(is_uniquified("proposal (12).pdf", "proposal.pdf"));
        // Verified against the browser: a compound extension stays whole, so the
        // counter goes before ".tar.gz" rather than before ".gz".
        assert!(is_uniquified("archive (3).tar.gz", "archive.tar.gz"));
        assert!(is_uniquified("notes (1)", "notes"));
        // A suggested name that already contains a parenthesised group.
        assert!(is_uniquified(
            "report (final) (1).pdf",
            "report (final).pdf"
        ));
    }

    #[test]
    fn does_not_mistake_a_different_file_for_a_rename() {
        assert!(!is_uniquified("proposal.pdf", "proposal.pdf"));
        assert!(!is_uniquified("other.pdf", "proposal.pdf"));
        assert!(!is_uniquified("proposal ().pdf", "proposal.pdf"));
        assert!(!is_uniquified("proposal (x).pdf", "proposal.pdf"));
        // A real file someone could plausibly have downloaded themselves.
        assert!(!is_uniquified("proposal v2.pdf", "proposal.pdf"));
        // Same counter, different extension: not this download.
        assert!(!is_uniquified("proposal (1).pdf", "proposal.txt"));
    }

    #[tokio::test]
    async fn names_the_file_the_download_actually_added() {
        let dir = scratch_dir();
        write_file(&dir, "already-here.pdf").await;
        let before = names(&["already-here.pdf"]);
        write_file(&dir, "proposal.pdf").await;

        assert_eq!(resolve(&dir, &before, "proposal.pdf").await, "proposal.pdf");
    }

    #[tokio::test]
    async fn follows_the_rename_when_the_name_was_already_taken() {
        // The case the reconstructed path got wrong: it named the file that was
        // already there instead of the one just downloaded.
        let dir = scratch_dir();
        write_file(&dir, "proposal.pdf").await;
        let before = names_on_disk(&dir).await;
        write_file(&dir, "proposal (1).pdf").await;

        assert_eq!(
            resolve(&dir, &before, "proposal.pdf").await,
            "proposal (1).pdf"
        );
    }

    #[tokio::test]
    async fn ignores_a_partial_download_still_being_written() {
        let dir = scratch_dir();
        let before = names_on_disk(&dir).await;
        write_file(&dir, "report.pdf.crdownload").await;
        write_file(&dir, "report.pdf").await;

        assert_eq!(resolve(&dir, &before, "report.pdf").await, "report.pdf");
    }

    #[tokio::test]
    async fn refuses_when_an_earlier_download_of_the_same_name_also_finished() {
        // A partial file is skipped by the scan, so when it completes during this
        // call its finished name also looks new. If it took the plain name and
        // this download took the rename, trusting the suggested name would return
        // the other download's file.
        let dir = scratch_dir();
        write_file(&dir, "report.pdf.crdownload").await;
        let before = names_on_disk(&dir).await;
        write_file(&dir, "report.pdf").await;
        write_file(&dir, "report (1).pdf").await;

        let Err(error) = resolve_downloaded_name(&dir, &before, "report.pdf").await else {
            panic!("two plausible names cannot identify which file this download wrote");
        };

        assert!(error.to_string().contains("could not be established"));
    }

    #[tokio::test]
    async fn still_resolves_when_an_unrelated_download_lands_alongside() {
        // Only one new name could be this download, so the other is irrelevant.
        let dir = scratch_dir();
        let before = names_on_disk(&dir).await;
        write_file(&dir, "report.pdf").await;
        write_file(&dir, "something-else.zip").await;

        assert_eq!(resolve(&dir, &before, "report.pdf").await, "report.pdf");
    }

    #[tokio::test]
    async fn refuses_to_guess_between_several_new_files() {
        // Someone downloaded something else in the same window. Naming the wrong
        // artifact is worse than saying the name could not be established.
        let dir = scratch_dir();
        let before = names_on_disk(&dir).await;
        write_file(&dir, "unrelated-a.pdf").await;
        write_file(&dir, "unrelated-b.pdf").await;

        let Err(error) = resolve_downloaded_name(&dir, &before, "proposal.pdf").await else {
            panic!("two unrelated new files cannot identify the download");
        };

        assert!(error.to_string().contains("could not be established"));
    }
}
