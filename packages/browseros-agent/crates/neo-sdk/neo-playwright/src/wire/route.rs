//! The visitor each family's `route` match is walked with: [`Run`] for `Playwright::invoke`,
//! [`Describe`] for `Op::wire`.

use std::future::Future;

use futures_util::future::BoxFuture;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use ts_rs::TS;

use super::{OpWire, OptionKeys, Upstream};
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::{Playwright, PlaywrightError, op_output};

/// What an op answers where Playwright's command returns nothing: JSON `null`, which ts-rs
/// prints as `null`. A unit type, so a handler cannot answer `{}` by accident.
pub(crate) type Nothing = ();

// Compiles only while `serde_json::Value` has no `TS` impl, which ts-rs's `serde-json-impl`
// feature would add for the whole build if any crate in the graph turned it on: a handler that
// answers a bare `Value` must stay a compile error, not print `JsonValue`.
const _: fn() = || {
    trait AmbiguousIfTs<Marker> {
        fn probe() {}
    }
    impl<T: ?Sized> AmbiguousIfTs<()> for T {}
    struct HasTs;
    impl<T: ?Sized + TS> AmbiguousIfTs<HasTs> for T {}
    let _ = <Value as AmbiguousIfTs<_>>::probe;
};

/// The two walks over a family's `route` match. A family never implements it; each arm calls
/// [`Route::op`] once. The bounds are the op contract, checked at compile time for every row:
///
/// - `I: DeserializeOwned + TS + OptionKeys`: the input decodes, prints itself as TypeScript and
///   names its option bag (`options!`);
/// - `O: Serialize + TS`: the result encodes and prints itself, so a bare `serde_json::Value`,
///   which has no `TS` impl here, does not compile as a result;
/// - `'static` on both, because ts-rs walks types by `TypeId`; inputs and results are owned;
/// - `Fut: Send`, because the door's future is (`neo-server`'s `playwright_ops.rs`).
pub(crate) trait Route<'a> {
    /// `Run`'s op future, or `Describe`'s wire description.
    type Out;

    /// The arm for one op: its place in Playwright's protocol and its handler.
    fn op<U, I, O, F, Fut>(self, upstream: U, handler: F) -> Self::Out
    where
        U: Into<Upstream>,
        I: DeserializeOwned + TS + OptionKeys + Send + 'static,
        O: Serialize + TS + 'static,
        F: FnOnce(&'a Playwright, &'a OpScope<'a>, I) -> Fut + Send + 'a,
        Fut: Future<Output = Result<O, PlaywrightError>> + Send + 'a;
}

/// Runs the op: decodes `I` from the op's arguments with `OpArgs::decode` (one boundary, the
/// failing field named), awaits the handler, and encodes its result with `to_output`.
pub(crate) struct Run<'a> {
    pub(crate) pw: &'a Playwright,
    pub(crate) scope: &'a OpScope<'a>,
    pub(crate) args: OpArgs,
}

impl<'a> Route<'a> for Run<'a> {
    type Out = BoxFuture<'a, Result<Value, PlaywrightError>>;

    fn op<U, I, O, F, Fut>(self, _upstream: U, handler: F) -> Self::Out
    where
        U: Into<Upstream>,
        I: DeserializeOwned + TS + OptionKeys + Send + 'static,
        O: Serialize + TS + 'static,
        F: FnOnce(&'a Playwright, &'a OpScope<'a>, I) -> Fut + Send + 'a,
        Fut: Future<Output = Result<O, PlaywrightError>> + Send + 'a,
    {
        let Self { pw, scope, args } = self;
        // Boxed so each family's `invoke` has one future type whichever arm ran.
        Box::pin(async move {
            let input: I = args.decode(scope)?;
            let output = handler(pw, scope, input).await?;
            op_output::to_output(scope, &output)
        })
    }
}

/// Describes the op without running it: never calls the handler and touches no browser or file.
pub(crate) struct Describe;

impl<'a> Route<'a> for Describe {
    type Out = OpWire;

    fn op<U, I, O, F, Fut>(self, upstream: U, _handler: F) -> OpWire
    where
        U: Into<Upstream>,
        I: DeserializeOwned + TS + OptionKeys + Send + 'static,
        O: Serialize + TS + 'static,
        F: FnOnce(&'a Playwright, &'a OpScope<'a>, I) -> Fut + Send + 'a,
        Fut: Future<Output = Result<O, PlaywrightError>> + Send + 'a,
    {
        OpWire::of::<I, O>(I::NESTING, I::option_keys(), upstream.into())
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};
    use ts_rs::TS;

    use crate::op::Op;
    use crate::op_input::OpInput;
    use crate::op_scope::OpScope;
    use crate::test_env::{self, TestEnv};
    use crate::wire::{
        Describe, Nothing, Route, Run, Upstream, command, neo_only, options, replaces,
    };
    use crate::{ErrorCode, Playwright, PlaywrightError};

    /// A family of four ops written the way the families write theirs.
    #[derive(Clone, Copy)]
    enum ProbeOp {
        Echo,
        Quiet,
        Counted,
        Shuffled,
    }

    #[derive(Debug, Deserialize, TS)]
    #[serde(deny_unknown_fields)]
    struct EchoInput {
        text: String,
        options: EchoOptions,
    }
    options!(EchoInput => EchoOptions);

    #[derive(Debug, Deserialize, TS)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    #[ts(optional_fields)]
    struct EchoOptions {
        upper_case: Option<bool>,
    }

    #[derive(Debug, Deserialize, TS)]
    #[serde(deny_unknown_fields)]
    struct QuietInput {}
    options!(QuietInput);

    #[derive(Debug, Serialize, TS)]
    struct Echoed {
        text: String,
    }

    /// A reply declaring its fields out of alphabetical order, nested too, as `NavigationResult`
    /// and `FrameInfo` do.
    #[derive(Debug, Serialize, TS)]
    struct Shuffled {
        zulu: u8,
        alpha: ShuffledInner,
        mike: u8,
    }

    #[derive(Debug, Serialize, TS)]
    struct ShuffledInner {
        yankee: u8,
        bravo: u8,
    }

    fn shuffled_reply() -> Shuffled {
        Shuffled {
            zulu: 4,
            alpha: ShuffledInner {
                yankee: 1,
                bravo: 2,
            },
            mike: 3,
        }
    }

    fn route<'a, R: Route<'a>>(op: ProbeOp, r: R) -> R::Out {
        match op {
            ProbeOp::Echo => r.op(command("Frame.echo").result("the echoed text"), echo),
            ProbeOp::Quiet => r.op(neo_only("a test op"), quiet),
            ProbeOp::Counted => r.op(replaces("Frame.count", "counts by hand"), counted),
            ProbeOp::Shuffled => r.op(neo_only("a test op"), shuffled),
        }
    }

    async fn echo(
        _pw: &Playwright,
        scope: &OpScope<'_>,
        input: EchoInput,
    ) -> Result<Echoed, PlaywrightError> {
        scope.log(format!("echo {}", input.text));
        let text = if input.options.upper_case.unwrap_or(false) {
            input.text.to_uppercase()
        } else {
            input.text
        };
        Ok(Echoed { text })
    }

    async fn quiet(
        _pw: &Playwright,
        _scope: &OpScope<'_>,
        _input: QuietInput,
    ) -> Result<Nothing, PlaywrightError> {
        Ok(())
    }

    async fn counted(
        _pw: &Playwright,
        _scope: &OpScope<'_>,
        _input: QuietInput,
    ) -> Result<u32, PlaywrightError> {
        Ok(3)
    }

    async fn shuffled(
        _pw: &Playwright,
        _scope: &OpScope<'_>,
        _input: QuietInput,
    ) -> Result<Shuffled, PlaywrightError> {
        Ok(shuffled_reply())
    }

    fn assert_send<T: Send>(_: &T) {}

    #[test]
    fn tests_that_describe_reads_the_row_and_the_handler_types() {
        let echo = route(ProbeOp::Echo, Describe);
        assert_eq!(
            (echo.input.as_str(), echo.output.as_str()),
            ("EchoInput", "Echoed")
        );
        assert_eq!(echo.option_keys, ["upperCase"]);
        assert_eq!(
            echo.upstream,
            Upstream::Command(command("Frame.echo").result("the echoed text"))
        );
        let quiet = route(ProbeOp::Quiet, Describe);
        assert_eq!(quiet.output, "null", "a void op answers null");
        assert!(
            quiet.option_keys.is_empty(),
            "options!(QuietInput) has none"
        );
        assert_eq!(quiet.upstream, neo_only("a test op"));
        let counted = route(ProbeOp::Counted, Describe);
        assert_eq!(counted.output, "number");
        assert!(
            counted.references == ["QuietInput"],
            "a primitive result names no type"
        );
    }

    /// One op through `Run`, as a family's `invoke` sends it.
    async fn run(op: ProbeOp, input: Value) -> Result<Value, PlaywrightError> {
        let env = TestEnv::new();
        let pw = Playwright::new(test_env::offline_session());
        let registered = Op::all()
            .next()
            .ok_or_else(|| PlaywrightError::invalid_input("the crate registers ops".to_owned()))?;
        let OpInput { call, args } = OpInput::decode(registered, input)?;
        let scope = OpScope::new(&env, registered, call);
        let future = route(
            op,
            Run {
                pw: &pw,
                scope: &scope,
                args,
            },
        );
        assert_send(&future);
        future.await
    }

    #[tokio::test]
    async fn tests_that_run_decodes_calls_and_encodes_the_handler() -> Result<(), Box<dyn Error>> {
        let echoed = run(
            ProbeOp::Echo,
            json!({ "text": "hi", "options": { "upperCase": true } }),
        )
        .await?;
        assert_eq!(echoed, json!({ "text": "HI" }));
        let quiet = run(ProbeOp::Quiet, json!({})).await?;
        assert_eq!(quiet, Value::Null, "`Nothing` is JSON null, never {{}}");
        assert_eq!(run(ProbeOp::Counted, json!({})).await?, json!(3));
        Ok(())
    }

    /// `op_output::to_output`'s guarantee: a reply whose type declares its fields out of order
    /// leaves the route with its keys sorted at every level. A failure here means a crate in the
    /// build turned on `serde_json`'s `preserve_order` (the `Map` assertion) or answers stopped
    /// going through `to_value`, and every reply type declared out of order now sends its keys
    /// in declaration order. `check.sh` tests neo-playwright together with neo-server, so the
    /// `Map` assertion sees the server binary's `serde_json` features.
    #[tokio::test]
    async fn tests_that_an_answer_leaves_with_sorted_keys() -> Result<(), Box<dyn Error>> {
        let mut map = serde_json::Map::new();
        map.insert("b".to_owned(), json!(1));
        map.insert("a".to_owned(), json!(2));
        assert!(
            map.keys().eq(["a", "b"]),
            "serde_json's Map must not keep insertion order: {map:?}"
        );
        let answer = run(ProbeOp::Shuffled, json!({})).await?;
        assert_eq!(
            serde_json::to_string(&answer)?,
            r#"{"alpha":{"bravo":2,"yankee":1},"mike":3,"zulu":4}"#
        );
        // The probe is out of order: written straight to text it keeps declaration order.
        assert_eq!(
            serde_json::to_string(&shuffled_reply())?,
            r#"{"zulu":4,"alpha":{"yankee":1,"bravo":2},"mike":3}"#
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_run_refuses_an_unknown_field_before_the_handler()
    -> Result<(), Box<dyn Error>> {
        let error = run(
            ProbeOp::Echo,
            json!({ "text": "hi", "options": { "upper": true } }),
        )
        .await
        .err()
        .ok_or("an unknown option must fail")?;
        assert_eq!(error.error_code(), Some(ErrorCode::InvalidInput));
        assert!(
            error
                .message()
                .contains("options.upper: unknown field `upper`"),
            "the decode names the field: {error}"
        );
        Ok(())
    }
}
