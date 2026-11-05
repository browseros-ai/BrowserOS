//! The one list of operations.
//!
//! Each family keeps its ops in its own subsystem's file (`action/action_op.rs`, ...), so work
//! on one family never edits a shared list. [`Op::all`] chains the families; claw-sdk's
//! registry and the prelude's `ops.generated.ts` are both derived from it, and
//! `tests/ops_generated.rs` fails when the TypeScript copy is stale. Adding an op is one
//! variant and one spec row in its family file.

use crate::action::ActionOp;
use crate::events::EventOp;
use crate::expect::ExpectOp;
use crate::input::InputOp;
use crate::op_spec::OpSpec;
use crate::page::{ContextOp, PageOp};
use crate::read::ReadOp;
use crate::snapshot::SnapshotOp;

/// One registered operation. Opaque: the host lists them with [`Op::all`], reads each one's
/// [`OpSpec`] and hands one back to `Playwright::invoke`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Op(OpKind);

/// The families, each owned by one subsystem.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum OpKind {
    Action(ActionOp),
    Read(ReadOp),
    Snapshot(SnapshotOp),
    Expect(ExpectOp),
    Input(InputOp),
    Page(PageOp),
    Context(ContextOp),
    Event(EventOp),
}

impl Op {
    /// Every operation, in a stable order: the families in [`OpKind`]'s order, then each
    /// family's declaration order. claw-sdk registers each one as a `MethodSpec`.
    pub fn all() -> impl Iterator<Item = Self> {
        let actions = ActionOp::ALL.iter().copied().map(OpKind::Action);
        let reads = ReadOp::ALL.iter().copied().map(OpKind::Read);
        let snapshots = SnapshotOp::ALL.iter().copied().map(OpKind::Snapshot);
        let expects = ExpectOp::ALL.iter().copied().map(OpKind::Expect);
        let inputs = InputOp::ALL.iter().copied().map(OpKind::Input);
        let pages = PageOp::ALL.iter().copied().map(OpKind::Page);
        let contexts = ContextOp::ALL.iter().copied().map(OpKind::Context);
        let events = EventOp::ALL.iter().copied().map(OpKind::Event);
        actions
            .chain(reads)
            .chain(snapshots)
            .chain(expects)
            .chain(inputs)
            .chain(pages)
            .chain(contexts)
            .chain(events)
            .map(Op)
    }

    /// The op whose wire name is `name` (`locator.click`), if there is one.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::all().find(|op| op.spec().name == name)
    }

    /// The op's registration metadata.
    #[must_use]
    pub fn spec(self) -> OpSpec {
        match self.0 {
            OpKind::Action(op) => op.spec(),
            OpKind::Read(op) => op.spec(),
            OpKind::Snapshot(op) => op.spec(),
            OpKind::Expect(op) => op.spec(),
            OpKind::Input(op) => op.spec(),
            OpKind::Page(op) => op.spec(),
            OpKind::Context(op) => op.spec(),
            OpKind::Event(op) => op.spec(),
        }
    }

    /// The op's family, for `Playwright::invoke`'s dispatch.
    pub(crate) fn kind(self) -> OpKind {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::error::Error;

    use browseros_trace::{Effect, JsonPointer};

    use super::Op;
    use crate::action::ActionOp;
    use crate::expect::ExpectOp;
    use crate::input::InputOp;
    use crate::op_spec::{OpLifetime, OpTarget};
    use crate::read::ReadOp;
    use crate::snapshot::SnapshotOp;

    #[test]
    fn tests_that_every_op_name_is_unique_and_finds_its_op() {
        let mut names = HashSet::new();
        for op in Op::all() {
            let name = op.spec().name;
            assert!(names.insert(name), "{name} is registered twice");
            assert_eq!(
                Op::from_name(name),
                Some(op),
                "{name} must find the op it names"
            );
        }
        assert_eq!(
            Op::from_name("locator.tap"),
            None,
            "an unsupported method is no op"
        );
        assert_eq!(names.len(), 75, "the v1 op list has 75 ops");
    }

    #[test]
    fn tests_that_names_are_playwright_receivers_and_methods() {
        const RECEIVERS: [&str; 8] = [
            "locator", "page", "keyboard", "mouse", "context", "expect", "events", "dialog",
        ];
        for op in Op::all() {
            let name = op.spec().name;
            let receiver = name.split_once('.').map(|(receiver, _)| receiver);
            let known = receiver.is_some_and(|receiver| RECEIVERS.contains(&receiver))
                || name.starts_with("download.");
            assert!(
                known,
                "{name} must be `<receiver>.<method>` on a known receiver"
            );
        }
    }

    #[test]
    fn tests_that_every_sensitive_argument_is_a_json_pointer() -> Result<(), Box<dyn Error>> {
        for op in Op::all() {
            let spec = op.spec();
            for pointer in spec.sensitive_args {
                JsonPointer::parse(pointer)?;
            }
            assert_eq!(
                spec.redaction().sensitive.len(),
                spec.sensitive_args.len(),
                "{} must redact every argument it declares sensitive",
                spec.name
            );
            assert!(
                spec.redaction().secret.is_empty(),
                "no Playwright argument is always secret"
            );
        }
        Ok(())
    }

    #[test]
    fn tests_that_typed_text_is_redacted_for_a_sensitive_target() -> Result<(), Box<dyn Error>> {
        for (name, pointer) in [
            ("locator.fill", "/value"),
            ("locator.pressSequentially", "/text"),
            ("locator.press", "/key"),
            ("keyboard.type", "/text"),
            ("keyboard.insertText", "/text"),
            ("keyboard.press", "/key"),
            ("keyboard.down", "/key"),
        ] {
            let op = Op::from_name(name).ok_or(name)?;
            assert_eq!(
                op.spec().sensitive_args,
                [pointer],
                "{name} carries typed text"
            );
        }
        Ok(())
    }

    #[test]
    fn tests_that_only_the_event_pump_runs_in_the_background() {
        for op in Op::all() {
            let spec = op.spec();
            let expected = if spec.name == "events.next" {
                OpLifetime::Background
            } else {
                OpLifetime::Operation
            };
            assert_eq!(spec.lifetime, expected, "{} lifetime", spec.name);
        }
    }

    /// A read family can never mark itself sent, so a host may retry any of its ops; input
    /// always may change the page.
    #[test]
    fn tests_that_read_families_only_read_and_input_writes() {
        let reads = ReadOp::ALL.iter().map(|op| op.spec());
        let snapshots = SnapshotOp::ALL.iter().map(|op| op.spec());
        let expects = ExpectOp::ALL.iter().map(|op| op.spec());
        for spec in reads.chain(snapshots).chain(expects) {
            assert_eq!(spec.effect, Effect::Read, "{} only reads", spec.name);
        }
        for spec in InputOp::ALL.iter().map(|op| op.spec()) {
            assert_eq!(spec.effect, Effect::Write, "{} sends input", spec.name);
        }
        for spec in ActionOp::ALL.iter().map(|op| op.spec()) {
            let expected = if spec.name == "locator.scrollIntoViewIfNeeded" {
                Effect::Read
            } else {
                Effect::Write
            };
            assert_eq!(spec.effect, expected, "{} effect", spec.name);
        }
    }

    #[test]
    fn tests_that_each_op_names_the_field_that_holds_its_page() {
        for op in Op::all() {
            let spec = op.spec();
            let receiver = spec.name.split_once('.').map(|(receiver, _)| receiver);
            let expected = match (spec.name, receiver) {
                ("expect.locator", _) | (_, Some("locator")) => OpTarget::Field("target"),
                (_, Some("page" | "keyboard" | "mouse" | "dialog" | "expect")) => {
                    OpTarget::Field("page")
                }
                _ => OpTarget::None,
            };
            assert_eq!(spec.target, expected, "{} target field", spec.name);
        }
    }
}
