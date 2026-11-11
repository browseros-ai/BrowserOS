//! The privacy callbacks that happen outside any one op's result.

use std::path::PathBuf;

use browseros_core::TabId;
use browseros_core::sensitive_fields::{FieldValues, JournalEntry};

use crate::Refusal;

/// What the crate tells the host's privacy layer, and asks it, outside an op's own result.
/// claw-sdk implements it over sdk-traces' `Privacy`. Implementations must not block: the
/// journal calls arrive on the crate's background event task.
pub trait PlaywrightHooks: Send + Sync {
    /// The sealed world's journal saw a value entered into a sensitive field of `page`; `by`
    /// says whether an agent input op on that page was typing at the time. The host decides
    /// what to keep, so the privacy policy lives in one place.
    fn field_entered(&self, page: TabId, entry: JournalEntry, by: TypedBy);

    /// A main-world script ran on `page`, a snapshot read one of its frames, or `title()` or
    /// `content()` read its document, while these sensitive values were on it. The crate has
    /// already masked the result with them; the host registers them as secrets. A snapshot
    /// reports each frame's values as it reads it.
    fn values_swept(&self, page: TabId, values: &FieldValues);

    /// May `page`'s pixels be captured now? Asked right before `Page.captureScreenshot` and
    /// `Page.printToPDF`, after the dispatcher's own entry checks passed, because a page can
    /// turn private while an op waits.
    fn capture_allowed(&self, page: TabId) -> Result<(), Refusal>;

    /// Legacy host-supplied secret paths, retained for compatibility with existing hosts.
    /// Uploads use the paths supplied by the script without consulting this hook.
    fn secret_paths(&self) -> Option<Vec<PathBuf>> {
        None
    }
}

/// Who typed a journaled value, as far as the crate can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypedBy {
    /// One of the agent's input ops on that page was between `mark_sent` and its end.
    Agent,
    /// Anyone else: the owner during a takeover, the page's own scripts, autofill.
    NotAgent,
}
