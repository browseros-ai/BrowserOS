//! `server.json`: written after the listener binds (port, pid, version) so local tools can find
//! the running server, removed on every orderly exit (shutdown route, SIGTERM/SIGINT). Owned by P3.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ServerFile {
    path: PathBuf,
}

impl ServerFile {
    pub fn new(browseros_dir: &Path) -> Self {
        Self {
            path: browseros_dir.join("server.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// S0 stub: writes nothing until P3 pins the TS file shape.
    pub async fn write(&self, port: u16) -> std::io::Result<()> {
        let _ = port;
        Ok(())
    }

    /// Best-effort; a missing file is fine.
    pub async fn remove(&self) {
        let _ = tokio::fs::remove_file(&self.path).await;
    }
}
