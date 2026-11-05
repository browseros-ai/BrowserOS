//! The one list of built-ins.

use super::placeholder_page::placeholder_page;
use crate::dispatch::{RegistryBuilder, RegistryError};

/// Adds every built-in to `builder`. Called once at startup; a duplicate name fails the start.
/// `neo.solveCaptcha` joins this list with its own dependencies.
pub(crate) fn builtin_set(builder: &mut RegistryBuilder) -> Result<(), RegistryError> {
    placeholder_page(builder)
}
