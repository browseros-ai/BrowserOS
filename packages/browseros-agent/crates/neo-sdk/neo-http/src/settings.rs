//! What the host set (`/v1/settings`): the live values every part reads when it acts, and the
//! file that keeps them across restarts of the server.
//!
//! ```text
//!  start ─▶ Settings::load(settings.json) ─ unknown names skipped; bad values use defaults,
//!             │                               except recording switches, which use off
//!             ▼
//!  PUT / PATCH ─▶ settings_routes ─▶ Settings::change ─ save (temp file, rename) ─▶ switch ─▶ 200
//!                                                        └ fails: 500, nothing switched
//!  janitor, retention sweep (the app) ─▶ Settings::tab_idle_close, ::trace_retention, each pass
//!  trace writer (the app's Settings::watch) ◀── RecordPolicy, at start and after each change,
//!                                              before the change answers
//! ```
//!
//! The browser restarts a killed server by itself, and no host notices (the settings design,
//! section 3). Values kept only in memory would silently go back to their defaults then, so a
//! change is saved before it is switched on, and a start loads the file before the first run.
//! The file lives beside `trace.db`, not in it: the server moves a trace store it cannot open
//! aside, and the settings must not vanish with it.

use crate::setting::{
    Checked, FILE_READ_DIRS, FILL_PICTURES, FILL_PICTURES_NEVER, SCREENSHOTS_OFF, SETTINGS,
    Setting, SettingValue, TAB_IDLE_CLOSE, TABS_SCOPE, TABS_SCOPE_BROWSER, TRACE_RETENTION,
    TRACE_SCREENSHOTS, TRACE_VALUE_BYTES, TRACE_VALUES,
};
use crate::wire;
use neo_trace::{RecordPolicy, Screenshots, Values};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// The settings the host set, by name. A setting absent here is at its default.
pub(crate) type SetValues = BTreeMap<&'static str, SettingValue>;

/// The startup file's omissions, retained for this settings instance. Only names belong here,
/// never input values or the validation rules used in logs.
pub(crate) struct FileLoadReport {
    pub(crate) state: FileLoadState,
    pub(crate) unknown: Vec<String>,
    pub(crate) unknown_values: Vec<String>,
    pub(crate) invalid: Vec<String>,
}

/// The outcome of one startup load, before any API changes can replace the saved file.
#[derive(Clone, Copy)]
pub(crate) enum FileLoadState {
    Absent,
    Applied,
    Partial,
    Unreadable,
}

/// Keeps the effective startup values and the report of that same load together.
struct SavedSettings {
    values: SetValues,
    report: FileLoadReport,
}

/// `tabs.scope` as a run reads it at its start ([`Settings::tabs_scope`]). With the run's task it
/// decides what the run may reach: under `Task`, a run that carries a task sees only its task
/// family's tabs, and a run with none sees every tab but never takes over a host's task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabsScope {
    /// No run is bound, whatever task it carries: every tab, as before task scope.
    Browser,
    /// A run that carries a task is bound to its task's family.
    Task,
}

/// The host's settings, live. Cheap to clone: every clone reads and changes the same values.
///
/// The app makes one per process before anything that reads it starts, hands it to the
/// service (`ServiceInputs::settings`) for the routes, and to each part that reads a setting.
#[derive(Clone)]
pub struct Settings {
    inner: Arc<Inner>,
}

/// Defaults whose values belong to the application's filesystem layout, rather than a
/// constant in the settings table. One instance drives GET, reset, load fallback and logs.
#[derive(Clone, Debug, Default)]
pub struct SettingsDefaults {
    /// The browser's downloads folder by default; the Node embedding may supply a list.
    pub read_dirs: Vec<PathBuf>,
}

impl SettingsDefaults {
    /// Checks an embedding's roots with the same rule as `/v1/settings`. Server defaults
    /// already come from the validated launch environment.
    pub fn new(read_dirs: Vec<PathBuf>) -> Result<Self, String> {
        match FILE_READ_DIRS.check(&Setting::json(&SettingValue::Dirs(read_dirs.clone()))) {
            Checked::Value(_) => Ok(Self { read_dirs }),
            Checked::Refused(rule) => Err(format!("`files.readDirs` {rule}")),
            Checked::UnknownValue => {
                Err("`files.readDirs` does not accept unknown values".to_owned())
            }
        }
    }
}

struct Inner {
    defaults: SettingsDefaults,
    /// Where a change is saved; `None` keeps changes in memory (the Node-hosted embedding,
    /// tests).
    file: Option<PathBuf>,
    /// Written once before sharing `Inner`; later saves never rewrite the startup result.
    file_report: Option<FileLoadReport>,
    /// What the host set. Read on every pass of every reader, written only by `change`.
    set: RwLock<SetValues>,
    /// One change at a time, from its save to its switch, so two calls cannot interleave a
    /// file and the live values that disagree.
    changing: tokio::sync::Mutex<()>,
    /// The parts told of every change, in order, before it answers (`Settings::watch`).
    watchers: Mutex<Vec<Watcher>>,
}

/// A part told of every change: the app hands the trace writer its record policy with one.
type Watcher = Box<dyn Fn(&Settings) + Send + Sync>;

/// How a change treats the settings it does not name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChangeMode {
    /// `PUT`: a setting left out goes back to its default.
    Replace,
    /// `PATCH`: a setting left out keeps its value.
    Merge,
}

impl Settings {
    /// Every setting at its default, kept in memory only for embeddings that supply settings
    /// in code rather than reading a server state folder.
    #[must_use]
    pub fn defaults(defaults: SettingsDefaults) -> Self {
        Self::new(None, SetValues::new(), defaults, None)
    }

    /// The settings saved in `file`, which later changes are saved to. A missing file means
    /// the defaults. Unreadable or malformed input uses defaults with recording switches off.
    /// Unknown names are skipped; invalid known values use their saved-input fallback or normal
    /// default, with a warning. Good siblings still apply, and this file never prevents startup.
    pub async fn load(file: PathBuf, defaults: SettingsDefaults) -> Self {
        let SavedSettings { values, report } = read_saved(&file).await;
        Self::new(Some(file), values, defaults, Some(report))
    }

    fn new(
        file: Option<PathBuf>,
        set: SetValues,
        defaults: SettingsDefaults,
        file_report: Option<FileLoadReport>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                defaults,
                file,
                file_report,
                set: RwLock::new(set),
                changing: tokio::sync::Mutex::new(()),
                watchers: Mutex::default(),
            }),
        }
    }

    /// The load this instance started with, shared unchanged across settings clones and saves.
    pub(crate) fn file_report(&self) -> Option<&FileLoadReport> {
        self.inner.file_report.as_ref()
    }

    /// `tabs.idleCloseSeconds`: the idle-tab janitor closes an agent's tab nobody used for this
    /// long. Read at each pass, so a change applies at the next one.
    #[must_use]
    pub fn tab_idle_close(&self) -> Duration {
        Duration::from_secs(self.whole(&TAB_IDLE_CLOSE))
    }

    /// `tabs.scope`: whether a run that carries a task is bound to its task's tabs. A run reads it
    /// once, at its start, so a change applies to the runs admitted after it answers, never to
    /// one already running.
    #[must_use]
    pub fn tabs_scope(&self) -> TabsScope {
        match self.value(&TABS_SCOPE) {
            SettingValue::Choice(TABS_SCOPE_BROWSER) => TabsScope::Browser,
            SettingValue::Choice(_) | SettingValue::Whole(_) | SettingValue::Dirs(_) => {
                TabsScope::Task
            }
        }
    }

    /// `trace.retentionSeconds`: the retention sweep deletes runs older than this. Read at each
    /// sweep.
    #[must_use]
    pub fn trace_retention(&self) -> Duration {
        Duration::from_secs(self.whole(&TRACE_RETENTION))
    }

    /// `fill.pictures` is `never`: no picture of a tab that took a host's private fill, by a script
    /// or by Neo, and each fill writes its tab into the record of filled tabs first (ruling R58).
    /// Read when a picture is asked for and when a fill starts, so a change takes effect from the
    /// next one.
    #[must_use]
    pub fn fill_pictures_never(&self) -> bool {
        matches!(
            self.value(&FILL_PICTURES),
            SettingValue::Choice(FILL_PICTURES_NEVER)
        )
    }

    /// `trace.screenshots`, `trace.valueBytes` and `trace.values`: what the trace writer records. It gets them
    /// in a message (`TraceSink::record_policy`), sent by a [`Settings::watch`] the app sets up.
    #[must_use]
    pub fn trace_policy(&self) -> RecordPolicy {
        let screenshots = match self.value(&TRACE_SCREENSHOTS) {
            SettingValue::Choice(SCREENSHOTS_OFF) => Screenshots::Off,
            SettingValue::Choice(_) | SettingValue::Whole(_) | SettingValue::Dirs(_) => {
                Screenshots::PerOperation
            }
        };
        let values = match self.value(&TRACE_VALUES) {
            SettingValue::Choice("off") => Values::Off,
            SettingValue::Choice(_) | SettingValue::Whole(_) | SettingValue::Dirs(_) => Values::On,
        };
        RecordPolicy {
            screenshots,
            values,
            value_bytes: usize::try_from(self.whole(&TRACE_VALUE_BYTES)).unwrap_or(usize::MAX),
        }
    }

    /// Calls `apply` now, and again after every change, before the change answers: a part that
    /// is handed its settings rather than reading them on each pass. Called in change order,
    /// under the change's own lock, so `apply` must not wait.
    pub fn watch(&self, apply: impl Fn(&Self) + Send + Sync + 'static) {
        // Under the watchers' lock, which a change takes to call them after it switched: a
        // change that lands meanwhile calls this one too, after it, so the last call is never
        // older than the values.
        let mut watchers = self
            .inner
            .watchers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        apply(self);
        watchers.push(Box::new(apply));
    }

    /// `files.readDirs`, read at every filesystem gateway so a change also applies to a run
    /// already in flight. The lock protects only this clone, never filesystem work.
    #[must_use]
    pub fn read_dirs(&self) -> Vec<PathBuf> {
        match self.value(&FILE_READ_DIRS) {
            SettingValue::Dirs(dirs) => dirs,
            SettingValue::Whole(_) | SettingValue::Choice(_) => {
                self.inner.defaults.read_dirs.clone()
            }
        }
    }

    pub(crate) fn default_value(&self, setting: &Setting) -> SettingValue {
        setting.default_value(&self.inner.defaults)
    }

    /// Every setting at its current value: what the host set, or the default.
    pub(crate) fn current(&self) -> Vec<(&'static Setting, SettingValue)> {
        let set = self
            .inner
            .set
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        SETTINGS
            .iter()
            .map(|setting| (setting, value_of(&set, setting, &self.inner.defaults)))
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
        for apply in self
            .inner
            .watchers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            apply(self);
        }
        log_change(&before, &after, &self.inner.defaults);
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
        let request =
            wire::settings_request(body).map_err(|refused| refused.message.into_owned())?;
        self.change(request.values, ChangeMode::Merge)
            .await
            .map_err(|error| error.to_string())?;
        Ok(request.unknown)
    }

    fn whole(&self, setting: &Setting) -> u64 {
        match (self.value(setting), self.default_value(setting)) {
            (SettingValue::Whole(whole), _) | (_, SettingValue::Whole(whole)) => whole,
            // Only Whole rows call this getter. Other kinds have typed getters.
            _ => 0,
        }
    }

    fn value(&self, setting: &Setting) -> SettingValue {
        let set = self
            .inner
            .set
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        value_of(&set, setting, &self.inner.defaults)
    }

    fn set(&self) -> SetValues {
        self.inner
            .set
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn value_of(set: &SetValues, setting: &Setting, defaults: &SettingsDefaults) -> SettingValue {
    set.get(setting.name)
        .cloned()
        .unwrap_or_else(|| setting.default_value(defaults))
}

/// Loads what the last run saved key by key, keeping the load's report with its fallbacks.
async fn read_saved(file: &Path) -> SavedSettings {
    let mut loaded = SavedSettings {
        values: SetValues::new(),
        report: FileLoadReport {
            state: FileLoadState::Unreadable,
            unknown: Vec::new(),
            unknown_values: Vec::new(),
            invalid: Vec::new(),
        },
    };
    let bytes = match fs::read(file).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            tracing::info!(file = %file.display(), "no saved settings; every setting is at its default");
            loaded.report.state = FileLoadState::Absent;
            return loaded;
        }
        Err(error) => {
            tracing::error!(%error, file = %file.display(), "the saved settings cannot be read; recording switches are off, other settings use defaults");
            saved_fallbacks("", &mut loaded.values);
            return loaded;
        }
    };
    let saved = match serde_json::from_slice(&bytes) {
        Ok(json) => wire::settings_body(&json),
        Err(_) => None,
    };
    let Some(mut saved) = saved else {
        tracing::error!(file = %file.display(), "the saved settings are not a JSON object; recording switches are off, other settings use defaults");
        saved_fallbacks("", &mut loaded.values);
        return loaded;
    };
    for name in &saved.unknown {
        tracing::warn!(file = %file.display(), name, "a saved setting this build does not know is skipped");
    }
    for fault in &saved.faults {
        let fallback = if saved_fallbacks(&fault.name, &mut saved.values) {
            "off"
        } else {
            "default"
        };
        tracing::warn!(file = %file.display(), %fault, fallback, "a saved setting cannot be used");
    }
    for name in &saved.unknown_values {
        let known = Setting::find(name)
            .map(Setting::values_text)
            .unwrap_or_default();
        let fallback = if saved_fallbacks(name, &mut saved.values) {
            "off"
        } else {
            "default"
        };
        tracing::warn!(file = %file.display(), name, known, fallback, "a saved setting has a value this build does not have");
    }
    let applied: Vec<String> = saved
        .values
        .iter()
        .map(|(name, value)| format!("{name}={}", Setting::json(value)))
        .collect();
    tracing::info!(file = %file.display(), applied = %applied.join(" "), "saved settings loaded");
    // Keep the report independent of the fallback values and log wording. Ordering is part
    // of the host contract even if the request parser's traversal order changes.
    let mut invalid: Vec<String> = saved.faults.into_iter().map(|fault| fault.name).collect();
    saved.unknown.sort_unstable();
    saved.unknown_values.sort_unstable();
    invalid.sort_unstable();
    loaded.report = FileLoadReport {
        state: if saved.unknown.is_empty() && saved.unknown_values.is_empty() && invalid.is_empty()
        {
            FileLoadState::Applied
        } else {
            FileLoadState::Partial
        },
        unknown: saved.unknown,
        unknown_values: saved
            .unknown_values
            .into_iter()
            .map(str::to_owned)
            .collect(),
        invalid,
    };
    loaded.values = saved.values;
    loaded
}

/// A fault belongs to a setting or a dotted ancestor group; an empty name covers the whole
/// file. Insert fallbacks into the explicit set so GET and the next API save preserve them.
fn saved_fallbacks(name: &str, set: &mut SetValues) -> bool {
    let mut applied = false;
    for setting in SETTINGS {
        if (name.is_empty()
            || setting.name == name
            || setting
                .name
                .strip_prefix(name)
                .is_some_and(|rest| rest.starts_with('.')))
            && let Some(value) = &setting.invalid_saved
        {
            set.insert(setting.name, value.clone());
            applied = true;
        }
    }
    applied
}

/// Writes `set` to a temporary file and renames it over `file`, so the file is never half
/// written: a crash leaves the old one or the new one. Mode 0600 keeps it private to Neo.
async fn save(file: &Path, set: &SetValues) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(&wire::settings_object(
        set.iter().map(|(name, value)| (*name, value.clone())),
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
fn log_change(before: &SetValues, after: &SetValues, defaults: &SettingsDefaults) {
    let changed: Vec<String> = SETTINGS
        .iter()
        .filter_map(|setting| {
            let from = value_of(before, setting, defaults);
            let to = value_of(after, setting, defaults);
            (from != to).then(|| {
                format!(
                    "{}: {} -> {}",
                    setting.name,
                    Setting::json(&from),
                    Setting::json(&to)
                )
            })
        })
        .collect();
    let set: Vec<&str> = after.keys().copied().collect();
    tracing::info!(changed = %changed.join(", "), set = %set.join(" "), "settings changed");
}

#[cfg(test)]
mod tests {
    use super::{Settings, TabsScope};
    use neo_trace::{RecordPolicy, RetentionPolicy};
    use std::time::Duration;

    #[test]
    fn every_default_keeps_todays_behaviour_but_fill_pictures_and_tabs_scope() {
        let settings = Settings::defaults(Default::default());
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
        assert_eq!(
            settings.trace_policy(),
            RecordPolicy::default(),
            "Neo's own screenshots on, and the trace's default value limits"
        );
        assert!(
            !settings.fill_pictures_never(),
            "ruling R58: a tab that took a private fill is pictured by default, the owner's \
             decision, where before it never was"
        );
        assert_eq!(
            settings.tabs_scope(),
            TabsScope::Task,
            "a run that carries a task sees only its task's tabs by default, where before it saw \
             every tab"
        );
    }
    /// Ruling R58 with the saved settings file: a saved `fill.pictures: never` is honoured from
    /// the start, before any fill; a saved value this build cannot use (an unknown name, a value
    /// of the wrong type) reads as `allowed`, the default, while the file's other settings still
    /// apply. Fails with `never` as the row's saved fallback (the unusable one reads `never`).
    #[tokio::test]
    async fn a_saved_fill_pictures_is_honoured_and_an_unusable_one_reads_allowed()
    -> Result<(), Box<dyn std::error::Error>> {
        use super::SettingsDefaults;

        let dir = tempfile::tempdir()?;
        let file = dir.path().join("settings.json");
        tokio::fs::write(&file, br#"{"fill":{"pictures":"never"}}"#).await?;
        let saved = Settings::load(file.clone(), SettingsDefaults::default()).await;
        assert!(
            saved.fill_pictures_never(),
            "a saved never holds from the start"
        );
        for unusable in [
            &br#"{"fill":{"pictures":"sometimes"},"trace":{"retentionSeconds":7200}}"#[..],
            &br#"{"fill":{"pictures":false},"trace":{"retentionSeconds":7200}}"#[..],
        ] {
            tokio::fs::write(&file, unusable).await?;
            let loaded = Settings::load(file.clone(), SettingsDefaults::default()).await;
            assert!(
                !loaded.fill_pictures_never(),
                "an unusable value reads allowed"
            );
            assert_eq!(
                loaded.trace_retention(),
                Duration::from_secs(7200),
                "the file's other settings still apply"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn application_defaults_drive_answers_reset_and_saved_fallback()
    -> Result<(), Box<dyn std::error::Error>> {
        use super::{ChangeMode, SetValues, SettingsDefaults};
        use crate::wire;
        use serde_json::{Value, json};

        let dir = tempfile::tempdir()?;
        let file = dir.path().join("settings.json");
        let downloads = dir.path().join("downloads");
        let defaults = SettingsDefaults::new(vec![downloads.clone()])?;
        let settings = Settings::load(file.clone(), defaults.clone()).await;
        let answer: Value = serde_json::to_value(
            wire::settings_state(&settings, None).map_err(|error| error.message.into_owned())?,
        )?;
        assert_eq!(answer["settings"]["files"]["readDirs"], json!([downloads]));
        assert_eq!(answer["defaults"]["files"]["readDirs"], json!([downloads]));
        settings.patch(br#"{"files":{"readDirs":["/"]}}"#).await?;
        let restored = Settings::load(file.clone(), defaults.clone()).await;
        assert_eq!(restored.read_dirs(), vec![std::path::PathBuf::from("/")]);
        settings
            .change(SetValues::new(), ChangeMode::Replace)
            .await?;
        assert_eq!(settings.read_dirs(), vec![downloads.clone()]);
        assert_eq!(
            Settings::load(file.clone(), defaults.clone())
                .await
                .read_dirs(),
            vec![downloads.clone()]
        );
        tokio::fs::write(
            &file,
            br#"{"files":{"readDirs":["relative"]},"trace":{"retentionSeconds":7200}}"#,
        )
        .await?;
        let fallback = Settings::load(file, defaults).await;
        assert_eq!(fallback.read_dirs(), vec![downloads]);
        assert_eq!(fallback.trace_retention(), Duration::from_secs(7200));
        Ok(())
    }
}
