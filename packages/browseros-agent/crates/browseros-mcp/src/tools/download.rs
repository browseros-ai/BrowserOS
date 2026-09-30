use crate::{
    constants::DOWNLOAD_TIMEOUT,
    framework::{
        ToolCtx, ToolError, ToolExecResult, ToolResult, parse_args, pending_dialog_result,
        text_result,
    },
    output_file::{create_download_output_dir, record_browser_output_file},
};
use browseros_core::{PageId, Ref, SessionId};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const DESCRIPTION: &str = "\
Click an element (by ref from the last snapshot) to trigger a file download, \
and save it to a BrowserOS output file. Returns the saved path and filename.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DownloadArgs {
    /// Page id from `tabs`.
    page: u32,
    /// Ref of the element that triggers the download, e.g. "e12".
    r#ref: String,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<DownloadArgs>("download", DESCRIPTION, None, handler)
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
        let page_session = ctx.session.pages.get_session(page_id.clone()).await?;
        let download_dir = create_download_output_dir().await?;
        page_session
            .session
            .send_value(
                "Page.setDownloadBehavior",
                json!({ "behavior": "allow", "downloadPath": chromium_download_path(&download_dir) }),
            )
            .await?;
        let capture = capture_download(
            ctx,
            page_id,
            page_session.session_id.clone(),
            &args.r#ref,
            &download_dir,
        )
        .await;
        let _ = page_session
            .session
            .send_value("Page.setDownloadBehavior", json!({ "behavior": "default" }))
            .await;
        let filename = capture?;
        let path = download_dir.join(&filename);
        record_browser_output_file(&ctx.output_files, path.clone()).await;
        Ok(Some(text_result(
            format!("Downloaded \"{filename}\" to: {}", path.display()),
            Some(json!({
                "page": args.page,
                "ref": args.r#ref,
                "path": path.to_string_lossy(),
                "filename": filename
            })),
        )))
    })
}

fn chromium_download_path(download_dir: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};

        let mut components = download_dir.components();
        let Some(Component::Prefix(prefix)) = components.next() else {
            return download_dir.to_path_buf();
        };
        // Chromium rejects the verbatim prefix added by Windows canonicalize.
        // Keep the original canonical path for filesystem access and tracking.
        let mut chromium_path = match prefix.kind() {
            Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:\\", char::from(drive))),
            Prefix::VerbatimUNC(server, share) => PathBuf::from(r"\\").join(server).join(share),
            _ => return download_dir.to_path_buf(),
        };
        for component in components {
            if component == Component::RootDir {
                continue;
            }
            chromium_path.push(component.as_os_str());
        }
        return chromium_path;
    }
    #[cfg(not(windows))]
    download_dir.to_path_buf()
}

#[cfg(test)]
mod download_path_tests {
    use super::chromium_download_path;
    use std::path::{Path, PathBuf};

    #[test]
    fn ordinary_paths_are_unchanged() {
        for path in ["/tmp/browseros/download", "relative/download", ""] {
            assert_eq!(chromium_download_path(Path::new(path)), PathBuf::from(path));
        }
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_drive_path_becomes_chromium_compatible() {
        for (canonical_path, chromium_path) in [
            (r"\\?\C:\", r"C:\"),
            (
                r"\\?\C:\Users\Test User\写真\download",
                r"C:\Users\Test User\写真\download",
            ),
            (r"\\?\d:\tool-output\download", r"D:\tool-output\download"),
        ] {
            assert_eq!(
                chromium_download_path(Path::new(canonical_path)),
                PathBuf::from(chromium_path),
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_unc_path_preserves_server_and_share() {
        assert_eq!(
            chromium_download_path(Path::new(r"\\?\UNC\server\shared photos\写真\download")),
            PathBuf::from(r"\\server\shared photos\写真\download"),
        );
    }

    #[cfg(windows)]
    #[test]
    fn ordinary_windows_and_device_paths_are_unchanged() {
        for path in [
            r"C:\Users\Test User\Downloads",
            r"\\server\shared photos\download",
            r"\\.\C:\download",
            r"\\?\Volume{00000000-0000-0000-0000-000000000000}\download",
        ] {
            assert_eq!(chromium_download_path(Path::new(path)), PathBuf::from(path));
        }
    }
}

async fn capture_download(
    ctx: &ToolCtx,
    page_id: PageId,
    session_id: SessionId,
    ref_id: &str,
    _download_dir: &PathBuf,
) -> ToolExecResult<String> {
    // Subscribe before clicking so synchronous download events cannot outrun the receiver.
    let mut events = ctx.session.cdp_events();
    let input = ctx.session.input(page_id).await;
    input
        .click(&Ref(ref_id.to_string()), Default::default())
        .await?;
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
                    "Page.downloadWillBegin" => {
                        guid = event.params.get("guid").and_then(Value::as_str).unwrap_or_default().to_string();
                        suggested_filename = event
                            .params
                            .get("suggestedFilename")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                    }
                    "Page.downloadProgress" => {
                        if event.params.get("guid").and_then(Value::as_str) != Some(guid.as_str()) {
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
