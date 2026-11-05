//! The `run` tool's arguments, as the model writes them.

use rmcp::model::JsonObject;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::dispatch::{ErrorCode, OperationError};

/// The `run` tool's arguments, and, derived from this type, its input schema: what a model
/// sends and what it reads about it are one definition. Unknown fields and `timeout: null` are
/// refused rather than ignored: the model meant something by them. The field docs are the
/// schema's descriptions.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunArgs {
    /// JavaScript: the body of an async function. `await` each SDK call and `return` a JSON
    /// value.
    pub(crate) code: String,
    /// The run's budget in milliseconds, from 1 to 900000. Leave it out for 300000.
    #[serde(default, deserialize_with = "present")]
    #[schemars(with = "u64", range(min = 1, max = 900_000))]
    pub(crate) timeout: Option<u64>,
}

impl RunArgs {
    /// Decodes the tool call's arguments. Every failure is the model's: `script /
    /// invalid_input`, naming the field.
    pub(crate) fn parse(arguments: Option<JsonObject>) -> Result<Self, OperationError> {
        let arguments = Value::Object(arguments.unwrap_or_default());
        serde_path_to_error::deserialize(arguments).map_err(|error| {
            let path = error.path().to_string();
            let message = if path == "." {
                format!("run: {}", error.inner())
            } else {
                format!("run: {} (at `{path}`)", error.inner())
            };
            OperationError::script(ErrorCode::INVALID_INPUT, message)
        })
    }
}

/// A `timeout` that is present must be a number: an explicit `null` fails here instead of
/// meaning "the default".
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    u64::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_trace::ErrorOwner;
    use rmcp::handler::server::common::schema_for_type;
    use rmcp::model::JsonObject;
    use serde_json::{Value, json};

    use super::RunArgs;
    use crate::dispatch::ErrorCode;

    fn arguments(value: Value) -> Option<JsonObject> {
        match value {
            Value::Object(map) => Some(map),
            _ => None,
        }
    }

    #[test]
    fn code_is_required_and_timeout_is_optional() -> Result<(), Box<dyn Error>> {
        let both = RunArgs::parse(arguments(json!({ "code": "return 1;", "timeout": 5000 })))?;
        assert_eq!(
            (both.code.as_str(), both.timeout),
            ("return 1;", Some(5000))
        );
        let code_only = RunArgs::parse(arguments(json!({ "code": "return 1;" })))?;
        assert_eq!(code_only.timeout, None, "the default budget");
        Ok(())
    }

    #[test]
    fn what_the_model_did_not_mean_to_send_is_refused() {
        for bad in [
            json!({ "code": "return 1;", "timeout": null }),
            json!({ "code": "return 1;", "timout": 5000 }),
            json!({ "code": 7 }),
            json!({}),
        ] {
            let error = RunArgs::parse(arguments(bad.clone())).err();
            assert_eq!(
                error.map(|error| (error.owner(), error.code())),
                Some((ErrorOwner::Script, ErrorCode::INVALID_INPUT)),
                "{bad} is the model's mistake"
            );
        }
    }

    #[test]
    fn the_input_schema_describes_exactly_these_arguments() {
        let schema = schema_for_type::<RunArgs>();
        assert_eq!(schema.get("additionalProperties"), Some(&json!(false)));
        assert_eq!(schema.get("required"), Some(&json!(["code"])));
        let timeout = schema
            .get("properties")
            .and_then(|properties| properties.get("timeout"));
        assert_eq!(
            timeout.and_then(|timeout| timeout.get("maximum")),
            Some(&json!(900_000))
        );
    }
}
