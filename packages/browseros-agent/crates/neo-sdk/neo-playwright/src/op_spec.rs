//! What the host needs to register one op: plain data, read once at startup.

use neo_trace::Effect;

/// One op's registration metadata. claw-sdk copies it field by field into its `MethodSpec`;
/// the prelude's `ops.generated.ts` carries the same name, effect, target and lifetime. A host
/// holds and polices ops by their [`OpSpec::capabilities`], which the prelude has no use for.
#[derive(Clone, Copy, Debug)]
pub struct OpSpec {
    /// The wire name the prelude invokes, such as `locator.click`. Unique across all ops.
    /// Playwright aliases (`page.click`, `frame.click`, `elementHandle.click`) share one op and
    /// tell it their own name in the input's `call.api`, which error messages start with.
    pub name: &'static str,
    /// One line for traces and the summarized listing in the `run` tool's description.
    pub doc: &'static str,
    /// `Read` ops never mark themselves sent, so a failure is always `not_sent` (P6).
    pub effect: Effect,
    /// Which input field holds the page handle, for the dispatcher's tab lock and page facts.
    pub target: OpTarget,
    /// `Background` only for the prelude's event pump.
    pub lifetime: OpLifetime,
    /// What the op reaches beyond its page's sealed world. Empty for an op that only reads and
    /// acts in the sealed world or sends trusted input, which is most of them. A unit test pins
    /// every op's list to an audit: changing one changes what a host holds or polices.
    pub capabilities: &'static [Capability],
    /// Every capability the op declares is policed at its gateway before the op's first
    /// effect. Set only by the crate's `policed` builder, once that gateway is built and
    /// reviewed. Hosts read [`OpSpec::gated`], which also covers an op that declares nothing.
    pub policed: bool,
}

/// What an op reaches beyond its page's sealed world, where the crate's own reads and actions
/// run and field values are masked at the source. These are the facts a host holds and polices
/// ops by, instead of lists of op names, so an op added later that reaches one is held or
/// policed from its first build rather than served unpoliced until someone lists it.
///
/// Each capability has one gateway in the crate, which first refuses, as `neo / internal` with
/// nothing sent, an op whose spec does not declare the capability (`OpScope::require`). An op
/// routed to a gateway without its declaration therefore fails on first use instead of being
/// served unpoliced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Runs agent-supplied script, or HTML whose scripts run, in a page's main world. There the
    /// script reads the page's own state and every frame of its site, and no seal masks a
    /// field. Gateways: the evaluate guard's entries, `EvaluateGuard::open` for a script and
    /// `DocumentWrite::open` for `page.setContent`, whose HTML the sealed world writes and whose
    /// scripts the main world runs; `page/init_script.rs` for the init scripts and binding stubs
    /// every new document runs; and the guard's `resolve_binding` for a binding call's answer.
    MainWorld,
    /// Returns the page's pixels, which no seal masks: `page.screenshot`, with its element
    /// forms, and `page.pdf`. A host refuses such an op on a private tab, and withholds pixels
    /// taken while the tab went private. Gateway: `Playwright::capture_allowed`, which each
    /// capture runs right before it reads pixels.
    Capture,
    /// Changes which tab Neo's one screen shows, a screen the owner's takeovers and the control
    /// displays share. No op declares it yet, and so it has no gateway.
    Screen,
    /// Hands files from Neo's disk to a page whose script can then read them. Relative paths
    /// resolve against Neo's working directory, following Playwright's path handling.
    /// Gateways: `input_files::set_input_files`, for every form of the op, payloads included,
    /// and `page/init_script.rs`, which reads an init script's `{ path }`.
    HostFiles,
}

/// Where the page an op acts on sits in its input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpTarget {
    /// The op acts on no page (`context.pages`, the event pump).
    None,
    /// This top-level input field holds a branded handle: `"page"` holds the page brand
    /// `{"$neo":"page","tab":1234}`, and `"target"` holds a locator or element handle
    /// whose `page` field is that brand. The host resolves and checks that page, and hands the
    /// crate its resolved brand there.
    Field(&'static str),
}

/// How long an op may run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpLifetime {
    /// Ends before the run does, like every Playwright call.
    Operation,
    /// The event pump: a long poll that the host cancels quietly when the run ends.
    Background,
}

/// The page field of ops that act on a page handle.
pub(crate) const PAGE: OpTarget = OpTarget::Field("page");
/// The target field of ops that act on a locator or an element handle.
pub(crate) const TARGET: OpTarget = OpTarget::Field("target");

impl OpSpec {
    /// Whether the op declares `capability`.
    #[must_use]
    pub fn uses(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Whether every capability the op declares is policed by its gateway before the op's first
    /// effect. A host may serve a `MainWorld` or `HostFiles` op only when this is true, and holds
    /// it otherwise. An op that declares nothing is gated trivially. It fails closed: a new op
    /// that declares a capability is ungated until its gateway is reviewed and its spec is marked
    /// `policed`, and a unit test pins every op's answer to the audit.
    #[must_use]
    pub fn gated(&self) -> bool {
        self.capabilities.is_empty() || self.policed
    }

    /// An op that only reads.
    pub(crate) const fn read(name: &'static str, target: OpTarget, doc: &'static str) -> Self {
        Self {
            name,
            doc,
            effect: Effect::Read,
            target,
            lifetime: OpLifetime::Operation,
            capabilities: &[],
            policed: false,
        }
    }

    /// An op that may change the page or the host's files (download/screenshot saves).
    pub(crate) const fn write(name: &'static str, target: OpTarget, doc: &'static str) -> Self {
        Self {
            effect: Effect::Write,
            ..Self::read(name, target, doc)
        }
    }

    /// The same op, as the background event pump.
    pub(crate) const fn background(self) -> Self {
        Self {
            lifetime: OpLifetime::Background,
            ..self
        }
    }

    /// The same op, declaring what it reaches beyond its page's sealed world.
    pub(crate) const fn using(self, capabilities: &'static [Capability]) -> Self {
        Self {
            capabilities,
            ..self
        }
    }

    /// The same op, with every capability it declares policed at its gateway before its first
    /// effect ([`OpSpec::gated`]). Said only once that gateway is built and reviewed.
    pub(crate) const fn policed(self) -> Self {
        Self {
            policed: true,
            ..self
        }
    }
}
