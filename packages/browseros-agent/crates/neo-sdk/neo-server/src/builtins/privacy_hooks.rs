//! The host callbacks neo-playwright requires, including Neo's upload deny list.

use std::path::PathBuf;

use browseros_core::TabId;
use browseros_core::sensitive_fields::{FieldValues, JournalEntry};
use neo_playwright::{PlaywrightHooks, Refusal, TypedBy};

use crate::config::SecretFiles;

/// Supplies Neo's own secret files to neo-playwright's upload checks. The remaining callbacks
/// retain no field values or private-tab state; captures are always allowed by this adapter.
pub(crate) struct PrivacyHooks {
    /// Located before the facade starts, since it holds these hooks for its whole lifetime.
    secret_files: SecretFiles,
}

impl PrivacyHooks {
    /// The upload deny list from startup config, before the facade takes ownership of the hooks.
    #[must_use]
    pub(crate) fn new(secret_files: SecretFiles) -> Self {
        Self { secret_files }
    }
}

impl PlaywrightHooks for PrivacyHooks {
    fn field_entered(&self, _tab: TabId, _entry: JournalEntry, _by: TypedBy) {}

    fn values_swept(&self, _tab: TabId, _values: &FieldValues) {}

    fn capture_allowed(&self, _tab: TabId) -> Result<(), Refusal> {
        Ok(())
    }

    /// The crate resolves this list on every upload, including paths created since startup.
    /// An unknown location refuses path uploads; payloads still upload without reading disk.
    fn secret_paths(&self) -> Option<Vec<PathBuf>> {
        self.secret_files.paths()
    }
}
