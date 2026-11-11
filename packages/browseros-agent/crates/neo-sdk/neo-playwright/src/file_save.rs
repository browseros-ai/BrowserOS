//! Error ownership for client-side saves on Neo's filesystem. Downloads and screenshots use
//! the same destination policy while retaining their own effect boundaries and messages.

use std::io;

use crate::ErrorCode;

/// A destination the script can fix is invalid input; disk/source failures belong to Neo.
pub(crate) const fn error_code(kind: io::ErrorKind) -> ErrorCode {
    match kind {
        io::ErrorKind::PermissionDenied
        | io::ErrorKind::NotADirectory
        | io::ErrorKind::IsADirectory
        | io::ErrorKind::ReadOnlyFilesystem
        | io::ErrorKind::InvalidFilename
        | io::ErrorKind::InvalidInput => ErrorCode::InvalidInput,
        _ => ErrorCode::Internal,
    }
}
