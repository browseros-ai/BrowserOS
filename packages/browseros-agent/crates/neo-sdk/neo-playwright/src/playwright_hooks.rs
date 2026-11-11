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

    /// Retained for host implementations; screenshots and PDFs do not call this hook.
    fn capture_allowed(&self, page: TabId) -> Result<(), Refusal>;

    /// Neo's own secret files, which `setInputFiles` never uploads: absolute paths, each a
    /// file or a directory, and a directory denies everything inside it. claw-sdk names its
    /// host token file, its CAPTCHA key directory, its trace directory and the browser
    /// profile directory. An entry that does not exist is skipped, and may appear later.
    ///
    /// Asked once per `setInputFiles` op, before any path is stat'ed. The crate matches what is
    /// on disk, not the strings: each entry and each path the script names is resolved through
    /// its symlinks and `..` and compared by `(dev, inode)`, so a symlink, a hard link, a
    /// different case or a bind mount names the same secret.
    ///
    /// The default is `None`, the host has named no secrets, and the crate then refuses every
    /// upload from a path as `neo / unsupported` (payloads still upload), so a host that serves
    /// the op without naming its secrets uploads nothing from Neo's disk rather than anything.
    /// `Some(vec![])` says there is nothing to deny. Whatever the list, a path whose resolution
    /// enters `/dev` or `/proc` is refused as a secret: there `/dev/fd/N` and `/proc/self/…`
    /// name the browser's own files once the browser opens the path.
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
