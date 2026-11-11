//! Where an op sits in Playwright's protocol, as each route row states it.

/// Keys with the reason each one differs from the upstream command, `(key, reason)`.
type Deviations = &'static [(&'static str, &'static str)];

/// An op's place in Playwright 1.64.1's protocol (`channels.d.ts`, vendored), which the
/// generated upstream check compares the op's input and result with. Built only through
/// [`command`], [`neo_only`] and [`replaces`], so every way Neo differs from upstream is a key
/// with a written reason; `op.rs`'s tests refuse an empty or duplicated one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Upstream {
    /// The op carries this command's params, nested under `options` as the prelude sends them,
    /// beside Neo's addressing (`target`, `strict`, `page`, `frame`), which the check sets aside.
    Command(Command),
    /// No upstream command: a client-side method upstream, an upstream event, or Neo's pull model.
    NeoOnly {
        /// Why the op has no command to compare with.
        reason: &'static str,
    },
    /// Upstream has a command for the method, but Neo's params share nothing with it
    /// (`expect.*`'s matcher model). The generated file still names the command, so a command
    /// that vanished upstream fails to resolve.
    Replaces {
        /// Upstream's command, `Interface.method` (`Frame.expect`).
        command: &'static str,
        /// Why Neo's params are its own.
        reason: &'static str,
    },
}

/// One upstream command and every way the op deviates from it. A row starts at [`command`] and
/// names each deviation with its builder, so the deviations stay beside the handler they describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    /// `Interface.method` as `channels.d.ts` declares it (`ElementHandle.click`), which names
    /// its `<Interface><Method>Params`, `Options` and `Result` types.
    pub name: &'static str,
    /// Keys Neo sends that the command lacks: fields the prelude computes (`publication`,
    /// `lease`, `noWait`), which live on the outer input and never in `options`.
    pub adds: Deviations,
    /// Command params Neo never sends.
    pub omits: Deviations,
    /// Keys both carry with a different type on purpose (`waitFor`'s `state`).
    pub retypes: Deviations,
    /// Why Neo answers more than `null` where the command returns nothing (`locator.click`'s
    /// navigation cut). Without it, the generated check holds a void command's op to `null`.
    pub result: Option<&'static str>,
}

/// The row of an op that carries upstream's `name` command (`"ElementHandle.click"`).
pub(crate) const fn command(name: &'static str) -> Command {
    Command {
        name,
        adds: &[],
        omits: &[],
        retypes: &[],
        result: None,
    }
}

/// The row of an op with no upstream command, and why.
pub(crate) const fn neo_only(reason: &'static str) -> Upstream {
    Upstream::NeoOnly { reason }
}

/// The row of an op whose params replace upstream's `command` wholesale, and why.
pub(crate) const fn replaces(command: &'static str, reason: &'static str) -> Upstream {
    Upstream::Replaces { command, reason }
}

impl Command {
    /// Keys Neo sends that the command lacks, each with its reason.
    pub(crate) const fn adds(self, adds: Deviations) -> Self {
        Self { adds, ..self }
    }

    /// Command params Neo never sends, each with its reason.
    pub(crate) const fn omits(self, omits: Deviations) -> Self {
        Self { omits, ..self }
    }

    /// Keys both carry, typed differently on purpose, each with its reason.
    pub(crate) const fn retypes(self, retypes: Deviations) -> Self {
        Self { retypes, ..self }
    }

    /// Why Neo answers a value where the command answers nothing.
    pub(crate) const fn result(self, reason: &'static str) -> Self {
        Self {
            result: Some(reason),
            ..self
        }
    }
}

impl From<Command> for Upstream {
    fn from(command: Command) -> Self {
        Self::Command(command)
    }
}

#[cfg(test)]
impl Upstream {
    /// What the row leaves unexplained: an empty reason, a key named twice, a key both added and
    /// omitted, a command not named `Interface.method`. Empty for a well-formed row.
    pub(crate) fn unexplained(&self) -> Vec<String> {
        use std::collections::HashSet;

        let mut out = Vec::new();
        let mut reason = |what: String, why: &str| {
            if why.trim().is_empty() {
                out.push(format!("{what} has no reason"));
            }
        };
        match *self {
            Self::NeoOnly { reason: why } => reason("neo_only".to_owned(), why),
            Self::Replaces {
                command,
                reason: why,
            } => {
                reason(format!("replaces({command})"), why);
                if !is_command_name(command) {
                    out.push(format!("replaces({command}) is not `Interface.method`"));
                }
            }
            Self::Command(row) => {
                for (list, keys) in [
                    ("adds", row.adds),
                    ("omits", row.omits),
                    ("retypes", row.retypes),
                ] {
                    for (key, why) in keys {
                        reason(format!("{} {list} `{key}`", row.name), why);
                    }
                }
                if let Some(why) = row.result {
                    reason(format!("{} result", row.name), why);
                }
                if !is_command_name(row.name) {
                    out.push(format!("command({}) is not `Interface.method`", row.name));
                }
                let mut named = HashSet::new();
                for (key, _) in row.adds.iter().chain(row.omits).chain(row.retypes) {
                    if !named.insert(*key) {
                        out.push(format!(
                            "{} names `{key}` twice across adds, omits and retypes",
                            row.name
                        ));
                    }
                }
            }
        }
        out
    }
}

/// `Interface.method`, the shape every `channels.d.ts` command has, so the generated file can
/// name `<Interface><Method>Params`.
#[cfg(test)]
fn is_command_name(name: &str) -> bool {
    let Some((interface, method)) = name.split_once('.') else {
        return false;
    };
    interface.starts_with(|c: char| c.is_ascii_uppercase())
        && method.starts_with(|c: char| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
        && !method.contains('.')
}

#[cfg(test)]
mod tests {
    use super::{Command, Upstream, command, neo_only, replaces};

    #[test]
    fn tests_that_each_builder_records_its_deviation_with_the_reason() {
        let row = command("Frame.drop")
            .adds(&[("files", "paths or payloads Rust resolves on Neo's disk")])
            .omits(&[("localPaths", "folded into files")])
            .retypes(&[("position", "Neo's probe point")])
            .result("answers the drop's navigation");
        assert_eq!(
            row,
            Command {
                name: "Frame.drop",
                adds: &[("files", "paths or payloads Rust resolves on Neo's disk")],
                omits: &[("localPaths", "folded into files")],
                retypes: &[("position", "Neo's probe point")],
                result: Some("answers the drop's navigation"),
            },
            "each builder sets its own list and keeps the others"
        );
        assert!(Upstream::from(row).unexplained().is_empty());
    }

    #[test]
    fn tests_that_a_row_without_its_reasons_is_unexplained() {
        let cases: [(Upstream, &str); 6] = [
            (neo_only(" "), "neo_only has no reason"),
            (
                replaces("Frame.expect", ""),
                "replaces(Frame.expect) has no reason",
            ),
            (
                command("Frame.goto").adds(&[("publication", "")]).into(),
                "Frame.goto adds `publication` has no reason",
            ),
            (
                command("ElementHandle.click").result("").into(),
                "ElementHandle.click result has no reason",
            ),
            (
                command("Frame.drop")
                    .adds(&[("files", "a")])
                    .omits(&[("files", "b")])
                    .into(),
                "Frame.drop names `files` twice across adds, omits and retypes",
            ),
            (
                command("locator.click").into(),
                "command(locator.click) is not `Interface.method`",
            ),
        ];
        for (row, why) in cases {
            assert_eq!(
                row.unexplained(),
                [why],
                "{row:?} must be refused for exactly this"
            );
        }
    }
}
