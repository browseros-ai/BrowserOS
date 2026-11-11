//! What neo-playwright tells Neo's privacy layer, and asks it, outside any one op's result.

use std::path::PathBuf;
use std::sync::Arc;

use browseros_core::TabId;
use browseros_core::sensitive_fields::{FieldValues, JournalEntry};
use neo_playwright::{PlaywrightHooks, Refusal, TypedBy};
use neo_trace::{ErrorOwner, Privacy, PrivacyRecord};

use crate::browser::TabRegistry;
use crate::config::SecretFiles;
use crate::dispatch::{ErrorCode, OperationError, Output, Policy, PolicyCheck, ResultHold};
use crate::host::FlagRecovery;

/// neo-playwright's privacy callbacks, over sdk-traces' `Privacy`: a private tab's pixels are
/// never captured, and no upload hands a page one of Neo's own secret files ([`SecretFiles`]).
/// Neo keeps nothing of what the crate reports seeing in password, card or one-time-code fields.
///
/// The crate calls `field_entered` from its journal task and the others from inside an op, so
/// nothing here awaits or holds a lock for longer than a lookup.
pub(crate) struct PrivacyHooks {
    privacy: Arc<Privacy>,
    /// Admission/close state: pixels fail closed after the tab leaves the registry.
    tabs: Arc<TabRegistry>,
    /// Pending while Neo makes up for private flags a replaced trace store lost: no tab's pixels
    /// are captured meanwhile.
    recovery: Arc<FlagRecovery>,
    /// Where Neo keeps its own secrets on disk, as `AppConfig::load` located them. The crate holds
    /// these hooks from `PlaywrightFacade::start` on, so a list found later would never reach it.
    secret_files: SecretFiles,
}

impl PrivacyHooks {
    /// The hooks over the process's one `Privacy`, which says which tabs are private, and over
    /// Neo's secret files, which no upload hands a page.
    #[must_use]
    pub(crate) fn new(
        privacy: Arc<Privacy>,
        tabs: Arc<TabRegistry>,
        recovery: Arc<FlagRecovery>,
        secret_files: SecretFiles,
    ) -> Self {
        Self {
            privacy,
            tabs,
            recovery,
            secret_files,
        }
    }
}

impl PlaywrightHooks for PrivacyHooks {
    /// Ignored: Neo keeps no value typed into a sensitive field. The crate's trait still asks
    /// every host for this hook.
    fn field_entered(&self, _tab: TabId, _entry: JournalEntry, _by: TypedBy) {}

    /// Ignored: Neo keeps no value a read found in a sensitive field. The crate's trait still
    /// asks every host for this hook.
    fn values_swept(&self, _tab: TabId, _values: &FieldValues) {}

    /// Refused as `host / page_private` while the tab is private. Pixels taken before the tab
    /// went private stay, as sdk-traces keeps them; this decides only whether to take new ones.
    fn capture_allowed(&self, tab: TabId) -> Result<(), Refusal> {
        if self.recovery.is_pending() {
            return Err(page_private(
                "Neo lost its record of which tabs are private when its trace store was \
                 replaced, so it captures no tab until every open one is private again",
            ));
        }
        if !self.tabs.is_open(tab) {
            // A close or an expired admission ticket leaves no trusted entry. Refusing both
            // cases prevents captures from bypassing the tab's privacy state.
            return Err(page_private(
                "Neo does not know this page's tab any more, so it cannot tell whether the tab \
                 is private",
            ));
        }
        match self.privacy.is_private(tab) {
            None => Ok(()),
            Some(record) => Err(page_private(&private_tab(record))),
        }
    }

    /// The crate's deny list, asked once per `setInputFiles` op before any path is stat'ed: each
    /// of Neo's secret files by its absolute path, or `None` while one has no known place, and
    /// then the crate refuses every upload from a path as `neo / unsupported` (payloads, which
    /// read no file, still upload). The crate resolves each entry again in every round of
    /// checks, so a file that appears later in an entry directory, as the trace store's `-wal`
    /// does, is denied too.
    fn secret_paths(&self) -> Option<Vec<PathBuf>> {
        self.secret_files.paths()
    }
}

/// The door's half of pixel privacy. The crate's check above runs once, right before the
/// capture is sent, so it cannot see a tab that goes private while the capture is in flight, and
/// the browser may draw the owner's first keystrokes into those very pixels. As a rule, the
/// door refuses a capture of a private tab, and holds sdk-traces' `CaptureGuard` from the op's
/// admission until its pixels go out, as sdk-traces does for its own captures.
impl Policy for PrivacyHooks {
    fn check(&self, check: &PolicyCheck<'_>) -> Result<(), OperationError> {
        let (Output::Pixels, Some(tab)) = (check.method.output, check.tab) else {
            return Ok(());
        };
        match self.privacy.is_private(tab) {
            None => Ok(()),
            Some(record) => Err(OperationError::host(
                ErrorCode::PAGE_PRIVATE,
                private_tab(record),
            )),
        }
    }

    /// Admits the pixels only when the tab's privacy never changed since the op was admitted.
    /// A tab that went private and then closed has no flag left for `check`, but its epoch
    /// moved, and the guard compares epochs.
    fn hold(&self, check: &PolicyCheck<'_>) -> Option<ResultHold> {
        let (Output::Pixels, Some(tab)) = (check.method.output, check.tab) else {
            return None;
        };
        // `None`: the tab went private between `check` and here.
        let guard = self.privacy.capture(tab);
        Some(Box::new(move || match guard {
            Some(guard) if guard.admit() => Ok(()),
            Some(_) | None => Err(OperationError::host(
                ErrorCode::PAGE_PRIVATE,
                "this tab went private while Neo captured it, so Neo captures none of its pixels",
            )),
        }))
    }
}

/// Why a private tab gives no pixels, in both refusals.
fn private_tab(record: PrivacyRecord) -> String {
    format!(
        "this tab is private ({}), so Neo captures none of its pixels",
        record.reason.as_str()
    )
}

/// The capture refusal. `builtins/playwright_ops.rs` maps it back by this code when the crate
/// returns it as the op's error.
fn page_private(message: &str) -> Refusal {
    Refusal {
        owner: ErrorOwner::Host,
        code: ErrorCode::PAGE_PRIVATE.as_str().to_owned(),
        message: message.to_owned(),
        host_code: None,
    }
}
