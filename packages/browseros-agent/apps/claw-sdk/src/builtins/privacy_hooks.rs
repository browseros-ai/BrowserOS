//! What browseros-playwright tells Neo's privacy layer, and asks it, outside any one op's result.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use browseros_core::PageId;
use browseros_core::sensitive_fields::{FieldValues, JournalEntry};
use browseros_playwright::{PlaywrightHooks, Refusal, TypedBy};
use browseros_trace::{ErrorOwner, Privacy, PrivacyRecord};

use super::TypedValues;
use crate::browser::TabRegistry;
use crate::config::SecretFiles;
use crate::dispatch::{ErrorCode, OperationError, Output, Policy, PolicyCheck, ResultHold};
use crate::host::FlagRecovery;

/// browseros-playwright's privacy callbacks, over sdk-traces' `Privacy` (privacy layers L1 and
/// L2). A value the crate saw in a password, card or one-time-code field joins the process's
/// secret set, which the door masks out of everything a script, a host or the trace sees, and a
/// private tab's pixels are never captured. A value typed or read there joins once its field is
/// done ([`TypedValues`]), so none of its prefixes does. No upload hands a page one of Neo's own
/// secret files ([`SecretFiles`]).
///
/// The crate calls `field_entered` from its journal task and the others from inside an op, so
/// nothing here awaits or holds a lock for longer than a lookup.
pub(crate) struct PrivacyHooks {
    privacy: Arc<Privacy>,
    /// Page id to tab id, without asking core: the door touched every op's page before the op
    /// ran, so the registry knows the tab of any page an op captures or types into.
    tabs: Arc<TabRegistry>,
    /// Typed values, until each field is done.
    typed: Arc<TypedValues>,
    /// Pending while Neo makes up for private flags a replaced trace store lost: no tab's pixels
    /// are captured meanwhile.
    recovery: Arc<FlagRecovery>,
    /// Where Neo keeps its own secrets on disk, as `AppConfig::load` located them. The crate holds
    /// these hooks from `PlaywrightFacade::start` on, so a list found later would never reach it.
    secret_files: SecretFiles,
}

impl PrivacyHooks {
    /// The hooks over the process's one `Privacy`, which holds the secret set the door masks with,
    /// and over Neo's secret files, which no upload hands a page. A typed value joins the set
    /// once its field got no input for `typed_quiet`.
    #[must_use]
    pub(crate) fn new(
        privacy: Arc<Privacy>,
        tabs: Arc<TabRegistry>,
        recovery: Arc<FlagRecovery>,
        secret_files: SecretFiles,
        typed_quiet: Duration,
    ) -> Self {
        let typed = TypedValues::start(
            Arc::clone(privacy.secrets()),
            typed_quiet,
            tabs.subscribe_closed(),
        );
        Self {
            privacy,
            tabs,
            typed,
            recovery,
            secret_files,
        }
    }

    /// The values typed into sensitive fields that the set has not learned yet. The door is built
    /// with them, and masks every run's records and reply with them.
    #[must_use]
    pub(crate) fn typed(&self) -> Arc<TypedValues> {
        Arc::clone(&self.typed)
    }
}

impl PlaywrightHooks for PrivacyHooks {
    /// Masked once its field is done, whoever typed it, the agent included, as the takeover
    /// journal masks the owner's typing. A page's privacy is the takeover's and the host's to
    /// decide, so a value entered here never makes the tab private by itself.
    fn field_entered(&self, page: PageId, entry: JournalEntry, _by: TypedBy) {
        let tab = self.tabs.tab_of(&page);
        self.typed.note(page, tab, entry);
    }

    /// Held until its field is done, as a typed value is: a read between two keystrokes finds a
    /// prefix. The crate masked these values out of the read's own result already.
    fn values_swept(&self, page: PageId, values: &FieldValues) {
        let tab = self.tabs.tab_of(&page);
        self.typed.swept(&page, tab, values);
    }

    /// Refused as `host / page_private` while the tab is private. Pixels taken before the tab
    /// went private stay, as sdk-traces keeps them; this decides only whether to take new ones.
    fn capture_allowed(&self, page: PageId) -> Result<(), Refusal> {
        if self.recovery.is_pending() {
            return Err(page_private(
                "Neo lost its record of which tabs are private when its trace store was \
                 replaced, so it captures no tab until every open one is private again",
            ));
        }
        let Some(tab) = self.tabs.tab_of(&page) else {
            // Only a tab that closed under the op gets here; refusing is the side that cannot
            // leak a private tab.
            return Err(page_private(
                "Neo does not know this page's tab any more, so it cannot tell whether the tab \
                 is private",
            ));
        };
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
