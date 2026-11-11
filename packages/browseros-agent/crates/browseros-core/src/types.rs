//! `PageId` is BrowserOS's process-local handle for tools and page ownership. It remains stable
//! when a live Chrome tab is rebound, while the tab's `TargetId` and attached `SessionId`
//! protocol incarnations may change.

use serde::{Deserialize, Serialize};
use std::{fmt, hash::Hash};

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident, $inner:ty) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
        )]
        pub struct $name(pub $inner);

        impl $name {
            #[must_use]
            pub fn into_inner(self) -> $inner {
                self.0
            }
        }

        impl From<$inner> for $name {
            fn from(value: $inner) -> Self {
                Self(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_newtype!(PageId, u32);
// The three ids neo-playwright's op wire carries also derive ts-rs's `TS` under the `ts-rs`
// feature, so its generated prelude types name them; each prints as its inner type
// (`type TabId = number` under that crate's 64-bit-as-number rule).
id_newtype!(
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
    TabId,
    i64
);
impl Copy for TabId {}
id_newtype!(
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
    TargetId,
    String
);
pub type SessionId = browseros_cdp::SessionId;
id_newtype!(
    #[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
    FrameId,
    String
);
id_newtype!(WindowId, i64);
impl Copy for WindowId {}
id_newtype!(Ref, String);
// Saved handles describe catalog entries for one native process lifetime; live context IDs
// describe loaded BrowserContexts. Neither is Claw's persisted agent configuration identity.
id_newtype!(BrowserProfileId, String);
id_newtype!(BrowserContextId, String);

impl BrowserProfileId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl BrowserContextId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TargetId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FrameId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Ref {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
