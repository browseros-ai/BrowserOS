//! Pure-Rust decoder, typed protobuf schema, and normalized [`tree::SnapshotTree`] layer for
//! Chromium's `AnnotatedPageContent` (APC) returned by CDP's `Page.getAnnotatedPageContent`.
//!
//! Its place in the snapshot pipeline is untrusted wire bytes in, immutable multi-frame tree out:
//!
//! ```text
//!  CDP JSON reply   ─▶ decode_reply_json ─┐
//!  base64 payload   ─▶ decode ────────────┼─▶ decode_protobuf ─▶ AnnotatedPageContent
//!  raw proto bytes  ──────────────────────┘       │                      │
//!                                                 ▼                      ▼
//!                                           proto::*             tree::SnapshotTree
//!                                    (Edition 2023 wire schema)  (flat-tree arena + docs)
//! ```
//!
//! Normal dependencies are strictly pure (`base64`, `prost`, `serde`, `serde_json`, `thiserror`):
//! no async runtime, no CDP transport, and no browser handles. Both live browser sessions and
//! recorded fixtures feed the same entry points and build into the same [`tree::SnapshotTree`].

mod decode;
mod decode_error;
pub mod proto;
pub mod tree;

pub use decode::{MAX_CONTENT_NODE_DEPTH, decode, decode_protobuf, decode_reply_json};
pub use decode_error::DecodeError;
pub use proto::AnnotatedPageContent;
