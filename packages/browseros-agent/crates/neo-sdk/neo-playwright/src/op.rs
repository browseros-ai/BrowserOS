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
    use std::collections::{BTreeMap, HashSet};

    use neo_trace::Effect;

    use super::Op;
    use crate::action::ActionOp;
    use crate::expect::ExpectOp;
    use crate::input::InputOp;
    use crate::op_spec::{Capability, OpLifetime, OpTarget};
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
        assert_eq!(names.len(), 76, "the v1 op list has 76 ops");
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

    /// The audit of record: every op, with exactly what it reaches beyond its page's sealed
    /// world, and whether its gateway polices all of it before its first effect (`gated`). A
    /// host holds and polices ops by these declarations, so a new op without a row, or a
    /// declaration changed without its row, fails here until the audit is redone on purpose.
    /// `page.setContent` is gated: its HTML goes out only through the main-world gateway's
    /// document write (`evaluate_guard/document_write.rs`). `locator.setInputFiles` is
    /// gated: its gateway refuses Neo's own secret files and any path into `/dev` or `/proc`
    /// before it sends a path (`input_files/secrets.rs`).
    ///
    /// The ops with an empty row send selectors, values or trusted input, or read in the sealed
    /// world. `page.goto` delegates URLs to Chromium, including `javascript:` scripts in the
    /// current document; it has no separate script gateway. Aliases share their op's row:
    /// `locator.screenshot` and `elementHandle.screenshot` send `page.screenshot`; `$eval`,
    /// `frame.$eval` and `elementHandle.evaluate` send `locator.evaluate`; `$$eval` sends
    /// `locator.evaluateAll`; the `frame` forms send the page ops.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one explicit audited capability row per operation keeps additions reviewable"
    )]
    fn tests_that_every_op_declares_exactly_its_audited_capabilities() {
        use Capability::{Capture, HostFiles, MainWorld};
        const AUDITED: [(&str, &[Capability], bool); 76] = [
            ("locator.click", &[], true),
            ("locator.dblclick", &[], true),
            ("locator.hover", &[], true),
            ("locator.check", &[], true),
            ("locator.uncheck", &[], true),
            ("locator.dragTo", &[], true),
            ("locator.fill", &[], true),
            ("locator.press", &[], true),
            ("locator.pressSequentially", &[], true),
            ("locator.selectOption", &[], true),
            ("locator.selectText", &[], true),
            ("locator.focus", &[], true),
            ("locator.blur", &[], true),
            ("locator.setInputFiles", &[HostFiles], true),
            ("locator.dispatchEvent", &[], true),
            ("locator.scrollIntoViewIfNeeded", &[], true),
            ("locator.textContent", &[], true),
            ("locator.innerText", &[], true),
            ("locator.innerHTML", &[], true),
            ("locator.getAttribute", &[], true),
            ("locator.inputValue", &[], true),
            ("locator.isEnabled", &[], true),
            ("locator.isDisabled", &[], true),
            ("locator.isEditable", &[], true),
            ("locator.isChecked", &[], true),
            ("locator.isVisible", &[], true),
            ("locator.isHidden", &[], true),
            ("locator.count", &[], true),
            ("locator.allTextContents", &[], true),
            ("locator.allInnerTexts", &[], true),
            ("locator.boundingBox", &[], true),
            ("locator.waitFor", &[], true),
            ("locator.elementHandle", &[], true),
            ("locator.elementHandles", &[], true),
            ("page.ariaSnapshot", &[], true),
            ("locator.ariaSnapshot", &[], true),
            ("expect.locator", &[], true),
            ("expect.page", &[], true),
            ("keyboard.down", &[], true),
            ("keyboard.up", &[], true),
            ("keyboard.press", &[], true),
            ("keyboard.type", &[], true),
            ("keyboard.insertText", &[], true),
            ("mouse.move", &[], true),
            ("mouse.down", &[], true),
            ("mouse.up", &[], true),
            ("mouse.click", &[], true),
            ("mouse.wheel", &[], true),
            ("page.goto", &[], true),
            ("page.reload", &[], true),
            ("page.goBack", &[], true),
            ("page.goForward", &[], true),
            ("page.waitForNavigation", &[], true),
            ("page.waitForURL", &[], true),
            ("page.waitForLoadState", &[], true),
            ("page.title", &[], true),
            ("page.content", &[], true),
            ("page.setContent", &[MainWorld], true),
            ("page.close", &[], true),
            ("page.opener", &[], true),
            ("page.frames", &[], true),
            ("page.evaluate", &[MainWorld], true),
            ("locator.evaluate", &[MainWorld], true),
            ("locator.evaluateAll", &[MainWorld], true),
            ("page.waitForFunction", &[MainWorld], true),
            ("page.screenshot", &[Capture], true),
            ("page.pdf", &[Capture], true),
            ("context.pages", &[], true),
            ("context.newPage", &[], true),
            ("events.listen", &[], true),
            ("events.next", &[], true),
            ("events.wait", &[], true),
            ("dialog.accept", &[], true),
            ("dialog.dismiss", &[], true),
            ("download.path", &[], true),
            ("download.saveAs", &[], true),
        ];
        let mut audited: BTreeMap<&str, (&[Capability], bool)> = AUDITED
            .into_iter()
            .map(|(name, capabilities, gated)| (name, (capabilities, gated)))
            .collect();
        let mut differ = Vec::new();
        for spec in Op::all().map(Op::spec) {
            let row = audited.remove(spec.name);
            let declared = (spec.capabilities, spec.gated());
            if row != Some(declared) {
                differ.push((spec.name, declared, row));
            }
        }
        assert!(
            differ.is_empty(),
            "each op declares exactly its audited row, (capabilities, gated): \
             (op, declared, audited): {differ:?}"
        );
        assert!(
            audited.is_empty(),
            "an audited op that is no longer registered must leave the audit: {:?}",
            audited.keys().collect::<Vec<_>>()
        );
    }

    /// A read never marks itself sent, so its failure tells the host nothing changed. A
    /// main-world script can change the page, so an op that declares `MainWorld` is a write.
    #[test]
    fn tests_that_every_main_world_op_is_a_write() {
        for spec in Op::all()
            .map(Op::spec)
            .filter(|spec| spec.uses(Capability::MainWorld))
        {
            assert_eq!(
                spec.effect,
                Effect::Write,
                "{}: its script can change the page",
                spec.name
            );
        }
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
