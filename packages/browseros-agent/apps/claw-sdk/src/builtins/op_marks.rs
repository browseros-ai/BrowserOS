//! What the door reads from one browseros-playwright op's declarations: the marks its rules key
//! by, and whether claw-sdk holds the op. This is the one place a crate `Capability` becomes door
//! policy. `playwright_ops` registers each op as [`OpMarks::of`] decides, and the `run`
//! description's held clause comes from the same [`Hold::of`], so the two cannot disagree.

use browseros_playwright::{Capability, OpLifetime, OpSpec};

use crate::dispatch::{ErrorCode, Lifetime, Listing, MethodSpec, OperationError, Output};

/// One crate op as the door registers it. A plain data bag, built once per op at startup.
pub(super) struct OpMarks {
    /// The crate's name, effect, lifetime and sensitive args, plus the marks the door's rules key
    /// by, derived from the op's capabilities: `output` is `Pixels` for a `Capture` op. Listed
    /// under the Playwright line when served, unlisted when held.
    pub(super) spec: MethodSpec,
    /// `Some` when claw-sdk holds the op, which is then registered as its refusal alone.
    pub(super) hold: Option<Hold>,
}

/// Why claw-sdk holds an op, in the words its refusal and the `run` description use. A held op
/// answers `script / method_unavailable` with nothing sent: no page opens, and the crate never
/// sees the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Hold {
    /// What the op does that keeps it held, read after "it" or "each": "runs script in ...".
    why: &'static str,
    /// What a script does instead, meanwhile.
    meanwhile: &'static str,
}

/// Whether claw-sdk's door polices a capability. claw-sdk serves an op only when the door polices
/// every capability the op declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Policing {
    /// A door rule polices every op that declares it, keyed by a `MethodSpec` mark that
    /// [`OpMarks::of`] sets from the declaration.
    Ready,
    /// No door rule polices it yet, so every op that declares it is held, for this reason.
    NotYet(Hold),
}

/// How claw-sdk's door polices `capability`. Exhaustive, with no `_` arm, on purpose: a capability
/// the crate adds breaks this build until someone decides how the door polices it, so no op that
/// declares it is ever served by default.
const fn policing(capability: Capability) -> Policing {
    match capability {
        // `Output::Pixels`: tab privacy refuses a capture of a private tab, and holds sdk-traces'
        // `CaptureGuard` until the pixels go out (`PrivacyHooks`).
        Capability::Capture => Policing::Ready,
        // Not until the door masks every reply into the realm with what its run holds, and
        // releases a main-world result only as a read of its tab: such a script reads fields no
        // seal masks.
        Capability::MainWorld => Policing::NotYet(Hold {
            why: "runs script in the page's main world, where no seal masks a password, card or \
                  code field",
            meanwhile: "read the page through locators (`textContent`, `innerText`, \
                        `getAttribute`, `inputValue`), `page.content()` or `page.title()`, wait \
                        through a locator, `page.waitForURL()` or `page.waitForLoadState()`, and \
                        open HTML as a page of its own with \
                        `page.goto('data:text/html,' + encodeURIComponent(html))`, whose scripts \
                        run in an origin of their own",
        }),
        // Not until the door names Neo's own secret files (its host token, CAPTCHA keys, trace
        // store and browser profile) for the crate's upload to refuse.
        Capability::HostFiles => Policing::NotYet(Hold {
            why: "hands the page files from Neo's own disk, where Neo keeps its secrets",
            meanwhile: "no op gives a page a file, so tell the user the upload needs them",
        }),
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
    why: "reaches beyond the page's sealed world before Neo's Playwright guards that reach",
    meanwhile: "reach the page through the rest of the Playwright API",
};

/// How every held op answers, and which Playwright APIs are held with it. Named by receiver, not
/// by op, so it stays true whichever ops are held.
const HELD_TAIL: &str = "A held op fails `method_unavailable` with nothing sent, and so does \
    every API that sends it: the `frame` forms send the `page` ops, and `$eval`, `$$eval`, \
    `setInputFiles` and element handles send `locator` ones.";

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
    sentences.push(HELD_TAIL.to_owned());
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
            output: if spec.uses(Capability::Capture) {
                Output::Pixels
            } else {
                Output::Data
            },
            sensitive_args: spec.sensitive_args,
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

    use browseros_playwright::{Capability, Op, OpSpec};
    use browseros_trace::ErrorOwner;

    use super::{OpMarks, Policing, policing};
    use crate::dispatch::{ErrorCode, Listing, Output};

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
        use Capability::{Capture, HostFiles, MainWorld};
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
        let expected = vec![
            Row {
                op: "locator.setInputFiles",
                declared: vec![HostFiles],
                gated: false,
                policed: vec![],
                placement: Held,
            },
            Row {
                op: "page.setContent",
                declared: vec![MainWorld],
                gated: false,
                policed: vec![],
                placement: Held,
            },
            Row {
                op: "page.evaluate",
                declared: vec![MainWorld],
                gated: true,
                policed: vec![],
                placement: Held,
            },
            Row {
                op: "locator.evaluate",
                declared: vec![MainWorld],
                gated: true,
                policed: vec![],
                placement: Held,
            },
            Row {
                op: "locator.evaluateAll",
                declared: vec![MainWorld],
                gated: true,
                policed: vec![],
                placement: Held,
            },
            Row {
                op: "page.waitForFunction",
                declared: vec![MainWorld],
                gated: true,
                policed: vec![],
                placement: Held,
            },
            Row {
                op: "page.screenshot",
                declared: vec![Capture],
                gated: true,
                policed: vec![Capture],
                placement: Served,
            },
            Row {
                op: "page.pdf",
                declared: vec![Capture],
                gated: true,
                policed: vec![Capture],
                placement: Served,
            },
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
                    .filter(|&capability| policing(capability) == Policing::Ready)
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
        use Capability::{Capture, HostFiles, MainWorld, Screen};
        let declarations: [&'static [Capability]; 5] = [
            &[Capture],
            &[MainWorld],
            &[HostFiles],
            &[Screen],
            &[Capture, MainWorld],
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
        // The same capture, once the crate guards it, is served: the door polices captures.
        let gated = OpMarks::of(&new_op(&[Capture], true)?, LINE);
        assert_eq!(gated.hold, None);
        assert_eq!(gated.spec.listing, Listing::Grouped(LINE));
        Ok(())
    }

    /// While the door does not police a capability, every op that declares it is held, guarded
    /// by the crate or not, alone or beside a capability the door does police. On this door only
    /// captures are policed.
    #[test]
    fn a_capability_the_door_does_not_police_yet_holds_every_op_declaring_it()
    -> Result<(), Box<dyn Error>> {
        use Capability::{Capture, HostFiles, MainWorld, Screen};
        let unpoliced: [&'static [Capability]; 6] = [
            &[MainWorld],
            &[HostFiles],
            &[Screen],
            &[Capture, MainWorld],
            &[Capture, HostFiles],
            &[Capture, Screen],
        ];
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
        for (capabilities, policed) in [(&[Capture][..], true), (&[], true), (&[], false)] {
            let marks = OpMarks::of(&new_op(capabilities, policed)?, LINE);
            assert_eq!(marks.hold, None, "{capabilities:?}, policed: {policed}");
            assert_eq!(marks.spec.listing, Listing::Grouped(LINE));
        }
        Ok(())
    }

    /// Pixels, which no mask reaches, come from the crate's declaration, so the tab-privacy rule
    /// refuses every capture of a private tab and no list of op names can miss one.
    #[test]
    fn a_capture_op_gives_pixels_and_no_other_op_does() {
        let mut captures = 0;
        for spec in Op::all().map(Op::spec) {
            let expected = if spec.uses(Capability::Capture) {
                captures += 1;
                Output::Pixels
            } else {
                Output::Data
            };
            assert_eq!(
                OpMarks::of(&spec, LINE).spec.output,
                expected,
                "{}",
                spec.name
            );
        }
        assert!(captures > 0, "the crate declares its captures");
    }
}
