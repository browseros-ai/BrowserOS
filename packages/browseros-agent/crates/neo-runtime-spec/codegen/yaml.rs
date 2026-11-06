//! YAML 1.2 into an ordered, untyped tree, the way Playwright's own loader reads the spec.
//!
//! Playwright parses its spec with the JS `yaml` package: YAML 1.2 core schema, key order kept,
//! duplicate keys rejected. serde-saphyr (the workspace's YAML crate) behaves the same once
//! three choices are made, and this module is the only place that knows them:
//!
//! - `strict_booleans`: without it, YAML 1.1 forms count as booleans, so the render param `y`
//!   in `page.yml` would read as `true`.
//! - Untyped decoding: serde-saphyr refuses the plain `null` among `SerializedValue.v`'s
//!   literals when the target is `String`, so every node decodes through `deserialize_any` and
//!   the dialect, not serde, decides what a scalar means.
//! - `merge_keys: Error`: YAML 1.2 has no `<<` merge key, so the spec must not use one.
//!
//! The comment directly above a key (or trailing on its line) rides along as that value's
//! `doc`, captured with serde-saphyr's `Commented`, and becomes the Rust and TypeScript docs of
//! the generated item. serde-saphyr drops empty comment lines and a comment line that a blank line
//! separates from its key, so a doc comment is one paragraph sitting right above its key.

use std::fmt;
use std::str;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_saphyr::{Commented, DuplicateKeyPolicy, MergeKeyPolicy};

use crate::error::{At, SpecError};
use crate::spec_files::SpecFile;

/// A YAML value with the comment written above its key, if any.
pub struct Node {
    pub value: Value,
    /// Comment lines without their `#`, joined with `\n`; empty when there is none.
    pub doc: String,
}

/// A YAML value. Scalars keep the meaning YAML 1.2's core schema gives them.
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Seq(Vec<Node>),
    /// Entries in document order. Keys are strings, as in Playwright's JS loader.
    Map(Vec<(String, Node)>),
}

impl Value {
    /// How an error message names this kind of value.
    pub fn describe(&self) -> String {
        match self {
            Value::Null => "null".to_owned(),
            Value::Bool(value) => format!("the boolean {value}"),
            Value::Int(value) => format!("the number {value}"),
            Value::Float(value) => format!("the number {value}"),
            Value::Str(value) => format!("\"{value}\""),
            Value::Seq(_) => "a list".to_owned(),
            Value::Map(_) => "a map".to_owned(),
        }
    }
}

/// Parses one spec file into its root node.
pub fn parse(file: &SpecFile) -> Result<Node, SpecError> {
    let at = At::file(&file.path);
    // Playwright's loader reads the files as UTF-8.
    let text =
        str::from_utf8(&file.bytes).map_err(|error| at.error(format!("is not UTF-8 ({error})")))?;
    // The spec hash covers raw bytes, so a CRLF checkout would change it without changing the
    // spec. Refuse it here, where the message can say why.
    if text.contains('\r') {
        return Err(at.error(
            "contains a carriage return: spec files are hashed as raw bytes, so they must use LF \
             line endings (spec/.gitattributes marks them -text)"
                .to_owned(),
        ));
    }
    let options = serde_saphyr::options! {
        strict_booleans: true,
        duplicate_keys: DuplicateKeyPolicy::Error,
        merge_keys: MergeKeyPolicy::Error,
    };
    let value: Value = serde_saphyr::from_str_with_options(text, options)
        .map_err(|error| at.error(format!("is not valid YAML: {error}")))?;
    Ok(Node {
        value,
        doc: String::new(),
    })
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a YAML node")
    }

    // serde-saphyr reports `null`, `~` and an empty scalar as unit.
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Int(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Value, E> {
        i64::try_from(value).map(Value::Int).map_err(E::custom)
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
        Ok(Value::Float(value))
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::Str(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Value, E> {
        Ok(Value::Str(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(Commented(value, doc)) = seq.next_element::<Commented<Value>>()? {
            items.push(Node { value, doc });
        }
        Ok(Value::Seq(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut entries = Vec::new();
        while let Some((key, Commented(value, doc))) =
            map.next_entry::<String, Commented<Value>>()?
        {
            entries.push((key, Node { value, doc }));
        }
        Ok(Value::Map(entries))
    }
}
