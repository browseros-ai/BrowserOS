//! Replay recorded typed answers through the family's real route and Run encoder. The stub
//! replaces browser work, while the row still chooses the input/output types; reply bytes
//! therefore include `to_output`'s sorted Value map, rather than struct declaration order.

use std::any::{Any, TypeId};
use std::error::Error;
use std::future::Future;

use futures_util::future::BoxFuture;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use ts_rs::TS;

use super::{ActionOp, Clicked, route};
use crate::action::requested_navigations::NavigationCut;
use crate::op::Op;
use crate::op_input::{OpArgs, OpInput};
use crate::op_scope::OpScope;
use crate::test_env::{self, TestEnv};
use crate::wire::{OptionKeys, Route, Run, Upstream};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// The three actual Action reply types, reconstructed from the saved before-build bodies.
/// Keeping this conversion typed makes a changed handler result fail the replay explicitly.
fn recorded<O: 'static>(body: Value) -> Result<O, Box<dyn Error>> {
    let value: Box<dyn Any> = if TypeId::of::<O>() == TypeId::of::<Option<Clicked>>() {
        let clicked = if body.is_null() {
            None
        } else {
            let navigation = body.get("navigation").ok_or("missing navigation cut")?;
            Some(Clicked {
                navigation: NavigationCut {
                    seq: navigation
                        .get("seq")
                        .and_then(Value::as_u64)
                        .ok_or("cut seq")?,
                    at: navigation
                        .get("at")
                        .and_then(Value::as_u64)
                        .ok_or("cut time")?,
                },
            })
        };
        Box::new(clicked)
    } else if TypeId::of::<O>() == TypeId::of::<Vec<String>>() {
        Box::new(serde_json::from_value::<Vec<String>>(body)?)
    } else if TypeId::of::<O>() == TypeId::of::<()>() && body.is_null() {
        Box::new(())
    } else {
        return Err("the action row has an unexpected recorded result type".into());
    };
    value
        .downcast::<O>()
        .map(|value| *value)
        .map_err(|_wrong_type| "recorded result type mismatch".into())
}

struct Reply<'a> {
    run: Run<'a>,
    body: Value,
}

impl<'a> Route<'a> for Reply<'a> {
    type Out = BoxFuture<'a, Result<Value, PlaywrightError>>;

    fn op<U, I, O, F, Fut>(self, upstream: U, handler: F) -> Self::Out
    where
        U: Into<Upstream>,
        I: DeserializeOwned + TS + OptionKeys + Send + 'static,
        O: Serialize + TS + 'static,
        F: FnOnce(&'a Playwright, &'a OpScope<'a>, I) -> Fut + Send + 'a,
        Fut: Future<Output = Result<O, PlaywrightError>> + Send + 'a,
    {
        let body = self.body;
        // Run still decodes the real row's I and encodes its real O via to_output. Only the
        // nondeterministic browser action is replaced by its saved, type-checked reply.
        drop(handler);
        self.run
            .op(upstream, move |_pw, scope, _input: I| async move {
                recorded::<O>(body)
                    .map_err(|error| scope.fail(ErrorCode::Internal, &error.to_string()))
            })
    }

    fn untyped<F, Fut>(self, _handler: F) -> Self::Out
    where
        F: FnOnce(&'a Playwright, &'a OpScope<'a>, OpArgs) -> Fut + Send + 'a,
        Fut: Future<Output = Result<Value, PlaywrightError>> + Send + 'a,
    {
        Box::pin(async move {
            Err(self.run.scope.fail(
                ErrorCode::Internal,
                "a recorded action bypassed its typed row",
            ))
        })
    }
}

pub(super) async fn encode(
    op: ActionOp,
    input: Value,
    body: Value,
) -> Result<Value, Box<dyn Error>> {
    let env = TestEnv::new();
    let pw = Playwright::new(test_env::offline_session());
    let registered = Op::from_name(op.spec().name).ok_or("registered action")?;
    let OpInput { call, args } = OpInput::decode(registered, input)?;
    let scope = OpScope::new(&env, registered, call);
    Ok(route(
        op,
        Reply {
            run: Run {
                pw: &pw,
                scope: &scope,
                args,
            },
            body,
        },
    )
    .await?)
}
