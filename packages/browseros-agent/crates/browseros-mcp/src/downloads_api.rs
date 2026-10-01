//! Reading the browser's own record of a download.
//!
//! The browser decides where a download goes and under what name, and it is the
//! only thing that knows: the protocol reports a suggested filename and progress
//! but never a path, the command line that would reveal the profile is withheld
//! unless the browser is launched for automation, and the folder it chooses is
//! not derived from this process's environment. Managing the download would make
//! the destination ours to know, and would also skip the target determination
//! that renames around a collision, so it replaces files instead.
//!
//! What does know is `chrome.downloads`, which reports an absolute path per
//! download. The cockpit extension already holds that permission and its service
//! worker is an attachable target, so the answer is reachable without a channel
//! of its own or any extension code.

use crate::framework::{ToolError, ToolExecResult};
use browseros_core::{BrowserSession, ProtocolSession, SessionId};
use serde::Deserialize;
use serde_json::{Value, json};

/// How many recent downloads to read. A download that just completed is the most
/// recent by start time, so a small window is enough and keeps a long history
/// from crossing the wire.
const RECENT_DOWNLOAD_LIMIT: u32 = 25;

/// A download as the browser recorded it.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DownloadRecord {
    pub id: i64,
    /// Absolute path, on the machine running the browser.
    pub filename: String,
    pub state: String,
    pub bytes: u64,
}

impl DownloadRecord {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.state == "complete"
    }
}

const READ_RECENT_DOWNLOADS: &str = r"
  (async () => {
    if (typeof chrome === 'undefined' || !chrome.downloads) {
      return { error: 'chrome.downloads is unavailable in this context' };
    }
    const items = await chrome.downloads.search({
      limit: LIMIT,
      orderBy: ['-startTime'],
    });
    return {
      records: items.map((item) => ({
        id: item.id,
        filename: item.filename ?? '',
        state: item.state ?? '',
        bytes: item.bytesReceived ?? 0,
      })),
    };
  })()
";

#[derive(Deserialize)]
struct ReadResult {
    #[serde(default)]
    records: Vec<DownloadRecord>,
    #[serde(default)]
    error: Option<String>,
}

/// The browser's most recent download records, newest first.
///
/// More than one extension can be running, and only the cockpit holds the
/// `downloads` permission, so each is asked in turn rather than assuming which
/// one is which. Identifying it by extension id instead would hard code a value
/// that differs per build.
pub async fn read_recent_downloads(
    session: &BrowserSession,
) -> ToolExecResult<Vec<DownloadRecord>> {
    let expression = READ_RECENT_DOWNLOADS.replace("LIMIT", &RECENT_DOWNLOAD_LIMIT.to_string());
    let root = ProtocolSession::root(session.connection());
    let targets = extension_worker_targets(&root).await?;
    if targets.is_empty() {
        return Err(no_extension_error());
    }
    let mut last_failure: Option<ToolError> = None;
    for target_id in targets {
        match evaluate_in_worker(session, &root, &target_id, &expression).await {
            Ok(value) => match serde_json::from_value::<ReadResult>(value) {
                // This extension cannot see downloads; another one may.
                Ok(result) if result.error.is_some() => continue,
                Ok(result) => return Ok(result.records),
                Err(err) => {
                    last_failure = Some(ToolError::message(format!(
                        "Could not read the browser's downloads: {err}"
                    )));
                }
            },
            Err(err) => last_failure = Some(err),
        }
    }
    Err(last_failure.unwrap_or_else(no_extension_error))
}

fn no_extension_error() -> ToolError {
    ToolError::message(
        "No browser extension that can report downloads is running, so where the file was saved cannot be read.",
    )
}

/// Evaluates in one extension service worker.
///
/// Attaching wakes a sleeping worker, and the session is released afterwards so
/// a download does not leave one attached.
async fn evaluate_in_worker(
    session: &BrowserSession,
    root: &ProtocolSession,
    target_id: &str,
    expression: &str,
) -> ToolExecResult<Value> {
    let attached: Value = root
        .send(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
        )
        .await?;
    let session_id = attached
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ToolError::message("Could not attach to the browser's extension to read downloads")
        })?
        .to_string();
    let worker =
        ProtocolSession::for_session(session.connection(), SessionId::from(session_id.clone()));
    let evaluated: Result<Value, _> = worker
        .send(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "awaitPromise": true,
                "returnByValue": true,
            }),
        )
        .await;
    // Released before the result is unwrapped, so a failed evaluate does not
    // leave a session attached to the worker.
    let _: Result<Value, _> = root
        .send(
            "Target.detachFromTarget",
            json!({ "sessionId": session_id }),
        )
        .await;
    let evaluated = evaluated?;
    if let Some(details) = evaluated.get("exceptionDetails") {
        return Err(ToolError::message(format!(
            "Reading the browser's downloads failed: {details}"
        )));
    }
    evaluated
        .get("result")
        .and_then(|result| result.get("value"))
        .cloned()
        .ok_or_else(|| ToolError::message("Reading the browser's downloads returned no value"))
}

/// Every extension service worker currently running.
async fn extension_worker_targets(root: &ProtocolSession) -> ToolExecResult<Vec<String>> {
    let targets: Value = root.send("Target.getTargets", json!({})).await?;
    Ok(targets
        .get("targetInfos")
        .and_then(Value::as_array)
        .map(|infos| {
            infos
                .iter()
                .filter(|info| is_extension_worker(info))
                .filter_map(|info| info.get("targetId").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default())
}

fn is_extension_worker(info: &Value) -> bool {
    info.get("type").and_then(Value::as_str) == Some("service_worker")
        && info
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|url| url.starts_with("chrome-extension://"))
}

/// The one record a download added, by id.
///
/// Compared against the ids that existed before the click rather than matched on
/// url or name: the id is the browser's own identity for the record, and two
/// downloads of one url would not be told apart by anything else. Several or none
/// is an error, because naming the wrong file is worse than saying so.
pub fn newly_added<'a>(
    before: &[i64],
    after: &'a [DownloadRecord],
    suggested: &str,
) -> ToolExecResult<&'a DownloadRecord> {
    let added = after
        .iter()
        .filter(|record| !before.contains(&record.id))
        .collect::<Vec<_>>();
    let complete = added
        .iter()
        .filter(|record| record.is_complete())
        .collect::<Vec<_>>();
    match complete.as_slice() {
        [only] => Ok(**only),
        [] if added.is_empty() => Err(ToolError::message(format!(
            "Download of \"{suggested}\" completed, but the browser recorded no new download"
        ))),
        [] => Err(ToolError::message(format!(
            "Download of \"{suggested}\" completed, but the browser has not finished writing it"
        ))),
        _ => Err(ToolError::message(format!(
            "Download of \"{suggested}\" completed, but {} downloads finished at once so the file cannot be named",
            complete.len()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: i64, filename: &str, state: &str) -> DownloadRecord {
        DownloadRecord {
            id,
            filename: filename.to_string(),
            state: state.to_string(),
            bytes: 10,
        }
    }

    #[test]
    fn names_the_one_download_that_appeared() {
        let before = vec![1, 2];
        let after = vec![
            record(3, "/home/u/Downloads/report.pdf", "complete"),
            record(2, "/home/u/Downloads/older.pdf", "complete"),
        ];

        let found = newly_added(&before, &after, "report.pdf")
            .unwrap_or_else(|err| panic!("the new record should be found: {err}"));

        assert_eq!(found.filename, "/home/u/Downloads/report.pdf");
    }

    #[test]
    fn takes_the_browsers_name_even_when_it_renamed_around_a_collision() {
        // The browser is the only thing that knows it renamed, which is the whole
        // reason the path comes from its record rather than from the suggestion.
        let after = vec![record(9, "/home/u/Downloads/report (1).pdf", "complete")];

        let found = newly_added(&[8], &after, "report.pdf")
            .unwrap_or_else(|err| panic!("the renamed record should be found: {err}"));

        assert_eq!(found.filename, "/home/u/Downloads/report (1).pdf");
    }

    #[test]
    fn refuses_when_the_browser_recorded_nothing_new() {
        let Err(error) = newly_added(&[1, 2], &[record(2, "/x/old.pdf", "complete")], "new.pdf")
        else {
            panic!("no new record cannot name a download");
        };

        assert!(error.to_string().contains("recorded no new download"));
    }

    #[test]
    fn refuses_while_the_new_download_is_still_writing() {
        let Err(error) = newly_added(
            &[1],
            &[record(2, "/x/partial.pdf", "in_progress")],
            "partial.pdf",
        ) else {
            panic!("an unfinished record cannot name a completed file");
        };

        assert!(error.to_string().contains("not finished writing"));
    }

    #[test]
    fn refuses_when_two_downloads_finished_at_once() {
        let after = vec![
            record(3, "/x/a.pdf", "complete"),
            record(4, "/x/b.pdf", "complete"),
        ];

        let Err(error) = newly_added(&[1], &after, "a.pdf") else {
            panic!("two finished downloads cannot identify one file");
        };

        assert!(error.to_string().contains("2 downloads finished at once"));
    }

    #[test]
    fn recognises_only_an_extension_service_worker() {
        assert!(is_extension_worker(&json!({
            "type": "service_worker",
            "url": "chrome-extension://abc/background.js"
        })));
        assert!(!is_extension_worker(&json!({
            "type": "page",
            "url": "chrome-extension://abc/newtab.html"
        })));
        assert!(!is_extension_worker(&json!({
            "type": "service_worker",
            "url": "https://example.com/sw.js"
        })));
    }
}
