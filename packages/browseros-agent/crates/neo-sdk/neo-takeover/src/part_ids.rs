//! The ids the takeover parts hand out: a UUIDv7 behind a short prefix, so a person reading a log
//! or a URL can tell a lock from a fit, and ids sort by creation time. The contract's ids are
//! plain strings, so the prefix costs a host nothing. Text that does not parse names no part;
//! the service answers 404 for it.

use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// Text that is not an id of the kind asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not an id Neo handed out")]
pub struct UnknownId;

macro_rules! part_id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(Uuid);

        impl $name {
            /// A new, time-ordered id.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, self.0.hyphenated())
            }
        }

        impl FromStr for $name {
            type Err = UnknownId;

            fn from_str(text: &str) -> Result<Self, UnknownId> {
                text.strip_prefix($prefix)
                    .and_then(|uuid| Uuid::try_parse(uuid).ok())
                    .map(Self)
                    .ok_or(UnknownId)
            }
        }
    };
}

part_id!(
    /// One lock, `lk_<uuid>`.
    LockId,
    "lk_"
);
part_id!(
    /// One fit, `ft_<uuid>`.
    FitId,
    "ft_"
);
part_id!(
    /// One takeover, `tk_<uuid>`.
    TakeoverId,
    "tk_"
);

#[cfg(test)]
mod tests {
    use super::{FitId, LockId, TakeoverId, UnknownId};

    #[test]
    fn an_id_reads_back_from_its_text_and_names_its_kind() {
        let lock = LockId::new();
        let text = lock.to_string();
        assert!(text.starts_with("lk_"), "{text}");
        assert_eq!(text.parse::<LockId>(), Ok(lock));
        assert!(FitId::new().to_string().starts_with("ft_"));
        assert!(TakeoverId::new().to_string().starts_with("tk_"));
    }

    #[test]
    fn text_that_is_not_an_id_of_the_kind_names_nothing() {
        let lock = LockId::new().to_string();
        let uuid = lock.trim_start_matches("lk_");
        for text in [
            uuid.to_string(),
            format!("tk_{uuid}"),
            "lk_".to_string(),
            "lk_not-a-uuid".to_string(),
            String::new(),
        ] {
            assert_eq!(text.parse::<LockId>(), Err(UnknownId), "{text:?}");
        }
    }
}
