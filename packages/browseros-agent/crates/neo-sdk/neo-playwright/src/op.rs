//! The one list of operations.
//!
//! Each family keeps its ops in its own subsystem's file (`action/action_op.rs`, ...), so work
//! on one family never edits a shared list. [`Op::all`] chains the families; claw-sdk's
//! registry and the prelude's generated `ops.ts` are both derived from it, and
//! `tests/wire_generated.rs` fails when the TypeScript copy is stale. Adding an op is one
//! variant and one spec row in its family file; in a family on route rows (`crate::wire`), also
//! its route row and handler, whose types [`Op::wire`] describes.

use crate::action::ActionOp;
use crate::events::EventOp;
use crate::expect::ExpectOp;
use crate::input::InputOp;
use crate::locator_handler::LocatorHandlerOp;
use crate::op_spec::OpSpec;
use crate::page::{ContextOp, PageOp};
use crate::read::ReadOp;
use crate::snapshot::SnapshotOp;
use crate::wire::OpWire;

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
    LocatorHandler(LocatorHandlerOp),
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
        let handlers = LocatorHandlerOp::ALL
            .iter()
            .copied()
            .map(OpKind::LocatorHandler);
        actions
            .chain(reads)
            .chain(snapshots)
            .chain(expects)
            .chain(inputs)
            .chain(pages)
            .chain(contexts)
            .chain(events)
            .chain(handlers)
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
            OpKind::LocatorHandler(op) => op.spec(),
        }
    }

    /// The op's wire: its input and result as TypeScript, its script option keys and its place
    /// in Playwright's protocol, described from its family's route row and handler types.
    /// `None` while its family still decodes it by hand (no route rows yet, or the op's
    /// transitional `untyped` row); the prelude then declares the op's shapes in `wire.ts`.
    #[must_use]
    pub fn wire(self) -> Option<OpWire> {
        match self.0 {
            OpKind::Snapshot(op) => op.wire(),
            OpKind::Action(op) => op.wire(),
            OpKind::Expect(op) => op.wire(),
            OpKind::Page(op) => op.wire(),
            OpKind::Context(op) => op.wire(),
            OpKind::Input(op) => op.wire(),
            OpKind::Read(op) => op.wire(),
            OpKind::LocatorHandler(op) => op.wire(),
            OpKind::Event(_) => None,
        }
    }

    /// The op's family as a file name (`action`, `locator-handler`): the generated prelude file
    /// its types and upstream checks land in, one per family, so changes to two families never
    /// edit one generated file.
    #[must_use]
    pub fn family(self) -> &'static str {
        match self.0 {
            OpKind::Action(_) => "action",
            OpKind::Read(_) => "read",
            OpKind::Snapshot(_) => "snapshot",
            OpKind::Expect(_) => "expect",
            OpKind::Input(_) => "input",
            OpKind::Page(_) => "page",
            OpKind::Context(_) => "context",
            OpKind::Event(_) => "event",
            OpKind::LocatorHandler(_) => "locator-handler",
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
    }

    /// Every way a described op differs from Playwright's command carries a reason, so the
    /// generated upstream check never hides a deviation nobody explained.
    #[test]
    fn tests_that_every_upstream_deviation_has_a_reason() {
        for op in Op::all() {
            let Some(wire) = op.wire() else { continue };
            let unexplained = wire.upstream.unexplained();
            assert!(
                unexplained.is_empty(),
                "{}: {unexplained:?}",
                op.spec().name
            );
        }
    }

    /// A described op's input and result are typed: TypeScript's `unknown` or `any` would let
    /// the prelude send or read anything, which is what describing the op exists to stop.
    #[test]
    fn tests_that_no_input_or_output_is_unknown() {
        for op in Op::all() {
            let Some(wire) = op.wire() else { continue };
            for (side, reference) in [("input", &wire.input), ("output", &wire.output)] {
                let bare = reference.trim_end_matches(" | null");
                assert!(
                    !matches!(bare, "unknown" | "any" | "JsonValue"),
                    "{}: its {side} prints as `{reference}`",
                    op.spec().name
                );
            }
        }
    }

    /// The generator writes one file per family: each name is a safe file stem, and no two
    /// families share one.
    #[test]
    fn tests_that_every_family_names_its_own_generated_file() {
        let mut kinds: BTreeMap<&str, HashSet<String>> = BTreeMap::new();
        for op in Op::all() {
            let family = op.family();
            assert!(
                !family.is_empty() && family.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{family} is written as generated/{family}.ts"
            );
            let kind = format!("{:?}", op.kind());
            let kind = kind.split_once('(').map_or(kind.as_str(), |(kind, _)| kind);
            kinds.entry(family).or_default().insert(kind.to_owned());
        }
        assert_eq!(kinds.len(), 9, "each OpKind family has its own file");
        assert!(
            kinds.values().all(|kinds| kinds.len() == 1),
            "a file holds one family: {kinds:?}"
        );
    }

    #[test]
    fn tests_that_names_are_playwright_receivers_and_methods() {
        const RECEIVERS: [&str; 12] = [
            "worker",
            "locator",
            "page",
            "keyboard",
            "mouse",
            "context",
            "expect",
            "events",
            "dialog",
            "disposable",
            "bindingCall",
            "webStorage",
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

    /// The audit of record: every op, with exactly what it reaches beyond its page's isolated
    /// world, and whether its gateway polices all of it before its first effect (`gated`). A
    /// host holds and polices ops by these declarations, so a new op without a row, or a
    /// declaration changed without its row, fails here until the audit is redone on purpose.
    /// `page.setContent` is gated: its HTML goes out only through the main-world gateway's
    /// document write (`evaluate_guard/document_write.rs`). `locator.setInputFiles` is
    /// gated: its gateway uploads paths and payloads using Playwright's file handling. `page.addInitScript` and
    /// `context.addInitScript` are gated: their source goes to the page, and a `{ path }` is read
    /// from Neo's disk, only through `page/init_script.rs`, which requires both capabilities;
    /// so are `page.exposeBinding` and `context.exposeBinding`, whose stubs go through it too, and
    /// `bindingCall.resolve`, whose answer goes through the evaluate guard's `resolve_binding`.
    ///
    /// The ops with an empty row send selectors, values or trusted input, or read in the isolated
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
        use Capability::{HostFiles, MainWorld};
        // The explicit rows are the capability contract. Exhaustive comparisons below catch
        // additions and removals without a second, independently maintained operation count.
        const AUDITED: &[(&str, &[Capability], bool)] = &[
            ("locator.click", &[], true),
            ("locator.dblclick", &[], true),
            ("locator.hover", &[], true),
            ("locator.check", &[], true),
            ("locator.uncheck", &[], true),
            ("locator.dragTo", &[], true),
            ("locator.drop", &[HostFiles], true),
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
            ("locator.normalize", &[], true),
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
            ("page.addScriptTag", &[MainWorld, HostFiles], true),
            ("page.addStyleTag", &[MainWorld, HostFiles], true),
            ("page.emulateMedia", &[], true),
            ("page.close", &[], true),
            ("page.runBeforeUnload", &[], true),
            ("page.opener", &[], true),
            ("page.frames", &[], true),
            ("worker.evaluate", &[MainWorld], true),
            ("page.evaluate", &[MainWorld], true),
            ("locator.evaluate", &[MainWorld], true),
            ("locator.evaluateAll", &[MainWorld], true),
            ("page.waitForFunction", &[MainWorld], true),
            ("locator.waitForFunction", &[MainWorld], true),
            ("page.screenshot", &[], true),
            ("page.saveScreenshot", &[], true),
            ("page.pdf", &[], true),
            ("page.setViewportSize", &[], true),
            ("page.requestGC", &[], true),
            ("page.addInitScript", &[MainWorld, HostFiles], true),
            ("page.exposeBinding", &[MainWorld], true),
            ("bindingCall.resolve", &[MainWorld], true),
            ("webStorage.items", &[], true),
            ("webStorage.getItem", &[], true),
            ("webStorage.setItem", &[], true),
            ("webStorage.removeItem", &[], true),
            ("webStorage.clear", &[], true),
            ("context.pages", &[], true),
            ("context.newPage", &[], true),
            ("context.addInitScript", &[MainWorld, HostFiles], true),
            ("context.exposeBinding", &[MainWorld], true),
            ("disposable.dispose", &[], true),
            ("events.listen", &[], true),
            ("events.next", &[], true),
            ("events.wait", &[], true),
            ("events.interceptFileChooser", &[], true),
            ("dialog.accept", &[], true),
            ("dialog.dismiss", &[], true),
            ("download.path", &[], true),
            ("download.saveAs", &[], true),
            ("page.addLocatorHandler", &[], true),
            ("page.removeLocatorHandler", &[], true),
            ("page.locatorHandlerTriggered", &[], true),
            ("page.resolveLocatorHandler", &[], true),
        ];
        let mut audited: BTreeMap<_, _> = AUDITED
            .iter()
            .map(|&(name, capabilities, gated)| (name, (capabilities, gated)))
            .collect();
        assert_eq!(
            audited.len(),
            AUDITED.len(),
            "each operation has one unambiguous audit row"
        );
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

    /// Only prelude-owned listener setup and pulls are exempt from unawaited-op notices.
    #[test]
    fn tests_that_only_prelude_owned_event_work_runs_in_the_background() {
        for op in Op::all() {
            let spec = op.spec();
            let expected = if matches!(
                spec.name,
                "events.listen" | "events.next" | "page.locatorHandlerTriggered"
            ) {
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
                // The screenshot client's filesystem step has no browser target.
                ("page.saveScreenshot", _) => OpTarget::None,
                ("expect.locator" | "page.addLocatorHandler", _) | (_, Some("locator")) => {
                    OpTarget::Field("target")
                }
                // They name a registration, not a page, and must answer after the handler
                // closed its page.
                (
                    "page.removeLocatorHandler"
                    | "page.locatorHandlerTriggered"
                    | "page.resolveLocatorHandler",
                    _,
                ) => OpTarget::None,
                (
                    _,
                    Some(
                        "page" | "keyboard" | "mouse" | "dialog" | "expect" | "bindingCall"
                        | "worker" | "webStorage",
                    ),
                ) => OpTarget::Field("page"),
                _ => OpTarget::None,
            };
            assert_eq!(spec.target, expected, "{} target field", spec.name);
        }
    }
}
