//! What the door reads from one neo-playwright op's declarations: the marks its rules key
//! by, and whether claw-sdk holds the op. This is the one place a crate `Capability` becomes door
//! policy. `playwright_ops` registers each op as [`OpMarks::of`] decides, and the `run`
//! description's held clause comes from the same [`Hold::of`], so the two cannot disagree.

use neo_playwright::{Capability, OpLifetime, OpSpec};

use crate::dispatch::{ErrorCode, Lifetime, Listing, MethodSpec, OperationError, PageScript};

/// One crate op as the door registers it. A plain data bag, built once per op at startup.
pub(super) struct OpMarks {
    /// The crate's name, effect and lifetime, plus the marks the door's rules key
    /// by, derived from the op's capabilities: `page_script` is `MainWorld` for a `MainWorld`
    /// op. Listed under the Playwright line when served, unlisted when held.
    pub(super) spec: MethodSpec,
    /// `Some` when claw-sdk holds the op, which is then registered as its refusal alone.
    pub(super) hold: Option<Hold>,
}

/// Why claw-sdk holds an op, in the words its refusal and the `run` description use. A held op
/// answers `script / method_unavailable` with nothing sent: no page opens, and the crate never
/// sees the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Hold {
    /// What the op does that keeps it held, read after "it" or "each": "changes which tab ...".
    why: &'static str,
    /// What a script does instead, meanwhile.
    meanwhile: &'static str,
}

/// Whether claw-sdk's door polices a capability. claw-sdk serves an op only when the door polices
/// every capability the op declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Policing {
    /// The door supports this capability, applying any required rules through the marks
    /// [`OpMarks::of`] sets from the declaration.
    Ready,
    /// No door rule polices it yet, so every op that declares it is held, for this reason.
    NotYet(Hold),
}

/// How claw-sdk's door polices `capability`. Exhaustive, with no `_` arm, on purpose: a capability
/// the crate adds breaks this build until someone decides how the door polices it, so no op that
/// declares it is ever served by default.
#[expect(
    clippy::match_same_arms,
    reason = "one arm per capability, each naming the guard that polices it, so each flips to \
              Ready in its own reviewed change"
)]
const fn policing(capability: Capability) -> Policing {
    match capability {
        // Main-world scripts pass the door's release recheck as a read of their tab, so a
        // takeover acquired during evaluation still governs the answer.
        Capability::MainWorld => Policing::Ready,
        // The engine implements file reads and uploads; the door adds no path policy.
        Capability::HostFiles => Policing::Ready,
        // Not until the takeover lock refuses a move of the front while the owner holds the
        // screen.
        Capability::Screen => Policing::NotYet(Hold {
            why: "changes which tab Neo's one screen shows, a screen the owner shares",
            meanwhile: "act on the tab where it is, since every SDK call works on a background tab",
        }),
    }
}

/// The hold of an op whose every capability the door polices, but which the crate does not guard
/// at its gateways yet ([`OpSpec::gated`] is false). A door rule cannot stand in for a guard the
/// crate lacks.
const UNGATED: Hold = Hold {
    why: "reaches beyond the page's isolated world before Neo's Playwright guards that reach",
    meanwhile: "reach the page through the rest of the Playwright API",
};

/// Which other Playwright APIs send the ops of a receiver: the prelude's aliases, by the receiver
/// of the op they send. The held clause names those of a receiver only while one of its ops is
/// held, so it never calls an API held that the line offers.
const SENDERS: [Senders; 2] = [
    Senders {
        receiver: "page.",
        phrase: "the `frame` forms send the `page` ops",
    },
    Senders {
        receiver: "locator.",
        phrase: "`$eval`, `$$eval` and element handles send `locator` ones",
    },
];

/// The Playwright APIs that send one receiver's ops besides its own methods. A plain data bag.
struct Senders {
    /// The prefix of the receiver's op names: `page.` or `locator.`.
    receiver: &'static str,
    /// How the held clause names those APIs.
    phrase: &'static str,
}

/// The ops one hold keeps, in the order the crate lists them, so the clause names them together.
struct HeldGroup {
    hold: Hold,
    names: Vec<&'static str>,
}

/// The `run` description's held clause over `specs`, or `None` when none is held. It names each
/// held op, grouped by its hold, with what the op does and what to use meanwhile. It comes from
/// the same [`Hold::of`] that registers each held op, so the description never offers a held op
/// and never calls a served one held.
#[must_use]
pub(super) fn held_clause(specs: impl IntoIterator<Item = OpSpec>) -> Option<String> {
    let mut groups: Vec<HeldGroup> = Vec::new();
    for spec in specs {
        let Some(hold) = Hold::of(&spec) else {
            continue;
        };
        match groups.iter_mut().find(|group| group.hold == hold) {
            Some(group) => group.names.push(spec.name),
            None => groups.push(HeldGroup {
                hold,
                names: vec![spec.name],
            }),
        }
    }
    if groups.is_empty() {
        return None;
    }
    let mut sentences: Vec<String> = groups
        .iter()
        .map(|HeldGroup { hold, names }| {
            let (verb, each) = match names.as_slice() {
                [_] => ("is", "it"),
                _ => ("are", "each"),
            };
            format!(
                "{} {verb} held for now because {each} {}; meanwhile, {}.",
                spoken(names),
                hold.why,
                hold.meanwhile
            )
        })
        .collect();
    let held_senders: Vec<&str> = SENDERS
        .iter()
        .filter(|senders| {
            groups
                .iter()
                .flat_map(|group| &group.names)
                .any(|name| name.starts_with(senders.receiver))
        })
        .map(|senders| senders.phrase)
        .collect();
    sentences.push(if held_senders.is_empty() {
        "A held op fails `method_unavailable` with nothing sent.".to_owned()
    } else {
        format!(
            "A held op fails `method_unavailable` with nothing sent, and so does every API that \
             sends it: {}.",
            held_senders.join(", and ")
        )
    });
    Some(sentences.join(" "))
}

/// `names` in backticks, as a list reads in prose: "`a`", "`a` and `b`", "`a`, `b` and `c`".
fn spoken(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        Some((last, _)) => last.clone(),
        None => String::new(),
    }
}

impl OpMarks {
    /// The marks of the op `spec` declares. Pure: the same spec always gives the same marks.
    /// `line` is the `run` description's line a served op is listed under.
    #[must_use]
    pub(super) fn of(spec: &OpSpec, line: &'static str) -> Self {
        let hold = Hold::of(spec);
        let method = MethodSpec {
            listing: match hold {
                Some(_) => Listing::Hidden,
                None => Listing::Grouped(line),
            },
            lifetime: match spec.lifetime {
                OpLifetime::Operation => Lifetime::Operation,
                OpLifetime::Background => Lifetime::Background,
            },
            page_script: if spec.uses(Capability::MainWorld) {
                PageScript::MainWorld
            } else {
                PageScript::Isolated
            },
            ..MethodSpec::new(spec.name, spec.effect)
        };
        Self { spec: method, hold }
    }
}

impl Hold {
    /// Why claw-sdk holds the op `spec` declares, or `None` when it serves it. It serves an op
    /// only when the crate guards it at its gateways ([`OpSpec::gated`]) and the door polices every
    /// capability it declares. When both fail, the door's reason is the one given, since it says
    /// what the op does.
    #[must_use]
    pub(super) fn of(spec: &OpSpec) -> Option<Self> {
        let unpoliced =
            spec.capabilities
                .iter()
                .find_map(|&capability| match policing(capability) {
                    Policing::Ready => None,
                    Policing::NotYet(hold) => Some(hold),
                });
        match unpoliced {
            Some(hold) => Some(hold),
            None if spec.gated() => None,
            None => Some(UNGATED),
        }
    }

    /// The held op's answer: `script / method_unavailable`, definite, so the door reports it
    /// `not_sent` and retryable. `name` is the crate op, which every alias of the API sends.
    #[must_use]
    pub(super) fn refusal(self, name: &str) -> OperationError {
        OperationError::script(
            ErrorCode::METHOD_UNAVAILABLE,
            format_args!(
                "`{name}` is held for now because it {}. Nothing was sent. Meanwhile, {}.",
                self.why, self.meanwhile
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::path::Path;

    use neo_playwright::{Capability, Op, OpSpec};
    use neo_trace::ErrorOwner;

    use super::{Hold, OpMarks, Policing};
    use crate::dispatch::{ErrorCode, Listing, PageScript};

    /// The line a served op is listed under, in these tests.
    const LINE: &str = "the Playwright API";

    /// Where claw-sdk places an op.
    #[derive(Debug, PartialEq, Eq)]
    enum Placement {
        /// Registered as its refusal alone, and unlisted.
        Held,
        /// Registered as the crate's op, and listed under the Playwright line.
        Served,
    }

    /// An op the crate might add: `page.title`'s spec, which declares nothing, declaring
    /// `capabilities` instead, and marked guarded at the crate's gateways when `policed`. Built
    /// over a real spec, so a field the crate adds to `OpSpec` does not break these tests.
    fn new_op(
        capabilities: &'static [Capability],
        policed: bool,
    ) -> Result<OpSpec, Box<dyn Error>> {
        let base = Op::from_name("page.title")
            .ok_or("page.title is an op")?
            .spec();
        Ok(OpSpec {
            name: "page.newOp",
            capabilities,
            policed,
            ..base
        })
    }

    /// The snapshot of record: every op that declares a capability, whether the crate guards it at
    /// its gateways, which of its capabilities the door polices, and where claw-sdk places it. A
    /// change on either side, the crate's declarations or the door's policing, fails here until
    /// the snapshot is redone on purpose.
    #[test]
    fn claw_sdk_places_every_capability_op() {
        use Capability::{HostFiles, MainWorld};
        use Placement::{Held, Served};
        /// One capability op as claw-sdk places it.
        #[derive(Debug, PartialEq)]
        struct Row {
            op: &'static str,
            declared: Vec<Capability>,
            /// The crate guards it at its gateways ([`OpSpec::gated`]).
            gated: bool,
            /// The capabilities the door polices.
            policed: Vec<Capability>,
            placement: Placement,
        }
        // Every capability op is gated, policed in full by the door, and served: `served` is that
        // row. One that stops being so needs a `Row` of its own here.
        let served = |op, declared: &[Capability]| Row {
            op,
            declared: declared.to_vec(),
            gated: true,
            policed: declared.to_vec(),
            placement: Served,
        };
        let expected = vec![
            served("locator.drop", &[HostFiles]),
            served("locator.setInputFiles", &[HostFiles]),
            served("page.setContent", &[MainWorld]),
            served("page.addScriptTag", &[MainWorld, HostFiles]),
            served("page.addStyleTag", &[MainWorld, HostFiles]),
            served("page.evaluate", &[MainWorld]),
            served("worker.evaluate", &[MainWorld]),
            served("locator.evaluate", &[MainWorld]),
            served("page.evalOnSelector", &[MainWorld]),
            served("locator.evaluateAll", &[MainWorld]),
            served("page.waitForFunction", &[MainWorld]),
            served("locator.waitForFunction", &[MainWorld]),
            served("page.addInitScript", &[MainWorld, HostFiles]),
            served("page.exposeBinding", &[MainWorld]),
            served("bindingCall.resolve", &[MainWorld]),
            served("context.addInitScript", &[MainWorld, HostFiles]),
            served("context.exposeBinding", &[MainWorld]),
        ];
        let placed: Vec<Row> = Op::all()
            .map(Op::spec)
            .filter(|spec| !spec.capabilities.is_empty())
            .map(|spec| Row {
                op: spec.name,
                declared: spec.capabilities.to_vec(),
                gated: spec.gated(),
                policed: spec
                    .capabilities
                    .iter()
                    .copied()
                    .filter(|&capability| super::policing(capability) == Policing::Ready)
                    .collect(),
                placement: match OpMarks::of(&spec, LINE).hold {
                    Some(_) => Held,
                    None => Served,
                },
            })
            .collect();
        assert_eq!(placed, expected);
    }

    /// A door rule cannot stand in for a guard the crate lacks: an op the crate has not guarded at
    /// its gateways is held whether or not the door polices what it declares.
    #[test]
    fn an_ungated_op_is_held_whatever_the_door_polices() -> Result<(), Box<dyn Error>> {
        use Capability::{HostFiles, MainWorld, Screen};
        let declarations: [&'static [Capability]; 4] = [
            &[MainWorld],
            &[HostFiles],
            &[Screen],
            &[HostFiles, MainWorld],
        ];
        for capabilities in declarations {
            let ungated = new_op(capabilities, false)?;
            let marks = OpMarks::of(&ungated, LINE);
            let hold = marks
                .hold
                .ok_or_else(|| format!("{capabilities:?}: an ungated op is held"))?;
            assert_eq!(
                marks.spec.listing,
                Listing::Hidden,
                "{capabilities:?}: the description does not offer it"
            );
            let refusal = hold.refusal(ungated.name);
            assert_eq!(
                (refusal.owner(), refusal.code()),
                (ErrorOwner::Script, ErrorCode::METHOD_UNAVAILABLE),
                "{capabilities:?}"
            );
        }
        // The same main-world op, once the crate guards it, is served: the door polices it.
        let gated = OpMarks::of(&new_op(&[MainWorld], true)?, LINE);
        assert_eq!(gated.hold, None);
        assert_eq!(gated.spec.listing, Listing::Grouped(LINE));
        Ok(())
    }

    /// While the door does not police a capability, every op that declares it is held, guarded
    /// by the crate or not, alone or beside a capability the door does police. On this door
    /// main-world scripts and host files are policed, and the screen is not.
    #[test]
    fn a_capability_the_door_does_not_police_yet_holds_every_op_declaring_it()
    -> Result<(), Box<dyn Error>> {
        use Capability::{HostFiles, MainWorld, Screen};
        let unpoliced: [&'static [Capability]; 3] =
            [&[Screen], &[MainWorld, Screen], &[HostFiles, Screen]];
        for capabilities in unpoliced {
            let marks = OpMarks::of(&new_op(capabilities, true)?, LINE);
            assert!(
                marks.hold.is_some(),
                "{capabilities:?}: held, though the crate guards it"
            );
            assert_eq!(marks.spec.listing, Listing::Hidden, "{capabilities:?}");
        }
        // What the door polices is served once the crate guards it, and so is an op that declares
        // nothing, which the crate counts as guarded whatever its mark says.
        for (capabilities, policed) in [
            (&[MainWorld][..], true),
            (&[HostFiles], true),
            (&[HostFiles, MainWorld], true),
            (&[], true),
            (&[], false),
        ] {
            let marks = OpMarks::of(&new_op(capabilities, policed)?, LINE);
            assert_eq!(marks.hold, None, "{capabilities:?}, policed: {policed}");
            assert_eq!(marks.spec.listing, Listing::Grouped(LINE));
        }
        Ok(())
    }

    /// The clause says which other APIs are held with a held op by the op's receiver, and names
    /// no sender of a receiver whose ops are all served: the line offers `$eval` while only a
    /// `page` op is held.
    #[test]
    fn the_held_clause_names_the_senders_of_held_ops_alone() -> Result<(), Box<dyn Error>> {
        let page_op = new_op(&[Capability::MainWorld], false)?;
        let locator_op = OpSpec {
            name: "locator.newOp",
            ..page_op
        };
        let page_only = super::held_clause([page_op]).ok_or("an ungated op is held")?;
        assert!(
            page_only.starts_with("`page.newOp` is held")
                && page_only.contains("the `frame` forms send the `page` ops")
                && !page_only.contains("$eval"),
            "{page_only}"
        );
        let both = super::held_clause([page_op, locator_op]).ok_or("both are held")?;
        assert!(
            both.contains("`$eval`, `$$eval` and element handles"),
            "{both}"
        );
        Ok(())
    }

    /// The door releases a main-world script's answer as a read of its tab, so the mark comes from
    /// the crate's declaration: no list of op names can miss a script the crate adds.
    #[test]
    fn a_main_world_op_is_a_page_script_and_no_other_op_is() {
        let mut scripts = 0;
        for spec in Op::all().map(Op::spec) {
            let expected = if spec.uses(Capability::MainWorld) {
                scripts += 1;
                PageScript::MainWorld
            } else {
                PageScript::Isolated
            };
            assert_eq!(
                OpMarks::of(&spec, LINE).spec.page_script,
                expected,
                "{}",
                spec.name
            );
        }
        assert!(scripts > 0, "the crate declares its main-world scripts");
    }

    /// What a BrowserOS case proves of its op on a real browser.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Proves {
        /// The op runs as Playwright 1.64.1's does, through the APIs that send it.
        Served,
        /// The op stays held: every API that sends it fails `method_unavailable`, nothing sent.
        Held,
    }

    /// One BrowserOS case: a test in `tests/<file>` that drives `op` on a real BrowserOS. A plain
    /// data bag.
    struct BrowserosCase {
        op: &'static str,
        file: &'static str,
        test: &'static str,
        proves: Proves,
    }

    /// The BrowserOS cases of the ops that reach past their page's isolated world into the main
    /// world, Neo's files or Neo's screen, where no fake browser shows what a real one does.
    const BROWSEROS_CASES: &[BrowserosCase] = &[
        BrowserosCase {
            op: "worker.evaluate",
            file: "main_world_browseros.rs",
            test: "worker_evaluate_behaves_as_playwright_on_browseros",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "page.addInitScript",
            file: "init_scripts_browseros.rs",
            test: "init_scripts_run_in_every_new_document_until_the_run_ends",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "context.addInitScript",
            file: "init_scripts_browseros.rs",
            test: "init_scripts_run_in_every_new_document_until_the_run_ends",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "page.addScriptTag",
            file: "main_world_browseros.rs",
            test: "tags_behave_as_playwright_on_browseros",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "page.addStyleTag",
            file: "main_world_browseros.rs",
            test: "tags_behave_as_playwright_on_browseros",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "page.exposeBinding",
            file: "init_scripts_browseros.rs",
            test: "pages_call_back_into_the_script_until_the_run_ends",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "context.exposeBinding",
            file: "init_scripts_browseros.rs",
            test: "pages_call_back_into_the_script_until_the_run_ends",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "bindingCall.resolve",
            file: "init_scripts_browseros.rs",
            test: "pages_call_back_into_the_script_until_the_run_ends",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "page.evaluate",
            file: "main_world_browseros.rs",
            test: "page_evaluate_behaves_as_playwright_on_browseros",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "page.evaluate",
            file: "main_world_browseros.rs",
            test: "an_owner_lock_that_blocks_reads_withholds_what_a_running_script_read",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "page.waitForFunction",
            file: "main_world_browseros.rs",
            test: "wait_for_function_behaves_as_playwright_on_browseros",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "locator.waitForFunction",
            file: "main_world_browseros.rs",
            test: "locator_wait_for_function_behaves_as_playwright_on_browseros",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "locator.evaluate",
            file: "main_world_browseros.rs",
            test: "locator_evaluate_and_dollar_eval_behave_as_playwright_on_browseros",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "page.evalOnSelector",
            file: "main_world_browseros.rs",
            test: "locator_evaluate_and_dollar_eval_behave_as_playwright_on_browseros",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "locator.evaluateAll",
            file: "main_world_browseros.rs",
            test: "evaluate_all_and_dollar_dollar_eval_behave_as_playwright_on_browseros",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "page.setContent",
            file: "main_world_browseros.rs",
            test: "set_content_behaves_as_playwright_on_browseros",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "locator.drop",
            file: "drop_browseros.rs",
            test: "external_drop_preserves_files_and_mime_data",
            proves: Proves::Served,
        },
        BrowserosCase {
            op: "locator.setInputFiles",
            file: "uploads_browseros.rs",
            test: "host_owned_paths_and_payloads_upload_and_an_empty_list_clears_the_input",
            proves: Proves::Served,
        },
    ];

    /// The capabilities whose every op needs a BrowserOS case.
    const ON_BROWSEROS: [Capability; 3] = [
        Capability::MainWorld,
        Capability::HostFiles,
        Capability::Screen,
    ];

    /// Whether `source` defines `test` as a test: `fn test(` right under a test attribute.
    fn defines_test(source: &str, test: &str) -> bool {
        let heads = [format!("async fn {test}("), format!("fn {test}(")];
        let lines: Vec<&str> = source.lines().map(str::trim).collect();
        lines.windows(2).any(|pair| match pair {
            [attribute, signature] => {
                (attribute.starts_with("#[tokio::test") || *attribute == "#[test]")
                    && heads
                        .iter()
                        .any(|head| signature.starts_with(head.as_str()))
            }
            _ => false,
        })
    }

    /// The coverage tripwire (decision 35a). Every op the crate marks `MainWorld`, `HostFiles` or
    /// `Screen` has a BrowserOS case that proves where the door places it now, served or held, and
    /// every case still proves that of its op. So an op the crate adds with such a capability turns
    /// this red until it has a case, and so does an op the door starts serving, or holds again,
    /// until a case proves it so.
    #[test]
    fn every_capability_op_has_a_browseros_case() -> Result<(), Box<dyn Error>> {
        let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let reaching = |spec: &OpSpec| ON_BROWSEROS.iter().any(|&capability| spec.uses(capability));
        let placed = |spec: &OpSpec| match Hold::of(spec) {
            Some(_) => Proves::Held,
            None => Proves::Served,
        };
        for spec in Op::all().map(Op::spec).filter(|spec| reaching(spec)) {
            let placement = placed(&spec);
            assert!(
                BROWSEROS_CASES
                    .iter()
                    .any(|case| case.op == spec.name && case.proves == placement),
                "{} is {placement:?} on this door, and no BrowserOS case proves it: add one to \
                 BROWSEROS_CASES",
                spec.name
            );
        }
        for case in BROWSEROS_CASES {
            let spec = Op::from_name(case.op)
                .ok_or_else(|| format!("{} is no crate op", case.op))?
                .spec();
            assert!(
                reaching(&spec),
                "{}: it reaches nothing a BrowserOS case is kept for",
                case.op
            );
            assert_eq!(
                case.proves,
                placed(&spec),
                "{} ({}): the case proves what the door no longer does",
                case.op,
                case.test
            );
            assert!(
                case.file.ends_with("_browseros.rs"),
                "{}: a BrowserOS suite",
                case.file
            );
            let source = fs::read_to_string(tests.join(case.file))
                .map_err(|error| format!("tests/{}: {error}", case.file))?;
            assert!(
                defines_test(&source, case.test),
                "{} has no test `{}`",
                case.file,
                case.test
            );
        }
        Ok(())
    }
}
