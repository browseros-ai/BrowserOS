//! What a built-in declares (its spec) and what it does (its run step).

use std::future::Future;

use browseros_trace::Effect;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{LeafError, OpCtx, PageTarget};

/// A built-in SDK method: a private type in `builtins/`, registered once at startup and run
/// only by the dispatcher.
///
/// `call` takes an `OpCtx<Self>`, which only the dispatcher can mint, so one method's body
/// cannot run another's: `Title.call(op, ..)` with an `&OpCtx<Close>` is a type error. Another
/// action is reached through `OpCtx::invoke`, as a child op with its own record, rules and
/// delivery (STYLE.md P2). The door decodes the input before any rule runs, so `call` never
/// sees malformed input (P9).
pub(crate) trait Method: Send + Sync + Sized + 'static {
    /// The decoded input. Every input type says `#[serde(deny_unknown_fields)]`.
    type Input: DeserializeOwned + Send + 'static;
    /// The output. The door encodes it to JSON and masks it before the script sees it.
    type Output: Serialize + Send + 'static;

    /// The method's facts, read once at registration.
    fn spec(&self) -> MethodSpec;

    /// The page this call acts on, read from the decoded input; `None` for a method with no
    /// page. The door resolves it to a live tab before the rules run, the rules and the trace
    /// key by that tab, and `OpCtx::page` hands it to `call`. There is no default: a method
    /// that forgot its page would act on a tab the takeover lock never saw.
    fn target(input: &Self::Input) -> Option<&PageTarget>;

    /// The run step: what the action does. The door races it with the run's stop, so a method
    /// is cut short without watching for cancellation itself.
    fn call(
        &self,
        op: &OpCtx<Self>,
        input: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, LeafError>> + Send;
}

/// What the door reads about a method before running it. A plain data bag, built with
/// `MethodSpec { listing: .., ..MethodSpec::new(name, effect) }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MethodSpec {
    /// The name a script calls, e.g. `page.goto`.
    pub(crate) name: &'static str,
    /// `Write` is what can be sent, refused by a takeover lock, and counted in the run's
    /// effects.
    pub(crate) effect: Effect,
    /// Who may call it.
    pub(crate) visibility: Visibility,
    /// How the `run` tool's description shows it.
    pub(crate) listing: Listing,
    /// A normal operation, or a background pull that the run's end cuts quietly.
    pub(crate) lifetime: Lifetime,
    /// Input fields (JSON pointers) the trace always stores as `[redacted]`.
    pub(crate) secret_args: &'static [&'static str],
    /// Input fields redacted when the target turned out to be a sensitive field.
    pub(crate) sensitive_args: &'static [&'static str],
}

impl MethodSpec {
    /// A script-visible, unlisted, ordinary operation with no redacted fields.
    #[must_use]
    pub(crate) const fn new(name: &'static str, effect: Effect) -> Self {
        Self {
            name,
            effect,
            visibility: Visibility::Script,
            listing: Listing::Hidden,
            lifetime: Lifetime::Operation,
            secret_args: &[],
            sensitive_args: &[],
        }
    }
}

/// Who may call a method. A script naming an `Internal` method gets `method_unavailable`,
/// exactly as for a name that does not exist, so the list of names cannot be probed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Visibility {
    /// Scripts and Neo's own Rust.
    Script,
    /// Neo's own Rust only, through `OpCtx::invoke` or a Neo-origin run.
    Internal,
}

/// How the `run` description shows a method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Listing {
    /// Its own line, e.g. `neo.solveCaptcha(page) → { verdict, … }`.
    Own(&'static str),
    /// One shared line per text, e.g. the Playwright API (about 120 methods) as one line.
    Grouped(&'static str),
    /// Not described, still callable (`neo.solveCaptcha` with no provider key).
    Hidden,
}

/// Whether a method is a normal operation or a background pull.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lifetime {
    /// Listed in the run's "still running when the script returned" line.
    Operation,
    /// A long poll (sdk-playwright's event pump) that the run's end cuts without a line.
    Background,
}
