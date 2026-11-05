//! claw-sdk's side of browseros-playwright: one `MethodSpec` per Playwright op, and an `OpEnv`
//! over the dispatcher's `OpCtx`.
//!
//! The dispatcher keeps its own chores (lookup, trace, policy, delivery, mask); this file only
//! translates between its types and the crate's. It names nothing beyond sdk-skeleton's
//! documented interfaces (design.md, Interfaces rev 3), so the same file compiles in
//! apps/claw-sdk, as `src/builtins/playwright.rs`, and, until that app exists, against the
//! mock in `tests/support/mock_skeleton.rs` (`tests/claw_sdk_adapter.rs`).
//!
//! ```text
//!  Registry ── MethodSpec per Op ── PlaywrightHandler::call(ctx, input)
//!                                       └─ Playwright::invoke(OpCtxEnv { ctx }, op, input)
//!                                            ├─ recheck, mark_sent, note ─▶ ctx
//!                                            └─ PlaywrightError ─ leaf_error ─▶ LeafError
//! ```

use std::sync::Arc;

use browseros_core::BrowserView;
use browseros_playwright::{
    Answer, Op, OpEnv, OpLifetime, OpTarget, Playwright, PlaywrightError, Refusal, RunState,
    TimedOut,
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::dispatch::{
    self, ErrorCode, Handler, LeafError, Lifetime, Listing, MethodSpec, OpCtx, OpName,
    OperationError, RegistryBuilder, RegistryError, Schema, Target, Visibility,
};

/// Registers every Playwright op with the dispatcher. Called once at startup, after
/// `Playwright::new`, while the registry is still being built.
pub(crate) fn register(
    registry: &mut RegistryBuilder,
    playwright: &Arc<Playwright>,
) -> Result<(), RegistryError> {
    for op in Op::all() {
        let spec = op.spec();
        registry.define(MethodSpec {
            name: OpName::from(spec.name),
            doc: spec.doc.into(),
            // The crate decodes every input once, with `deny_unknown_fields`, and fails with
            // the same `script / invalid_input`; a permissive schema keeps it the one gate.
            input: Schema::from(json!({ "type": "object" })),
            output: Schema::from(json!({})),
            effect: spec.effect,
            redaction: Arc::new(spec.redaction()),
            visibility: Visibility::Script,
            target: match spec.target {
                OpTarget::None => Target::None,
                OpTarget::Field(field) => Target::Field(field),
            },
            // The `run` tool's description lists the Playwright API once, not op by op.
            listing: Listing::Summarized,
            lifetime: match spec.lifetime {
                OpLifetime::Operation => Lifetime::Operation,
                OpLifetime::Background => Lifetime::Background,
            },
            handler: Arc::new(PlaywrightHandler {
                playwright: Arc::clone(playwright),
                op,
            }),
        })?;
    }
    Ok(())
}

/// One op's handler: the call goes to `Playwright::invoke` with an `OpEnv` over its context.
struct PlaywrightHandler {
    playwright: Arc<Playwright>,
    op: Op,
}

impl Handler for PlaywrightHandler {
    fn call<'a>(&'a self, ctx: &'a OpCtx, input: Value) -> BoxFuture<'a, Result<Value, LeafError>> {
        Box::pin(async move {
            let env = OpCtxEnv::new(ctx);
            self.playwright
                .invoke(&env, self.op, input)
                .await
                .map_err(|error| leaf_error(&error))
        })
    }
}

/// `OpEnv` for one call, over the dispatcher's `OpCtx`.
pub(crate) struct OpCtxEnv<'a> {
    ctx: &'a OpCtx,
    /// The run's Playwright state, which lives in the run's slot: created by the run's first
    /// op and dropped with the run.
    run_state: Arc<RunState>,
}

impl<'a> OpCtxEnv<'a> {
    /// The env for the op that `ctx` runs.
    pub(crate) fn new(ctx: &'a OpCtx) -> Self {
        Self {
            ctx,
            run_state: ctx.run().slot::<RunState>(),
        }
    }
}

impl OpEnv for OpCtxEnv<'_> {
    fn view(&self) -> &BrowserView {
        self.ctx.run().view()
    }

    fn run_state(&self) -> &Arc<RunState> {
        &self.run_state
    }

    fn recheck(&self) -> Result<(), Refusal> {
        self.ctx.recheck().map_err(refusal)
    }

    fn mark_sent(&self) {
        self.ctx.mark_sent();
    }

    fn mark_sensitive_target(&self) {
        self.ctx.mark_sensitive_target();
    }

    fn note(&self, line: String) {
        self.ctx.note(line);
    }
}

/// A policy's refusal from `OpCtx::recheck`, carried through the crate unchanged; the crate
/// hands it back as a `PlaywrightError` with the same owner, code, message and host code.
fn refusal(error: OperationError) -> Refusal {
    Refusal {
        owner: error.owner,
        code: error.code.as_str().to_owned(),
        message: error.message,
        host_code: error.host_code,
    }
}

/// The crate's error as the dispatcher's. Owner, code, timeout and answer come from the
/// crate's one error table; the dispatcher derives delivery from them and `mark_sent`.
pub(crate) fn leaf_error(error: &PlaywrightError) -> LeafError {
    LeafError {
        error: OperationError {
            owner: error.owner(),
            code: ErrorCode::from(error.code().to_owned()),
            message: error.message().to_owned(),
            host_code: error.host_code().map(str::to_owned),
            timed_out: match error.timed_out() {
                TimedOut::No => dispatch::TimedOut::No,
                TimedOut::Yes => dispatch::TimedOut::Yes,
            },
        },
        answer: match error.answer() {
            Answer::Definite => dispatch::Answer::Definite,
            Answer::Ambiguous => dispatch::Answer::Ambiguous,
        },
    }
}
