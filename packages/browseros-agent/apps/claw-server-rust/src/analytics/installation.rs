//! Read-only compatibility with Chromium's identity. BrowserClaw owns analytics.json;
//! an existing Chromium UUID can seed it or be linked to it, but never replaces it.

use serde::Deserialize;
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Deserialize)]
struct InstallationFile {
    install_id: String,
}

pub(crate) fn installation_path(browserclaw_dir: &Path) -> PathBuf {
    browserclaw_dir.join("installation.json")
}

pub(crate) fn valid_identity(id: &str) -> bool {
    Uuid::parse_str(id).is_ok_and(|uuid| {
        uuid.hyphenated().to_string() == id.to_ascii_lowercase() && !uuid.is_nil()
    })
}

pub(crate) fn read_installation_id(browserclaw_dir: &Path) -> io::Result<Option<String>> {
    let raw = match fs::read_to_string(installation_path(browserclaw_dir)) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let file: InstallationFile = serde_json::from_str(&raw)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if !valid_identity(&file.install_id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid installation UUID",
        ));
    }
    Ok(Some(file.install_id))
}
