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

use crate::limits::{IDLE_CLOSE_SECONDS, RETENTION_SECONDS};
use serde_json::Value;
use std::ops::RangeInclusive;

/// One row of the table: a setting's dotted name, which is its path in the JSON, and its rule.
#[derive(Debug)]
pub(crate) struct Setting {
    /// `group.key`, as `tabs.idleCloseSeconds`. Nested the same way in every body, answer and
    /// the saved file.
    pub(crate) name: &'static str,
    /// What the value may be, and its default.
    pub(crate) rule: Rule,
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
}

/// One setting's value, as checked against its rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingValue {
    /// A [`Rule::Whole`] value.
    Whole(u64),
}

/// Neo closes an agent's tab once nothing used it for this long (the idle-tab janitor).
pub(crate) const TAB_IDLE_CLOSE: Setting = Setting {
    name: "tabs.idleCloseSeconds",
    rule: Rule::Whole {
        default: 30 * 60,
        range: IDLE_CLOSE_SECONDS,
    },
};

/// Runs older than this are deleted from the trace (the retention sweep). The default is
/// `neo_trace::RetentionPolicy`'s own `max_age`, which a test keeps equal.
pub(crate) const TRACE_RETENTION: Setting = Setting {
    name: "trace.retentionSeconds",
    rule: Rule::Whole {
        default: 7 * 24 * 60 * 60,
        range: RETENTION_SECONDS,
    },
};

/// Every setting this server knows, in the order the README lists them.
pub(crate) const SETTINGS: &[Setting] = &[TAB_IDLE_CLOSE, TRACE_RETENTION];

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
    pub(crate) const fn default_value(&self) -> SettingValue {
        match self.rule {
            Rule::Whole { default, .. } => SettingValue::Whole(default),
        }
    }

    /// `value` checked against the rule, or the rule it broke, worded to follow the setting's
    /// name in a message. The rule never quotes the value, as the other routes never do.
    pub(crate) fn check(&self, value: &Value) -> Result<SettingValue, String> {
        match &self.rule {
            Rule::Whole { range, .. } => value
                .as_u64()
                .filter(|whole| range.contains(whole))
                .map(SettingValue::Whole)
                .ok_or_else(|| {
                    format!(
                        "must be a whole number from {} to {}",
                        range.start(),
                        range.end()
                    )
                }),
        }
    }

    /// `value` as JSON, for an answer and the saved file.
    pub(crate) fn json(value: SettingValue) -> Value {
        match value {
            SettingValue::Whole(whole) => Value::from(whole),
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
                listed.insert(
                    format!("{group}.{key}"),
                    (
                        field("default: "),
                        format!("{} to {}", field("minimum: "), field("maximum: ")),
                    ),
                );
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
                name.contains('.').then(|| {
                    (
                        name.to_owned(),
                        (
                            cells.get(2).copied().unwrap_or_default().to_owned(),
                            cells.get(3).copied().unwrap_or_default().to_owned(),
                        ),
                    )
                })
            })
            .collect()
    }

    #[test]
    fn the_table_the_contract_and_the_readme_list_the_same_settings() -> Result<(), Box<dyn Error>>
    {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("neo-service sits in crates/neo-sdk")?;
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
