//! The boundary between the prelude's JSON and the crate's types.
//!
//! Every op input is split once: the header all ops share (`call`) is decoded by
//! `Playwright::invoke`, which opens the op's `OpScope` with it, and the rest is decoded by the
//! op's family into its own typed input. Both decodes use `deny_unknown_fields` (STYLE P9), and
//! a bad input is `script / invalid_input` before anything is sent, with the failing field
//! named (`target.chunks[0]: ...`). Family inputs therefore never declare `call`.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::op::Op;
use crate::op_scope::OpScope;
use crate::target::CallHeader;
use crate::{ErrorCode, PlaywrightError};

/// One op's input, split at the boundary.
#[derive(Debug, Deserialize)]
pub(crate) struct OpInput {
    /// The prelude always sends it; hand-written inputs (tests) may leave it out.
    #[serde(default)]
    pub(crate) call: CallHeader,
    /// Everything else, for the family to decode.
    #[serde(flatten)]
    pub(crate) args: OpArgs,
}

impl OpInput {
    /// Splits `input`. A header that does not decode fails with the op's own name, because
    /// the header is what names the API the script called.
    pub(crate) fn decode(op: Op, input: Value) -> Result<Self, PlaywrightError> {
        serde_path_to_error::deserialize(input)
            .map_err(|error| PlaywrightError::invalid_input(format!("{}: {error}", op.spec().name)))
    }
}

/// An op's own arguments, still JSON until its family decodes them.
#[derive(Debug, Deserialize)]
#[serde(transparent)]
pub(crate) struct OpArgs(Map<String, Value>);

impl OpArgs {
    /// Decodes the family's typed input, which declares `deny_unknown_fields`.
    pub(crate) fn decode<T: DeserializeOwned>(
        self,
        scope: &OpScope<'_>,
    ) -> Result<T, PlaywrightError> {
        serde_path_to_error::deserialize(Value::Object(self.0))
            .map_err(|error| scope.fail(ErrorCode::InvalidInput, &error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use neo_trace::ErrorOwner;
    use serde::Deserialize;
    use serde_json::json;

    use super::OpInput;
    use crate::ErrorCode;
    use crate::op::Op;
    use crate::op_scope::OpScope;
    use crate::target::{CallHeader, Strictness, Target};
    use crate::test_env::TestEnv;

    /// A family input shaped like the action family's.
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ClickArgs {
        target: Target,
        strict: Strictness,
    }

    fn click() -> Result<Op, Box<dyn Error>> {
        Ok(Op::from_name("locator.click").ok_or("locator.click is an op")?)
    }

    #[test]
    fn tests_that_the_header_is_split_from_the_arguments() -> Result<(), Box<dyn Error>> {
        let input = OpInput::decode(
            click()?,
            json!({ "call": { "api": "page.click", "timeoutMs": 250 }, "strict": "firstMatch" }),
        )?;
        assert_eq!(
            input.call,
            CallHeader {
                api: Some("page.click".to_owned()),
                timeout_ms: Some(250)
            }
        );
        assert_eq!(input.args.0.get("strict"), Some(&json!("firstMatch")));
        assert!(
            !input.args.0.contains_key("call"),
            "the family never sees the header"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_bad_header_is_invalid_input_named_after_the_op() -> Result<(), Box<dyn Error>> {
        let error = OpInput::decode(click()?, json!({ "call": { "timeout": 5 } }))
            .err()
            .ok_or("an unknown header field must fail")?;
        assert_eq!(error.error_code(), Some(ErrorCode::InvalidInput));
        assert_eq!(error.owner(), ErrorOwner::Script);
        assert!(
            error
                .message()
                .starts_with("locator.click: call.timeout: unknown field `timeout`"),
            "the failing field is named: {error}"
        );
        let error = OpInput::decode(click()?, json!(["not", "an", "object"]))
            .err()
            .ok_or("an input must be an object")?;
        assert_eq!(error.error_code(), Some(ErrorCode::InvalidInput));
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_family_arguments_fail_with_the_field_and_the_api()
    -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let input = OpInput::decode(
            click()?,
            json!({
                "call": { "api": "page.click" },
                "target": { "$neo": "locator", "page": { "$neo": "page", "id": 7, "tab": 1007 }, "display": "x",
                            "chunks": [{ "parts": [{ "name": "aria-ref", "body": " e5", "source": " e5" }] }] },
                "strict": "strict",
            }),
        )?;
        let scope = OpScope::new(&env, click()?, input.call);
        let error = input
            .args
            .decode::<ClickArgs>(&scope)
            .err()
            .ok_or("a ref with a space must fail")?;
        assert_eq!(error.error_code(), Some(ErrorCode::InvalidInput));
        assert!(
            error.message().starts_with("page.click: target: "),
            "{error}"
        );
        assert!(
            error.message().contains("aria-ref=\" e5\" is not a ref"),
            "{error}"
        );
        assert!(
            env.calls().is_empty(),
            "a decode failure reaches nothing on the host"
        );
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_family_arguments_decode_a_target() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let input = OpInput::decode(
            click()?,
            json!({
                "target": { "$neo": "locator", "page": { "$neo": "page", "id": 7, "tab": 1007 }, "display": "getByRef('f7e3')",
                            "chunks": [{ "parts": [{ "name": "aria-ref", "body": "f7e3", "source": "f7e3" }] }] },
                "strict": "strict",
            }),
        )?;
        let scope = OpScope::new(&env, click()?, input.call);
        let args: ClickArgs = input.args.decode(&scope)?;
        let Target::Locator(locator) = args.target else {
            panic!("a locator brand is a locator target");
        };
        assert_eq!(
            locator
                .chunks
                .last()
                .jump
                .map(|jump| jump.to_string())
                .as_deref(),
            Some("f7e3")
        );
        assert_eq!(args.strict, Strictness::Strict);
        Ok(())
    }
}
