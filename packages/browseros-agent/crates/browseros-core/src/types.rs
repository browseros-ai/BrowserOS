//! `PageId` is BrowserOS's process-local handle for tools and page ownership. It remains stable
//! when a live Chrome tab is rebound, while the tab's `TargetId` and attached `SessionId`
//! protocol incarnations may change.

use serde::{Deserialize, Serialize};
use std::{fmt, hash::Hash};

macro_rules! id_newtype {
    ($name:ident, $inner:ty) => {
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
id_newtype!(TabId, i64);
id_newtype!(TargetId, String);
pub type SessionId = browseros_cdp::SessionId;
id_newtype!(FrameId, String);
id_newtype!(WindowId, i64);
id_newtype!(Ref, String);

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
