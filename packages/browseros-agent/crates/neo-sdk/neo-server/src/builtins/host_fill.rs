//! The host's private typing as two methods only Neo's own Rust reaches: `host.fill` types the
//! host's values into the boxes it named, and `host.fillCheck` looks at the boxes without
//! typing (protected-fill design, sections 4, 5 and 9).
//!
//! ```text
//!  POST /v1/tabs/{tab}/fill ─▶ FillService: a run of Neo's own, its code FILL_CODE
//!    run slot FillValues ◀─ the values (PrivateText): beside the op, never in its input
//!    Dispatcher::submit(host.fill, { page, url, fields: [ref, expect, cut, spaced, characters] })
//!      the door: the takeover lock and the CAPTCHA busy rule refuse it like any write
//!      HostFill::call ─ take the values ─ Playwright::enter_privately(env, request)
//!                     ─ the Entry into run slot FillProgress (`at` and `typed` for the answer)
//!      ◀─ Ok({ outcome, typed }), or the entry's error, definite or ambiguous as it says
//! ```
//!
//! The door records an op's input as its `args` and events carry them, so the values never
//! enter the input: they wait in the run's typed slot, which the run's end empties, and
//! `PrivateText` cannot be serialized, so putting one in the input does not compile. Both
//! methods are `Internal` (a script naming either gets `method_unavailable`, as for a name that
//! does not exist) and recorded as kind `protected`, which the trace never photographs and the
//! registry refuses on any method a script may call.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_core::TabId;
use futures_util::future::BoxFuture;
use neo_playwright::{
    Answer, BoxFacts, BoxLook, BoxSpec, Entry, EntryError, EntryOutcome, EntryRequest, Expect,
    FileAccess, LookField, LookRequest, OpEnv, PrivateField, PrivateText,
};
use neo_sdk_api as api;
use neo_trace::{Effect, ErrorOwner, OperationKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::Instant;

use super::PlaywrightFacade;
use super::playwright_ops::OpCtxEnv;
use crate::browser::PageRef;
use crate::dispatch::{
    ErrorCode, LeafError, Method, MethodSpec, OpCtx, OperationError, PageTarget, RegistryBuilder,
    RegistryError, Visibility,
};
use crate::host::AgentFront;

/// The fill's method name, as its record shows it.
pub(crate) const FILL: &str = "host.fill";
/// The check's method name.
pub(crate) const CHECK: &str = "host.fillCheck";
/// The `code` a fill's run records in place of a script: the trace says what ran and that the
/// values are kept nowhere.
pub(crate) const FILL_CODE: &str = "// host.fill: the host typed private values into the boxes named below; the values are not recorded";
/// The `code` a check's run records.
pub(crate) const CHECK_CODE: &str = "// host.fillCheck: the host looked at the boxes named below";
/// How long the run outlives the entry's own deadline. The entry stops itself at its deadline
/// and answers `run_timeout` with its counts; the run's deadline only cuts an entry that stopped
/// answering, and an entry cut that way has no counts to give.
pub(crate) const ENTRY_BACKSTOP: Duration = Duration::from_secs(2);

/// A fill's values, from the route to its method. Put by the fill service before it submits the
/// op and taken once by the method; whatever is left goes with the run's slots when its record
/// ends, and each `PrivateText` zeroes itself as it drops.
#[derive(Default)]
pub(crate) struct FillValues(Mutex<Option<Vec<PrivateText>>>);

/// What the entry answered, for the route's `at` and `typed`. A failed op's record keeps only
/// its error, so the counts travel here, beside it. Holds no value.
#[derive(Default)]
pub(crate) struct FillProgress(Mutex<Option<Entry>>);

/// One box as the fill's record names it: what the host named and approved, and for a fill how
/// many characters its value has. Never the value. The fill service builds it; the method decodes
/// it back. A plain data bag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedBox {
    /// The box's ref.
    #[serde(rename = "ref")]
    pub(crate) box_ref: String,
    /// The facts the host approved: always for a fill; a check may come before any approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expect: Option<Expect>,
    /// Whether the host accepts a value the box's `maxlength` cuts.
    #[serde(default)]
    pub(crate) cut: bool,
    /// Whether the read-back sets spaces, dashes and slashes aside.
    #[serde(default)]
    pub(crate) spaced: bool,
    /// Whether the value is typed key by key rather than in one insertion.
    #[serde(default)]
    pub(crate) keys: bool,
    /// The value's length in characters; absent for a check, which has no value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) characters: Option<usize>,
}

/// Both methods' input, as the door decodes it from what the fill service built.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BoxesInput {
    /// The tab, as its page brand.
    page: PageTarget,
    /// The address the tab must still be at.
    #[serde(default)]
    url: Option<String>,
    /// The boxes, in typing order.
    fields: Vec<RecordedBox>,
}

/// The same input as the fill service writes it: the page as its brand.
#[derive(Serialize)]
struct BoxesArgs<'a> {
    page: PageRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<&'a str>,
    fields: &'a [RecordedBox],
}

/// The entry the two methods drive: neo-playwright's `Playwright::enter_privately` and
/// `look_boxes` (protected-fill pull requests 1 and 2), behind one seam so the route's tests can
/// script it. The methods own everything around it: the values' hand-over, the deadline, the
/// mapping of its answer onto the door's error and delivery.
pub(crate) trait PrivateEntry: Send + Sync + 'static {
    /// Types every value of `request`; drops the values before it answers.
    fn enter<'a>(&'a self, env: &'a dyn OpEnv, request: EntryRequest) -> BoxFuture<'a, Entry>;

    /// Looks at the boxes of `request`, typing nothing.
    fn look<'a>(
        &'a self,
        env: &'a dyn OpEnv,
        request: &'a LookRequest,
    ) -> BoxFuture<'a, Result<Vec<BoxLook>, EntryError>>;
}

impl PrivateEntry for PlaywrightFacade {
    fn enter<'a>(&'a self, env: &'a dyn OpEnv, request: EntryRequest) -> BoxFuture<'a, Entry> {
        Box::pin(async move { self.get().await.enter_privately(env, request).await })
    }

    fn look<'a>(
        &'a self,
        env: &'a dyn OpEnv,
        request: &'a LookRequest,
    ) -> BoxFuture<'a, Result<Vec<BoxLook>, EntryError>> {
        Box::pin(async move { self.get().await.look_boxes(env, request).await })
    }
}

/// What both methods hold: the entry, and what an `OpEnv` needs from the facade.
#[derive(Clone)]
pub(crate) struct EntryParts {
    /// The entry they drive.
    pub(crate) entry: Arc<dyn PrivateEntry>,
    /// Whether the browser runs in hosted mode (`OpEnv::hosted_mode`).
    pub(crate) hosted: bool,
    /// Brings the tab forward before its input (`OpEnv::bring_forward`).
    pub(crate) front: Arc<AgentFront>,
    /// The process's file policy, which every op's environment carries.
    pub(crate) file_access: Arc<FileAccess>,
}

struct HostFill(EntryParts);

struct HostFillCheck(EntryParts);

/// Registers both methods over the process's one facade. Called once at startup.
pub(crate) fn host_fill(
    builder: &mut RegistryBuilder,
    playwright: &Arc<PlaywrightFacade>,
) -> Result<(), RegistryError> {
    register(
        builder,
        EntryParts {
            entry: Arc::clone(playwright) as Arc<dyn PrivateEntry>,
            hosted: playwright.hosted_mode(),
            front: Arc::clone(playwright.front()),
            file_access: Arc::clone(playwright.file_access()),
        },
    )
}

/// Registers both methods over `parts`: production's facade, or a test's scripted entry.
pub(crate) fn register(
    builder: &mut RegistryBuilder,
    parts: EntryParts,
) -> Result<(), RegistryError> {
    builder.define(HostFill(parts.clone()))?;
    builder.define(HostFillCheck(parts))
}

/// The op's input for one fill or check on `tab`, as the door will record it.
#[must_use]
pub(crate) fn boxes_args(tab: TabId, url: Option<&str>, fields: &[RecordedBox]) -> Value {
    serde_json::to_value(BoxesArgs {
        page: PageRef { tab },
        url,
        fields,
    })
    // A struct of strings, numbers and flags always serializes.
    .unwrap_or(Value::Null)
}

impl RecordedBox {
    /// The box as the entry takes it. A fill's box always names its approval (the parser
    /// requires `expect.origin`); one without is Neo's own mistake, and types nothing.
    fn into_spec(self) -> Result<BoxSpec, LeafError> {
        let expect = self.expect.ok_or_else(|| {
            LeafError::definite(OperationError::neo(
                ErrorCode::INTERNAL_ERROR,
                "a fill's box reached the entry without its approval; nothing was typed",
            ))
        })?;
        Ok(BoxSpec {
            box_ref: self.box_ref,
            expect,
            cut: self.cut,
            spaced: self.spaced,
            keys: self.keys,
        })
    }

    /// The box as the look takes it.
    fn into_look(self) -> LookField {
        LookField {
            box_ref: self.box_ref,
            expect: self.expect,
            keys: self.keys,
        }
    }
}

impl FillValues {
    /// Hands the run its values, before its op is submitted.
    pub(crate) fn put(&self, values: Vec<PrivateText>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(values);
    }

    fn take(&self) -> Option<Vec<PrivateText>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

impl FillProgress {
    fn put(&self, entry: Entry) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(entry);
    }

    /// What the entry answered, once its op ended; `None` when the door refused the op before
    /// the entry ran, or cut it short.
    pub(crate) fn take(&self) -> Option<Entry> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

impl Method for HostFill {
    type Input = BoxesInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            visibility: Visibility::Internal,
            record: OperationKind::Protected,
            ..MethodSpec::new(FILL, Effect::Write)
        }
    }

    fn target(input: &BoxesInput) -> Option<&PageTarget> {
        Some(&input.page)
    }

    async fn call(&self, op: &OpCtx<Self>, input: BoxesInput) -> Result<Value, LeafError> {
        let tab = op.page()?.tab;
        let values = op
            .run()
            .slot::<FillValues>()
            .take()
            .filter(|values| values.len() == input.fields.len())
            .ok_or_else(|| {
                LeafError::definite(OperationError::neo(
                    ErrorCode::INTERNAL_ERROR,
                    "the fill's values did not reach its operation; nothing was typed",
                ))
            })?;
        let fields = input
            .fields
            .into_iter()
            .zip(values)
            .map(|(recorded, value)| {
                Ok(PrivateField {
                    spec: recorded.into_spec()?,
                    value,
                })
            })
            .collect::<Result<Vec<_>, LeafError>>()?;
        let request = EntryRequest {
            tab,
            url: input.url,
            fields,
            deadline: entry_deadline(op.deadline()),
        };
        let env = OpCtxEnv::new(
            op,
            self.0.hosted,
            Arc::clone(&self.0.front),
            Arc::clone(&self.0.file_access),
        );
        let entry = op
            .race_classified(self.0.entry.enter(&env, request))
            .await?;
        let settled = settle(&entry);
        op.run().slot::<FillProgress>().put(entry);
        settled
    }
}

impl Method for HostFillCheck {
    type Input = BoxesInput;
    type Output = Value;

    fn spec(&self) -> MethodSpec {
        MethodSpec {
            visibility: Visibility::Internal,
            record: OperationKind::Protected,
            ..MethodSpec::new(CHECK, Effect::Read)
        }
    }

    fn target(input: &BoxesInput) -> Option<&PageTarget> {
        Some(&input.page)
    }

    async fn call(&self, op: &OpCtx<Self>, input: BoxesInput) -> Result<Value, LeafError> {
        let request = LookRequest {
            tab: op.page()?.tab,
            url: input.url,
            fields: input
                .fields
                .into_iter()
                .map(RecordedBox::into_look)
                .collect(),
            deadline: entry_deadline(op.deadline()),
        };
        let env = OpCtxEnv::new(
            op,
            self.0.hosted,
            Arc::clone(&self.0.front),
            Arc::clone(&self.0.file_access),
        );
        let looks = op
            .race_classified(self.0.entry.look(&env, &request))
            .await?
            .map_err(|error| LeafError::definite(operation_error(&error)))?;
        // The op's recorded result is the route's answer, so the record and the reply agree.
        let fields: Vec<api::FillBoxCheck> = looks.into_iter().map(checked_box).collect();
        Ok(json!({ "fields": fields }))
    }
}

/// One box of the check's answer, in the contract's shape. A look sends nothing, so a refusal is
/// always `retryable` in the SDK's one meaning (ruling R5).
fn checked_box(look: BoxLook) -> api::FillBoxCheck {
    api::FillBoxCheck {
        facts: look.facts.map(|facts| Box::new(wire_facts(facts))),
        refusal: look.refusal.map(|refusal| {
            Box::new(api::OperationError {
                owner: owner_name(refusal.owner),
                code: refusal.code,
                message: refusal.message,
                retryable: true,
                host_code: refusal.host_code,
                operation_id: None,
            })
        }),
    }
}

fn wire_facts(facts: BoxFacts) -> api::BoxFacts {
    api::BoxFacts {
        origin: facts.origin,
        r#type: facts.kind,
        autocomplete: facts.autocomplete,
        max_length: facts
            .max_length
            .map(|length| i32::try_from(length).unwrap_or(i32::MAX)),
        input_mode: facts.input_mode,
        name: facts.name,
        id: facts.id,
        sized: facts.sized,
        empty: facts.empty,
    }
}

fn owner_name(owner: ErrorOwner) -> String {
    owner.as_str().to_owned()
}

/// The entry's own deadline: the run's, less the backstop, so the entry answers with its
/// counts before the door would cut it.
fn entry_deadline(run_deadline: Instant) -> Instant {
    run_deadline
        .checked_sub(ENTRY_BACKSTOP)
        .unwrap_or(run_deadline)
}

/// The op's end from the entry's answer. An entry that typed every value returns its outcome
/// and counts; any other is the op's error, definite or ambiguous as the entry's error says, so
/// the door derives from "sent" and that answer the delivery the entry reported (ruling R5).
fn settle(entry: &Entry) -> Result<Value, LeafError> {
    match entry.outcome {
        EntryOutcome::Entered | EntryOutcome::EnteredPageMoved => Ok(json!({
            "outcome": entry.outcome.as_str(),
            "typed": entry.typed,
        })),
        EntryOutcome::Refused | EntryOutcome::Partial | EntryOutcome::Uncertain => {
            let Some(error) = entry.error.as_ref() else {
                // Unexplained, so nothing about the page is certain.
                return Err(LeafError::ambiguous(OperationError::neo(
                    ErrorCode::INTERNAL_ERROR,
                    "the entry stopped without saying why",
                )));
            };
            Err(match error.answer() {
                Answer::Ambiguous => LeafError::ambiguous(operation_error(error)),
                Answer::Definite => LeafError::definite(operation_error(error)),
            })
        }
    }
}

/// The entry's error as the door's. Its codes are on the door's one list; one that is not (the
/// entry's `internal`) is Neo's `internal_error`. The message is the entry's fixed sentence, or
/// a rule's own refusal passed through.
fn operation_error(error: &EntryError) -> OperationError {
    let code = ErrorCode::known(&error.code).unwrap_or(ErrorCode::INTERNAL_ERROR);
    match error.owner {
        ErrorOwner::Neo => OperationError::neo(code, &error.message),
        ErrorOwner::Script => OperationError::script(code, &error.message),
        ErrorOwner::Website => OperationError::website(code, &error.message),
        ErrorOwner::Host => OperationError::host(code, &error.message),
    }
}
