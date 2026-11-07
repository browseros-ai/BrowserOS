//! Neo: the Rust type of Playwright's protocol `binary`, the seam every byte payload crosses
//! (screenshots, PDFs, response and download bodies, route bodies, `setInputFiles` buffers).
//!
//! ```text
//!  neo-runtime                          wire (JSON)                     neo-sdk (QuickJS)
//!  Binary(Vec<u8>) ──serialize──▶ "iVBORw0KGgo…" (base64) ──▶ Buffer.fromBinary → Buffer
//!  Binary(Vec<u8>) ◀─deserialize── "aGVsbG8="             ◀── toBinary(Uint8Array)
//! ```
//!
//! The wire form is Playwright's: one string of standard base64 with padding, in both
//! directions. Decoding is strict, so text that is not exactly that fails the call's params
//! rather than turning into other bytes.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};

/// Playwright's protocol `binary`: bytes that cross the wire as standard base64 text. The
/// translator types every `binary` field of a served command with it, and TypeScript sees the
/// same field as `string`.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Binary(pub Vec<u8>);

impl Binary {
    /// The bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The bytes, without a copy.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// How many bytes this holds, not the length of its base64 text.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<u8>> for Binary {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl AsRef<[u8]> for Binary {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

// A screenshot is megabytes; a call record or a test failure prints its size instead.
impl fmt::Debug for Binary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Binary({} bytes)", self.0.len())
    }
}

impl Serialize for Binary {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for Binary {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(Base64Visitor)
    }
}

struct Base64Visitor;

impl Visitor<'_> for Base64Visitor {
    type Value = Binary;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a base64 string")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<Binary, E> {
        STANDARD
            .decode(text)
            .map(Binary)
            .map_err(|error| E::custom(format!("invalid base64: {error}")))
    }
}
