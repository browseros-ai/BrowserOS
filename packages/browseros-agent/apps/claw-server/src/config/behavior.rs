//! The behaviour file (`neo.toml`, named by [`super::BEHAVIOR_FILE_ENV`]): what a deployment can
//! tune about this server, and how a file written for any server version applies to this one.
//!
//! A Docker image pairs its shipped `neo.toml` with whichever claw-server its browser package
//! bundles, so the file and the server are often different versions. A file that parses as TOML
//! therefore never stops the server: a section, key or value this build does not know keeps its
//! default and is reported, naming the nearest known key when it looks like a typo. Only a file
//! that is not TOML at all is fatal, since none of it can be applied.

use super::ServerMode;
use serde::Deserialize;
use std::fmt;

/// What a deployment can tune about how this server behaves, fixed for the process.
///
/// [`ServerMode`] supplies every default and the behaviour file may override any of them. Code
/// asks these fields, not the mode, so a new knob is one field here, one entry in [`KNOBS`] and
/// one commented line in the Docker image's `neo.toml`, rather than another environment variable
/// for every deployment to carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Behavior {
    pub tab_grouping: TabGrouping,
    /// Whether BrowserOS's DOM-only page scripts run in its isolated JavaScript world, which the
    /// page cannot see or tamper with (`browseros_core::page_js`). Off runs every script in the
    /// page's own world, as before isolated worlds; it is the rollout's off switch.
    pub isolated_world: bool,
}

impl Behavior {
    #[must_use]
    pub fn defaults(mode: ServerMode) -> Self {
        Self {
            tab_grouping: match mode {
                ServerMode::Desktop => TabGrouping::Automatic,
                ServerMode::Hosted => TabGrouping::Explicit,
            },
            isolated_world: true,
        }
    }
}

/// The desktop defaults, matching [`ServerMode`]'s own default.
impl Default for Behavior {
    fn default() -> Self {
        Self::defaults(ServerMode::Desktop)
    }
}

/// Who decides which tab group an agent's page joins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabGrouping {
    /// The server keeps each session's pages in a `<client>/<task>` group that it creates,
    /// titles, collapses and closes. The desktop app's behaviour.
    #[serde(rename = "auto")]
    Automatic,
    /// The server never creates, moves, titles or closes a group: a page joins one only when the
    /// call names `groupId`. For hosts whose own harness owns the tab layout.
    Explicit,
}

impl TabGrouping {
    /// The spelling the behaviour file uses.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "auto",
            Self::Explicit => "explicit",
        }
    }

    #[must_use]
    pub fn is_automatic(self) -> bool {
        self == Self::Automatic
    }
}

/// A behaviour file applied to the mode's defaults, with every entry this build could not apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub behavior: Behavior,
    pub ignored: Vec<Ignored>,
}

/// One entry of the behaviour file this server did not apply; the knob kept its default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ignored {
    /// `section` or `section.key`, as the file spells it.
    pub key: String,
    pub reason: String,
}

impl fmt::Display for Ignored {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.key, self.reason)
    }
}

/// One key the behaviour file may set, and how its value lands on [`Behavior`]. A value that does
/// not decode leaves the field as it was.
struct Knob {
    section: &'static str,
    key: &'static str,
    apply: fn(&mut Behavior, toml::Value) -> Result<(), toml::de::Error>,
}

/// Every key the behaviour file may set.
const KNOBS: &[Knob] = &[
    Knob {
        section: "tabs",
        key: "grouping",
        apply: |behavior, value| {
            behavior.tab_grouping = value.try_into()?;
            Ok(())
        },
    },
    Knob {
        section: "scripts",
        key: "isolated_world",
        apply: |behavior, value| {
            behavior.isolated_world = value.try_into()?;
            Ok(())
        },
    },
];

/// Applies a behaviour file to `defaults`. Fails only when `raw` is not TOML.
pub fn read(raw: &str, defaults: Behavior) -> Result<Applied, toml::de::Error> {
    let file: toml::Table = raw.parse()?;
    let mut behavior = defaults;
    let mut ignored = Vec::new();
    for (section, entries) in file {
        let knobs: Vec<&Knob> = KNOBS
            .iter()
            .filter(|knob| knob.section == section)
            .collect();
        if knobs.is_empty() {
            let reason = unknown_section(&section);
            ignored.push(Ignored {
                key: section,
                reason,
            });
            continue;
        }
        let toml::Value::Table(entries) = entries else {
            ignored.push(Ignored {
                reason: format!("expected a table, found {}", entries.type_str()),
                key: section,
            });
            continue;
        };
        for (key, value) in entries {
            let path = format!("{section}.{key}");
            match knobs.iter().find(|knob| knob.key == key) {
                Some(knob) => {
                    if let Err(error) = (knob.apply)(&mut behavior, value) {
                        ignored.push(Ignored {
                            key: path,
                            reason: error.message().to_string(),
                        });
                    }
                }
                None => {
                    let near = closest(&key, knobs.iter().map(|knob| knob.key))
                        .map(|near| format!("{section}.{near}"));
                    ignored.push(Ignored {
                        key: path,
                        reason: unknown("key", near),
                    });
                }
            }
        }
    }
    Ok(Applied { behavior, ignored })
}

fn unknown_section(section: &str) -> String {
    // A key written above every table header lands at the top level, where it reads as a section.
    let misplaced = KNOBS
        .iter()
        .find(|knob| knob.key == section)
        .map(|knob| format!("{}.{}", knob.section, knob.key));
    let near = misplaced
        .or_else(|| closest(section, KNOBS.iter().map(|knob| knob.section)).map(str::to_string));
    unknown("section", near)
}

fn unknown(kind: &str, near: Option<String>) -> String {
    match near {
        Some(near) => format!("unknown {kind}; did you mean `{near}`?"),
        None => format!("unknown {kind}"),
    }
}

/// The candidate within a few edits of `name`, for a typo's suggestion. The allowance grows with
/// the name so a short word is not matched to an unrelated one.
fn closest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let allowance = (name.chars().count() / 3).max(1);
    candidates
        .map(|candidate| (edit_distance(name, candidate), candidate))
        .filter(|(distance, _)| *distance <= allowance)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left_char) in left.chars().enumerate() {
        let mut current = vec![row + 1; right.len() + 1];
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != *right_char);
            current[column + 1] = substitution
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        previous = current;
    }
    previous[right.len()]
}

#[cfg(test)]
mod tests {
    use super::{Behavior, Ignored, KNOBS, TabGrouping, read};
    use crate::config::ServerMode;

    fn hosted() -> Behavior {
        Behavior::defaults(ServerMode::Hosted)
    }

    fn ignored(key: &str, reason: &str) -> Ignored {
        Ignored {
            key: key.to_string(),
            reason: reason.to_string(),
        }
    }

    #[test]
    fn tests_that_known_keys_override_the_defaults_and_absent_ones_keep_them()
    -> Result<(), toml::de::Error> {
        let applied = read(
            "[tabs]\ngrouping = \"auto\"\n[scripts]\nisolated_world = false\n",
            hosted(),
        )?;
        assert_eq!(applied.behavior.tab_grouping, TabGrouping::Automatic);
        assert!(!applied.behavior.isolated_world);
        assert!(applied.ignored.is_empty());

        for unset in [
            "",
            "# [tabs]\n# grouping = \"auto\"\n",
            "[tabs]\n[scripts]\n",
        ] {
            let applied = read(unset, hosted())?;
            assert_eq!(applied.behavior, hosted(), "{unset:?}");
            assert!(applied.ignored.is_empty(), "{unset:?}");
        }
        Ok(())
    }

    #[test]
    fn tests_that_what_a_newer_file_adds_keeps_its_default_and_is_reported()
    -> Result<(), toml::de::Error> {
        let applied = read(
            "[tabs]\ngrouping = \"manual\"\nlimit = 4\n[scripts]\nisolated_world = \"yes\"\n[memory]\nbudget = 1\n",
            hosted(),
        )?;
        assert_eq!(applied.behavior, hosted());
        assert_eq!(
            applied.ignored,
            [
                ignored("memory", "unknown section"),
                ignored(
                    "scripts.isolated_world",
                    "invalid type: string \"yes\", expected a boolean"
                ),
                ignored(
                    "tabs.grouping",
                    "unknown variant `manual`, expected `auto` or `explicit`"
                ),
                ignored("tabs.limit", "unknown key"),
            ]
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_near_miss_names_the_key_it_probably_meant() -> Result<(), toml::de::Error> {
        let applied = read(
            "grouping = \"auto\"\n[tab]\ngrouping = \"auto\"\n[scripts]\nisolated_wrold = false\n",
            hosted(),
        )?;
        assert_eq!(applied.behavior, hosted());
        assert_eq!(
            applied.ignored,
            [
                ignored("grouping", "unknown section; did you mean `tabs.grouping`?"),
                ignored(
                    "scripts.isolated_wrold",
                    "unknown key; did you mean `scripts.isolated_world`?"
                ),
                ignored("tab", "unknown section; did you mean `tabs`?"),
            ]
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_known_section_written_as_a_value_is_reported() -> Result<(), toml::de::Error> {
        let applied = read("tabs = \"auto\"\n", hosted())?;
        assert_eq!(applied.behavior, hosted());
        assert_eq!(
            applied.ignored,
            [ignored("tabs", "expected a table, found string")]
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_file_that_is_not_toml_is_an_error() {
        assert!(read("[tabs\ngrouping = auto\n", hosted()).is_err());
    }

    /// Runtime no longer stops on a typo, so the file this repository ships is checked here:
    /// every key it documents, uncommented, must apply cleanly on this server, and every knob
    /// must be documented in it.
    #[test]
    fn tests_that_the_shipped_docker_file_documents_every_knob_and_each_applies()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../browseros/docker/neo/neo.toml");
        let shipped = std::fs::read_to_string(&path)?;
        let uncommented = shipped
            .lines()
            .map(|line| match line.strip_prefix("# ") {
                Some(rest) if is_table_header(rest) || is_assignment(rest) => rest,
                _ => line,
            })
            .collect::<Vec<_>>()
            .join("\n");

        let applied = read(&uncommented, hosted())?;
        assert!(applied.ignored.is_empty(), "{:?}", applied.ignored);
        let documented: toml::Table = uncommented.parse()?;
        for knob in KNOBS {
            assert!(
                documented
                    .get(knob.section)
                    .and_then(|section| section.get(knob.key))
                    .is_some(),
                "{}.{} is not documented in {}",
                knob.section,
                knob.key,
                path.display()
            );
        }
        Ok(())
    }

    fn is_table_header(line: &str) -> bool {
        line.strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .is_some_and(is_identifier)
    }

    fn is_assignment(line: &str) -> bool {
        line.split_once(" = ")
            .is_some_and(|(key, value)| is_identifier(key) && !value.is_empty())
    }

    fn is_identifier(text: &str) -> bool {
        !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    }
}
