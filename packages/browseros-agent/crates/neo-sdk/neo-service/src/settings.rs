//! What the host set (`/v1/settings`): the live values every part reads when it acts, and the
//! file that keeps them across restarts of the server.
//!
//! ```text
//!  start ─▶ Settings::load(settings.json) ─ key by key: an unknown name or a bad value
//!             │                               falls back to its default, with a warning
//!             ▼
//!  PUT / PATCH ─▶ settings_routes ─▶ Settings::change ─ save (temp file, rename) ─▶ switch ─▶ 200
//!                                                        └ fails: 500, nothing switched
//!  janitor, retention sweep (the app) ─▶ Settings::tab_idle_close, ::trace_retention, each pass
//! ```
//!
//! The browser restarts a killed server by itself, and no host notices (the settings design,
//! section 3). Values kept only in memory would silently go back to their defaults then, so a
//! change is saved before it is switched on, and a start loads the file before the first run.
//! The file lives beside `trace.db`, not in it: the server moves a trace store it cannot open
//! aside, and the settings must not vanish with it.

use crate::setting::{SETTINGS, Setting, SettingValue, TAB_IDLE_CLOSE, TRACE_RETENTION};
use crate::wire;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// The settings the host set, by name. A setting absent here is at its default.
pub(crate) type SetValues = BTreeMap<&'static str, SettingValue>;

/// The host's settings, live. Cheap to clone: every clone reads and changes the same values.
///
/// The app makes one per process before anything that reads it starts, hands it to the
/// service (`ServiceInputs::settings`) for the routes, and to each part that reads a setting.
#[derive(Clone)]
pub struct Settings {
    inner: Arc<Inner>,
}

struct Inner {
    /// Where a change is saved; `None` keeps changes in memory (no host API, the Node-hosted
    /// embedding, tests).
    file: Option<PathBuf>,
    /// What the host set. Read on every pass of every reader, written only by `change`.
    set: RwLock<SetValues>,
    /// One change at a time, from its save to its switch, so two calls cannot interleave a
    /// file and the live values that disagree.
    changing: tokio::sync::Mutex<()>,
}

/// How a change treats the settings it does not name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChangeMode {
    /// `PUT`: a setting left out goes back to its default.
    Replace,
    /// `PATCH`: a setting left out keeps its value.
    Merge,
}

impl Settings {
    /// Every setting at its default, kept in memory only. For a Neo with no host API: nothing
    /// could change them, and a file saved by an earlier run with a host API is ignored.
    #[must_use]
    pub fn defaults() -> Self {
        Self::new(None, SetValues::new())
    }

    /// The settings saved in `file`, which later changes are saved to. A missing file means
    /// the defaults. A file that cannot be read or is not a JSON object means the defaults and
    /// an error in the log. A name this build does not know, or a value it cannot use, falls
    /// back to its default with a warning, and the other settings still apply: the server never
    /// fails to start over this file.
    pub async fn load(file: PathBuf) -> Self {
        let set = read_saved(&file).await;
        Self::new(Some(file), set)
    }

    fn new(file: Option<PathBuf>, set: SetValues) -> Self {
        Self {
            inner: Arc::new(Inner {
                file,
                set: RwLock::new(set),
                changing: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// `tabs.idleCloseSeconds`: the idle-tab janitor closes an agent's tab nobody used for this
    /// long. Read at each pass, so a change applies at the next one.
    #[must_use]
    pub fn tab_idle_close(&self) -> Duration {
        Duration::from_secs(self.whole(&TAB_IDLE_CLOSE))
    }

    /// `trace.retentionSeconds`: the retention sweep deletes runs older than this. Read at each
    /// sweep.
    #[must_use]
    pub fn trace_retention(&self) -> Duration {
        Duration::from_secs(self.whole(&TRACE_RETENTION))
    }

    /// Every setting at its current value: what the host set, or the default.
    pub(crate) fn current(&self) -> Vec<(&'static Setting, SettingValue)> {
        let set = self.set();
        SETTINGS
            .iter()
            .map(|setting| (setting, value_of(&set, setting)))
            .collect()
    }

    /// Applies what a `PUT` or a `PATCH` set. Saved first, then switched on, so a failed save
    /// changes nothing. A change that leaves what the host set as it was writes and logs
    /// nothing.
    ///
    /// # Errors
    /// The file could not be written; the live values are as they were.
    pub(crate) async fn change(&self, values: SetValues, mode: ChangeMode) -> io::Result<()> {
        let _changing = self.inner.changing.lock().await;
        let before = self.set();
        let after = match mode {
            ChangeMode::Replace => values,
            ChangeMode::Merge => {
                let mut merged = before.clone();
                merged.extend(values);
                merged
            }
        };
        if after == before {
            return Ok(());
        }
        if let Some(file) = &self.inner.file {
            save(file, &after).await?;
        }
        *self
            .inner
            .set
            .write()
            .unwrap_or_else(PoisonError::into_inner) = after.clone();
        log_change(&before, &after);
        Ok(())
    }

    /// Applies a `PATCH /v1/settings` body from inside the process, with the route's rules:
    /// unknown names are skipped and returned, a bad value refuses the whole body with the
    /// route's message. For tests that change a setting while the parts that read it run.
    ///
    /// # Errors
    /// The route's 400 message, or the save's failure.
    #[doc(hidden)]
    pub async fn patch(&self, body: &[u8]) -> Result<Vec<String>, String> {
        let (values, unknown) =
            wire::settings_request(body).map_err(|refused| refused.message.into_owned())?;
        self.change(values, ChangeMode::Merge)
            .await
            .map_err(|error| error.to_string())?;
        Ok(unknown)
    }

    fn whole(&self, setting: &Setting) -> u64 {
        match value_of(&self.set(), setting) {
            SettingValue::Whole(whole) => whole,
        }
    }

    fn set(&self) -> SetValues {
        self.inner
            .set
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn value_of(set: &SetValues, setting: &Setting) -> SettingValue {
    set.get(setting.name)
        .copied()
        .unwrap_or_else(|| setting.default_value())
}

/// What the last run saved, key by key (`Settings::load`).
async fn read_saved(file: &Path) -> SetValues {
    let bytes = match fs::read(file).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            tracing::info!(file = %file.display(), "no saved settings; every setting is at its default");
            return SetValues::new();
        }
        Err(error) => {
            tracing::error!(%error, file = %file.display(), "the saved settings cannot be read; every setting is at its default");
            return SetValues::new();
        }
    };
    let saved = match serde_json::from_slice(&bytes) {
        Ok(json) => wire::settings_body(&json),
        Err(_) => None,
    };
    let Some(saved) = saved else {
        tracing::error!(file = %file.display(), "the saved settings are not a JSON object; every setting is at its default");
        return SetValues::new();
    };
    for name in &saved.unknown {
        tracing::warn!(file = %file.display(), name, "a saved setting this build does not know is skipped");
    }
    for fault in &saved.faults {
        tracing::warn!(file = %file.display(), fault, "a saved setting falls back to its default");
    }
    let applied: Vec<String> = saved
        .values
        .iter()
        .map(|(name, value)| format!("{name}={}", Setting::json(*value)))
        .collect();
    tracing::info!(file = %file.display(), applied = %applied.join(" "), "saved settings loaded");
    saved.values
}

/// Writes `set` to a temporary file and renames it over `file`, so the file is never half
/// written: a crash leaves the old one or the new one. Mode 0600: it is the host's.
async fn save(file: &Path, set: &SetValues) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(&wire::settings_object(
        set.iter().map(|(name, value)| (*name, *value)),
    ))?;
    bytes.push(b'\n');
    let mut temp = file.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    let written = async {
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)
            .await?;
        out.write_all(&bytes).await?;
        out.sync_all().await?;
        drop(out);
        fs::rename(&temp, file).await
    }
    .await;
    if written.is_err() {
        // Absent when the open failed.
        let _ = fs::remove_file(&temp).await;
    }
    written
}

/// One line per change, with every setting whose value changed, from and to. No setting is a
/// secret (the settings design, section 2), so values are logged.
fn log_change(before: &SetValues, after: &SetValues) {
    let changed: Vec<String> = SETTINGS
        .iter()
        .filter_map(|setting| {
            let from = value_of(before, setting);
            let to = value_of(after, setting);
            (from != to).then(|| {
                format!(
                    "{}: {} -> {}",
                    setting.name,
                    Setting::json(from),
                    Setting::json(to)
                )
            })
        })
        .collect();
    let set: Vec<&str> = after.keys().copied().collect();
    tracing::info!(changed = %changed.join(", "), set = %set.join(" "), "settings changed");
}

#[cfg(test)]
mod tests {
    use super::Settings;
    use neo_trace::RetentionPolicy;
    use std::time::Duration;

    #[test]
    fn every_default_is_todays_behaviour() {
        let settings = Settings::defaults();
        assert_eq!(
            settings.trace_retention(),
            RetentionPolicy::default().max_age,
            "the retention default is the trace store's own policy"
        );
        assert_eq!(
            settings.tab_idle_close(),
            Duration::from_mins(30),
            "the janitor closed agent tabs after 30 idle minutes before the setting existed"
        );
    }
}
