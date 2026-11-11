//! What a script types, as an op input carries it.

use std::fmt;

use serde::Deserialize;

/// A key, a chord or a text the script types. It can be a password typed key by key, so its
/// `Debug` never prints it (STYLE P4); the call log shows it as upstream's does.
#[derive(Clone, Deserialize)]
#[serde(transparent)]
pub(crate) struct Typed(pub(crate) String);

impl fmt::Debug for Typed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[typed]")
    }
}
