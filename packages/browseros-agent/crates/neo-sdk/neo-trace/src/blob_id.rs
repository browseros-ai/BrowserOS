//! The id of one blob a run's reply names: `b0`, `b1`, ... within the run.
//!
//! A reply too large to send inline keeps its byte values with the run and carries
//! `{"$neo":"blob","id":"b0","size":N}` in their place; the host reads each at
//! `GET /v1/runs/{runId}/blobs/{blobId}`. The id is also the blob's file name under the run's
//! outputs, so only the canonical spelling parses: `b01` names no blob, and no text can name a
//! path.

use std::fmt;
use std::str::FromStr;

/// One blob of a run, numbered from 0 in the order the reply names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlobId(u32);

impl BlobId {
    /// The run's `index`th blob.
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }
}

impl fmt::Display for BlobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "b{}", self.0)
    }
}

/// Text that is not a blob id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a blob id is `b` and a number, such as b0")]
pub struct NotABlobId;

impl FromStr for BlobId {
    type Err = NotABlobId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let index: u32 = text
            .strip_prefix('b')
            .ok_or(NotABlobId)?
            .parse()
            .map_err(|_| NotABlobId)?;
        let id = Self(index);
        if id.to_string() == text {
            Ok(id)
        } else {
            Err(NotABlobId)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BlobId;

    #[test]
    fn only_the_canonical_spelling_names_a_blob() {
        assert_eq!("b0".parse::<BlobId>(), Ok(BlobId::new(0)));
        assert_eq!("b17".parse::<BlobId>(), Ok(BlobId::new(17)));
        assert_eq!(BlobId::new(3).to_string(), "b3");
        for text in [
            "",
            "b",
            "0",
            "b01",
            "b+1",
            "b-1",
            "B1",
            "b1/../x",
            "b99999999999",
        ] {
            assert!(text.parse::<BlobId>().is_err(), "{text:?}");
        }
    }
}
