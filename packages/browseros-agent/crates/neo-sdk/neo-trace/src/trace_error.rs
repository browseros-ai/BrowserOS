//! Why a trace read or write failed.

/// A trace store or writer failure.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    /// SQLite refused the statement or the file.
    #[error("trace store: {0}")]
    Store(#[from] sea_orm::DbErr),
    /// The store's directory could not be created.
    #[error("trace store directory: {0}")]
    Directory(#[source] std::io::Error),
    /// The writer task has stopped, so nothing more can be written.
    #[error("the trace writer has stopped")]
    Closed,
    /// A batch failed and was rolled back, so some records in it, or sent before it, are lost.
    #[error("a trace batch was rolled back: {0}")]
    BatchLost(String),
    /// A stored screenshot's file could not be read.
    #[error("screenshot file: {0}")]
    Screenshot(#[source] std::io::Error),
    /// A stored row did not read back as a record (a newer schema, or a damaged file).
    #[error("corrupt trace row: {0}")]
    Corrupt(String),
    /// The store's write-ahead log is damaged, so commits in it, private flags among them, may
    /// be lost. The store is refused before any connection could read past the damage.
    #[error("the trace store's write-ahead log is damaged")]
    DamagedLog,
}
