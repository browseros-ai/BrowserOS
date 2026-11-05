//! Why a trace read or write failed.

/// A trace store or writer failure.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    /// SQLite refused the statement or the file.
    #[error("trace store: {0}")]
    Store(#[from] sqlx::Error),
    /// The store's directory could not be created.
    #[error("trace store directory: {0}")]
    Directory(#[source] std::io::Error),
    /// The writer task has stopped, so nothing more can be written.
    #[error("the trace writer has stopped")]
    Closed,
    /// A stored row did not read back as a record (a newer schema, or a damaged file).
    #[error("corrupt trace row: {0}")]
    Corrupt(String),
}
