//! JavaScript's own semantics, for each byte of a snapshot's text that they decide.
//!
//! The TypeScript renderer runs in QuickJS, so its lengths are UTF-16 units, its `\s` and `trim`
//! are ECMA-262's whitespace set, `Number(string)` is `StringToNumber`, numbers print as
//! `Number::toString`, names are quoted by `JSON.stringify`, and `Math.round` takes a half up.
//! Rust's defaults differ on every one, so printing calls these and never `str::len`,
//! `char::is_whitespace`, `str::trim`, `f64::from_str`, `{}` on a float or `f64::round`.
//!
//! Each leaf names the exact operation it reproduces and where Rust's default differs.
//! `tests.rs` runs every one against the same expression in QuickJS, the engine the TypeScript
//! runs in.

mod json_quote;
mod math_round;
mod number_text;
mod space;
mod string_to_number;
#[cfg(test)]
mod tests;
mod utf16;

pub(crate) use json_quote::json_quote;
pub(crate) use math_round::math_round;
pub(crate) use number_text::number_text;
pub(crate) use space::{is_space, trim, trim_end, trim_start};
pub(crate) use string_to_number::is_number;
pub(crate) use utf16::{prefix_as_javascript, prefix_between_characters, skip_units, units};
