//! JSON text with object keys in sorted order, whatever map serde_json was built with.
//!
//! serde_json's `Map` is a `BTreeMap` unless any crate in the build enables its
//! `preserve_order` feature, which turns every map in the process into an insertion-ordered
//! `IndexMap`. Feature unification makes that a property of the whole `cargo` invocation,
//! so text rendered from a `Value` would change with which other crates are built
//! alongside. Output that is stored, truncated or shown to an agent goes through here.

use serde::{Serialize, Serializer, ser::SerializeMap, ser::SerializeSeq};
use serde_json::Value;

/// Compact JSON with every object's keys sorted.
pub fn to_string(value: &Value) -> serde_json::Result<String> {
    serde_json::to_string(&Sorted(value))
}

/// Pretty JSON with every object's keys sorted.
pub fn to_string_pretty(value: &Value) -> serde_json::Result<String> {
    serde_json::to_string_pretty(&Sorted(value))
}

struct Sorted<'a>(&'a Value);

impl Serialize for Sorted<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Value::Object(object) => {
                let mut entries = object.iter().collect::<Vec<_>>();
                entries.sort_unstable_by_key(|(key, _)| *key);
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, &Sorted(value))?;
                }
                map.end()
            }
            Value::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(&Sorted(item))?;
                }
                seq.end()
            }
            scalar => scalar.serialize(serializer),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nested_objects_are_sorted_and_arrays_keep_their_order() -> serde_json::Result<()> {
        let value = json!({ "b": 1, "a": { "z": [3, { "y": 1, "x": 2 }], "m": null } });
        assert_eq!(
            to_string(&value)?,
            r#"{"a":{"m":null,"z":[3,{"x":2,"y":1}]},"b":1}"#
        );
        assert_eq!(
            to_string_pretty(&json!({ "b": true, "a": "s" }))?,
            "{\n  \"a\": \"s\",\n  \"b\": true\n}"
        );
        Ok(())
    }
}
