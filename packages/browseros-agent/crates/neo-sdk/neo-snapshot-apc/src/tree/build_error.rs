/// Failure constructing a normalized [`SnapshotTree`](super::SnapshotTree) from an
/// [`AnnotatedPageContent`](crate::AnnotatedPageContent) payload.
///
/// Separates contract-version rejection (stock Chromium without BrowserOS APC extensions) from
/// malformed root-frame metadata and arena address-space overflow.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// BrowserOS APC extension metadata is absent or older than the minimum required contract
    /// version.
    #[error(
        "unpatched Chromium APC payload (found contract version {found:?}, required >= {required})"
    )]
    Unpatched {
        /// Contract version reported by the browser, or `None` when the extension is absent.
        found: Option<u32>,
        /// Minimum contract version required to build the tree.
        required: u32,
    },
    /// The top-level `AnnotatedPageContent` omitted `main_frame_data`.
    #[error("AnnotatedPageContent missing main_frame_data")]
    MissingMainFrame,
    /// `main_frame_data.document_identifier.serialized_token` was absent or empty.
    #[error("AnnotatedPageContent main frame missing non-empty document token")]
    MissingMainDocumentToken,
    /// The node arena exceeded the maximum number of nodes addressable by `NodeId` (`u32`).
    #[error(
        "AnnotatedPageContent tree has {count} nodes, exceeding maximum addressable arena size"
    )]
    TreeTooLarge {
        /// Node count at which arena allocation failed.
        count: usize,
    },
}
