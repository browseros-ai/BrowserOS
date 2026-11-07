//! Playwright: `server/dialog.ts` `Dialog`
//!
//! One tab's JavaScript dialogs for one run: each `Page.javascriptDialogOpening` the tab's
//! watcher sees gets a record and a guid (`dialog@<targetId>:<n>`), and the script's
//! `Dialog.accept` and `Dialog.dismiss` answer it with `Page.handleJavaScriptDialog` on the
//! tab's session. A dialog is answered once: a second answer rejects as Playwright's does.
//!
//! Who answers: with no `dialog` listener the watcher does at once (accept `beforeunload`,
//! dismiss the rest, Playwright's no-listener default); with one, the script does, or the SDK
//! gives that default when no listener was left by the time the event ran. A dialog the script
//! was handed and never answered is answered with the default when the run ends (Deviation on
//! [`Dialogs::left_open`]).

use std::collections::HashSet;
use std::sync::Mutex;

use browseros_cdp::CdpError;
use neo_runtime_spec::serve::{DialogCall, DialogInitializer, PageChannel};
use serde::Serialize;
use serde_json::{Value, json};

use crate::click::PageError;
use crate::page::{DialogAnswer, PageSession};
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::world;

/// Neo: the `dialog@` guid prefix; the rest is `<targetId>:<n>`, minted by the run.
const DIALOG_PREFIX: &str = "dialog@";

/// Playwright: `server/dialog.ts` `Dialog._accept`: `assert(!this._handled, 'Cannot accept dialog which is already handled!');`
const ACCEPT_HANDLED: &str = "Cannot accept dialog which is already handled!";

/// Playwright: `server/dialog.ts` `Dialog._dismiss`: `assert(!this._handled, 'Cannot dismiss dialog which is already handled!');`
const DISMISS_HANDLED: &str = "Cannot dismiss dialog which is already handled!";

/// Neo: a poisoned lock (a panic elsewhere while holding it), which no script can cause.
const DIALOGS_UNREADABLE: &str = "neo-runtime: the tab's dialogs cannot be read";

/// The guid of dialog `index` on tab `target`.
pub(crate) fn guid(target: &str, index: u64) -> String {
    format!("{DIALOG_PREFIX}{target}:{index}")
}

/// The tab and index a dialog guid names, or `None` for another guid.
pub(crate) fn parse(guid: &str) -> Option<(&str, u64)> {
    let (target, index) = guid.strip_prefix(DIALOG_PREFIX)?.rsplit_once(':')?;
    if target.is_empty() || target.contains('/') {
        return None;
    }
    Some((target, index.parse().ok()?))
}

/// A dialog as the realm receives it: Playwright's `{guid, initializer}` record, which the SDK
/// turns into one `Dialog` object per guid.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct DialogRecord {
    pub(crate) guid: String,
    pub(crate) initializer: DialogInitializer,
}

/// One tab's dialogs in this run.
#[derive(Default)]
pub(crate) struct Dialogs(Mutex<State>);

#[derive(Default)]
struct State {
    next: u64,
    /// The dialog showing now, from its opening event to its closing one.
    open: Option<Open>,
    /// Dialogs that have had their one answer, from the script or a default.
    handled: HashSet<u64>,
}

#[derive(Clone)]
struct Open {
    index: u64,
    record: DialogRecord,
    /// Handed to the script, which owes the answer.
    emitted: bool,
}

impl Dialogs {
    /// Records a dialog the tab opened, as its watcher saw it, and returns its record.
    pub(crate) fn opened(
        &self,
        target: &str,
        page_guid: &str,
        kind: &str,
        message: &str,
        default_value: &str,
    ) -> Option<(u64, DialogRecord)> {
        let mut state = self.0.lock().ok()?;
        state.next += 1;
        let index = state.next;
        let record = DialogRecord {
            guid: guid(target, index),
            initializer: DialogInitializer {
                page: Some(PageChannel {
                    guid: page_guid.to_string(),
                }),
                r#type: kind.to_string(),
                message: message.to_string(),
                default_value: default_value.to_string(),
            },
        };
        state.open = Some(Open {
            index,
            record: record.clone(),
            emitted: false,
        });
        Some((index, record))
    }

    /// Marks dialog `index` as handed to the script, which now owes its answer.
    pub(crate) fn emitted(&self, index: u64) {
        if let Ok(mut state) = self.0.lock()
            && let Some(open) = state.open.as_mut().filter(|open| open.index == index)
        {
            open.emitted = true;
        }
    }

    /// Records the answer the watcher gives itself, so a late script answer rejects.
    pub(crate) fn answered_by_default(&self, index: u64) {
        if let Ok(mut state) = self.0.lock() {
            state.handled.insert(index);
        }
    }

    /// `Page.javascriptDialogClosed`: the dialog showing closed, answered by anyone. Returns its
    /// record for the `dialogClosed` event.
    pub(crate) fn closed(&self) -> Option<DialogRecord> {
        self.0.lock().ok()?.open.take().map(|open| open.record)
    }

    /// The watcher lost events: whatever dialog was showing may have closed unseen.
    pub(crate) fn forget(&self) {
        if let Ok(mut state) = self.0.lock()
            && let Some(open) = state.open.take()
        {
            state.handled.insert(open.index);
        }
    }

    /// Takes the one answer of dialog `index` for the script, or refuses a second one with
    /// Playwright's error.
    fn take_answer(&self, index: u64, accept: bool) -> Result<(), PlaywrightError> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| PlaywrightError::error(DIALOGS_UNREADABLE))?;
        if !state.handled.insert(index) {
            return Err(PlaywrightError::error(if accept {
                ACCEPT_HANDLED
            } else {
                DISMISS_HANDLED
            }));
        }
        Ok(())
    }

    /// Deviation: `server/dialog.ts` `DialogManager.dialogDidOpen`: `dialog._close().catch(() => {});`
    /// Playwright answers a dialog only when nobody listens, and leaves a listener's unanswered
    /// dialog open for the page's life. Neo's tab outlives the run and must not stay frozen for
    /// its user, so the run's end gives a dialog it handed out and nobody answered the default
    /// answer, which this takes.
    pub(crate) fn left_open(&self) -> Option<DialogAnswer> {
        let mut state = self.0.lock().ok()?;
        let open = state.open.clone().filter(|open| open.emitted)?;
        if !state.handled.insert(open.index) {
            return None;
        }
        Some(default_answer(&open.record.initializer.r#type))
    }
}

/// Playwright: `server/dialog.ts` `Dialog._close`: `if (this._type === 'beforeunload')`
///
/// Accept `beforeunload`, dismiss every other dialog.
pub(crate) fn default_answer(kind: &str) -> DialogAnswer {
    if kind == "beforeunload" {
        DialogAnswer::Accept
    } else {
        DialogAnswer::Dismiss
    }
}

/// `Dialog.accept` and `Dialog.dismiss` on dialog `index` of `page`'s tab.
///
/// # Errors
///
/// Playwright's error for a second answer; `TargetClosedError` once the tab is gone; the
/// browser's error otherwise ("No dialog is showing" for one the page already closed).
pub(crate) async fn serve(
    page: &PageSession,
    index: u64,
    call: DialogCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let (accept, prompt_text) = match call {
        DialogCall::Accept(params) => (true, params.prompt_text),
        DialogCall::Dismiss(_) => (false, None),
    };
    page.events.dialogs.take_answer(index, accept)?;
    let mut params = json!({ "accept": accept });
    if let Some(text) = prompt_text {
        params["promptText"] = json!(text);
    }
    progress
        .race(page.send("Page.handleJavaScriptDialog", params))
        .await?
        .map_err(|error| answer_error(&error))?;
    Ok(None)
}

/// Playwright: `server/protocolError.ts` `ProtocolError.setMessage`: `Protocol error (${this.method}): ${message}`
fn answer_error(error: &CdpError) -> PlaywrightError {
    match error {
        CdpError::Protocol { message, .. } => PlaywrightError::error(format!(
            "Protocol error (Page.handleJavaScriptDialog): {message}"
        )),
        error if world::classify(error) == PageError::Closed => PlaywrightError::target_closed(),
        error => PlaywrightError::error(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tests_that_dialog_guids_round_trip() {
        let guid = guid("ABC", 3);
        assert_eq!(guid, "dialog@ABC:3");
        assert_eq!(parse(&guid), Some(("ABC", 3)));
        assert_eq!(parse("dialog@ABC"), None);
        assert_eq!(parse("dialog@:1"), None);
        assert_eq!(parse("page@ABC"), None);
    }

    #[test]
    fn tests_that_a_dialog_takes_one_answer() {
        let dialogs = Dialogs::default();
        let (index, record) = dialogs
            .opened("T", "page@T", "prompt", "Name?", "anon")
            .unwrap();
        assert_eq!(record.guid, "dialog@T:1");
        assert_eq!(record.initializer.default_value, "anon");
        dialogs.take_answer(index, true).unwrap();
        let second = dialogs.take_answer(index, false).unwrap_err();
        assert_eq!(second.message, DISMISS_HANDLED);
    }

    #[test]
    fn tests_that_only_an_emitted_unanswered_dialog_is_left_open() {
        let dialogs = Dialogs::default();
        let (first, _) = dialogs.opened("T", "page@T", "alert", "hi", "").unwrap();
        assert_eq!(dialogs.left_open(), None, "never handed to the script");
        dialogs.emitted(first);
        assert_eq!(dialogs.left_open(), Some(DialogAnswer::Dismiss));
        assert_eq!(dialogs.left_open(), None, "answered once");
        let (second, _) = dialogs
            .opened("T", "page@T", "beforeunload", "", "")
            .unwrap();
        dialogs.emitted(second);
        assert_eq!(dialogs.left_open(), Some(DialogAnswer::Accept));
        assert!(dialogs.closed().is_some());
        assert!(dialogs.closed().is_none());
    }
}
