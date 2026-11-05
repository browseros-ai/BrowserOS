//! What browseros-playwright tells Neo's privacy layer, and asks it, outside any one op's result.

use std::mem;
use std::sync::Arc;

use browseros_core::PageId;
use browseros_core::sensitive_fields::{FieldValues, JournalEntry};
use browseros_playwright::{PlaywrightHooks, Refusal, TypedBy};
use browseros_trace::{ErrorOwner, Privacy, SecretValue};

use crate::browser::TabRegistry;
use crate::dispatch::ErrorCode;

/// browseros-playwright's privacy callbacks, over sdk-traces' `Privacy` (privacy layers L1 and
/// L2). A value the crate saw in a password, card or one-time-code field joins the process's
/// secret set, which the door masks out of everything a script, a host or the trace sees, and a
/// private tab's pixels are never captured.
///
/// The crate calls `field_entered` from its journal task and the other two from inside an op,
/// so nothing here awaits or holds a lock for longer than a lookup.
pub(crate) struct PrivacyHooks {
    privacy: Arc<Privacy>,
    /// Page id to tab id, without asking core: the door touched every op's page before the op
    /// ran, so the registry knows the tab of any page an op captures.
    tabs: Arc<TabRegistry>,
}

impl PrivacyHooks {
    /// The hooks over the process's one `Privacy`, which holds the secret set the door masks with.
    #[must_use]
    pub(crate) fn new(privacy: Arc<Privacy>, tabs: Arc<TabRegistry>) -> Self {
        Self { privacy, tabs }
    }

    fn register(&self, value: String) {
        // Too short to mask without masking ordinary text: the set keeps nothing, which is its
        // rule, not a failure here.
        let _registration = self
            .privacy
            .secrets()
            .register(SecretValue::new(value), None);
    }
}

impl PlaywrightHooks for PrivacyHooks {
    /// Masked from now on whoever typed it, the agent included, as the takeover journal masks
    /// the owner's typing. A page's privacy is the takeover's and the host's to decide, so a
    /// value entered here never makes the tab private by itself.
    fn field_entered(&self, _page: PageId, mut entry: JournalEntry, _by: TypedBy) {
        // Taken out of the entry, so the set's copy is the only one left.
        self.register(mem::take(&mut *entry.value));
    }

    fn values_swept(&self, _page: PageId, values: &FieldValues) {
        for value in values.registrable() {
            self.register(value.to_owned());
        }
    }

    /// Refused as `host / page_private` while the tab is private. Pixels taken before the tab
    /// went private stay, as sdk-traces keeps them; this decides only whether to take new ones.
    fn capture_allowed(&self, page: PageId) -> Result<(), Refusal> {
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
            Some(record) => Err(page_private(&format!(
                "this tab is private ({}), so Neo captures none of its pixels",
                record.reason.as_str()
            ))),
        }
    }
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
