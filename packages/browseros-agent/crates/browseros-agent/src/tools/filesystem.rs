//! The 7 filesystem tools (read/write/edit/bash/grep/find/ls) rooted at the workspace, or the
//! output-only `filesystem_read` limited to a lease's output grants (`apps/server/src/tools/filesystem`).

use super::{ToolSource, unimplemented_source};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// Which tool-output files a lease may read back; implemented by `apps/server-rust`'s lease.
pub trait OutputGrants: Send + Sync {
    fn allows(&self, path: &Path) -> bool;
}

pub enum FsScope {
    Workspace(PathBuf),
    OutputOnly(Arc<dyn OutputGrants>),
}

pub fn filesystem_tools(scope: FsScope) -> Arc<dyn ToolSource> {
    let _ = scope;
    unimplemented_source("filesystem")
}
