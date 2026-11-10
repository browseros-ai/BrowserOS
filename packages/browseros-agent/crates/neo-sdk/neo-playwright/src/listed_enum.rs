//! `listed_enum!`: a closed list of variants that also knows itself as data.
//!
//! The op families and the error codes are lists the host registers, the prelude mirrors and
//! the tests walk in full. Declaring the variants once and deriving `ALL` from the same tokens
//! means a variant cannot exist without being registered, generated and tested; a hand-kept
//! `ALL` array would let a new variant compile and then silently never reach claw-sdk.

/// Declares an enum (always `Clone, Copy, Debug, PartialEq, Eq, Hash`) and an associated
/// `ALL` constant that holds every variant in declaration order.
macro_rules! listed_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $(#[$variant_meta:meta])* $variant:ident ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        $vis enum $name {
            $( $(#[$variant_meta])* $variant ),+
        }

        impl $name {
            /// Every variant, in declaration order.
            $vis const ALL: &'static [Self] = &[$(Self::$variant),+];
        }
    };
}

pub(crate) use listed_enum;
