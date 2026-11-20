//! `neo.find`, `neo.findAll`, `neo.check` and `neo.choose`: Jev's decisions for a blind script,
//! over neo-jev; and `neo.stop`, the script's own way out, listed beside them.
//!
//! ```text
//!  script ─▶ neo_prelude.js: page.ariaSnapshot() ─▶ invoke('neo.find', { url, title, snapshot,
//!            ask, role, picked }) ─▶ door ─▶ Find::call ─▶ neo_jev::Jev (HTTP) ─▶ verdict JSON
//!            ◀─ re-look while not_found, else stop(), page.getByRef(pick.ref) ◀─┘
//! ```
//!
//! These methods never touch a page: the page arrives as the snapshot TEXT the prelude took, and a
//! verdict goes back as a plain value (contract decisions 1–3). So they have no target, never mark
//! themselves sent, and the takeover lock is the prelude's snapshot op's business, not theirs.
//! Every verdict is a value, a miss included; only a transport fault is an error. The prelude
//! turns a verdict it cannot act on into a stop, a branded refusal the door turns down at
//! admission, so `neo.stop` is registered only to be listed. The key stays in neo-jev's client:
//! no input, output, note or error here carries it.

use std::future;
use std::sync::Arc;

use neo_jev::{
    CheckVerdict, ChooseVerdict, EarlierPick, FindAllVerdict, FindOpts, FindVerdict, Jev, JevFault,
    PageText,
};
use neo_trace::Effect;
use serde::Deserialize;
use serde_json::Value;

use crate::dispatch::{
    ErrorCode, LeafError, Listing, Method, MethodSpec, OpCtx, OperationError, PageTarget,
    RegistryBuilder, RegistryError,
};

/// The methods' own lines in the `run` description, shown only when this Neo has a Jev key. They
/// say when a helper stops the script, and the `run` description's `AHEAD` paragraph
/// (`mcp/run_description.rs`) says what a stop hands back, so a model that reads only that tool
/// writes ahead and carries on from a stop by hand.
const FIND_LISTED: &str = "neo.find(page, ask, { role?, timeout? = 5000, optional? }) → Locator: \
    the one element `ask` describes in plain words (\"the product search box\"), on the page as it \
    is now, a tab the page just opened included. Looks again until `timeout` while it is not \
    there. Stops the script when nothing fits or two fit; `optional: true` returns null instead.";
const FIND_ALL_LISTED: &str = "neo.findAll(page, ask, { role? }) → Locator[]: every element `ask` \
    describes, in page order; count and index in code (`rows[8]`), never in the ask. Stops the \
    script when it cannot tell for some element.";
const CHECK_LISTED: &str = "neo.check(page, claim, { timeout? = 0 }) → boolean: whether the page \
    shows `claim` is true; with `timeout` it looks again until it is, so use it in place of a \
    sleep. Stops the script when it cannot tell.";
const CHOOSE_LISTED: &str = "neo.choose(page, question, options) → one of `options`, exactly as \
    given: the option the page answers `question` with; add 'other' yourself. Stops the script \
    when no option clearly leads.";
const STOP_LISTED: &str = "neo.stop(page, why): ends the script here with the same hand-back, for \
    when your own check says the page is not what you expected.";

/// Why every call fails without a key: the deployment's to fix, not the script's.
const NO_KEY: &str = "this Neo has no Jev key, so neo.find, neo.findAll, neo.check and \
    neo.choose are off; the deployment must set NEO_JEV_KEY_FILE";

/// Registers the four verbs over the process's one Jev client, and `neo.stop`: listed with a key,
/// registered but unlisted without one, so a script that calls a verb anyway learns why it cannot
/// work.
pub(crate) fn jev_verbs(
    builder: &mut RegistryBuilder,
    jev: Option<Jev>,
) -> Result<(), RegistryError> {
    let jev = jev.map(Arc::new);
    let stop = Stop {
        listing: listing(STOP_LISTED, jev.as_ref()),
    };
    builder.define(Find { jev: jev.clone() })?;
    builder.define(FindAll { jev: jev.clone() })?;
    builder.define(Check { jev: jev.clone() })?;
    builder.define(Choose { jev })?;
    builder.define(stop)
}

/// A helper's own line with a key; hidden without one, since the description must not offer what
/// cannot work.
fn listing(line: &'static str, jev: Option<&Arc<Jev>>) -> Listing {
    match jev {
        Some(_) => Listing::Own(line),
        None => Listing::Hidden,
    }
}

/// The spec every helper shares: a read with no page, listed only when Jev is on.
fn spec(name: &'static str, listing: Listing) -> MethodSpec {
    MethodSpec {
        listing,
        ..MethodSpec::new(name, Effect::Read)
    }
}

/// The client, or the deployment's refusal.
fn client(jev: Option<&Arc<Jev>>) -> Result<&Jev, LeafError> {
    jev.map(AsRef::as_ref).ok_or_else(|| {
        LeafError::definite(OperationError::host(ErrorCode::JEV_UNAVAILABLE, NO_KEY))
    })
}

/// Awaits one Jev call, raced with the run's stop (STYLE.md P7). Not `OpCtx::race`: Jev is not
/// the browser, so a browser reconnect during the call does not make its answer ambiguous.
async fn decide<T>(
    op: &OpCtx<impl Method>,
    work: impl Future<Output = Result<T, JevFault>>,
) -> Result<T, LeafError> {
    let answer = tokio::select! {
        biased;
        () = op.cancel_token().cancelled() => return Err(op.stopped()),
        answer = work => answer,
    };
    answer.map_err(|error| fault(&error))
}

/// A transport fault as the script's error. Nothing was sent to the page, so it is definite.
/// Jev down or refusing the key is the deployment's; a page too big for Jev is the script's step to
/// do by hand (`role` narrows the candidates, not the page text Jev reads).
fn fault(fault: &JevFault) -> LeafError {
    let error = match fault {
        JevFault::Down(_) | JevFault::Refused { .. } => {
            OperationError::host(ErrorCode::JEV_DOWN, fault)
        }
        JevFault::TooBig { .. } => OperationError::script(ErrorCode::JEV_TOO_BIG, fault),
    };
    LeafError::definite(error)
}

/// The page as the prelude saw it: one snapshot, plus where it was taken.
fn page_text(url: String, title: String, snapshot: String) -> PageText {
    PageText {
        url,
        title,
        snapshot,
    }
}

/// What reaches `neo.stop`'s call: only a script that invokes the method raw, past the prelude.
const RAW_STOP: &str = "neo.stop is the prelude's: call `neo.stop(page, why)`, which ends the script \
    with a stop and sends nothing";

/// `neo.stop`: registered so the description lists it beside the helpers whose stop it shares.
/// The prelude's `neo.stop(page, why)` never reaches `call`: it sends a refusal brand, which the
/// door turns down at admission under any name (`dispatch/refusal_brand.rs`).
struct Stop {
    /// Own line with a key, hidden without one, like the four verbs.
    listing: Listing,
}

impl Method for Stop {
    /// Whatever a raw call sends; it is never read.
    type Input = Value;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        spec("neo.stop", self.listing)
    }

    fn target(_input: &Value) -> Option<&PageTarget> {
        None
    }

    fn call(
        &self,
        _op: &OpCtx<Self>,
        _input: Value,
    ) -> impl Future<Output = Result<Value, LeafError>> + Send {
        future::ready(Err(LeafError::definite(OperationError::script(
            ErrorCode::INVALID_INPUT,
            RAW_STOP,
        ))))
    }
}

/// `neo.find`'s and `neo.findAll`'s input.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FindInput {
    url: String,
    title: String,
    snapshot: String,
    ask: String,
    #[serde(default)]
    role: Option<String>,
    /// This run's earlier picks, which the prelude keeps: Jev flags a ref picked again under a
    /// different ask (`reused`), the confident-wrong pick the form test found.
    #[serde(default)]
    picked: Vec<EarlierPick>,
}

impl FindInput {
    fn into_parts(self) -> (PageText, String, FindOpts) {
        let Self {
            url,
            title,
            snapshot,
            ask,
            role,
            picked,
        } = self;
        (
            page_text(url, title, snapshot),
            ask,
            FindOpts { role, picked },
        )
    }
}

/// `neo.find`: one element, or a verdict saying why not.
struct Find {
    /// `None` without a key.
    jev: Option<Arc<Jev>>,
}

impl Method for Find {
    type Input = FindInput;
    type Output = FindVerdict;

    fn spec(&self) -> MethodSpec {
        spec("neo.find", listing(FIND_LISTED, self.jev.as_ref()))
    }

    fn target(_input: &FindInput) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, op: &OpCtx<Self>, input: FindInput) -> Result<FindVerdict, LeafError> {
        let jev = client(self.jev.as_ref())?;
        let (page, ask, opts) = input.into_parts();
        decide(op, jev.find(&page, &ask, &opts)).await
    }
}

/// `neo.findAll`: every matching element, in page order.
struct FindAll {
    /// `None` without a key.
    jev: Option<Arc<Jev>>,
}

impl Method for FindAll {
    type Input = FindInput;
    type Output = FindAllVerdict;

    fn spec(&self) -> MethodSpec {
        spec("neo.findAll", listing(FIND_ALL_LISTED, self.jev.as_ref()))
    }

    fn target(_input: &FindInput) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, op: &OpCtx<Self>, input: FindInput) -> Result<FindAllVerdict, LeafError> {
        let jev = client(self.jev.as_ref())?;
        let (page, ask, opts) = input.into_parts();
        decide(op, jev.find_all(&page, &ask, &opts)).await
    }
}

/// `neo.check`'s input.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckInput {
    url: String,
    title: String,
    snapshot: String,
    claim: String,
}

/// `neo.check`: is the claim true of the page.
struct Check {
    /// `None` without a key.
    jev: Option<Arc<Jev>>,
}

impl Method for Check {
    type Input = CheckInput;
    type Output = CheckVerdict;

    fn spec(&self) -> MethodSpec {
        spec("neo.check", listing(CHECK_LISTED, self.jev.as_ref()))
    }

    fn target(_input: &CheckInput) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, op: &OpCtx<Self>, input: CheckInput) -> Result<CheckVerdict, LeafError> {
        let jev = client(self.jev.as_ref())?;
        let CheckInput {
            url,
            title,
            snapshot,
            claim,
        } = input;
        decide(op, jev.check(&page_text(url, title, snapshot), &claim)).await
    }
}

/// `neo.choose`'s input.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChooseInput {
    url: String,
    title: String,
    snapshot: String,
    question: String,
    options: Vec<String>,
}

/// `neo.choose`: which of the script's own strings the page answers with.
struct Choose {
    /// `None` without a key.
    jev: Option<Arc<Jev>>,
}

impl Method for Choose {
    type Input = ChooseInput;
    type Output = ChooseVerdict;

    fn spec(&self) -> MethodSpec {
        spec("neo.choose", listing(CHOOSE_LISTED, self.jev.as_ref()))
    }

    fn target(_input: &ChooseInput) -> Option<&PageTarget> {
        None
    }

    async fn call(&self, op: &OpCtx<Self>, input: ChooseInput) -> Result<ChooseVerdict, LeafError> {
        let jev = client(self.jev.as_ref())?;
        let ChooseInput {
            url,
            title,
            snapshot,
            question,
            options,
        } = input;
        if options.is_empty() {
            return Err(LeafError::definite(OperationError::script(
                ErrorCode::INVALID_INPUT,
                "neo.choose needs at least one option",
            )));
        }
        decide(
            op,
            jev.choose(&page_text(url, title, snapshot), &question, &options),
        )
        .await
    }
}
