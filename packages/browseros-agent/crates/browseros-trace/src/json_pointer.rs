//! A path into an operation's input, naming a field to redact (RFC 6901, plus `*` for "every
//! key or index here"). Method specs declare these: `/password` is always secret, and
//! `/fields/*/value` covers each field of a fill.

use serde_json::Value;

/// A JSON pointer whose `*` segments match every member of an object or array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonPointer {
    segments: Vec<String>,
}

/// A pointer that is not `""` or does not start with `/`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a JSON pointer must start with '/': {0:?}")]
pub struct PointerError(String);

impl JsonPointer {
    /// Parses `/a/b~1c/*`. `~1` is `/` and `~0` is `~`, as in RFC 6901.
    ///
    /// # Errors
    /// When the text is empty or does not start with `/`.
    pub fn parse(text: &str) -> Result<Self, PointerError> {
        let Some(rest) = text.strip_prefix('/') else {
            return Err(PointerError(text.to_string()));
        };
        Ok(Self {
            segments: rest
                .split('/')
                .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
                .collect(),
        })
    }

    /// Replaces every value the pointer reaches with `replacement`. Returns whether any did.
    pub fn replace_in(&self, value: &mut Value, replacement: &Value) -> bool {
        replace(value, &self.segments, replacement)
    }
}

fn replace(value: &mut Value, segments: &[String], replacement: &Value) -> bool {
    let Some((head, rest)) = segments.split_first() else {
        *value = replacement.clone();
        return true;
    };
    // A wildcard visits every member: stopping at the first hit would leave the rest unredacted.
    match value {
        Value::Object(map) if head == "*" => {
            let mut hit = false;
            for child in map.values_mut() {
                hit |= replace(child, rest, replacement);
            }
            hit
        }
        Value::Object(map) => map
            .get_mut(head)
            .is_some_and(|child| replace(child, rest, replacement)),
        Value::Array(items) if head == "*" => {
            let mut hit = false;
            for child in items.iter_mut() {
                hit |= replace(child, rest, replacement);
            }
            hit
        }
        Value::Array(items) => head
            .parse::<usize>()
            .ok()
            .and_then(|index| items.get_mut(index))
            .is_some_and(|child| replace(child, rest, replacement)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::JsonPointer;
    use serde_json::{Value, json};

    fn redact(pointer: &str, mut value: Value) -> Value {
        if let Ok(pointer) = JsonPointer::parse(pointer) {
            pointer.replace_in(&mut value, &json!("[redacted]"));
        }
        value
    }

    #[test]
    fn a_plain_pointer_replaces_one_field() {
        assert_eq!(
            redact(
                "/user/password",
                json!({"user": {"password": "p", "name": "n"}})
            ),
            json!({"user": {"password": "[redacted]", "name": "n"}})
        );
    }

    #[test]
    fn a_wildcard_reaches_every_member() {
        assert_eq!(
            redact(
                "/fields/*/value",
                json!({"fields": [{"value": "a"}, {"value": "b", "k": 1}]})
            ),
            json!({"fields": [{"value": "[redacted]"}, {"value": "[redacted]", "k": 1}]})
        );
        assert_eq!(
            redact("/fields/*", json!({"fields": {"u": "a", "p": "b"}})),
            json!({"fields": {"u": "[redacted]", "p": "[redacted]"}})
        );
    }

    #[test]
    fn escapes_and_indexes_follow_rfc_6901() {
        assert_eq!(
            redact("/a~1b", json!({"a/b": 1})),
            json!({"a/b": "[redacted]"})
        );
        assert_eq!(
            redact("/list/1", json!({"list": [1, 2]})),
            json!({"list": [1, "[redacted]"]})
        );
    }

    #[test]
    fn a_missing_path_changes_nothing() {
        let value = json!({"a": 1});
        assert_eq!(redact("/b/c", value.clone()), value);
        assert!(JsonPointer::parse("no-slash").is_err());
    }
}
