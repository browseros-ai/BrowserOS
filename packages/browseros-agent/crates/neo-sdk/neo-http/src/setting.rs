//! The table of every setting a host can make (`/v1/settings`), and how one value is checked.
//!
//! Adding a setting is one row in [`SETTINGS`], its key in the `Settings` schema of
//! `openapi.yaml`, and a reader where the server uses it. `GET`, `PUT`, `PATCH`, the checks, the
//! saved file and the README's table all work from the rows; a test keeps the three lists equal.
//!
//! The rules are protobuf's for fields, which the owner asked for: every setting is optional and
//! has a default, a name the server does not know is skipped and reported (never refused), and
//! a setting never changes its name, its type or its meaning. A retired setting keeps its row
//! and is read by nobody, so its name is never reused.

use crate::limits::{IDLE_CLOSE_SECONDS, RETENTION_SECONDS, VALUE_BYTES};
use serde_json::Value;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use crate::settings::SettingsDefaults;

/// One row of the table: a setting's dotted name, which is its path in the JSON, and its rule.
#[derive(Debug)]
pub(crate) struct Setting {
    /// `group.key`, as `tabs.idleCloseSeconds`. Nested the same way in every body, answer and
    /// the saved file.
    pub(crate) name: &'static str,
    /// What the value may be, and its default.
    pub(crate) rule: Rule,
    /// A saved value that cannot be used takes this value instead of the normal default.
    /// Only recording switches opt in, so malformed input cannot start recording or shorten retention.
    pub(crate) invalid_saved: Option<SettingValue>,
}

/// What one setting's value may be.
#[derive(Debug)]
pub(crate) enum Rule {
    /// A whole number within `range`.
    Whole {
        /// Used when the host set nothing: today's behaviour.
        default: u64,
        /// The values a host may set, both ends included.
        range: RangeInclusive<u64>,
    },
    /// One name from an open list: a later server may know more names than `values`. A name
    /// this server does not know is skipped, as an unknown setting is, and never refused.
    Choice {
        /// Used when the host set nothing: today's behaviour, unless the row says otherwise.
        default: &'static str,
        /// The names this server knows.
        values: &'static [&'static str],
    },
    /// Absolute read roots. The application supplies the downloads default, since the library
    /// does not know the browser's filesystem layout.
    AbsoluteDirs {
        /// The largest accepted list.
        max: usize,
    },
}

/// One setting's value, as checked against its rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SettingValue {
    /// A [`Rule::Whole`] value.
    Whole(u64),
    /// A [`Rule::Choice`] value, one of its `values`.
    Choice(&'static str),
    /// A [`Rule::AbsoluteDirs`] value.
    Dirs(Vec<PathBuf>),
}

/// What a value sent for a known setting is, against its rule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Checked {
    /// A value the setting takes.
    Value(SettingValue),
    /// A string that is no name this server knows for an open list: skipped and named in
    /// `unknownValues`, as a name from a newer server would be.
    UnknownValue,
    /// A value the rule refuses, with the rule worded to follow the setting's name.
    Refused(String),
}

/// Neo closes an agent's tab once nothing used it for this long (the idle-tab janitor).
pub(crate) const TAB_IDLE_CLOSE: Setting = Setting {
    name: "tabs.idleCloseSeconds",
    invalid_saved: None,
    rule: Rule::Whole {
        default: 30 * 60,
        range: IDLE_CLOSE_SECONDS,
    },
};

/// Runs older than this are deleted from the trace (the retention sweep). The default is
/// `neo_trace::RetentionPolicy`'s own `max_age`, which a test keeps equal.
pub(crate) const TRACE_RETENTION: Setting = Setting {
    name: "trace.retentionSeconds",
    invalid_saved: None,
    rule: Rule::Whole {
        default: 7 * 24 * 60 * 60,
        range: RETENTION_SECONDS,
    },
};

/// Neo's own screenshots: `perOperation`, or `off` (the trace writer's `RecordPolicy`).
pub(crate) const TRACE_SCREENSHOTS: Setting = Setting {
    name: "trace.screenshots",
    invalid_saved: Some(SettingValue::Choice(SCREENSHOTS_OFF)),
    rule: Rule::Choice {
        default: SCREENSHOTS_PER_OPERATION,
        values: &[SCREENSHOTS_PER_OPERATION, SCREENSHOTS_OFF],
    },
};

/// The size up to which the trace keeps values whole; `0` keeps the trace writer's defaults.
pub(crate) const TRACE_VALUE_BYTES: Setting = Setting {
    name: "trace.valueBytes",
    invalid_saved: None,
    rule: Rule::Whole {
        default: 0,
        range: VALUE_BYTES,
    },
};

/// `trace.screenshots`'s names.
pub(crate) const SCREENSHOTS_PER_OPERATION: &str = "perOperation";
pub(crate) const SCREENSHOTS_OFF: &str = "off";

/// Whether the trace keeps script and host values.
pub(crate) const TRACE_VALUES: Setting = Setting {
    name: "trace.values",
    invalid_saved: Some(SettingValue::Choice("off")),
    rule: Rule::Choice {
        default: "on",
        values: &["on", "off"],
    },
};

/// Whether a tab that took a host's private fill may be pictured (protected-fill ruling R58):
/// `allowed`, like any other tab; `never`, no picture of it for the tab's life, by a script or by
/// Neo, kept across a restart by the record of filled tabs. The default is `allowed`, the owner's
/// decision of 2026-11-13, which changes what the server did before it: a host that wants the old
/// rule sets `never` before its first fill.
pub(crate) const FILL_PICTURES: Setting = Setting {
    name: "fill.pictures",
    // A saved value this build cannot use reads as the default, `allowed`, with the load's loud
    // warning (the lead's decision, protected-fill spec R58, "A saved settings file at start"). A
    // bad settings file never costs the product a function: a recording switch falls to `off`
    // because that loses nothing a run does, and this one falls to `allowed` because `never`
    // refuses pictures, and refuses a fill when its record cannot be written. A host that needs
    // `never` reads `GET /v1/settings` after start.
    invalid_saved: None,
    rule: Rule::Choice {
        default: FILL_PICTURES_ALLOWED,
        values: &[FILL_PICTURES_ALLOWED, FILL_PICTURES_NEVER],
    },
};

/// `fill.pictures`'s names.
pub(crate) const FILL_PICTURES_ALLOWED: &str = "allowed";
pub(crate) const FILL_PICTURES_NEVER: &str = "never";

/// The live folders scripts may read by path, with the application-supplied downloads default.
pub(crate) const FILE_READ_DIRS: Setting = Setting {
    name: "files.readDirs",
    invalid_saved: None,
    rule: Rule::AbsoluteDirs { max: 16 },
};

/// Every setting this server knows, in the order the README lists them.
pub(crate) const SETTINGS: &[Setting] = &[
    TAB_IDLE_CLOSE,
    TRACE_RETENTION,
    TRACE_SCREENSHOTS,
    TRACE_VALUE_BYTES,
    TRACE_VALUES,
    FILE_READ_DIRS,
    FILL_PICTURES,
];

impl Setting {
    /// The row named `name`, if this server knows it.
    pub(crate) fn find(name: &str) -> Option<&'static Self> {
        SETTINGS.iter().find(|setting| setting.name == name)
    }

    /// Whether `prefix` is a group some setting's name sits under (`tabs` for
    /// `tabs.idleCloseSeconds`): its value is then an object of settings.
    pub(crate) fn is_group(prefix: &str) -> bool {
        SETTINGS.iter().any(|setting| {
            setting
                .name
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('.'))
        })
    }

    /// The value used when the host set nothing.
    pub(crate) fn default_value(&self, defaults: &SettingsDefaults) -> SettingValue {
        match self.rule {
            Rule::Whole { default, .. } => SettingValue::Whole(default),
            Rule::Choice { default, .. } => SettingValue::Choice(default),
            Rule::AbsoluteDirs { .. } => SettingValue::Dirs(defaults.read_dirs.clone()),
        }
    }

    /// `value` checked against the rule. A refusal's rule never quotes the value, as the other
    /// routes never do.
    pub(crate) fn check(&self, value: &Value) -> Checked {
        match &self.rule {
            Rule::Whole { range, .. } => value
                .as_u64()
                .filter(|whole| range.contains(whole))
                .map_or_else(
                    || {
                        Checked::Refused(format!(
                            "must be a whole number from {} to {}",
                            range.start(),
                            range.end()
                        ))
                    },
                    |whole| Checked::Value(SettingValue::Whole(whole)),
                ),
            // Only a string can be a name a newer server knows; any other type is wrong here.
            Rule::Choice { values, .. } => match value.as_str() {
                Some(name) => values
                    .iter()
                    .find(|known| **known == name)
                    .map_or(Checked::UnknownValue, |known| {
                        Checked::Value(SettingValue::Choice(known))
                    }),
                None => Checked::Refused(format!("must be one of {}", self.values_text())),
            },
            Rule::AbsoluteDirs { max } => {
                let dirs = value
                    .as_array()
                    .filter(|dirs| dirs.len() <= *max)
                    .and_then(|dirs| {
                        dirs.iter()
                            .map(|dir| {
                                let path = PathBuf::from(dir.as_str()?);
                                path.is_absolute().then_some(path)
                            })
                            .collect::<Option<Vec<_>>>()
                    });
                dirs.map_or_else(
                    || {
                        Checked::Refused(format!(
                            "must be a list of at most {max} absolute folders"
                        ))
                    },
                    |dirs| Checked::Value(SettingValue::Dirs(dirs)),
                )
            }
        }
    }

    /// The names an open list's setting takes, for a refusal or a warning: `perOperation, off`.
    /// Empty for any other rule.
    pub(crate) fn values_text(&self) -> String {
        match &self.rule {
            Rule::Choice { values, .. } => values.join(", "),
            Rule::Whole { .. } | Rule::AbsoluteDirs { .. } => String::new(),
        }
    }

    /// `value` as JSON, for an answer and the saved file.
    pub(crate) fn json(value: &SettingValue) -> Value {
        match value {
            SettingValue::Whole(whole) => Value::from(*whole),
            SettingValue::Choice(name) => Value::from(*name),
            SettingValue::Dirs(dirs) => Value::Array(
                dirs.iter()
                    .map(|dir| Value::from(dir.to_string_lossy().into_owned()))
                    .collect(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    //! The table, the contract's `Settings` schema and the README's table of settings name the
    //! same settings with the same defaults and ranges. A setting in one and not the others
    //! would be dropped from an answer, refused by a client's generated types, or undocumented.

    use super::{Rule, SETTINGS};
    use std::collections::BTreeMap;
    use std::error::Error;
    use std::path::Path;

    /// A setting's facts as one list states them: default and range, as text.
    type Listed = BTreeMap<String, (String, String)>;

    /// The table's own rows.
    fn table() -> Listed {
        SETTINGS
            .iter()
            .map(|setting| match &setting.rule {
                Rule::Whole { default, range } => (
                    setting.name.to_owned(),
                    (
                        default.to_string(),
                        format!("{} to {}", range.start(), range.end()),
                    ),
                ),
                Rule::Choice { default, values } => (
                    setting.name.to_owned(),
                    ((*default).to_owned(), values.join(", ")),
                ),
                Rule::AbsoluteDirs { max } => (
                    setting.name.to_owned(),
                    ("downloads folder".to_owned(), format!("0 to {max} folders")),
                ),
            })
            .collect()
    }

    /// The lines of the top-level schema `name` in `components.schemas`, after its own line.
    fn schema<'a>(yaml: &'a str, name: &str) -> Result<Vec<&'a str>, Box<dyn Error>> {
        let header = format!("    {name}:");
        let mut lines = yaml.lines().skip_while(|line| *line != header);
        lines
            .next()
            .ok_or(format!("openapi.yaml has no schema {name}"))?;
        Ok(lines
            .take_while(|line| line.is_empty() || line.starts_with("     "))
            .collect())
    }

    /// The properties of a schema's lines (those under its `properties:`), each with its own
    /// indented lines.
    fn properties<'a>(lines: &[&'a str]) -> Vec<(String, Vec<&'a str>)> {
        let mut found: Vec<(String, Vec<&'a str>)> = Vec::new();
        let under = lines
            .iter()
            .skip_while(|line| **line != "      properties:")
            .skip(1)
            .take_while(|line| line.is_empty() || line.starts_with("       "));
        for line in under {
            let property = line
                .strip_prefix("        ")
                .filter(|rest| !rest.starts_with(' '))
                .and_then(|rest| rest.strip_suffix(':'));
            match (property, found.last_mut()) {
                (Some(property), _) => found.push((property.to_owned(), Vec::new())),
                (None, Some((_, own))) if line.starts_with("          ") => own.push(line.trim()),
                (None, _) => {}
            }
        }
        found
    }

    /// The `Settings` schema of the contract: each group's `$ref`, then its keys' bounds.
    fn contract(yaml: &str) -> Result<Listed, Box<dyn Error>> {
        let mut listed = Listed::new();
        for (group, own) in properties(&schema(yaml, "Settings")?) {
            let reference = own
                .iter()
                .find_map(|line| line.strip_prefix("$ref: '#/components/schemas/"))
                .and_then(|rest| rest.strip_suffix('\''))
                .ok_or(format!("Settings.{group} is a $ref to a schema"))?;
            for (key, own) in properties(&schema(yaml, reference)?) {
                let field = |name: &str| {
                    own.iter()
                        .find_map(|line| line.strip_prefix(name))
                        .map(str::to_owned)
                        .unwrap_or_default()
                };
                // An open list states its names as `x-extensible-enum: [a, b]`; a number, its
                // bounds. A folder list has an application-owned default and a size bound.
                let names = field("x-extensible-enum: [");
                let (default, range) = if field("type: ") == "array" {
                    (
                        field("x-neo-default: "),
                        format!("0 to {} folders", field("maxItems: ")),
                    )
                } else {
                    let range = match names.strip_suffix(']') {
                        Some(names) => names.to_owned(),
                        None => format!("{} to {}", field("minimum: "), field("maximum: ")),
                    };
                    (field("default: "), range)
                };
                listed.insert(format!("{group}.{key}"), (default, range));
            }
        }
        Ok(listed)
    }

    /// The README's table of settings: in its Settings section, the rows whose first cell is a
    /// backquoted dotted name.
    fn readme(text: &str) -> Listed {
        text.lines()
            .skip_while(|line| *line != "### Settings")
            .skip(1)
            .take_while(|line| !line.starts_with('#'))
            .filter_map(|line| {
                let cells: Vec<&str> = line.split('|').map(str::trim).collect();
                let name = cells.get(1)?.strip_prefix('`')?.strip_suffix('`')?;
                // Names in a cell are backquoted: `perOperation`, `off`.
                let cell = |index: usize| {
                    cells
                        .get(index)
                        .copied()
                        .unwrap_or_default()
                        .replace('`', "")
                };
                name.contains('.')
                    .then(|| (name.to_owned(), (cell(2), cell(3))))
            })
            .collect()
    }

    #[test]
    fn the_table_the_contract_and_the_readme_list_the_same_settings() -> Result<(), Box<dyn Error>>
    {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("neo-http sits in crates/neo-sdk")?;
        let yaml = std::fs::read_to_string(crates.join("neo-sdk-api/openapi.yaml"))?;
        let text = std::fs::read_to_string(crates.join("neo-server/README.md"))?;
        let table = table();
        assert!(!table.is_empty(), "the table has rows");
        assert_eq!(
            contract(&yaml)?,
            table,
            "openapi.yaml's Settings schema names every setting, with the table's default, minimum and maximum"
        );
        assert_eq!(
            readme(&text),
            table,
            "neo-server/README.md's table of settings has a row for every setting, with its default and range"
        );
        Ok(())
    }
}
