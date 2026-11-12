//! Which of an op's input fields are the script's options, and how they are nested.
//!
//! An op that takes script options carries them either in its input's `options` field (the
//! shape Neo has always sent, `{ target, strict, options: { force, .. } }`) or, once task 006
//! aligns it with Playwright, at the input's top level as upstream's params are
//! (`{ target, strict, force, .. }`). Fields the prelude computes itself (`publication`, `lease`,
//! `noWait`, ...) are never options: they live outside the bag, and on a flat input they are the
//! row's `adds`. The keys are what the prelude's `pick` may forward, and the generated file holds
//! each list to upstream's `<Command>Options` type.

use serde::Deserialize;
use serde::de::{self, Deserializer, Visitor};

/// Where an op input carries the script's options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionNesting {
    /// The op takes no script options.
    None,
    /// Under the input's `options` field, the shape Neo has always sent.
    Nested,
    /// At the input's top level beside Neo's addressing, as upstream's params are (task 006).
    /// The generated upstream check then compares the nesting too, so a bag left nested fails.
    Flat,
}

/// Neo's addressing fields: never a script option. The upstream check sets the same names aside
/// (`NeoAddressing` in the prelude's `upstream-check.ts`).
pub(crate) const ADDRESSING: &[&str] = &["target", "strict", "page", "frame"];

/// An op input's option keys. Implemented for every input type by [`options!`], never by hand.
pub(crate) trait OptionKeys {
    /// Where the input carries the script's options.
    const NESTING: OptionNesting;

    /// The wire names of the fields that may hold options, in declaration order: the `options`
    /// struct's for a nested input, the input's own for a flat one (`Describe` then sets Neo's
    /// addressing and the row's `adds` aside), none for an input without options.
    fn option_keys() -> Vec<&'static str>;
}

/// Declares an input's options. `options!(ClickInput => ClickOptions)` reads the keys from
/// `ClickOptions`, and fails to compile unless `ClickInput` has an `options` field of that type;
/// `options!(ClickInput, flat)` declares the input's own fields as the options, as upstream's
/// params are; `options!(TitleInput)` declares an input without options.
macro_rules! options {
    ($input:ty => $options:ty) => {
        impl $crate::wire::OptionKeys for $input {
            const NESTING: $crate::wire::OptionNesting = $crate::wire::OptionNesting::Nested;

            fn option_keys() -> Vec<&'static str> {
                // Ties the keys to the input's own `options` field: naming another struct here
                // does not compile, so the keys cannot drift from what the input decodes.
                let _: fn(&$input) -> &$options = |input| &input.options;
                $crate::wire::serde_fields::<$options>()
            }
        }
    };
    ($input:ty,flat) => {
        impl $crate::wire::OptionKeys for $input {
            const NESTING: $crate::wire::OptionNesting = $crate::wire::OptionNesting::Flat;

            fn option_keys() -> Vec<&'static str> {
                $crate::wire::serde_fields::<$input>()
            }
        }
    };
    ($input:ty) => {
        impl $crate::wire::OptionKeys for $input {
            const NESTING: $crate::wire::OptionNesting = $crate::wire::OptionNesting::None;

            fn option_keys() -> Vec<&'static str> {
                Vec::new()
            }
        }
    };
}

pub(crate) use options;

/// serde's own list of `T`'s fields, in wire spelling: the derive hands it to
/// `deserialize_struct`, so `rename` and `rename_all` apply and nothing is restated. Empty for a
/// type that does not decode as a plain struct (an enum, a map, a struct with a `flatten` field).
#[must_use]
pub(crate) fn serde_fields<'de, T: Deserialize<'de>>() -> Vec<&'static str> {
    let mut probe = FieldProbe::default();
    // The probe refuses every value; it only reads the field list on the way in.
    let _refused = T::deserialize(&mut probe);
    probe.fields
}

/// A deserializer that records the field list a struct's derive asks for, then fails.
#[derive(Default)]
struct FieldProbe {
    fields: Vec<&'static str>,
}

impl<'de> Deserializer<'de> for &mut FieldProbe {
    type Error = de::value::Error;

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.fields = fields.to_vec();
        Err(de::Error::custom("only the field list was read"))
    }

    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        Err(de::Error::custom("not a struct"))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf option
        unit unit_struct newtype_struct seq tuple tuple_struct map enum identifier ignored_any
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde::Deserialize;

    use super::{OptionKeys, OptionNesting, serde_fields};

    /// A script option bag in the shape the families use: camelCase on the wire.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    #[expect(
        dead_code,
        reason = "only the field list and one decoded field are read"
    )]
    struct ProbeOptions {
        force: Option<bool>,
        click_count: Option<u32>,
        #[serde(rename = "noWaitAfter")]
        wait: Option<bool>,
        #[serde(skip)]
        local: (),
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WithOptions {
        options: ProbeOptions,
    }
    options!(WithOptions => ProbeOptions);

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WithoutOptions {
        _url: String,
    }
    options!(WithoutOptions);

    /// An input aligned with upstream: its options beside its addressing.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    #[expect(dead_code, reason = "only the field list is read")]
    struct FlatInput {
        target: (),
        click_count: Option<u32>,
    }
    options!(FlatInput, flat);

    #[test]
    fn tests_that_option_keys_are_the_options_struct_fields_in_wire_spelling() {
        assert_eq!(
            WithOptions::option_keys(),
            ["force", "clickCount", "noWaitAfter"],
            "rename_all and rename apply, and a skipped field is no key"
        );
        let decoded: Result<WithOptions, _> =
            serde_json::from_str(r#"{ "options": { "clickCount": 2 } }"#);
        assert_eq!(
            decoded.map(|input| input.options.click_count).ok(),
            Some(Some(2)),
            "the probe reads the same names the decode accepts"
        );
    }

    #[test]
    fn tests_that_an_input_without_options_has_no_keys() {
        assert!(
            WithoutOptions::option_keys().is_empty(),
            "an op that takes no option bag gets no OPTION_KEYS entry"
        );
        assert_eq!(WithoutOptions::NESTING, OptionNesting::None);
    }

    #[test]
    fn tests_that_a_flat_input_offers_its_own_fields() {
        assert_eq!(FlatInput::NESTING, OptionNesting::Flat);
        assert_eq!(
            FlatInput::option_keys(),
            ["target", "clickCount"],
            "every wire field, in wire spelling; Describe sets the addressing aside"
        );
        assert_eq!(WithOptions::NESTING, OptionNesting::Nested);
    }

    #[test]
    fn tests_that_a_type_that_is_not_a_struct_has_no_fields() {
        assert!(serde_fields::<String>().is_empty());
        assert!(
            serde_fields::<BTreeMap<String, u32>>().is_empty(),
            "a map has keys, not fields"
        );
    }
}
